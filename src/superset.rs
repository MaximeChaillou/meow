//! The `superset` CLI, for workspaces and coding agents on this machine.
//!
//! Every call runs `superset … --json --local` and names its workspace and
//! terminal explicitly: Meow started from a Superset terminal inherits
//! `SUPERSET_WORKSPACE_ID`, which some commands would otherwise default to.
//! Values go as `--option=value`: the CLI takes a separate value starting with
//! `-` (a prompt opening on a bullet list) for a missing one.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::claude::find_in_path;

/// Creating a worktree can run the project's setup script.
const CREATE_TIMEOUT: Duration = Duration::from_secs(180);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const ERROR_TAIL_LEN: usize = 400;
/// How long to wait for the rest of the output once the command exited: a
/// process it left behind may hold its pipes open.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// A project registered on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Its remote, when it has one.
    pub repo: Option<String>,
    pub path: Option<String>,
}

/// A workspace Superset created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub branch: String,
}

/// Where the `superset` command is, and the PATH it runs with.
#[derive(Clone, Debug)]
pub struct Superset {
    pub binary: PathBuf,
    pub path_var: String,
}

impl Superset {
    /// `MEOW_SUPERSET_BIN`, else `superset` on `path_var`, else the shim the
    /// desktop app installs in `~/.superset/bin`.
    pub fn discover(path_var: &str) -> Result<Self, String> {
        let installed = std::env::var_os("HOME")
            .map(|home| Path::new(&home).join(".superset/bin/superset"))
            .filter(|path| path.is_file());
        let binary = std::env::var_os("MEOW_SUPERSET_BIN")
            .map(PathBuf::from)
            .or_else(|| find_in_path("superset", path_var))
            .or(installed)
            .ok_or("Could not find the `superset` command. Is Superset installed?")?;
        Ok(Self {
            binary,
            path_var: path_var.to_string(),
        })
    }

    pub fn projects(&self) -> Result<Vec<Project>, String> {
        parse_projects(&self.call("projects list", &[], CALL_TIMEOUT)?)
    }

    /// A new worktree of `project_id` on `branch`.
    pub fn create_workspace(
        &self,
        project_id: &str,
        name: &str,
        branch: &str,
    ) -> Result<Workspace, String> {
        let args = [
            option("project", project_id),
            option("name", name),
            option("branch", branch),
        ];
        parse_workspace(&self.call("workspaces create", &args, CREATE_TIMEOUT)?)
    }

    /// Shows the workspace in the desktop app, which does not list it otherwise.
    pub fn open_workspace(&self, workspace: &str) -> Result<(), String> {
        self.call("workspaces open", &[workspace.to_string()], CALL_TIMEOUT)
            .map(|_| ())
    }

    /// Starts `agent` (a preset id such as `claude`) in `workspace`; returns
    /// the terminal it runs in.
    pub fn start_agent(
        &self,
        workspace: &str,
        agent: &str,
        prompt: &str,
    ) -> Result<String, String> {
        let args = [
            option("workspace", workspace),
            option("agent", agent),
            option("prompt", prompt),
        ];
        parse_agent(&self.call("agents create", &args, CALL_TIMEOUT)?)
    }

    /// The agent's transcript so far, as `User:` and `Assistant:` turns.
    pub fn read_agent(&self, workspace: &str, terminal: &str) -> Result<String, String> {
        let args = [option("workspace", workspace), option("terminal", terminal)];
        parse_transcript(&self.call("agents read", &args, CALL_TIMEOUT)?)
    }

    /// Runs `superset <subcommand> <args> --json --local`.
    fn call(&self, subcommand: &str, args: &[String], timeout: Duration) -> Result<Value, String> {
        let mut command = Command::new(&self.binary);
        command
            .args(subcommand.split(' '))
            .args(args)
            .args(["--json", "--local"])
            .env("PATH", &self.path_var)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let what = format!("superset {subcommand}");
        let output = run_with_timeout(command, timeout).map_err(|e| format!("{what}: {e}"))?;
        if !output.success {
            let detail = tail(output.stderr.trim());
            let detail = if detail.is_empty() {
                tail(output.stdout.trim())
            } else {
                detail
            };
            return Err(format!("{what} failed: {detail}"));
        }
        serde_json::from_str(output.stdout.trim())
            .map_err(|e| format!("{what} returned unexpected output: {e}"))
    }
}

struct Output {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Runs `command`, killing it if it outlives `timeout`.
fn run_with_timeout(mut command: Command, timeout: Duration) -> Result<Output, String> {
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start: {e}"))?;
    let stdout = child.stdout.take().map(read_in_background);
    let stderr = child.stderr.take().map(read_in_background);
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("no answer after {}s", timeout.as_secs()));
        }
        thread::sleep(Duration::from_millis(50));
    };
    Ok(Output {
        success: status.success(),
        stdout: drain(stdout),
        stderr: drain(stderr),
    })
}

/// Reads `pipe` on a thread, passing each chunk on as it arrives.
fn read_in_background(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        while let Ok(n) = pipe.read(&mut buffer) {
            if n == 0 || tx.send(buffer[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    rx
}

/// What the command wrote: everything up to the pipe's end, or what arrived
/// within [`DRAIN_TIMEOUT`] when a leftover process keeps it open.
fn drain(chunks: Option<mpsc::Receiver<Vec<u8>>>) -> String {
    let Some(chunks) = chunks else {
        return String::new();
    };
    let deadline = Instant::now() + DRAIN_TIMEOUT;
    let mut bytes = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match chunks.recv_timeout(left) {
            Ok(chunk) => bytes.extend(chunk),
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn option(name: &str, value: &str) -> String {
    format!("--{name}={value}")
}

fn tail(text: &str) -> String {
    let count = text.chars().count();
    text.chars()
        .skip(count.saturating_sub(ERROR_TAIL_LEN))
        .collect()
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "-")
        .map(str::to_string)
}

fn parse_projects(value: &Value) -> Result<Vec<Project>, String> {
    let list = value
        .as_array()
        .ok_or("superset projects list did not return a list")?;
    Ok(list
        .iter()
        .filter_map(|project| {
            Some(Project {
                id: text_field(project, "id")?,
                name: text_field(project, "name")?,
                repo: text_field(project, "repo"),
                path: text_field(project, "path"),
            })
        })
        .collect())
}

fn parse_workspace(value: &Value) -> Result<Workspace, String> {
    let workspace = value.get("workspace").unwrap_or(value);
    let id =
        text_field(workspace, "id").ok_or("Superset did not say which workspace it created")?;
    Ok(Workspace {
        name: text_field(workspace, "name").unwrap_or_else(|| id.clone()),
        branch: text_field(workspace, "branch").unwrap_or_default(),
        id,
    })
}

fn parse_agent(value: &Value) -> Result<String, String> {
    match text_field(value, "kind").as_deref() {
        Some("terminal") | None => {}
        Some(kind) => return Err(format!("Superset started a {kind} session, not a terminal")),
    }
    text_field(value, "sessionId").ok_or_else(|| "Superset did not say where the agent runs".into())
}

fn parse_transcript(value: &Value) -> Result<String, String> {
    value
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "superset agents read returned no transcript".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn projects_skip_entries_without_id_and_blank_remotes() {
        let value = json!([
            {"name": "Lazy", "repo": "-", "path": "/p/Lazy", "id": "p1"},
            {"name": "api", "repo": "https://github.com/x/api", "path": "/p/api", "id": "p2"},
            {"name": "broken"}
        ]);
        let projects = parse_projects(&value).unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].repo, None);
        assert_eq!(
            projects[1].repo.as_deref(),
            Some("https://github.com/x/api")
        );
        assert_eq!(projects[1].path.as_deref(), Some("/p/api"));
        assert!(parse_projects(&json!({"error": "x"})).is_err());
    }

    #[test]
    fn created_workspace_is_read_from_its_envelope() {
        let value = json!({
            "workspace": {"id": "w1", "name": "Fix export", "branch": "meow/fix-export"},
            "alreadyExists": false
        });
        assert_eq!(
            parse_workspace(&value).unwrap(),
            Workspace {
                id: "w1".into(),
                name: "Fix export".into(),
                branch: "meow/fix-export".into()
            }
        );
        assert!(parse_workspace(&json!({"workspace": {}})).is_err());
    }

    #[test]
    fn agent_must_run_in_a_terminal() {
        let started = json!({"kind": "terminal", "sessionId": "t1", "label": "Claude"});
        assert_eq!(parse_agent(&started).unwrap(), "t1");
        assert!(parse_agent(&json!({"kind": "chat", "sessionId": "c1"})).is_err());
        assert!(parse_agent(&json!({"kind": "terminal"})).is_err());
    }

    #[test]
    fn transcript_is_the_text_field() {
        let value = json!({"text": "User: hi\n\nAssistant: hello", "harness": "claude"});
        assert_eq!(
            parse_transcript(&value).unwrap(),
            "User: hi\n\nAssistant: hello"
        );
        assert!(parse_transcript(&json!({})).is_err());
    }

    fn fake_superset(name: &str, body: &str) -> (Superset, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("meow-superset-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("superset");
        let body = format!("#!/bin/sh\ncd \"$(dirname \"$0\")\"\n{body}\n");
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let superset = Superset {
            binary: script,
            path_var: "/bin:/usr/bin".into(),
        };
        (superset, dir)
    }

    #[test]
    fn calls_pass_json_and_local_and_parse_stdout() {
        let (superset, dir) = fake_superset(
            "args",
            "echo \"$@\" > args.txt\necho '{\"kind\":\"terminal\",\"sessionId\":\"t9\"}'",
        );
        let terminal = superset.start_agent("w1", "claude", "fix it").unwrap();
        assert_eq!(terminal, "t9");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn arguments_reach_the_cli_in_order() {
        let (superset, dir) = fake_superset("order", "printf '{\"text\":\"%s\"}' \"$*\"");
        let text = superset.read_agent("w1", "t1").unwrap();
        assert_eq!(
            text,
            "agents read --workspace=w1 --terminal=t1 --json --local"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failures_report_stderr() {
        let (superset, dir) = fake_superset("fail", "echo 'No active organization' >&2\nexit 1");
        let error = superset.projects().unwrap_err();
        assert!(error.contains("superset projects list failed"), "{error}");
        assert!(error.contains("No active organization"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn non_json_output_is_an_error() {
        let (superset, dir) = fake_superset("garbage", "echo 'Created workspace'");
        let error = superset.open_workspace("w1").unwrap_err();
        assert!(error.contains("unexpected output"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn values_starting_with_a_dash_stay_attached_to_their_option() {
        let (superset, dir) = fake_superset(
            "dash",
            "for a in \"$@\"; do echo \"[$a]\"; done > args.txt\n\
             echo '{\"kind\":\"terminal\",\"sessionId\":\"t1\"}'",
        );
        superset
            .start_agent("w1", "claude", "- fix\n- test")
            .unwrap();
        let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
        assert!(args.contains("[--prompt=- fix\n- test]"), "{args}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn output_held_open_by_a_leftover_process_does_not_hang() {
        let (superset, dir) = fake_superset("leftover", "echo '{}'\nsleep 30 &");
        let started = Instant::now();
        superset.open_workspace("w1").unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_hung_call_is_killed() {
        let (superset, dir) = fake_superset("hang", "sleep 5");
        let mut command = Command::new(&superset.binary);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let started = Instant::now();
        let error = run_with_timeout(command, Duration::from_millis(200))
            .err()
            .unwrap();
        assert!(error.contains("no answer"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(3));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
