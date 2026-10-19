//! The live Slack client: a background worker that owns the Web API and reaches
//! the 30 Hz tick over channels, like [`crate::claude::Run`].
//!
//! Phase one keeps the conversation fresh by polling the open conversation every
//! few seconds. The same [`Event`] stream will later be fed by a websocket, with
//! polling as the fallback, without the UI noticing.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::api::{ApiError, HISTORY_LIMIT, SlackApi};
use super::auth::Credentials;
use super::http::{Http, Web};
use super::model::{Activity, AuthTest, Channel, Message, User};

/// How often an idle worker re-checks the open conversation.
const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Re-list conversations once every this many polls (~30s).
const REFRESH_EVERY_POLLS: u32 = 10;
/// At most this many people are looked up for an invite.
const MAX_PEOPLE: usize = 100;
/// Conversation types the first version shows: DMs and group DMs.
const TYPES: &str = super::api::DIRECT_TYPES;

/// A request from the main thread to the worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Reload the list of conversations.
    Refresh,
    /// Load a conversation's recent messages and mark it read.
    Open(String),
    /// Send a message, optionally as a thread reply.
    Send {
        channel: String,
        text: String,
        thread_ts: Option<String>,
    },
    /// Look up everyone in a conversation, with their email and time zone.
    People(String),
    /// Fetch the user's own recent messages, to learn how they write.
    FetchMyMessages { limit: usize },
    /// Stop the worker.
    Stop,
}

/// Something the worker learned, for the tick to apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The credentials work; here is who we are.
    Connected(AuthTest),
    /// The full conversation list.
    Conversations(Vec<Channel>),
    /// Names learned for some users.
    Users(Vec<User>),
    /// The recent messages of the conversation just opened.
    History {
        channel: String,
        messages: Vec<Message>,
    },
    /// Messages that arrived in the open conversation since last seen.
    New {
        channel: String,
        messages: Vec<Message>,
    },
    /// A message was sent to this channel.
    Sent { channel: String },
    /// Something went wrong, with a message to show.
    Error(String),
    /// The session is no longer valid; the user must sign in again.
    Expired,
    /// The people in a conversation, or why they could not be looked up.
    People {
        channel: String,
        people: Result<Vec<User>, String>,
    },
    /// The user's own recent messages, newest first, or why they could not be fetched.
    MyMessages(Result<Vec<String>, String>),
}

/// Owns the worker thread; commands go in, events come out on `events`.
pub struct SlackClient {
    commands: Sender<Command>,
    pub events: Receiver<Event>,
    handle: Option<JoinHandle<()>>,
}

impl SlackClient {
    /// Connects with `creds` and begins polling in the background. `known`
    /// people (from the cache) are not looked up again.
    pub fn connect(creds: Credentials, known: Vec<User>) -> Self {
        let (commands_tx, commands_rx) = mpsc::channel();
        let (events_tx, events_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            Worker::new(SlackApi::new(Web::new(), creds), events_tx)
                .knowing(known)
                .run(commands_rx);
        });
        Self {
            commands: commands_tx,
            events: events_rx,
            handle: Some(handle),
        }
    }

    /// Queues a command for the worker. A dropped worker swallows it harmlessly.
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

impl Drop for SlackClient {
    fn drop(&mut self) {
        // Ask the worker to stop and let it wind down on its own; joining could
        // block the app for the length of an in-flight request.
        let _ = self.commands.send(Command::Stop);
        let _ = self.handle.take();
    }
}

/// The background side. Generic over the transport so tests drive it directly.
struct Worker<H: Http> {
    api: SlackApi<H>,
    events: Sender<Event>,
    /// The conversation being viewed, polled for new messages.
    open: Option<String>,
    /// Latest message ts applied per conversation.
    seen: HashMap<String, String>,
    /// Users whose names have already been sent, or were known from the cache.
    known_users: HashSet<String>,
    /// Everything known about those users, for `People` without a lookup.
    users: HashMap<String, User>,
    polls: u32,
    /// Our own user id, once `auth.test` has answered.
    me: Option<String>,
}

impl<H: Http> Worker<H> {
    fn new(api: SlackApi<H>, events: Sender<Event>) -> Self {
        Self {
            api,
            events,
            open: None,
            seen: HashMap::new(),
            known_users: HashSet::new(),
            users: HashMap::new(),
            polls: 0,
            me: None,
        }
    }

    /// Starts out knowing `users`, so they are not looked up again.
    fn knowing(mut self, users: Vec<User>) -> Self {
        for user in users {
            self.known_users.insert(user.id.clone());
            self.users.insert(user.id.clone(), user);
        }
        self
    }

    fn run(mut self, commands: Receiver<Command>) {
        if !self.connect() {
            return;
        }
        self.handle(Command::Refresh);
        loop {
            match commands.recv_timeout(POLL_INTERVAL) {
                Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(command) => self.handle(command),
                Err(RecvTimeoutError::Timeout) => self.poll(),
            }
        }
    }

    /// Confirms the session and announces who we are. Returns whether to carry on.
    fn connect(&mut self) -> bool {
        match self.api.auth_test() {
            Ok(auth) => {
                self.me = Some(auth.user_id.clone());
                self.emit(Event::Connected(auth))
            }
            Err(err) => self.fail(&err),
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Refresh => self.refresh(),
            Command::Open(channel) => self.open(channel),
            Command::Send {
                channel,
                text,
                thread_ts,
            } => self.send(channel, text, thread_ts),
            Command::FetchMyMessages { limit } => self.fetch_my_messages(limit),
            Command::People(channel) => self.people(channel),
            Command::Stop => {}
        }
    }

    fn refresh(&mut self) {
        match self.api.all_conversations(TYPES) {
            Ok(channels) => {
                // Without activity, show every conversation rather than none.
                let channels = match self.api.counts() {
                    Ok(activity) if !activity.is_empty() => with_activity(channels, &activity),
                    _ => channels,
                };
                let others: Vec<String> = channels.iter().filter_map(|c| c.user.clone()).collect();
                self.emit(Event::Conversations(channels));
                self.resolve_users(&others);
            }
            Err(err) => self.report(&err),
        }
    }

    fn open(&mut self, channel: String) {
        match self.api.history(&channel, HISTORY_LIMIT) {
            Ok(messages) => {
                self.remember_latest(&channel, &messages);
                self.mark_read(&channel, &messages);
                let authors = authors(&messages);
                self.open = Some(channel.clone());
                self.emit(Event::History { channel, messages });
                self.resolve_users(&authors);
            }
            Err(err) => self.report(&err),
        }
    }

    fn send(&mut self, channel: String, text: String, thread_ts: Option<String>) {
        match self.api.post_message(&channel, &text, thread_ts.as_deref()) {
            Ok(()) => {
                self.emit(Event::Sent {
                    channel: channel.clone(),
                });
                // Pick up our own message (and anything else) right away.
                if self.open.as_deref() == Some(&channel) {
                    self.poll();
                }
            }
            Err(err) => self.report(&err),
        }
    }

    fn people(&mut self, channel: String) {
        let people = self
            .api
            .members(&channel)
            .and_then(|ids| {
                ids.iter()
                    .take(MAX_PEOPLE)
                    .map(|id| self.user(id))
                    .collect::<Result<Vec<User>, ApiError>>()
            })
            .map_err(|err| err.to_string());
        self.emit(Event::People { channel, people });
    }

    /// A user, from what is already known or else looked up.
    fn user(&mut self, id: &str) -> Result<User, ApiError> {
        if let Some(user) = self.users.get(id) {
            return Ok(user.clone());
        }
        let user = self.api.user_info(id)?;
        self.users.insert(id.to_string(), user.clone());
        Ok(user)
    }

    fn fetch_my_messages(&self, limit: usize) {
        let result = match &self.me {
            Some(me) => self
                .api
                .messages_from(me, limit)
                .map_err(|err| err.to_string()),
            None => Err("not connected to Slack yet".to_string()),
        };
        self.emit(Event::MyMessages(result));
    }

    /// Idle work: new messages in the open conversation, and an occasional relist.
    fn poll(&mut self) {
        self.polls = self.polls.wrapping_add(1);
        if self.polls.is_multiple_of(REFRESH_EVERY_POLLS) {
            self.refresh();
        }
        let Some(channel) = self.open.clone() else {
            return;
        };
        match self.api.history(&channel, HISTORY_LIMIT) {
            Ok(messages) => {
                let fresh = self.since_seen(&channel, messages);
                if !fresh.is_empty() {
                    self.remember_latest(&channel, &fresh);
                    self.mark_read(&channel, &fresh);
                    let authors = authors(&fresh);
                    self.emit(Event::New {
                        channel,
                        messages: fresh,
                    });
                    self.resolve_users(&authors);
                }
            }
            // A failed poll is silent unless the session died; the next tick retries.
            Err(err) if !err.retriable() => {
                self.report(&err);
            }
            Err(_) => {}
        }
    }

    /// Keeps only messages newer than the last one applied for `channel`.
    fn since_seen(&self, channel: &str, messages: Vec<Message>) -> Vec<Message> {
        match self.seen.get(channel) {
            Some(last) => messages.into_iter().filter(|m| &m.ts > last).collect(),
            None => messages,
        }
    }

    fn remember_latest(&mut self, channel: &str, messages: &[Message]) {
        if let Some(last) = messages.last() {
            self.seen.insert(channel.to_string(), last.ts.clone());
        }
    }

    fn mark_read(&self, channel: &str, messages: &[Message]) {
        if let Some(last) = messages.last() {
            let _ = self.api.mark(channel, &last.ts);
        }
    }

    /// Looks up users not seen before and sends their names in one batch.
    fn resolve_users(&mut self, ids: &[String]) {
        let mut users = Vec::new();
        for id in ids {
            if id.is_empty() || !self.known_users.insert(id.clone()) {
                continue;
            }
            if let Ok(user) = self.api.user_info(id) {
                self.users.insert(id.clone(), user.clone());
                users.push(user);
            }
        }
        if !users.is_empty() {
            self.emit(Event::Users(users));
        }
    }

    /// Turns an API failure into an event: expiry, or a one-off error.
    fn report(&self, err: &ApiError) {
        let _ = self.fail(err);
    }

    fn fail(&self, err: &ApiError) -> bool {
        if is_expiry(err) {
            self.emit(Event::Expired)
        } else {
            self.emit(Event::Error(err.to_string()))
        }
    }

    fn emit(&self, event: Event) -> bool {
        self.events.send(event).is_ok()
    }
}

/// Keeps the conversations Slack's sidebar shows, with their unread state,
/// most recent first. Users accumulate hundreds of stale direct messages; the
/// sidebar's few dozen are the ones that matter.
fn with_activity(channels: Vec<Channel>, activity: &[Activity]) -> Vec<Channel> {
    let by_id: HashMap<&str, &Activity> = activity.iter().map(|a| (a.id.as_str(), a)).collect();
    let mut shown: Vec<Channel> = channels
        .into_iter()
        .filter_map(|channel| {
            let activity = by_id.get(channel.id.as_str())?;
            Some(Channel {
                latest_ts: activity.latest.clone().or(channel.latest_ts.clone()),
                unread: activity.unread,
                mentions: activity.mentions,
                ..channel
            })
        })
        .collect();
    shown.sort_by(|a, b| b.latest_ts.cmp(&a.latest_ts));
    shown
}

/// The distinct, non-empty authors of some messages.
fn authors(messages: &[Message]) -> Vec<String> {
    let mut seen = HashSet::new();
    messages
        .iter()
        .filter_map(|m| m.user.clone())
        .filter(|u| seen.insert(u.clone()))
        .collect()
}

/// Whether the error means the session is no longer valid.
fn is_expiry(err: &ApiError) -> bool {
    use super::model::SlackError;
    matches!(
        err,
        ApiError::Slack(SlackError::Api(code))
            if matches!(code.as_str(),
                "invalid_auth" | "not_authed" | "token_revoked" | "token_expired" | "account_inactive")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack::http::{Http, Response};
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Answers each Slack method from its own queue, so a `mark` or `users.info`
    /// never steals a `conversations.history` body. Unqueued methods reply `ok`.
    struct Router {
        by_method: RefCell<HashMap<String, Vec<String>>>,
    }

    impl Router {
        fn new() -> Self {
            Self {
                by_method: RefCell::new(HashMap::new()),
            }
        }

        fn on(self, method: &str, bodies: &[&str]) -> Self {
            self.by_method.borrow_mut().insert(
                method.to_string(),
                bodies.iter().map(|s| s.to_string()).collect(),
            );
            self
        }
    }

    impl Http for Router {
        fn post_form(&self, url: &str, _: &str, _: &[(&str, &str)]) -> Result<Response, String> {
            let method = url.rsplit('/').next().unwrap_or_default();
            let body = self
                .by_method
                .borrow_mut()
                .get_mut(method)
                .filter(|queue| !queue.is_empty())
                .map(|queue| queue.remove(0))
                .unwrap_or_else(|| r#"{"ok":true}"#.to_string());
            Ok(Response { status: 200, body })
        }
    }

    fn creds() -> Credentials {
        Credentials {
            token: "xoxc".into(),
            cookie: "xoxd".into(),
            team_id: "T1".into(),
            domain: "d".into(),
            user_id: "U1".into(),
        }
    }

    fn worker(router: Router) -> (Worker<Router>, Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        (Worker::new(SlackApi::new(router, creds()), tx), rx)
    }

    fn events(rx: &Receiver<Event>) -> Vec<Event> {
        rx.try_iter().collect()
    }

    const BOB: &str = r#"{"ok":true,"user":{"id":"U2","profile":{"display_name":"bob"}}}"#;

    fn channel(id: &str, user: &str) -> Channel {
        Channel {
            id: id.into(),
            kind: crate::slack::model::ChannelKind::Im,
            name: String::new(),
            user: Some(user.into()),
            latest_ts: None,
            unread: false,
            mentions: 0,
        }
    }

    #[test]
    fn activity_keeps_sidebar_conversations_newest_first() {
        let channels = vec![
            channel("D1", "U1"),
            channel("D2", "U2"),
            channel("D3", "U3"),
        ];
        let activity = [
            Activity {
                id: "D1".into(),
                latest: Some("1.0".into()),
                unread: false,
                mentions: 0,
            },
            Activity {
                id: "D3".into(),
                latest: Some("9.0".into()),
                unread: true,
                mentions: 2,
            },
        ];
        let shown = with_activity(channels, &activity);
        let ids: Vec<&str> = shown.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["D3", "D1"]);
        assert!(shown[0].unread);
        assert_eq!(shown[0].mentions, 2);
        assert_eq!(shown[0].latest_ts.as_deref(), Some("9.0"));
    }

    #[test]
    fn refresh_resolves_only_shown_conversations() {
        let list = r#"{"ok":true,"channels":[
            {"id":"D1","is_im":true,"user":"U2"},{"id":"D2","is_im":true,"user":"U3"}]}"#;
        let counts = r#"{"ok":true,"ims":[{"id":"D1","latest":"1.0"}]}"#;
        let (mut worker, rx) = worker(
            Router::new()
                .on("users.conversations", &[list])
                .on("client.counts", &[counts])
                .on("users.info", &[BOB]),
        );
        worker.refresh();
        match events(&rx).as_slice() {
            [Event::Conversations(shown), Event::Users(users)] => {
                assert_eq!(shown.len(), 1);
                assert_eq!(users.len(), 1);
            }
            other => panic!("{other:?}"),
        }
        assert!(!worker.known_users.contains("U3"));
    }

    #[test]
    fn connect_emits_identity() {
        let (mut worker, rx) =
            worker(Router::new().on("auth.test", &[r#"{"ok":true,"user_id":"U1"}"#]));
        assert!(worker.connect());
        assert!(matches!(events(&rx).as_slice(), [Event::Connected(_)]));
    }

    #[test]
    fn people_are_the_members_with_their_profiles() {
        let members = r#"{"ok":true,"members":["U2"]}"#;
        let bob = r#"{"ok":true,"user":{"id":"U2","tz":"Europe/Paris",
            "profile":{"display_name":"bob","email":"bob@example.com"}}}"#;
        let (mut worker, rx) = worker(
            Router::new()
                .on("conversations.members", &[members])
                .on("users.info", &[bob]),
        );
        worker.handle(Command::People("G1".into()));
        match events(&rx).as_slice() {
            [
                Event::People {
                    channel,
                    people: Ok(people),
                },
            ] => {
                assert_eq!(channel, "G1");
                assert_eq!(people[0].email, "bob@example.com");
                assert_eq!(people[0].tz, "Europe/Paris");
            }
            other => panic!("{other:?}"),
        }
    }

    fn cached_bob() -> User {
        User {
            id: "U2".into(),
            real_name: "Bob".into(),
            display_name: "bob".into(),
            email: "bob@example.com".into(),
            tz: "Europe/Paris".into(),
        }
    }

    #[test]
    fn people_known_from_the_cache_are_not_looked_up() {
        let list = r#"{"ok":true,"channels":[{"id":"D1","is_im":true,"user":"U2"}]}"#;
        let (worker, rx) = worker(Router::new().on("users.conversations", &[list]));
        let mut worker = worker.knowing(vec![cached_bob()]);
        // No users.info is queued: a lookup would fail and send no name.
        worker.refresh();
        assert!(matches!(events(&rx).as_slice(), [Event::Conversations(_)]));
    }

    #[test]
    fn people_in_a_conversation_come_from_the_cache_when_known() {
        let members = r#"{"ok":true,"members":["U2"]}"#;
        let (worker, rx) = worker(Router::new().on("conversations.members", &[members]));
        let mut worker = worker.knowing(vec![cached_bob()]);
        worker.handle(Command::People("G1".into()));
        match events(&rx).as_slice() {
            [
                Event::People {
                    people: Ok(people), ..
                },
            ] => assert_eq!(people, &[cached_bob()]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn my_messages_are_searched_as_the_connected_user() {
        let search = r#"{"ok":true,"messages":{"paging":{"page":1,"pages":1},
            "matches":[{"text":"on it"},{"text":"done"}]}}"#;
        let (mut worker, rx) = worker(
            Router::new()
                .on("auth.test", &[r#"{"ok":true,"user_id":"U1"}"#])
                .on("search.messages", &[search]),
        );
        worker.handle(Command::FetchMyMessages { limit: 10 });
        assert_eq!(
            events(&rx),
            vec![Event::MyMessages(Err("not connected to Slack yet".into()))]
        );
        worker.connect();
        let _ = events(&rx);
        worker.handle(Command::FetchMyMessages { limit: 10 });
        assert_eq!(
            events(&rx),
            vec![Event::MyMessages(Ok(vec!["on it".into(), "done".into()]))]
        );
    }

    #[test]
    fn connect_reports_expiry_on_invalid_auth() {
        let (mut worker, rx) =
            worker(Router::new().on("auth.test", &[r#"{"ok":false,"error":"invalid_auth"}"#]));
        worker.connect();
        assert_eq!(events(&rx), vec![Event::Expired]);
    }

    #[test]
    fn open_loads_history_then_resolves_authors() {
        let history = r#"{"ok":true,"messages":[{"ts":"1.0","user":"U2","text":"hi"}]}"#;
        let (mut worker, rx) = worker(
            Router::new()
                .on("conversations.history", &[history])
                .on("users.info", &[BOB]),
        );
        worker.open("D1".into());
        let got = events(&rx);
        assert!(matches!(got[0], Event::History { .. }));
        match &got[1] {
            Event::Users(users) => assert_eq!(users[0].name(), "bob"),
            other => panic!("{other:?}"),
        }
        assert_eq!(worker.open.as_deref(), Some("D1"));
        assert_eq!(worker.seen.get("D1").map(String::as_str), Some("1.0"));
    }

    #[test]
    fn poll_emits_only_messages_newer_than_seen() {
        let first = r#"{"ok":true,"messages":[{"ts":"1.0","user":"U2","text":"one"}]}"#;
        let second = r#"{"ok":true,"messages":[
            {"ts":"2.0","user":"U2","text":"two"},{"ts":"1.0","user":"U2","text":"one"}]}"#;
        let (mut worker, rx) = worker(Router::new().on("conversations.history", &[first, second]));
        worker.known_users.insert("U2".into()); // skip user_info lookups
        worker.open("D1".into());
        let _ = events(&rx);
        worker.poll();
        let got = events(&rx);
        match got.iter().find(|e| matches!(e, Event::New { .. })) {
            Some(Event::New { messages, .. }) => {
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].text, "two");
            }
            _ => panic!("expected one new message, got {got:?}"),
        }
        assert_eq!(worker.seen.get("D1").map(String::as_str), Some("2.0"));
    }

    #[test]
    fn poll_without_new_messages_is_quiet() {
        let same = r#"{"ok":true,"messages":[{"ts":"1.0","user":"U2","text":"one"}]}"#;
        let (mut worker, rx) = worker(Router::new().on("conversations.history", &[same, same]));
        worker.known_users.insert("U2".into());
        worker.open("D1".into());
        let _ = events(&rx);
        worker.poll();
        assert!(!events(&rx).iter().any(|e| matches!(e, Event::New { .. })));
    }

    #[test]
    fn send_reports_a_slack_error() {
        let (mut worker, rx) = worker(Router::new().on(
            "chat.postMessage",
            &[r#"{"ok":false,"error":"msg_too_long"}"#],
        ));
        worker.send("D1".into(), "x".into(), None);
        assert!(matches!(events(&rx).as_slice(), [Event::Error(_)]));
    }

    #[test]
    fn refresh_lists_conversations_and_resolves_im_partners() {
        let list = r#"{"ok":true,"channels":[{"id":"D1","is_im":true,"user":"U2"}]}"#;
        let (mut worker, rx) = worker(
            Router::new()
                .on("users.conversations", &[list])
                .on("users.info", &[BOB]),
        );
        worker.refresh();
        let got = events(&rx);
        assert!(matches!(&got[0], Event::Conversations(c) if c.len() == 1));
        assert!(matches!(&got[1], Event::Users(u) if u[0].name() == "bob"));
    }
}
