//! The cat's coat, picked from its menu and remembered across launches.

use std::path::PathBuf;

use meow::conversations::ChatId;
use meow::settings::{self, Settings};
use meow::sprites::Coat;

use crate::ui::menus;

use super::App;

/// Saved settings and where to save them; `None` when the file could not be
/// read, so it is never overwritten, plus the reason.
pub(super) fn load_settings() -> (Settings, Option<PathBuf>, Option<String>) {
    let Some(path) = settings::default_path() else {
        return (Settings::default(), None, None);
    };
    match Settings::load(&path) {
        Ok(saved) => (saved, Some(path), None),
        Err(error) => (
            Settings::default(),
            None,
            Some(format!("{error}. Theme changes won't be saved.")),
        ),
    }
}

impl App {
    /// Index into `Coat::ALL`, as tagged on the theme menu items.
    pub(super) fn set_coat(&mut self, index: usize) {
        let Some(&coat) = Coat::ALL.get(index) else {
            return;
        };
        self.settings = Settings {
            coat,
            ..self.settings.clone()
        };
        menus::mark_coat(&self.coat_menu, coat);
        // Redraw now, even if the cat holds still.
        self.last_look = None;
        let Some(path) = &self.settings_path else {
            return;
        };
        if let Err(error) = self.settings.save(path) {
            eprintln!("meow: {error}");
            self.error_in(&ChatId::Main, &error);
        }
    }
}
