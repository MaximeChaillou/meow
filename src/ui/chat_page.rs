//! The conversation page: transcript above, input and "New" below.

use meow::transcript::Transcript;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSTextView, NSView, NSWindow};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize};

use super::bubble;
use super::controller::Controller;
use super::rich_text;
use super::widgets;

/// Space between the transcript and the page's edges.
const MARGIN: f64 = 16.0;
const BUTTON_WIDTH: f64 = 64.0;

pub struct ChatPage {
    pub view: Retained<NSView>,
    transcript: Retained<NSTextView>,
    input: Retained<widgets::GrowingField>,
}

impl ChatPage {
    pub fn new(mtm: MainThreadMarker, controller: &Controller, frame: NSRect) -> Self {
        let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        // The page runs under the bubble's right padding; content keeps clear of it.
        let (width, height) = (frame.size.width - bubble::PADDING, frame.size.height);
        let target: &AnyObject = controller;

        let input = widgets::text_field(mtm, "Ask the cat… (Esc: cancel / close)");
        // SAFETY: the controller lives as long as the app, so the weak target stays valid.
        unsafe {
            input.setTarget(Some(target));
            input.setAction(Some(sel!(submit:)));
        }
        // SAFETY: the controller outlives the field, so the weak delegate stays valid.
        unsafe { input.setDelegate(Some(ProtocolObject::from_ref(controller))) };

        let new_button = widgets::text_button(
            mtm,
            "New",
            widgets::ButtonStyle::Quiet,
            target,
            sel!(newConversation:),
        );
        let top = widgets::composer(
            mtm,
            &view,
            &input,
            &[],
            &[(&new_button, BUTTON_WIDTH, widgets::ButtonStyle::Quiet)],
        ) + MARGIN;

        let scroll = NSTextView::scrollableTextView(mtm);
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
        // Message boxes are text blocks, which only TextKit 1 lays out right;
        // asking for the layout manager switches the view over to it.
        // SAFETY: a plain getter on a text view the page owns.
        let _ = unsafe { transcript.layoutManager() };
        // The bubble's paper shows through.
        transcript.setDrawsBackground(false);
        transcript.setTextContainerInset(NSSize::new(4.0, 6.0));
        rich_text::style_links(&transcript);
        transcript.setDelegate(Some(ProtocolObject::from_ref(controller)));
        view.addSubview(&scroll);
        input.keep_clear(&scroll);

        Self {
            view,
            transcript,
            input,
        }
    }

    pub fn focus(&self, window: &NSWindow) {
        window.makeFirstResponder(Some(&self.input));
    }

    /// Puts `text` in the input, as if typed (debug snapshots).
    pub fn set_draft(&self, text: &str) {
        self.input.set_text(text);
    }

    pub fn take_input(&self) -> String {
        let text = self.input.stringValue().to_string();
        self.input.set_text("");
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
