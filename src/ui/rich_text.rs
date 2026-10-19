//! Builds attributed strings for the transcript, including rendered Markdown.

use meow::markdown::{self, Span};
use meow::transcript::Block;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread, Message};
use objc2_app_kit::{
    NSBackgroundColorAttributeName, NSColor, NSCursor, NSCursorAttributeName, NSFont,
    NSFontAttributeName, NSForegroundColorAttributeName, NSLinkAttributeName,
    NSStrikethroughStyleAttributeName, NSStrokeWidthAttributeName, NSTextView,
    NSUnderlineStyleAttributeName,
};
use objc2_foundation::{
    NSAttributedString, NSAttributedStringKey, NSDictionary, NSMutableAttributedString, NSNumber,
    NSString, NSURL,
};

use super::theme;

pub const BODY_SIZE: f64 = theme::BODY_SIZE;
const HEADING_SIZES: [f64; 3] = [24.0, 21.0, 19.0];
/// Stroke that thickens the single-weight pixel font into bold (negative: fill and stroke).
const HEAVY_STROKE: f64 = -3.0;

/// How one run of text looks.
pub struct Attributes {
    pub color: Retained<NSColor>,
    pub font: Retained<NSFont>,
    pub background: Option<Retained<NSColor>>,
    pub link: Option<Retained<NSURL>>,
    pub strike: bool,
    /// Bold, faked with a stroke since the pixel font has one weight.
    pub heavy: bool,
    pub underline: bool,
}

impl Attributes {
    pub fn plain(color: Retained<NSColor>, font: Retained<NSFont>) -> Self {
        Self {
            color,
            font,
            background: None,
            link: None,
            strike: false,
            heavy: false,
            underline: false,
        }
    }
}

pub fn attributed(text: &str, attributes: &Attributes) -> Retained<NSAttributedString> {
    let single_line = NSNumber::numberWithInteger(1);
    let stroke = NSNumber::numberWithDouble(HEAVY_STROKE);
    // SAFETY: AppKit's attribute-name constants are immutable statics.
    let mut keys: Vec<&NSAttributedStringKey> =
        unsafe { vec![NSForegroundColorAttributeName, NSFontAttributeName] };
    let mut values: Vec<&AnyObject> = vec![&attributes.color, &attributes.font];
    if let Some(background) = &attributes.background {
        keys.push(unsafe { NSBackgroundColorAttributeName });
        values.push(background);
    }
    if let Some(link) = &attributes.link {
        keys.push(unsafe { NSLinkAttributeName });
        values.push(link);
    }
    if attributes.strike {
        keys.push(unsafe { NSStrikethroughStyleAttributeName });
        values.push(&single_line);
    }
    if attributes.underline {
        keys.push(unsafe { NSUnderlineStyleAttributeName });
        values.push(&single_line);
    }
    if attributes.heavy {
        keys.push(unsafe { NSStrokeWidthAttributeName });
        values.push(&stroke);
    }
    let dictionary = NSDictionary::from_slices(&keys, &values);
    // SAFETY: every value has the type its attribute key expects.
    unsafe {
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &NSString::from_str(text),
            Some(&dictionary),
        )
    }
}

/// URL scheme of the clickable "N steps" line; the path is the turn's index.
const STEPS_SCHEME: &str = "meow-steps:";

/// The turn index behind a steps-line link, if `url` is one.
pub fn steps_turn(url: &str) -> Option<usize> {
    url.strip_prefix(STEPS_SCHEME)?.parse().ok()
}

/// Links keep their own colors; hovering any link shows the hand cursor.
pub fn style_links(view: &NSTextView) {
    let cursor = NSCursor::pointingHandCursor();
    // SAFETY: AppKit's attribute-name constant is an immutable static.
    let keys = [unsafe { NSCursorAttributeName }];
    let values: [&AnyObject; 1] = [&cursor];
    // SAFETY: the cursor attribute holds an NSCursor, as AppKit expects.
    unsafe { view.setLinkTextAttributes(Some(&NSDictionary::from_slices(&keys, &values))) };
}

/// The whole conversation as styled text.
pub fn transcript(blocks: &[Block]) -> Retained<NSAttributedString> {
    let result = NSMutableAttributedString::new();
    for block in blocks {
        result.appendAttributedString(&block_text(block));
    }
    result.into_super()
}

fn block_text(block: &Block) -> Retained<NSAttributedString> {
    let line = |text: String, color: Retained<NSColor>, font: Retained<NSFont>| {
        attributed(&text, &Attributes::plain(color, font))
    };
    let small = || theme::font(theme::SMALL_SIZE);
    match block {
        Block::User(text) => attributed(
            &format!("\n> {text}\n"),
            &Attributes {
                heavy: true,
                ..Attributes::plain(theme::accent(), theme::font(BODY_SIZE))
            },
        ),
        Block::Info(text) => line(format!("{text}\n"), theme::muted(), small()),
        Block::Error(text) => line(
            format!("! {text}\n"),
            theme::error(),
            theme::font(BODY_SIZE),
        ),
        Block::Tool(text) => line(format!("    - {text}\n"), theme::muted(), small()),
        Block::Steps {
            turn,
            label,
            expanded,
        } => {
            let arrow = if *expanded { "▾" } else { "▸" };
            let url = NSURL::URLWithString(&NSString::from_str(&format!("{STEPS_SCHEME}{turn}")));
            attributed(
                &format!("{arrow} {label}\n"),
                &Attributes {
                    link: url,
                    ..Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE))
                },
            )
        }
        Block::Note(text) => markdown(text, theme::muted()),
        Block::Answer(text) => markdown(text, theme::ink()),
    }
}

/// Height of the blank line that separates runs of Slack messages.
const SLACK_GAP_SIZE: f64 = 8.0;

/// A Slack day divider: "── Today ──", set apart from the runs around it.
pub fn slack_day(label: &str) -> Retained<NSAttributedString> {
    let result = NSMutableAttributedString::new();
    result.appendAttributedString(&slack_gap());
    result.appendAttributedString(&attributed(
        &format!("── {label} ──\n"),
        &Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE)),
    ));
    result.into_super()
}

/// A run of Slack messages by one author: the name in bold (in the link color
/// when it is our own) and the time once, then each body rendered from Markdown.
pub fn slack_run(
    author: &str,
    time: &str,
    mine: bool,
    bodies: &[String],
) -> Retained<NSAttributedString> {
    let result = NSMutableAttributedString::new();
    result.appendAttributedString(&slack_gap());
    let name_color = if mine { theme::link() } else { theme::accent() };
    let name = Attributes {
        heavy: true,
        ..Attributes::plain(name_color, theme::font(theme::SMALL_SIZE))
    };
    result.appendAttributedString(&attributed(author, &name));
    result.appendAttributedString(&attributed(
        &format!("  {time}\n"),
        &Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE)),
    ));
    for body in bodies {
        result.appendAttributedString(&markdown(body, theme::ink()));
    }
    result.into_super()
}

/// Text in the private Claude panel: Markdown in the main ink color.
pub fn slack_assist_body(text: &str) -> Retained<NSAttributedString> {
    markdown(text, theme::ink())
}

fn slack_gap() -> Retained<NSAttributedString> {
    attributed(
        "\n",
        &Attributes::plain(theme::ink(), theme::font(SLACK_GAP_SIZE)),
    )
}

/// Markdown rendered for the transcript in `color`, ending with a newline.
fn markdown(text: &str, color: Retained<NSColor>) -> Retained<NSAttributedString> {
    let result = NSMutableAttributedString::new();
    for span in markdown::to_spans(text) {
        let attributes = span_attributes(&span, &color);
        result.appendAttributedString(&attributed(&span.text, &attributes));
    }
    let newline = Attributes::plain(color, theme::font(BODY_SIZE));
    result.appendAttributedString(&attributed("\n", &newline));
    result.into_super()
}

fn span_attributes(span: &Span, color: &NSColor) -> Attributes {
    let style = span.style;
    let size = match style.heading {
        0 => BODY_SIZE,
        level => HEADING_SIZES
            .get(usize::from(level) - 1)
            .copied()
            .unwrap_or(BODY_SIZE),
    };
    let font = theme::font(size);
    let link = span
        .link
        .as_deref()
        .and_then(|url| NSURL::URLWithString(&NSString::from_str(url)));
    Attributes {
        color: if link.is_some() {
            theme::link()
        } else if style.italic {
            theme::emphasis()
        } else {
            color.retain()
        },
        font,
        background: style.code.then(theme::code_background),
        link,
        strike: style.strike,
        heavy: style.bold || style.heading > 0,
        underline: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_links_round_trip_their_turn() {
        assert_eq!(steps_turn(&format!("{STEPS_SCHEME}7")), Some(7));
        assert_eq!(steps_turn("https://linear.app/x"), None);
        assert_eq!(steps_turn("meow-steps:abc"), None);
    }
}
