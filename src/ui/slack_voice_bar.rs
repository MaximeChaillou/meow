//! The "write like me" controls above the Slack conversation list: a toggle,
//! a button to (re)learn the user's style from their messages, and a status.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSButton, NSTextField, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::controller::Controller;
use super::rich_text::{self, Attributes};
use super::theme;
use super::widgets;

pub const HEIGHT: f64 = 52.0;
const BUTTON_HEIGHT: f64 = 28.0;
const TOGGLE_WIDTH: f64 = 196.0;
const LEARN_WIDTH: f64 = 156.0;
const GAP: f64 = 8.0;

/// What the bar shows.
pub struct VoiceBarState<'a> {
    /// Drafts follow the style guide.
    pub enabled: bool,
    /// A style guide has been learned.
    pub learned: bool,
    /// Learning is under way.
    pub busy: bool,
    pub status: &'a str,
}

pub struct VoiceBar {
    pub view: Retained<NSView>,
    toggle: Retained<NSButton>,
    learn: Retained<NSButton>,
    status: Retained<NSTextField>,
}

impl VoiceBar {
    /// A bar filling `frame`, pinned to the top of its superview.
    pub fn new(mtm: MainThreadMarker, controller: &Retained<Controller>, frame: NSRect) -> Self {
        let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        let target: &AnyObject = controller;
        let top = frame.size.height - BUTTON_HEIGHT;

        let toggle = widgets::text_button(mtm, "Write as me: off", target, sel!(toggleWriteAsMe:));
        toggle.setFrame(rect(0.0, top, TOGGLE_WIDTH, BUTTON_HEIGHT));
        view.addSubview(&toggle);

        let learn = widgets::text_button(mtm, "Learn my style", target, sel!(learnMyStyle:));
        learn.setFrame(rect(TOGGLE_WIDTH + GAP, top, LEARN_WIDTH, BUTTON_HEIGHT));
        view.addSubview(&learn);

        let status = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        status.setFrame(rect(4.0, 0.0, frame.size.width - 4.0, top));
        status.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        view.addSubview(&status);

        Self {
            view,
            toggle,
            learn,
            status,
        }
    }

    pub fn render(&self, state: &VoiceBarState) {
        let toggle = if state.enabled {
            "Write as me: on"
        } else {
            "Write as me: off"
        };
        widgets::set_text_button_label(&self.toggle, toggle);
        let learn = if state.learned {
            "Relearn style"
        } else {
            "Learn my style"
        };
        widgets::set_text_button_label(&self.learn, learn);
        self.learn.setEnabled(!state.busy);
        self.status.setAttributedStringValue(&rich_text::attributed(
            state.status,
            &Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE)),
        ));
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
