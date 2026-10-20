//! Which of the displays something is on, so the cat can live on any of them.

use meow::behavior::OpenEdges;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::NSScreen;
use objc2_foundation::{NSPoint, NSRect, NSSize};

/// The screen showing `point` (screen coordinates), or the nearest one when
/// it falls in a gap between screens of different sizes.
pub fn showing(mtm: MainThreadMarker, point: NSPoint) -> Option<Retained<NSScreen>> {
    NSScreen::screens(mtm)
        .iter()
        .min_by(|a, b| distance(a.frame(), point).total_cmp(&distance(b.frame(), point)))
}

/// The screen other than the one with `frame` that shows the most of `rect`,
/// if one shows any of it.
pub fn next_under(
    mtm: MainThreadMarker,
    rect: NSRect,
    frame: NSRect,
) -> Option<Retained<NSScreen>> {
    NSScreen::screens(mtm)
        .iter()
        .filter(|screen| screen.frame() != frame)
        .map(|screen| (overlap(screen.frame(), rect), screen))
        .filter(|&(area, _)| area > 0.0)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, screen)| screen)
}

/// Which edges of `screen` lead straight on to another screen: its sides at
/// the height of `point`, its top above `point`.
pub fn open_edges(mtm: MainThreadMarker, screen: &NSScreen, point: NSPoint) -> OpenEdges {
    let frame = screen.frame();
    let shown = |x: f64, y: f64| {
        let beyond = NSPoint::new(x, y);
        NSScreen::screens(mtm)
            .iter()
            .any(|other| distance(other.frame(), beyond) == 0.0)
    };
    OpenEdges {
        left: shown(frame.origin.x - 1.0, point.y),
        right: shown(frame.origin.x + frame.size.width + 1.0, point.y),
        top: shown(point.x, frame.origin.y + frame.size.height + 1.0),
    }
}

/// Where a window of `size` at `origin` goes to be whole on the screen with
/// `frame`. A window across two displays shows on only one of them, and
/// flickers as it changes over.
pub fn keep_whole_on(frame: NSRect, origin: NSPoint, size: NSSize) -> NSPoint {
    let fit = |v: f64, min: f64, len: f64, own: f64| v.min(min + len - own).max(min);
    NSPoint::new(
        fit(origin.x, frame.origin.x, frame.size.width, size.width),
        fit(origin.y, frame.origin.y, frame.size.height, size.height),
    )
}

/// The area `a` and `b` have in common.
fn overlap(a: NSRect, b: NSRect) -> f64 {
    let span = |a0: f64, a_len: f64, b0: f64, b_len: f64| {
        ((a0 + a_len).min(b0 + b_len) - a0.max(b0)).max(0.0)
    };
    span(a.origin.x, a.size.width, b.origin.x, b.size.width)
        * span(a.origin.y, a.size.height, b.origin.y, b.size.height)
}

/// How far `point` is from `rect`: zero inside it.
fn distance(rect: NSRect, point: NSPoint) -> f64 {
    let gap = |v: f64, min: f64, len: f64| (min - v).max(v - (min + len)).max(0.0);
    let dx = gap(point.x, rect.origin.x, rect.size.width);
    let dy = gap(point.y, rect.origin.y, rect.size.height);
    dx.hypot(dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }

    #[test]
    fn a_window_across_the_edge_is_pulled_whole_onto_the_screen() {
        let screen = rect(0.0, 0.0, 1000.0, 800.0);
        let size = NSSize::new(72.0, 90.0);
        let at = keep_whole_on(screen, NSPoint::new(980.0, 790.0), size);
        assert_eq!((at.x, at.y), (928.0, 710.0));
        let inside = keep_whole_on(screen, NSPoint::new(300.0, 0.0), size);
        assert_eq!((inside.x, inside.y), (300.0, 0.0));
    }

    #[test]
    fn overlap_is_the_shared_area() {
        let screen = rect(1000.0, 0.0, 1000.0, 800.0);
        assert_eq!(overlap(screen, rect(980.0, 100.0, 72.0, 90.0)), 52.0 * 90.0);
        assert_eq!(overlap(screen, rect(928.0, 100.0, 72.0, 90.0)), 0.0);
    }

    #[test]
    fn distance_is_zero_inside_and_to_the_nearest_edge_outside() {
        let screen = rect(0.0, 0.0, 1000.0, 800.0);
        assert_eq!(distance(screen, NSPoint::new(500.0, 400.0)), 0.0);
        assert_eq!(distance(screen, NSPoint::new(1010.0, 400.0)), 10.0);
        assert_eq!(distance(screen, NSPoint::new(1003.0, 804.0)), 5.0);
    }
}
