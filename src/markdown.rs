//! Turns Claude's Markdown into styled text spans for the chat transcript.
//!
//! Block structure (lists, quotes, tables, rules) becomes plain characters,
//! inline formatting becomes a `Style` the UI maps to fonts and colors.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

const BULLET: &str = "• ";
const LIST_INDENT: &str = "   ";
const QUOTE_BAR: &str = "▎ ";
const CELL_SEPARATOR: &str = "  │  ";
const RULE: &str = "────────────";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    /// 1 to 6 inside a heading, 0 otherwise.
    pub heading: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
    pub link: Option<String>,
}

pub fn to_spans(markdown: &str) -> Vec<Span> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut renderer = Renderer::default();
    for event in Parser::new_ext(markdown, options) {
        renderer.event(event);
    }
    renderer.spans
}

#[derive(Default)]
struct Renderer {
    spans: Vec<Span>,
    style: Style,
    link: Option<String>,
    /// Next number of each open list, `None` for bullet lists.
    lists: Vec<Option<u64>>,
    quote_depth: usize,
    /// Newlines owed before the next text, so blocks never leave trailing ones.
    pending_breaks: usize,
    cell_index: usize,
}

impl Renderer {
    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => self.push(&text),
            Event::Code(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                let style = Style {
                    code: true,
                    ..self.style
                };
                self.push_styled(&text, style, self.link.clone());
            }
            Event::FootnoteReference(name) => self.push(&format!("[{name}]")),
            Event::SoftBreak | Event::HardBreak => self.line_break(),
            Event::Rule => {
                self.push_plain(RULE);
                self.breaks(2);
            }
            Event::TaskListMarker(done) => self.push_plain(if done { "☑ " } else { "☐ " }),
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph if self.quote_depth > 0 => self.push_plain(QUOTE_BAR),
            Tag::Heading { level, .. } => {
                self.style.heading = heading_number(level);
                self.style.bold = true;
            }
            Tag::BlockQuote(_) => self.quote_depth += 1,
            Tag::CodeBlock(_) => self.style.code = true,
            Tag::List(start) => {
                if !self.lists.is_empty() {
                    self.breaks(1);
                }
                self.lists.push(start);
            }
            Tag::Item => self.start_item(),
            Tag::TableHead => {
                self.cell_index = 0;
                self.style.bold = true;
            }
            Tag::TableRow => self.cell_index = 0,
            Tag::TableCell => {
                if self.cell_index > 0 {
                    self.push_plain(CELL_SEPARATOR);
                }
                self.cell_index += 1;
            }
            Tag::Emphasis => self.style.italic = true,
            Tag::Strong => self.style.bold = true,
            Tag::Strikethrough => self.style.strike = true,
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.link = Some(dest_url.to_string());
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.breaks(if self.lists.is_empty() { 2 } else { 1 }),
            TagEnd::Heading(_) => {
                self.style.heading = 0;
                self.style.bold = false;
                self.breaks(1);
            }
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.breaks(2);
            }
            TagEnd::CodeBlock => {
                self.style.code = false;
                self.trim_trailing_newlines();
                self.breaks(2);
            }
            TagEnd::List(_) => {
                self.lists.pop();
                self.breaks(if self.lists.is_empty() { 2 } else { 1 });
            }
            TagEnd::Item | TagEnd::TableRow => self.breaks(1),
            TagEnd::TableHead => {
                self.style.bold = false;
                self.breaks(1);
            }
            TagEnd::Table => self.breaks(2),
            TagEnd::Emphasis => self.style.italic = false,
            TagEnd::Strong => self.style.bold = false,
            TagEnd::Strikethrough => self.style.strike = false,
            TagEnd::Link | TagEnd::Image => self.link = None,
            _ => {}
        }
    }

    fn start_item(&mut self) {
        let depth = self.lists.len().saturating_sub(1);
        let marker = match self.lists.last_mut() {
            Some(Some(number)) => {
                let marker = format!("{number}. ");
                *number += 1;
                marker
            }
            _ => BULLET.to_string(),
        };
        self.push_plain(&format!("{}{marker}", LIST_INDENT.repeat(depth)));
    }

    fn line_break(&mut self) {
        self.push_styled("\n", self.style, None);
        if self.quote_depth > 0 {
            self.push_plain(QUOTE_BAR);
        }
    }

    fn breaks(&mut self, count: usize) {
        self.pending_breaks = self.pending_breaks.max(count);
    }

    fn push(&mut self, text: &str) {
        self.push_styled(text, self.style, self.link.clone());
    }

    fn push_plain(&mut self, text: &str) {
        self.push_styled(text, Style::default(), None);
    }

    fn push_styled(&mut self, text: &str, style: Style, link: Option<String>) {
        if self.pending_breaks > 0 && !self.spans.is_empty() {
            let breaks = "\n".repeat(self.pending_breaks);
            self.append(&breaks, Style::default(), None);
        }
        self.pending_breaks = 0;
        self.append(text, style, link);
    }

    fn append(&mut self, text: &str, style: Style, link: Option<String>) {
        if text.is_empty() {
            return;
        }
        match self.spans.last_mut() {
            Some(last) if last.style == style && last.link == link => last.text.push_str(text),
            _ => self.spans.push(Span {
                text: text.to_string(),
                style,
                link,
            }),
        }
    }

    fn trim_trailing_newlines(&mut self) {
        if let Some(last) = self.spans.last_mut() {
            let trimmed = last.text.trim_end_matches('\n').len();
            last.text.truncate(trimmed);
        }
        self.spans.retain(|span| !span.text.is_empty());
    }
}

fn heading_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(markdown: &str) -> String {
        to_spans(markdown).into_iter().map(|s| s.text).collect()
    }

    fn span_with<'a>(spans: &'a [Span], needle: &str) -> &'a Span {
        spans
            .iter()
            .find(|s| s.text.contains(needle))
            .unwrap_or_else(|| panic!("no span with {needle:?} in {spans:?}"))
    }

    #[test]
    fn plain_paragraph_is_one_unstyled_span() {
        assert_eq!(
            to_spans("Hello there"),
            vec![Span {
                text: "Hello there".into(),
                style: Style::default(),
                link: None
            }]
        );
    }

    #[test]
    fn inline_styles_are_applied() {
        let spans = to_spans("a **bold** _it_ `code` ~~gone~~");
        assert!(span_with(&spans, "bold").style.bold);
        assert!(span_with(&spans, "it").style.italic);
        assert!(span_with(&spans, "code").style.code);
        assert!(span_with(&spans, "gone").style.strike);
        assert_eq!(
            text("a **bold** _it_ `code` ~~gone~~"),
            "a bold it code gone"
        );
    }

    #[test]
    fn paragraphs_are_separated_by_a_blank_line_without_trailing_newlines() {
        assert_eq!(text("one\n\ntwo\n\n"), "one\n\ntwo");
    }

    #[test]
    fn soft_breaks_keep_line_structure() {
        assert_eq!(text("line one\nline two"), "line one\nline two");
    }

    #[test]
    fn headings_are_bold_with_their_level() {
        let spans = to_spans("## Linear\nbody");
        let heading = span_with(&spans, "Linear");
        assert_eq!(heading.style.heading, 2);
        assert!(heading.style.bold);
        assert!(!span_with(&spans, "body").style.bold);
        assert_eq!(text("## Linear\nbody"), "Linear\nbody");
    }

    #[test]
    fn bullet_and_numbered_lists_get_markers() {
        assert_eq!(text("- a\n- b"), "• a\n• b");
        assert_eq!(text("3. x\n4. y"), "3. x\n4. y");
    }

    #[test]
    fn nested_lists_are_indented() {
        assert_eq!(text("- a\n  - b\n- c"), "• a\n   • b\n• c");
    }

    #[test]
    fn list_is_separated_from_following_paragraph() {
        assert_eq!(text("- a\n- b\n\nafter"), "• a\n• b\n\nafter");
    }

    #[test]
    fn task_list_shows_checkboxes() {
        assert_eq!(text("- [x] done\n- [ ] todo"), "• ☑ done\n• ☐ todo");
    }

    #[test]
    fn links_keep_their_url() {
        let spans = to_spans("see [ACC-52](https://linear.app/x) now");
        let link = span_with(&spans, "ACC-52");
        assert_eq!(link.link.as_deref(), Some("https://linear.app/x"));
        assert_eq!(span_with(&spans, "now").link, None);
    }

    #[test]
    fn bare_urls_stay_readable() {
        assert_eq!(text("go to <https://a.b>"), "go to https://a.b");
    }

    #[test]
    fn code_block_is_monospaced_without_trailing_newline() {
        let spans = to_spans("```sh\ncargo run\n```\nafter");
        let code = span_with(&spans, "cargo run");
        assert!(code.style.code);
        assert_eq!(code.text, "cargo run");
        assert_eq!(text("```sh\ncargo run\n```\nafter"), "cargo run\n\nafter");
    }

    #[test]
    fn tables_become_separated_rows_with_bold_header() {
        let markdown = "| a | b |\n|---|---|\n| 1 | 2 |";
        assert_eq!(text(markdown), "a  │  b\n1  │  2");
        let spans = to_spans(markdown);
        assert!(span_with(&spans, "a").style.bold);
        assert!(!span_with(&spans, "1").style.bold);
    }

    #[test]
    fn block_quotes_get_a_bar() {
        assert_eq!(text("> quoted\n> more"), "▎ quoted\n▎ more");
    }

    #[test]
    fn rules_become_a_line() {
        assert_eq!(text("a\n\n---\n\nb"), format!("a\n\n{RULE}\n\nb"));
    }

    #[test]
    fn adjacent_spans_with_same_style_are_merged() {
        assert_eq!(to_spans("plain text & more").len(), 1);
    }
}
