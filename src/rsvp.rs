//! Answering a calendar invitation from the day plan: a run that may only call
//! the calendar's respond tool sets my answer, and reports how it went.

use crate::plan::{Event, Rsvp};

const DONE: &str = "DONE";
const FAILED: &str = "FAILED:";

/// The prompt that sets my answer to `event`; `None` without an event id.
pub fn prompt(event: &Event, answer: Rsvp) -> Option<String> {
    let id = event.id.as_deref()?;
    let calendar = event.calendar.as_deref().unwrap_or("primary");
    let when = event.start.as_deref().unwrap_or("all day");
    Some(format!(
        "Set my response to the calendar event with id `{id}` on calendar `{calendar}` \
         (\"{}\", today at {when}) to {}. Change nothing else on the event. When it is done, \
         reply with exactly {DONE}; if it fails, reply with {FAILED} and the reason in a few \
         words.",
        event.title,
        answer.as_str()
    ))
}

/// The run's answer: `Ok` only on an explicit success.
pub fn parse(result: &str) -> Result<(), String> {
    let line = result.trim();
    if line == DONE {
        Ok(())
    } else {
        Err(line.strip_prefix(FAILED).unwrap_or(line).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: Option<&str>) -> Event {
        Event {
            start: Some("09:30".into()),
            end: None,
            title: "Standup".into(),
            url: None,
            id: id.map(Into::into),
            calendar: None,
            location: None,
            response: Some(Rsvp::NeedsAction),
        }
    }

    #[test]
    fn prompt_names_the_event_and_the_answer() {
        let text = prompt(&event(Some("e1")), Rsvp::Declined).unwrap();
        assert!(text.contains("`e1`") && text.contains("`primary`"));
        assert!(text.contains("to declined"));
        assert_eq!(prompt(&event(None), Rsvp::Accepted), None);
    }

    #[test]
    fn only_an_explicit_success_counts() {
        assert_eq!(parse(" DONE \n"), Ok(()));
        assert_eq!(parse("FAILED: not invited"), Err("not invited".into()));
        assert_eq!(parse("Sure!"), Err("Sure!".into()));
    }
}
