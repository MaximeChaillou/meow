//! The Slack page: a list of conversations, a message view with a reply box,
//! and a signed-out state offering to sign in.
//!
//! Like the other pages it is pure view. The app fills it with
//! [`SlackPage::show_signed_out`], [`SlackPage::render_list`] and
//! [`SlackPage::render_conversation`], and reads back the row index of a click
//! ([`SlackPage::id_at`]) and the typed reply ([`SlackPage::take_input`]).

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSScrollView, NSTextAlignment, NSTextField, NSTextView,
    NSView, NSWindow,
};
use objc2_foundation::{
    NSAttributedString, NSMutableAttributedString, NSPoint, NSRange, NSRect, NSSize, NSString,
};

use super::controller::Controller;
use super::flipped::FlippedView;
use super::link_button::LinkButton;
use super::rich_text::{self, Attributes};
use super::slack_assist::{self, AssistPanel, AssistView};
use super::slack_voice_bar::{self, VoiceBar, VoiceBarState};
use super::theme;
use super::widgets;

const MARGIN: f64 = 10.0;
const INPUT_HEIGHT: f64 = 28.0;
const SEND_WIDTH: f64 = 64.0;
const ASK_WIDTH: f64 = 60.0;
const AUTO_WIDTH: f64 = 68.0;
const AGENT_WIDTH: f64 = 76.0;
const BACK_WIDTH: f64 = 96.0;
const TOP_HEIGHT: f64 = 28.0;
const ROW_HEIGHT: f64 = 30.0;
const INDENT: f64 = 6.0;
/// Room on the right of a row for its mention count and time.
const META_WIDTH: f64 = 120.0;

/// One conversation in the list.
pub struct ConversationRow {
    pub id: String,
    pub title: String,
    pub unread: bool,
    /// Unread mentions of the user.
    pub mentions: u32,
    /// When it was last active ("14:32", "Mon"); may be empty.
    pub time: String,
}

/// A piece of the open conversation, names resolved and bodies in Markdown.
pub enum TranscriptItem {
    Day(String),
    /// Consecutive messages by one author, under a single header.
    Run {
        author: String,
        time: String,
        mine: bool,
        bodies: Vec<String>,
    },
}

pub struct SlackPage {
    pub view: Retained<NSView>,
    /// Shown signed out: a prompt and a sign-in button.
    signin: Retained<NSButton>,
    status: Retained<NSTextField>,
    /// Conversation list (list mode).
    list_scroll: Retained<NSScrollView>,
    list: Retained<FlippedView>,
    /// Message view and reply box (conversation mode).
    convo_scroll: Retained<NSScrollView>,
    transcript: Retained<NSTextView>,
    input: Retained<NSTextField>,
    send: Retained<NSButton>,
    /// Asks Claude privately about the conversation, with the typed question.
    ask: Retained<NSButton>,
    /// Has Claude research and draft the reply, with the user's tools.
    auto: Retained<NSButton>,
    /// Has Claude propose a coding agent for the conversation.
    agent: Retained<NSButton>,
    /// "Write like me" controls, above the list.
    voice: VoiceBar,
    assist: AssistPanel,
    back: Retained<NSButton>,
    convo_title: Retained<NSTextField>,
    /// Conversation ids in the order shown; row buttons carry an index into it.
    shown_ids: RefCell<Vec<String>>,
    target: Retained<Controller>,
    mtm: MainThreadMarker,
}

impl SlackPage {
    pub fn new(mtm: MainThreadMarker, controller: &Retained<Controller>, frame: NSRect) -> Self {
        let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let (width, height) = (frame.size.width, frame.size.height);
        let target: &AnyObject = controller;

        // --- Signed-out state ---------------------------------------------
        let signin = widgets::text_button(mtm, "Sign in to Slack", target, sel!(signInSlack:));
        signin.setFrame(rect(
            MARGIN,
            height / 2.0,
            width - 2.0 * MARGIN,
            INPUT_HEIGHT,
        ));
        signin.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        view.addSubview(&signin);

        let status = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        status.setFrame(rect(
            MARGIN,
            height / 2.0 - ROW_HEIGHT,
            width - 2.0 * MARGIN,
            ROW_HEIGHT,
        ));
        status.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        view.addSubview(&status);

        // --- List mode ----------------------------------------------------
        let list_scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            rect(
                MARGIN,
                MARGIN,
                width - 2.0 * MARGIN,
                height - 3.0 * MARGIN - slack_voice_bar::HEIGHT,
            ),
        );
        list_scroll.setHasVerticalScroller(true);
        list_scroll.setDrawsBackground(false);
        list_scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let list = FlippedView::new(mtm, rect(0.0, 0.0, list_scroll.contentSize().width, 0.0));
        list.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        list_scroll.setDocumentView(Some(&list));
        view.addSubview(&list_scroll);

        let voice = VoiceBar::new(
            mtm,
            controller,
            rect(
                MARGIN,
                height - MARGIN - slack_voice_bar::HEIGHT,
                width - 2.0 * MARGIN,
                slack_voice_bar::HEIGHT,
            ),
        );
        view.addSubview(&voice.view);

        // --- Conversation mode --------------------------------------------
        let back = widgets::text_button(mtm, "‹ All chats", target, sel!(slackBack:));
        back.setFrame(rect(MARGIN, height - TOP_HEIGHT, BACK_WIDTH, TOP_HEIGHT));
        back.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
        view.addSubview(&back);

        let convo_title = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        convo_title.setFrame(rect(
            MARGIN + BACK_WIDTH + INDENT,
            height - TOP_HEIGHT,
            width - 2.0 * MARGIN - BACK_WIDTH - INDENT,
            TOP_HEIGHT,
        ));
        convo_title.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        view.addSubview(&convo_title);

        let input = widgets::text_field(mtm, "Reply, or ask Claude… (Esc: back)");
        input.setFrame(rect(
            MARGIN,
            MARGIN,
            width - 6.0 * MARGIN - SEND_WIDTH - ASK_WIDTH - AUTO_WIDTH - AGENT_WIDTH,
            INPUT_HEIGHT,
        ));
        input.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        // SAFETY: the controller lives as long as the app, so the weak target/delegate stay valid.
        unsafe {
            input.setTarget(Some(target));
            input.setAction(Some(sel!(sendSlack:)));
            input.setDelegate(Some(ProtocolObject::from_ref(&**controller)));
        }
        // Send on Return only. By default the field also fires its action when
        // editing ends (focus moving, a click elsewhere), which would post a
        // half-written or freshly drafted message.
        if let Some(cell) = input.cell() {
            cell.setSendsActionOnEndEditing(false);
        }
        view.addSubview(&input);

        let send = widgets::text_button(mtm, "Send", target, sel!(sendSlack:));
        send.setFrame(rect(
            width - MARGIN - SEND_WIDTH,
            MARGIN - 2.0,
            SEND_WIDTH,
            INPUT_HEIGHT + 4.0,
        ));
        send.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        view.addSubview(&send);

        let ask = widgets::text_button(mtm, "Ask", target, sel!(askSlack:));
        ask.setFrame(rect(
            width - 2.0 * MARGIN - SEND_WIDTH - ASK_WIDTH,
            MARGIN - 2.0,
            ASK_WIDTH,
            INPUT_HEIGHT + 4.0,
        ));
        ask.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        view.addSubview(&ask);

        let auto = widgets::text_button(mtm, "Auto", target, sel!(autocompleteSlack:));
        auto.setFrame(rect(
            width - 3.0 * MARGIN - SEND_WIDTH - ASK_WIDTH - AUTO_WIDTH,
            MARGIN - 2.0,
            AUTO_WIDTH,
            INPUT_HEIGHT + 4.0,
        ));
        auto.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        view.addSubview(&auto);

        let agent = widgets::text_button(mtm, "Agent", target, sel!(agentSlack:));
        agent.setToolTip(Some(&NSString::from_str(
            "Start a coding agent in Superset for this conversation",
        )));
        agent.setFrame(rect(
            width - 4.0 * MARGIN - SEND_WIDTH - ASK_WIDTH - AUTO_WIDTH - AGENT_WIDTH,
            MARGIN - 2.0,
            AGENT_WIDTH,
            INPUT_HEIGHT + 4.0,
        ));
        agent.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        view.addSubview(&agent);

        let assist = AssistPanel::new(
            mtm,
            controller,
            rect(
                MARGIN,
                2.0 * MARGIN + INPUT_HEIGHT,
                width - 2.0 * MARGIN,
                slack_assist::HEIGHT,
            ),
        );
        view.addSubview(&assist.view);

        let convo_scroll = NSTextView::scrollableTextView(mtm);
        let top = 2.0 * MARGIN + INPUT_HEIGHT;
        convo_scroll.setFrame(rect(
            MARGIN,
            top,
            width - 2.0 * MARGIN,
            height - top - TOP_HEIGHT,
        ));
        convo_scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        convo_scroll.setDrawsBackground(false);
        let transcript = convo_scroll
            .documentView()
            .and_then(|view| view.downcast::<NSTextView>().ok())
            .expect("scrollable text view contains a text view");
        transcript.setEditable(false);
        transcript.setDrawsBackground(false);
        transcript.setTextContainerInset(NSSize::new(4.0, 6.0));
        rich_text::style_links(&transcript);
        transcript.setDelegate(Some(ProtocolObject::from_ref(&**controller)));
        view.addSubview(&convo_scroll);

        let page = Self {
            view,
            signin,
            status,
            list_scroll,
            list,
            convo_scroll,
            transcript,
            input,
            send,
            ask,
            auto,
            agent,
            voice,
            assist,
            back,
            convo_title,
            shown_ids: RefCell::new(Vec::new()),
            target: controller.clone(),
            mtm,
        };
        page.show_signed_out(None);
        page
    }

    pub fn focus(&self, window: &NSWindow) {
        if !self.input.isHidden() {
            window.makeFirstResponder(Some(&self.input));
        }
    }

    pub fn take_input(&self) -> String {
        let text = self.input.stringValue().to_string();
        self.input.setStringValue(&NSString::from_str(""));
        text
    }

    /// Puts `text` in the reply box, ready to edit and send.
    /// Whether the reply box holds only whitespace.
    pub fn input_is_empty(&self) -> bool {
        self.input.stringValue().to_string().trim().is_empty()
    }

    /// Nothing is sent: the user reviews it and presses Return or Send.
    pub fn set_input(&self, text: &str, window: &NSWindow) {
        let text = NSString::from_str(text);
        if self.input.currentEditor().is_none() {
            window.makeFirstResponder(Some(&self.input));
        }
        self.input.setStringValue(&text);
        // Caret at the end, ready to edit, rather than the whole draft selected.
        if let Some(editor) = self.input.currentEditor() {
            editor.setString(&text);
            editor.setSelectedRange(NSRange::new(text.length(), 0));
        }
    }

    /// Updates the "write like me" controls above the list.
    pub fn render_voice(&self, state: &VoiceBarState) {
        self.voice.render(state);
    }

    /// Shows the private Claude panel above the reply box.
    pub fn show_assist(&self, content: &AssistView) {
        self.assist.show(content);
        self.layout_transcript();
    }

    pub fn hide_assist(&self) {
        self.assist.hide();
        self.layout_transcript();
    }

    pub fn assist_shown(&self) -> bool {
        self.assist.is_shown()
    }

    /// Fits the transcript between the title and the reply box (and the Claude
    /// panel, when it is shown).
    fn layout_transcript(&self) {
        let size = self.view.bounds().size;
        let mut bottom = 2.0 * MARGIN + INPUT_HEIGHT;
        if self.assist.is_shown() {
            bottom += slack_assist::HEIGHT + MARGIN;
        }
        self.convo_scroll.setFrame(rect(
            MARGIN,
            bottom,
            size.width - 2.0 * MARGIN,
            (size.height - bottom - TOP_HEIGHT).max(0.0),
        ));
    }

    /// The conversation id of row `index`, as last listed.
    pub fn id_at(&self, index: usize) -> Option<String> {
        self.shown_ids.borrow().get(index).cloned()
    }

    /// Whether a conversation (not the list) is currently shown.
    pub fn in_conversation(&self) -> bool {
        !self.convo_scroll.isHidden()
    }

    /// Shows the sign-in prompt, optionally with a status line (e.g. an error).
    pub fn show_signed_out(&self, message: Option<&str>) {
        self.signin.setHidden(false);
        self.status.setHidden(false);
        self.list_scroll.setHidden(true);
        self.voice.view.setHidden(true);
        self.set_conversation_hidden(true);
        let text = message.unwrap_or("Sign in with your usual Slack login.");
        self.status.setAttributedStringValue(&rich_text::attributed(
            text,
            &Attributes::plain(theme::muted(), theme::font(theme::BODY_SIZE)),
        ));
    }

    /// Shows the conversation list.
    pub fn render_list(&self, rows: &[ConversationRow]) {
        self.signin.setHidden(true);
        self.status.setHidden(true);
        self.set_conversation_hidden(true);
        self.list_scroll.setHidden(false);
        self.voice.view.setHidden(false);

        let width = self.list_scroll.contentSize().width;
        for row in self.list.subviews().iter() {
            row.removeFromSuperview();
        }
        let mut ids = Vec::new();
        let mut y = 0.0;
        if rows.is_empty() {
            self.add_placeholder(width);
            y = ROW_HEIGHT;
        }
        for row in rows {
            self.add_row(ids.len(), row, width, y);
            ids.push(row.id.clone());
            y += ROW_HEIGHT;
        }
        let height = y.max(self.list_scroll.contentSize().height);
        self.list.setFrameSize(NSSize::new(width, height));
        self.shown_ids.replace(ids);
    }

    /// Shows one conversation's messages and the reply box.
    pub fn render_conversation(&self, title: &str, items: &[TranscriptItem]) {
        self.signin.setHidden(true);
        self.status.setHidden(true);
        self.list_scroll.setHidden(true);
        self.voice.view.setHidden(true);
        self.set_conversation_hidden(false);

        self.convo_title
            .setAttributedStringValue(&rich_text::attributed(
                title,
                &Attributes {
                    heavy: true,
                    ..Attributes::plain(theme::ink(), theme::font(theme::BODY_SIZE))
                },
            ));
        let text = NSMutableAttributedString::new();
        for item in items {
            let piece = match item {
                TranscriptItem::Day(label) => rich_text::slack_day(label),
                TranscriptItem::Run {
                    author,
                    time,
                    mine,
                    bodies,
                } => rich_text::slack_run(author, time, *mine, bodies),
            };
            text.appendAttributedString(&piece);
        }
        // SAFETY: the text view always owns a text storage.
        if let Some(storage) = unsafe { self.transcript.textStorage() } {
            storage.setAttributedString(&text);
            self.transcript
                .scrollRangeToVisible(NSRange::new(storage.length(), 0));
        }
    }

    fn set_conversation_hidden(&self, hidden: bool) {
        self.convo_scroll.setHidden(hidden);
        self.input.setHidden(hidden);
        self.send.setHidden(hidden);
        self.ask.setHidden(hidden);
        self.auto.setHidden(hidden);
        self.agent.setHidden(hidden);
        if hidden {
            self.assist.hide();
            self.layout_transcript();
        }
        self.back.setHidden(hidden);
        self.convo_title.setHidden(hidden);
    }

    fn add_placeholder(&self, width: f64) {
        let label = NSTextField::labelWithAttributedString(
            &rich_text::attributed(
                "No direct messages yet.",
                &Attributes::plain(theme::muted(), theme::font(theme::BODY_SIZE)),
            ),
            self.mtm,
        );
        label.setFrame(rect(INDENT, 4.0, width - 2.0 * INDENT, ROW_HEIGHT));
        self.list.addSubview(&label);
    }

    /// A clickable conversation name, with a dot when it has unread messages,
    /// and its mention count and last activity on the right.
    fn add_row(&self, index: usize, row: &ConversationRow, width: f64, y: f64) {
        let target: &AnyObject = &self.target;
        let marker = if row.unread { "● " } else { "   " };
        let label = format!("{marker}{}", row.title);
        let button = LinkButton::new(
            self.mtm,
            rect(INDENT, y, width - 2.0 * INDENT - META_WIDTH, ROW_HEIGHT),
            row_title(&label, row.unread, false),
            row_title(&label, row.unread, true),
            target,
            sel!(openSlack:),
        );
        button.setTag(index as isize);
        button.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.list.addSubview(&button);
        self.add_meta(row, width, y);
    }

    /// The right-aligned "@2  14:32" of a row.
    fn add_meta(&self, row: &ConversationRow, width: f64, y: f64) {
        let meta = NSMutableAttributedString::new();
        if row.mentions > 0 {
            meta.appendAttributedString(&rich_text::attributed(
                &format!("@{}  ", row.mentions),
                &Attributes {
                    heavy: true,
                    ..Attributes::plain(theme::accent(), theme::font(theme::SMALL_SIZE))
                },
            ));
        }
        meta.appendAttributedString(&rich_text::attributed(
            &row.time,
            &Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE)),
        ));
        let label = NSTextField::labelWithAttributedString(&meta, self.mtm);
        label.setAlignment(NSTextAlignment::Right);
        let height = label.fittingSize().height;
        label.setFrame(rect(
            width - INDENT - META_WIDTH,
            y + (ROW_HEIGHT - height) / 2.0,
            META_WIDTH,
            height,
        ));
        label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        self.list.addSubview(&label);
    }
}

/// A conversation name; bold for unread, accent-underlined on hover.
fn row_title(text: &str, unread: bool, hover: bool) -> Retained<NSAttributedString> {
    let color = if hover {
        theme::accent()
    } else if unread {
        theme::ink()
    } else {
        theme::muted()
    };
    rich_text::attributed(
        text,
        &Attributes {
            heavy: unread && !hover,
            underline: hover,
            ..Attributes::plain(color, theme::font(theme::BODY_SIZE))
        },
    )
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
