//! A `Host` for feature tests: records what a feature tells the app, with no
//! AppKit and no Claude.

use meow::activity::Note;
use meow::claude::ClaudeEnv;
use meow::conversations::ChatId;

use crate::app::feature::Host;

#[derive(Default)]
pub(crate) struct FakeHost {
    pub(crate) infos: Vec<(ChatId, String)>,
    pub(crate) errors: Vec<(ChatId, String)>,
    pub(crate) banners: Vec<String>,
    pub(crate) notes: Vec<Note>,
    pub(crate) badges: Vec<(&'static str, usize)>,
    pub(crate) idle: bool,
    pub(crate) quit: bool,
}

impl Host for FakeHost {
    fn info_in(&mut self, chat: &ChatId, text: &str) {
        self.infos.push((chat.clone(), text.to_string()));
    }

    fn error_in(&mut self, chat: &ChatId, text: &str) {
        self.errors.push((chat.clone(), text.to_string()));
    }

    fn notify(&mut self, text: &str) {
        self.banners.push(text.to_string());
    }

    fn record(&mut self, note: Note) {
        self.notes.push(note);
    }

    fn claude(&self) -> Option<&ClaudeEnv> {
        None
    }

    fn set_badge(&mut self, page: &'static str, count: usize) {
        self.badges.push((page, count));
    }

    fn is_idle(&self) -> bool {
        self.idle
    }

    fn quit(&mut self) {
        self.quit = true;
    }
}
