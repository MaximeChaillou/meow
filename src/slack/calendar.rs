//! Creating calendar events from a Slack conversation ("set up a call with
//! us both tomorrow at 10"), through whatever calendar connector the user's
//! Claude Code has.
//!
//! Two steps, each run limited to the tools it needs: a proposal, where Claude
//! may only read calendars (free/busy, slot suggestions); then, once the user
//! confirms, the creation, where Claude may only call the create tool.

use serde::{Deserialize, Serialize};

use super::assist::{self, Context};
use super::voice;

const EVENT_OPEN: &str = "<event>";
const EVENT_CLOSE: &str = "</event>";
const CREATED: &str = "CREATED:";
const FAILED: &str = "FAILED:";
/// What Ask replies when the request is to schedule something.
pub const ROUTE_MARKER: &str = "<calendar/>";

/// The calendar tools found among the session's tools.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CalendarTools {
    /// Tools that only look: list, search, free/busy, slot suggestions.
    pub read: Vec<String>,
    /// Tools that create an event.
    pub create: Vec<String>,
}

impl CalendarTools {
    /// Picks the calendar tools out of `tools` by name, whichever connector
    /// provides them; `None` without a way to both read and create.
    pub fn find(tools: &[String]) -> Option<Self> {
        let mut found = Self::default();
        for tool in tools {
            let name = tool.to_lowercase();
            if !name.starts_with("mcp__") || !is_calendar(&name) {
                continue;
            }
            if is_create(&name) {
                found.create.push(tool.clone());
            } else if is_read(&name) {
                found.read.push(tool.clone());
            }
        }
        (!found.read.is_empty() && !found.create.is_empty()).then_some(found)
    }
}

fn is_calendar(name: &str) -> bool {
    ["calendar", "gcal"].iter().any(|word| name.contains(word))
}

fn is_create(name: &str) -> bool {
    let creates = ["create", "insert", "add", "new"]
        .iter()
        .any(|word| name.contains(word));
    let event = ["event", "meeting"].iter().any(|word| name.contains(word));
    creates && event
}

/// Looks without changing: anything that edits, answers or removes is left out.
fn is_read(name: &str) -> bool {
    let changes = [
        "delete", "update", "cancel", "respond", "remove", "move", "patch",
    ]
    .iter()
    .any(|word| name.contains(word));
    let reads = [
        "list", "get", "search", "suggest", "find", "free", "busy", "query", "availab",
    ]
    .iter()
    .any(|word| name.contains(word));
    reads && !changes
}

/// Someone in the conversation, who may be invited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    /// Empty when Slack does not share it.
    pub email: String,
    pub tz: String,
    /// The user themselves.
    pub me: bool,
}

/// The event Claude proposes, for the user to confirm.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub title: String,
    /// RFC 3339, with the offset.
    pub start: String,
    pub end: String,
    pub time_zone: String,
    /// Attendees' email addresses, the user included.
    pub attendees: Vec<String>,
    #[serde(default)]
    pub description: String,
    /// The tool that will create it.
    pub create_tool: String,
    /// One line for the user: what, when, with whom.
    pub summary: String,
    /// A message the user may post about the invite.
    #[serde(default)]
    pub announcement: Option<String>,
}

/// The prompt that plans the event from the user's `request`, reading calendars only.
pub fn proposal_prompt(
    context: &Context,
    request: &str,
    people: &[Person],
    tools: &CalendarTools,
) -> String {
    let me = context.me;
    let people: Vec<String> = people.iter().map(person_line).collect();
    let style = context
        .style
        .map(|guide| format!("\n\n{}", voice::draft_instruction(me, guide)))
        .unwrap_or_default();
    format!(
        "You schedule a calendar event for {me}, from their request in a Slack \
conversation.\n\
{conversation}\n\n\
People in this conversation:\n{people}\n\n\
{me}'s request: {request}\n\n\
You may only read calendars, with: {read}. You cannot create the event: {me} \
confirms it first.\n\
Work out:\n\
- who attends: by default everyone in this conversation, {me} included; follow the \
request when it says otherwise. Invite people by the email listed above.\n\
- when: dates are relative to today, in {me}'s time zone unless said otherwise. When \
the request asks for a time when people are free, check their availability with the \
tools and pick the earliest slot that suits everyone, within working hours.\n\
- how long: as asked, else 30 minutes.\n\
- a short title from what the conversation is about, in the conversation's language, \
and an optional one-line description.\n\n\
Reply with one short line for {me}, in the language of the request, then the event as \
JSON inside {EVENT_OPEN}{EVENT_CLOSE} tags, with these keys: \"title\", \"start\" and \
\"end\" (RFC 3339 with offset), \"time_zone\" (IANA), \"attendees\" (emails), \
\"description\", \"create_tool\" (one of: {create}), \"summary\" (one line for {me}: \
what, when, with whom, in the language of the request), \"announcement\" (a short \
message {me} could post in the conversation about the invite, written as them, in the \
conversation's language, with no emoji).\n\
When the event cannot be planned (no common slot, someone without an email, a \
missing detail), give the reason in one line and no {EVENT_OPEN} tag.{style}",
        conversation = assist::conversation(context),
        people = people.join("\n"),
        read = tools.read.join(", "),
        create = tools.create.join(", "),
    )
}

/// The prompt that creates exactly `proposal`, with its create tool only.
pub fn create_prompt(proposal: &Proposal) -> String {
    let event = serde_json::to_string_pretty(proposal).unwrap_or_default();
    format!(
        "Create this calendar event with {tool}, exactly as described, inviting the \
attendees:\n{event}\n\n\
Do nothing else. Then reply with one line starting with {CREATED} and the event's link \
if the tool returned one, or with {FAILED} and the reason.",
        tool = proposal.create_tool,
    )
}

/// What the proposal run came back with: an event, or the reason there is none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Planned {
    Event {
        note: String,
        proposal: Box<Proposal>,
    },
    NotPlanned(String),
}

/// Reads the proposal run's result. The create tool must be one of `tools`.
pub fn parse_proposal(result: &str, tools: &CalendarTools) -> Planned {
    let Some(start) = result.find(EVENT_OPEN) else {
        return Planned::NotPlanned(result.trim().to_string());
    };
    let inner_start = start + EVENT_OPEN.len();
    let inner_end = result[inner_start..]
        .find(EVENT_CLOSE)
        .map_or(result.len(), |len| inner_start + len);
    let note = result[..start].trim().to_string();
    let json = strip_code_fence(&result[inner_start..inner_end]);
    let mut proposal: Proposal = match serde_json::from_str(json) {
        Ok(proposal) => proposal,
        Err(error) => {
            return Planned::NotPlanned(format!("Claude's event could not be read ({error})."));
        }
    };
    if !tools.create.contains(&proposal.create_tool) {
        // Never call a tool the user's setup did not offer for creating events.
        match tools.create.first() {
            Some(tool) => proposal.create_tool = tool.clone(),
            None => return Planned::NotPlanned("No calendar tool can create events.".into()),
        }
    }
    if proposal.attendees.is_empty() {
        return Planned::NotPlanned("Claude's event has no attendees.".into());
    }
    Planned::Event {
        note,
        proposal: Box::new(proposal),
    }
}

/// The creation run's line without its prefix: `Ok` when the event exists.
/// Anything but an explicit success counts as a failure.
pub fn parse_created(result: &str) -> Result<String, String> {
    let line = result.trim();
    if let Some(rest) = line.strip_prefix(CREATED) {
        Ok(rest.trim().to_string())
    } else {
        Err(line.strip_prefix(FAILED).unwrap_or(line).trim().to_string())
    }
}

/// Whether Ask's result routes the request to the calendar.
pub fn wants_calendar(result: &str) -> bool {
    result.contains(ROUTE_MARKER)
}

/// The instruction that lets Ask hand scheduling requests over.
pub fn route_instruction() -> String {
    format!(
        "When the request asks to create or schedule a calendar event, meeting or call, \
reply with exactly {ROUTE_MARKER} and nothing else: the calendar is handled separately."
    )
}

impl Proposal {
    /// "2026-10-20 10:00–10:30", or both full stamps when it spans days or
    /// the stamps are not RFC 3339.
    pub fn when(&self) -> String {
        let split = |stamp: &str| -> Option<(String, String)> {
            let (date, time) = stamp.split_once('T')?;
            let time = time.get(..5)?;
            (date.len() == 10).then(|| (date.to_string(), time.to_string()))
        };
        match (split(&self.start), split(&self.end)) {
            (Some((day, from)), Some((end_day, to))) if day == end_day => {
                format!("{day} {from}–{to} ({})", self.time_zone)
            }
            (Some((day, from)), Some((end_day, to))) => {
                format!("{day} {from} – {end_day} {to} ({})", self.time_zone)
            }
            _ => format!("{} – {}", self.start, self.end),
        }
    }
}

fn person_line(person: &Person) -> String {
    let email = if person.email.is_empty() {
        "no email known".to_string()
    } else {
        person.email.clone()
    };
    let tz = if person.tz.is_empty() {
        String::new()
    } else {
        format!(", time zone {}", person.tz)
    };
    let me = if person.me { " (this is the user)" } else { "" };
    format!("- {} <{email}>{tz}{me}", person.name)
}

/// Models sometimes wrap JSON in a Markdown code fence.
fn strip_code_fence(text: &str) -> &str {
    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .unwrap_or(text);
    text.strip_suffix("```").unwrap_or(text).trim()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack::assist::Line;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn tools() -> CalendarTools {
        CalendarTools {
            read: names(&["mcp__cal__list_events", "mcp__cal__suggest_time"]),
            create: names(&["mcp__cal__create_event"]),
        }
    }

    #[test]
    fn calendar_tools_are_found_by_name_and_split_by_effect() {
        let all = names(&[
            "Bash",
            "mcp__claude_ai_Google_Calendar__create_event",
            "mcp__claude_ai_Google_Calendar__delete_event",
            "mcp__claude_ai_Google_Calendar__list_events",
            "mcp__claude_ai_Google_Calendar__suggest_time",
            "mcp__claude_ai_Google_Calendar__update_event",
            "mcp__claude_ai_Google_Calendar__respond_to_event",
            "mcp__other__list_issues",
        ]);
        let found = CalendarTools::find(&all).unwrap();
        assert_eq!(
            found.create,
            names(&["mcp__claude_ai_Google_Calendar__create_event"])
        );
        assert_eq!(
            found.read,
            names(&[
                "mcp__claude_ai_Google_Calendar__list_events",
                "mcp__claude_ai_Google_Calendar__suggest_time",
            ])
        );
    }

    #[test]
    fn no_calendar_without_both_reading_and_creating() {
        assert_eq!(
            CalendarTools::find(&names(&["mcp__cal__list_events"])),
            None
        );
        assert_eq!(CalendarTools::find(&names(&["Bash", "Read"])), None);
    }

    fn proposal_json(tool: &str) -> String {
        format!(
            r#"{{"title":"Sync","start":"2026-10-20T10:00:00+02:00","end":"2026-10-20T10:30:00+02:00",
            "time_zone":"Europe/Paris","attendees":["a@example.com","b@example.com"],
            "description":"","create_tool":"{tool}","summary":"Sync tomorrow 10:00 with A",
            "announcement":"Invite sent for tomorrow 10:00"}}"#
        )
    }

    #[test]
    fn proposal_is_read_from_the_tags() {
        let result = format!(
            "Found a slot.\n<event>\n```json\n{}\n```\n</event>",
            proposal_json("mcp__cal__create_event")
        );
        match parse_proposal(&result, &tools()) {
            Planned::Event { note, proposal } => {
                assert_eq!(note, "Found a slot.");
                assert_eq!(proposal.title, "Sync");
                assert_eq!(proposal.attendees.len(), 2);
                assert_eq!(
                    proposal.announcement.as_deref(),
                    Some("Invite sent for tomorrow 10:00")
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_create_tool_falls_back_to_one_on_offer() {
        let result = format!("<event>{}</event>", proposal_json("mcp__evil__send"));
        match parse_proposal(&result, &tools()) {
            Planned::Event { proposal, .. } => {
                assert_eq!(proposal.create_tool, "mcp__cal__create_event")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn no_event_tag_means_not_planned() {
        assert_eq!(
            parse_proposal("No common slot tomorrow.", &tools()),
            Planned::NotPlanned("No common slot tomorrow.".into())
        );
        assert!(matches!(
            parse_proposal("<event>not json</event>", &tools()),
            Planned::NotPlanned(_)
        ));
    }

    #[test]
    fn proposal_prompt_lists_people_and_only_read_tools() {
        let lines = [Line {
            ts: "1.0".into(),
            author: "Robin".into(),
            text: "let's talk".into(),
        }];
        let context = Context {
            title: "Robin",
            me: "Sam",
            lines: &lines,
            now: 1_792_411_200,
            style: None,
            can_schedule: true,
        };
        let people = [
            Person {
                name: "Sam".into(),
                email: "sam@example.com".into(),
                tz: "Europe/Paris".into(),
                me: true,
            },
            Person {
                name: "Robin".into(),
                email: String::new(),
                tz: String::new(),
                me: false,
            },
        ];
        let prompt = proposal_prompt(&context, "a call tomorrow at 10", &people, &tools());
        assert!(
            prompt.contains("- Sam <sam@example.com>, time zone Europe/Paris (this is the user)")
        );
        assert!(prompt.contains("- Robin <no email known>"));
        assert!(prompt.contains("Sam's request: a call tomorrow at 10"));
        assert!(prompt.contains("with: mcp__cal__list_events, mcp__cal__suggest_time"));
        assert!(prompt.contains("You cannot create the event"));
        assert!(prompt.contains("(one of: mcp__cal__create_event)"));
    }

    #[test]
    fn create_prompt_names_the_tool_and_the_event() {
        let proposal: Proposal =
            serde_json::from_str(&proposal_json("mcp__cal__create_event")).unwrap();
        let prompt = create_prompt(&proposal);
        assert!(prompt.contains("with mcp__cal__create_event"));
        assert!(prompt.contains("\"title\": \"Sync\""));
        assert!(prompt.contains("Do nothing else"));
    }

    #[test]
    fn when_reads_naturally() {
        let mut proposal: Proposal =
            serde_json::from_str(&proposal_json("mcp__cal__create_event")).unwrap();
        assert_eq!(proposal.when(), "2026-10-20 10:00–10:30 (Europe/Paris)");
        proposal.end = "2026-10-21T09:00:00+02:00".into();
        assert_eq!(
            proposal.when(),
            "2026-10-20 10:00 – 2026-10-21 09:00 (Europe/Paris)"
        );
        proposal.start = "tomorrow".into();
        assert_eq!(proposal.when(), "tomorrow – 2026-10-21T09:00:00+02:00");
    }

    #[test]
    fn only_an_explicit_success_counts_as_created() {
        assert_eq!(
            parse_created("CREATED: https://cal.example.com/e/1"),
            Ok("https://cal.example.com/e/1".into())
        );
        assert_eq!(parse_created("FAILED: no access"), Err("no access".into()));
        assert_eq!(parse_created("Hmm, not sure"), Err("Hmm, not sure".into()));
    }

    #[test]
    fn ask_routes_with_the_marker() {
        assert!(wants_calendar("<calendar/>"));
        assert!(!wants_calendar("Here is a summary"));
        assert!(route_instruction().contains("<calendar/>"));
    }
}
