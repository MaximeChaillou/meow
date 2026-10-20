//! Debug snapshots of the bubble, for checking the look without screen access.
//!
//! With `MEOW_SNAPSHOT_DIR` set, Meow opens the bubble a few seconds after
//! launch, shows each page in turn, saves it as a PNG in that folder, then
//! quits. The Slack pages are filled with sample messages, so no sign-in is
//! needed (launch with dummy `MEOW_SLACK_*` values to keep off the Keychain).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use meow::transcript::Transcript;

use crate::ui::events::UiEvent;
use crate::ui::slack_assist::AssistView;
use crate::ui::slack_page::{ConversationRow, TranscriptItem};

use super::App;

/// The pages, in order; each is saved once it has settled.
const STEPS: [&str; 8] = [
    "chat",
    "chat-two-lines",
    "chat-long",
    "tasks",
    "plan",
    "slack-list",
    "slack-conversation",
    "slack-assist",
];
/// Time for the app to settle before the first step (fonts, tasks).
const START_DELAY: Duration = Duration::from_secs(4);
const STEP_DELAY: Duration = Duration::from_millis(2500);

pub(super) struct Snapshot {
    dir: PathBuf,
    started: Instant,
    /// Index of the next step to start; a step's file is saved before the next one.
    next: usize,
}

impl Snapshot {
    pub(super) fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os("MEOW_SNAPSHOT_DIR")?);
        if let Err(error) = std::fs::create_dir_all(&dir) {
            eprintln!("meow: snapshot folder {}: {error}", dir.display());
            return None;
        }
        Some(Self {
            dir,
            started: Instant::now(),
            next: 0,
        })
    }
}

impl App {
    pub(super) fn poll_snapshot(&mut self) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let next = snapshot.next;
        if snapshot.started.elapsed() < START_DELAY + STEP_DELAY * next as u32 {
            return;
        }
        if next == 0 && !self.chat.is_visible() {
            self.handle(UiEvent::ToggleChat);
        }
        if next > 0 {
            self.save_snapshot(STEPS[next - 1]);
        }
        if next == STEPS.len() {
            eprintln!("meow: snapshots saved");
            std::process::exit(0);
        }
        self.show_snapshot_step(STEPS[next]);
        if let Some(snapshot) = self.snapshot.as_mut() {
            snapshot.next += 1;
        }
    }

    fn show_snapshot_step(&mut self, step: &str) {
        match step {
            "chat" => {
                self.handle(UiEvent::ShowChat);
                self.chat.chat().render(&sample_chat(), true);
            }
            "chat-two-lines" => self.chat.chat().set_draft(
                "Rédige une relance pour la relecture Android, en restant court et sympa avec Hugo.",
            ),
            "chat-long" => self.chat.chat().set_draft(
                "Rédige une relance pour la relecture Android.\nReste court et sympa.\n\
                 Mentionne que la mise en prod attend cette PR.\nEt propose un créneau demain.",
            ),
            "tasks" => {
                self.chat.chat().set_draft("");
                self.handle(UiEvent::ShowTasks);
            }
            "plan" => self.handle(UiEvent::QuickAction(0)),
            "slack-list" => {
                self.handle(UiEvent::ShowSlack);
                self.chat.slack().render_list(&sample_rows());
            }
            "slack-conversation" => {
                self.chat
                    .slack()
                    .render_conversation(
                        "Hugo",
                        &sample_transcript(),
                        &Default::default(),
                        &Default::default(),
                    );
            }
            _ => {
                let attendees = ["toi".to_string(), "Hugo".to_string()];
                self.chat.slack().show_assist(&AssistView::Event {
                    question: "Cale un point avec Hugo demain",
                    note: "Vous êtes libres tous les deux demain à 10:00. Je propose :",
                    summary: "Point PR Android",
                    when: "mar. 20 oct · 10:00 – 10:30",
                    attendees: &attendees,
                    creating: false,
                });
            }
        }
    }

    fn save_snapshot(&self, name: &str) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let path = snapshot.dir.join(format!("{name}.png"));
        match self.chat.snapshot(&path) {
            Ok(()) => eprintln!("meow: saved {}", path.display()),
            Err(error) => eprintln!("meow: snapshot {name}: {error}"),
        }
    }
}

fn sample_rows() -> Vec<ConversationRow> {
    let row = |title: &str, unread: bool, mentions: u32, time: &str| ConversationRow {
        id: title.to_string(),
        title: title.to_string(),
        unread,
        mentions,
        time: time.to_string(),
        direct: !title.starts_with('#'),
    };
    vec![
        row("Hugo", true, 1, "09:42"),
        row("Inès", true, 0, "09:31"),
        row("Hugo, Inès", false, 0, "Mon"),
        row("Léa", false, 0, "Fri"),
        row("#general", true, 0, "10:02"),
    ]
}

fn sample_transcript() -> Vec<TranscriptItem> {
    let run = |author: &str, time: &str, mine: bool, bodies: &[&str]| TranscriptItem::Run {
        author: author.to_string(),
        time: time.to_string(),
        mine,
        bodies: bodies.iter().map(|b| b.to_string()).collect(),
    };
    vec![
        TranscriptItem::Day("Today".to_string()),
        run(
            "Hugo",
            "09:41",
            false,
            &["Tu aurais 30 min demain pour la PR Android ? Je préfère qu'on la passe ensemble."],
        ),
        run("Toi", "09:42", true, &["Oui, le matin m'arrange."]),
        run("Hugo", "09:42", false, &["Top, merci !"]),
    ]
}

fn sample_chat() -> Transcript {
    let mut chat = Transcript::default();
    chat.info("Click the cat or press ⌥⇧Space to talk. Right-click for options.");
    chat.start_turn("Tu peux résumer le fil #release de ce matin ?");
    chat.tool("slack_read_channel #release");
    chat.tool("slack_read_thread");
    chat.text(
        "Voici l'essentiel :\n\n- build iOS **validée**\n- la PR Android attend une relecture\n\
         - mise en prod après le standup",
    );
    chat.finish_turn("");
    chat.start_turn("Rédige une relance pour la relecture Android.");
    chat.text("Proposition :\n\n> Petite relance sur la PR Android : elle bloque la mise en prod.");
    chat.finish_turn("");
    chat
}
