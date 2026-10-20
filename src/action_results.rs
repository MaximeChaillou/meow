//! The last result of each quick action, kept so clicking an action shows it
//! instead of running it again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::plan::{DayPlan, Event, Rsvp};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResult {
    pub answer: String,
    pub ran_at: u64,
    /// Claude session of that run, so follow-up questions can resume it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The day plan of a plan action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<DayPlan>,
}

/// Results by action title.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResults {
    results: HashMap<String, ActionResult>,
}

impl ActionResults {
    pub fn get(&self, title: &str) -> Option<&ActionResult> {
        self.results.get(title)
    }

    pub fn record(
        &mut self,
        title: &str,
        answer: &str,
        session: Option<&str>,
        plan: Option<DayPlan>,
        now: u64,
    ) {
        self.results.insert(
            title.to_string(),
            ActionResult {
                answer: answer.to_string(),
                ran_at: now,
                session: session.map(str::to_string),
                plan,
            },
        );
    }

    /// Moves the result saved under `from` to `to`, after an action is
    /// renamed; nothing happens if `to` already has one.
    pub fn rename(&mut self, from: &str, to: &str) {
        if self.results.contains_key(to) {
            return;
        }
        if let Some(result) = self.results.remove(from) {
            self.results.insert(to.to_string(), result);
        }
    }

    /// Updates the session after a follow-up, keeping the result itself.
    pub fn set_session(&mut self, title: &str, session: &str) {
        if let Some(result) = self.results.get_mut(title) {
            result.session = Some(session.to_string());
        }
    }

    /// Replaces the agenda of a result's day plan, keeping the rest; `false`
    /// when there is no plan to update.
    pub fn set_agenda(&mut self, title: &str, agenda: Vec<Event>) -> bool {
        let Some(result) = self.results.get_mut(title) else {
            return false;
        };
        let Some(plan) = &result.plan else {
            return false;
        };
        result.plan = Some(plan.with_agenda(agenda));
        true
    }

    /// Records my answer to event `id` in every plan that has it; `false`
    /// when none does.
    pub fn set_response(&mut self, id: &str, response: Rsvp) -> bool {
        let mut changed = false;
        for result in self.results.values_mut() {
            let Some(plan) = &result.plan else { continue };
            if plan.agenda.iter().any(|e| e.id.as_deref() == Some(id)) {
                result.plan = Some(plan.with_response(id, response));
                changed = true;
            }
        }
        changed
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| {
                format!(
                    "Could not read quick action results from {}: {e}",
                    path.display()
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Could not read {}: {e}", path.display())),
        }
    }

    /// Writes atomically, so a crash never leaves a half-written file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let fail = |e: &dyn std::fmt::Display| {
            format!(
                "Could not save quick action results to {}: {e}",
                path.display()
            )
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| fail(&e))?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, json).map_err(|e| fail(&e))?;
        std::fs::rename(&temp, path).map_err(|e| fail(&e))
    }
}

/// `~/Library/Application Support/Meow/action-results.json`.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home).join("Library/Application Support/Meow/action-results.json")
    })
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ran_ago(ran_at: u64, now: u64) -> String {
    let minutes = now.saturating_sub(ran_at) / 60;
    match minutes {
        0 => "just now".to_string(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", minutes / 60),
        1440..=2879 => "yesterday".to_string(),
        _ => format!("{} days ago", minutes / 1440),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_replaces_the_last_result() {
        let mut results = ActionResults::default();
        assert!(results.get("Pending tasks").is_none());
        results.record("Pending tasks", "old", Some("s1"), None, 10);
        results.record("Pending tasks", "new", Some("s2"), None, 20);
        let last = results.get("Pending tasks").unwrap();
        assert_eq!((last.answer.as_str(), last.ran_at), ("new", 20));
        assert_eq!(last.session.as_deref(), Some("s2"));
    }

    #[test]
    fn follow_up_session_keeps_the_answer() {
        let mut results = ActionResults::default();
        results.record("Plan", "answer", Some("s1"), None, 10);
        results.set_session("Plan", "s2");
        results.set_session("Unknown", "s3");
        let last = results.get("Plan").unwrap();
        assert_eq!(
            (last.answer.as_str(), last.session.as_deref()),
            ("answer", Some("s2"))
        );
        assert!(results.get("Unknown").is_none());
    }

    #[test]
    fn renaming_carries_the_result_over_unless_taken() {
        let mut results = ActionResults::default();
        results.record("Old", "old answer", None, None, 10);
        results.rename("Old", "New");
        assert!(results.get("Old").is_none());
        assert_eq!(results.get("New").unwrap().answer, "old answer");
        results.record("Old", "other", None, None, 20);
        results.rename("Old", "New");
        assert_eq!(results.get("New").unwrap().answer, "old answer");
        assert!(results.get("Old").is_some());
    }

    #[test]
    fn agenda_updates_keep_the_rest_of_the_result() {
        let mut results = ActionResults::default();
        results.record("Text", "answer", None, None, 10);
        results.record("Plan", "answer", Some("s1"), Some(DayPlan::default()), 10);
        let event = Event {
            start: Some("09:30".into()),
            end: None,
            title: "Standup".into(),
            url: None,
            id: Some("e1".into()),
            calendar: None,
            location: None,
            response: Some(Rsvp::NeedsAction),
        };
        assert!(!results.set_agenda("Text", vec![event.clone()]));
        assert!(!results.set_agenda("Unknown", vec![event.clone()]));
        assert!(results.set_agenda("Plan", vec![event.clone()]));
        let result = results.get("Plan").unwrap();
        assert_eq!(result.plan.as_ref().unwrap().agenda, [event]);
        assert_eq!((result.ran_at, result.session.as_deref()), (10, Some("s1")));
        assert!(results.set_response("e1", Rsvp::Accepted));
        assert!(!results.set_response("other", Rsvp::Accepted));
        let agenda = &results.get("Plan").unwrap().plan.as_ref().unwrap().agenda;
        assert_eq!(agenda[0].response, Some(Rsvp::Accepted));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("meow-results-{}", std::process::id()));
        let path = dir.join("action-results.json");
        let mut results = ActionResults::default();
        let plan = DayPlan {
            summary: "Quiet day.".into(),
            ..DayPlan::default()
        };
        results.record("Plan", "## Today", None, Some(plan), 5);
        results.save(&path).unwrap();
        assert_eq!(ActionResults::load(&path).unwrap(), results);
        assert_eq!(
            ActionResults::load(&dir.join("missing.json")).unwrap(),
            ActionResults::default()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ages_read_naturally() {
        let now = 10_000_000;
        assert_eq!(ran_ago(now - 30, now), "just now");
        assert_eq!(ran_ago(now - 5 * 60, now), "5 min ago");
        assert_eq!(ran_ago(now - 3 * 3600, now), "3 h ago");
        assert_eq!(ran_ago(now - 30 * 3600, now), "yesterday");
        assert_eq!(ran_ago(now - 3 * 86_400, now), "3 days ago");
        assert_eq!(
            ran_ago(now + 50, now),
            "just now",
            "clock skew is not negative"
        );
    }
}
