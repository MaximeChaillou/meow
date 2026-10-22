//! Scheduled tasks, asked for in the chat: "at 17:00, do X" runs X at that
//! time (optionally every day); "when Y happens, do X" has Claude check for Y
//! every few minutes, read-only, then do X (often just: let me know).
//!
//! Claude reports a request as a fenced `meow-schedule` JSON block, which the
//! app strips from the chat and turns into a [`Routine`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::clock;
use crate::tasks::take_fence;

pub const SCHEDULE_FENCE: &str = "meow-schedule";
pub const WATCH_FENCE: &str = "meow-watch";
/// A watch is checked this often unless the request says otherwise.
pub const DEFAULT_CHECK_MINUTES: u64 = 10;
const MIN_CHECK_MINUTES: u64 = 5;
const MAX_CHECK_MINUTES: u64 = 24 * 60;
/// Finished routines stay listed this long.
pub const KEEP_DONE_SECS: u64 = 7 * 24 * 60 * 60;
const MAX_TEXT_CHARS: usize = 1000;
const MAX_FIRED: usize = 20;
/// At most this many scheduled tasks are shown to Claude in a chat message.
const MAX_LISTED: usize = 20;
const DAY_SECS: u64 = 24 * 60 * 60;

/// When a routine runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Trigger {
    /// At a time (seconds since the epoch), then every day if `daily`.
    At { at: u64, daily: bool },
    /// Whenever `condition` is found true, checked every `every_minutes`.
    When {
        condition: String,
        every_minutes: u64,
        /// Stop after the first time it happens.
        once: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Active,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routine {
    pub id: u64,
    pub name: String,
    pub trigger: Trigger,
    /// What to do when it fires.
    pub then: String,
    /// Firing only means telling the user; nothing else is run.
    #[serde(default)]
    pub notify_only: bool,
    pub created_at: u64,
    pub status: Status,
    /// The next run (`At`) or check (`When`), seconds since the epoch.
    pub next_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked: Option<u64>,
    /// The last thing it did or found, for the Scheduled page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<u64>,
    /// What a watch already reported, so the same thing is not reported twice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fired: Vec<String>,
}

impl Routine {
    pub fn is_active(&self) -> bool {
        self.status == Status::Active
    }

    /// "today 17:00 · every day", "checks every 10 min".
    pub fn schedule_label(&self, now: u64) -> String {
        match &self.trigger {
            Trigger::At { daily, .. } => {
                let when = clock::when_label(self.next_at, now);
                if *daily {
                    format!("{when} · every day")
                } else {
                    when
                }
            }
            Trigger::When { every_minutes, .. } => format!("checks every {every_minutes} min"),
        }
    }

    /// After running at its time: the next day, or done.
    fn after_run(&mut self, now: u64, result: &str) {
        self.last_result = clean(result);
        match self.trigger {
            Trigger::At { daily: true, at } => {
                // The next occurrence after now, keeping the time of day.
                let mut next = at.max(self.next_at);
                while next <= now {
                    next += DAY_SECS;
                }
                self.next_at = next;
            }
            Trigger::At { daily: false, .. } => self.finish(now),
            Trigger::When { .. } => {}
        }
    }

    fn finish(&mut self, now: u64) {
        self.status = Status::Done;
        self.done_at = Some(now);
    }
}

/// A request as Claude reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Spec {
    /// An existing routine to change (or restart, once finished) or cancel.
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub name: String,
    /// "at", "when", or "cancel" (with `id`).
    #[serde(default)]
    pub kind: String,
    /// "YYYY-MM-DDTHH:MM", local time, for "at".
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub daily: bool,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub every_minutes: Option<u64>,
    #[serde(default = "yes")]
    pub once: bool,
    #[serde(default)]
    pub then: String,
    #[serde(default)]
    pub notify_only: bool,
}

fn yes() -> bool {
    true
}

/// What a reported request did to the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Added(Routine),
    Updated(Routine),
    Cancelled(Routine),
}

/// What a watch check found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Check {
    pub happened: bool,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub details: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routines {
    routines: Vec<Routine>,
    #[serde(default)]
    next_id: u64,
}

impl Routines {
    /// Active ones first (soonest first), then finished ones, most recent first.
    pub fn display(&self) -> Vec<&Routine> {
        let mut active: Vec<&Routine> = self.routines.iter().filter(|r| r.is_active()).collect();
        active.sort_by_key(|r| r.next_at);
        let mut done: Vec<&Routine> = self.routines.iter().filter(|r| !r.is_active()).collect();
        done.sort_by_key(|r| std::cmp::Reverse(r.done_at));
        active.into_iter().chain(done).collect()
    }

    pub fn get(&self, id: u64) -> Option<&Routine> {
        self.routines.iter().find(|r| r.id == id)
    }

    pub fn active_count(&self) -> usize {
        self.routines.iter().filter(|r| r.is_active()).count()
    }

    /// Adds, changes or cancels a routine as `spec` asks; the error says what
    /// is missing.
    pub fn apply(&mut self, spec: &Spec, now: u64) -> Result<Change, String> {
        let Some(id) = spec.id else {
            return self.add(spec, now).cloned().map(Change::Added);
        };
        let index = self
            .routines
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| format!("there is no scheduled task #{id}"))?;
        if spec.kind.trim() == "cancel" {
            return Ok(Change::Cancelled(self.routines.remove(index)));
        }
        let (trigger, next_at, then) = build(spec, now)?;
        let routine = &mut self.routines[index];
        if let Some(name) = clean(&spec.name) {
            routine.name = name;
        }
        routine.trigger = trigger;
        routine.next_at = next_at;
        routine.then = then;
        routine.notify_only = spec.notify_only;
        routine.status = Status::Active;
        routine.done_at = None;
        // A watch looks again from now on, still skipping what it reported.
        routine.created_at = now;
        routine.last_checked = None;
        Ok(Change::Updated(routine.clone()))
    }

    /// Turns `spec` into a new routine; the error says what is missing.
    pub fn add(&mut self, spec: &Spec, now: u64) -> Result<&Routine, String> {
        let (trigger, next_at, then) = build(spec, now)?;
        let name = clean(&spec.name).unwrap_or_else(|| then.chars().take(60).collect());
        let routine = Routine {
            id: self.next_id,
            name,
            trigger,
            then,
            notify_only: spec.notify_only,
            created_at: now,
            status: Status::Active,
            next_at,
            last_checked: None,
            last_result: None,
            done_at: None,
            fired: Vec::new(),
        };
        self.next_id += 1;
        self.routines.push(routine);
        Ok(self.routines.last().expect("just pushed"))
    }

    /// The routines as Claude sees them in a chat message, one JSON object
    /// per line in the shape of a request, so it can restart or change one.
    pub fn describe_for_claude(&self, now: u64) -> String {
        let shown = self.display();
        if shown.is_empty() {
            return "I have no scheduled tasks.".to_string();
        }
        let lines: Vec<String> = shown
            .iter()
            .take(MAX_LISTED)
            .map(|routine| describe(routine, now).to_string())
            .collect();
        let hidden = shown.len().saturating_sub(MAX_LISTED);
        let more = if hidden > 0 {
            format!("\n({hidden} older finished ones not shown.)")
        } else {
            String::new()
        };
        format!(
            "My scheduled tasks (\"active\" ones still run; \"done\" ones ran or fired and \
             stopped):\n{}{more}",
            lines.join("\n")
        )
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.routines.len();
        self.routines.retain(|r| r.id != id);
        self.routines.len() != before
    }

    /// The first active routine whose time has come.
    pub fn due(&self, now: u64) -> Option<&Routine> {
        self.routines
            .iter()
            .filter(|r| r.is_active() && r.next_at <= now)
            .min_by_key(|r| r.next_at)
    }

    /// A timed routine ran (or failed: `result` says so).
    pub fn record_run(&mut self, id: u64, now: u64, result: &str) {
        if let Some(routine) = self.routines.iter_mut().find(|r| r.id == id) {
            routine.after_run(now, result);
        }
    }

    /// A watch was checked: schedule the next check, and when it happened,
    /// remember it (and stop, for a one-time watch).
    pub fn record_check(&mut self, id: u64, now: u64, check: &Check) {
        let Some(routine) = self.routines.iter_mut().find(|r| r.id == id) else {
            return;
        };
        let Trigger::When {
            every_minutes,
            once,
            ..
        } = routine.trigger
        else {
            return;
        };
        routine.last_checked = Some(now);
        routine.next_at = now + every_minutes * 60;
        if check.happened {
            routine.last_result = clean(&check.summary);
            routine.fired.push(check.summary.clone());
            let extra = routine.fired.len().saturating_sub(MAX_FIRED);
            routine.fired.drain(..extra);
            if once {
                routine.finish(now);
            }
        }
    }

    /// A check failed: try again at the normal pace.
    pub fn postpone(&mut self, id: u64, now: u64, reason: &str) {
        if let Some(routine) = self.routines.iter_mut().find(|r| r.id == id) {
            let minutes = match routine.trigger {
                Trigger::When { every_minutes, .. } => every_minutes,
                Trigger::At { .. } => DEFAULT_CHECK_MINUTES,
            };
            routine.next_at = now + minutes * 60;
            routine.last_result = clean(&format!("failed: {reason}"));
        }
    }

    /// Forgets routines finished over [`KEEP_DONE_SECS`] ago.
    pub fn prune(&mut self, now: u64) {
        self.routines.retain(|r| {
            r.done_at
                .is_none_or(|done| now.saturating_sub(done) < KEEP_DONE_SECS)
        });
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| {
                format!(
                    "Could not read scheduled tasks from {}: {e}",
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
            format!("Could not save scheduled tasks to {}: {e}", path.display())
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

/// `~/Library/Application Support/Meow/routines.json`.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/Meow/routines.json"))
}

/// Added to chat messages: the scheduled tasks there are, and how to report
/// a scheduling request.
pub fn instruction(now: u64, routines: &Routines) -> String {
    format!(
        "(Meow note, not from the user: {} If this message asks to do something later, at a \
         given time, or when something happens, do not do it now. Reply with one short \
         sentence confirming what will happen, then a fenced code block with the info string \
         `{SCHEDULE_FENCE}` holding {{\"name\": short title, \"kind\": \"at\" or \"when\", \
         \"at\": \"YYYY-MM-DDTHH:MM\" local time (for \"at\"), \"daily\": true to repeat every \
         day, \"condition\": what to watch for, precise enough to check in my connectors (for \
         \"when\"), \"every_minutes\": how often to check (default {DEFAULT_CHECK_MINUTES}), \
         \"once\": false to keep watching after the first time, \"then\": what to do, written \
         as an instruction, \"notify_only\": true when it only means letting me know}}. \
         To change one of my scheduled tasks, or start a finished one again, send the same \
         block with its \"id\" and all its fields as they should now be (copy the ones that \
         stay); to cancel one, send {{\"kind\": \"cancel\", \"id\": its id}}. Only one block \
         per reply. Otherwise ignore this note.\n\n{})",
        clock::describe(now),
        routines.describe_for_claude(now)
    )
}

/// Splits Claude's answer into the text to show and the request it reported.
pub fn extract(answer: &str) -> (String, Option<Spec>) {
    let (shown, body) = take_fence(answer, SCHEDULE_FENCE);
    let spec = body.and_then(|body| serde_json::from_str(&body).ok());
    (shown, spec)
}

/// The prompt that runs a timed routine.
pub fn run_prompt(routine: &Routine, now: u64) -> String {
    format!(
        "{} This is a scheduled task I asked for earlier (\"{}\"). Do it now: {}\n\nWhen done, \
         reply with a short summary of what you did, in one or two sentences.",
        clock::describe(now),
        routine.name,
        routine.then
    )
}

/// The read-only prompt that checks a watch's condition.
pub fn check_prompt(routine: &Routine, now: u64) -> Option<String> {
    let Trigger::When { condition, .. } = &routine.trigger else {
        return None;
    };
    let since = routine.last_checked.unwrap_or(routine.created_at);
    let reported = if routine.fired.is_empty() {
        String::new()
    } else {
        format!(
            "\nAlready reported, do not count again: {}.",
            routine.fired.join("; ")
        )
    };
    Some(format!(
        "{} Background check for a scheduled watch. Only read: do not send, post, reply, \
         create or modify anything. Use my connectors directly and keep it quick.\n\n\
         Has this happened (look at what is new since {}): {condition}{reported}\n\n\
         Reply with only a fenced code block with the info string `{WATCH_FENCE}` holding \
         {{\"happened\": true or false, \"summary\": one line on what happened (empty if not), \
         \"details\": the useful facts, quoted briefly, with links (empty if not)}}.",
        clock::describe(now),
        clock::when_label(since, now)
    ))
}

/// The prompt that acts once a watch fired.
pub fn act_prompt(routine: &Routine, check: &Check, now: u64) -> String {
    format!(
        "{} A scheduled watch I set up fired (\"{}\"): {}\nDetails: {}\n\nDo this now: {}\n\n\
         When done, reply with a short summary of what you did.",
        clock::describe(now),
        routine.name,
        check.summary,
        check.details,
        routine.then
    )
}

/// Reads a watch check's answer.
pub fn parse_check(answer: &str) -> Result<Check, String> {
    let (_, body) = take_fence(answer, WATCH_FENCE);
    let body = body.ok_or("the check returned no result block")?;
    serde_json::from_str(&body).map_err(|e| format!("the check's result is not valid JSON ({e})"))
}

/// The trigger, first run and action `spec` asks for.
fn build(spec: &Spec, now: u64) -> Result<(Trigger, u64, String), String> {
    let then = clean(&spec.then).ok_or("the request does not say what to do")?;
    let (trigger, next_at) = match spec.kind.trim() {
        "at" => {
            let text = spec.at.as_deref().ok_or("the request has no time")?;
            let at = parse_local(text).ok_or_else(|| format!("unreadable time \"{text}\""))?;
            if at <= now && !spec.daily {
                return Err(format!("{text} is already past"));
            }
            let mut next = at;
            while next <= now {
                next += DAY_SECS;
            }
            (
                Trigger::At {
                    at,
                    daily: spec.daily,
                },
                next,
            )
        }
        "when" => {
            let condition = spec
                .condition
                .as_deref()
                .and_then(clean)
                .ok_or("the request does not say what to watch for")?;
            let every = spec
                .every_minutes
                .unwrap_or(DEFAULT_CHECK_MINUTES)
                .clamp(MIN_CHECK_MINUTES, MAX_CHECK_MINUTES);
            (
                Trigger::When {
                    condition,
                    every_minutes: every,
                    once: spec.once,
                },
                now + 60,
            )
        }
        "cancel" => return Err("cancelling needs the task's id".to_string()),
        other => return Err(format!("unknown kind \"{other}\"")),
    };
    Ok((trigger, next_at, then))
}

/// One routine as Claude sees it.
fn describe(routine: &Routine, now: u64) -> serde_json::Value {
    use serde_json::Value;
    let status = if routine.is_active() {
        "active"
    } else {
        "done"
    };
    let mut fields: Vec<(&str, Value)> = vec![
        ("id", routine.id.into()),
        ("status", status.into()),
        ("name", routine.name.clone().into()),
    ];
    match &routine.trigger {
        Trigger::At { at, daily } => {
            let shown = if routine.is_active() {
                routine.next_at
            } else {
                *at
            };
            fields.push(("kind", "at".into()));
            fields.push(("at", local_text(shown).into()));
            fields.push(("daily", (*daily).into()));
        }
        Trigger::When {
            condition,
            every_minutes,
            once,
        } => {
            fields.push(("kind", "when".into()));
            fields.push(("condition", condition.clone().into()));
            fields.push(("every_minutes", (*every_minutes).into()));
            fields.push(("once", (*once).into()));
        }
    }
    fields.push(("then", routine.then.clone().into()));
    fields.push(("notify_only", routine.notify_only.into()));
    if let Some(result) = &routine.last_result {
        fields.push(("last_result", result.clone().into()));
    }
    if let Some(done) = routine.done_at {
        fields.push(("finished", clock::when_label(done, now).into()));
    }
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

/// "2026-10-20T17:00" on the local clock, as [`parse_local`] reads it.
fn local_text(at: u64) -> Option<String> {
    let t = clock::local(at)?;
    Some(format!(
        "{}-{:02}-{:02}T{:02}:{:02}",
        t.year, t.month, t.day, t.hour, t.minute
    ))
}

/// "2026-10-20T17:00" (or with a space, or seconds) on the local clock.
fn parse_local(text: &str) -> Option<u64> {
    let text = text.trim();
    let (date, time) = text.split_once(['T', ' '])?;
    let mut date = date.split('-').map(|p| p.parse::<i32>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let mut time = time.split(':').map(|p| p.parse::<i32>().ok());
    let (hour, minute) = (time.next()??, time.next()??);
    let valid = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && (0..24).contains(&hour)
        && (0..60).contains(&minute);
    valid
        .then(|| clock::local_timestamp(year, month, day, hour, minute))
        .flatten()
}

fn clean(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(MAX_TEXT_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_792_411_200;

    fn at_spec(at: &str, daily: bool) -> Spec {
        Spec {
            name: "Send report".into(),
            kind: "at".into(),
            at: Some(at.into()),
            daily,
            then: "Send the weekly report".into(),
            once: true,
            ..Spec::default()
        }
    }

    fn watch_spec() -> Spec {
        Spec {
            name: "Staples reply".into(),
            kind: "when".into(),
            condition: Some("Staples replied to Cyril's email".into()),
            then: "Let me know".into(),
            notify_only: true,
            once: true,
            ..Spec::default()
        }
    }

    fn local(at: u64) -> String {
        local_text(at).unwrap()
    }

    #[test]
    fn extract_strips_the_block() {
        let answer = "I'll tell you.\n```meow-schedule\n{\"name\": \"Staples\", \"kind\": \
            \"when\", \"condition\": \"Staples replied\", \"then\": \"Tell me\", \
            \"notify_only\": true}\n```";
        let (shown, spec) = extract(answer);
        assert_eq!(shown, "I'll tell you.");
        let spec = spec.unwrap();
        assert!(spec.once && spec.notify_only);
        assert_eq!(extract("hi").1, None);
    }

    #[test]
    fn timed_routines_run_once_at_their_time() {
        let mut routines = Routines::default();
        let id = routines
            .add(&at_spec(&local(NOW + 3600), false), NOW)
            .unwrap()
            .id;
        assert!(routines.due(NOW + 60).is_none());
        assert_eq!(routines.due(NOW + 3600).map(|r| r.id), Some(id));
        routines.record_run(id, NOW + 3600, "Sent.");
        let routine = routines.get(id).unwrap();
        assert!(!routine.is_active());
        assert_eq!(routine.last_result.as_deref(), Some("Sent."));
        assert!(routines.due(NOW + 7200).is_none());
    }

    #[test]
    fn daily_routines_move_to_the_next_day() {
        let mut routines = Routines::default();
        let id = routines
            .add(&at_spec(&local(NOW + 60), true), NOW)
            .unwrap()
            .id;
        routines.record_run(id, NOW + 120, "ok");
        let routine = routines.get(id).unwrap();
        assert!(routine.is_active());
        assert_eq!(routine.next_at, NOW + 60 + DAY_SECS);
    }

    #[test]
    fn past_times_are_refused_unless_daily() {
        let mut routines = Routines::default();
        assert!(
            routines
                .add(&at_spec(&local(NOW - 60), false), NOW)
                .is_err()
        );
        let daily = routines.add(&at_spec(&local(NOW - 60), true), NOW).unwrap();
        assert_eq!(daily.next_at, NOW - 60 + DAY_SECS);
        assert!(routines.add(&at_spec("tomorrow", false), NOW).is_err());
    }

    #[test]
    fn watches_check_until_it_happens_once() {
        let mut routines = Routines::default();
        let id = routines.add(&watch_spec(), NOW).unwrap().id;
        assert_eq!(routines.due(NOW + 60).map(|r| r.id), Some(id));
        let nothing = Check::default();
        routines.record_check(id, NOW + 60, &nothing);
        assert_eq!(routines.get(id).unwrap().next_at, NOW + 60 + 600);
        let prompt = check_prompt(routines.get(id).unwrap(), NOW + 700).unwrap();
        assert!(prompt.contains("Staples replied") && prompt.contains("Only read"));
        let found = Check {
            happened: true,
            summary: "Staples answered: delivery Friday".into(),
            details: "".into(),
        };
        routines.record_check(id, NOW + 700, &found);
        let routine = routines.get(id).unwrap();
        assert!(!routine.is_active());
        assert_eq!(routine.fired, ["Staples answered: delivery Friday"]);
    }

    #[test]
    fn repeating_watches_remember_what_they_reported() {
        let mut routines = Routines::default();
        let spec = Spec {
            once: false,
            ..watch_spec()
        };
        let id = routines.add(&spec, NOW).unwrap().id;
        let found = Check {
            happened: true,
            summary: "first reply".into(),
            details: "".into(),
        };
        routines.record_check(id, NOW + 60, &found);
        let routine = routines.get(id).unwrap();
        assert!(routine.is_active());
        assert!(
            check_prompt(routine, NOW + 700)
                .unwrap()
                .contains("first reply")
        );
    }

    #[test]
    fn incomplete_requests_are_refused() {
        let mut routines = Routines::default();
        let no_action = Spec {
            then: " ".into(),
            ..watch_spec()
        };
        assert!(routines.add(&no_action, NOW).is_err());
        let no_condition = Spec {
            condition: None,
            ..watch_spec()
        };
        assert!(routines.add(&no_condition, NOW).is_err());
        let odd = Spec {
            kind: "sometimes".into(),
            ..watch_spec()
        };
        assert!(routines.add(&odd, NOW).is_err());
    }

    #[test]
    fn check_frequency_is_bounded() {
        let mut routines = Routines::default();
        let spec = Spec {
            every_minutes: Some(1),
            ..watch_spec()
        };
        let routine = routines.add(&spec, NOW).unwrap();
        assert!(matches!(
            routine.trigger,
            Trigger::When {
                every_minutes: 5,
                ..
            }
        ));
    }

    #[test]
    fn parses_check_answers() {
        let check = parse_check(
            "```meow-watch\n{\"happened\": true, \"summary\": \"s\", \"details\": \"d\"}\n```",
        )
        .unwrap();
        assert!(check.happened);
        assert!(parse_check("no").is_err());
        assert!(
            parse_check("```meow-watch\n{}\n```").is_err(),
            "happened is required"
        );
    }

    #[test]
    fn display_lists_active_first_and_prune_forgets_old_ones() {
        let mut routines = Routines::default();
        let late = routines
            .add(&at_spec(&local(NOW + 7200), false), NOW)
            .unwrap()
            .id;
        let soon = routines
            .add(&at_spec(&local(NOW + 3600), false), NOW)
            .unwrap()
            .id;
        let done = routines.add(&watch_spec(), NOW).unwrap().id;
        routines.record_check(
            done,
            NOW,
            &Check {
                happened: true,
                ..Check::default()
            },
        );
        let order: Vec<u64> = routines.display().iter().map(|r| r.id).collect();
        assert_eq!(order, [soon, late, done]);
        routines.prune(NOW + KEEP_DONE_SECS + 1);
        assert!(routines.get(done).is_none() && routines.get(soon).is_some());
        assert!(routines.remove(soon) && !routines.remove(soon));
    }

    #[test]
    fn finished_routines_can_be_started_again() {
        let mut routines = Routines::default();
        let id = routines.add(&watch_spec(), NOW).unwrap().id;
        let found = Check {
            happened: true,
            summary: "Staples answered".into(),
            details: "".into(),
        };
        routines.record_check(id, NOW + 60, &found);
        assert!(!routines.get(id).unwrap().is_active());
        let again = Spec {
            id: Some(id),
            ..watch_spec()
        };
        let Ok(Change::Updated(routine)) = routines.apply(&again, NOW + 3600) else {
            panic!("expected an update");
        };
        assert!(routine.is_active() && routine.done_at.is_none());
        assert_eq!(routine.id, id);
        assert_eq!(routine.next_at, NOW + 3600 + 60);
        assert_eq!(
            routine.fired,
            ["Staples answered"],
            "still not reported twice"
        );
        assert_eq!(routines.active_count(), 1);
    }

    #[test]
    fn routines_can_be_rescheduled_and_cancelled() {
        let mut routines = Routines::default();
        let id = routines
            .add(&at_spec(&local(NOW + 3600), false), NOW)
            .unwrap()
            .id;
        let later = Spec {
            id: Some(id),
            name: String::new(),
            ..at_spec(&local(NOW + 7200), true)
        };
        let Ok(Change::Updated(routine)) = routines.apply(&later, NOW) else {
            panic!("expected an update");
        };
        assert_eq!(
            routine.name, "Send report",
            "an empty name keeps the old one"
        );
        assert_eq!(routine.next_at, NOW + 7200);
        let cancel = Spec {
            id: Some(id),
            kind: "cancel".into(),
            ..Spec::default()
        };
        assert!(matches!(
            routines.apply(&cancel, NOW),
            Ok(Change::Cancelled(_))
        ));
        assert!(routines.get(id).is_none());
        assert!(routines.apply(&cancel, NOW).is_err(), "unknown id");
        let no_id = Spec { id: None, ..cancel };
        assert!(routines.apply(&no_id, NOW).is_err());
    }

    #[test]
    fn claude_sees_every_routine_as_a_request() {
        let mut routines = Routines::default();
        assert!(instruction(NOW, &routines).contains("no scheduled tasks"));
        routines
            .add(&at_spec(&local(NOW + 3600), true), NOW)
            .unwrap();
        let watch = routines.add(&watch_spec(), NOW).unwrap().id;
        routines.record_check(
            watch,
            NOW + 60,
            &Check {
                happened: true,
                summary: "Staples answered".into(),
                details: "".into(),
            },
        );
        let text = routines.describe_for_claude(NOW + 120);
        let lines: Vec<serde_json::Value> = text
            .lines()
            .skip(1)
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["status"], "active");
        assert_eq!(lines[0]["at"], local(NOW + 3600));
        assert_eq!(lines[1]["status"], "done");
        assert_eq!(lines[1]["condition"], "Staples replied to Cyril's email");
        assert_eq!(lines[1]["last_result"], "Staples answered");
        // A listed routine reads back as a request for itself.
        let spec: Spec = serde_json::from_value(lines[1].clone()).unwrap();
        assert_eq!(spec.id, Some(watch));
        assert!(routines.apply(&spec, NOW + 180).is_ok());
        assert!(routines.get(watch).unwrap().is_active());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("meow-routines-{}", std::process::id()));
        let path = dir.join("routines.json");
        let mut routines = Routines::default();
        routines.add(&watch_spec(), NOW).unwrap();
        routines.save(&path).unwrap();
        assert_eq!(Routines::load(&path).unwrap(), routines);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
