//! The Slack Web API methods Meow uses, over an [`Http`] transport.
//!
//! Every method is a form POST to `https://<workspace>.slack.com/api/<method>`
//! carrying the session token and the `d` cookie. Bodies are turned into domain
//! types by [`super::model`].

use std::collections::HashMap;

use super::auth::Credentials;
use super::http::Http;
use super::model::{self, Activity, AuthTest, Channel, Message, Page, SlackError, User};

/// Conversation kinds to list for the first version: DMs and group DMs.
pub const DIRECT_TYPES: &str = "im,mpim";
/// Direct and group messages, plus the channels the user is in.
pub const ALL_TYPES: &str = "im,mpim,public_channel,private_channel";
/// Results per `search.messages` page (Slack's maximum).
const SEARCH_PAGE_SIZE: &str = "100";
/// How many messages to load for a conversation at once.
pub const HISTORY_LIMIT: u16 = 50;
/// Replies per `conversations.replies` page.
const REPLIES_PAGE_SIZE: &str = "200";
/// A thread is cut off after this many pages of replies.
const MAX_REPLY_PAGES: usize = 5;

/// Why an API call did not yield data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// The request could not be made (network).
    Transport(String),
    /// A non-success HTTP status (e.g. 429 rate limited, 5xx).
    Http(u16),
    /// Slack replied, but with an error or an unexpected shape.
    Slack(SlackError),
}

impl ApiError {
    /// Whether retrying later might succeed.
    pub fn retriable(&self) -> bool {
        match self {
            ApiError::Transport(_) => true,
            ApiError::Http(status) => *status == 429 || *status >= 500,
            ApiError::Slack(_) => false,
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Transport(why) => write!(f, "{why}"),
            ApiError::Http(429) => write!(f, "Slack is rate limiting; backing off"),
            ApiError::Http(status) => write!(f, "Slack returned HTTP {status}"),
            ApiError::Slack(err) => write!(f, "{err}"),
        }
    }
}

/// A Slack client bound to one set of credentials.
pub struct SlackApi<H: Http> {
    http: H,
    creds: Credentials,
}

impl<H: Http> SlackApi<H> {
    pub fn new(http: H, creds: Credentials) -> Self {
        Self { http, creds }
    }

    /// The signed-in user and workspace.
    pub fn creds(&self) -> &Credentials {
        &self.creds
    }

    /// Confirms the credentials still work.
    pub fn auth_test(&self) -> Result<AuthTest, ApiError> {
        let body = self.call("auth.test", &[])?;
        model::parse_auth_test(&body).map_err(ApiError::Slack)
    }

    /// One page of the user's conversations of the given `types`.
    pub fn conversations(
        &self,
        types: &str,
        cursor: Option<&str>,
    ) -> Result<Page<Channel>, ApiError> {
        let mut params = vec![
            ("types", types),
            ("exclude_archived", "true"),
            ("limit", "200"),
        ];
        if let Some(cursor) = cursor {
            params.push(("cursor", cursor));
        }
        let body = self.call("users.conversations", &params)?;
        model::parse_channels(&body).map_err(ApiError::Slack)
    }

    /// Every conversation of the given `types`, following pagination.
    pub fn all_conversations(&self, types: &str) -> Result<Vec<Channel>, ApiError> {
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = self.conversations(types, cursor.as_deref())?;
            all.extend(page.items);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(all),
            }
        }
    }

    /// The most recent messages of a conversation, oldest-first.
    pub fn history(&self, channel: &str, limit: u16) -> Result<Vec<Message>, ApiError> {
        let limit = limit.to_string();
        let body = self.call(
            "conversations.history",
            &[("channel", channel), ("limit", &limit)],
        )?;
        model::parse_messages(&body)
            .map(|page| page.items)
            .map_err(ApiError::Slack)
    }

    /// A thread: its parent, then the replies oldest-first, following
    /// pagination up to [`MAX_REPLY_PAGES`].
    pub fn replies(&self, channel: &str, thread_ts: &str) -> Result<Vec<Message>, ApiError> {
        let mut all: Vec<Message> = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_REPLY_PAGES {
            let mut params = vec![
                ("channel", channel),
                ("ts", thread_ts),
                ("limit", REPLIES_PAGE_SIZE),
            ];
            if let Some(cursor) = cursor.as_deref() {
                params.push(("cursor", cursor));
            }
            let body = self.call("conversations.replies", &params)?;
            let page = model::parse_replies(&body).map_err(ApiError::Slack)?;
            // Each page may lead with the parent again.
            let fresh: Vec<Message> = page
                .items
                .into_iter()
                .filter(|message| !all.iter().any(|seen| seen.ts == message.ts))
                .collect();
            all.extend(fresh);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Ok(all)
    }

    /// Unread state and latest activity of the direct and group conversations
    /// Slack's own sidebar shows. A web-client method, as the session token is.
    pub fn counts(&self) -> Result<Vec<Activity>, ApiError> {
        let body = self.call("client.counts", &[])?;
        model::parse_counts(&body).map_err(ApiError::Slack)
    }

    /// The workspace's custom emoji: names to an image URL or `alias:name`.
    pub fn emoji_list(&self) -> Result<HashMap<String, String>, ApiError> {
        let body = self.call("emoji.list", &[])?;
        model::parse_emoji_list(&body).map_err(ApiError::Slack)
    }

    /// Up to `limit` of `user_id`'s own messages, newest first, from search.
    pub fn messages_from(&self, user_id: &str, limit: usize) -> Result<Vec<String>, ApiError> {
        let query = format!("from:<@{user_id}>");
        let mut all = Vec::new();
        let mut page = "1".to_string();
        loop {
            let body = self.call(
                "search.messages",
                &[
                    ("query", &query),
                    ("count", SEARCH_PAGE_SIZE),
                    ("page", &page),
                    ("sort", "timestamp"),
                    ("sort_dir", "desc"),
                ],
            )?;
            let found = model::parse_search(&body).map_err(ApiError::Slack)?;
            all.extend(found.items);
            match found.next_cursor {
                Some(next) if all.len() < limit => page = next,
                _ => break,
            }
        }
        all.truncate(limit);
        Ok(all)
    }

    /// Every member of a conversation, following pagination.
    pub fn members(&self, channel: &str) -> Result<Vec<String>, ApiError> {
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut params = vec![("channel", channel), ("limit", "200")];
            if let Some(cursor) = cursor.as_deref() {
                params.push(("cursor", cursor));
            }
            let body = self.call("conversations.members", &params)?;
            let page = model::parse_members(&body).map_err(ApiError::Slack)?;
            all.extend(page.items);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(all),
            }
        }
    }

    /// Looks up one user.
    pub fn user_info(&self, id: &str) -> Result<User, ApiError> {
        let body = self.call("users.info", &[("user", id)])?;
        model::parse_user(&body).map_err(ApiError::Slack)
    }

    /// Sends a message as the signed-in user. `thread_ts` replies in a thread.
    pub fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<(), ApiError> {
        let mut params = vec![("channel", channel), ("text", text)];
        if let Some(thread_ts) = thread_ts {
            params.push(("thread_ts", thread_ts));
        }
        let body = self.call("chat.postMessage", &params)?;
        model::parse_ok(&body).map_err(ApiError::Slack)
    }

    /// Marks a conversation read up to `ts`, syncing with the user's other Slack apps.
    pub fn mark(&self, channel: &str, ts: &str) -> Result<(), ApiError> {
        let body = self.call("conversations.mark", &[("channel", channel), ("ts", ts)])?;
        model::parse_ok(&body).map_err(ApiError::Slack)
    }

    /// Posts to `method` with the token prepended, returning the raw body.
    fn call(&self, method: &str, params: &[(&str, &str)]) -> Result<String, ApiError> {
        let url = format!("https://{}.slack.com/api/{method}", self.creds.domain);
        let mut all = Vec::with_capacity(params.len() + 1);
        all.push(("token", self.creds.token.as_str()));
        all.extend_from_slice(params);
        let response = self
            .http
            .post_form(&url, &self.creds.cookie, &all)
            .map_err(ApiError::Transport)?;
        if (200..300).contains(&response.status) {
            Ok(response.body)
        } else {
            Err(ApiError::Http(response.status))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack::http::Response;
    use std::cell::RefCell;

    /// Records each request and replays queued responses.
    struct FakeHttp {
        calls: RefCell<Vec<Call>>,
        responses: RefCell<Vec<Response>>,
    }

    #[derive(Clone)]
    struct Call {
        url: String,
        cookie: String,
        params: Vec<(String, String)>,
    }

    impl FakeHttp {
        fn with(bodies: &[&str]) -> Self {
            let responses = bodies
                .iter()
                .rev()
                .map(|b| Response {
                    status: 200,
                    body: (*b).to_string(),
                })
                .collect();
            Self {
                calls: RefCell::new(Vec::new()),
                responses: RefCell::new(responses),
            }
        }

        fn status(status: u16) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                responses: RefCell::new(vec![Response {
                    status,
                    body: String::new(),
                }]),
            }
        }

        fn last(&self) -> Call {
            self.calls.borrow().last().cloned().unwrap()
        }
    }

    impl Http for FakeHttp {
        fn post_form(
            &self,
            url: &str,
            cookie: &str,
            params: &[(&str, &str)],
        ) -> Result<Response, String> {
            self.calls.borrow_mut().push(Call {
                url: url.to_string(),
                cookie: cookie.to_string(),
                params: params
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            });
            Ok(self
                .responses
                .borrow_mut()
                .pop()
                .expect("no response queued"))
        }
    }

    fn creds() -> Credentials {
        Credentials {
            token: "xoxc-tok".into(),
            cookie: "xoxd-ck".into(),
            team_id: "T1".into(),
            domain: "acme".into(),
            user_id: "U1".into(),
        }
    }

    fn api(http: FakeHttp) -> SlackApi<FakeHttp> {
        SlackApi::new(http, creds())
    }

    #[test]
    fn call_targets_the_workspace_and_sends_token_and_cookie() {
        let api = api(FakeHttp::with(&[
            r#"{"ok":true,"user_id":"U1","team_id":"T1"}"#,
        ]));
        api.auth_test().unwrap();
        let call = api.http.last();
        assert_eq!(call.url, "https://acme.slack.com/api/auth.test");
        assert_eq!(call.cookie, "xoxd-ck");
        assert!(call.params.contains(&("token".into(), "xoxc-tok".into())));
    }

    #[test]
    fn auth_test_returns_identity() {
        let api = api(FakeHttp::with(&[
            r#"{"ok":true,"user_id":"U7","team_id":"T7"}"#,
        ]));
        assert_eq!(api.auth_test().unwrap().user_id, "U7");
    }

    #[test]
    fn slack_error_becomes_api_error() {
        let api = api(FakeHttp::with(&[r#"{"ok":false,"error":"invalid_auth"}"#]));
        let err = api.auth_test().unwrap_err();
        assert_eq!(err, ApiError::Slack(SlackError::Api("invalid_auth".into())));
        assert!(!err.retriable());
    }

    #[test]
    fn rate_limit_is_a_retriable_http_error() {
        let api = api(FakeHttp::status(429));
        let err = api.conversations(DIRECT_TYPES, None).unwrap_err();
        assert_eq!(err, ApiError::Http(429));
        assert!(err.retriable());
    }

    #[test]
    fn all_conversations_follows_the_cursor() {
        let page1 = r#"{"ok":true,"channels":[{"id":"D1","is_im":true}],
            "response_metadata":{"next_cursor":"c2"}}"#;
        let page2 = r#"{"ok":true,"channels":[{"id":"D2","is_im":true}]}"#;
        let api = api(FakeHttp::with(&[page1, page2]));
        let channels = api.all_conversations(DIRECT_TYPES).unwrap();
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[1].id, "D2");
        // Second request carried the cursor from the first page.
        let calls = api.http.calls.borrow();
        assert!(calls[1].params.contains(&("cursor".into(), "c2".into())));
    }

    #[test]
    fn history_is_oldest_first() {
        let body = r#"{"ok":true,"messages":[
            {"ts":"2.0","text":"b"},{"ts":"1.0","text":"a"}]}"#;
        let api = api(FakeHttp::with(&[body]));
        let messages = api.history("D1", 50).unwrap();
        assert_eq!(messages[0].text, "a");
        assert_eq!(messages[1].text, "b");
    }

    #[test]
    fn replies_are_parent_first_across_pages() {
        let page1 = r#"{"ok":true,"messages":[
            {"ts":"1.0","text":"parent","thread_ts":"1.0","reply_count":2},
            {"ts":"2.0","text":"a","thread_ts":"1.0"}],
            "response_metadata":{"next_cursor":"c2"}}"#;
        // Later pages may repeat the parent; it is kept once.
        let page2 = r#"{"ok":true,"messages":[
            {"ts":"1.0","text":"parent","thread_ts":"1.0","reply_count":2},
            {"ts":"3.0","text":"b","thread_ts":"1.0"}]}"#;
        let api = api(FakeHttp::with(&[page1, page2]));
        let messages = api.replies("C1", "1.0").unwrap();
        let texts: Vec<&str> = messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["parent", "a", "b"]);
        let calls = api.http.calls.borrow();
        assert_eq!(
            calls[0].url,
            "https://acme.slack.com/api/conversations.replies"
        );
        assert!(calls[0].params.contains(&("ts".into(), "1.0".into())));
        assert!(calls[1].params.contains(&("cursor".into(), "c2".into())));
    }

    #[test]
    fn post_message_sends_channel_text_and_thread() {
        let api = api(FakeHttp::with(&[r#"{"ok":true}"#]));
        api.post_message("D1", "hello", Some("1.0")).unwrap();
        let call = api.http.last();
        assert_eq!(call.url, "https://acme.slack.com/api/chat.postMessage");
        assert!(call.params.contains(&("channel".into(), "D1".into())));
        assert!(call.params.contains(&("text".into(), "hello".into())));
        assert!(call.params.contains(&("thread_ts".into(), "1.0".into())));
    }

    #[test]
    fn post_message_without_thread_omits_it() {
        let api = api(FakeHttp::with(&[r#"{"ok":true}"#]));
        api.post_message("D1", "hello", None).unwrap();
        let call = api.http.last();
        assert!(!call.params.iter().any(|(k, _)| k == "thread_ts"));
    }

    #[test]
    fn mark_reports_slack_failure() {
        let api = api(FakeHttp::with(&[
            r#"{"ok":false,"error":"channel_not_found"}"#,
        ]));
        assert!(api.mark("D1", "1.0").is_err());
    }

    #[test]
    fn messages_from_pages_through_search_until_the_limit() {
        let page = |n: u32, texts: &[&str]| {
            let matches: Vec<String> = texts
                .iter()
                .map(|t| format!(r#"{{"text":"{t}"}}"#))
                .collect();
            format!(
                r#"{{"ok":true,"messages":{{"paging":{{"page":{n},"pages":3}},"matches":[{}]}}}}"#,
                matches.join(",")
            )
        };
        let (one, two) = (page(1, &["a", "b"]), page(2, &["c", "d"]));
        let api = SlackApi::new(FakeHttp::with(&[&one, &two]), creds());
        assert_eq!(api.messages_from("U1", 3).unwrap(), ["a", "b", "c"]);
        let call = api.http.last();
        assert!(call.url.ends_with("/api/search.messages"));
        let param = |k: &str| {
            call.params
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(param("query").as_deref(), Some("from:<@U1>"));
        assert_eq!(param("page").as_deref(), Some("2"));
        assert_eq!(param("sort").as_deref(), Some("timestamp"));
    }
}
