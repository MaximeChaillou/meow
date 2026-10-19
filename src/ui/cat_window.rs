//! Transparent, click-through-free panel that shows the cat sprite.

use std::cell::{Cell, RefCell};

use meow::sprites::{CANVAS_H, CANVAS_W, Frame, PROP_H, PROP_W, SCRUFF};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSCursor, NSEvent, NSMenu, NSPanel,
    NSStatusWindowLevel, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use super::events::{UiEvent, push};

pub const SCALE: f64 = meow::sprites::SCALE;
pub const WIDTH: f64 = CANVAS_W as f64 * SCALE;
pub const HEIGHT: f64 = CANVAS_H as f64 * SCALE;
/// Mouse travel (in points) that turns a click into a drag.
const DRAG_THRESHOLD: f64 = 3.0;

/// A mouse press on the cat, in screen coordinates.
#[derive(Clone, Copy)]
struct Press {
    mouse: NSPoint,
    window_origin: NSPoint,
    dragging: bool,
}

#[derive(Default)]
pub struct CatViewIvars {
    frame: RefCell<Option<Frame>>,
    press: Cell<Option<Press>>,
    /// Last mouse position over the cat, to measure strokes.
    hover: Cell<Option<NSPoint>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "MeowCatView"]
    #[ivars = CatViewIvars]
    pub struct CatView;

    impl CatView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            let Some(window) = self.window() else { return };
            self.ivars().press.set(Some(Press {
                mouse: NSEvent::mouseLocation(),
                window_origin: window.frame().origin,
                dragging: false,
            }));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, _event: &NSEvent) {
            let Some(mut press) = self.ivars().press.get() else { return };
            let mouse = NSEvent::mouseLocation();
            let (dx, dy) = (mouse.x - press.mouse.x, mouse.y - press.mouse.y);
            if !press.dragging && dx.hypot(dy) >= DRAG_THRESHOLD {
                press.dragging = true;
                // Hang the cat by its scruff right under the cursor.
                press.window_origin = NSPoint::new(
                    press.mouse.x - SCRUFF.0 as f64 * SCALE,
                    press.mouse.y - (HEIGHT - (SCRUFF.1 as f64 + 0.5) * SCALE),
                );
                NSCursor::closedHandCursor().set();
                push(UiEvent::Grab);
            }
            if press.dragging {
                push(UiEvent::DragTo {
                    x: press.window_origin.x + dx,
                    y: press.window_origin.y + dy,
                });
            }
            self.ivars().press.set(Some(press));
        }

        /// Moving the mouse over the cat, without a button, strokes it.
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, _event: &NSEvent) {
            let mouse = NSEvent::mouseLocation();
            if let Some(last) = self.ivars().hover.replace(Some(mouse)) {
                let distance = (mouse.x - last.x).hypot(mouse.y - last.y);
                push(UiEvent::Stroke(distance));
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.ivars().hover.set(None);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            match self.ivars().press.take() {
                Some(Press { dragging: true, .. }) => {
                    NSCursor::arrowCursor().set();
                    push(UiEvent::Drop);
                }
                Some(_) => push(UiEvent::CatClicked),
                None => {}
            }
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw_pixels();
        }
    }
);

impl CatView {
    fn new(mtm: MainThreadMarker, rect: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(CatViewIvars::default());
        // SAFETY: NSView's designated initializer, called once on a fresh allocation.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: rect] };
        // Layer backing gives a cleared buffer on every redraw of the transparent window.
        view.setWantsLayer(true);
        let options = NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        let owner: &AnyObject = &view;
        // SAFETY: the view owns the area and outlives it; no user info is attached.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                rect,
                options,
                Some(owner),
                None,
            )
        };
        view.addTrackingArea(&area);
        view
    }

    pub fn show(&self, frame: Frame) {
        self.ivars().frame.replace(Some(frame));
        self.setNeedsDisplay(true);
    }

    fn draw_pixels(&self) {
        let frame = self.ivars().frame.borrow();
        let Some(frame) = frame.as_ref() else { return };
        for y in 0..frame.height {
            for x in 0..frame.width {
                let Some([r, g, b]) = frame.get(x, y) else {
                    continue;
                };
                let channel = |v: u8| f64::from(v) / 255.0;
                NSColor::colorWithSRGBRed_green_blue_alpha(channel(r), channel(g), channel(b), 1.0)
                    .setFill();
                let origin = NSPoint::new(x as f64 * SCALE, y as f64 * SCALE);
                NSBezierPath::fillRect(NSRect::new(origin, NSSize::new(SCALE, SCALE)));
            }
        }
    }
}

pub struct CatWindow {
    pub panel: Retained<NSPanel>,
    pub view: Retained<CatView>,
}

impl CatWindow {
    pub fn new(mtm: MainThreadMarker, menu: &NSMenu) -> Self {
        let window = Self::panel(mtm, NSSize::new(WIDTH, HEIGHT));
        // SAFETY: the menu is an app-lifetime object retained by the view.
        unsafe { window.view.setMenu(Some(menu)) };
        window.panel.orderFrontRegardless();
        window
    }

    /// A window for a prop next to the cat (its bowl, a mouse): it lets
    /// clicks through and starts hidden.
    pub fn prop(mtm: MainThreadMarker) -> Self {
        let size = NSSize::new(PROP_W as f64 * SCALE, PROP_H as f64 * SCALE);
        let window = Self::panel(mtm, size);
        window.panel.setIgnoresMouseEvents(true);
        window
    }

    fn panel(mtm: MainThreadMarker, size: NSSize) -> Self {
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), size);
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            rect,
            style,
            NSBackingStoreType::Buffered,
            false,
        );
        // SAFETY: the panel is owned by `CatWindow` for the app's lifetime.
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(false);
        // Mouse moves over the cat are how it gets petted.
        panel.setAcceptsMouseMovedEvents(true);
        panel.setHidesOnDeactivate(false);
        panel.setLevel(NSStatusWindowLevel);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );

        let view = CatView::new(mtm, rect);
        panel.setContentView(Some(&view));
        Self { panel, view }
    }

    pub fn move_to(&self, x: f64, y: f64) {
        self.panel
            .setFrameOrigin(NSPoint::new(x.round(), y.round()));
    }

    pub fn frame(&self) -> NSRect {
        self.panel.frame()
    }
}
