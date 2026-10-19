//! The pixel-art look: the cat's palette and the bundled 8-bit fonts.
//!
//! The font is VT323 (SIL Open Font License, see assets/fonts), embedded in
//! the binary.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSFont};
use objc2_core_foundation::{CFData, CFRetained};
use objc2_core_text::{CTFont, CTFontDescriptor, CTFontManagerCreateFontDescriptorFromData};

const PIXEL_FONT: &str = "VT323-Regular";

const FONTS: [(&str, &[u8]); 1] = [(
    PIXEL_FONT,
    include_bytes!("../../assets/fonts/VT323-400.ttf"),
)];

/// VT323 is drawn small for its point size, so sizes run larger than usual.
pub const BODY_SIZE: f64 = 18.0;
pub const SMALL_SIZE: f64 = 16.0;
pub const TITLE_SIZE: f64 = 28.0;

thread_local! {
    /// The bundled fonts, loaded straight from the binary; no system registration needed.
    static LOADED: RefCell<Vec<(&'static str, CFRetained<CTFontDescriptor>)>> =
        const { RefCell::new(Vec::new()) };
}

/// Loads the bundled fonts. Text falls back to the system font for any that fail.
pub fn register_fonts() -> Result<(), String> {
    let mut missing = Vec::new();
    LOADED.with(|loaded| {
        let mut loaded = loaded.borrow_mut();
        loaded.clear();
        for (name, bytes) in FONTS {
            // SAFETY: the data is a complete font file embedded in the binary.
            match unsafe { CTFontManagerCreateFontDescriptorFromData(&CFData::from_bytes(bytes)) } {
                Some(descriptor) => loaded.push((name, descriptor)),
                None => missing.push(name),
            }
        }
    });
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Could not load the pixel font ({}).",
            missing.join(", ")
        ))
    }
}

fn named(name: &str, size: f64) -> Option<Retained<NSFont>> {
    LOADED.with(|loaded| {
        let loaded = loaded.borrow();
        let (_, descriptor) = loaded
            .iter()
            .find(|(loaded_name, _)| *loaded_name == name)?;
        // SAFETY: a valid descriptor; a null matrix means the identity transform.
        let font = unsafe { CTFont::with_font_descriptor(descriptor, size, std::ptr::null()) };
        // SAFETY: CTFont is toll-free bridged to NSFont, and ownership moves with the pointer.
        unsafe { Retained::from_raw(CFRetained::into_raw(font).as_ptr().cast::<NSFont>()) }
    })
}

/// The pixel font. It has a single weight: bold text is drawn with a stroke
/// (see `Attributes::heavy`).
pub fn font(size: f64) -> Retained<NSFont> {
    named(PIXEL_FONT, size).unwrap_or_else(|| {
        NSFont::userFixedPitchFontOfSize(size).unwrap_or_else(|| NSFont::systemFontOfSize(size))
    })
}

fn rgb(hex: u32) -> Retained<NSColor> {
    rgba(hex, 1.0)
}

fn rgba(hex: u32, alpha: f64) -> Retained<NSColor> {
    let channel = |shift: u32| f64::from((hex >> shift) & 0xff) / 255.0;
    NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), alpha)
}

/// Outline and main text: the cat's dark outline.
pub fn ink() -> Retained<NSColor> {
    rgb(0x2a2230)
}

pub fn muted() -> Retained<NSColor> {
    rgb(0x806e74)
}

pub fn faint() -> Retained<NSColor> {
    rgb(0xb3a3a6)
}

/// Bubble fill: the cat's white chest.
pub fn paper() -> Retained<NSColor> {
    rgb(0xfff6e9)
}

pub fn sidebar() -> Retained<NSColor> {
    rgb(0xf8e6cb)
}

pub fn divider() -> Retained<NSColor> {
    rgb(0xe9cfa6)
}

/// The cat's orange fur.
pub fn accent() -> Retained<NSColor> {
    rgb(0xc8742a)
}

pub fn link() -> Retained<NSColor> {
    rgb(0x2f6db3)
}

pub fn error() -> Retained<NSColor> {
    rgb(0xc0392b)
}

/// Emphasis: the pixel font has no italic, so emphasized text is tinted.
pub fn emphasis() -> Retained<NSColor> {
    rgb(0x8a4f1d)
}

pub fn code_background() -> Retained<NSColor> {
    rgb(0xf3e3cc)
}

pub fn field_background() -> Retained<NSColor> {
    rgb(0xfffdf8)
}

pub fn shadow() -> Retained<NSColor> {
    rgba(0x2a2230, 0.25)
}
