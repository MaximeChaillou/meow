//! Calendar events from a Slack conversation, through the calendar connector
//! of the user's Claude Code, whichever it is.
//!
//! The calendar tools are found at launch (see `super::calendar`). A
//! scheduling request goes: Ask routes it here, the people in the
//! conversation are looked up, Claude plans the event with the calendar's read
//! tools only, the user confirms, and Claude creates it with the create tool
//! only. Each run is limited to those tools; nothing else can be called.

use meow::claude::{Run, STRONG_MODEL};
use meow::slack::assist::{self, Context, Reply};
use meow::slack::calendar::{self, Person, Planned, Proposal};
use meow::slack::client::Command;
use meow::slack::model::User;

use super::slack_assist::Stage;
use super::{App, now_secs};

impl App {
    /// Ask found a scheduling request: look up who is in the conversation.
    pub(super) fn start_calendar(&mut self) {
        let Some(channel) = self.slack_assist.channel.clone() else {
            return;
        };
        if !self.slack.request(Command::People(channel)) {
            return self.answer(Err("Not connected to Slack.".to_string()));
        }
        self.slack_assist.stage = Stage::FindingPeople;
        self.render_slack_assist();
    }

    /// The worker's answer to [`Command::People`]: plan the event with them.
    pub(super) fn on_people(&mut self, channel: String, people: Result<Vec<User>, String>) {
        let waiting = matches!(self.slack_assist.stage, Stage::FindingPeople)
            && self.slack_assist.channel.as_deref() == Some(channel.as_str());
        if !waiting {
            return;
        }
        let people = match people {
            Ok(people) => people,
            Err(error) => {
                return self.answer(Err(format!("Could not look up the people here: {error}")));
            }
        };
        let (Some(tools), Some(context)) =
            (self.calendar.tools().cloned(), self.slack.assist_context())
        else {
            return self.answer(Err("No calendar connector found in Claude Code.".into()));
        };
        let me = self.slack.my_id();
        let people: Vec<Person> = people
            .into_iter()
            .map(|user| Person {
                name: user.name().to_string(),
                me: me == Some(user.id.as_str()),
                email: user.email,
                tz: user.tz,
            })
            .collect();
        let ctx = Context {
            title: &context.title,
            me: &context.me,
            lines: &context.lines,
            now: now_secs(),
            style: self.voice_guide_for_drafts(),
            can_schedule: true,
        };
        let prompt = calendar::proposal_prompt(&ctx, &self.slack_assist.question, &people, &tools);
        let started = self
            .claude_env()
            .and_then(|env| Run::start_restricted(env, &prompt, STRONG_MODEL, &tools.read));
        match started {
            Ok(run) => {
                self.slack_assist.run = Some(run);
                self.slack_assist.stage = Stage::Planning;
                self.render_slack_assist();
            }
            Err(error) => self.answer(Err(error)),
        }
    }

    /// The planning run's result: an event to confirm, or why there is none.
    pub(super) fn on_planned(&mut self, result: &str) {
        let Some(tools) = self.calendar.tools() else {
            return self.answer(Err("No calendar connector found in Claude Code.".into()));
        };
        match calendar::parse_proposal(result, tools) {
            Planned::Event { note, proposal } => {
                self.slack_assist.stage = Stage::Proposed {
                    note,
                    proposal: *proposal,
                };
                self.render_slack_assist();
            }
            Planned::NotPlanned(reason) => self.answer(Ok(Reply {
                answer: reason,
                draft: None,
            })),
        }
    }

    /// The user confirmed: create the event, with the create tool only.
    pub(super) fn create_slack_event(&mut self) {
        let Stage::Proposed { note, proposal } = std::mem::take(&mut self.slack_assist.stage)
        else {
            return;
        };
        let started = self.claude_env().and_then(|env| {
            let allowed = [proposal.create_tool.clone()];
            Run::start_restricted(
                env,
                &calendar::create_prompt(&proposal),
                STRONG_MODEL,
                &allowed,
            )
        });
        match started {
            Ok(run) => {
                self.slack_assist.run = Some(run);
                self.slack_assist.stage = Stage::Creating { note, proposal };
                self.render_slack_assist();
            }
            Err(error) => self.answer(Err(error)),
        }
    }

    /// The event is created (or not): say so, and offer the announcement.
    pub(super) fn on_created(&mut self, result: &str, proposal: Proposal) {
        match calendar::parse_created(result) {
            Ok(link) => {
                let summary = format!("Created: {} · {}", proposal.summary, proposal.when());
                let answer = if link.is_empty() {
                    summary
                } else {
                    format!("{summary}\n{link}")
                };
                // Through the reply parser, so the draft follows the same rules (no emoji).
                let draft = proposal
                    .announcement
                    .map(|text| assist::parse(&text).answer)
                    .filter(|text| !text.is_empty());
                self.answer(Ok(Reply { answer, draft }));
            }
            Err(reason) => self.answer(Err(format!("The event was not created: {reason}"))),
        }
    }
}
