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

/// The permission rule allowing every tool of the server; all only read.
pub fn allowed_tools() -> String {
    format!("mcp__{SERVER_NAME}")
}

/// Appended to Claude's system prompt in chat runs.
pub fn system_prompt() -> String {
    format!(
        "You are the assistant built into Meow, a macOS app: a pixel-art cat that walks along \
         the bottom of the user's screen. The user reaches you through Meow: its chat bubble, \
         a task's conversation, a quick action (like Today's planning) or a scheduled task. \
         Meow also has its own pages: Tasks, Activity, Scheduled and a live Slack client.\n\n\
         Meow's own data is available through the `{SERVER_NAME}` MCP tools \
         (mcp__{SERVER_NAME}__*), all read-only:\n\
         - Slack, through the user's own Slack session in Meow: slack_conversations, \
         slack_history, slack_thread, slack_search. Prefer them for anything about the user's \
         Slack messages, DMs, channels or threads.\n\
         - meow_tasks (the task list), meow_activity (what happened while the user was away), \
         meow_quick_actions (quick actions and their last results, today's plan among them) \
         and meow_scheduled (scheduled tasks and watches).\n\
         When the user mentions their messages, a conversation, their tasks, activity, plan \
         or anything shown in Meow, look it up with these tools rather than saying you cannot \
         see it. They cannot send or change anything: the user replies on Slack from Meow's \
         Slack page."
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
        },
        ToolSpec {
            name: "meow_scheduled",
            description: "The user's scheduled tasks and watches in Meow, active and recently \
                          finished, with their last result.",
            input_schema: object(json!({}), &[]),
        },
    ]
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
        assert_eq!(allowed_tools(), "mcp__meow");
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
