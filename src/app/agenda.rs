//! Keeps today's plan agenda current: every few minutes, a quiet Claude run
//! limited to the calendar's read tools reports today's events, and they
//! replace the agenda of each plan made today. It never blocks the chat.

use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use meow::actions::Layout;
use meow::activity::{Kind, Link, Note};
use meow::agenda::{self, Change};
use meow::claude::{ClaudeEvent, Run};
use meow::clock;

use super::{App, now_secs};

const DEFAULT_MINUTES: u64 = 15;
/// The first refresh waits a little after launch, so startup stays quick.
const FIRST_DELAY: Duration = Duration::from_secs(30);

pub(super) struct AgendaState {
    /// `None` when refreshes are turned off (`MEOW_AGENDA_MINUTES=0`).
    interval: Option<Duration>,
    next_at: Instant,
    run: Option<Run>,
}

impl AgendaState {
    /// Reads `MEOW_AGENDA_MINUTES` (default 15, 0 turns refreshes off).
    pub(super) fn from_env() -> Self {
        let minutes = std::env::var("MEOW_AGENDA_MINUTES")
            .ok()
            .and_then(|m| m.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_MINUTES);
        Self {
            interval: (minutes > 0).then(|| Duration::from_secs(minutes * 60)),
            next_at: Instant::now() + FIRST_DELAY,
            run: None,
        }
    }

    /// A refresh is in flight.
    pub(super) fn busy(&self) -> bool {
        self.run.is_some()
    }
}

impl App {
    /// Starts a refresh when one is due, and collects the one in flight.
    pub(super) fn poll_agenda(&mut self) {
        if self.agenda.run.is_some() {
            self.collect_agenda();
        } else if let Some(interval) = self.agenda.interval
            && Instant::now() >= self.agenda.next_at
        {
            self.agenda.next_at = Instant::now() + interval;
            self.start_agenda(interval);
        }
    }

    /// Titles of the plan actions whose plan was made today, and not just now.
    fn plans_to_refresh(&self, interval: Duration) -> Vec<String> {
        let now = now_secs();
        let Some(today) = clock::local(now) else {
            return Vec::new();
        };
        self.actions
            .iter()
            .filter(|action| action.layout == Layout::Plan)
            .filter(|action| {
                self.action_results
                    .get(&action.title)
                    .filter(|result| result.plan.is_some())
                    .is_some_and(|result| {
                        let fresh = now.saturating_sub(result.ran_at) < interval.as_secs();
                        let made_today =
                            clock::local(result.ran_at).is_some_and(|at| at.same_day(&today));
                        made_today && !fresh
                    })
            })
            .map(|action| action.title.clone())
            .collect()
    }

    fn start_agenda(&mut self, interval: Duration) {
        let (Some(Ok(env)), Some(tools)) = (&self.claude, self.calendar.tools()) else {
            return;
        };
        if self.plans_to_refresh(interval).is_empty() {
            return;
        }
        match Run::start_restricted(env, &agenda::prompt(now_secs()), &env.model, &tools.read) {
            Ok(run) => self.agenda.run = Some(run),
            Err(error) => eprintln!("meow: could not refresh the agenda: {error}"),
        }
    }

    fn collect_agenda(&mut self) {
        loop {
            let Some(run) = &self.agenda.run else { return };
            let outcome = match run.events.try_recv() {
                Ok(ClaudeEvent::Done {
                    is_error: false,
                    result,
                }) => Ok(result),
                Ok(ClaudeEvent::Done {
                    is_error: true,
                    result,
                }) => Err(result),
                Ok(ClaudeEvent::Failed(reason)) => Err(reason),
                Ok(_) => continue,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err("the refresh stopped early".to_string()),
            };
            self.agenda.run = None;
            return self.finish_agenda(outcome.and_then(|answer| agenda::parse(&answer)));
        }
    }

    fn finish_agenda(&mut self, outcome: Result<Vec<meow::plan::Event>, String>) {
        let events = match outcome {
            Ok(events) => events,
            Err(error) => return eprintln!("meow: agenda refresh failed: {error}"),
        };
        // Zero: the plan made at start-up counts as fresh, never as stale.
        let titles = self.plans_to_refresh(Duration::ZERO);
        let Some(first) = titles.first().cloned() else {
            return;
        };
        let old = self
            .action_results
            .get(&first)
            .and_then(|result| result.plan.as_ref())
            .map(|plan| plan.agenda.clone())
            .unwrap_or_default();
        let mut changed = false;
        for title in &titles {
            changed |= self.action_results.set_agenda(title, events.clone());
        }
        if !changed {
            return;
        }
        self.save_action_results();
        self.render_plan();
        // Compared once cleaned, the way the plan keeps them.
        let new = self
            .action_results
            .get(&first)
            .and_then(|result| result.plan.as_ref())
            .map(|plan| plan.agenda.clone())
            .unwrap_or_default();
        let now_minutes = clock::local(now_secs()).map_or(0, |t| t.minutes_of_day());
        for change in agenda::changes(&old, &new, now_minutes) {
            self.notify(calendar_note(change, &first));
        }
    }
}

/// The Activity item for one agenda change, linking to the plan.
fn calendar_note(change: Change, plan: &str) -> Note {
    let at = |start: &Option<String>| start.clone().unwrap_or_else(|| "all day".to_string());
    let (title, detail) = match change {
        Change::Added(event) => (format!("New event: {}", event.title), at(&event.start)),
        Change::Cancelled(event) => (
            format!("Cancelled: {}", event.title),
            format!("was at {}", at(&event.start)),
        ),
        Change::Moved { event, from } => (
            format!("Moved: {}", event.title),
            format!("{} → {}", at(&from), at(&event.start)),
        ),
        Change::Invited(event) => (
            format!("Invitation: {}", event.title),
            format!("{} · waiting for your answer", at(&event.start)),
        ),
    };
    Note {
        kind: Kind::Calendar,
        title,
        detail: Some(detail),
        link: Some(Link::Plan {
            action: plan.to_string(),
        }),
    }
}
