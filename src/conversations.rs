//! The main chat and one conversation per task, each with its own transcript
//! and Claude session.

use std::collections::HashMap;

use crate::transcript::Transcript;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ChatId {
    Main,
    /// The conversation about the task with this id.
    Task(String),
    /// The latest run of the quick action at this index, and follow-ups to it.
    Action(usize),
}

#[derive(Debug, Default)]
pub struct Conversation {
    pub transcript: Transcript,
    /// Claude session to resume, set once a reply completed.
    pub session: Option<String>,
    /// Session reported by the running request, kept only if it completes.
    pub pending_session: Option<String>,
}

impl Conversation {
    pub fn resuming(session: Option<String>) -> Self {
        Self {
            session,
            ..Self::default()
        }
    }

    /// Commits the running request's session; returns it when it changed.
    pub fn complete(&mut self) -> Option<&str> {
        let session = self.pending_session.take()?;
        if self.session.as_deref() == Some(session.as_str()) {
            return None;
        }
        self.session = Some(session);
        self.session.as_deref()
    }

    /// Forgets the transcript and the session: the next message starts afresh.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug)]
pub struct Conversations {
    chats: HashMap<ChatId, Conversation>,
    active: ChatId,
}

impl Default for Conversations {
    fn default() -> Self {
        Self {
            chats: HashMap::from([(ChatId::Main, Conversation::default())]),
            active: ChatId::Main,
        }
    }
}

impl Conversations {
    pub fn active(&self) -> &ChatId {
        &self.active
    }

    /// Shows `id`, creating it with `make` the first time.
    pub fn open(&mut self, id: ChatId, make: impl FnOnce() -> Conversation) {
        self.chats.entry(id.clone()).or_insert_with(make);
        self.active = id;
    }

    pub fn current(&self) -> &Conversation {
        self.chats
            .get(&self.active)
            .expect("the active conversation exists")
    }

    /// The conversation `id`, created empty if it was never opened.
    pub fn get_mut(&mut self, id: &ChatId) -> &mut Conversation {
        self.chats.entry(id.clone()).or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str) -> ChatId {
        ChatId::Task(id.into())
    }

    #[test]
    fn starts_on_the_main_chat() {
        let chats = Conversations::default();
        assert_eq!(chats.active(), &ChatId::Main);
        assert!(chats.current().transcript.blocks().is_empty());
    }

    #[test]
    fn opening_a_task_chat_keeps_each_transcript_separate() {
        let mut chats = Conversations::default();
        chats.get_mut(&ChatId::Main).transcript.info("main");
        chats.open(task("t1"), || Conversation::resuming(Some("s1".into())));
        assert_eq!(chats.current().session.as_deref(), Some("s1"));
        assert!(chats.current().transcript.blocks().is_empty());

        chats.get_mut(&task("t1")).transcript.info("about t1");
        chats.open(ChatId::Main, Conversation::default);
        assert_eq!(chats.current().transcript.blocks().len(), 1);
    }

    #[test]
    fn reopening_keeps_the_existing_conversation() {
        let mut chats = Conversations::default();
        chats.open(task("t1"), Conversation::default);
        chats.get_mut(&task("t1")).transcript.info("hello");
        chats.open(task("t1"), || panic!("must not be recreated"));
        assert_eq!(chats.current().transcript.blocks().len(), 1);
    }

    #[test]
    fn session_is_committed_only_on_completion() {
        let mut chat = Conversation {
            pending_session: Some("s1".into()),
            ..Conversation::default()
        };
        assert_eq!(chat.session, None);
        assert_eq!(chat.complete(), Some("s1"));
        assert_eq!(chat.session.as_deref(), Some("s1"));
        chat.pending_session = Some("s1".into());
        assert_eq!(chat.complete(), None, "same session is not reported again");
    }

    #[test]
    fn reset_forgets_transcript_and_session() {
        let mut chat = Conversation::resuming(Some("s1".into()));
        chat.transcript.info("x");
        chat.reset();
        assert!(chat.session.is_none());
        assert!(chat.transcript.blocks().is_empty());
    }
}
