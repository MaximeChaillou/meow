//! The Slack tools of Meow's MCP server: the user's conversations, their
//! history and threads, and search, through the session Meow signed in with.
//!
//! Names come from Meow's Slack cache; the session is only read from the
//! Keychain once a tool needs Slack itself, so chats that never touch Slack
//! never reach for it.

use std::cell::OnceCell;
use std::collections::HashSet;

use serde_json::Value;

use super::{int_arg, str_arg};
use crate::clock;
use crate::slack::api::{ALL_TYPES, SlackApi};
use crate::slack::cache::SlackCache;
use crate::slack::format::{self, NameMap};
use crate::slack::http::Http;
use crate::slack::model::{Channel, ChannelKind, Message, SearchHit};

const DEFAULT_CONVERSATIONS: u64 = 50;
const DEFAULT_HISTORY: u64 = 30;
const MAX_HISTORY: u64 = 200;
const DEFAULT_SEARCH: u64 = 20;
const MAX_SEARCH: u64 = 100;
/// People looked up per call when the cache does not know them.
const MAX_LOOKUPS: usize = 20;

type Connect<H> = Box<dyn Fn() -> Result<SlackApi<H>, String>>;

pub struct SlackTools<H: Http> {
    cache: SlackCache,
    connect: Connect<H>,
    api: OnceCell<Result<SlackApi<H>, String>>,
    now: u64,
}

impl<H: Http> SlackTools<H> {
    /// `connect` opens the session the first time a tool needs Slack.
    pub fn new(cache: SlackCache, connect: Connect<H>, now: u64) -> Self {
        Self {
            cache,
            connect,
            api: OnceCell::new(),
            now,
        }
    }

    fn api(&self) -> Result<&SlackApi<H>, String> {
        self.api
            .get_or_init(|| (self.connect)())
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The user's conversations, most recent first.
    pub fn conversations(&self, args: &Value) -> Result<String, String> {
        let unread_only = args.get("unread_only").and_then(Value::as_bool) == Some(true);
        let limit = int_arg(args, "limit", DEFAULT_CONVERSATIONS, u64::MAX) as usize;
        let fetched;
        let channels = if self.cache.channels.is_empty() {
            fetched = self
                .api()?
                .all_conversations(ALL_TYPES)
                .map_err(|e| e.to_string())?;
            &fetched
        } else {
            &self.cache.channels
        };
        let names = self.names();
        let lines: Vec<String> = channels
            .iter()
            .filter(|c| !unread_only || c.unread)
            .take(limit)
            .map(|c| conversation_line(c, &names))
            .collect();
        if lines.is_empty() {
            return Ok("No conversations.".to_string());
        }
        Ok(lines.join("\n"))
    }

    /// The latest messages of one conversation, oldest first.
    pub fn history(&self, args: &Value) -> Result<String, String> {
        let channel = self.resolve(str_arg(args, "channel")?)?;
        let limit = int_arg(args, "limit", DEFAULT_HISTORY, MAX_HISTORY) as u16;
        let messages = self
            .api()?
            .history(&channel, limit)
            .map_err(|e| e.to_string())?;
        Ok(self.render(&messages, "No messages."))
    }

    /// A thread: its first message, then the replies.
    pub fn thread(&self, args: &Value) -> Result<String, String> {
        let channel = self.resolve(str_arg(args, "channel")?)?;
        let thread_ts = str_arg(args, "thread_ts")?;
        let messages = self
            .api()?
            .replies(&channel, thread_ts)
            .map_err(|e| e.to_string())?;
        Ok(self.render(&messages, "No such thread."))
    }

    /// Messages matching a Slack search query, newest first.
    pub fn search(&self, args: &Value) -> Result<String, String> {
        let query = str_arg(args, "query")?;
        let limit = int_arg(args, "limit", DEFAULT_SEARCH, MAX_SEARCH) as u16;
        let hits = self
            .api()?
            .search(query, limit)
            .map_err(|e| e.to_string())?;
        if hits.is_empty() {
            return Ok("No messages match.".to_string());
        }
        let users: Vec<&str> = hits.iter().filter_map(|h| h.user.as_deref()).collect();
        let names = self.names_with(&users);
        let lines: Vec<String> = hits
            .iter()
            .map(|hit| hit_line(hit, &names, self.now))
            .collect();
        Ok(lines.join("\n"))
    }

    /// A conversation id from an id, `#channel`, channel name or person's name.
    fn resolve(&self, wanted: &str) -> Result<String, String> {
        let wanted = wanted.trim().trim_start_matches(['#', '@']);
        let names = self.names();
        let found = self.cache.channels.iter().find(|c| {
            c.id == wanted
                || c.name.eq_ignore_ascii_case(wanted)
                || c.user
                    .as_deref()
                    .and_then(|u| names.users.get(u))
                    .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
        });
        match found {
            Some(channel) => Ok(channel.id.clone()),
            None if looks_like_id(wanted) => Ok(wanted.to_string()),
            None => Err(format!(
                "No conversation called \"{wanted}\". Use slack_conversations to find its id."
            )),
        }
    }

    fn names(&self) -> NameMap {
        NameMap {
            users: self.cache.names().collect(),
            channels: self
                .cache
                .channels
                .iter()
                .filter(|c| !c.name.is_empty())
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect(),
        }
    }

    /// Cached names, plus `users` the cache does not know, looked up.
    fn names_with(&self, users: &[&str]) -> NameMap {
        let mut names = self.names();
        let unknown: HashSet<&str> = users
            .iter()
            .copied()
            .filter(|id| !names.users.contains_key(*id))
            .collect();
        let Ok(api) = self.api() else { return names };
        let found = unknown
            .into_iter()
            .take(MAX_LOOKUPS)
            .filter_map(|id| api.user_info(id).ok());
        names
            .users
            .extend(found.map(|user| (user.id.clone(), user.name().to_string())));
        names
    }

    fn render(&self, messages: &[Message], empty: &str) -> String {
        if messages.is_empty() {
            return empty.to_string();
        }
        let users: Vec<&str> = messages.iter().filter_map(|m| m.user.as_deref()).collect();
        let names = self.names_with(&users);
        messages
            .iter()
            .map(|m| message_line(m, &names, self.now))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn looks_like_id(text: &str) -> bool {
    text.len() >= 9
        && text.starts_with(['C', 'D', 'G'])
        && text
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn conversation_line(channel: &Channel, names: &NameMap) -> String {
    let label = match channel.kind {
        ChannelKind::Im => {
            let person = channel.user.as_deref().unwrap_or("someone");
            let person = names.users.get(person).map_or(person, String::as_str);
            format!("DM with {person}")
        }
        ChannelKind::Mpim => format!("group DM {}", channel.name),
        ChannelKind::Channel => format!("#{}", channel.name),
    };
    let mut flags = Vec::new();
    if channel.unread {
        flags.push("unread".to_string());
    }
    if channel.mentions > 0 {
        flags.push(format!("{} mentions", channel.mentions));
    }
    let flags = if flags.is_empty() {
        String::new()
    } else {
        format!(", {}", flags.join(", "))
    };
    format!("- {label} (id {}{flags})", channel.id)
}

fn author(user: Option<&str>, fallback: Option<&str>, names: &NameMap) -> String {
    user.and_then(|id| names.users.get(id).cloned())
        .or_else(|| fallback.filter(|f| !f.is_empty()).map(str::to_string))
        .or_else(|| user.map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn when(ts: &str, now: u64) -> String {
    let secs = ts.split('.').next().and_then(|s| s.parse::<u64>().ok());
    secs.map(|at| clock::when_label(at, now))
        .unwrap_or_default()
}

fn message_line(message: &Message, names: &NameMap, now: u64) -> String {
    let who = author(message.user.as_deref(), message.bot_name.as_deref(), names);
    let mut line = format!(
        "[{}] {who}: {}",
        when(&message.ts, now),
        format::to_markdown(&message.text, names)
    );
    let attached: Vec<&str> = message
        .files
        .iter()
        .map(String::as_str)
        .chain(message.images.iter().map(|i| i.title.as_str()))
        .collect();
    if !attached.is_empty() {
        line.push_str(&format!(" [attached: {}]", attached.join(", ")));
    }
    if let Some(summary) = &message.attachment {
        line.push_str(&format!(" [{summary}]"));
    }
    for forwarded in &message.forwarded {
        let from = author(forwarded.user.as_deref(), Some(&forwarded.author), names);
        let text = format::to_markdown(&forwarded.text, names);
        line.push_str(&format!(" [forwarded from {from}: {text}]"));
    }
    let thread = if message.reply_count > 0 {
        format!(", thread with {} replies", message.reply_count)
    } else {
        String::new()
    };
    format!("{line} (ts {}{thread})", message.ts)
}

fn hit_line(hit: &SearchHit, names: &NameMap, now: u64) -> String {
    let who = author(hit.user.as_deref(), Some(&hit.username), names);
    let place = names
        .users
        .get(&hit.channel_name)
        .map(|person| format!("DM with {person}"))
        .unwrap_or_else(|| format!("#{}", hit.channel_name));
    let link = hit
        .permalink
        .as_deref()
        .map(|l| format!(", {l}"))
        .unwrap_or_default();
    format!(
        "[{}] {who} in {place}: {} (channel {}, ts {}{link})",
        when(&hit.ts, now),
        format::to_markdown(&hit.text, names),
        hit.channel_id,
        hit.ts
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack::auth::Credentials;
    use crate::slack::http::Response;
    use crate::slack::model::User;
    use serde_json::json;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Replays queued bodies and records the methods called.
    #[derive(Clone, Default)]
    struct FakeHttp {
        bodies: Rc<RefCell<Vec<String>>>,
        methods: Rc<RefCell<Vec<String>>>,
    }

    impl Http for FakeHttp {
        fn post_form(&self, url: &str, _: &str, _: &[(&str, &str)]) -> Result<Response, String> {
            let method = url.rsplit('/').next().unwrap_or_default().to_string();
            self.methods.borrow_mut().push(method);
            let body = self.bodies.borrow_mut().remove(0);
            Ok(Response { status: 200, body })
        }
    }

    fn channel(id: &str, kind: ChannelKind, name: &str, user: Option<&str>) -> Channel {
        Channel {
            id: id.into(),
            kind,
            name: name.into(),
            user: user.map(str::to_string),
            latest_ts: None,
            unread: id == "D0000ANA1",
            mentions: 0,
        }
    }

    fn cache() -> SlackCache {
        let mut cache = SlackCache::default();
        cache.set_channels(vec![
            channel("D0000ANA1", ChannelKind::Im, "", Some("U1")),
            channel("C0000ENG1", ChannelKind::Channel, "eng", None),
        ]);
        let ana = User {
            id: "U1".into(),
            real_name: "Ana Lopez".into(),
            display_name: "ana".into(),
            email: String::new(),
            tz: String::new(),
        };
        cache.add_users(&[ana], 0);
        cache
    }

    fn tools(bodies: &[&str]) -> (SlackTools<FakeHttp>, FakeHttp) {
        let http = FakeHttp::default();
        *http.bodies.borrow_mut() = bodies.iter().map(|b| b.to_string()).collect();
        let shared = http.clone();
        let connect: Connect<FakeHttp> = Box::new(move || {
            Ok(SlackApi::new(
                shared.clone(),
                Credentials {
                    token: "xoxc".into(),
                    cookie: "xoxd".into(),
                    team_id: "T1".into(),
                    domain: "acme".into(),
                    user_id: "U0".into(),
                },
            ))
        });
        (SlackTools::new(cache(), connect, 0), http)
    }

    #[test]
    fn conversations_come_from_the_cache_without_slack() {
        let (tools, http) = tools(&[]);
        let all = tools.conversations(&json!({})).unwrap();
        assert_eq!(
            all,
            "- DM with ana (id D0000ANA1, unread)\n- #eng (id C0000ENG1)"
        );
        let unread = tools
            .conversations(&json!({ "unread_only": true }))
            .unwrap();
        assert_eq!(unread, "- DM with ana (id D0000ANA1, unread)");
        assert!(http.methods.borrow().is_empty());
    }

    #[test]
    fn conversations_resolve_by_name_person_or_id() {
        let (tools, _) = tools(&[]);
        assert_eq!(tools.resolve("#eng").unwrap(), "C0000ENG1");
        assert_eq!(tools.resolve("ANA").unwrap(), "D0000ANA1");
        assert_eq!(tools.resolve("C0123ABCD").unwrap(), "C0123ABCD");
        assert!(tools.resolve("random").is_err());
    }

    #[test]
    fn history_names_authors_and_marks_threads() {
        let body = r#"{"ok":true,"messages":[
            {"ts":"2.0","user":"U1","text":"see <@U1>","thread_ts":"2.0","reply_count":3}]}"#;
        let (tools, http) = tools(&[body]);
        let text = tools.history(&json!({ "channel": "eng" })).unwrap();
        assert!(text.contains("] ana: see @ana"), "{text}");
        assert!(text.ends_with("(ts 2.0, thread with 3 replies)"), "{text}");
        assert_eq!(*http.methods.borrow(), ["conversations.history"]);
    }

    #[test]
    fn unknown_authors_are_looked_up() {
        let history = r#"{"ok":true,"messages":[{"ts":"1.0","user":"U9","text":"hi"}]}"#;
        let user = r#"{"ok":true,"user":{"id":"U9","name":"bo","real_name":"Bo"}}"#;
        let (tools, http) = tools(&[history, user]);
        let text = tools.history(&json!({ "channel": "C0000ENG1" })).unwrap();
        assert!(text.contains("] Bo: hi"), "{text}");
        assert_eq!(
            *http.methods.borrow(),
            ["conversations.history", "users.info"]
        );
    }

    #[test]
    fn thread_needs_its_ts() {
        let (tools, _) = tools(&[]);
        assert!(tools.thread(&json!({ "channel": "eng" })).is_err());
    }

    #[test]
    fn search_shows_where_each_hit_is() {
        let body = r#"{"ok":true,"messages":{"matches":[{"text":"ship it","ts":"5.0",
            "user":"U1","channel":{"id":"C0000ENG1","name":"eng"},"permalink":"https://x/p"}]}}"#;
        let (tools, _) = tools(&[body]);
        let text = tools.search(&json!({ "query": "ship" })).unwrap();
        assert!(
            text.ends_with("ana in #eng: ship it (channel C0000ENG1, ts 5.0, https://x/p)"),
            "{text}"
        );
    }

    #[test]
    fn a_missing_session_is_reported() {
        let connect: Connect<FakeHttp> = Box::new(|| Err("Not signed in to Slack.".into()));
        let tools = SlackTools::new(cache(), connect, 0);
        let err = tools.history(&json!({ "channel": "eng" })).unwrap_err();
        assert_eq!(err, "Not signed in to Slack.");
    }
}
