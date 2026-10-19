//! Slack's domain types and the parsing of Web API response bodies into them.
//!
//! The Web API always replies with a JSON object carrying `ok`; on failure it
//! is `{ "ok": false, "error": "…" }`. Each parser here checks `ok` first and
//! surfaces the error code, so the transport layer only has to pass the body on.

use serde::{Deserialize, Serialize};

/// Why a response could not be turned into domain data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlackError {
    /// Slack replied `ok: false` with this error code (e.g. `invalid_auth`).
    Api(String),
    /// The body was not the JSON shape expected.
    Malformed(String),
}

impl std::fmt::Display for SlackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SlackError::Api(code) => write!(f, "Slack error: {code}"),
            SlackError::Malformed(why) => write!(f, "unexpected Slack response: {why}"),
        }
    }
}

impl std::error::Error for SlackError {}

/// A page of results, with the cursor to fetch the next one if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

/// The kinds of conversation Meow lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelKind {
    /// A direct message with one other person.
    Im,
    /// A group direct message.
    Mpim,
    /// A named channel.
    Channel,
}

/// One conversation in the sidebar list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub kind: ChannelKind,
    /// Slack's name; empty for a direct message (resolve it from [`Channel::user`]).
    pub name: String,
    /// The other person, for a direct message.
    pub user: Option<String>,
    /// Timestamp of the latest message, when Slack includes it.
    pub latest_ts: Option<String>,
    /// Whether it has messages the user has not read.
    pub unread: bool,
    /// Unread mentions of the user.
    pub mentions: u32,
}

/// One message in a conversation or thread.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Slack's `ts`, which is both the id and the ordering key.
    pub ts: String,
    pub user: Option<String>,
    /// Raw `mrkdwn`; render with [`super::format::to_markdown`].
    pub text: String,
    /// The thread this belongs to; `Some` on a reply or a threaded parent.
    pub thread_ts: Option<String>,
    /// Replies to this message, when it starts a thread.
    pub reply_count: u32,
    /// Names of attached files.
    pub files: Vec<String>,
    /// A one-line summary of the first attachment (link unfurl, bot card).
    pub attachment: Option<String>,
    /// The name an app or bot posted under, when there is no `user`.
    pub bot_name: Option<String>,
}

impl Message {
    /// Whether this message starts, or lives in, a thread.
    pub fn in_thread(&self) -> bool {
        self.thread_ts.is_some()
    }
}

/// A Slack user, enough to show a name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub real_name: String,
    pub display_name: String,
    /// Empty when the workspace hides it.
    pub email: String,
    /// IANA time zone, e.g. `Europe/Paris`; empty when unknown.
    pub tz: String,
}

impl User {
    /// The best name to show: display name, else real name, else the id.
    pub fn name(&self) -> &str {
        [&self.display_name, &self.real_name]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or(&self.id)
    }
}

/// The signed-in identity, from `auth.test`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthTest {
    pub user_id: String,
    pub team_id: String,
    pub user: String,
    pub team: String,
    pub url: String,
}

/// Parses an `auth.test` body.
pub fn parse_auth_test(body: &str) -> Result<AuthTest, SlackError> {
    let raw: AuthResponse = decode(body)?;
    ok(raw.ok, raw.error)?;
    Ok(AuthTest {
        user_id: raw.user_id,
        team_id: raw.team_id,
        user: raw.user,
        team: raw.team,
        url: raw.url,
    })
}

/// Parses a `users.conversations` / `conversations.list` body.
pub fn parse_channels(body: &str) -> Result<Page<Channel>, SlackError> {
    let raw: ChannelsResponse = decode(body)?;
    ok(raw.ok, raw.error)?;
    let items = raw.channels.into_iter().map(Channel::from).collect();
    Ok(Page {
        items,
        next_cursor: cursor(raw.response_metadata.next_cursor),
    })
}

/// Parses a `conversations.history` / `conversations.replies` body. Messages come
/// back oldest-first (Slack returns newest-first).
pub fn parse_messages(body: &str) -> Result<Page<Message>, SlackError> {
    let raw: HistoryResponse = decode(body)?;
    ok(raw.ok, raw.error)?;
    let mut items: Vec<Message> = raw
        .messages
        .into_iter()
        .filter(|m| !m.ts.is_empty() && !is_system_notice(m.subtype.as_deref()))
        .map(Message::from)
        .collect();
    items.reverse();
    Ok(Page {
        items,
        next_cursor: cursor(raw.response_metadata.next_cursor),
    })
}

/// Per-conversation activity from `client.counts`: what Slack's own sidebar
/// shows, for direct and group messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    pub id: String,
    pub latest: Option<String>,
    pub unread: bool,
    pub mentions: u32,
}

/// Parses a `client.counts` body into its direct and group conversations.
pub fn parse_counts(body: &str) -> Result<Vec<Activity>, SlackError> {
    let raw: CountsResponse = decode(body)?;
    ok(raw.ok, raw.error)?;
    Ok(raw
        .ims
        .into_iter()
        .chain(raw.mpims)
        .map(|count| Activity {
            id: count.id,
            latest: Some(count.latest).filter(|ts| !ts.is_empty()),
            unread: count.has_unreads,
            mentions: count.mention_count,
        })
        .collect())
}

/// Parses a `search.messages` body into the matching messages' text. The
/// next page's number stands in for a cursor.
pub fn parse_search(body: &str) -> Result<Page<String>, SlackError> {
    let raw: SearchResponse = decode(body)?;
    ok(raw.ok, raw.error)?;
    let paging = raw.messages.paging;
    let items = raw
        .messages
        .matches
        .into_iter()
        .map(|m| m.text)
        .filter(|text| !text.trim().is_empty())
        .collect();
    Ok(Page {
        items,
        next_cursor: (paging.page < paging.pages).then(|| (paging.page + 1).to_string()),
    })
}

/// Parses a `conversations.members` body into user ids.
pub fn parse_members(body: &str) -> Result<Page<String>, SlackError> {
    let raw: MembersResponse = decode(body)?;
    ok(raw.ok, raw.error)?;
    Ok(Page {
        items: raw.members,
        next_cursor: cursor(raw.response_metadata.next_cursor),
    })
}

/// Membership and settings notices ("joined", "set the topic"), which carry no
/// conversation.
fn is_system_notice(subtype: Option<&str>) -> bool {
    matches!(
        subtype,
        Some(
            "channel_join"
                | "channel_leave"
                | "group_join"
                | "group_leave"
                | "channel_topic"
                | "channel_purpose"
                | "channel_name"
                | "group_topic"
                | "group_purpose"
                | "group_name"
        )
    )
}

/// Parses a `users.info` body.
pub fn parse_user(body: &str) -> Result<User, SlackError> {
    let raw: UserResponse = decode(body)?;
    ok(raw.ok, raw.error)?;
    let user = raw
        .user
        .ok_or_else(|| SlackError::Malformed("no user in response".into()))?;
    Ok(User::from(user))
}

/// Confirms a bare `{ "ok": … }` body, as `chat.postMessage` and
/// `conversations.mark` return.
pub fn parse_ok(body: &str) -> Result<(), SlackError> {
    let raw: Envelope = decode(body)?;
    ok(raw.ok, raw.error)
}

fn decode<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, SlackError> {
    serde_json::from_str(body).map_err(|e| SlackError::Malformed(e.to_string()))
}

fn ok(is_ok: bool, error: Option<String>) -> Result<(), SlackError> {
    if is_ok {
        Ok(())
    } else {
        Err(SlackError::Api(error.unwrap_or_else(|| "unknown".into())))
    }
}

/// An empty cursor means there is no next page.
fn cursor(raw: String) -> Option<String> {
    (!raw.is_empty()).then_some(raw)
}

// --- Wire formats -----------------------------------------------------------

#[derive(Deserialize)]
struct Envelope {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize, Default)]
struct ResponseMetadata {
    #[serde(default)]
    next_cursor: String,
}

#[derive(Deserialize)]
struct AuthResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    team_id: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    team: String,
    #[serde(default)]
    url: String,
}

#[derive(Deserialize)]
struct ChannelsResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    channels: Vec<RawChannel>,
    #[serde(default)]
    response_metadata: ResponseMetadata,
}

#[derive(Deserialize)]
struct RawChannel {
    id: String,
    #[serde(default)]
    is_im: bool,
    #[serde(default)]
    is_mpim: bool,
    #[serde(default)]
    name: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    latest: Option<RawLatest>,
}

#[derive(Deserialize)]
struct RawLatest {
    #[serde(default)]
    ts: String,
}

impl From<RawChannel> for Channel {
    fn from(raw: RawChannel) -> Self {
        let kind = if raw.is_im {
            ChannelKind::Im
        } else if raw.is_mpim {
            ChannelKind::Mpim
        } else {
            ChannelKind::Channel
        };
        Channel {
            id: raw.id,
            kind,
            name: raw.name,
            user: raw.user,
            latest_ts: raw.latest.map(|l| l.ts).filter(|ts| !ts.is_empty()),
            unread: false,
            mentions: 0,
        }
    }
}

#[derive(Deserialize)]
struct HistoryResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    messages: Vec<RawMessage>,
    #[serde(default)]
    response_metadata: ResponseMetadata,
}

#[derive(Deserialize)]
struct RawMessage {
    #[serde(default)]
    ts: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    thread_ts: Option<String>,
    #[serde(default)]
    reply_count: u32,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    files: Vec<RawFile>,
    #[serde(default)]
    attachments: Vec<RawAttachment>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    bot_profile: Option<RawBotProfile>,
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
struct RawAttachment {
    #[serde(default)]
    title: String,
    #[serde(default)]
    fallback: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct RawBotProfile {
    #[serde(default)]
    name: String,
}

impl From<RawMessage> for Message {
    fn from(raw: RawMessage) -> Self {
        let files = raw
            .files
            .into_iter()
            .map(|file| first_non_empty([file.title, file.name]))
            .filter(|name| !name.is_empty())
            .collect();
        let attachment = raw
            .attachments
            .into_iter()
            .map(|a| first_non_empty([a.title, a.fallback, a.text]))
            .find(|summary| !summary.is_empty());
        let bot_name = raw
            .bot_profile
            .map(|bot| bot.name)
            .into_iter()
            .chain(raw.username)
            .find(|name| !name.is_empty());
        Message {
            ts: raw.ts,
            user: raw.user,
            text: raw.text,
            thread_ts: raw.thread_ts,
            reply_count: raw.reply_count,
            files,
            attachment,
            bot_name,
        }
    }
}

fn first_non_empty<const N: usize>(candidates: [String; N]) -> String {
    candidates
        .into_iter()
        .find(|s| !s.trim().is_empty())
        .unwrap_or_default()
}

#[derive(Deserialize)]
struct MembersResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    response_metadata: ResponseMetadata,
}

#[derive(Deserialize)]
struct SearchResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    messages: RawSearchMessages,
}

#[derive(Deserialize, Default)]
struct RawSearchMessages {
    #[serde(default)]
    matches: Vec<RawSearchMatch>,
    #[serde(default)]
    paging: RawPaging,
}

#[derive(Deserialize)]
struct RawSearchMatch {
    #[serde(default)]
    text: String,
}

#[derive(Deserialize, Default)]
struct RawPaging {
    #[serde(default)]
    page: u32,
    #[serde(default)]
    pages: u32,
}

#[derive(Deserialize)]
struct CountsResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    ims: Vec<RawCount>,
    #[serde(default)]
    mpims: Vec<RawCount>,
}

#[derive(Deserialize)]
struct RawCount {
    id: String,
    #[serde(default)]
    latest: String,
    #[serde(default)]
    has_unreads: bool,
    #[serde(default)]
    mention_count: u32,
}

#[derive(Deserialize)]
struct UserResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    user: Option<RawUser>,
}

#[derive(Deserialize)]
struct RawUser {
    id: String,
    #[serde(default)]
    real_name: String,
    #[serde(default)]
    tz: String,
    #[serde(default)]
    profile: RawProfile,
}

#[derive(Deserialize, Default)]
struct RawProfile {
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    real_name: String,
}

impl From<RawUser> for User {
    fn from(raw: RawUser) -> Self {
        // The profile's real name is the fuller one; fall back to the top-level.
        let real_name = if raw.profile.real_name.is_empty() {
            raw.real_name
        } else {
            raw.profile.real_name
        };
        User {
            id: raw.id,
            real_name,
            display_name: raw.profile.display_name,
            email: raw.profile.email,
            tz: raw.tz,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_is_surfaced() {
        let err = parse_auth_test(r#"{"ok":false,"error":"invalid_auth"}"#).unwrap_err();
        assert_eq!(err, SlackError::Api("invalid_auth".into()));
    }

    #[test]
    fn malformed_body_is_reported() {
        assert!(matches!(
            parse_messages("not json"),
            Err(SlackError::Malformed(_))
        ));
    }

    #[test]
    fn auth_test_is_parsed() {
        let body = r#"{"ok":true,"url":"https://x.slack.com/","team":"X",
            "user":"me","team_id":"T1","user_id":"U1"}"#;
        let auth = parse_auth_test(body).unwrap();
        assert_eq!(auth.user_id, "U1");
        assert_eq!(auth.team_id, "T1");
        assert_eq!(auth.team, "X");
    }

    #[test]
    fn channels_are_classified_by_kind() {
        let body = r#"{"ok":true,"channels":[
            {"id":"D1","is_im":true,"user":"U2","latest":{"ts":"1.2"}},
            {"id":"G1","is_mpim":true,"name":"mpdm-a--b--c"},
            {"id":"C1","is_channel":true,"name":"general"}
        ],"response_metadata":{"next_cursor":"next"}}"#;
        let page = parse_channels(body).unwrap();
        assert_eq!(page.next_cursor.as_deref(), Some("next"));
        assert_eq!(page.items[0].kind, ChannelKind::Im);
        assert_eq!(page.items[0].user.as_deref(), Some("U2"));
        assert_eq!(page.items[0].latest_ts.as_deref(), Some("1.2"));
        assert_eq!(page.items[1].kind, ChannelKind::Mpim);
        assert_eq!(page.items[2].kind, ChannelKind::Channel);
        assert_eq!(page.items[2].name, "general");
    }

    #[test]
    fn empty_cursor_is_none() {
        let page = parse_channels(r#"{"ok":true,"channels":[]}"#).unwrap();
        assert_eq!(page.next_cursor, None);
    }

    #[test]
    fn history_is_reversed_to_oldest_first() {
        let body = r#"{"ok":true,"messages":[
            {"ts":"3.0","user":"U1","text":"third"},
            {"ts":"2.0","user":"U2","text":"second"},
            {"ts":"1.0","user":"U1","text":"first"}
        ]}"#;
        let page = parse_messages(body).unwrap();
        let order: Vec<&str> = page.items.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(order, ["first", "second", "third"]);
    }

    #[test]
    fn thread_fields_are_captured() {
        let body = r#"{"ok":true,"messages":[
            {"ts":"1.0","user":"U1","text":"parent","thread_ts":"1.0","reply_count":2}
        ]}"#;
        let msg = &parse_messages(body).unwrap().items[0];
        assert!(msg.in_thread());
        assert_eq!(msg.reply_count, 2);
    }

    #[test]
    fn messages_without_ts_are_dropped() {
        let body = r#"{"ok":true,"messages":[{"text":"ghost"},{"ts":"1.0","text":"real"}]}"#;
        let page = parse_messages(body).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].text, "real");
    }

    #[test]
    fn system_notices_are_dropped() {
        let body = r#"{"ok":true,"messages":[
            {"ts":"2.0","subtype":"channel_join","user":"U1","text":"<@U1> has joined"},
            {"ts":"1.0","user":"U1","text":"hello"}
        ]}"#;
        let page = parse_messages(body).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].text, "hello");
    }

    #[test]
    fn files_attachments_and_bot_names_are_captured() {
        let body = r#"{"ok":true,"messages":[{"ts":"1.0","text":"",
            "files":[{"name":"a.png","title":"Screenshot"},{"name":"b.pdf"}],
            "attachments":[{"fallback":"","title":"","text":"Build passed"}],
            "bot_profile":{"name":"CI"}}]}"#;
        let msg = &parse_messages(body).unwrap().items[0];
        assert_eq!(msg.files, ["Screenshot", "b.pdf"]);
        assert_eq!(msg.attachment.as_deref(), Some("Build passed"));
        assert_eq!(msg.bot_name.as_deref(), Some("CI"));
    }

    #[test]
    fn counts_merge_direct_and_group_messages() {
        let body = r#"{"ok":true,
            "ims":[{"id":"D1","latest":"5.0","has_unreads":true,"mention_count":2}],
            "mpims":[{"id":"G1","latest":"","has_unreads":false,"mention_count":0}],
            "channels":[{"id":"C1","latest":"9.0","has_unreads":true,"mention_count":0}]}"#;
        let counts = parse_counts(body).unwrap();
        assert_eq!(
            counts,
            [
                Activity {
                    id: "D1".into(),
                    latest: Some("5.0".into()),
                    unread: true,
                    mentions: 2
                },
                Activity {
                    id: "G1".into(),
                    latest: None,
                    unread: false,
                    mentions: 0
                },
            ]
        );
    }

    #[test]
    fn members_are_listed_with_their_cursor() {
        let body = r#"{"ok":true,"members":["U1","U2"],"response_metadata":{"next_cursor":"c2"}}"#;
        let page = parse_members(body).unwrap();
        assert_eq!(page.items, ["U1", "U2"]);
        assert_eq!(page.next_cursor.as_deref(), Some("c2"));
    }

    #[test]
    fn search_yields_texts_and_the_next_page() {
        let body = r#"{"ok":true,"messages":{"total":250,
            "paging":{"count":100,"total":250,"page":1,"pages":3},
            "matches":[{"text":"on it"},{"text":"  "},{"text":"done"}]}}"#;
        let page = parse_search(body).unwrap();
        assert_eq!(page.items, ["on it", "done"]);
        assert_eq!(page.next_cursor.as_deref(), Some("2"));
        let last = r#"{"ok":true,"messages":{"paging":{"page":3,"pages":3},"matches":[]}}"#;
        assert_eq!(parse_search(last).unwrap().next_cursor, None);
    }

    #[test]
    fn user_name_prefers_display_then_real_then_id() {
        let display = User {
            id: "U1".into(),
            real_name: "Alice A".into(),
            display_name: "alice".into(),
            email: String::new(),
            tz: String::new(),
        };
        assert_eq!(display.name(), "alice");
        let real = User {
            display_name: String::new(),
            ..display.clone()
        };
        assert_eq!(real.name(), "Alice A");
        let bare = User {
            real_name: String::new(),
            ..real
        };
        assert_eq!(bare.name(), "U1");
    }

    #[test]
    fn user_is_parsed_from_profile() {
        let body = r#"{"ok":true,"user":{"id":"U1","real_name":"Alice Top","tz":"Europe/Paris",
            "profile":{"display_name":"alice","real_name":"Alice Profile","email":"alice@example.com"}}}"#;
        let user = parse_user(body).unwrap();
        assert_eq!(user.display_name, "alice");
        assert_eq!(user.real_name, "Alice Profile");
        assert_eq!(user.email, "alice@example.com");
        assert_eq!(user.tz, "Europe/Paris");
    }

    #[test]
    fn bare_ok_is_checked() {
        assert!(parse_ok(r#"{"ok":true}"#).is_ok());
        assert_eq!(
            parse_ok(r#"{"ok":false,"error":"msg_too_long"}"#).unwrap_err(),
            SlackError::Api("msg_too_long".into())
        );
    }
}
