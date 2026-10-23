//! The tools of Meow's MCP server that read what Meow keeps on disk: tasks,
//! the activity log, quick action results (the day plan among them) and
//! scheduled tasks. Each call reads the files afresh, so it sees what the app
//! saved last.

use std::path::PathBuf;

use serde_json::Value;

use super::{int_arg, str_arg};
use crate::action_results::{self, ActionResults};
use crate::actions;
use crate::activity::{self, ActivityLog};
use crate::clock;
use crate::routines::{self, Routines};
use crate::tasks::{self, Task, TaskList};

const DEFAULT_ACTIVITY: u64 = 30;
const MAX_ACTIVITY: u64 = 200;
const DONE_SHOWN: usize = 20;

/// Where each file lives; tests point them at a temporary folder.
#[derive(Clone, Debug, Default)]
pub struct Paths {
    pub tasks: Option<PathBuf>,
    pub activity: Option<PathBuf>,
    pub actions: Option<PathBuf>,
    pub action_results: Option<PathBuf>,
    pub routines: Option<PathBuf>,
}

impl Paths {
    pub fn standard() -> Self {
        Self {
            tasks: tasks::default_path(),
            activity: activity::default_path(),
            actions: actions::config_path(),
            action_results: action_results::default_path(),
            routines: routines::default_path(),
        }
    }
}

pub struct LocalTools {
    paths: Paths,
    now: u64,
}

impl LocalTools {
    pub fn new(paths: Paths, now: u64) -> Self {
        Self { paths, now }
    }

    pub fn tasks(&self, args: &Value) -> Result<String, String> {
        let list = load(&self.paths.tasks, TaskList::load)?;
        let include_done = args.get("include_done").and_then(Value::as_bool) == Some(true);
        let mut sections: Vec<String> = list
            .groups()
            .iter()
            .map(|group| {
                let items: Vec<String> = group.tasks.iter().map(|t| task_line(t)).collect();
                format!("{}:\n{}", group.source, items.join("\n"))
            })
            .collect();
        if sections.is_empty() {
            sections.push("No open tasks.".to_string());
        }
        if include_done {
            let done: Vec<String> = list
                .completed()
                .into_iter()
                .take(DONE_SHOWN)
                .map(|t| {
                    let at = t.done_at.map(|at| clock::when_label(at, self.now));
                    format!("{} (done {})", task_line(t), at.unwrap_or_default())
                })
                .collect();
            if !done.is_empty() {
                sections.push(format!("Recently done:\n{}", done.join("\n")));
            }
        }
        Ok(sections.join("\n\n"))
    }

    pub fn activity(&self, args: &Value) -> Result<String, String> {
        let log = load(&self.paths.activity, ActivityLog::load)?;
        let limit = int_arg(args, "limit", DEFAULT_ACTIVITY, MAX_ACTIVITY) as usize;
        let lines: Vec<String> = log
            .items()
            .iter()
            .take(limit)
            .map(|item| {
                let detail = item
                    .detail
                    .as_deref()
                    .map(|d| format!(" — {d}"))
                    .unwrap_or_default();
                let unread = if item.read { "" } else { " (unread)" };
                format!(
                    "- [{}] {} {}{detail}{unread}",
                    clock::when_label(item.at, self.now),
                    item.kind.label(),
                    item.title
                )
            })
            .collect();
        if lines.is_empty() {
            return Ok("Nothing in Activity.".to_string());
        }
        Ok(lines.join("\n"))
    }

    /// Every quick action and when it last ran, or one action's last result.
    pub fn quick_actions(&self, args: &Value) -> Result<String, String> {
        let results = load(&self.paths.action_results, ActionResults::load)?;
        let titles: Vec<String> = match &self.paths.actions {
            Some(path) => actions::load(path)?,
            None => actions::defaults(),
        }
        .into_iter()
        .map(|action| action.title)
        .collect();
        if args.get("title").is_none() {
            let lines: Vec<String> = titles
                .iter()
                .map(|title| {
                    let ran = results.get(title).map_or("never ran".to_string(), |r| {
                        format!("last ran {}", action_results::ran_ago(r.ran_at, self.now))
                    });
                    format!("- {title} ({ran})")
                })
                .collect();
            return Ok(lines.join("\n"));
        }
        let wanted = str_arg(args, "title")?;
        let title = titles
            .iter()
            .find(|t| t.eq_ignore_ascii_case(wanted.trim()))
            .ok_or_else(|| format!("No quick action called \"{wanted}\"."))?;
        let Some(result) = results.get(title) else {
            return Ok(format!("{title} has never run."));
        };
        let plan = match &result.plan {
            Some(plan) => {
                let json = serde_json::to_string_pretty(plan).map_err(|e| e.to_string())?;
                format!("\n\nDay plan:\n{json}")
            }
            None => String::new(),
        };
        Ok(format!(
            "{title}, ran {}:\n\n{}{plan}",
            action_results::ran_ago(result.ran_at, self.now),
            result.answer
        ))
    }

    pub fn scheduled(&self) -> Result<String, String> {
        let routines = load(&self.paths.routines, Routines::load)?;
        Ok(routines.describe_for_claude(self.now))
    }
}

fn task_line(task: &Task) -> String {
    let check = if task.is_done() { "x" } else { " " };
    let url = task
        .url
        .as_deref()
        .map(|u| format!(" <{u}>"))
        .unwrap_or_default();
    format!("- [{check}] {}{url}", task.title)
}

/// The file at `path`, or an empty one when there is no home folder.
fn load<T: Default>(
    path: &Option<PathBuf>,
    read: impl Fn(&std::path::Path) -> Result<T, String>,
) -> Result<T, String> {
    path.as_deref()
        .map(read)
        .transpose()
        .map(Option::unwrap_or_default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{Kind, Note};
    use crate::tasks::FoundTask;
    use serde_json::json;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("meow-context-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tools_in(dir: &std::path::Path) -> LocalTools {
        let paths = Paths {
            tasks: Some(dir.join("tasks.json")),
            activity: Some(dir.join("activity.json")),
            actions: Some(dir.join("actions.json")),
            action_results: Some(dir.join("action-results.json")),
            routines: Some(dir.join("routines.json")),
        };
        LocalTools::new(paths, 1_000_000)
    }

    #[test]
    fn nothing_saved_yet_reads_as_empty() {
        let dir = temp_dir("empty");
        let tools = tools_in(&dir);
        assert_eq!(tools.tasks(&json!({})).unwrap(), "No open tasks.");
        assert_eq!(tools.activity(&json!({})).unwrap(), "Nothing in Activity.");
        assert!(tools.scheduled().unwrap().contains("no scheduled tasks"));
        let actions = tools.quick_actions(&json!({})).unwrap();
        assert!(
            actions.contains("Today's planning (never ran)"),
            "{actions}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tasks_are_grouped_by_source() {
        let dir = temp_dir("tasks");
        let mut list = TaskList::default();
        let found = FoundTask {
            title: "Review PR 12".into(),
            source: Some("GitHub".into()),
            url: Some("https://github.com/x/pull/12".into()),
        };
        list.merge(&[found], 1);
        list.add_manual("Call the bank", 1);
        list.save(&dir.join("tasks.json")).unwrap();
        let text = tools_in(&dir).tasks(&json!({})).unwrap();
        assert_eq!(
            text,
            "GitHub:\n- [ ] Review PR 12 <https://github.com/x/pull/12>\n\nMe:\n- [ ] Call the bank"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn activity_lists_newest_first_with_unread() {
        let dir = temp_dir("activity");
        let mut log = ActivityLog::default();
        let note = |title: &str| Note {
            kind: Kind::Slack,
            title: title.into(),
            detail: Some("hey".into()),
            link: None,
            sources: Vec::new(),
            key: None,
        };
        log.push(note("Ana"), 999_000);
        log.push(note("Bo"), 999_500);
        log.save(&dir.join("activity.json")).unwrap();
        let text = tools_in(&dir).activity(&json!({ "limit": 1 })).unwrap();
        assert!(text.ends_with("SLACK Bo — hey (unread)"), "{text}");
        assert_eq!(text.lines().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn one_quick_action_shows_its_last_answer() {
        let dir = temp_dir("actions");
        let mut results = ActionResults::default();
        results.record("Pending tasks", "3 PRs to review", None, None, 999_000);
        results.save(&dir.join("action-results.json")).unwrap();
        let tools = tools_in(&dir);
        let text = tools
            .quick_actions(&json!({ "title": "pending tasks" }))
            .unwrap();
        assert!(text.starts_with("Pending tasks, ran "), "{text}");
        assert!(text.ends_with("3 PRs to review"), "{text}");
        assert!(tools.quick_actions(&json!({ "title": "nope" })).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
