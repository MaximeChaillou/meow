//! What a feature is to the app. A feature lives in its own folder under
//! `src/features/`, keeps its own state and messages, and reaches the rest of
//! the app only through `Host`; see ARCHITECTURE.md. A new feature adds files
//! and one line in `features::all`, never a field on `App` or a `UiEvent`.

use meow::activity::Note;
use meow::claude::ClaudeEnv;
use meow::conversations::ChatId;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_foundation::NSRect;

use super::App;
use crate::ui::menus::MenuEntry;
use crate::ui::notify;
use crate::ui::sidebar::{Page, PageSpec};

pub(crate) trait Feature {
    /// Once per tick, after the UI events: handle the feature's own messages
    /// and check on its background work.
    fn poll(&mut self, host: &mut dyn Host);

    /// Why the feature's saved data could not be read, told once at launch.
    fn startup_error(&self) -> Option<String> {
        None
    }

    /// A command typed in conversation `chat`, such as `/update`: whether this
    /// feature takes it. It is acted on at the next poll.
    fn command(&mut self, _text: &str, _chat: &ChatId) -> bool {
        false
    }

    /// Added to the prompt of each message typed in the chat.
    fn chat_instruction(&self, _now: u64) -> Option<String> {
        None
    }

    /// A reply in conversation `chat`, `done` for the final one: the text
    /// without the blocks this feature reads in it. Acted on at the next poll.
    fn read_reply(&mut self, _chat: &ChatId, text: &str, _done: bool) -> String {
        text.to_string()
    }

    /// Items for the cat's menu, above "Quit Meow".
    fn menu(&self) -> Vec<MenuEntry> {
        Vec::new()
    }

    /// The feature's page in the sidebar, if it has one.
    fn page(&self) -> Option<PageSpec> {
        None
    }

    /// Builds the page's view, in `frame` of the chat bubble. Called once,
    /// for a feature with a `page`.
    fn build_page(&mut self, _mtm: MainThreadMarker, _frame: NSRect) -> Option<Retained<NSView>> {
        None
    }

    /// The page is about to be shown.
    fn page_shown(&mut self) {}

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
    /// Something that happened, for the Activity page (with a banner).
    fn record(&mut self, note: Note);
    /// The Claude Code setup, once found; `None` before or without one.
    fn claude(&self) -> Option<&ClaudeEnv>;
    /// The count shown at the end of `page`'s row in the sidebar.
    fn set_badge(&mut self, page: &'static str, count: usize);
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

    fn record(&mut self, note: Note) {
        self.app.notify(note);
    }

    fn claude(&self) -> Option<&ClaudeEnv> {
        self.app.claude.as_ref()?.as_ref().ok()
    }

    fn set_badge(&mut self, page: &'static str, count: usize) {
        self.app.chat.set_badge(page, count);
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

    /// What the features add to the prompt of a message typed in the chat.
    pub(super) fn feature_instructions(&self, now: u64) -> Vec<String> {
        self.features
            .iter()
            .filter_map(|f| f.chat_instruction(now))
            .collect()
    }

    /// Reply `text` without the blocks the features read in it.
    pub(super) fn features_read_reply(&mut self, chat: &ChatId, text: &str, done: bool) -> String {
        self.features
            .iter_mut()
            .fold(text.to_string(), |text, f| f.read_reply(chat, &text, done))
    }

    /// Opens the chat bubble on `page`, letting its feature refresh it first.
    pub(super) fn open_page(&mut self, page: Page) {
        if !self.chat.is_visible() {
            self.open_window();
        }
        if let Page::Feature(id) = page {
            self.features
                .iter_mut()
                .filter(|f| f.page().is_some_and(|spec| spec.id == id))
                .for_each(|f| f.page_shown());
        }
        self.chat.show_page(page);
    }

    /// Nothing outside the features that a restart would lose.
    fn is_idle(&self) -> bool {
        self.runs.is_empty()
            && !self.chat.is_visible()
            && !self.sync.is_running()
            && !self.agenda.is_running()
            && !self.rsvp.is_running()
            && self.slack_assist.run.is_none()
            && !self.agents.is_running()
    }
}

/// Builds each feature's page into the chat bubble, whose sidebar already
/// lists them (see `ChatWindow::new`).
pub(super) fn attach_pages(
    mtm: MainThreadMarker,
    features: &mut [Box<dyn Feature>],
    chat: &crate::ui::chat_window::ChatWindow,
) {
    for feature in features {
        let Some(spec) = feature.page() else {
            continue;
        };
        if let Some(view) = feature.build_page(mtm, chat.page_frame()) {
            chat.add_page(spec.id, &view);
        }
    }
}
