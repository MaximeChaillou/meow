//! What tells Claude it runs inside Meow, and lets it see Meow's data.
//!
//! Chat runs get a system prompt describing Meow, and Meow's own binary as an
//! MCP server (`meow --mcp`): read-only tools over the user's Slack (through
//! Meow's session), tasks, activity, quick action results and scheduled tasks.

mod local;
mod slack;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::mcp::{self, ToolSpec, Toolbox};
use crate::slack::api::SlackApi;
use crate::slack::auth::Credentials;
use crate::slack::cache::{self, SlackCache};
use crate::slack::http::{Http, Web};
use crate::slack::keychain;
use local::{LocalTools, Paths};
use slack::SlackTools;

/// The argument that makes the binary serve MCP on stdio instead of the app.
pub const MCP_FLAG: &str = "--mcp";
/// The server's name, so its tools are `mcp__meow__<tool>`.
pub const SERVER_NAME: &str = "meow";

/// The `--mcp-config` value naming `exe` as the `meow` server.
pub fn mcp_config(exe: &Path) -> String {
    json!({
        "mcpServers": {
            SERVER_NAME: { "type": "stdio", "command": exe, "args": [MCP_FLAG] }
        }
    })
    .to_string()
}

/// The tool sending a Slack message as the user.
pub const SLACK_SEND: &str = "slack_send_message";
/// The tool leaving a Slack message as the user's draft, unsent.
pub const SLACK_DRAFT: &str = "slack_draft";

/// The Slack connector's tools that post or draft a message: Meow's own
/// [`SLACK_SEND`] and [`SLACK_DRAFT`] take their place, so every message a
/// Claude session sends goes through the Slack session Meow signed in with.
const CONNECTOR_SLACK_WRITES: &[&str] = &[
    "mcp__claude_ai_Slack__slack_schedule_message",
    "mcp__claude_ai_Slack__slack_send_message",
    "mcp__claude_ai_Slack__slack_send_message_draft",
];

/// The full name Claude calls the server's tool `name` by.
pub fn tool_name(name: &str) -> String {
    format!("mcp__{SERVER_NAME}__{name}")
}

/// The permission rules for the tools Claude may use without asking: those
/// that only read and, in a run that may act, leaving a draft, which sends
/// nothing. Sending a message goes through the session's permission mode
/// (and Autopilot's guard).
pub fn allowed_tools(acts: bool) -> String {
    specs()
        .iter()
        .filter(|spec| spec.read_only || (acts && spec.name == SLACK_DRAFT))
        .map(|spec| tool_name(spec.name))
        .collect::<Vec<_>>()
        .join(",")
}

/// The tools removed from every session given Meow's tools: the connector's
/// Slack writes, and in a read-only run Meow's own as well.
pub fn disallowed_tools(acts: bool) -> String {
    let own = specs()
        .into_iter()
        .filter(|spec| !acts && !spec.read_only)
        .map(|spec| tool_name(spec.name));
    CONNECTOR_SLACK_WRITES
        .iter()
        .map(|tool| tool.to_string())
        .chain(own)
        .collect::<Vec<_>>()
        .join(",")
}

/// Whether the server's tool `name` only reads.
pub fn only_reads(name: &str) -> bool {
    specs()
        .iter()
        .any(|spec| spec.name == name && spec.read_only)
}

/// Appended to Claude's system prompt in chat runs.
pub fn system_prompt() -> String {
    format!(
        "You are the assistant built into Meow, a macOS app: a pixel-art cat that walks along \
         the bottom of the user's screen. The user reaches you through Meow: its chat bubble, \
         a task's conversation, a quick action (like Today's planning) or a scheduled task. \
         Meow also has its own pages: Tasks, Activity, Scheduled and a live Slack client.\n\n\
         Meow's own data is available through the `{SERVER_NAME}` MCP tools \
         (mcp__{SERVER_NAME}__*):\n\
         - Slack, through the user's own Slack session in Meow: slack_conversations, \
         slack_history, slack_thread and slack_search read; {SLACK_SEND} sends a message as \
         the user and {SLACK_DRAFT} leaves one as their draft, unsent (both answer in a \
         thread with `thread_ts`). Prefer them for anything about the user's Slack messages, \
         DMs, channels or threads, and send or draft every Slack message with them, never \
         through another Slack connector.\n\
         - meow_tasks (the task list), meow_activity (what happened while the user was away), \
         meow_quick_actions (quick actions and their last results, today's plan among them) \
         and meow_scheduled (scheduled tasks and watches).\n\
         When the user mentions their messages, a conversation, their tasks, activity, plan \
         or anything shown in Meow, look it up with these tools rather than saying you cannot \
         see it. All but {SLACK_SEND} and {SLACK_DRAFT} only read."
    )
}

/// Serves Meow's tools on stdio until Claude closes it.
pub fn serve() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let cache = cache::default_path()
        .map(|path| SlackCache::load(&path).unwrap_or_default())
        .unwrap_or_default();
    let toolbox = MeowTools {
        slack: SlackTools::new(cache, Box::new(connect), now),
        local: LocalTools::new(Paths::standard(), now),
    };
    let stdin = std::io::stdin();
    mcp::serve(&toolbox, stdin.lock(), std::io::stdout());
}

/// The session Meow signed in with: the environment's, else the Keychain's.
fn connect() -> Result<SlackApi<Web>, String> {
    let creds = match env_credentials() {
        Some(creds) => creds,
        None => keychain::load()?.ok_or("The user is not signed in to Slack in Meow.")?,
    };
    Ok(SlackApi::new(Web::new(), creds))
}

fn env_credentials() -> Option<Credentials> {
    Some(Credentials {
        token: std::env::var("MEOW_SLACK_TOKEN").ok()?,
        cookie: std::env::var("MEOW_SLACK_COOKIE").ok()?,
        domain: std::env::var("MEOW_SLACK_DOMAIN").ok()?,
        team_id: String::new(),
        user_id: String::new(),
    })
}

struct MeowTools<H: Http> {
    slack: SlackTools<H>,
    local: LocalTools,
}

impl<H: Http> Toolbox for MeowTools<H> {
    fn tools(&self) -> Vec<ToolSpec> {
        specs()
    }

    fn call(&self, name: &str, args: &Value) -> Result<String, String> {
        match name {
            "slack_conversations" => self.slack.conversations(args),
            "slack_history" => self.slack.history(args),
            "slack_thread" => self.slack.thread(args),
            "slack_search" => self.slack.search(args),
            SLACK_SEND => self.slack.send(args),
            SLACK_DRAFT => self.slack.draft(args),
            "meow_tasks" => self.local.tasks(args),
            "meow_activity" => self.local.activity(args),
            "meow_quick_actions" => self.local.quick_actions(args),
            "meow_scheduled" => self.local.scheduled(),
            other => Err(format!("Unknown tool {other}.")),
        }
    }
}

fn specs() -> Vec<ToolSpec> {
    let channel = json!({
        "type": "string",
        "description": "Conversation id, #channel, channel name, or the person's name for a DM"
    });
    vec![
        ToolSpec {
            name: "slack_conversations",
            description: "The user's Slack DMs, group DMs and channels, most recent first, \
                          with their ids and whether they have unread messages or mentions.",
            input_schema: object(
                json!({
                    "unread_only": { "type": "boolean", "description": "Only unread ones" },
                    "limit": { "type": "integer", "description": "At most this many (default 50)" }
                }),
                &[],
            ),
            read_only: true,
        },
        ToolSpec {
            name: "slack_history",
            description: "The latest messages of a Slack conversation, oldest first, with each \
                          message's ts and whether it starts a thread.",
            input_schema: object(
                json!({
                    "channel": channel,
                    "limit": { "type": "integer", "description": "Messages to read (default 30, max 200)" }
                }),
                &["channel"],
            ),
            read_only: true,
        },
        ToolSpec {
            name: "slack_thread",
            description: "A Slack thread: its first message, then every reply.",
            input_schema: object(
                json!({
                    "channel": channel,
                    "thread_ts": { "type": "string", "description": "ts of the thread's first message" }
                }),
                &["channel", "thread_ts"],
            ),
            read_only: true,
        },
        ToolSpec {
            name: "slack_search",
            description: "Searches the user's Slack messages, newest first. Supports Slack's \
                          search modifiers, e.g. `from:@ana in:#eng after:2026-10-01 release`.",
            input_schema: object(
                json!({
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "description": "At most this many (default 20, max 100)" }
                }),
                &["query"],
            ),
            read_only: true,
        },
        ToolSpec {
            name: SLACK_SEND,
            description: "Sends a Slack message as the user, in a conversation or (with \
                          thread_ts) as a reply in its thread.",
            input_schema: message_schema(&channel),
            read_only: false,
        },
        ToolSpec {
            name: SLACK_DRAFT,
            description: "Leaves a Slack message as the user's draft, unsent: it waits in \
                          their composer for that conversation or (with thread_ts) thread.",
            input_schema: message_schema(&channel),
            read_only: false,
        },
        ToolSpec {
            name: "meow_tasks",
            description: "The user's task list in Meow, open tasks grouped by source \
                          (GitHub, Linear, Slack, their own…).",
            input_schema: object(
                json!({
                    "include_done": { "type": "boolean", "description": "Also list recently done tasks" }
                }),
                &[],
            ),
            read_only: true,
        },
        ToolSpec {
            name: "meow_activity",
            description: "Meow's Activity page, newest first: Slack DMs and mentions, calendar \
                          changes, tasks found or resolved, scheduled tasks that ran.",
            input_schema: object(
                json!({
                    "limit": { "type": "integer", "description": "At most this many (default 30)" }
                }),
                &[],
            ),
            read_only: true,
        },
        ToolSpec {
            name: "meow_quick_actions",
            description: "Meow's quick actions and when each last ran; with a title, that \
                          action's last result (for Today's planning, the day plan).",
            input_schema: object(
                json!({
                    "title": { "type": "string", "description": "A quick action's title" }
                }),
                &[],
            ),
            read_only: true,
        },
        ToolSpec {
            name: "meow_scheduled",
            description: "The user's scheduled tasks and watches in Meow, active and recently \
                          finished, with their last result.",
            input_schema: object(json!({}), &[]),
            read_only: true,
        },
    ]
}

/// What a message tool takes: where it goes, what it says, and the thread.
fn message_schema(channel: &Value) -> Value {
    object(
        json!({
            "channel": channel,
            "text": { "type": "string", "description": "What the message says" },
            "thread_ts": { "type": "string", "description": "ts of the thread's first message, to reply in it" }
        }),
        &["channel", "text"],
    )
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

/// A string argument the tool cannot do without.
fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Missing \"{key}\"."))
}

/// A count argument, `default` when absent, kept within 1..=`max`.
fn int_arg(args: &Value, key: &str, default: u64, max: u64) -> u64 {
    args.get(key)
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .clamp(1, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_runs_this_binary_as_the_meow_server() {
        let config: Value = serde_json::from_str(&mcp_config(Path::new(
            "/Applications/Meow.app/Contents/MacOS/meow",
        )))
        .unwrap();
        let server = &config["mcpServers"]["meow"];
        assert_eq!(
            server["command"],
            "/Applications/Meow.app/Contents/MacOS/meow"
        );
        assert_eq!(server["args"][0], MCP_FLAG);
    }

    #[test]
    fn only_reading_and_drafting_are_allowed_without_asking() {
        let read_only = allowed_tools(false);
        assert!(read_only.contains("mcp__meow__slack_history"));
        assert!(!read_only.contains("slack_draft"));
        let allowed: Vec<String> = allowed_tools(true).split(',').map(str::to_string).collect();
        assert!(allowed.contains(&"mcp__meow__slack_history".to_string()));
        assert!(allowed.contains(&"mcp__meow__meow_tasks".to_string()));
        assert!(allowed.contains(&"mcp__meow__slack_draft".to_string()));
        assert!(!allowed.contains(&"mcp__meow__slack_send_message".to_string()));
        assert!(only_reads("slack_search"));
        assert!(!only_reads(SLACK_SEND));
        assert!(!only_reads(SLACK_DRAFT));
        assert!(!only_reads("nope"));
    }

    #[test]
    fn the_slack_connector_cannot_send_beside_meow() {
        let disallowed = disallowed_tools(true);
        assert!(disallowed.contains("mcp__claude_ai_Slack__slack_send_message,"));
        assert!(disallowed.contains("mcp__claude_ai_Slack__slack_send_message_draft"));
        assert!(disallowed.contains("mcp__claude_ai_Slack__slack_schedule_message"));
        assert!(!disallowed.contains("read"));
        assert!(!disallowed.contains("mcp__meow"));
        let read_only = disallowed_tools(false);
        assert!(read_only.contains("mcp__meow__slack_send_message"));
        assert!(read_only.contains("mcp__meow__slack_draft"));
        assert!(!read_only.contains("mcp__meow__slack_history"));
    }

    #[test]
    fn every_listed_tool_is_dispatched() {
        let tools = MeowTools::<Web> {
            slack: SlackTools::new(SlackCache::default(), Box::new(|| Err("offline".into())), 0),
            local: LocalTools::new(Paths::default(), 0),
        };
        for spec in specs() {
            let result = tools.call(spec.name, &json!({}));
            assert!(
                !matches!(&result, Err(e) if e.starts_with("Unknown tool")),
                "{} is not dispatched",
                spec.name
            );
        }
        assert!(tools.call("nope", &json!({})).is_err());
    }

    #[test]
    fn the_prompt_names_every_tool() {
        let prompt = system_prompt();
        for spec in specs() {
            assert!(prompt.contains(spec.name), "{} is not described", spec.name);
        }
    }

    #[test]
    fn count_arguments_stay_in_range() {
        assert_eq!(int_arg(&json!({}), "limit", 30, 200), 30);
        assert_eq!(int_arg(&json!({ "limit": 0 }), "limit", 30, 200), 1);
        assert_eq!(int_arg(&json!({ "limit": 999 }), "limit", 30, 200), 200);
        assert!(str_arg(&json!({ "q": "  " }), "q").is_err());
    }
}
