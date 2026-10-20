//! Accepting or declining invitations from the plan's agenda: each answer is
//! a run that may only call the calendar's respond tool. The event shows as
//! sending until it is done, then the plan records the answer.

use std::sync::mpsc::TryRecvError;

use meow::activity::{Kind, Note};
use meow::claude::{ClaudeEvent, Run};
use meow::plan::Rsvp;
use meow::rsvp;

use super::App;

struct Pending {
    run: Run,
    event_id: String,
    title: String,
    answer: Rsvp,
}

#[derive(Default)]
pub(super) struct RsvpState {
    pending: Vec<Pending>,
}

impl RsvpState {
    /// Ids of the events whose answer is being sent.
    pub(super) fn sending(&self) -> Vec<String> {
        self.pending.iter().map(|p| p.event_id.clone()).collect()
    }

    pub(super) fn busy(&self) -> bool {
        !self.pending.is_empty()
    }
}

impl App {
    /// Answers the invitation of agenda event `index`, as last rendered.
    pub(super) fn answer_plan_event(&mut self, index: usize, answer: Rsvp) {
        let Some(event) = self.chat.plan().event_at(index) else {
            return;
        };
        let (Some(event_id), Some(prompt)) = (event.id.clone(), rsvp::prompt(&event, answer))
        else {
            return;
        };
        if self.rsvp.pending.iter().any(|p| p.event_id == event_id) {
            return;
        }
        let started = match (&self.claude, self.calendar.tools()) {
            (Some(Ok(env)), Some(tools)) if !tools.respond.is_empty() => {
                Run::start_restricted(env, &prompt, &env.model, &tools.respond)
            }
            (Some(Ok(_)), Some(_)) => {
                Err("Your calendar connector cannot answer invitations.".to_string())
            }
            _ => Err("No calendar connector found in Claude Code.".to_string()),
        };
        match started {
            Ok(run) => {
                self.rsvp.pending.push(Pending {
                    run,
                    event_id,
                    title: event.title.clone(),
                    answer,
                });
                self.render_plan();
            }
            Err(error) => self.rsvp_failed(&event.title, &error),
        }
    }

    /// Collects the answers that are done.
    pub(super) fn poll_rsvp(&mut self) {
        if self.rsvp.pending.is_empty() {
            return;
        }
        let (finished, still): (Vec<_>, Vec<_>) = std::mem::take(&mut self.rsvp.pending)
            .into_iter()
            .map(|pending| (outcome(&pending.run), pending))
            .partition(|(outcome, _)| outcome.is_some());
        self.rsvp.pending = still.into_iter().map(|(_, pending)| pending).collect();
        if finished.is_empty() {
            return;
        }
        for (outcome, pending) in finished {
            match outcome {
                Some(Ok(())) => {
                    if self
                        .action_results
                        .set_response(&pending.event_id, pending.answer)
                    {
                        self.save_action_results();
                    }
                    let verb = match pending.answer {
                        Rsvp::Accepted => "Accepted",
                        _ => "Declined",
                    };
                    self.notify(Note {
                        kind: Kind::Calendar,
                        title: format!("{verb}: {}", pending.title),
                        detail: None,
                        link: None,
                    });
                }
                Some(Err(reason)) => self.rsvp_failed(&pending.title, &reason),
                None => {}
            }
        }
        self.render_plan();
    }

    fn rsvp_failed(&mut self, what: &str, reason: &str) {
        eprintln!("meow: could not answer {what}: {reason}");
        self.notify(Note {
            kind: Kind::Calendar,
            title: format!("Could not answer: {what}"),
            detail: Some(reason.to_string()),
            link: None,
        });
        let active = self.conversations.active().clone();
        self.error_in(
            &active,
            &format!("Could not answer the invitation: {reason}"),
        );
    }
}

/// `None` while the run goes on; then whether it succeeded.
fn outcome(run: &Run) -> Option<Result<(), String>> {
    loop {
        return match run.events.try_recv() {
            Ok(ClaudeEvent::Done {
                is_error: false,
                result,
            }) => Some(rsvp::parse(&result)),
            Ok(ClaudeEvent::Done {
                is_error: true,
                result,
            }) => Some(Err(result)),
            Ok(ClaudeEvent::Failed(reason)) => Some(Err(reason)),
            Ok(_) => continue,
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the run stopped early".to_string())),
        };
    }
}
