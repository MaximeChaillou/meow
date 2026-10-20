//! The Activity page: what happened in the background, newest first. Each
//! item has its kind, when it happened, a link to the thing itself and a ×
//! to dismiss it; Clear all empties the list.

use std::cell::RefCell;

use meow::activity::{Activity, ActivityLog, Kind};
use meow::clock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSColor, NSScrollView, NSTextField, NSView};
use objc2_foundation::{
    NSAttributedString, NSMutableAttributedString, NSPoint, NSRect, NSSize, NSString,
};

use super::bubble;
use super::controller::Controller;
use super::flipped::FlippedView;
use super::link_button::LinkButton;
use super::rich_text::{self, Attributes};
use super::theme;
use super::widgets;

const MARGIN: f64 = 16.0;
const BAR_HEIGHT: f64 = 26.0;
const CLEAR_WIDTH: f64 = 100.0;
const ROW_HEIGHT: f64 = 22.0;
const DETAIL_HEIGHT: f64 = 18.0;
const CARD_PADDING: f64 = 8.0;
const CARD_GAP: f64 = 6.0;
const TAG_WIDTH: f64 = 92.0;
const TIME_WIDTH: f64 = 110.0;
const DISMISS_WIDTH: f64 = 22.0;
const INDENT: f64 = 6.0;
const NEW_BORDER: f64 = 2.0;

pub struct ActivityPage {
    pub view: Retained<NSView>,
    scroll: Retained<NSScrollView>,
    list: Retained<FlippedView>,
    target: Retained<Controller>,
    /// Item ids in the order shown; row controls carry an index into it.
    shown_ids: RefCell<Vec<u64>>,
    mtm: MainThreadMarker,
}

impl ActivityPage {
    pub fn new(mtm: MainThreadMarker, controller: &Retained<Controller>, frame: NSRect) -> Self {
        let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let (width, height) = (frame.size.width - bubble::PADDING, frame.size.height);
        let target: &AnyObject = controller;

        // The page view is not flipped: the bar sits at the top, y from the bottom.
        let clear = widgets::text_button(
            mtm,
            "Clear all",
            widgets::ButtonStyle::Quiet,
            target,
            sel!(clearActivity:),
        );
        clear.setFrame(rect(
            width - MARGIN - CLEAR_WIDTH,
            height - MARGIN / 2.0 - BAR_HEIGHT,
            CLEAR_WIDTH,
            BAR_HEIGHT,
        ));
        clear.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        view.addSubview(&clear);

        let top = MARGIN / 2.0 + BAR_HEIGHT + CARD_GAP;
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            rect(MARGIN, MARGIN, width - 2.0 * MARGIN, height - top - MARGIN),
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
            target: controller.clone(),
            shown_ids: RefCell::new(Vec::new()),
            mtm,
        }
    }

    /// The id of the item in row `index`, as last rendered.
    pub fn id_at(&self, index: usize) -> Option<u64> {
        self.shown_ids.borrow().get(index).copied()
    }

    /// Draws the log; unread items get an orange outline.
    pub fn render(&self, log: &ActivityLog, now: u64) {
        let width = self.scroll.contentSize().width;
        for row in self.list.subviews().iter() {
            row.removeFromSuperview();
        }
        let mut ids = Vec::new();
        let mut y = 0.0;
        if log.items().is_empty() {
            self.add_label(
                &rich_text::attributed(
                    "Nothing new. Slack messages for you, calendar changes and tasks show up here.",
                    &Attributes::plain(theme::muted(), theme::font(theme::BODY_SIZE)),
                ),
                rect(INDENT, 4.0, width - 2.0 * INDENT, ROW_HEIGHT),
            );
            y = ROW_HEIGHT;
        }
        for item in log.items() {
            y = self.add_item(ids.len(), item, width, y, now) + CARD_GAP;
            ids.push(item.id);
        }
        let height = y.max(self.scroll.contentSize().height);
        self.list.setFrameSize(NSSize::new(width, height));
        self.shown_ids.replace(ids);
    }

    /// A card: kind tag, title (opens it), time, ×; the detail below. Returns the y below.
    fn add_item(&self, index: usize, item: &Activity, width: f64, y: f64, now: u64) -> f64 {
        let target: &AnyObject = &self.target;
        let detail = if item.detail.is_some() {
            DETAIL_HEIGHT
        } else {
            0.0
        };
        let height = ROW_HEIGHT + detail + 2.0 * CARD_PADDING;
        let card = FlippedView::new(self.mtm, rect(0.0, y, width, height));
        card.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        let border = (!item.read).then(theme::accent);
        widgets::paint(
            &card,
            Some(&theme::card()),
            border.as_deref().map(|color| (color, NEW_BORDER)),
        );
        self.list.addSubview(&card);
        let top = y + CARD_PADDING;

        let tag = Attributes::plain(kind_color(item.kind), theme::label_font(theme::LABEL_SIZE));
        self.add_label(
            &rich_text::attributed(item.kind.label(), &tag),
            rect(2.0 * INDENT, top + 3.0, TAG_WIDTH, ROW_HEIGHT - 3.0),
        );

        let dismiss_x = width - INDENT - DISMISS_WIDTH;
        let time_x = dismiss_x - TIME_WIDTH;
        let time = Attributes::plain(theme::faint(), theme::font(theme::SMALL_SIZE));
        let when = self.add_label(
            &rich_text::attributed(&clock::when_label(item.at, now), &time),
            rect(time_x, top + 3.0, TIME_WIDTH, ROW_HEIGHT - 3.0),
        );
        when.setAlignment(objc2_app_kit::NSTextAlignment::Right);
        when.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);

        let dismiss = widgets::text_button(
            self.mtm,
            "×",
            widgets::ButtonStyle::Link,
            target,
            sel!(dismissActivity:),
        );
        dismiss.setTag(index as isize);
        dismiss.setToolTip(Some(&NSString::from_str("Dismiss")));
        dismiss.setFrame(rect(dismiss_x, top, DISMISS_WIDTH, ROW_HEIGHT));
        dismiss.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        self.list.addSubview(&dismiss);

        let x = 2.0 * INDENT + TAG_WIDTH;
        let frame = rect(x, top, time_x - x - INDENT, ROW_HEIGHT);
        if item.link.is_some() {
            let title = LinkButton::new(
                self.mtm,
                frame,
                item_title(item, false),
                item_title(item, true),
                target,
                sel!(openActivity:),
            );
            title.setTag(index as isize);
            title.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            self.list.addSubview(&title);
        } else {
            self.add_label(&item_title(item, false), frame);
        }
        if let Some(text) = &item.detail {
            let style = Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE));
            let detail = self.add_label(
                &rich_text::attributed(text, &style),
                rect(x, top + ROW_HEIGHT - 2.0, dismiss_x - x, DETAIL_HEIGHT),
            );
            detail.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
        }
        y + height
    }

    fn add_label(&self, text: &NSAttributedString, frame: NSRect) -> Retained<NSTextField> {
        let label = NSTextField::labelWithAttributedString(text, self.mtm);
        label.setFrame(frame);
        label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.list.addSubview(&label);
        label
    }
}

fn kind_color(kind: Kind) -> Retained<NSColor> {
    match kind {
        Kind::Slack => theme::link(),
        Kind::Calendar => theme::live(),
        Kind::Tasks => theme::highlight(),
        Kind::Scheduled => theme::accent(),
    }
}

/// The title, bold while unread; hovering underlines it.
fn item_title(item: &Activity, hover: bool) -> Retained<NSAttributedString> {
    let attributes = Attributes {
        heavy: !item.read,
        underline: hover,
        ..Attributes::plain(
            if hover { theme::accent() } else { theme::ink() },
            theme::font(theme::BODY_SIZE),
        )
    };
    let title = NSMutableAttributedString::new();
    title.appendAttributedString(&rich_text::attributed(&item.title, &attributes));
    title.into_super()
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
