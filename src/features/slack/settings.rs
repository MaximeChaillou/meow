//! The Slack feature's sections of the Settings page: drafting in the
//! user's style, and Autopilot (on or off, when it starts, how it works).

use meow::autopilot;

use super::{Msg, Slack, SlackSettings};
use crate::app::feature::Host;
use crate::app::mailbox::Sender;
use crate::ui::settings_page::{SettingEntry, SettingsSection};

const WHEN_HINT: &str = "Which messages to prepare work for, e.g. \"Only private messages \
                         about a coding task\". Empty: any message asking you for something.";
const HOW_HINT: &str = "How to do the work, e.g. \"For code, open a new Superset workspace \
                        for the repository, and a draft PR at the end\".";

/// What a control of the Slack feature's settings asks for.
#[derive(Clone, Debug)]
pub(super) enum Setting {
    WriteAsMe,
    LearnMyStyle,
    Autopilot,
    AutopilotWhen(String),
    AutopilotHow(String),
}

impl Slack {
    pub(super) fn settings_sections(&self) -> Vec<SettingsSection> {
        let inbox = self.inbox.sender();
        let mut slack = vec![toggle(
            &inbox,
            "Write drafts like me",
            self.settings.write_as_me && self.voice.guide.is_some(),
            Setting::WriteAsMe,
        )];
        if !self.voice.busy() {
            let title = match self.voice.guide {
                Some(_) => "Relearn my style",
                None => "Learn my style",
            };
            slack.push(button(&inbox, title, Setting::LearnMyStyle));
        }
        if !self.voice.status.is_empty() {
            slack.push(SettingEntry::Note(self.voice.status.clone()));
        }
        let autopilot = vec![
            toggle(
                &inbox,
                "Prepare work from Slack messages",
                self.settings.autopilot,
                Setting::Autopilot,
            ),
            SettingEntry::Note(
                "Nothing that can't be undone (a push, a message sent, an issue changed) \
                 happens until you approve it on the Autopilot page."
                    .to_string(),
            ),
            text(
                &inbox,
                "When to start",
                WHEN_HINT,
                &self.settings.autopilot_when,
                Setting::AutopilotWhen,
            ),
            text(
                &inbox,
                "How to work",
                HOW_HINT,
                &self.settings.autopilot_how,
                Setting::AutopilotHow,
            ),
        ];
        vec![
            SettingsSection {
                title: "Slack",
                entries: slack,
            },
            SettingsSection {
                title: "Autopilot",
                entries: autopilot,
            },
        ]
    }

    pub(super) fn change_setting(&mut self, setting: Setting, host: &mut dyn Host) {
        match setting {
            Setting::WriteAsMe => self.toggle_write_as_me(host),
            Setting::LearnMyStyle => self.learn_my_style(host),
            Setting::Autopilot => self.toggle_autopilot(host),
            Setting::AutopilotWhen(when) => {
                let settings = SlackSettings {
                    autopilot_when: when.trim().to_string(),
                    ..self.settings.clone()
                };
                self.save_settings(settings, host);
            }
            Setting::AutopilotHow(how) => {
                let settings = SlackSettings {
                    autopilot_how: how.trim().to_string(),
                    ..self.settings.clone()
                };
                self.save_settings(settings, host);
            }
        }
    }

    /// Moves the Autopilot feedback kept before "How to work" existed into it, once.
    pub(super) fn fold_autopilot_feedback(&mut self, host: &mut dyn Host) {
        if self.settings.autopilot_feedback.is_empty() {
            return;
        }
        let settings = SlackSettings {
            autopilot_how: autopilot::fold_feedback(
                &self.settings.autopilot_how,
                &self.settings.autopilot_feedback,
            ),
            autopilot_feedback: Vec::new(),
            ..self.settings.clone()
        };
        self.save_settings(settings, host);
    }

    /// Keeps and saves `settings`, and has the Settings page show them.
    pub(super) fn save_settings(&mut self, settings: SlackSettings, host: &mut dyn Host) {
        if settings == self.settings {
            return;
        }
        self.settings = settings;
        if let Err(error) = host.settings().set(&self.settings) {
            eprintln!("meow: {error}");
        }
        host.settings_changed();
    }
}

fn toggle(inbox: &Sender<Msg>, title: &str, on: bool, setting: Setting) -> SettingEntry {
    let inbox = inbox.clone();
    SettingEntry::Toggle {
        title: title.to_string(),
        on,
        run: Box::new(move || inbox.send(Msg::Setting(setting.clone()))),
    }
}

fn button(inbox: &Sender<Msg>, title: &str, setting: Setting) -> SettingEntry {
    let inbox = inbox.clone();
    SettingEntry::Button {
        title: title.to_string(),
        run: Box::new(move || inbox.send(Msg::Setting(setting.clone()))),
    }
}

fn text(
    inbox: &Sender<Msg>,
    title: &str,
    hint: &str,
    value: &str,
    setting: fn(String) -> Setting,
) -> SettingEntry {
    let inbox = inbox.clone();
    SettingEntry::Text {
        title: title.to_string(),
        hint: hint.to_string(),
        value: value.to_string(),
        save: Box::new(move |text| inbox.send(Msg::Setting(setting(text)))),
    }
}
