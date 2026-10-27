//! A minimal MCP server over stdio: newline-delimited JSON-RPC, tools only.
//!
//! Claude Code starts it as a subprocess (see [`crate::context`]) and calls
//! `initialize`, `tools/list` and `tools/call`. Everything else is answered
//! with "method not found", and notifications are ignored.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

/// Spoken when the client does not say which version it wants.
const PROTOCOL_VERSION: &str = "2025-06-18";
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const PARSE_ERROR: i64 = -32700;

/// One tool as listed to the client.
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema of its arguments.
    pub input_schema: Value,
    /// Whether it only reads; otherwise it acts (e.g. sends a message).
    pub read_only: bool,
}

/// What the server offers: its tools.
pub trait Toolbox {
    fn tools(&self) -> Vec<ToolSpec>;
    /// Runs tool `name`; the text is the result, or why it failed.
    fn call(&self, name: &str, args: &Value) -> Result<String, String>;
}

/// Answers requests from `input` on `output` until `input` ends.
pub fn serve(toolbox: &impl Toolbox, input: impl BufRead, mut output: impl Write) {
    for line in input.lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(toolbox, &line) {
            let sent = writeln!(output, "{reply}").and_then(|()| output.flush());
            if sent.is_err() {
                return;
            }
        }
    }
}

/// The reply to one JSON-RPC line, or `None` for a notification.
pub fn handle(toolbox: &impl Toolbox, line: &str) -> Option<String> {
    let Ok(request) = serde_json::from_str::<Value>(line) else {
        return Some(error(Value::Null, PARSE_ERROR, "invalid JSON").to_string());
    };
    let id = request.get("id").cloned()?;
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let reply = match method {
        "initialize" => success(id, initialize(&params)),
        "ping" => success(id, json!({})),
        "tools/list" => success(id, json!({ "tools": list(toolbox) })),
        "tools/call" => match params.get("name").and_then(Value::as_str) {
            Some(name) => {
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                success(id, call(toolbox, name, &args))
            }
            None => error(id, INVALID_PARAMS, "tools/call needs a tool name"),
        },
        other => error(id, METHOD_NOT_FOUND, &format!("unknown method {other}")),
    };
    Some(reply.to_string())
}

fn initialize(params: &Value) -> Value {
    let version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "meow", "version": env!("CARGO_PKG_VERSION") },
    })
}

fn list(toolbox: &impl Toolbox) -> Vec<Value> {
    toolbox
        .tools()
        .into_iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.input_schema,
                "annotations": { "readOnlyHint": tool.read_only },
            })
        })
        .collect()
}

/// A tool's failure is a result the model reads, not a protocol error.
fn call(toolbox: &impl Toolbox, name: &str, args: &Value) -> Value {
    let (text, is_error) = match toolbox.call(name, args) {
        Ok(text) => (text, false),
        Err(why) => (why, true),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl Toolbox for Echo {
        fn tools(&self) -> Vec<ToolSpec> {
            vec![
                ToolSpec {
                    name: "echo",
                    description: "Says it back",
                    input_schema: json!({ "type": "object" }),
                    read_only: true,
                },
                ToolSpec {
                    name: "shout",
                    description: "Says it to everyone",
                    input_schema: json!({ "type": "object" }),
                    read_only: false,
                },
            ]
        }

        fn call(&self, name: &str, args: &Value) -> Result<String, String> {
            match args.get("text").and_then(Value::as_str) {
                Some(text) if name == "echo" => Ok(text.to_string()),
                _ => Err("nothing to say".into()),
            }
        }
    }

    fn reply(line: &str) -> Value {
        serde_json::from_str(&handle(&Echo, line).expect("a reply")).unwrap()
    }

    #[test]
    fn initialize_echoes_the_version_and_offers_tools() {
        let r = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
        );
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        assert_eq!(r["result"]["serverInfo"]["name"], "meow");
    }

    #[test]
    fn notifications_get_no_reply() {
        assert_eq!(
            handle(
                &Echo,
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
            ),
            None
        );
    }

    #[test]
    fn tools_are_listed_with_whether_they_only_read() {
        let r = reply(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#);
        let tool = &r["result"]["tools"][0];
        assert_eq!(tool["name"], "echo");
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(
            r["result"]["tools"][1]["annotations"]["readOnlyHint"],
            false
        );
    }

    #[test]
    fn calls_return_text_and_failures_as_tool_errors() {
        let ok = reply(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}}"#,
        );
        assert_eq!(ok["result"]["content"][0]["text"], "hi");
        assert_eq!(ok["result"]["isError"], false);
        let failed =
            reply(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo"}}"#);
        assert_eq!(failed["result"]["isError"], true);
        assert_eq!(failed["result"]["content"][0]["text"], "nothing to say");
    }

    #[test]
    fn unknown_methods_and_bad_json_are_errors() {
        let unknown = reply(r#"{"jsonrpc":"2.0","id":4,"method":"resources/list"}"#);
        assert_eq!(unknown["error"]["code"], METHOD_NOT_FOUND);
        let bad = reply("{nope");
        assert_eq!(bad["error"]["code"], PARSE_ERROR);
    }

    #[test]
    fn serve_answers_each_request_line() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            "\n\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
            "\n"
        );
        let mut output = Vec::new();
        serve(&Echo, input.as_bytes(), &mut output);
        let lines: Vec<&str> = std::str::from_utf8(&output).unwrap().lines().collect();
        assert_eq!(lines.len(), 2);
    }
}
