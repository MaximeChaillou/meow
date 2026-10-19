//! The conversation page: transcript above, input and "New" below.

use meow::transcript::Transcript;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSTextField, NSTextView, NSView, NSWindow};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};

use super::controller::Controller;
use super::rich_text;
use super::widgets;

const MARGIN: f64 = 10.0;
const INPUT_HEIGHT: f64 = 28.0;
const BUTTON_WIDTH: f64 = 64.0;

pub struct ChatPage {
    pub view: Retained<NSView>,
    transcript: Retained<NSTextView>,
    input: Retained<NSTextField>,
}

impl ChatPage {
    pub fn new(mtm: MainThreadMarker, controller: &Controller, frame: NSRect) -> Self {
        let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let (width, height) = (frame.size.width, frame.size.height);
        let target: &AnyObject = controller;

        let input = widgets::text_field(mtm, "Ask the cat… (Esc: cancel / close)");
        input.setFrame(rect(
            MARGIN,
            MARGIN,
            width - 3.0 * MARGIN - BUTTON_WIDTH,
            INPUT_HEIGHT,
        ));
        input.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        // SAFETY: the controller lives as long as the app, so the weak target stays valid.
        unsafe {
            input.setTarget(Some(target));
            input.setAction(Some(sel!(submit:)));
        }
        // SAFETY: the controller outlives the field, so the weak delegate stays valid.
        unsafe { input.setDelegate(Some(ProtocolObject::from_ref(controller))) };
        view.addSubview(&input);

        let new_button = widgets::text_button(mtm, "New", target, sel!(newConversation:));
        new_button.setFrame(rect(
            width - MARGIN - BUTTON_WIDTH,
            MARGIN - 2.0,
            BUTTON_WIDTH,
            INPUT_HEIGHT + 4.0,
        ));
        new_button.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        view.addSubview(&new_button);

        let scroll = NSTextView::scrollableTextView(mtm);
        let top = 2.0 * MARGIN + INPUT_HEIGHT;
        scroll.setFrame(rect(
            MARGIN,
            top,
            width - 2.0 * MARGIN,
            height - top - MARGIN,
        ));
        scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        scroll.setDrawsBackground(false);
        let transcript = scroll
            .documentView()
            .and_then(|view| view.downcast::<NSTextView>().ok())
            .expect("scrollable text view contains a text view");
        transcript.setEditable(false);
        // The bubble's paper shows through.
        transcript.setDrawsBackground(false);
        transcript.setTextContainerInset(NSSize::new(4.0, 6.0));
        rich_text::style_links(&transcript);
        transcript.setDelegate(Some(ProtocolObject::from_ref(controller)));
        view.addSubview(&scroll);

        Self {
            view,
            transcript,
            input,
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

    /// Redraws the whole conversation; follows the end unless the user is folding steps.
    pub fn render(&self, transcript: &Transcript, scroll_to_end: bool) {
        let text = rich_text::transcript(&transcript.blocks());
        // SAFETY: the text view always owns a text storage.
        if let Some(storage) = unsafe { self.transcript.textStorage() } {
            storage.setAttributedString(&text);
            if scroll_to_end {
                self.transcript
                    .scrollRangeToVisible(NSRange::new(storage.length(), 0));
            }
        }
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
