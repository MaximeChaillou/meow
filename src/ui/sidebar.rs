//! Left column of the bubble: title, pages (Chat, Tasks) and the quick actions.

use std::cell::Cell;

use meow::actions::QuickAction;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSButton, NSTextAlignment, NSTextField};
use objc2_foundation::{NSAttributedString, NSPoint, NSRect, NSSize, NSString};

use super::controller::Controller;
use super::flipped::FlippedView;
use super::rich_text::{self, Attributes};
use super::theme;

pub const WIDTH: f64 = 156.0;
const PADDING: f64 = 6.0;
const TITLE_HEIGHT: f64 = 38.0;
const NAV_HEIGHT: f64 = 28.0;
const ACTION_HEIGHT: f64 = 26.0;
const SECTION_GAP: f64 = 16.0;
const NAV_SIZE: f64 = 21.0;
const ACTION_SIZE: f64 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Chat,
    Tasks,
    /// The day plan of the quick action shown.
    Plan,
    /// Slack conversations.
    Slack,
}

/// What the sidebar highlights: a page, or the quick action shown on the chat page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Chat,
    Tasks,
    Slack,
    Action(usize),
}

pub struct Sidebar {
    pub view: Retained<FlippedView>,
    title: Retained<NSTextField>,
    chat: Retained<NSButton>,
    tasks: Retained<NSButton>,
    slack: Retained<NSButton>,
    actions: Vec<(Retained<NSButton>, String)>,
    selected: Cell<Selection>,
    open_tasks: Cell<usize>,
    busy: Cell<bool>,
}

impl Sidebar {
    /// Builds the column at the left of `area` (the bubble's content area).
    pub fn new(
        mtm: MainThreadMarker,
        controller: &Controller,
        actions: &[QuickAction],
        area: NSRect,
    ) -> Self {
        let view = FlippedView::new(
            mtm,
            rect(area.origin.x, area.origin.y, WIDTH, area.size.height),
        );
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);

        let title = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        title.setFrame(rect(
            PADDING,
            2.0,
            WIDTH - 2.0 * PADDING,
            TITLE_HEIGHT - 4.0,
        ));
        view.addSubview(&title);

        let target: &AnyObject = controller;
        let mut y = TITLE_HEIGHT + PADDING;
        let chat = borderless_button(mtm, target, sel!(showChat:), y, NAV_HEIGHT);
        y += NAV_HEIGHT;
        let tasks = borderless_button(mtm, target, sel!(showTasks:), y, NAV_HEIGHT);
        y += NAV_HEIGHT;
        let slack = borderless_button(mtm, target, sel!(showSlack:), y, NAV_HEIGHT);
        y += NAV_HEIGHT + SECTION_GAP;
        view.addSubview(&chat);
        view.addSubview(&tasks);
        view.addSubview(&slack);

        if !actions.is_empty() {
            let label = NSTextField::labelWithString(&NSString::from_str("QUICK ACTIONS"), mtm);
            label.setFont(Some(&theme::font(theme::SMALL_SIZE)));
            label.setTextColor(Some(&theme::muted()));
            label.setFrame(rect(PADDING, y, WIDTH - 2.0 * PADDING, 20.0));
            view.addSubview(&label);
            y += 22.0;
        }
        let mut action_buttons = Vec::new();
        for (index, action) in actions.iter().enumerate() {
            let button = borderless_button(mtm, target, sel!(quickAction:), y, ACTION_HEIGHT);
            button.setTag(index as isize);
            button.setToolTip(Some(&NSString::from_str(&format!(
                "Show the last result of {}",
                action.title
            ))));
            view.addSubview(&button);
            action_buttons.push((button, action.title.clone()));
            y += ACTION_HEIGHT;
        }

        let sidebar = Self {
            view,
            title,
            chat,
            tasks,
            slack,
            actions: action_buttons,
            selected: Cell::new(Selection::Chat),
            open_tasks: Cell::new(0),
            busy: Cell::new(false),
        };
        sidebar.refresh();
        sidebar
    }

    pub fn select(&self, selection: Selection) {
        self.selected.set(selection);
        self.refresh();
    }

    pub fn set_open_tasks(&self, count: usize) {
        self.open_tasks.set(count);
        self.refresh();
    }

    pub fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        self.refresh();
    }

    fn refresh(&self) {
        let title = if self.busy.get() {
            "MEOW ···"
        } else {
            "MEOW"
        };
        self.title.setAttributedStringValue(&rich_text::attributed(
            title,
            &Attributes {
                heavy: true,
                ..Attributes::plain(theme::accent(), theme::font(theme::TITLE_SIZE))
            },
        ));
        let tasks = match self.open_tasks.get() {
            0 => "Tasks".to_string(),
            n => format!("Tasks ({n})"),
        };
        let selected = self.selected.get();
        self.chat
            .setAttributedTitle(&nav_title("Chat", NAV_SIZE, selected == Selection::Chat));
        self.tasks
            .setAttributedTitle(&nav_title(&tasks, NAV_SIZE, selected == Selection::Tasks));
        self.slack
            .setAttributedTitle(&nav_title("Slack", NAV_SIZE, selected == Selection::Slack));
        for (index, (button, title)) in self.actions.iter().enumerate() {
            let on = selected == Selection::Action(index);
            button.setAttributedTitle(&nav_title(title, ACTION_SIZE, on));
        }
    }
}

/// "> Chat" in heavy orange when selected, plain ink otherwise.
fn nav_title(text: &str, size: f64, selected: bool) -> Retained<NSAttributedString> {
    let (marker, color) = if selected {
        ("> ", theme::accent())
    } else {
        ("  ", theme::ink())
    };
    rich_text::attributed(
        &format!("{marker}{text}"),
        &Attributes {
            heavy: selected,
            ..Attributes::plain(color, theme::font(size))
        },
    )
}

fn borderless_button(
    mtm: MainThreadMarker,
    target: &AnyObject,
    action: Sel,
    y: f64,
    height: f64,
) -> Retained<NSButton> {
    // SAFETY: the controller is an app-lifetime target that implements every sidebar action.
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(""),
            Some(target),
            Some(action),
            mtm,
        )
    };
    button.setBordered(false);
    button.setAlignment(NSTextAlignment::Left);
    button.setFrame(rect(PADDING, y, WIDTH - 2.0 * PADDING, height));
    button
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
