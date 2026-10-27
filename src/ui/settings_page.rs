//! The Settings page: every choice the user keeps, one section per part of
//! the app. The features describe their sections (see `Feature::settings`),
//! like their menu items, and this page only draws them: each control runs a
//! closure that sends to its feature's mailbox.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSEvent, NSScrollView, NSTextField, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::action::Action;
use super::bubble;
use super::controller::Controller;
use super::flipped::FlippedView;
use super::rich_text::{self, Attributes};
use super::theme;
use super::widgets;

const MARGIN: f64 = 16.0;
const INDENT: f64 = 10.0;
const ROW_HEIGHT: f64 = 26.0;
const LINE_GAP: f64 = 6.0;
const ENTRY_GAP: f64 = 12.0;
const SECTION_GAP: f64 = 18.0;
const CARD_PADDING: f64 = 10.0;
const HEADING_HEIGHT: f64 = 20.0;
const CHECKBOX_WIDTH: f64 = 26.0;
/// Room around a button's label.
const BUTTON_PADDING: f64 = 24.0;
const BUTTON_GAP: f64 = 6.0;
/// A text entry shows about this many lines before its text scrolls.
const TEXT_HEIGHT: f64 = 110.0;
const TEXT_TIP: &str = "Return saves, as does leaving the field · ⇧ Return: new line";

/// A part of the Settings page, under its title.
pub struct SettingsSection {
    pub title: &'static str,
    pub entries: Vec<SettingEntry>,
}

/// One control of a section. Each closure should only send to its feature's
/// mailbox; the feature then saves, and asks for the page again.
pub enum SettingEntry {
    /// A line of explanation or status.
    Note(String),
    /// On or off: `run` is called to flip it.
    Toggle {
        title: String,
        on: bool,
        run: Box<dyn Fn()>,
    },
    /// One of `options`, the `selected` one highlighted: `pick` gets the index chosen.
    Choice {
        title: String,
        options: Vec<String>,
        selected: Option<usize>,
        pick: Box<dyn Fn(usize)>,
    },
    /// Text the user writes, `hint` showing while empty: `save` gets it on
    /// Return, or when the field is left.
    Text {
        title: String,
        hint: String,
        value: String,
        save: Box<dyn Fn(String)>,
    },
    Button {
        title: String,
        run: Box<dyn Fn()>,
    },
}

pub struct SettingsPage {
    pub view: Retained<NSView>,
    scroll: Retained<NSScrollView>,
    list: Retained<FlippedView>,
    /// The text fields' delegate, for Esc and ⇧ Return.
    controller: Retained<Controller>,
    /// The targets of the controls drawn, which hold them only weakly.
    kept: RefCell<Vec<Retained<Action>>>,
    mtm: MainThreadMarker,
}

impl SettingsPage {
    pub fn new(mtm: MainThreadMarker, controller: &Retained<Controller>, frame: NSRect) -> Self {
        let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let (width, height) = (frame.size.width - bubble::PADDING, frame.size.height);
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            rect(MARGIN, MARGIN, width - 2.0 * MARGIN, height - 1.5 * MARGIN),
        );
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);
        scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let list = FlippedView::new(mtm, rect(0.0, 0.0, scroll.contentSize().width, 0.0));
        list.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        scroll.setDocumentView(Some(&list));
        view.addSubview(&scroll);
        Self {
            view,
            scroll,
            list,
            controller: controller.clone(),
            kept: RefCell::new(Vec::new()),
            mtm,
        }
    }

    /// Whether the user is typing in one of the page's fields, or holding a
    /// mouse button down: drawing it again now would take the field away from
    /// under them, or swallow the click.
    pub fn in_use(&self) -> bool {
        if self.view.isHiddenOrHasHiddenAncestor() {
            return false;
        }
        if NSEvent::pressedMouseButtons() != 0 {
            return true;
        }
        let Some(responder) = self.view.window().and_then(|w| w.firstResponder()) else {
            return false;
        };
        // While a field is edited, its field editor (a text view inside it) has the focus.
        responder
            .downcast::<NSView>()
            .is_ok_and(|view| view.isDescendantOf(&self.list))
    }

    /// Draws `sections` in order, each under its title.
    pub fn render(&self, sections: Vec<SettingsSection>) {
        for view in self.list.subviews().iter() {
            view.removeFromSuperview();
        }
        self.kept.borrow_mut().clear();
        let width = self.scroll.contentSize().width;
        let mut y = 0.0;
        for section in sections {
            y = self.add_section(section, width, y) + SECTION_GAP;
        }
        let height = y.max(self.scroll.contentSize().height);
        self.list.setFrameSize(NSSize::new(width, height));
    }

    /// The section's title, then its entries on a card. Returns the y below.
    fn add_section(&self, section: SettingsSection, width: f64, y: f64) -> f64 {
        let heading = NSTextField::labelWithString(&NSString::from_str(""), self.mtm);
        heading.setAttributedStringValue(&rich_text::attributed(
            &section.title.to_uppercase(),
            &Attributes::plain(theme::muted(), theme::label_font(theme::LABEL_SIZE)),
        ));
        heading.setFrame(rect(INDENT, y, width - 2.0 * INDENT, HEADING_HEIGHT));
        heading.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.list.addSubview(&heading);
        let card_top = y + HEADING_HEIGHT + LINE_GAP;

        let card = FlippedView::new(self.mtm, rect(0.0, card_top, width, 0.0));
        card.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.list.addSubview(&card);
        let inner = width - 2.0 * INDENT;
        let mut top = card_top + CARD_PADDING;
        for entry in section.entries {
            top = self.add_entry(entry, inner, top) + ENTRY_GAP;
        }
        let height = top - ENTRY_GAP + CARD_PADDING - card_top;
        card.setFrameSize(NSSize::new(width, height));
        widgets::paint(&card, Some(&theme::card()), None);
        card_top + height
    }

    /// One entry, `width` wide from the card's indent. Returns the y below.
    fn add_entry(&self, entry: SettingEntry, width: f64, y: f64) -> f64 {
        match entry {
            SettingEntry::Note(text) => {
                let muted = Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE));
                y + self.add_wrapped(&text, &muted, INDENT, y, width)
            }
            SettingEntry::Toggle { title, on, run } => {
                let target = self.keep(Action::new(self.mtm, run));
                let checkbox = widgets::checkbox(self.mtm, &target, Action::selector(), on);
                checkbox.setFrame(rect(INDENT, y, CHECKBOX_WIDTH, ROW_HEIGHT));
                self.list.addSubview(&checkbox);
                let x = INDENT + CHECKBOX_WIDTH + BUTTON_GAP;
                // The title is clickable too, like a checkbox's own label.
                let label = self.button(&title, widgets::ButtonStyle::Link, &target);
                label.setFrame(rect(x, y, width - x, ROW_HEIGHT));
                label.setAlignment(objc2_app_kit::NSTextAlignment::Left);
                set_plain_title(&label, &title);
                y + ROW_HEIGHT
            }
            SettingEntry::Choice {
                title,
                options,
                selected,
                pick,
            } => {
                let mut top = y + self.add_title(&title, width, y);
                let pick = Rc::new(pick);
                let mut x = INDENT;
                for (index, option) in options.iter().enumerate() {
                    let style = if selected == Some(index) {
                        widgets::ButtonStyle::Accent
                    } else {
                        widgets::ButtonStyle::Quiet
                    };
                    let pick = Rc::clone(&pick);
                    let target = self.keep(Action::new(self.mtm, move || pick(index)));
                    let button = self.button(option, style, &target);
                    let button_width = button.fittingSize().width + BUTTON_PADDING;
                    if x > INDENT && x + button_width > INDENT + width {
                        x = INDENT;
                        top += ROW_HEIGHT + BUTTON_GAP;
                    }
                    button.setFrame(rect(x, top, button_width, ROW_HEIGHT));
                    x += button_width + BUTTON_GAP;
                }
                top + ROW_HEIGHT
            }
            SettingEntry::Text {
                title,
                hint,
                value,
                save,
            } => self.add_text(&title, &hint, &value, save, width, y),
            SettingEntry::Button { title, run } => {
                let target = self.keep(Action::new(self.mtm, run));
                let button = self.button(&title, widgets::ButtonStyle::Quiet, &target);
                let button_width = button.fittingSize().width + BUTTON_PADDING;
                button.setFrame(rect(INDENT, y, button_width, ROW_HEIGHT));
                y + ROW_HEIGHT
            }
        }
    }

    /// A text entry: its title, the field, and how it saves, which says
    /// "Saved." once it did. Returns the y below.
    fn add_text(
        &self,
        title: &str,
        hint: &str,
        value: &str,
        save: Box<dyn Fn(String)>,
        width: f64,
        y: f64,
    ) -> f64 {
        let mut top = y + self.add_title(title, width, y);
        let field = widgets::text_area(self.mtm, hint);
        field.setStringValue(&NSString::from_str(value));
        field.setFrame(rect(INDENT, top, width, TEXT_HEIGHT));
        field.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        // Saved when the field is left too, not only on Return.
        if let Some(cell) = field.cell() {
            cell.setSendsActionOnEndEditing(true);
        }
        self.list.addSubview(&field);
        top += TEXT_HEIGHT + LINE_GAP / 2.0;

        let tip = NSTextField::labelWithString(&NSString::from_str(""), self.mtm);
        let faint = Attributes::plain(theme::faint(), theme::font(theme::SMALL_SIZE));
        tip.setAttributedStringValue(&rich_text::attributed(TEXT_TIP, &faint));
        tip.setFrame(rect(INDENT, top, width, ROW_HEIGHT - 4.0));
        tip.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.list.addSubview(&tip);

        let (typed, said) = (field.clone(), tip.clone());
        let target = self.keep(Action::new(self.mtm, move || {
            save(typed.stringValue().to_string());
            let live = Attributes::plain(theme::live(), theme::font(theme::SMALL_SIZE));
            said.setAttributedStringValue(&rich_text::attributed("Saved.", &live));
        }));
        // SAFETY: the page keeps the target until it draws again, which
        // removes the field; the controller lives as long as the app.
        unsafe {
            field.setTarget(Some(&target));
            field.setAction(Some(Action::selector()));
            field.setDelegate(Some(ProtocolObject::from_ref(&*self.controller)));
        }
        top + ROW_HEIGHT - 4.0
    }

    /// An entry's title, in the body ink. Returns its height and the gap below.
    fn add_title(&self, title: &str, width: f64, y: f64) -> f64 {
        let style = Attributes {
            heavy: true,
            ..Attributes::plain(theme::ink(), theme::font(theme::SMALL_SIZE))
        };
        self.add_wrapped(title, &style, INDENT, y, width) + LINE_GAP
    }

    /// A button on the list, sending to `target`.
    fn button(
        &self,
        label: &str,
        style: widgets::ButtonStyle,
        target: &Action,
    ) -> Retained<NSButton> {
        let button = widgets::text_button(self.mtm, label, style, target, Action::selector());
        self.list.addSubview(&button);
        button
    }

    /// Keeps `target` until the page draws again: controls hold it weakly.
    fn keep(&self, target: Retained<Action>) -> Retained<Action> {
        self.kept.borrow_mut().push(target.clone());
        target
    }

    /// A label wrapped to `width`; returns its height.
    fn add_wrapped(&self, text: &str, style: &Attributes, x: f64, y: f64, width: f64) -> f64 {
        let label = NSTextField::wrappingLabelWithString(&NSString::from_str(""), self.mtm);
        label.setAttributedStringValue(&rich_text::attributed(text, style));
        label.setPreferredMaxLayoutWidth(width);
        let height = label
            .sizeThatFits(NSSize::new(width, f64::MAX))
            .height
            .ceil();
        label.setFrame(rect(x, y, width, height));
        label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.list.addSubview(&label);
        height
    }
}

/// A toggle's title as plain text rather than a button's capitals.
fn set_plain_title(button: &NSButton, title: &str) {
    button.setAttributedTitle(&rich_text::attributed(
        title,
        &Attributes::plain(theme::ink(), theme::font(theme::BODY_SIZE)),
    ));
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
