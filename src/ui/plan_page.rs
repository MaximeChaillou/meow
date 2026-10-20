//! The day plan page: the date and summary, today's agenda, then the focus,
//! replies and later items with checkboxes (they are tasks), and an input for
//! questions about the day.

use std::cell::RefCell;

use meow::clock;
use meow::plan::{DayPlan, Event, EventState, PlanItem, Rsvp};
use meow::tasks::{FoundTask, TaskList};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSScrollView, NSTextField, NSView, NSWindow,
};
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

/// Space between the plan and the page's edges.
const MARGIN: f64 = 16.0;
const ASK_WIDTH: f64 = 64.0;
const ROW_HEIGHT: f64 = 24.0;
const WHY_HEIGHT: f64 = 18.0;
const HEADER_HEIGHT: f64 = 30.0;
const SECTION_GAP: f64 = 10.0;
const TIME_WIDTH: f64 = 64.0;
const CHECKBOX_WIDTH: f64 = 22.0;
const INDENT: f64 = 6.0;
/// Agenda events and plan items each sit on a square card.
const CARD_PADDING: f64 = 6.0;
const CARD_GAP: f64 = 6.0;
const NOW_BORDER: f64 = 2.0;
/// The Accept and Decline buttons of an invitation.
const RSVP_WIDTH: f64 = 72.0;
const RSVP_GAP: f64 = 4.0;

pub struct PlanPage {
    pub view: Retained<NSView>,
    scroll: Retained<NSScrollView>,
    list: Retained<FlippedView>,
    input: Retained<widgets::GrowingField>,
    target: Retained<Controller>,
    /// Items in the order shown; their rows' controls carry an index into it.
    items: RefCell<Vec<FoundTask>>,
    /// Agenda links; event titles carry an index into it.
    links: RefCell<Vec<String>>,
    /// Agenda events in the order shown; their answer buttons carry an index into it.
    events: RefCell<Vec<Event>>,
    mtm: MainThreadMarker,
}

impl PlanPage {
    pub fn new(mtm: MainThreadMarker, controller: &Retained<Controller>, frame: NSRect) -> Self {
        let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        // The page runs under the bubble's right padding; content keeps clear of it.
        let (width, height) = (frame.size.width - bubble::PADDING, frame.size.height);
        let target: &AnyObject = controller;

        let input = widgets::text_field(mtm, "Ask about your day…");
        // SAFETY: the controller lives as long as the app, so the weak target stays valid.
        unsafe {
            input.setTarget(Some(target));
            input.setAction(Some(sel!(askPlan:)));
        }
        // SAFETY: the controller outlives the field, so the weak delegate stays valid.
        unsafe { input.setDelegate(Some(ProtocolObject::from_ref(&**controller))) };

        let ask = widgets::text_button(
            mtm,
            "Ask",
            widgets::ButtonStyle::Primary,
            target,
            sel!(askPlan:),
        );
        let top = widgets::composer(
            mtm,
            &view,
            &input,
            &[],
            &[(&ask, ASK_WIDTH, widgets::ButtonStyle::Primary)],
        ) + MARGIN;
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            rect(MARGIN, top, width - 2.0 * MARGIN, height - top - MARGIN),
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
        input.keep_clear(&scroll);

        Self {
            view,
            scroll,
            list,
            input,
            target: controller.clone(),
            items: RefCell::new(Vec::new()),
            links: RefCell::new(Vec::new()),
            events: RefCell::new(Vec::new()),
            mtm,
        }
    }

    pub fn focus(&self, window: &NSWindow) {
        window.makeFirstResponder(Some(&self.input));
    }

    pub fn take_input(&self) -> String {
        let text = self.input.stringValue().to_string();
        self.input.set_text("");
        text
    }

    /// The item in row `index`, as last rendered.
    pub fn item_at(&self, index: usize) -> Option<FoundTask> {
        self.items.borrow().get(index).cloned()
    }

    pub fn link_at(&self, index: usize) -> Option<String> {
        self.links.borrow().get(index).cloned()
    }

    pub fn event_at(&self, index: usize) -> Option<Event> {
        self.events.borrow().get(index).cloned()
    }

    /// Draws `plan`, made at `made_at`; item checkboxes follow `tasks`, and
    /// the events in `sending` show their answer on its way.
    pub fn render(
        &self,
        plan: &DayPlan,
        made_at: u64,
        tasks: &TaskList,
        sending: &[String],
        now: u64,
    ) {
        let width = self.scroll.contentSize().width;
        for view in self.list.subviews().iter() {
            view.removeFromSuperview();
        }
        let mut items = Vec::new();
        let mut links = Vec::new();
        let mut y = self.add_intro(plan, made_at, now, width);
        y = self.add_agenda(&plan.agenda, sending, now, width, y, &mut links);
        for (heading, section) in plan.sections() {
            if !section.is_empty() {
                y = self.add_section(heading, section, tasks, width, y, &mut items);
            }
        }
        let height = y.max(self.scroll.contentSize().height);
        self.list.setFrameSize(NSSize::new(width, height));
        self.items.replace(items);
        self.links.replace(links);
        self.events.replace(plan.agenda.clone());
    }

    /// "Monday 19 October" and the summary; returns the y below them.
    fn add_intro(&self, plan: &DayPlan, made_at: u64, now: u64, width: f64) -> f64 {
        let made = clock::local(made_at);
        let today = clock::local(now);
        let mut date = made.map(|t| t.date_label()).unwrap_or_default();
        let same_day = made
            .zip(today)
            .is_some_and(|(m, t)| (m.year, m.month, m.day) == (t.year, t.month, t.day));
        if !same_day {
            date.push_str(" · not today's plan");
        }
        let title = Attributes::plain(theme::highlight(), theme::title_font(theme::TITLE_SIZE));
        let text = rich_text::attributed(&date.to_uppercase(), &title);
        self.add_label(&text, rect(INDENT, 2.0, width - 2.0 * INDENT, ROW_HEIGHT));
        let mut y = ROW_HEIGHT + 2.0;
        if !plan.summary.is_empty() {
            let summary = rich_text::attributed(
                &plan.summary,
                &Attributes::plain(theme::ink(), theme::font(theme::BODY_SIZE)),
            );
            y += self.add_wrapped(&summary, INDENT, y, width - 2.0 * INDENT);
        }
        y + SECTION_GAP
    }

    fn add_agenda(
        &self,
        agenda: &[Event],
        sending: &[String],
        now: u64,
        width: f64,
        y: f64,
        links: &mut Vec<String>,
    ) -> f64 {
        let count = match agenda.len() {
            0 => "  nothing on the calendar".to_string(),
            1 => "  1 event".to_string(),
            n => format!("  {n} events"),
        };
        let mut y = self.add_header("Agenda", &count, width, y);
        let now_minutes = clock::local(now).map_or(0, |t| t.minutes_of_day());
        for (index, event) in agenda.iter().enumerate() {
            let state = event.state(now_minutes);
            let height = ROW_HEIGHT + 2.0 * CARD_PADDING;
            // The event under way gets an orange outline.
            let border = (state == EventState::Now).then(theme::accent);
            self.add_card(width, y, height, border.as_deref());
            let top = y + CARD_PADDING;
            let is_sending = event.id.as_ref().is_some_and(|id| sending.contains(id));
            let buttons = if state == EventState::Past {
                0.0
            } else {
                self.add_rsvp(index, event, is_sending, width, top)
            };
            self.add_event(event, state, width - buttons, top, links);
            y += height + CARD_GAP;
        }
        y + SECTION_GAP
    }

    /// "09:30  Standup", dimmed once over, highlighted while it runs.
    fn add_event(
        &self,
        event: &Event,
        state: EventState,
        width: f64,
        y: f64,
        links: &mut Vec<String>,
    ) {
        let target: &AnyObject = &self.target;
        let declined = event.response == Some(Rsvp::Declined);
        let color = match state {
            _ if declined => theme::faint(),
            EventState::Past => theme::faint(),
            EventState::Now => theme::accent(),
            EventState::AllDay | EventState::Upcoming => theme::ink(),
        };
        let time = event.start.as_deref().unwrap_or("all day");
        let time_style = Attributes {
            heavy: state == EventState::Now,
            ..Attributes::plain(color.clone(), theme::font(theme::BODY_SIZE))
        };
        self.add_label(
            &rich_text::attributed(time, &time_style),
            rect(2.0 * INDENT, y, TIME_WIDTH, ROW_HEIGHT),
        );
        let x = 2.0 * INDENT + TIME_WIDTH;
        let frame = rect(x, y, width - x - INDENT, ROW_HEIGHT);
        let title = |hover: bool| {
            let attributes = Attributes {
                heavy: state == EventState::Now,
                underline: hover,
                strike: declined && !hover,
                ..Attributes::plain(
                    if hover {
                        theme::accent()
                    } else {
                        color.clone()
                    },
                    theme::font(theme::BODY_SIZE),
                )
            };
            let text = NSMutableAttributedString::new();
            text.appendAttributedString(&rich_text::attributed(&event.title, &attributes));
            // "· 30 min · Salle Kodak": how long, and where.
            let details: Vec<String> = event
                .duration_label()
                .into_iter()
                .chain(event.location.clone())
                .collect();
            if !details.is_empty() {
                let dim = Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE));
                let line = format!("  · {}", details.join(" · "));
                text.appendAttributedString(&rich_text::attributed(&line, &dim));
            }
            if state == EventState::Now {
                let mark = Attributes::plain(theme::accent(), theme::font(theme::SMALL_SIZE));
                text.appendAttributedString(&rich_text::attributed("  ◀ now", &mark));
            }
            let answer = match event.response {
                Some(Rsvp::NeedsAction) => Some("  · not answered"),
                Some(Rsvp::Tentative) => Some("  · maybe"),
                Some(Rsvp::Accepted | Rsvp::Declined) | None => None,
            };
            if let Some(answer) = answer {
                let dim = Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE));
                text.appendAttributedString(&rich_text::attributed(answer, &dim));
            }
            text.into_super()
        };
        match &event.url {
            Some(url) => {
                let button = LinkButton::new(
                    self.mtm,
                    frame,
                    title(false),
                    title(true),
                    target,
                    sel!(openPlanLink:),
                );
                button.setTag(links.len() as isize);
                button.setToolTip(Some(&NSString::from_str(&format!("Open {url}"))));
                button.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
                self.list.addSubview(&button);
                links.push(url.clone());
            }
            None => self.add_label(&title(false), frame),
        }
    }

    /// Accept and Decline at the right of an invitation, minus the answer
    /// already given, or "sending…" while an answer is on its way. Returns
    /// the width they take.
    fn add_rsvp(&self, index: usize, event: &Event, sending: bool, width: f64, y: f64) -> f64 {
        if !event.answerable() {
            return 0.0;
        }
        let target: &AnyObject = &self.target;
        let mut right = width - INDENT;
        let pin = NSAutoresizingMaskOptions::ViewMinXMargin;
        if sending {
            let style = Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE));
            right -= RSVP_WIDTH;
            let label = NSTextField::labelWithAttributedString(
                &rich_text::attributed("sending…", &style),
                self.mtm,
            );
            label.setFrame(rect(right, y + 4.0, RSVP_WIDTH, ROW_HEIGHT - 4.0));
            label.setAutoresizingMask(pin);
            self.list.addSubview(&label);
            return width - right;
        }
        let buttons = [
            (Rsvp::Declined, "Decline", sel!(declinePlanEvent:)),
            (Rsvp::Accepted, "Accept", sel!(acceptPlanEvent:)),
        ];
        for (answer, label, action) in buttons {
            if event.response == Some(answer) {
                continue;
            }
            let style = match answer {
                Rsvp::Accepted => widgets::ButtonStyle::Go,
                _ => widgets::ButtonStyle::Quiet,
            };
            let button = widgets::text_button(self.mtm, label, style, target, action);
            right -= RSVP_WIDTH;
            button.setFrame(rect(right, y, RSVP_WIDTH, ROW_HEIGHT));
            button.setTag(index as isize);
            button.setAutoresizingMask(pin);
            self.list.addSubview(&button);
            right -= RSVP_GAP;
        }
        width - right
    }

    /// A heading and its items; focus items are numbered, with their reason below.
    fn add_section(
        &self,
        heading: &str,
        section: &[PlanItem],
        tasks: &TaskList,
        width: f64,
        y: f64,
        items: &mut Vec<FoundTask>,
    ) -> f64 {
        let done = |item: &PlanItem| tasks.find(&item.task()).is_some_and(|task| task.is_done());
        let open = section.iter().filter(|item| !done(item)).count();
        let count = match open {
            0 => "  all done".to_string(),
            n => format!("  {n} open"),
        };
        let numbered = heading == "Focus";
        let mut y = self.add_header(heading, &count, width, y);
        for (n, item) in section.iter().enumerate() {
            let number = numbered.then_some(n + 1);
            let why_height = if item.why.is_some() {
                WHY_HEIGHT - 2.0
            } else {
                0.0
            };
            let height = ROW_HEIGHT + why_height + 2.0 * CARD_PADDING;
            self.add_card(width, y, height, None);
            let top = y + CARD_PADDING;
            self.add_item(items.len(), item, number, done(item), width, top);
            items.push(item.task());
            if let Some(why) = &item.why {
                let style = Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE));
                let x = 2.0 * INDENT + CHECKBOX_WIDTH;
                self.add_label(
                    &rich_text::attributed(why, &style),
                    rect(x, top + ROW_HEIGHT - 4.0, width - x - INDENT, WHY_HEIGHT),
                );
            }
            y += height + CARD_GAP;
        }
        y + SECTION_GAP
    }

    /// A checkbox, then the title (opens the item) and its source.
    fn add_item(
        &self,
        index: usize,
        item: &PlanItem,
        number: Option<usize>,
        done: bool,
        width: f64,
        y: f64,
    ) {
        let target: &AnyObject = &self.target;
        // The controller is an app-lifetime target that implements `togglePlanItem:`.
        let checkbox = widgets::checkbox(self.mtm, target, sel!(togglePlanItem:), done);
        checkbox.setTag(index as isize);
        checkbox.setFrame(rect(2.0 * INDENT, y, CHECKBOX_WIDTH, ROW_HEIGHT));
        self.list.addSubview(&checkbox);

        let x = 2.0 * INDENT + CHECKBOX_WIDTH;
        let title = LinkButton::new(
            self.mtm,
            rect(x, y, width - x - INDENT, ROW_HEIGHT),
            item_title(item, number, done, false),
            item_title(item, number, done, true),
            target,
            sel!(openPlanItem:),
        );
        title.setTag(index as isize);
        let tip = match &item.url {
            Some(url) => format!("Open {url}"),
            None => "Chat about this task".to_string(),
        };
        title.setToolTip(Some(&NSString::from_str(&tip)));
        title.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.list.addSubview(&title);
    }

    /// "FOCUS  2 open": the heading in bold, the count dimmed. Returns the y below.
    fn add_header(&self, heading: &str, count: &str, width: f64, y: f64) -> f64 {
        let text = NSMutableAttributedString::new();
        let bold = Attributes::plain(
            theme::highlight(),
            theme::label_font(theme::LABEL_SIZE + 1.0),
        );
        text.appendAttributedString(&rich_text::attributed(&heading.to_uppercase(), &bold));
        let dim = Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE));
        text.appendAttributedString(&rich_text::attributed(count, &dim));
        self.add_label(
            &text,
            rect(INDENT, y + 4.0, width - 2.0 * INDENT, HEADER_HEIGHT - 4.0),
        );
        y + HEADER_HEIGHT
    }

    /// A card behind a row, drawn before the row so it stays underneath.
    fn add_card(&self, width: f64, y: f64, height: f64, border: Option<&NSColor>) {
        let card = FlippedView::new(self.mtm, rect(0.0, y, width, height));
        card.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        widgets::paint(
            &card,
            Some(&theme::card()),
            border.map(|color| (color, NOW_BORDER)),
        );
        self.list.addSubview(&card);
    }

    fn add_label(&self, text: &NSAttributedString, frame: NSRect) {
        let label = NSTextField::labelWithAttributedString(text, self.mtm);
        label.setFrame(frame);
        label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.list.addSubview(&label);
    }

    /// A label wrapped to `width`; returns its height.
    fn add_wrapped(&self, text: &NSAttributedString, x: f64, y: f64, width: f64) -> f64 {
        let label = NSTextField::wrappingLabelWithString(&NSString::from_str(""), self.mtm);
        label.setAttributedStringValue(text);
        label.setPreferredMaxLayoutWidth(width);
        let height = label
            .sizeThatFits(NSSize::new(width, f64::MAX))
            .height
            .ceil();
        label.setFrame(rect(x, y, width, height));
        self.list.addSubview(&label);
        height
    }
}

/// "1. Fix ACC-52 · Linear", struck through once done; hovering underlines it.
fn item_title(
    item: &PlanItem,
    number: Option<usize>,
    done: bool,
    hover: bool,
) -> Retained<NSAttributedString> {
    let color = if hover {
        theme::accent()
    } else if done {
        theme::faint()
    } else {
        theme::ink()
    };
    let attributes = Attributes {
        strike: done && !hover,
        underline: hover,
        ..Attributes::plain(color, theme::font(theme::BODY_SIZE))
    };
    let label = match number {
        Some(n) => format!("{n}. {}", item.title),
        None => item.title.clone(),
    };
    let title = NSMutableAttributedString::new();
    title.appendAttributedString(&rich_text::attributed(&label, &attributes));
    if let Some(source) = &item.source {
        let dim = Attributes::plain(theme::faint(), theme::font(theme::SMALL_SIZE));
        title.appendAttributedString(&rich_text::attributed(&format!(" · {source}"), &dim));
    }
    title.into_super()
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
