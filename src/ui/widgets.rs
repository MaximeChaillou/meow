//! Controls in the pixel style: text fields and bracketed text buttons.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2_app_kit::{NSButton, NSTextField, NSTextFieldBezelStyle};
use objc2_foundation::NSString;

use super::rich_text::{self, Attributes};
use super::theme;

/// An input field with the pixel font on a light background.
pub fn text_field(mtm: MainThreadMarker, placeholder: &str) -> Retained<NSTextField> {
    let field = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
    field.setFont(Some(&theme::font(theme::BODY_SIZE)));
    field.setTextColor(Some(&theme::ink()));
    field.setBackgroundColor(Some(&theme::field_background()));
    field.setDrawsBackground(true);
    field.setBezeled(true);
    field.setBezelStyle(NSTextFieldBezelStyle::SquareBezel);
    field.setPlaceholderAttributedString(Some(&rich_text::attributed(
        placeholder,
        &Attributes::plain(theme::faint(), theme::font(theme::BODY_SIZE)),
    )));
    field
}

/// A borderless button reading `[ LABEL ]` in the accent color.
pub fn text_button(
    mtm: MainThreadMarker,
    label: &str,
    target: &AnyObject,
    action: Sel,
) -> Retained<NSButton> {
    // SAFETY: callers pass the app-lifetime controller and one of its actions.
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(""),
            Some(target),
            Some(action),
            mtm,
        )
    };
    button.setBordered(false);
    set_text_button_label(&button, label);
    button
}

/// Changes the label of a [`text_button`].
pub fn set_text_button_label(button: &NSButton, label: &str) {
    button.setAttributedTitle(&rich_text::attributed(
        &format!("[ {} ]", label.to_uppercase()),
        &Attributes {
            heavy: true,
            ..Attributes::plain(theme::accent(), theme::font(theme::BODY_SIZE))
        },
    ));
}
