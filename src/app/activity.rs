//! The Activity page on the app side: background events become items in the
//! log and macOS banners. Opening the page marks them read; each can be
//! dismissed, or all cleared.

use meow::activity::{self, ActivityLog, Kind, Link, Note};
use meow::conversations::ChatId;

use crate::ui::notify;
use crate::ui::sidebar::Page;

use super::plan::open_url;
use super::{App, now_secs};

pub(super) struct ActivityState {
    pub(super) log: ActivityLog,
    /// Where the log is saved; `None` when the file could not be read.
    path: Option<std::path::PathBuf>,
}

impl ActivityState {
    /// The saved log, plus the reason it could not be read.
    pub(super) fn load() -> (Self, Option<String>) {
        let Some(path) = activity::default_path() else {
            return (Self::empty(None), None);
        };
        match ActivityLog::load(&path) {
            Ok(mut log) => {
                log.prune(now_secs());
                (
                    Self {
                        log,
                        path: Some(path),
                    },
                    None,
                )
            }
            Err(error) => (
                Self::empty(None),
                Some(format!("{error}. Activity won't be saved.")),
            ),
        }
    }

    fn empty(path: Option<std::path::PathBuf>) -> Self {
        Self {
            log: ActivityLog::default(),
            path,
        }
    }
}

impl App {
    /// Records something that happened, with a banner unless the Activity
    /// page is on screen (then it is read at once).
    pub(super) fn notify(&mut self, note: Note) {
        let now = now_secs();
        self.activity.log.prune(now);
        let Some(item) = self.activity.log.push(note, now) else {
            return;
        };
        let body = item
            .detail
            .clone()
            .unwrap_or_else(|| kind_name(item.kind).to_string());
        let title = item.title.clone();
        if self.activity_visible() {
            self.activity.log.mark_all_read();
        } else {
            notify::post(&title, &body);
        }
        self.activity_changed();
    }

    fn activity_visible(&self) -> bool {
        self.chat.is_visible() && self.chat.page() == Page::Activity
    }

    /// Opens the Activity page; what was new stays outlined this time, and
    /// counts as read from now on.
    pub(super) fn show_activity(&mut self) {
        if !self.chat.is_visible() {
            self.open_window();
        }
        self.chat.render_activity(&self.activity.log, now_secs());
        self.chat.show_page(Page::Activity);
        if self.activity.log.mark_all_read() {
            self.save_activity();
            self.chat.set_unread_activity(0);
        }
    }

    pub(super) fn open_activity(&mut self, row: usize) {
        let Some(link) = self
            .chat
            .activity()
            .id_at(row)
            .and_then(|id| self.activity.log.get(id))
            .and_then(|item| item.link.clone())
        else {
            return;
        };
        match link {
            Link::Slack { channel } => {
                self.show_slack();
                self.open_slack_channel(channel);
            }
            Link::Plan { action } => {
                if let Some(index) = self.actions.iter().position(|a| a.title == action) {
                    self.show_conversation(ChatId::Action(index));
                }
            }
            Link::Tasks => self.chat.show_page(Page::Tasks),
            Link::Url { url } => open_url(&url),
        }
    }

    pub(super) fn dismiss_activity(&mut self, row: usize) {
        if let Some(id) = self.chat.activity().id_at(row)
            && self.activity.log.dismiss(id)
        {
            self.activity_changed();
        }
    }

    pub(super) fn clear_activity(&mut self) {
        self.activity.log.clear();
        self.activity_changed();
    }

    fn activity_changed(&mut self) {
        self.save_activity();
        self.chat.render_activity(&self.activity.log, now_secs());
    }

    fn save_activity(&mut self) {
        let Some(path) = &self.activity.path else {
            return;
        };
        if let Err(error) = self.activity.log.save(path) {
            eprintln!("meow: {error}");
        }
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Slack => "Slack",
        Kind::Calendar => "Calendar",
        Kind::Tasks => "Tasks",
        Kind::Scheduled => "Scheduled",
    }
}
