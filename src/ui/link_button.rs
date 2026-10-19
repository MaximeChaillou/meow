//! A borderless text button that looks clickable: hand cursor and an
//! underlined, highlighted title while the mouse is over it.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_app_kit::{
    NSButton, NSCursor, NSEvent, NSTextAlignment, NSTrackingArea, NSTrackingAreaOptions,
};
use objc2_foundation::{NSAttributedString, NSRect};

#[derive(Default)]
pub struct LinkButtonIvars {
    normal: RefCell<Option<Retained<NSAttributedString>>>,
    hover: RefCell<Option<Retained<NSAttributedString>>>,
}

define_class!(
    #[unsafe(super(NSButton))]
    #[thread_kind = MainThreadOnly]
    #[name = "MeowLinkButton"]
    #[ivars = LinkButtonIvars]
    pub struct LinkButton;

    impl LinkButton {
        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            self.addCursorRect_cursor(self.bounds(), &NSCursor::pointingHandCursor());
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            if let Some(title) = self.ivars().hover.borrow().as_deref() {
                self.setAttributedTitle(title);
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            if let Some(title) = self.ivars().normal.borrow().as_deref() {
                self.setAttributedTitle(title);
            }
        }
    }
);

impl LinkButton {
    /// `normal` is shown at rest, `hover` while the mouse is over the button.
    pub fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        normal: Retained<NSAttributedString>,
        hover: Retained<NSAttributedString>,
        target: &AnyObject,
        action: Sel,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(LinkButtonIvars::default());
        // SAFETY: NSButton's designated initializer, called once on a fresh allocation.
        let button: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        button.setBordered(false);
        button.setAlignment(NSTextAlignment::Left);
        button.setAttributedTitle(&normal);
        // SAFETY: the target is the app-lifetime controller, which implements `action`.
        unsafe {
            button.setTarget(Some(target));
            button.setAction(Some(action));
        }
        button.ivars().normal.replace(Some(normal));
        button.ivars().hover.replace(Some(hover));

        let options = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        let owner: &AnyObject = &button;
        // SAFETY: the button owns the area and outlives it; no user info is attached.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                frame,
                options,
                Some(owner),
                None,
            )
        };
        button.addTrackingArea(&area);
        button
    }
}
