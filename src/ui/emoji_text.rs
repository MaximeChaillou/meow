//! Pictures in the Slack transcript: `:name:` left in a message becomes the
//! workspace's custom emoji, drawn inline at text size (or the standard emoji
//! a custom name is an alias of), and an attached image's marker becomes the
//! image itself, on its own line. Animated ones are handed back with their
//! attachment, for the page to play.

use std::collections::HashMap;
use std::rc::Rc;

use meow::slack::emoji;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAttributedStringAttachmentConveniences, NSFont, NSFontAttributeName, NSImage,
    NSTextAttachment,
};
use objc2_foundation::{
    NSAttributedString, NSMutableAttributedString, NSPoint, NSRange, NSRect, NSSize, NSString,
};

use super::picture::Picture;
use super::theme;

/// How tall an emoji image is next to text of the font's size.
const IMAGE_SCALE: f64 = 1.25;
/// Attached images are scaled down to fit this box.
const MAX_IMAGE_WIDTH: f64 = 320.0;
const MAX_IMAGE_HEIGHT: f64 = 240.0;
/// Around an attached image's file id in a message body.
const MARK_OPEN: &str = "⟦image:";
const MARK_CLOSE: &str = "⟧";

/// What a custom emoji name is drawn as.
pub enum EmojiGlyph {
    Image(Rc<Picture>),
    /// An alias of a standard emoji.
    Text(String),
}

/// An image attached to a message: drawn once downloaded, named until then.
pub struct ImageGlyph {
    pub image: Option<Rc<Picture>>,
    pub title: String,
    /// The file in Slack; clicking the image opens it.
    pub link: Option<String>,
}

/// The marker standing for attached image `id` in a message body.
pub fn image_marker(id: &str) -> String {
    format!("{MARK_OPEN}{id}{MARK_CLOSE}")
}

/// An attachment drawing an animated picture, whose frames it should cycle.
pub struct Animated {
    pub attachment: Retained<NSTextAttachment>,
    pub picture: Rc<Picture>,
}

/// Notes `attachment` in `animated` when `picture` has frames to play.
fn track(
    animated: &mut Vec<Animated>,
    attachment: &Retained<NSTextAttachment>,
    picture: &Rc<Picture>,
) {
    if picture.is_animated() {
        animated.push(Animated {
            attachment: attachment.clone(),
            picture: picture.clone(),
        });
    }
}

/// Swaps each image marker of `text` for its image, or its name while it
/// loads (or when unknown). Animated images are added to `animated`.
pub fn insert_images(
    text: &NSMutableAttributedString,
    images: &HashMap<String, ImageGlyph>,
    animated: &mut Vec<Animated>,
) {
    let string = text.string().to_string();
    let mut marks = Vec::new();
    let mut from = 0;
    while let Some(offset) = string[from..].find(MARK_OPEN) {
        let start = from + offset;
        let id_start = start + MARK_OPEN.len();
        let Some(id_len) = string[id_start..].find(MARK_CLOSE) else {
            break;
        };
        let end = id_start + id_len + MARK_CLOSE.len();
        marks.push((start, end, &string[id_start..id_start + id_len]));
        from = end;
    }
    for (start, end, id) in marks.into_iter().rev() {
        let location = utf16_len(&string[..start]);
        let range = NSRange::new(location, utf16_len(&string[start..end]));
        // SAFETY: `location` is inside the string, which is `text`'s own.
        let attributes =
            unsafe { text.attributesAtIndex_effectiveRange(location, std::ptr::null_mut()) };
        let glyph = images.get(id);
        let piece = match glyph.and_then(|g| g.image.as_ref()) {
            Some(picture) => {
                let attachment = fitted_attachment(picture.still());
                track(animated, &attachment, picture);
                NSMutableAttributedString::initWithAttributedString(
                    NSMutableAttributedString::alloc(),
                    &NSAttributedString::attributedStringWithAttachment(&attachment),
                )
            }
            None => {
                let title = glyph.map_or("image", |g| g.title.as_str());
                NSMutableAttributedString::initWithString(
                    NSMutableAttributedString::alloc(),
                    &NSString::from_str(&format!("🖼 {title}")),
                )
            }
        };
        let whole = NSRange::new(0, piece.length());
        // SAFETY: the attributes come from the same text, so their values fit their keys.
        unsafe { piece.addAttributes_range(&attributes, whole) };
        if let Some(link) = glyph.and_then(|g| g.link.as_deref())
            && let Some(url) = objc2_foundation::NSURL::URLWithString(&NSString::from_str(link))
        {
            // SAFETY: the link attribute takes an NSURL.
            unsafe {
                piece.addAttribute_value_range(objc2_app_kit::NSLinkAttributeName, &url, whole)
            };
        }
        text.replaceCharactersInRange_withAttributedString(range, &piece);
    }
}

/// `image` scaled down to fit the attached-image box, keeping its proportions.
fn fitted_attachment(image: &NSImage) -> Retained<NSTextAttachment> {
    let natural = image.size();
    let scale = if natural.width > 0.0 && natural.height > 0.0 {
        (MAX_IMAGE_WIDTH / natural.width)
            .min(MAX_IMAGE_HEIGHT / natural.height)
            .min(1.0)
    } else {
        1.0
    };
    let attachment = NSTextAttachment::new();
    attachment.setImage(Some(image));
    attachment.setBounds(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(natural.width * scale, natural.height * scale),
    ));
    attachment
}

/// Replaces each `:name:` of `text` found in `glyphs`, keeping the text's
/// look (font, paragraph indent) around it. Animated emoji are added to `animated`.
pub fn insert(
    text: &NSMutableAttributedString,
    glyphs: &HashMap<String, EmojiGlyph>,
    animated: &mut Vec<Animated>,
) {
    if glyphs.is_empty() {
        return;
    }
    let string = text.string().to_string();
    // From the end, so earlier ranges stay valid.
    for (start, end, name) in emoji::shortcodes(&string).into_iter().rev() {
        let Some(glyph) = glyphs.get(name) else {
            continue;
        };
        let location = utf16_len(&string[..start]);
        let range = NSRange::new(location, utf16_len(&string[start..end]));
        // SAFETY: `location` is inside the string, which is `text`'s own.
        let attributes =
            unsafe { text.attributesAtIndex_effectiveRange(location, std::ptr::null_mut()) };
        let replacement = match glyph {
            // SAFETY: the attributes come from the same text, so their values fit their keys.
            EmojiGlyph::Text(emoji) => unsafe {
                NSAttributedString::initWithString_attributes(
                    NSAttributedString::alloc(),
                    &NSString::from_str(emoji),
                    Some(&attributes),
                )
            },
            EmojiGlyph::Image(picture) => {
                // SAFETY: the font attribute, when set, holds an NSFont.
                let font = unsafe { attributes.objectForKey(NSFontAttributeName) }
                    .and_then(|font| font.downcast::<NSFont>().ok());
                let size = font.as_ref().map_or(theme::BODY_SIZE, |f| f.pointSize());
                let descender = font.as_ref().map_or(-3.0, |f| f.descender());
                let attachment = image_attachment(picture.still(), size * IMAGE_SCALE, descender);
                track(animated, &attachment, picture);
                let piece = NSMutableAttributedString::initWithAttributedString(
                    NSMutableAttributedString::alloc(),
                    &NSAttributedString::attributedStringWithAttachment(&attachment),
                );
                // SAFETY: the attributes come from the same text, so their values fit their keys.
                unsafe { piece.addAttributes_range(&attributes, NSRange::new(0, piece.length())) };
                piece.into_super()
            }
        };
        text.replaceCharactersInRange_withAttributedString(range, &replacement);
    }
}

/// `image` scaled to `height`, keeping its proportions, sitting on the baseline.
fn image_attachment(image: &NSImage, height: f64, descender: f64) -> Retained<NSTextAttachment> {
    let natural = image.size();
    let width = if natural.height > 0.0 {
        height * natural.width / natural.height
    } else {
        height
    };
    let attachment = NSTextAttachment::new();
    attachment.setImage(Some(image));
    attachment.setBounds(NSRect::new(
        NSPoint::new(0.0, descender),
        NSSize::new(width, height),
    ));
    attachment
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}
