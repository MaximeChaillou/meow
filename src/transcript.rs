//! The conversation shown in the chat.
//!
//! Each request is a turn: the working steps (tool calls, sub-agents and
//! Claude's in-between messages) fold under one line, and only the final
//! answer is shown in full.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Tool(String),
    /// Text Claude wrote before its final answer.
    Note(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Turn {
    steps: Vec<Step>,
    /// Latest text: the answer if nothing follows it, otherwise a note.
    latest_text: Option<String>,
    answer: Option<String>,
    running: bool,
    expanded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Entry {
    User(String),
    Info(String),
    Error(String),
    Turn(Turn),
}

/// What the chat draws, top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block<'a> {
    User(&'a str),
    Info(&'a str),
    Error(&'a str),
    /// The fold line of turn `turn`; clicking it toggles the steps.
    Steps {
        turn: usize,
        label: String,
        expanded: bool,
    },
    Tool(&'a str),
    Note(&'a str),
    Answer(&'a str),
}

#[derive(Clone, Debug, Default)]
pub struct Transcript {
    entries: Vec<Entry>,
}

impl Transcript {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn info(&mut self, text: &str) {
        self.entries.push(Entry::Info(text.to_string()));
    }

    pub fn error(&mut self, text: &str) {
        self.entries.push(Entry::Error(text.to_string()));
    }

    /// Shows the user's request and opens a running turn for the reply.
    pub fn start_turn(&mut self, label: &str) {
        self.entries.push(Entry::User(label.to_string()));
        self.entries.push(Entry::Turn(Turn {
            running: true,
            ..Turn::default()
        }));
    }

    /// A finished exchange, as when showing a saved result.
    pub fn answered(&mut self, label: &str, answer: &str) {
        self.start_turn(label);
        self.text(answer);
        self.finish_turn(answer);
    }

    pub fn text(&mut self, text: &str) {
        if let Some(turn) = self.running_turn_index() {
            self.text_at(turn, text);
        }
    }

    pub fn tool(&mut self, summary: &str) {
        if let Some(turn) = self.running_turn_index() {
            self.tool_at(turn, summary);
        }
    }

    /// Closes the running turn; its latest text, or `fallback`, becomes the answer.
    pub fn finish_turn(&mut self, fallback: &str) {
        if let Some(turn) = self.running_turn_index() {
            self.finish_turn_at(turn, fallback);
        }
    }

    /// The latest running turn, to keep writing to it while later turns run.
    pub fn running_turn_index(&self) -> Option<usize> {
        self.entries
            .iter()
            .rposition(|entry| matches!(entry, Entry::Turn(turn) if turn.running))
    }

    /// [`Transcript::text`] for turn `turn`, if it is still running.
    pub fn text_at(&mut self, turn: usize, text: &str) {
        if let Some(turn) = self.turn_mut(turn)
            && let Some(previous) = turn.latest_text.replace(text.to_string())
        {
            turn.steps.push(Step::Note(previous));
        }
    }

    /// Replaces the latest text of turn `turn` (a message that grew).
    pub fn replace_text_at(&mut self, turn: usize, text: &str) {
        if let Some(turn) = self.turn_mut(turn) {
            turn.latest_text = Some(text.to_string());
        }
    }

    /// [`Transcript::tool`] for turn `turn`, if it is still running.
    pub fn tool_at(&mut self, turn: usize, summary: &str) {
        if let Some(turn) = self.turn_mut(turn) {
            if let Some(note) = turn.latest_text.take() {
                turn.steps.push(Step::Note(note));
            }
            turn.steps.push(Step::Tool(summary.to_string()));
        }
    }

    /// [`Transcript::finish_turn`] for turn `turn`, if it is still running.
    pub fn finish_turn_at(&mut self, turn: usize, fallback: &str) {
        if let Some(turn) = self.turn_mut(turn) {
            turn.running = false;
            turn.answer = turn
                .latest_text
                .take()
                .or_else(|| (!fallback.is_empty()).then(|| fallback.to_string()));
        }
    }

    pub fn toggle(&mut self, turn: usize) {
        if let Some(Entry::Turn(turn)) = self.entries.get_mut(turn) {
            turn.expanded = !turn.expanded;
        }
    }

    fn turn_mut(&mut self, index: usize) -> Option<&mut Turn> {
        match self.entries.get_mut(index) {
            Some(Entry::Turn(turn)) if turn.running => Some(turn),
            _ => None,
        }
    }

    pub fn blocks(&self) -> Vec<Block<'_>> {
        let mut blocks = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            match entry {
                Entry::User(text) => blocks.push(Block::User(text)),
                Entry::Info(text) => blocks.push(Block::Info(text)),
                Entry::Error(text) => blocks.push(Block::Error(text)),
                Entry::Turn(turn) => push_turn(&mut blocks, index, turn),
            }
        }
        blocks
    }
}

fn push_turn<'a>(blocks: &mut Vec<Block<'a>>, index: usize, turn: &'a Turn) {
    if turn.running || !turn.steps.is_empty() {
        blocks.push(Block::Steps {
            turn: index,
            label: steps_label(turn),
            expanded: turn.expanded,
        });
    }
    if turn.expanded {
        blocks.extend(turn.steps.iter().map(|step| match step {
            Step::Tool(text) => Block::Tool(text),
            Step::Note(text) => Block::Note(text),
        }));
    }
    if let Some(answer) = &turn.answer {
        blocks.push(Block::Answer(answer));
    }
}

fn steps_label(turn: &Turn) -> String {
    let count = match turn.steps.len() {
        1 => "1 step".to_string(),
        n => format!("{n} steps"),
    };
    match (turn.running, turn.steps.len()) {
        (true, 0) => "Working…".to_string(),
        (true, _) => format!("Working… · {count}"),
        (false, _) => count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps_header(blocks: &[Block]) -> (usize, String, bool) {
        blocks
            .iter()
            .find_map(|b| match b {
                Block::Steps {
                    turn,
                    label,
                    expanded,
                } => Some((*turn, label.clone(), *expanded)),
                _ => None,
            })
            .expect("a steps line")
    }

    fn worked_turn() -> Transcript {
        let mut t = Transcript::default();
        t.start_turn("⚡︎ Pending tasks");
        t.text("Let me check your sources.");
        t.tool("Agent: Linear");
        t.tool("✓ Linear");
        t.text("Still checking GitHub.");
        t.text("## Pending\n- ACC-52");
        t.finish_turn("## Pending\n- ACC-52");
        t
    }

    #[test]
    fn steps_are_folded_and_only_the_answer_is_shown() {
        let t = worked_turn();
        let blocks = t.blocks();
        assert_eq!(blocks.len(), 3, "{blocks:?}");
        assert_eq!(blocks[0], Block::User("⚡︎ Pending tasks"));
        assert_eq!(steps_header(&blocks).1, "4 steps");
        assert_eq!(blocks[2], Block::Answer("## Pending\n- ACC-52"));
    }

    #[test]
    fn expanding_shows_steps_in_order_before_the_answer() {
        let mut t = worked_turn();
        let turn = steps_header(&t.blocks()).0;
        t.toggle(turn);
        assert_eq!(
            t.blocks()[2..],
            [
                Block::Note("Let me check your sources."),
                Block::Tool("Agent: Linear"),
                Block::Tool("✓ Linear"),
                Block::Note("Still checking GitHub."),
                Block::Answer("## Pending\n- ACC-52"),
            ]
        );
        assert!(steps_header(&t.blocks()).2);
        t.toggle(turn);
        assert_eq!(t.blocks().len(), 3);
    }

    #[test]
    fn running_turn_shows_progress_but_no_answer_yet() {
        let mut t = Transcript::default();
        t.start_turn("hi");
        assert_eq!(steps_header(&t.blocks()).1, "Working…");
        t.tool("Bash: date");
        t.text("It is Monday.");
        let blocks = t.blocks();
        assert_eq!(steps_header(&blocks).1, "Working… · 1 step");
        assert!(!blocks.iter().any(|b| matches!(b, Block::Answer(_))));
    }

    #[test]
    fn simple_answer_has_no_steps_line() {
        let mut t = Transcript::default();
        t.start_turn("hi");
        t.text("Hello!");
        t.finish_turn("Hello!");
        assert_eq!(t.blocks(), [Block::User("hi"), Block::Answer("Hello!")]);
    }

    #[test]
    fn result_is_used_when_no_text_was_streamed() {
        let mut t = Transcript::default();
        t.start_turn("hi");
        t.finish_turn("From the result line");
        assert_eq!(t.blocks()[1], Block::Answer("From the result line"));
    }

    #[test]
    fn failed_turn_keeps_steps_and_shows_the_error_after() {
        let mut t = Transcript::default();
        t.start_turn("hi");
        t.tool("Bash: ls");
        t.finish_turn("");
        t.error("claude stopped");
        let blocks = t.blocks();
        assert_eq!(steps_header(&blocks).1, "1 step");
        assert_eq!(blocks.last(), Some(&Block::Error("claude stopped")));
    }

    #[test]
    fn events_after_a_turn_finished_are_ignored() {
        let mut t = worked_turn();
        let before = t.blocks().len();
        t.text("late");
        t.tool("late tool");
        assert_eq!(t.blocks().len(), before);
    }

    #[test]
    fn toggling_a_non_turn_entry_does_nothing() {
        let mut t = worked_turn();
        t.toggle(0);
        t.toggle(99);
        assert!(!steps_header(&t.blocks()).2);
    }

    #[test]
    fn answered_shows_a_finished_exchange() {
        let mut t = Transcript::default();
        t.answered("⚡︎ Plan", "## Today");
        assert_eq!(
            t.blocks(),
            [Block::User("⚡︎ Plan"), Block::Answer("## Today")]
        );
    }

    #[test]
    fn clear_empties_the_transcript() {
        let mut t = worked_turn();
        t.info("x");
        t.clear();
        assert!(t.blocks().is_empty());
    }

    #[test]
    fn an_earlier_running_turn_is_written_by_index() {
        let mut t = Transcript::default();
        t.start_turn("/agent fix it");
        let agent = t.running_turn_index().unwrap();
        t.start_turn("hi");
        t.text("Hello!");
        t.finish_turn("");
        t.tool_at(agent, "Created workspace");
        t.text_at(agent, "Looking.");
        t.replace_text_at(agent, "Looking at the export.");
        t.finish_turn_at(agent, "");
        assert_eq!(
            t.blocks(),
            [
                Block::User("/agent fix it"),
                Block::Steps {
                    turn: agent,
                    label: "1 step".into(),
                    expanded: false
                },
                Block::Answer("Looking at the export."),
                Block::User("hi"),
                Block::Answer("Hello!"),
            ]
        );
    }

    #[test]
    fn finished_or_missing_turns_ignore_writes_by_index() {
        let mut t = worked_turn();
        let before = t.blocks().len();
        t.text_at(1, "late");
        t.tool_at(1, "late");
        t.text_at(99, "nowhere");
        t.finish_turn_at(0, "not a turn");
        assert_eq!(t.blocks().len(), before);
        assert_eq!(t.running_turn_index(), None);
    }
}
