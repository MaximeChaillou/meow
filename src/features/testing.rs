//! What feature tests share: a `Host` that records what a feature tells the
//! app (no AppKit, no Claude, settings kept in memory), and helpers to read
//! and click a feature's menu entries.

use meow::activity::Note;
use meow::claude::ClaudeEnv;
use meow::conversations::ChatId;
use meow::settings::Store;
use meow::sprites::Coat;

use crate::app::feature::Host;
use crate::ui::menus::MenuEntry;

#[derive(Default)]
pub(crate) struct FakeHost {
    pub(crate) infos: Vec<(ChatId, String)>,
    pub(crate) errors: Vec<(ChatId, String)>,
    pub(crate) banners: Vec<String>,
    pub(crate) notes: Vec<Note>,
    pub(crate) badges: Vec<(&'static str, usize)>,
    pub(crate) settings: Store,
    /// Each coat the cat was dressed in.
    pub(crate) worn: Vec<Coat>,
    /// Each size the chat bubble was given.
    pub(crate) sizes: Vec<(f64, f64)>,
    pub(crate) menu_changes: usize,
    /// Conversations flagged with `!` over the cat.
    pub(crate) unread: Vec<ChatId>,
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

    fn flag_unread(&mut self, chat: &ChatId) {
        self.unread.push(chat.clone());
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

    fn settings(&mut self) -> &mut Store {
        &mut self.settings
    }

    fn wear(&mut self, coat: Coat) {
        self.worn.push(coat);
    }

    fn resize_chat(&mut self, size: (f64, f64)) {
        self.sizes.push(size);
    }

    fn menu_changed(&mut self) {
        self.menu_changes += 1;
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

/// The entries as lines: `label: …`, `action: …` (`[x]` when checked), `---`,
/// and a submenu's title then its entries, indented.
pub(crate) fn describe(entries: &[MenuEntry]) -> Vec<String> {
    entries
        .iter()
        .flat_map(|entry| match entry {
            MenuEntry::Label(text) => vec![format!("label: {text}")],
            MenuEntry::Separator => vec!["---".to_string()],
            MenuEntry::Action { title, checked, .. } => {
                let mark = if *checked { "[x] " } else { "" };
                vec![format!("{mark}action: {title}")]
            }
            MenuEntry::Submenu { title, entries } => std::iter::once(format!("{title}:"))
                .chain(
                    describe(entries)
                        .into_iter()
                        .map(|line| format!("  {line}")),
                )
                .collect(),
        })
        .collect()
}

/// Chooses the item at `path`: submenu titles, then the item's title.
pub(crate) fn click(entries: &[MenuEntry], path: &[&str]) {
    let Some((first, rest)) = path.split_first() else {
        panic!("empty menu path");
    };
    for entry in entries {
        match entry {
            MenuEntry::Action { title, run, .. } if title == first && rest.is_empty() => {
                return run();
            }
            MenuEntry::Submenu { title, entries } if title == first => {
                return click(entries, rest);
            }
            _ => {}
        }
    }
    panic!("no menu item {path:?} in {:#?}", describe(entries));
}
