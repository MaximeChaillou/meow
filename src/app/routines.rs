//! Scheduled tasks on the app side: asked for in the chat, kept on the
//! Scheduled page, and run in the background next to the chat, one at a time.
//! A timed task runs with the chat's permissions; a watch is checked
//! read-only, and acts (or just tells the user) once its condition is met.
//! Every outcome goes to Activity.

use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use meow::activity::{Kind, Note};
use meow::claude::{ClaudeEvent, Run};
use meow::conversations::ChatId;
use meow::routines::{self, Change, Check, Routines, Spec, Trigger};

use crate::ui::sidebar::Page;

use super::{App, now_secs};

const CHECK_EVERY: Duration = Duration::from_secs(30);
/// The first look waits a little after launch, so startup stays quick.
const FIRST_CHECK: Duration = Duration::from_secs(25);

/// What the run in flight is doing for its routine.
enum Stage {
    /// A timed routine doing its job.
    Running,
    /// A watch checking its condition, read-only.
    Checking,
    /// A watch that fired, doing what it was asked to.
    Acting(Check),
}

struct Active {
    run: Run,
    id: u64,
    stage: Stage,
}

pub(super) struct RoutineState {
    pub(super) routines: Routines,
    /// Where they are saved; `None` when the file could not be read.
    path: Option<std::path::PathBuf>,
    next_check: Instant,
    active: Option<Active>,
}

impl RoutineState {
    /// The saved routines, plus the reason they could not be read.
    pub(super) fn load() -> (Self, Option<String>) {
        let state = |routines, path| Self {
            routines,
            path,
            next_check: Instant::now() + FIRST_CHECK,
            active: None,
        };
        let Some(path) = routines::default_path() else {
            return (state(Routines::default(), None), None);
        };
        match Routines::load(&path) {
            Ok(mut list) => {
                list.prune(now_secs());
                (state(list, Some(path)), None)
            }
            Err(error) => (
                state(Routines::default(), None),
                Some(format!("{error}. Scheduled tasks won't be saved.")),
            ),
        }
    }

    /// The id of the routine being run or checked.
    pub(super) fn running(&self) -> Option<u64> {
        self.active.as_ref().map(|active| active.id)
    }
}

impl App {
    /// The chat reported a scheduling request: add, change or cancel the
    /// routine and say so in `chat`.
    pub(super) fn apply_routine(&mut self, chat: &ChatId, spec: &Spec) {
        let now = now_secs();
        let line = match self.routines.routines.apply(spec, now) {
            Ok(Change::Added(routine)) => format!(
                "Scheduled: {} · {} (see Scheduled)",
                routine.name,
                routine.schedule_label(now)
            ),
            Ok(Change::Updated(routine)) => {
                self.stop_routine_run(routine.id);
                format!(
                    "Updated: {} · {} (see Scheduled)",
                    routine.name,
                    routine.schedule_label(now)
                )
            }
            Ok(Change::Cancelled(routine)) => {
                self.stop_routine_run(routine.id);
                format!("Cancelled: {}", routine.name)
            }
            Err(error) => {
                return self.error_in(chat, &format!("Could not schedule that: {error}."));
            }
        };
        self.info_in(chat, &line);
        self.routines_changed();
    }

    /// Drops the run in flight for routine `id`, which changed or went away.
    fn stop_routine_run(&mut self, id: u64) {
        if self.routines.running() == Some(id) {
            self.routines.active = None;
        }
    }

    /// Starts the routine that is due, and collects the run in flight.
    pub(super) fn poll_routines(&mut self) {
        if self.routines.active.is_some() {
            return self.collect_routine();
        }
        if Instant::now() < self.routines.next_check {
            return;
        }
        self.routines.next_check = Instant::now() + CHECK_EVERY;
        let now = now_secs();
        let Some(routine) = self.routines.routines.due(now).cloned() else {
            return;
        };
        let Some(Ok(env)) = &self.claude else {
            return;
        };
        let started = match &routine.trigger {
            Trigger::At { .. } => Run::start(env, &routines::run_prompt(&routine, now), None)
                .map(|run| (run, Stage::Running)),
            Trigger::When { .. } => {
                let Some(prompt) = routines::check_prompt(&routine, now) else {
                    return;
                };
                Run::start_read_only(env, &prompt, &env.model).map(|run| (run, Stage::Checking))
            }
        };
        match started {
            Ok((run, stage)) => {
                self.routines.active = Some(Active {
                    run,
                    id: routine.id,
                    stage,
                });
                self.render_routines();
            }
            Err(error) => {
                self.routines.routines.postpone(routine.id, now, &error);
                self.routines_changed();
            }
        }
    }

    fn collect_routine(&mut self) {
        let outcome = loop {
            let Some(active) = &self.routines.active else {
                return;
            };
            break match active.run.events.try_recv() {
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
                Err(TryRecvError::Disconnected) => Err("the run stopped early".to_string()),
            };
        };
        let Some(Active { id, stage, .. }) = self.routines.active.take() else {
            return;
        };
        match stage {
            Stage::Running => self.routine_ran(id, outcome),
            // What the watch found, then what was done about it.
            Stage::Acting(check) => self.routine_ran(
                id,
                outcome.map(|done| format!("{} — {}", check.summary.trim(), done.trim())),
            ),
            Stage::Checking => self.routine_checked(id, outcome),
        }
        self.routines_changed();
    }

    /// A timed routine, or a fired watch's action, finished.
    fn routine_ran(&mut self, id: u64, outcome: Result<String, String>) {
        let now = now_secs();
        let Some(name) = self.routines.routines.get(id).map(|r| r.name.clone()) else {
            return;
        };
        let (title, detail) = match &outcome {
            Ok(summary) => (format!("Done: {name}"), summary.trim().to_string()),
            Err(reason) => (format!("Failed: {name}"), reason.clone()),
        };
        let result = match &outcome {
            Ok(summary) => summary.clone(),
            Err(reason) => format!("failed: {reason}"),
        };
        if matches!(
            self.routines.routines.get(id).map(|r| &r.trigger),
            Some(Trigger::At { .. })
        ) {
            self.routines.routines.record_run(id, now, &result);
        }
        self.notify(Note {
            kind: Kind::Scheduled,
            title,
            detail: Some(detail),
            link: None,
            sources: Vec::new(),
            key: None,
        });
    }

    /// A watch was checked: tell the user, or act, when it happened.
    fn routine_checked(&mut self, id: u64, outcome: Result<String, String>) {
        let now = now_secs();
        let check = match outcome.and_then(|answer| routines::parse_check(&answer)) {
            Ok(check) => check,
            Err(error) => {
                eprintln!("meow: scheduled watch check failed: {error}");
                self.routines.routines.postpone(id, now, &error);
                return;
            }
        };
        self.routines.routines.record_check(id, now, &check);
        if !check.happened {
            return;
        }
        let Some(routine) = self.routines.routines.get(id).cloned() else {
            return;
        };
        if routine.notify_only {
            let detail = [check.summary.trim(), check.details.trim()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" — ");
            return self.notify(Note {
                kind: Kind::Scheduled,
                title: routine.name.clone(),
                detail: Some(detail),
                link: None,
                sources: Vec::new(),
                key: None,
            });
        }
        let Some(Ok(env)) = &self.claude else {
            return;
        };
        match Run::start(env, &routines::act_prompt(&routine, &check, now), None) {
            Ok(run) => {
                self.routines.active = Some(Active {
                    run,
                    id,
                    stage: Stage::Acting(check),
                });
            }
            Err(error) => self.routine_ran(id, Err(error)),
        }
    }

    pub(super) fn show_routines(&mut self) {
        if !self.chat.is_visible() {
            self.open_window();
        }
        self.render_routines();
        self.chat.show_page(Page::Routines);
    }

    pub(super) fn delete_routine(&mut self, row: usize) {
        let Some(id) = self.chat.routines().id_at(row) else {
            return;
        };
        // Dropping the run stops it.
        self.stop_routine_run(id);
        if self.routines.routines.remove(id) {
            self.routines_changed();
        }
    }

    fn routines_changed(&mut self) {
        self.routines.routines.prune(now_secs());
        if let Some(path) = &self.routines.path
            && let Err(error) = self.routines.routines.save(path)
        {
            eprintln!("meow: {error}");
        }
        self.render_routines();
    }

    pub(super) fn render_routines(&self) {
        self.chat
            .render_routines(&self.routines.routines, self.routines.running(), now_secs());
    }
}
