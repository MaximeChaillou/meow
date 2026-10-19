//! The main floating window, drawn as the cat's speech bubble: a sidebar on
//! the left, and the chat, task or plan page.

use std::cell::{Cell, RefCell};

use meow::actions::QuickAction;
use meow::tasks::TaskList;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSColor, NSLineBreakMode, NSScreen, NSStatusWindowLevel,
    NSTextField, NSWindowCollectionBehavior,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::bubble::{BubblePanel, BubbleView};
use super::chat_page::ChatPage;
use super::controller::Controller;
use super::plan_page::PlanPage;
use super::rich_text::{self, Attributes};
use super::sidebar::{self, Page, Selection, Sidebar};
use super::slack_page::SlackPage;
use super::task_page::TaskPage;
use super::theme;
use super::widgets;

const WIDTH: f64 = 620.0;
const HEIGHT: f64 = 470.0;
const MIN_SIZE: NSSize = NSSize::new(480.0, 300.0);
const CLOSE_SIZE: f64 = 26.0;
const HEADER_INSET: f64 = 10.0;
const RUN_WIDTH: f64 = 70.0;

pub struct ChatWindow {
    panel: Retained<BubblePanel>,
    bubble: Retained<BubbleView>,
    sidebar: Sidebar,
    chat: ChatPage,
    tasks: TaskPage,
    plan: PlanPage,
    slack: SlackPage,
    page: Cell<Page>,
    /// Top line of the page: "Chat", "Tasks", or the task being discussed.
    header: Retained<NSTextField>,
    /// Re-runs the quick action shown on the chat page.
    run_button: Retained<NSButton>,
    chat_title: RefCell<String>,
    /// "Tasks", with the background check's status.
    tasks_title: RefCell<String>,
    chat_selection: Cell<Selection>,
    can_run: Cell<bool>,
}

impl ChatWindow {
    pub fn new(
        mtm: MainThreadMarker,
        controller: &Retained<Controller>,
        actions: &[QuickAction],
    ) -> Self {
        let panel = make_panel(mtm);
        let bubble = BubbleView::new(mtm, rect(0.0, 0.0, WIDTH, HEIGHT));
        bubble.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        panel.setContentView(Some(&bubble));
        // Tells the controller when the bubble loses focus, so it can close.
        panel.setDelegate(Some(ProtocolObject::from_ref(&**controller)));

        let inner = bubble.content_rect();
        let sidebar = Sidebar::new(mtm, controller, actions, inner);
        bubble.addSubview(&sidebar.view);
        bubble.set_sidebar_right(inner.origin.x + sidebar::WIDTH);

        let page_frame = rect(
            inner.origin.x + sidebar::WIDTH,
            inner.origin.y + CLOSE_SIZE,
            inner.size.width - sidebar::WIDTH,
            inner.size.height - CLOSE_SIZE,
        );
        let chat = ChatPage::new(mtm, controller, page_frame);
        let tasks = TaskPage::new(mtm, controller, page_frame);
        let plan = PlanPage::new(mtm, controller, page_frame);
        let slack = SlackPage::new(mtm, controller, page_frame);
        bubble.addSubview(&chat.view);
        bubble.addSubview(&tasks.view);
        bubble.addSubview(&plan.view);
        bubble.addSubview(&slack.view);
        bubble.addSubview(&close_button(mtm, controller, inner));
        let header = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        header.setFrame(rect(
            inner.origin.x + sidebar::WIDTH + HEADER_INSET,
            inner.origin.y + 2.0,
            inner.size.width - sidebar::WIDTH - CLOSE_SIZE - RUN_WIDTH - 2.0 * HEADER_INSET,
            CLOSE_SIZE - 2.0,
        ));
        header.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        header.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        bubble.addSubview(&header);

        let target: &AnyObject = controller;
        let run_button = widgets::text_button(mtm, "Run", target, sel!(runAction:));
        run_button.setToolTip(Some(&NSString::from_str("Run this quick action again")));
        run_button.setFrame(rect(
            inner.origin.x + inner.size.width - CLOSE_SIZE - RUN_WIDTH,
            inner.origin.y,
            RUN_WIDTH,
            CLOSE_SIZE,
        ));
        // Flipped bubble: a flexible bottom margin keeps it in the top row.
        run_button.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        bubble.addSubview(&run_button);

        let window = Self {
            panel,
            bubble,
            sidebar,
            chat,
            tasks,
            plan,
            slack,
            page: Cell::new(Page::Chat),
            header,
            run_button,
            chat_title: RefCell::new("Chat".to_string()),
            tasks_title: RefCell::new("Tasks".to_string()),
            chat_selection: Cell::new(Selection::Chat),
            can_run: Cell::new(false),
        };
        window.show_page(Page::Chat);
        window
    }

    pub fn chat(&self) -> &ChatPage {
        &self.chat
    }

    pub fn tasks(&self) -> &TaskPage {
        &self.tasks
    }

    pub fn plan(&self) -> &PlanPage {
        &self.plan
    }

    pub fn slack(&self) -> &SlackPage {
        &self.slack
    }

    /// Puts a draft in the Slack reply box and focuses it.
    pub fn put_slack_draft(&self, text: &str) {
        self.slack.set_input(text, &self.panel);
    }

    pub fn is_visible(&self) -> bool {
        self.panel.isVisible()
    }

    pub fn is_key(&self) -> bool {
        self.panel.isKeyWindow()
    }

    pub fn page(&self) -> Page {
        self.page.get()
    }

    /// Switches the right side to `page` and puts the cursor in its input.
    pub fn show_page(&self, page: Page) {
        self.page.set(page);
        self.chat.view.setHidden(page != Page::Chat);
        self.tasks.view.setHidden(page != Page::Tasks);
        self.plan.view.setHidden(page != Page::Plan);
        self.slack.view.setHidden(page != Page::Slack);
        let (title, selection, can_run) = match page {
            // A plan is a quick action's result: same title and [ RUN ] as its chat.
            Page::Chat | Page::Plan => (
                self.chat_title.borrow().clone(),
                self.chat_selection.get(),
                self.can_run.get(),
            ),
            Page::Tasks => (self.tasks_title.borrow().clone(), Selection::Tasks, false),
            Page::Slack => ("Slack".to_string(), Selection::Slack, false),
        };
        self.sidebar.select(selection);
        self.run_button.setHidden(!can_run);
        self.header.setAttributedStringValue(&rich_text::attributed(
            &title,
            &Attributes {
                heavy: true,
                ..Attributes::plain(theme::ink(), theme::font(theme::BODY_SIZE))
            },
        ));
        self.focus();
    }

    /// Names the conversation shown on the chat or plan page, the sidebar
    /// entry it belongs to, and whether it is a quick action that can be run again.
    pub fn set_chat_title(&self, title: &str, selection: Selection, can_run: bool) {
        self.chat_title.replace(title.to_string());
        self.chat_selection.set(selection);
        self.can_run.set(can_run);
        if self.page() != Page::Tasks {
            self.show_page(self.page());
        }
    }

    /// Header of the Tasks page, e.g. "Tasks · checked 2 min ago".
    pub fn set_tasks_title(&self, title: &str) {
        if *self.tasks_title.borrow() == title {
            return;
        }
        self.tasks_title.replace(title.to_string());
        if self.page() == Page::Tasks {
            self.show_page(Page::Tasks);
        }
    }

    fn focus(&self) {
        match self.page() {
            Page::Chat => self.chat.focus(&self.panel),
            Page::Tasks => self.tasks.focus(&self.panel),
            Page::Plan => self.plan.focus(&self.panel),
            Page::Slack => self.slack.focus(&self.panel),
        }
    }

    /// Redraws the task page and the open count in the sidebar.
    pub fn render_tasks(&self, tasks: &TaskList) {
        self.tasks.render(tasks);
        self.sidebar.set_open_tasks(tasks.open_count());
    }

    /// Opens the bubble just above `anchor` (the cat), kept on screen, with
    /// its tail pointing at the cat.
    pub fn show_above(&self, anchor: NSRect, mtm: MainThreadMarker) {
        let size = self.panel.frame().size;
        let cat_center = anchor.origin.x + anchor.size.width / 2.0;
        let mut x = cat_center - size.width / 2.0;
        let mut y = anchor.origin.y + anchor.size.height;
        if let Some(screen) = NSScreen::screens(mtm).firstObject() {
            let visible = screen.visibleFrame();
            x = x.clamp(
                visible.origin.x,
                visible.origin.x + visible.size.width - size.width,
            );
            y = y.min(visible.origin.y + visible.size.height - size.height);
        }
        self.panel.setFrameOrigin(NSPoint::new(x, y));
        self.bubble.set_tail_x(cat_center - x);
        self.panel.makeKeyAndOrderFront(None);
        self.focus();
    }

    pub fn hide(&self) {
        self.panel.orderOut(None);
    }

    pub fn set_busy(&self, busy: bool) {
        self.sidebar.set_busy(busy);
    }
}

fn make_panel(mtm: MainThreadMarker) -> Retained<BubblePanel> {
    let panel = BubblePanel::new(mtm, NSSize::new(WIDTH, HEIGHT));
    // SAFETY: the panel is owned by `ChatWindow` for the app's lifetime.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setTitle(&NSString::from_str("Meow"));
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    // The bubble draws its own pixel shadow.
    panel.setHasShadow(false);
    panel.setMovableByWindowBackground(true);
    panel.setFloatingPanel(true);
    panel.setHidesOnDeactivate(false);
    panel.setBecomesKeyOnlyIfNeeded(false);
    panel.setLevel(NSStatusWindowLevel);
    panel.setMinSize(MIN_SIZE);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel
}

/// The bubble has no title bar, so it gets its own small close button.
fn close_button(
    mtm: MainThreadMarker,
    controller: &Controller,
    inner: NSRect,
) -> Retained<NSButton> {
    let target: &AnyObject = controller;
    // SAFETY: the controller is an app-lifetime target that implements `toggleChat:`.
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(""),
            Some(target),
            Some(sel!(toggleChat:)),
            mtm,
        )
    };
    button.setBordered(false);
    button.setAttributedTitle(&rich_text::attributed(
        "✕",
        &Attributes::plain(theme::muted(), theme::font(theme::TITLE_SIZE)),
    ));
    button.setToolTip(Some(&NSString::from_str("Close (Esc)")));
    button.setFrame(rect(
        inner.origin.x + inner.size.width - CLOSE_SIZE,
        inner.origin.y,
        CLOSE_SIZE,
        CLOSE_SIZE,
    ));
    // The bubble view is flipped: a flexible bottom margin keeps it at the top.
    button.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMaxYMargin,
    );
    button
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
