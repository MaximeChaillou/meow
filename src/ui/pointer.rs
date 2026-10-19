//! Holds the mouse pointer still while the cat has it under its paw: the
//! pointer is moved under the paw and stops following the mouse until let go.

use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
use objc2_foundation::NSPoint;

#[repr(C)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWarpMouseCursorPosition(point: CGPoint) -> i32;
    fn CGAssociateMouseAndMouseCursorPosition(connected: u32) -> i32;
}

/// Puts the pointer at `at` (screen coordinates, origin bottom left) and
/// detaches it from the mouse.
pub fn hold(mtm: MainThreadMarker, at: NSPoint) {
    // Quartz measures from the top of the main screen, AppKit from the bottom.
    let Some(main) = NSScreen::screens(mtm).firstObject() else {
        return;
    };
    let point = CGPoint {
        x: at.x,
        y: main.frame().size.height - at.y,
    };
    // SAFETY: plain CoreGraphics calls taking values; they only fail (and
    // return an error code) without a window server.
    unsafe {
        CGWarpMouseCursorPosition(point);
        CGAssociateMouseAndMouseCursorPosition(0);
    }
}

/// Lets the pointer follow the mouse again.
pub fn release() {
    // SAFETY: as in `hold`.
    unsafe {
        CGAssociateMouseAndMouseCursorPosition(1);
    }
}
