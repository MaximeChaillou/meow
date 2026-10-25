//! What a feature is to the app. A feature lives in its own folder under
//! `src/features/`, keeps its own state and messages, and reaches the rest of
//! the app only through `Host`; see ARCHITECTURE.md. A new feature adds files
//! and one line in `features::all`, never a field on `App` or a `UiEvent`.

use meow::conversations::ChatId;

use super::App;
use crate::ui::menus::MenuEntry;
use crate::ui::notify;

pub(crate) trait Feature {
    /// Once per tick, after the UI events: handle the feature's own messages
    /// and check on its background work.
    fn poll(&mut self, host: &mut dyn Host);

    /// A command typed in conversation `chat`, such as `/update`: whether this
    /// feature takes it. It is acted on at the next poll.
    fn command(&mut self, _text: &str, _chat: &ChatId) -> bool {
        false
    }

    /// Items for the cat's menu, above "Quit Meow".
    fn menu(&self) -> Vec<MenuEntry> {
        Vec::new()
    }

    /// Work under way that a restart would lose.
    fn is_busy(&self) -> bool {
        false
    }
}

/// What the app offers its features.
pub(crate) trait Host {
    /// A note in conversation `chat`.
    fn info_in(&mut self, chat: &ChatId, text: &str);
    fn error_in(&mut self, chat: &ChatId, text: &str);
    /// A macOS banner.
    fn notify(&mut self, text: &str);
    /// Nothing the user would lose to a restart: no run, no open chat, no
    /// background work, in the app or in any other feature.
    fn is_idle(&self) -> bool;
    fn quit(&mut self);
}

/// The app, as seen by the feature being polled.
struct AppHost<'a> {
    app: &'a mut App,
    /// Whether another feature has work under way.
    others_busy: bool,
}

impl Host for AppHost<'_> {
    fn info_in(&mut self, chat: &ChatId, text: &str) {
        self.app.info_in(chat, text);
    }

    fn error_in(&mut self, chat: &ChatId, text: &str) {
        self.app.error_in(chat, text);
    }

    fn notify(&mut self, text: &str) {
        notify::post("Meow", text);
    }

    fn is_idle(&self) -> bool {
        !self.others_busy && self.app.is_idle()
    }

    fn quit(&mut self) {
        self.app.quit();
    }
}

impl App {
    pub(super) fn poll_features(&mut self) {
        // Out of `self` while they run, so each can be handed the app.
        let mut features = std::mem::take(&mut self.features);
        for index in 0..features.len() {
            let (before, rest) = features.split_at_mut(index);
            let Some((feature, after)) = rest.split_first_mut() else {
                continue;
            };
            let others_busy = before.iter().chain(after.iter()).any(|f| f.is_busy());
            feature.poll(&mut AppHost {
                app: self,
                others_busy,
            });
        }
        self.features = features;
    }

    /// Whether a feature takes the command `text` typed in conversation `chat`.
    pub(super) fn feature_command(&mut self, text: &str, chat: &ChatId) -> bool {
        self.features.iter_mut().any(|f| f.command(text, chat))
    }

    /// Nothing outside the features that a restart would lose.
    fn is_idle(&self) -> bool {
        self.runs.is_empty()
            && !self.chat.is_visible()
            && !self.sync.is_running()
            && !self.agenda.is_running()
            && !self.rsvp.is_running()
            && self.routines.running().is_none()
            && self.slack_assist.run.is_none()
            && !self.agents.is_running()
    }
}
