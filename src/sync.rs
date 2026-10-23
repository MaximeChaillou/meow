//! The background check: Claude looks at the open tasks and at what is new
//! since the last check, then reports, in a fenced `meow-sync` JSON block,
//! which tasks are resolved and which new ones appeared.

use serde::Deserialize;

use crate::tasks::{FoundTask, Task};

pub const SYNC_FENCE: &str = "meow-sync";
/// How far back the very first check looks for new things.
const FIRST_LOOKBACK_HOURS: u64 = 24;
/// At most this many open tasks are sent, to keep the check small.
const MAX_CHECKED: usize = 40;
/// At most this many handled tasks are sent, so they are not reported again.
pub const MAX_HANDLED: usize = 40;
/// How Claude should judge whether I answered on Slack.
pub const SLACK_REPLY_RULE: &str = "On Slack, I have answered when I posted in the same DM or channel after that message, in its thread or not, or reacted to it.";

/// What a check found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct SyncReport {
    /// Ids of open tasks that are resolved at their source.
    #[serde(default)]
    pub done: Vec<String>,
    #[serde(default)]
    pub new: Vec<FoundTask>,
}

/// The prompt for one check. `handled` are tasks I already ticked off, never
/// to be reported again; `since` is the time of the last successful check.
pub fn prompt(open: &[&Task], handled: &[&Task], since: Option<u64>, now: u64) -> String {
    let tasks: Vec<_> = open
        .iter()
        .take(MAX_CHECKED)
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "title": t.title,
                "source": t.source,
                "url": t.url,
            })
        })
        .collect();
    let handled: Vec<_> = handled
        .iter()
        .take(MAX_HANDLED)
        .map(|t| serde_json::json!({"title": t.title, "source": t.source, "url": t.url}))
        .collect();
    let window = match since {
        Some(since) => format!(
            "since {} (UTC, {} minutes ago)",
            iso_time(since),
            now.saturating_sub(since) / 60
        ),
        None => format!("in the last {FIRST_LOOKBACK_HOURS} hours"),
    };
    format!(
        "Background check of my task list. Only read: do not send, post, reply, create or \
         modify anything. Use my connectors directly (Linear, GitHub, Slack, Gmail); do not \
         launch sub-agents. Keep it quick.\n\n\
         1. For each open task below, check whether it is resolved at its source: Linear issue \
         done or canceled, pull request merged, closed or already reviewed by me, Slack message \
         I have answered, email I have answered. {SLACK_REPLY_RULE} Only count it as resolved \
         when you are sure.\n\
         2. Find new things waiting on me {window}: Linear issues newly assigned to me, pull \
         requests newly requesting my review, Slack DMs or mentions I have not answered, emails \
         waiting for my answer. A search result shows the message, not what came after it: \
         before reporting a Slack message, read its conversation (and its thread) from that \
         message on, and leave it out if I answered there. {SLACK_REPLY_RULE} Skip anything in either list below, including the same message, \
         issue or pull request under another title or link: I have dealt with the handled ones \
         and do not want them back.\n\n\
         Open tasks (JSON):\n{}\n\n\
         Already handled, never report again (JSON):\n{}\n\n\
         Reply with only a fenced code block with the info string `{SYNC_FENCE}` holding \
         {{\"done\": [ids of resolved tasks], \"new\": [{{\"title\": short imperative title, \
         \"source\": \"Linear\"|\"GitHub\"|\"Slack\"|\"Gmail\", \"url\": link or null}}]}}. Give Slack messages \
         and emails their link, which later checks need to tell when they are resolved.",
        serde_json::Value::Array(tasks),
        serde_json::Value::Array(handled)
    )
}

/// Reads the `meow-sync` block out of Claude's answer.
pub fn parse(answer: &str) -> Result<SyncReport, String> {
    let opening = format!("```{SYNC_FENCE}");
    let start = answer
        .find(&opening)
        .ok_or("the check returned no result block")?
        + opening.len();
    let len = answer[start..]
        .find("```")
        .ok_or("the check's result block is not closed")?;
    serde_json::from_str(answer[start..start + len].trim())
        .map_err(|e| format!("the check's result is not valid JSON ({e})"))
}

/// `2026-10-19T09:30:00Z` for a Unix time, without pulling in a date crate.
fn iso_time(secs: u64) -> String {
    let days = secs / 86_400;
    let (h, m, s) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    // Civil-from-days (Howard Hinnant's algorithm), for dates after 1970.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskList;

    fn list() -> TaskList {
        let mut list = TaskList::default();
        list.merge(
            &[FoundTask {
                title: "Review PR 3172".into(),
                source: Some("GitHub".into()),
                url: Some("https://github.com/x/3172".into()),
            }],
            1,
        );
        list
    }

    #[test]
    fn prompt_lists_open_tasks_with_their_ids_and_is_read_only() {
        let list = list();
        let prompt = prompt(&list.display(), &[], None, 100);
        assert!(prompt.contains("\"id\":\"url:https://github.com/x/3172\""));
        assert!(prompt.contains("Only read"));
        assert!(prompt.contains("in the last 24 hours"));
        assert!(prompt.contains(SYNC_FENCE));
    }

    #[test]
    fn prompt_lists_handled_tasks_and_counts_replies_outside_threads() {
        let mut list = list();
        let id = list.display()[0].id.clone();
        list.set_done(&id, true, 2);
        let prompt = prompt(&[], &list.recently_done(MAX_HANDLED), None, 100);
        assert!(prompt.contains("never report again"));
        assert!(prompt.contains("Review PR 3172"));
        assert!(prompt.contains("in its thread or not"));
        assert!(
            prompt.contains("before reporting a Slack message, read its conversation"),
            "a search hit alone does not show my reply"
        );
    }

    #[test]
    fn prompt_looks_back_to_the_last_check() {
        let prompt = prompt(&[], &[], Some(1_792_400_000), 1_792_400_600);
        assert!(prompt.contains("since 2026-10-19T"), "{prompt}");
        assert!(prompt.contains("10 minutes ago"));
    }

    #[test]
    fn parses_resolved_and_new_tasks() {
        let answer = "Done.\n```meow-sync\n{\"done\": [\"url:a\"], \"new\": [{\"title\": \"Reply to Bo\", \"source\": \"Slack\", \"url\": null}]}\n```";
        let report = parse(answer).unwrap();
        assert_eq!(report.done, ["url:a"]);
        assert_eq!(report.new[0].title, "Reply to Bo");
    }

    #[test]
    fn missing_fields_mean_nothing_to_report() {
        assert_eq!(
            parse("```meow-sync\n{}\n```").unwrap(),
            SyncReport::default()
        );
    }

    #[test]
    fn missing_or_broken_block_is_an_error() {
        assert!(parse("All good!").is_err());
        assert!(parse("```meow-sync\n{oops\n```").is_err());
        assert!(parse("```meow-sync\n{}").is_err());
    }

    #[test]
    fn iso_time_formats_utc() {
        assert_eq!(iso_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_time(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso_time(1_792_400_000), "2026-10-19T08:53:20Z");
    }
}
