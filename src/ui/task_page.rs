//! The tasks page: tasks grouped by source, with checkboxes, links and an input.

use std::cell::RefCell;

use meow::tasks::{Task, TaskGroup, TaskList};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSControlStateValueOff, NSControlStateValueOn,
    NSScrollView, NSTextField, NSView, NSWindow,
};
use objc2_foundation::{
    NSAttributedString, NSMutableAttributedString, NSPoint, NSRect, NSSize, NSString,
};

use super::controller::Controller;
use super::flipped::FlippedView;
use super::link_button::LinkButton;
use super::rich_text::{self, Attributes};
use super::theme;
use super::widgets;

const MARGIN: f64 = 10.0;
const INPUT_HEIGHT: f64 = 28.0;
const ADD_WIDTH: f64 = 64.0;
const ROW_HEIGHT: f64 = 26.0;
const HEADER_HEIGHT: f64 = 30.0;
const GROUP_GAP: f64 = 8.0;
const CHECKBOX_WIDTH: f64 = 22.0;
const CHAT_WIDTH: f64 = 76.0;
const INDENT: f64 = 6.0;

pub struct TaskPage {
    pub view: Retained<NSView>,
    scroll: Retained<NSScrollView>,
    list: Retained<FlippedView>,
    input: Retained<NSTextField>,
    target: Retained<Controller>,
    /// Task ids in the order shown; row buttons carry an index into it.
    shown_ids: RefCell<Vec<String>>,
    mtm: MainThreadMarker,
}

impl TaskPage {
    pub fn new(mtm: MainThreadMarker, controller: &Retained<Controller>, frame: NSRect) -> Self {
        let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let (width, height) = (frame.size.width, frame.size.height);
        let target: &AnyObject = controller;

        let input = widgets::text_field(mtm, "Add a task…");
        input.setFrame(rect(
            MARGIN,
            MARGIN,
            width - 3.0 * MARGIN - ADD_WIDTH,
            INPUT_HEIGHT,
        ));
        input.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        // SAFETY: the controller lives as long as the app, so the weak target stays valid.
        unsafe {
            input.setTarget(Some(target));
            input.setAction(Some(sel!(addTask:)));
        }
        view.addSubview(&input);

        let add = widgets::text_button(mtm, "Add", target, sel!(addTask:));
        add.setFrame(rect(
            width - MARGIN - ADD_WIDTH,
            MARGIN - 2.0,
            ADD_WIDTH,
            INPUT_HEIGHT + 4.0,
        ));
        add.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        view.addSubview(&add);

        let top = 2.0 * MARGIN + INPUT_HEIGHT;
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            rect(MARGIN, top, width - 2.0 * MARGIN, height - top - MARGIN),
        );
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);
        scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let list = FlippedView::new(mtm, rect(0.0, 0.0, scroll.contentSize().width, 0.0));
        list.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        scroll.setDocumentView(Some(&list));
        view.addSubview(&scroll);

        Self {
            view,
            scroll,
            list,
            input,
            target: controller.clone(),
            shown_ids: RefCell::new(Vec::new()),
            mtm,
        }
    }

    pub fn focus(&self, window: &NSWindow) {
        window.makeFirstResponder(Some(&self.input));
    }

    pub fn take_input(&self) -> String {
        let text = self.input.stringValue().to_string();
        self.input.setStringValue(&NSString::from_str(""));
        text
    }

    /// The id of the task in row `index`, as last rendered.
    pub fn id_at(&self, index: usize) -> Option<String> {
        self.shown_ids.borrow().get(index).cloned()
    }

    pub fn render(&self, tasks: &TaskList) {
        let width = self.scroll.contentSize().width;
        for row in self.list.subviews().iter() {
            row.removeFromSuperview();
        }
        let groups = tasks.groups();
        let mut ids = Vec::new();
        let mut y = 0.0;
        if groups.is_empty() {
            self.add_placeholder(width);
            y = ROW_HEIGHT;
        }
        for group in &groups {
            y = self.add_group(group, width, y, &mut ids) + GROUP_GAP;
        }
        let height = y.max(self.scroll.contentSize().height);
        self.list.setFrameSize(NSSize::new(width, height));
        self.shown_ids.replace(ids);
    }

    /// Draws a source header and its rows from `y`; returns the y below them.
    fn add_group(&self, group: &TaskGroup, width: f64, y: f64, ids: &mut Vec<String>) -> f64 {
        let header = NSTextField::labelWithAttributedString(&group_header(group), self.mtm);
        header.setFrame(rect(
            INDENT,
            y + 4.0,
            width - 2.0 * INDENT,
            HEADER_HEIGHT - 4.0,
        ));
        header.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.list.addSubview(&header);
        let mut y = y + HEADER_HEIGHT;
        for task in &group.tasks {
            self.add_row(ids.len(), task, width, y);
            ids.push(task.id.clone());
            y += ROW_HEIGHT;
        }
        y
    }

    fn add_placeholder(&self, width: f64) {
        let label = NSTextField::labelWithAttributedString(
            &rich_text::attributed(
                "No tasks yet. Run a quick action, or add one below.",
                &Attributes::plain(theme::muted(), theme::font(theme::BODY_SIZE)),
            ),
            self.mtm,
        );
        label.setFrame(rect(INDENT, 4.0, width - 2.0 * INDENT, ROW_HEIGHT));
        self.list.addSubview(&label);
    }

    /// A checkbox, the title (opens the task), and a button to chat about it.
    fn add_row(&self, index: usize, task: &Task, width: f64, y: f64) {
        let target: &AnyObject = &self.target;
        // SAFETY: the controller is an app-lifetime target that implements `toggleTask:`.
        let checkbox = unsafe {
            NSButton::checkboxWithTitle_target_action(
                &NSString::from_str(""),
                Some(target),
                Some(sel!(toggleTask:)),
                self.mtm,
            )
        };
        checkbox.setTag(index as isize);
        checkbox.setState(if task.is_done() {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        checkbox.setFrame(rect(2.0 * INDENT, y, CHECKBOX_WIDTH, ROW_HEIGHT));
        self.list.addSubview(&checkbox);

        let x = 2.0 * INDENT + CHECKBOX_WIDTH;
        let title = LinkButton::new(
            self.mtm,
            rect(x, y, width - x - CHAT_WIDTH - INDENT, ROW_HEIGHT),
            row_title(task, false),
            row_title(task, true),
            target,
            sel!(openTask:),
        );
        title.setTag(index as isize);
        let mut tip = match &task.url {
            Some(url) => format!("Open {url}"),
            None => "Chat about this task".to_string(),
        };
        if task.auto_done {
            tip.push_str("\nTicked automatically: resolved at the source.");
        }
        title.setToolTip(Some(&NSString::from_str(&tip)));
        title.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.list.addSubview(&title);

        let chat = widgets::text_button(self.mtm, "Chat", target, sel!(chatAboutTask:));
        chat.setTag(index as isize);
        chat.setToolTip(Some(&NSString::from_str(
            "Chat with the cat about this task",
        )));
        chat.setFrame(rect(width - CHAT_WIDTH, y, CHAT_WIDTH, ROW_HEIGHT));
        chat.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        self.list.addSubview(&chat);
    }
}

/// "GITHUB  2 open": the source in bold, its open count dimmed.
fn group_header(group: &TaskGroup) -> Retained<NSAttributedString> {
    let result = NSMutableAttributedString::new();
    let source = Attributes {
        heavy: true,
        ..Attributes::plain(theme::accent(), theme::font(theme::BODY_SIZE))
    };
    result.appendAttributedString(&rich_text::attributed(
        &group.source.to_uppercase(),
        &source,
    ));
    let count = match group.open_count() {
        0 => "  all done".to_string(),
        n => format!("  {n} open"),
    };
    let dim = Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE));
    result.appendAttributedString(&rich_text::attributed(&count, &dim));
    result.into_super()
}

/// The task title, faded and struck through once done. Hovering underlines it
/// in the accent color, to show it can be clicked.
fn row_title(task: &Task, hover: bool) -> Retained<NSAttributedString> {
    let color = if hover {
        theme::accent()
    } else if task.is_done() {
        theme::faint()
    } else {
        theme::ink()
    };
    let attributes = Attributes {
        strike: task.is_done() && !hover,
        underline: hover,
        ..Attributes::plain(color, theme::font(theme::BODY_SIZE))
    };
    let title = NSMutableAttributedString::new();
    title.appendAttributedString(&rich_text::attributed(&task.title, &attributes));
    if task.auto_done {
        // Ticked by the background check, not by hand.
        let mark = Attributes::plain(theme::faint(), theme::font(theme::SMALL_SIZE));
        title.appendAttributedString(&rich_text::attributed(" · auto", &mark));
    }
    title.into_super()
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
