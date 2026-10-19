//! The day plan: a plan quick action (Morning planning) reports it in a fenced
//! `meow-plan` JSON block, and it is shown as its own page (agenda, focus,
//! replies) instead of prose. Its items join the task list.

use serde::{Deserialize, Serialize};

use crate::clock;
use crate::tasks::{FoundTask, take_fence};

pub const PLAN_FENCE: &str = "meow-plan";
const MAX_EVENTS: usize = 20;
const MAX_ITEMS: usize = 12;
const MAX_TEXT_CHARS: usize = 300;
/// An event without an end is taken to last this long.
const DEFAULT_EVENT_MINUTES: u32 = 30;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayPlan {
    /// One or two sentences on the day.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub agenda: Vec<Event>,
    /// What to tackle first, in order.
    #[serde(default)]
    pub focus: Vec<PlanItem>,
    /// Messages waiting for an answer.
    #[serde(default)]
    pub replies: Vec<PlanItem>,
    /// Everything else to act on, when there is time.
    #[serde(default)]
    pub later: Vec<PlanItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// "09:30"; `None` for an all-day event.
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
    pub title: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    pub title: String,
    /// Why it matters today, for focus items.
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

impl PlanItem {
    pub fn task(&self) -> FoundTask {
        FoundTask {
            title: self.title.clone(),
            source: self.source.clone(),
            url: self.url.clone(),
        }
    }
}

/// Where an event stands compared to now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventState {
    AllDay,
    Past,
    Now,
    Upcoming,
}

impl Event {
    pub fn state(&self, now_minutes: u32) -> EventState {
        let Some(start) = self.start.as_deref().and_then(minutes) else {
            return EventState::AllDay;
        };
        let end = self
            .end
            .as_deref()
            .and_then(minutes)
            .filter(|end| *end > start)
            .unwrap_or(start + DEFAULT_EVENT_MINUTES);
        if now_minutes >= end {
            EventState::Past
        } else if now_minutes >= start {
            EventState::Now
        } else {
            EventState::Upcoming
        }
    }

    fn cleaned(self) -> Option<Self> {
        Some(Self {
            title: clean_text(&self.title)?,
            start: self.start.as_deref().and_then(normalize_time),
            end: self.end.as_deref().and_then(normalize_time),
            url: self.url.as_deref().and_then(clean_text),
        })
    }
}

impl DayPlan {
    /// The item lists with their headings, in display order.
    pub fn sections(&self) -> [(&'static str, &[PlanItem]); 3] {
        [
            ("Focus", &self.focus),
            ("Replies", &self.replies),
            ("Later", &self.later),
        ]
    }

    /// Every item, as tasks for the task list.
    pub fn tasks(&self) -> Vec<FoundTask> {
        self.sections()
            .iter()
            .flat_map(|(_, items)| items.iter().map(PlanItem::task))
            .collect()
    }

    /// Trims and bounds what Claude sent, sorts the agenda; `None` if nothing is left.
    fn cleaned(self) -> Option<Self> {
        let mut agenda: Vec<Event> = self
            .agenda
            .into_iter()
            .filter_map(Event::cleaned)
            .take(MAX_EVENTS)
            .collect();
        agenda.sort_by_key(|event| event.start.as_deref().and_then(minutes));
        let plan = Self {
            summary: clean_text(&self.summary).unwrap_or_default(),
            agenda,
            focus: clean_items(self.focus),
            replies: clean_items(self.replies),
            later: clean_items(self.later),
        };
        let empty = plan.summary.is_empty()
            && plan.agenda.is_empty()
            && plan.sections().iter().all(|(_, items)| items.is_empty());
        (!empty).then_some(plan)
    }
}

fn clean_items(items: Vec<PlanItem>) -> Vec<PlanItem> {
    items
        .into_iter()
        .filter_map(|item| {
            Some(PlanItem {
                title: clean_text(&item.title)?,
                why: item.why.as_deref().and_then(clean_text),
                source: item.source.as_deref().and_then(clean_text),
                url: item.url.as_deref().and_then(clean_text),
            })
        })
        .take(MAX_ITEMS)
        .collect()
}

fn clean_text(text: &str) -> Option<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then(|| text.chars().take(MAX_TEXT_CHARS).collect())
}

/// Minutes since midnight of "9:30" or "09:30".
fn minutes(time: &str) -> Option<u32> {
    let (hour, minute) = time.trim().split_once(':')?;
    let (hour, minute): (u32, u32) = (hour.parse().ok()?, minute.parse().ok()?);
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}

/// "9:30" → "09:30"; anything that is not a time of day → `None`.
fn normalize_time(time: &str) -> Option<String> {
    minutes(time).map(|m| format!("{:02}:{:02}", m / 60, m % 60))
}

/// Splits Claude's answer into the text to show and the plan it reported.
pub fn extract(answer: &str) -> (String, Option<DayPlan>) {
    let (shown, body) = take_fence(answer, PLAN_FENCE);
    let plan = body
        .and_then(|body| serde_json::from_str::<DayPlan>(&body).ok())
        .and_then(DayPlan::cleaned);
    (shown, plan)
}

/// Asks Claude to report its plan as a `meow-plan` block, for the plan page.
pub fn instruction(now: u64) -> String {
    format!(
        "{} Your answer is shown as a structured page, so keep the prose to one or two \
         sentences summing up the day, then end with a fenced code block with the info string \
         `{PLAN_FENCE}` holding one JSON object:\n\
         {{\"summary\": the same one or two sentences,\n \
         \"agenda\": today's events in order, each {{\"start\": \"HH:MM\" (24-hour, my local time; \
         null if all-day), \"end\": \"HH:MM\" or null, \"title\": short title, \"url\": link or \
         null}},\n \
         \"focus\": the 3 things to tackle first, most important first, each {{\"title\": short \
         imperative title, \"why\": why it matters today in a few words, \"source\": Linear, \
         GitHub, Slack, Gmail…, \"url\": link or null}},\n \
         \"replies\": messages waiting for my answer, same shape without \"why\",\n \
         \"later\": anything else I need to act on, same shape}}.\n\
         Do not repeat an item in two lists, and use empty lists when there is nothing.",
        clock::describe(now)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(start: Option<&str>, end: Option<&str>) -> Event {
        Event {
            start: start.map(Into::into),
            end: end.map(Into::into),
            title: "Standup".into(),
            url: None,
        }
    }

    #[test]
    fn extract_strips_the_block_and_parses_the_plan() {
        let answer = "Busy day.\n\n```meow-plan\n{\"summary\": \"Busy day.\", \
            \"agenda\": [{\"start\": \"14:00\", \"title\": \"1:1\"}, \
                         {\"start\": \"9:30\", \"end\": \"9:45\", \"title\": \"Standup\"}], \
            \"focus\": [{\"title\": \"Fix ACC-52\", \"why\": \"blocks release\", \
                         \"source\": \"Linear\", \"url\": \"https://l/52\"}]}\n```";
        let (shown, plan) = extract(answer);
        let plan = plan.unwrap();
        assert_eq!(shown, "Busy day.");
        let starts: Vec<_> = plan.agenda.iter().map(|e| e.start.as_deref()).collect();
        assert_eq!(starts, [Some("09:30"), Some("14:00")]);
        assert_eq!(plan.focus[0].why.as_deref(), Some("blocks release"));
        assert!(plan.replies.is_empty() && plan.later.is_empty());
    }

    #[test]
    fn extract_tolerates_missing_or_broken_plans() {
        assert_eq!(extract("hi"), ("hi".into(), None));
        assert_eq!(extract("hi\n```meow-plan\nnope\n```").1, None);
        assert_eq!(extract("```meow-plan\n{}\n```").1, None, "empty plan");
    }

    #[test]
    fn cleaning_drops_untitled_items_and_bad_times() {
        let plan = DayPlan {
            agenda: vec![
                Event {
                    start: Some("lunch".into()),
                    ..event(None, None)
                },
                Event {
                    title: " ".into(),
                    ..event(Some("10:00"), None)
                },
            ],
            later: vec![
                PlanItem {
                    title: "  Reply  to Bo ".into(),
                    why: Some(" ".into()),
                    source: None,
                    url: Some("".into()),
                },
                PlanItem {
                    title: "".into(),
                    why: None,
                    source: None,
                    url: None,
                },
            ],
            ..DayPlan::default()
        }
        .cleaned()
        .unwrap();
        assert_eq!(plan.agenda.len(), 1);
        assert_eq!(
            plan.agenda[0].start, None,
            "unreadable time reads as all-day"
        );
        assert_eq!(plan.later.len(), 1);
        let item = &plan.later[0];
        assert_eq!(
            (item.title.as_str(), &item.why, &item.url),
            ("Reply to Bo", &None, &None)
        );
    }

    #[test]
    fn events_know_whether_they_are_past_now_or_upcoming() {
        let ten = 10 * 60;
        let standup = event(Some("09:30"), Some("10:00"));
        assert_eq!(standup.state(9 * 60), EventState::Upcoming);
        assert_eq!(standup.state(9 * 60 + 45), EventState::Now);
        assert_eq!(standup.state(ten), EventState::Past);
        let no_end = event(Some("10:00"), None);
        assert_eq!(no_end.state(ten + 29), EventState::Now);
        assert_eq!(no_end.state(ten + 30), EventState::Past);
        assert_eq!(event(None, None).state(ten), EventState::AllDay);
    }

    #[test]
    fn every_item_becomes_a_task() {
        let item = |title: &str| PlanItem {
            title: title.into(),
            why: None,
            source: Some("Slack".into()),
            url: None,
        };
        let plan = DayPlan {
            focus: vec![item("a")],
            replies: vec![item("b")],
            later: vec![item("c")],
            ..DayPlan::default()
        };
        let titles: Vec<_> = plan.tasks().into_iter().map(|t| t.title).collect();
        assert_eq!(titles, ["a", "b", "c"]);
    }

    #[test]
    fn instruction_names_the_fence_and_the_date() {
        let text = instruction(1_792_411_200);
        assert!(text.contains(PLAN_FENCE));
        assert!(text.starts_with("Today is "));
    }
}
