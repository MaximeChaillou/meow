//! The background check of tasks: every few minutes, a quiet Claude session
//! ticks off resolved tasks and adds new ones. It runs next to the chat, never
//! blocks it, and reports only in the Tasks header.

use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use meow::activity::{self, Kind, Link, Note};
use meow::claude::{ClaudeEvent, Run};
use meow::clock;
use meow::{sync, tasks};

use super::{App, now_secs};

const DEFAULT_MINUTES: u64 = 5;
/// The first check waits a little after launch, so startup stays quick.
const FIRST_DELAY: Duration = Duration::from_secs(20);
/// How often "checked N min ago" is refreshed. A failed check is retried at
/// the normal pace, so a broken connector does not burn usage.
const STATUS_REFRESH: Duration = Duration::from_secs(30);

pub(super) struct SyncState {
    /// `None` when checks are turned off (`MEOW_SYNC_MINUTES=0`).
    interval: Option<Duration>,
    next_at: Instant,
    run: Option<Run>,
    started_at: u64,
    /// Time of the last successful check, for "checked 3 min ago" and the
    /// next check's look-back window.
    last_ok: Option<u64>,
    last_failed: bool,
    status_shown_at: Instant,
}

impl SyncState {
    /// Reads `MEOW_SYNC_MINUTES` (default 5, 0 turns checks off).
    pub(super) fn from_env() -> Self {
        let minutes = std::env::var("MEOW_SYNC_MINUTES")
            .ok()
            .and_then(|m| m.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_MINUTES);
        Self {
            interval: (minutes > 0).then(|| Duration::from_secs(minutes * 60)),
            next_at: Instant::now() + FIRST_DELAY,
            run: None,
            started_at: 0,
            last_ok: None,
            last_failed: false,
            status_shown_at: Instant::now(),
        }
    }
}

impl App {
    /// Starts a check when one is due, and collects the one in flight.
    pub(super) fn poll_sync(&mut self) {
        if self.sync.run.is_some() {
            self.collect_sync();
        } else if self.sync_due() {
            self.start_sync();
        }
        if self.sync.status_shown_at.elapsed() >= STATUS_REFRESH {
            self.show_sync_status();
        }
    }

    fn sync_due(&self) -> bool {
        let ready = matches!(self.claude, Some(Ok(_)));
        ready && self.sync.interval.is_some() && Instant::now() >= self.sync.next_at
    }

    fn start_sync(&mut self) {
        let Some(Ok(env)) = &self.claude else {
            return;
        };
        let now = now_secs();
        let prompt = sync::prompt(
            &self.tasks.open(),
            &self.tasks.recently_done(sync::MAX_HANDLED),
            self.sync.last_ok,
            now,
        );
        match Run::start(env, &prompt, None) {
            Ok(run) => {
                self.sync.run = Some(run);
                self.sync.started_at = now;
            }
            Err(error) => self.finish_sync(Err(error)),
        }
        self.show_sync_status();
    }

    fn collect_sync(&mut self) {
        loop {
            let Some(run) = &self.sync.run else { return };
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
                Err(TryRecvError::Disconnected) => Err("the check stopped early".to_string()),
            };
            return self.finish_sync(outcome);
        }
    }

    fn finish_sync(&mut self, outcome: Result<String, String>) {
        self.sync.run = None;
        if let Some(interval) = self.sync.interval {
            self.sync.next_at = Instant::now() + interval;
        }
        match outcome.and_then(|answer| sync::parse(&answer)) {
            Ok(report) => {
                let now = now_secs();
                let closed_titles: Vec<String> = report
                    .done
                    .iter()
                    .filter_map(|id| self.tasks.get(id))
                    .filter(|task| !task.is_done())
                    .map(|task| task.title.clone())
                    .collect();
                let new_titles: Vec<&str> = report
                    .new
                    .iter()
                    .filter(|found| {
                        tasks::id_for(found).is_some() && self.tasks.find(found).is_none()
                    })
                    .map(|found| found.title.as_str())
                    .collect();
                let closed = self.tasks.close_resolved(&report.done, now);
                let added = self.tasks.merge(&report.new, now);
                if closed + added > 0 {
                    self.tasks_changed();
                }
                self.notify_task_changes(&new_titles, &closed_titles);
                // New things are looked for from when this check started.
                self.sync.last_ok = Some(self.sync.started_at);
                self.sync.last_failed = false;
            }
            Err(error) => {
                eprintln!("meow: task check failed: {error}");
                self.sync.last_failed = true;
            }
        }
        self.show_sync_status();
    }

    /// One Activity item for the tasks a check added, one for those it closed.
    fn notify_task_changes(&mut self, added: &[&str], closed: &[String]) {
        let count = |n: usize, what: &str| match n {
            1 => format!("1 {what}"),
            n => format!("{n} {what}s"),
        };
        if !added.is_empty() {
            self.notify(Note {
                kind: Kind::Tasks,
                title: format!("{} waiting on you", count(added.len(), "new task")),
                detail: Some(activity::list_titles(added, 3)),
                link: Some(Link::Tasks),
            });
        }
        if !closed.is_empty() {
            let titles: Vec<&str> = closed.iter().map(String::as_str).collect();
            self.notify(Note {
                kind: Kind::Tasks,
                title: format!("{} resolved at the source", count(closed.len(), "task")),
                detail: Some(activity::list_titles(&titles, 3)),
                link: Some(Link::Tasks),
            });
        }
    }

    /// "Tasks · checked at 09:40" in the Tasks header.
    fn show_sync_status(&mut self) {
        self.sync.status_shown_at = Instant::now();
        let status = if self.sync.interval.is_none() {
            None
        } else if self.sync.run.is_some() {
            Some("checking…".to_string())
        } else if self.sync.last_failed {
            Some("last check failed".to_string())
        } else {
            self.sync
                .last_ok
                .and_then(clock::local)
                .map(|at| format!("checked at {:02}:{:02}", at.hour, at.minute))
        };
        let title = match status {
            Some(status) => format!("Tasks · {status}"),
            None => "Tasks".to_string(),
        };
        self.chat.set_tasks_title(&title);
    }
}
