//! The Autopilot page: work Meow started by itself from Slack messages, newest
//! first. Each card says who asked what, how the work went, and lists the
//! actions that wait for the user (a push, a reply…) with Approve and Reject.
//! A running job shows its last steps. Feedback typed at the bottom is kept
//! and followed by every later job, or, after "Give feedback" on a job that
//! ran, sent to that job, which works again following it. Below the jobs,
//! the messages the triage last left alone say why.

use std::cell::RefCell;

use meow::autopilot::{self, Action, ActionState, Job, Jobs, LeftAlone, Stage};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSColor, NSEvent, NSScrollView, NSTextAlignment,
    NSTextField, NSView,
};
use objc2_foundation::{NSAttributedString, NSPoint, NSRect, NSSize, NSString};

use super::action::{Action as Target, ActionSink};
use super::bubble;
use super::controller::Controller;
use super::flipped::FlippedView;
use super::rich_text::{self, Attributes};
use super::theme;
use super::widgets;

const MARGIN: f64 = 16.0;
const ROW_HEIGHT: f64 = 22.0;
const CARD_PADDING: f64 = 8.0;
const CARD_GAP: f64 = 6.0;
const LINE_GAP: f64 = 4.0;
const STATUS_WIDTH: f64 = 150.0;
const DELETE_WIDTH: f64 = 22.0;
const BUTTON_WIDTH: f64 = 82.0;
const BUTTON_GAP: f64 = 6.0;
const FOLDER_WIDTH: f64 = 110.0;
const TOGGLE_WIDTH: f64 = 110.0;
const SAVE_WIDTH: f64 = 76.0;
const REPLY_WIDTH: f64 = 130.0;
const INDENT: f64 = 6.0;
const ACTION_INSET: f64 = 14.0;
const WAITING_BORDER: f64 = 2.0;
const FEEDBACK_HINT: &str = "Feedback for Autopilot, e.g. \"open the PR, don't message on Slack\"";
const SAVE_TIP: &str = "Every job from now on follows this feedback";
const SEND_TIP: &str = "The job works again, following this; Esc gives feedback for every job";
/// Longest a held call's details are shown.
const MAX_DETAIL_CHARS: usize = 700;
/// Held actions are named `job row * TAG_STRIDE + action index`.
pub const TAG_STRIDE: usize = 1000;

/// The Autopilot page's id among the features' pages.
pub const PAGE_ID: &str = "autopilot";

/// What the Autopilot page's controls ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutopilotAction {
    /// Turn Autopilot on or off.
    Toggle,
    /// Approve or reject a held action: job row × `TAG_STRIDE` + action index.
    Approve(usize),
    Reject(usize),
    /// Remove the job in this row.
    Dismiss(usize),
    /// Show the worktree of the job in this row.
    OpenFolder(usize),
    /// Keep the feedback typed at the bottom.
    Feedback,
    /// Forget the feedback at this index.
    RemoveFeedback(usize),
    /// Give feedback to the job in this row, or stop to.
    Reply(usize),
}

pub struct AutopilotPage {
    pub view: Retained<NSView>,
    scroll: Retained<NSScrollView>,
    list: Retained<FlippedView>,
    input: Retained<widgets::GrowingField>,
    save: Retained<NSButton>,
    /// The targets of the list's controls, all drawn again with it.
    actions: ActionSink<AutopilotAction>,
    /// The targets of the feedback field and its button, kept for them.
    _fixed: ActionSink<AutopilotAction>,
    /// Job ids in the order shown; buttons carry an index into it.
    shown_ids: RefCell<Vec<u64>>,
    mtm: MainThreadMarker,
}

impl AutopilotPage {
    /// The page, its controls reporting to `on_action`; `controller` handles
    /// the feedback field's keys.
    pub fn new(
        mtm: MainThreadMarker,
        controller: &Retained<Controller>,
        frame: NSRect,
        on_action: impl Fn(AutopilotAction) + 'static,
    ) -> Self {
        let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let (width, height) = (frame.size.width - bubble::PADDING, frame.size.height);
        let fixed = ActionSink::new(mtm, on_action);

        let input = widgets::text_field(mtm, FEEDBACK_HINT);
        let save_target = fixed.target(AutopilotAction::Feedback);
        // SAFETY: the page keeps the target, and the controller lives as long
        // as the app, so the weak target and delegate stay valid.
        unsafe {
            input.setTarget(Some(&save_target));
            input.setAction(Some(Target::selector()));
            input.setDelegate(Some(ProtocolObject::from_ref(&**controller)));
        }
        // Saved on Return only, not when the field merely loses focus.
        if let Some(cell) = input.cell() {
            cell.setSendsActionOnEndEditing(false);
        }
        let save = widgets::text_button(
            mtm,
            "Save",
            widgets::ButtonStyle::Accent,
            &save_target,
            Target::selector(),
        );
        save.setToolTip(Some(&NSString::from_str(SAVE_TIP)));
        let bottom = widgets::composer(
            mtm,
            &view,
            &input,
            &[],
            &[(&save, SAVE_WIDTH, widgets::ButtonStyle::Accent)],
        ) + 0.5 * MARGIN;

        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            rect(
                MARGIN,
                bottom,
                width - 2.0 * MARGIN,
                height - bottom - 0.5 * MARGIN,
            ),
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
            save,
            actions: fixed.sibling(),
            _fixed: fixed,
            shown_ids: RefCell::new(Vec::new()),
            mtm,
        }
    }

    /// The id of the job in row `index`, as last rendered.
    pub fn id_at(&self, index: usize) -> Option<u64> {
        self.shown_ids.borrow().get(index).copied()
    }

    /// Whether a mouse button is down: drawing the list again now would
    /// swallow the click on the button under it.
    pub fn is_clicked(&self) -> bool {
        NSEvent::pressedMouseButtons() != 0
    }

    /// The feedback typed, emptying the field.
    pub fn take_feedback(&self) -> String {
        let text = self.input.stringValue().to_string();
        self.input.set_text("");
        text
    }

    /// Puts the cursor in the feedback field.
    pub fn focus_feedback(&self) {
        if let Some(window) = self.input.window() {
            window.makeFirstResponder(Some(&self.input));
        }
    }

    /// Draws the switch, the feedback kept, the jobs, then the messages left
    /// alone; the feedback field speaks to `replying`, else to every job.
    pub fn render(
        &self,
        jobs: &Jobs,
        enabled: bool,
        feedback: &[String],
        replying: Option<&Job>,
        now: u64,
    ) {
        self.show_target(replying);
        let width = self.scroll.contentSize().width;
        for row in self.list.subviews().iter() {
            row.removeFromSuperview();
        }
        self.actions.clear();
        let mut y = self.add_switch(enabled, width);
        y = self.add_feedback(feedback, width, y);
        let mut ids = Vec::new();
        for job in jobs.all() {
            let replied = replying.is_some_and(|r| r.id == job.id);
            y = self.add_job(ids.len(), job, replied, width, y, now) + CARD_GAP;
            ids.push(job.id);
        }
        y = self.add_left_alone(jobs.left_alone(), width, y, now);
        let height = y.max(self.scroll.contentSize().height);
        self.list.setFrameSize(NSSize::new(width, height));
        self.shown_ids.replace(ids);
    }

    /// The feedback field's hint and button, for `replying` or for every job.
    fn show_target(&self, replying: Option<&Job>) {
        let (hint, label, tip) = match replying {
            Some(job) => (
                format!(
                    "What should \"{}\" do instead? (Esc: for every job)",
                    clip(&job.title, 60)
                ),
                "Send",
                SEND_TIP,
            ),
            None => (FEEDBACK_HINT.to_string(), "Save", SAVE_TIP),
        };
        widgets::set_placeholder(&self.input, &hint);
        widgets::set_text_button_label(&self.save, label, widgets::ButtonStyle::Accent);
        self.save.setToolTip(Some(&NSString::from_str(tip)));
    }

    /// What Autopilot does, and the button turning it on or off. Returns the y below.
    fn add_switch(&self, enabled: bool, width: f64) -> f64 {
        let hint = if enabled {
            "On: when a Slack message asks you for something, Claude prepares it here. \
             Nothing that can't be undone (a push, a message sent, an issue changed) \
             happens until you approve it."
        } else {
            "Off. Turned on, Claude prepares what Slack messages ask you for (fixing a \
             PR's conflicts, drafting a reply…), and waits for your approval before \
             anything that can't be undone."
        };
        let style = Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE));
        let text_width = width - TOGGLE_WIDTH - 3.0 * INDENT;
        let height = self
            .add_wrapped(
                &rich_text::attributed(hint, &style),
                INDENT,
                0.0,
                text_width,
            )
            .max(ROW_HEIGHT);
        let (label, style) = if enabled {
            ("Turn off", widgets::ButtonStyle::Quiet)
        } else {
            ("Turn on", widgets::ButtonStyle::Go)
        };
        let target = self.actions.target(AutopilotAction::Toggle);
        let toggle = widgets::text_button(self.mtm, label, style, &target, Target::selector());
        toggle.setFrame(rect(
            width - INDENT - TOGGLE_WIDTH,
            0.0,
            TOGGLE_WIDTH,
            ROW_HEIGHT,
        ));
        toggle.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        self.list.addSubview(&toggle);
        height + 2.0 * CARD_GAP
    }

    /// The feedback every job follows, each line with × to forget it.
    /// Returns the y below.
    fn add_feedback(&self, feedback: &[String], width: f64, y: f64) -> f64 {
        if feedback.is_empty() {
            return y;
        }
        let heading = Attributes {
            heavy: true,
            ..Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE))
        };
        let mut top =
            y + self.add_wrapped(
                &rich_text::attributed("Your feedback, followed by every job", &heading),
                INDENT,
                y,
                width - 2.0 * INDENT,
            ) + LINE_GAP;
        let body = Attributes::plain(theme::ink(), theme::font(theme::SMALL_SIZE));
        let text_width = width - 3.0 * INDENT - DELETE_WIDTH;
        for (index, line) in feedback.iter().enumerate() {
            let height = self
                .add_wrapped(
                    &rich_text::attributed(&format!("• {line}"), &body),
                    INDENT,
                    top,
                    text_width,
                )
                .max(ROW_HEIGHT);
            let remove = widgets::text_button(
                self.mtm,
                "×",
                widgets::ButtonStyle::Link,
                &self.actions.target(AutopilotAction::RemoveFeedback(index)),
                Target::selector(),
            );
            remove.setToolTip(Some(&NSString::from_str("Forget this feedback")));
            remove.setFrame(rect(
                width - INDENT - DELETE_WIDTH,
                top,
                DELETE_WIDTH,
                ROW_HEIGHT,
            ));
            remove.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
            self.list.addSubview(&remove);
            top += height + LINE_GAP;
        }
        top + 2.0 * CARD_GAP
    }

    /// The messages the triage last made no job of, each with why. Returns the y below.
    fn add_left_alone(&self, left: &[LeftAlone], width: f64, y: f64, now: u64) -> f64 {
        if left.is_empty() {
            return y;
        }
        let heading = Attributes {
            heavy: true,
            ..Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE))
        };
        let text_width = width - 2.0 * INDENT;
        let mut top = y
            + CARD_GAP
            + self.add_wrapped(
                &rich_text::attributed("Left alone recently", &heading),
                INDENT,
                y + CARD_GAP,
                text_width,
            )
            + LINE_GAP;
        let faint = Attributes::plain(theme::faint(), theme::font(theme::SMALL_SIZE));
        for entry in left {
            let line = format!(
                "{} · {}: {} — {}",
                ago(now, entry.at),
                entry.from,
                clip(&entry.text, 120),
                entry.why
            );
            top += self.add_wrapped(
                &rich_text::attributed(&line, &faint),
                INDENT,
                top,
                text_width,
            ) + LINE_GAP;
        }
        top + CARD_GAP
    }

    /// A card: title, status and ×; who asked what; the feedback it was
    /// given; the work's last steps while it runs, else its summary; each
    /// held action; then the button giving it feedback once it ran.
    /// `replied` when the feedback field speaks to it. Returns the y below.
    fn add_job(&self, row: usize, job: &Job, replied: bool, width: f64, y: f64, now: u64) -> f64 {
        let card = FlippedView::new(self.mtm, rect(0.0, y, width, 0.0));
        card.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.list.addSubview(&card);
        let inner = width - 4.0 * INDENT;
        let mut top = y + CARD_PADDING;

        let delete_x = width - INDENT - DELETE_WIDTH;
        let status_x = delete_x - STATUS_WIDTH;
        let title = Attributes {
            heavy: true,
            ..Attributes::plain(theme::ink(), theme::font(theme::BODY_SIZE))
        };
        self.add_label(
            &rich_text::attributed(&job.title, &title),
            rect(2.0 * INDENT, top, status_x - 3.0 * INDENT, ROW_HEIGHT),
        );
        let (status, color) = status(job, now);
        let small = Attributes::plain(color, theme::font(theme::SMALL_SIZE));
        let label = self.add_label(
            &rich_text::attributed(&status, &small),
            rect(status_x, top + 3.0, STATUS_WIDTH, ROW_HEIGHT - 3.0),
        );
        label.setAlignment(NSTextAlignment::Right);
        label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        let delete = widgets::text_button(
            self.mtm,
            "×",
            widgets::ButtonStyle::Link,
            &self.actions.target(AutopilotAction::Dismiss(row)),
            Target::selector(),
        );
        delete.setToolTip(Some(&NSString::from_str(
            "Remove this job, and its worktree",
        )));
        delete.setFrame(rect(delete_x, top, DELETE_WIDTH, ROW_HEIGHT));
        delete.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        self.list.addSubview(&delete);
        top += ROW_HEIGHT;

        let muted = Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE));
        let asked = format!("{}: {}", job.origin.from, clip(&job.origin.text, 240));
        top += self.add_wrapped(
            &rich_text::attributed(&asked, &muted),
            2.0 * INDENT,
            top,
            inner,
        ) + LINE_GAP;

        if let Some(folder) = &job.folder {
            let faint = Attributes::plain(theme::faint(), theme::font(theme::SMALL_SIZE));
            let line = format!("{} · {}", folder.branch, folder.path.display());
            let text_width = inner - FOLDER_WIDTH - INDENT;
            let label = self.add_label(
                &rich_text::attributed(&line, &faint),
                rect(2.0 * INDENT, top + 3.0, text_width, ROW_HEIGHT - 3.0),
            );
            label.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingMiddle);
            let open = widgets::text_button(
                self.mtm,
                "Open folder",
                widgets::ButtonStyle::Link,
                &self.actions.target(AutopilotAction::OpenFolder(row)),
                Target::selector(),
            );
            open.setFrame(rect(
                width - 2.0 * INDENT - FOLDER_WIDTH,
                top,
                FOLDER_WIDTH,
                ROW_HEIGHT,
            ));
            open.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
            self.list.addSubview(&open);
            top += ROW_HEIGHT + LINE_GAP;
        }

        let yours = Attributes::plain(theme::highlight(), theme::font(theme::SMALL_SIZE));
        for feedback in &job.feedback {
            top += self.add_wrapped(
                &rich_text::attributed(&format!("You: {feedback}"), &yours),
                2.0 * INDENT,
                top,
                inner,
            ) + LINE_GAP;
        }

        if job.stage == Stage::Working {
            let faint = Attributes::plain(theme::faint(), theme::font(theme::SMALL_SIZE));
            for step in &job.steps {
                top += self.add_wrapped(
                    &rich_text::attributed(&format!("› {step}"), &faint),
                    2.0 * INDENT,
                    top,
                    inner,
                );
            }
            if !job.steps.is_empty() {
                top += LINE_GAP;
            }
        }

        if let Some(summary) = &job.summary {
            let color = if job.stage == Stage::Failed {
                theme::error()
            } else {
                theme::ink()
            };
            let body = Attributes::plain(color, theme::font(theme::SMALL_SIZE));
            top += self.add_wrapped(
                &rich_text::attributed(summary, &body),
                2.0 * INDENT,
                top,
                inner,
            ) + LINE_GAP;
        }

        for (index, action) in job.actions.iter().enumerate() {
            top = self.add_action(row * TAG_STRIDE + index, action, width, top) + LINE_GAP;
        }

        if job.takes_feedback() {
            let (label, tip) = if replied {
                ("Cancel feedback", "Feedback goes to every job again")
            } else {
                (
                    "Give feedback",
                    "Tell this job what to do instead; it works again",
                )
            };
            let reply = widgets::text_button(
                self.mtm,
                label,
                widgets::ButtonStyle::Link,
                &self.actions.target(AutopilotAction::Reply(row)),
                Target::selector(),
            );
            reply.setToolTip(Some(&NSString::from_str(tip)));
            reply.setFrame(rect(
                width - 2.0 * INDENT - REPLY_WIDTH,
                top,
                REPLY_WIDTH,
                ROW_HEIGHT,
            ));
            reply.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
            self.list.addSubview(&reply);
            top += ROW_HEIGHT + LINE_GAP;
        }

        let height = top - y + CARD_PADDING - LINE_GAP;
        card.setFrameSize(NSSize::new(width, height));
        let fill = if job.is_open() {
            theme::card()
        } else {
            theme::card_done()
        };
        let border = (replied || job.pending() > 0).then(theme::accent);
        widgets::paint(
            &card,
            Some(&fill),
            border.as_deref().map(|color| (color, WAITING_BORDER)),
        );
        y + height
    }

    /// A held action: what it does and its details, with Approve and Reject
    /// while it waits, else how it went. Returns the y below.
    fn add_action(&self, tag: usize, action: &Action, width: f64, y: f64) -> f64 {
        let (what, detail) = autopilot::describe(&action.call);
        let x = 2.0 * INDENT + ACTION_INSET;
        let buttons_width = 2.0 * BUTTON_WIDTH + BUTTON_GAP;
        let text_width = width - x - 2.0 * INDENT - buttons_width - INDENT;
        let heading = Attributes {
            heavy: true,
            ..Attributes::plain(theme::highlight(), theme::font(theme::SMALL_SIZE))
        };
        let mut top = y;
        top += self.add_wrapped(&rich_text::attributed(&what, &heading), x, top, text_width);
        let code = Attributes::plain(theme::ink(), theme::font(theme::SMALL_SIZE));
        top += self.add_wrapped(
            &rich_text::attributed(&clip(&detail, MAX_DETAIL_CHARS), &code),
            x,
            top,
            text_width,
        );
        let right = width - 2.0 * INDENT;
        let pin = NSAutoresizingMaskOptions::ViewMinXMargin;
        match action.state {
            ActionState::Pending => {
                let buttons = [
                    (
                        "Approve",
                        widgets::ButtonStyle::Go,
                        AutopilotAction::Approve(tag),
                    ),
                    (
                        "Reject",
                        widgets::ButtonStyle::Quiet,
                        AutopilotAction::Reject(tag),
                    ),
                ];
                let mut x = right - buttons_width;
                for (label, style, choice) in buttons {
                    let target = self.actions.target(choice);
                    let button =
                        widgets::text_button(self.mtm, label, style, &target, Target::selector());
                    button.setFrame(rect(x, y, BUTTON_WIDTH, ROW_HEIGHT));
                    button.setAutoresizingMask(pin);
                    self.list.addSubview(&button);
                    x += BUTTON_WIDTH + BUTTON_GAP;
                }
            }
            state => {
                let (text, color) = action_status(state);
                let style = Attributes::plain(color, theme::font(theme::SMALL_SIZE));
                let label = self.add_label(
                    &rich_text::attributed(text, &style),
                    rect(
                        right - buttons_width,
                        y + 3.0,
                        buttons_width,
                        ROW_HEIGHT - 3.0,
                    ),
                );
                label.setAlignment(NSTextAlignment::Right);
                label.setAutoresizingMask(pin);
                if let Some(result) = &action.result {
                    let muted = Attributes::plain(theme::muted(), theme::font(theme::SMALL_SIZE));
                    top += self.add_wrapped(
                        &rich_text::attributed(result, &muted),
                        x,
                        top,
                        text_width,
                    );
                }
            }
        }
        top.max(y + ROW_HEIGHT)
    }

    fn add_label(&self, text: &NSAttributedString, frame: NSRect) -> Retained<NSTextField> {
        let label = NSTextField::labelWithAttributedString(text, self.mtm);
        label.setFrame(frame);
        label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.list.addSubview(&label);
        label
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

/// The job's status at the top right of its card.
fn status(job: &Job, now: u64) -> (String, Retained<NSColor>) {
    match job.stage {
        Stage::Queued => ("waiting its turn".into(), theme::muted()),
        Stage::Preparing => ("making a worktree…".into(), theme::live()),
        Stage::Working => match &job.ran_on {
            Some(model) => (format!("working on {}…", capitalized(model)), theme::live()),
            None => ("working…".into(), theme::live()),
        },
        Stage::Failed => ("failed".into(), theme::error()),
        Stage::Finished => match job.pending() {
            0 => {
                let done = job
                    .finished_at
                    .map(|at| format!("done {}", ago(now, at)))
                    .unwrap_or_else(|| "done".into());
                (done, theme::faint())
            }
            1 => ("1 to approve".into(), theme::accent()),
            n => (format!("{n} to approve"), theme::accent()),
        },
    }
}

/// How long before `now` `at` was: "just now", "5 min ago", "2 h ago".
fn ago(now: u64, at: u64) -> String {
    match now.saturating_sub(at) / 60 {
        0 => "just now".to_string(),
        m if m < 60 => format!("{m} min ago"),
        m => format!("{} h ago", m / 60),
    }
}

/// "opus" as "Opus".
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn action_status(state: ActionState) -> (&'static str, Retained<NSColor>) {
    match state {
        ActionState::Pending => ("", theme::muted()),
        ActionState::Approved => ("approved, next up", theme::live()),
        ActionState::Running => ("making it…", theme::live()),
        ActionState::Done => ("done", theme::live()),
        ActionState::Rejected => ("rejected", theme::faint()),
        ActionState::Failed => ("failed", theme::error()),
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(max).collect();
    clipped.push('…');
    clipped
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
