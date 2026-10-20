//! Keeping the day plan's agenda current: through the day, a quiet Claude run
//! that may only read the calendar reports today's events in a fenced
//! `meow-agenda` JSON block, and they replace the plan's agenda.

use serde::Deserialize;

use crate::clock;
use crate::plan::{EVENT_SHAPE, Event, EventState, Rsvp};
use crate::tasks::take_fence;

pub const AGENDA_FENCE: &str = "meow-agenda";

#[derive(Deserialize)]
struct Report {
    /// Required: an answer without it must not wipe the agenda.
    agenda: Vec<Event>,
}

/// The prompt for one refresh.
pub fn prompt(now: u64) -> String {
    format!(
        "{} Background refresh of my agenda. Only read: do not create, modify, answer or \
         delete any event. Use the calendar tools directly and keep it quick.\n\n\
         List all of today's events on my primary calendar, declined ones included.\n\n\
         Reply with only a fenced code block with the info string `{AGENDA_FENCE}` holding \
         {{\"agenda\": [each event as {EVENT_SHAPE}]}}, with an empty list when there is \
         nothing today.",
        clock::describe(now)
    )
}

/// Reads today's events out of Claude's answer.
pub fn parse(answer: &str) -> Result<Vec<Event>, String> {
    let (_, body) = take_fence(answer, AGENDA_FENCE);
    let body = body.ok_or("the refresh returned no agenda block")?;
    serde_json::from_str::<Report>(&body)
        .map(|report| report.agenda)
        .map_err(|e| format!("the refreshed agenda is not valid JSON ({e})"))
}

/// What changed between two versions of today's agenda.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Added(Event),
    Cancelled(Event),
    /// Its start moved from `from`.
    Moved {
        event: Event,
        from: Option<String>,
    },
    /// A new invitation waiting for my answer.
    Invited(Event),
}

/// The changes from `old` to `new`, leaving out events already over at
/// `now_minutes` and ones I declined. Events match by id when both have one,
/// else by title.
pub fn changes(old: &[Event], new: &[Event], now_minutes: u32) -> Vec<Change> {
    let relevant = |event: &&Event| {
        event.state(now_minutes) != EventState::Past && event.response != Some(Rsvp::Declined)
    };
    let mut found = Vec::new();
    for event in new.iter().filter(relevant) {
        let change = match old.iter().find(|before| same_event(before, event)) {
            None if event.response == Some(Rsvp::NeedsAction) => {
                Some(Change::Invited(event.clone()))
            }
            None => Some(Change::Added(event.clone())),
            Some(before) if before.start != event.start => Some(Change::Moved {
                event: event.clone(),
                from: before.start.clone(),
            }),
            Some(_) => None,
        };
        found.extend(change);
    }
    let cancelled = old
        .iter()
        .filter(relevant)
        .filter(|before| !new.iter().any(|event| same_event(before, event)))
        .map(|before| Change::Cancelled(before.clone()));
    found.extend(cancelled);
    found
}

fn same_event(a: &Event, b: &Event) -> bool {
    match (&a.id, &b.id) {
        (Some(x), Some(y)) => x == y,
        _ => a.title.eq_ignore_ascii_case(&b.title),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: Option<&str>, title: &str, start: &str) -> Event {
        Event {
            start: Some(start.into()),
            end: None,
            title: title.into(),
            url: None,
            id: id.map(Into::into),
            calendar: None,
            location: None,
            response: None,
        }
    }

    #[test]
    fn finds_added_moved_cancelled_and_invited_events() {
        let old = vec![
            event(Some("a"), "Standup", "09:30"),
            event(Some("b"), "1:1", "14:00"),
            event(Some("c"), "Lunch", "12:00"),
        ];
        let invite = Event {
            response: Some(Rsvp::NeedsAction),
            ..event(Some("d"), "Review", "16:00")
        };
        let new = vec![
            event(Some("a"), "Standup", "09:30"),
            event(Some("b"), "1:1", "15:00"),
            event(Some("e"), "Demo", "17:00"),
            invite.clone(),
        ];
        let found = changes(&old, &new, 8 * 60);
        assert_eq!(
            found,
            [
                Change::Moved {
                    event: event(Some("b"), "1:1", "15:00"),
                    from: Some("14:00".into())
                },
                Change::Added(event(Some("e"), "Demo", "17:00")),
                Change::Invited(invite),
                Change::Cancelled(event(Some("c"), "Lunch", "12:00")),
            ]
        );
    }

    #[test]
    fn past_and_declined_events_and_title_matches_are_quiet() {
        let old = vec![
            event(None, "Standup", "09:30"),
            event(Some("x"), "Old", "07:00"),
        ];
        let declined = Event {
            response: Some(Rsvp::Declined),
            ..event(Some("y"), "Party", "18:00")
        };
        let new = vec![event(Some("s"), "standup", "09:30"), declined];
        assert!(changes(&old, &new, 8 * 60).is_empty());
    }

    #[test]
    fn prompt_is_read_only_and_names_the_fence_and_the_date() {
        let text = prompt(1_792_411_200);
        assert!(text.starts_with("Today is "));
        assert!(text.contains("Only read"));
        assert!(text.contains(AGENDA_FENCE));
    }

    #[test]
    fn parses_todays_events() {
        let answer = "Here:\n```meow-agenda\n{\"agenda\": [{\"start\": \"9:30\", \
            \"end\": \"10:00\", \"title\": \"Standup\", \"url\": null}]}\n```";
        let agenda = parse(answer).unwrap();
        assert_eq!(agenda.len(), 1);
        assert_eq!(agenda[0].title, "Standup");
    }

    #[test]
    fn an_empty_day_is_a_valid_answer() {
        assert!(
            parse("```meow-agenda\n{\"agenda\": []}\n```")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn missing_or_broken_answers_are_errors() {
        assert!(parse("No events.").is_err());
        assert!(parse("```meow-agenda\n{}\n```").is_err(), "no agenda key");
        assert!(parse("```meow-agenda\n{oops\n```").is_err());
    }
}
