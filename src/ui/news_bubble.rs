//! The bubble above the cat's head that shows where unread news comes from,
//! in place of its `!`: a click-through panel that follows the cat.

use meow::source::NewsBubble;
use meow::sprites::{BUBBLE_TAIL_X, CANVAS_W, CAT_H, Facing};
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSImageView, NSPanel, NSStatusWindowLevel,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use super::cat_window::SCALE;
use super::widgets;

/// Sprite pixels between the tail's tip and the top of the cat's ears.
const GAP: f64 = 1.0;

pub struct NewsBubbleWindow {
    panel: Retained<NSPanel>,
    image: Retained<NSImageView>,
    /// The tail's column and the bubble's size, as last shown.
    tip_x: usize,
    size: NSSize,
}

impl NewsBubbleWindow {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            rect,
            style,
            NSBackingStoreType::Buffered,
            false,
        );
        // SAFETY: the panel is owned by this window for the app's lifetime.
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(false);
        panel.setIgnoresMouseEvents(true);
        panel.setHidesOnDeactivate(false);
        panel.setLevel(NSStatusWindowLevel);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        let image = NSImageView::initWithFrame(NSImageView::alloc(mtm), rect);
        panel.setContentView(Some(&image));
        Self {
            panel,
            image,
            tip_x: 0,
            size: NSSize::new(0.0, 0.0),
        }
    }

    /// Draws `bubble`; it shows from the next [`Self::place`].
    pub fn show(&mut self, bubble: &NewsBubble) {
        let frame = &bubble.frame;
        self.size = NSSize::new(frame.width as f64 * SCALE, frame.height as f64 * SCALE);
        self.tip_x = bubble.tip_x;
        self.image
            .setImage(Some(&widgets::pixel_image(frame, SCALE)));
        self.panel.setContentSize(self.size);
        self.image
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), self.size));
    }

    /// Puts the tail just above the head of the cat drawn at `cat_origin`
    /// (screen coordinates), kept within the width of `screen`.
    pub fn place(&self, cat_origin: NSPoint, facing: Facing, screen: NSRect) {
        let head_x = match facing {
            Facing::Right => BUBBLE_TAIL_X,
            Facing::Left => CANVAS_W - 1 - BUBBLE_TAIL_X,
        };
        let x = cat_origin.x + (head_x as f64 - self.tip_x as f64) * SCALE;
        let max_x = screen.origin.x + screen.size.width - self.size.width;
        let x = x.min(max_x).max(screen.origin.x);
        let y = cat_origin.y + (CAT_H as f64 + GAP) * SCALE;
        self.panel
            .setFrameOrigin(NSPoint::new(x.round(), y.round()));
        if !self.panel.isVisible() {
            self.panel.orderFrontRegardless();
        }
    }

    pub fn hide(&self) {
        if self.panel.isVisible() {
            self.panel.orderOut(None);
        }
    }
}
