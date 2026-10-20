//! The pixel-art look: a dark 8-bit palette (Sweetie 16) and the bundled
//! pixel fonts.
//!
//! The fonts are VT323 (body), Silkscreen (labels) and Press Start 2P
//! (titles), all under the SIL Open Font License (see assets/fonts), embedded
//! in the binary.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSFont};
use objc2_core_foundation::{CFData, CFRetained};
use objc2_core_text::{CTFont, CTFontDescriptor, CTFontManagerCreateFontDescriptorFromData};

const BODY_FONT: &str = "VT323";
const LABEL_FONT: &str = "Silkscreen";
const TITLE_FONT: &str = "PressStart2P";

const FONTS: [(&str, &[u8]); 3] = [
    (
        BODY_FONT,
        include_bytes!("../../assets/fonts/VT323-400.ttf"),
    ),
    (
        LABEL_FONT,
        include_bytes!("../../assets/fonts/Silkscreen-Regular.ttf"),
    ),
    (
        TITLE_FONT,
        include_bytes!("../../assets/fonts/PressStart2P-Regular.ttf"),
    ),
];

/// VT323 is drawn small for its point size, so sizes run larger than usual.
pub const BODY_SIZE: f64 = 18.0;
pub const SMALL_SIZE: f64 = 16.0;
/// Silkscreen: page names, section headings, button labels.
pub const LABEL_SIZE: f64 = 12.0;
/// Press Start 2P is wide and tall for its size.
pub const TITLE_SIZE: f64 = 12.0;

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
            "Could not load the pixel fonts ({}).",
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

fn named_or_fallback(name: &str, size: f64) -> Retained<NSFont> {
    named(name, size).unwrap_or_else(|| {
        NSFont::userFixedPitchFontOfSize(size).unwrap_or_else(|| NSFont::systemFontOfSize(size))
    })
}

/// The body font. Bold text is drawn with a stroke (see `Attributes::heavy`).
pub fn font(size: f64) -> Retained<NSFont> {
    named_or_fallback(BODY_FONT, size)
}

/// The label font: blocky capitals for page names, headings and buttons.
pub fn label_font(size: f64) -> Retained<NSFont> {
    named_or_fallback(LABEL_FONT, size)
}

/// The title font, for the MEOW wordmark and page titles.
pub fn title_font(size: f64) -> Retained<NSFont> {
    named_or_fallback(TITLE_FONT, size)
}

fn rgb(hex: u32) -> Retained<NSColor> {
    let channel = |shift: u32| f64::from((hex >> shift) & 0xff) / 255.0;
    NSColor::colorWithSRGBRed_green_blue_alpha(channel(16), channel(8), channel(0), 1.0)
}

/// Main text: off-white on the dark panels.
pub fn ink() -> Retained<NSColor> {
    rgb(0xf4f4f4)
}

/// Text drawn on the light fills (selected page, buttons).
pub fn on_accent() -> Retained<NSColor> {
    rgb(0x1a1c2c)
}

pub fn muted() -> Retained<NSColor> {
    rgb(0x94b0c2)
}

pub fn faint() -> Retained<NSColor> {
    rgb(0x7c8ba3)
}

/// The bubble's pixel outline.
pub fn outline() -> Retained<NSColor> {
    rgb(0xf4f4f4)
}

/// Bubble fill.
pub fn paper() -> Retained<NSColor> {
    rgb(0x333c57)
}

pub fn sidebar() -> Retained<NSColor> {
    rgb(0x1a1c2c)
}

pub fn divider() -> Retained<NSColor> {
    rgb(0xf4f4f4)
}

/// The cat's orange fur.
pub fn accent() -> Retained<NSColor> {
    rgb(0xef7d57)
}

/// Gold: page titles, section headings and the main buttons.
pub fn highlight() -> Retained<NSColor> {
    rgb(0xffcd75)
}

/// The darker line under a gold button.
pub fn primary_shade() -> Retained<NSColor> {
    rgb(0xb13e53)
}

/// The darker line under a green button.
pub fn live_shade() -> Retained<NSColor> {
    rgb(0x38b764)
}

/// Background of the user's own messages.
pub fn user_bubble() -> Retained<NSColor> {
    rgb(0x3b5dc9)
}

/// Cyan: one of the Slack avatar colors.
pub fn cyan() -> Retained<NSColor> {
    rgb(0x73eff7)
}

pub fn link() -> Retained<NSColor> {
    rgb(0x41a6f6)
}

pub fn error() -> Retained<NSColor> {
    rgb(0xff7b7b)
}

/// Emphasis: italic text is tinted.
pub fn emphasis() -> Retained<NSColor> {
    rgb(0xffcd75)
}

pub fn code_background() -> Retained<NSColor> {
    rgb(0x1a1c2c)
}

pub fn field_background() -> Retained<NSColor> {
    rgb(0x262b44)
}

/// Background of a list card (a task, an agenda item).
pub fn card() -> Retained<NSColor> {
    rgb(0x262b44)
}

/// A done card: lighter, so it recedes from the open ones.
pub fn card_done() -> Retained<NSColor> {
    rgb(0x3e4866)
}

/// Pixel border of fields and secondary buttons.
pub fn field_border() -> Retained<NSColor> {
    rgb(0x566c86)
}

/// Green: live status, send buttons, ticked checkboxes.
pub fn live() -> Retained<NSColor> {
    rgb(0xa7f070)
}

/// The bubble's hard drop shadow.
pub fn shadow() -> Retained<NSColor> {
    rgb(0x0b0c16)
}
