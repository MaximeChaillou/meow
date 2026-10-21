//! A downloaded picture (custom emoji, attached image): one image, or every
//! frame of an animated GIF with how long each shows. Text attachments only
//! draw a GIF's first frame, so the Slack page swaps frames in on each tick.

use std::path::Path;

use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSBitmapImageRep, NSImage, NSImageCurrentFrame, NSImageCurrentFrameDuration, NSImageFrameCount,
};
use objc2_foundation::{NSNumber, NSString};

/// Browsers show frames asking for less than this at [`DEFAULT_DELAY`];
/// GIFs are made to look right there.
const MIN_DELAY: f64 = 0.02;
const DEFAULT_DELAY: f64 = 0.1;
/// Decoded frames past this many pixels in all stay a still image.
const MAX_PIXELS: f64 = 12_000_000.0;

pub struct Picture {
    /// Never empty; the first is the still image.
    frames: Vec<Retained<NSImage>>,
    /// How long each frame shows, in seconds; empty when still.
    delays: Vec<f64>,
}

impl Picture {
    /// The picture in `file`, with its frames when it is an animated GIF.
    pub fn load(file: &Path) -> Result<Self, String> {
        let path = NSString::from_str(&file.to_string_lossy());
        let image = NSImage::initWithContentsOfFile(NSImage::alloc(), &path)
            .ok_or("not an image".to_string())?;
        Ok(animated(&image).unwrap_or_else(|| Self::new(image)))
    }

    /// A still picture.
    pub fn new(image: Retained<NSImage>) -> Self {
        Self {
            frames: vec![image],
            delays: Vec::new(),
        }
    }

    pub fn still(&self) -> &NSImage {
        &self.frames[0]
    }

    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// The frame to show `elapsed` seconds into the animation.
    pub fn frame_at(&self, elapsed: f64) -> (usize, &NSImage) {
        let index = frame_index(&self.delays, elapsed).min(self.frames.len() - 1);
        (index, &self.frames[index])
    }
}

/// Every frame of `image` and their delays, when it has several and they fit
/// the pixel budget.
fn animated(image: &NSImage) -> Option<Picture> {
    let rep = image
        .representations()
        .iter()
        .find_map(|rep| rep.downcast::<NSBitmapImageRep>().ok())?;
    // SAFETY: the keys are AppKit's own constants.
    let (count_key, current_key, duration_key) = unsafe {
        (
            NSImageFrameCount,
            NSImageCurrentFrame,
            NSImageCurrentFrameDuration,
        )
    };
    let count = number(&rep, count_key)?.integerValue();
    let pixels = (rep.pixelsWide() * rep.pixelsHigh()) as f64 * count as f64;
    if count < 2 || pixels > MAX_PIXELS {
        return None;
    }
    let size = image.size();
    let mut frames = Vec::new();
    let mut delays = Vec::new();
    for index in 0..count {
        // SAFETY: the current frame takes an NSNumber below the frame count.
        unsafe { rep.setProperty_withValue(current_key, Some(&NSNumber::new_isize(index))) };
        let frame = rep.CGImage()?;
        frames.push(NSImage::initWithCGImage_size(
            NSImage::alloc(),
            &frame,
            size,
        ));
        delays.push(delay(number(&rep, duration_key).map(|n| n.doubleValue())));
    }
    Some(Picture { frames, delays })
}

fn number(rep: &NSBitmapImageRep, key: &NSString) -> Option<Retained<NSNumber>> {
    rep.valueForProperty(key)?.downcast::<NSNumber>().ok()
}

/// How long a frame asking for `seconds` shows.
fn delay(seconds: Option<f64>) -> f64 {
    match seconds {
        Some(seconds) if seconds >= MIN_DELAY => seconds,
        _ => DEFAULT_DELAY,
    }
}

/// Which frame shows `elapsed` seconds in, looping forever.
fn frame_index(delays: &[f64], elapsed: f64) -> usize {
    let total: f64 = delays.iter().sum();
    if total <= 0.0 {
        return 0;
    }
    let mut left = elapsed.max(0.0) % total;
    for (index, delay) in delays.iter().enumerate() {
        if left < *delay {
            return index;
        }
        left -= delay;
    }
    delays.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_follow_their_delays_and_loop() {
        let delays = [0.1, 0.5, 0.2];
        assert_eq!(frame_index(&delays, 0.0), 0);
        assert_eq!(frame_index(&delays, 0.09), 0);
        assert_eq!(frame_index(&delays, 0.1), 1);
        assert_eq!(frame_index(&delays, 0.59), 1);
        assert_eq!(frame_index(&delays, 0.7), 2);
        assert_eq!(frame_index(&delays, 0.85), 0);
        assert_eq!(frame_index(&delays, 8.05), 0);
    }

    #[test]
    fn a_still_picture_stays_on_its_first_frame() {
        assert_eq!(frame_index(&[], 3.0), 0);
        assert_eq!(frame_index(&[0.0, 0.0], 3.0), 0);
    }

    #[test]
    fn too_short_or_missing_delays_get_the_browser_default() {
        assert_eq!(delay(Some(0.0)), DEFAULT_DELAY);
        assert_eq!(delay(Some(0.01)), DEFAULT_DELAY);
        assert_eq!(delay(None), DEFAULT_DELAY);
        assert_eq!(delay(Some(0.04)), 0.04);
    }
}
