//! Ties the cat, the chat bubble, the hotkey and Claude runs together.

mod agents;
mod coat;
mod plan;
mod quick_actions;
mod requests;
mod schedule;
mod slack;
mod slack_assist;
mod slack_calendar;
mod slack_voice;
mod sync;
mod task_actions;

use std::cell::RefCell;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use meow::action_results::ActionResults;
use meow::actions::{self, QuickAction};
use meow::behavior::{Bounds, Cat, Inputs, PropView, Rng};
use meow::claude::ClaudeEnv;
use meow::conversations::{ChatId, Conversation, Conversations};
use meow::settings::Settings;
use meow::sprites::{Look, render, render_prop};
use meow::tasks::{self, TaskList};
use meow::transcript::Transcript;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSEvent, NSMenu, NSScreen};
use objc2_foundation::{NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSTimer};

use crate::ui::cat_window::{self, CatWindow};
use crate::ui::chat_window::ChatWindow;
use crate::ui::controller::Controller;
use crate::ui::events::{self, UiEvent};
use crate::ui::menus;
use crate::ui::pointer;
use crate::ui::sidebar::{Page, Selection};
use crate::ui::theme;

use requests::ActiveRun;

const TICK_SECONDS: f64 = 1.0 / 30.0;
const MAX_STEP_SECONDS: f64 = 0.1;
pub const HOTKEY_LABEL: &str = "⌥⇧Space";

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

struct App {
    mtm: MainThreadMarker,
    _controller: Retained<Controller>,
    cat_window: CatWindow,
    /// The cat's bowl, or the mouse it chases.
    prop_window: CatWindow,
    last_prop: Option<PropView>,
    /// Where the cat holds the pointer still, while it has it caught.
    held_pointer: Option<NSPoint>,
    chat: ChatWindow,
    conversations: Conversations,
    tasks: TaskList,
    /// Where tasks are saved; `None` when the file could not be read, so it is never overwritten.
    tasks_path: Option<PathBuf>,
    action_results: ActionResults,
    /// Where action results are saved; `None` when the file could not be read.
    results_path: Option<PathBuf>,
    settings: Settings,
    /// Where settings are saved; `None` when the file could not be read.
    settings_path: Option<PathBuf>,
    /// The theme submenu of the cat's menu, to check the current coat.
    coat_menu: Retained<NSMenu>,
    cat: Cat,
    rng: Rng,
    last_look: Option<Look>,
    last_tick: Instant,
    claude: Option<Result<ClaudeEnv, String>>,
    discovery: Option<Receiver<Result<ClaudeEnv, String>>>,
    run: Option<ActiveRun>,
    actions: Vec<QuickAction>,
    /// The live Slack connection and conversation state.
    slack: slack::SlackState,
    /// The private Claude question about the open Slack conversation.
    slack_assist: slack_assist::SlackAssist,
    /// The user's learned writing style, for Slack drafts.
    voice: slack_voice::VoiceState,
    /// The calendar tools of the user's Claude Code, for events from Slack.
    calendar: slack_calendar::CalendarState,
    /// The background check of tasks.
    sync: sync::SyncState,
    /// Daily quick actions.
    schedule: schedule::ScheduleState,
    /// Coding agents started in Superset from the chat or Slack.
    agents: agents::AgentsState,
    /// A reply arrived out of sight; the cat shows `!` until it is seen.
    unread: bool,
    /// The conversation whose reply is waiting.
    unread_chat: ChatId,
    _hotkeys: Option<GlobalHotKeyManager>,
}

pub fn start(mtm: MainThreadMarker) {
    // Before any window is built, so every label gets the pixel font.
    let fonts_error = theme::register_fonts().err();
    // Before anything is loaded, so data saved under the old name is found.
    let migration_errors = std::env::var_os("HOME")
        .map(|home| meow::legacy::migrate_data(std::path::Path::new(&home)))
        .unwrap_or_default();
    let controller = Controller::new(mtm);
    let (actions, actions_error) = load_actions();
    let (settings, settings_path, settings_error) = coat::load_settings();
    menus::install_main_menu(mtm, &controller);
    let (cat_menu, coat_menu) = menus::cat_menu(mtm, &controller, &actions, settings.coat);
    let cat_window = CatWindow::new(mtm, &cat_menu);
    let prop_window = CatWindow::prop(mtm);
    let chat = ChatWindow::new(mtm, &controller, &actions);

    let mut main = Transcript::default();
    let (hotkeys, hotkey_error) = match register_hotkey() {
        Ok(manager) => (Some(manager), None),
        Err(e) => (None, Some(format!("Global shortcut unavailable: {e}"))),
    };
    let actions_error = actions_error.map(|e| format!("Using the default quick actions. {e}"));
    main.info(&format!(
        "Click the cat or press {HOTKEY_LABEL} to talk. Right-click for options."
    ));
    let (action_results, results_path, results_error) = quick_actions::load_action_results();
    let startup_errors = [
        &fonts_error,
        &actions_error,
        &hotkey_error,
        &results_error,
        &settings_error,
    ];
    for error in startup_errors
        .into_iter()
        .flatten()
        .chain(&migration_errors)
    {
        eprintln!("meow: {error}");
        main.error(error);
    }
    let (task_list, tasks_path) = load_tasks(&mut main);
    chat.render_tasks(&task_list);
    chat.chat().render(&main, true);
    let mut conversations = Conversations::default();
    conversations.get_mut(&ChatId::Main).transcript = main;

    let (discovered, discovery) = mpsc::channel();
    thread::spawn(move || {
        let _ = discovered.send(ClaudeEnv::discover());
    });

    let start_x = screen_bounds(mtm)
        .map(|(bounds, _)| (bounds.min_x + bounds.max_x) / 2.0)
        .unwrap_or(0.0);
    let app = App {
        mtm,
        _controller: controller,
        cat_window,
        prop_window,
        last_prop: None,
        held_pointer: None,
        chat,
        conversations,
        tasks: task_list,
        tasks_path,
        action_results,
        results_path,
        settings,
        settings_path,
        coat_menu,
        cat: Cat::new(start_x),
        rng: Rng::new(seed()),
        last_look: None,
        last_tick: Instant::now(),
        unread: startup_errors.iter().any(|e| e.is_some()) || !migration_errors.is_empty(),
        unread_chat: ChatId::Main,
        claude: None,
        discovery: Some(discovery),
        run: None,
        actions,
        slack: slack::SlackState::new(),
        slack_assist: slack_assist::SlackAssist::default(),
        voice: slack_voice::VoiceState::load(),
        calendar: slack_calendar::CalendarState::default(),
        sync: sync::SyncState::from_env(),
        schedule: schedule::ScheduleState::new(),
        agents: agents::AgentsState::default(),
        _hotkeys: hotkeys,
    };
    APP.with(|cell| cell.replace(Some(app)));
    schedule_ticks();
}

fn schedule_ticks() {
    let block = RcBlock::new(|_timer: NonNull<NSTimer>| {
        APP.with(|cell| {
            if let Ok(mut app) = cell.try_borrow_mut()
                && let Some(app) = app.as_mut()
            {
                app.tick();
            }
        });
    });
    // SAFETY: the block only touches main-thread state and the timer fires on the main run
    // loop. Common modes keep the cat moving while a menu is open or a window is resized.
    unsafe {
        let timer = NSTimer::timerWithTimeInterval_repeats_block(TICK_SECONDS, true, &block);
        NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
    }
}

/// Quick actions from the config file, or the defaults plus the reason it could not be used.
fn load_actions() -> (Vec<QuickAction>, Option<String>) {
    let Some(path) = actions::config_path() else {
        return (actions::defaults(), None);
    };
    match actions::load(&path) {
        Ok(list) => (list, None),
        Err(error) => (actions::defaults(), Some(error)),
    }
}

/// Saved tasks, minus those done over a day ago. A file that cannot be read is
/// reported and left untouched.
fn load_tasks(transcript: &mut Transcript) -> (TaskList, Option<PathBuf>) {
    let Some(path) = tasks::default_path() else {
        return (TaskList::default(), None);
    };
    match TaskList::load(&path) {
        Ok(mut list) => {
            list.prune(now_secs());
            (list, Some(path))
        }
        Err(error) => {
            eprintln!("meow: {error}");
            transcript.error(&format!("{error}. Task changes won't be saved."));
            (TaskList::default(), None)
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn register_hotkey() -> Result<GlobalHotKeyManager, String> {
    let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
    manager
        .register(HotKey::new(
            Some(Modifiers::ALT | Modifiers::SHIFT),
            Code::Space,
        ))
        .map_err(|e| e.to_string())?;
    Ok(manager)
}

fn seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
}

/// Where the cat window may go on the main screen, and the ground's screen height.
fn screen_bounds(mtm: MainThreadMarker) -> Option<(Bounds, f64)> {
    let visible = NSScreen::screens(mtm).firstObject()?.visibleFrame();
    let bounds = Bounds::new(
        visible.origin.x,
        visible.origin.x + visible.size.width - cat_window::WIDTH,
        visible.size.height - cat_window::HEIGHT,
    );
    Some((bounds, visible.origin.y))
}

impl App {
    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_tick)
            .as_secs_f64()
            .min(MAX_STEP_SECONDS);
        self.last_tick = now;

        for event in events::drain() {
            self.handle(event);
        }
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == HotKeyState::Pressed {
                self.toggle_chat();
            }
        }
        self.poll_discovery();
        self.poll_claude();
        self.poll_slack();
        self.poll_slack_assist();
        self.poll_voice();
        self.poll_calendar_probe();
        self.poll_sync();
        self.poll_schedule();
        self.poll_agents();
        self.animate(dt);
    }

    fn inputs(&self) -> Inputs {
        Inputs {
            busy: self.run.is_some(),
            chat_open: self.chat.is_visible(),
            unread: self.unread,
        }
    }

    fn animate(&mut self, dt: f64) {
        let Some((bounds, ground)) = screen_bounds(self.mtm) else {
            return;
        };
        let inputs = self.inputs();
        let pointer = NSEvent::mouseLocation();
        self.cat.see_pointer(pointer.x, pointer.y - ground);
        self.cat.update(dt, bounds, inputs, &mut self.rng);
        self.cat_window.move_to(self.cat.x, ground + self.cat.y);
        self.show_prop(ground);
        self.hold_pointer(ground);

        let look = self.cat.look(inputs);
        if self.last_look != Some(look) {
            self.cat_window.view.show(render(&look, self.settings.coat));
            self.last_look = Some(look);
        }
    }

    /// Pins the pointer under the cat's paw while it has it caught.
    fn hold_pointer(&mut self, ground: f64) {
        let held = self
            .cat
            .caught_pointer()
            .map(|(x, y)| NSPoint::new(x.round(), (ground + y).round()));
        if held == self.held_pointer {
            return;
        }
        match held {
            Some(at) => pointer::hold(self.mtm, at),
            None => pointer::release(),
        }
        self.held_pointer = held;
    }

    fn show_prop(&mut self, ground: f64) {
        let prop = self.cat.prop();
        if prop == self.last_prop {
            return;
        }
        match prop {
            Some(view) => {
                self.prop_window.move_to(view.x as f64, ground);
                self.prop_window
                    .view
                    .show(render_prop(view.prop, view.facing));
                if self.last_prop.is_none() {
                    self.prop_window.panel.orderFrontRegardless();
                    // The cat's head goes over the bowl's rim.
                    self.cat_window.panel.orderFrontRegardless();
                }
            }
            None => self.prop_window.panel.orderOut(None),
        }
        self.last_prop = prop;
    }

    /// Feeds the cat, or lets a mouse in, from its menu.
    fn play(&mut self, event: UiEvent) {
        let Some((bounds, _)) = screen_bounds(self.mtm) else {
            return;
        };
        match event {
            UiEvent::Feed => self.cat.feed(bounds),
            _ => self.cat.release_mouse(bounds, &mut self.rng),
        };
    }

    fn handle(&mut self, event: UiEvent) {
        match event {
            UiEvent::CatClicked | UiEvent::ToggleChat => self.toggle_chat(),
            UiEvent::Grab => self.cat.grab(),
            UiEvent::DragTo { x, y } => {
                let ground = screen_bounds(self.mtm).map_or(0.0, |(_, ground)| ground);
                self.cat.drag_to(x, y - ground);
            }
            UiEvent::Drop => self.cat.release(),
            UiEvent::Stroke(distance) => self.cat.stroke(distance),
            UiEvent::SetCoat(index) => self.set_coat(index),
            UiEvent::Feed | UiEvent::ReleaseMouse => self.play(event),
            UiEvent::QuickAction(index) => self.quick_action(index),
            UiEvent::RunAction => self.rerun_shown_action(),
            UiEvent::ShowChat => self.show_conversation(ChatId::Main),
            UiEvent::ShowTasks => {
                if !self.chat.is_visible() {
                    self.open_window();
                }
                self.chat.show_page(Page::Tasks);
            }
            UiEvent::AddTask => self.add_task(),
            UiEvent::ToggleTask(row) => self.toggle_task(row),
            UiEvent::OpenTask(row) => self.open_task(row),
            UiEvent::ChatAboutTask(row) => self.chat_about_task(row),
            UiEvent::ShowSlack => self.show_slack(),
            UiEvent::SignInSlack => self.sign_in_slack(),
            UiEvent::OpenSlackConversation(row) => self.open_slack_conversation(row),
            UiEvent::SlackBack => self.slack_back(),
            UiEvent::SendSlack => self.send_slack(),
            UiEvent::AskSlack => self.ask_slack(),
            UiEvent::AutocompleteSlack => self.autocomplete_slack(),
            UiEvent::CreateSlackEvent => self.create_slack_event(),
            UiEvent::ToggleWriteAsMe => self.toggle_write_as_me(),
            UiEvent::LearnMyStyle => self.learn_my_style(),
            UiEvent::UseSlackDraft => self.use_slack_draft(),
            UiEvent::CloseSlackAssist => self.close_slack_assist(),
            UiEvent::AgentSlack => self.agent_slack(),
            UiEvent::StartSlackAgent => self.start_slack_agent(),
            UiEvent::AskPlan => self.ask_about_plan(),
            UiEvent::TogglePlanItem(row) => self.toggle_plan_item(row),
            UiEvent::OpenPlanItem(row) => self.open_plan_item(row),
            UiEvent::OpenPlanLink(index) => self.open_plan_link(index),
            UiEvent::ToggleSteps(turn) => self.toggle_steps(turn),
            UiEvent::Submit => self.submit(),
            UiEvent::Escape if self.run.is_some() => self.cancel_run(),
            UiEvent::Escape if self.chat.page() == Page::Slack && self.slack_assist_shown() => {
                self.close_slack_assist()
            }
            UiEvent::Escape
                if self.chat.page() == Page::Slack && self.chat.slack().in_conversation() =>
            {
                self.slack_back()
            }
            UiEvent::Escape => self.chat.hide(),
            // Checked on the next tick: focus may already be back (e.g. a sheet closed).
            UiEvent::FocusLost if !self.chat.is_key() => self.chat.hide(),
            UiEvent::FocusLost => {}
            UiEvent::NewConversation => self.new_conversation(),
            UiEvent::Quit => {
                pointer::release();
                self.slack.save_cache();
                NSApplication::sharedApplication(self.mtm).terminate(None);
            }
        }
    }

    fn toggle_chat(&mut self) {
        if self.chat.is_visible() && self.chat.is_key() {
            self.chat.hide();
            return;
        }
        if self.unread {
            // A reply is waiting: show it rather than whatever was open.
            self.unread = false;
            self.show_conversation(self.unread_chat.clone());
        }
        self.open_window();
    }

    fn open_window(&self) {
        NSApplication::sharedApplication(self.mtm).activate();
        self.chat.show_above(self.cat_frame(), self.mtm);
    }

    fn cat_frame(&self) -> NSRect {
        self.cat_window.frame()
    }

    /// Shows conversation `id`, opening it the first time: a plan action's
    /// plan on the plan page, anything else (or a run in progress) on the chat page.
    fn show_conversation(&mut self, id: ChatId) {
        self.open_conversation(id, true);
    }

    fn open_conversation(&mut self, id: ChatId, prefer_plan: bool) {
        let fresh = self.fresh_conversation(&id);
        self.conversations.open(id.clone(), || fresh);
        self.update_chat_title();
        let running = self.run.as_ref().is_some_and(|run| run.chat() == &id);
        let has_plan = match id {
            ChatId::Action(index) => self.plan_of(index).is_some(),
            ChatId::Main | ChatId::Task(_) => false,
        };
        if prefer_plan && has_plan && !running {
            self.render_plan();
            self.chat.show_page(Page::Plan);
            return;
        }
        self.chat.show_page(Page::Chat);
        self.chat
            .chat()
            .render(&self.conversations.current().transcript, true);
    }

    /// How conversation `id` starts the first time it is shown.
    fn fresh_conversation(&self, id: &ChatId) -> Conversation {
        match id {
            ChatId::Main => Conversation::default(),
            ChatId::Task(task) => {
                Conversation::resuming(self.tasks.get(task).and_then(|t| t.session.clone()))
            }
            ChatId::Action(index) => self.saved_action_conversation(*index),
        }
    }

    /// Header and sidebar highlight for the conversation on the chat page.
    fn update_chat_title(&self) {
        let (title, selection, can_run) = match self.conversations.active() {
            ChatId::Main => ("Chat".to_string(), Selection::Chat, false),
            ChatId::Task(task) => {
                let title = match self.tasks.get(task) {
                    Some(task) => format!("{} · {}", task.source, task.title),
                    None => "Task".to_string(),
                };
                (title, Selection::Tasks, false)
            }
            ChatId::Action(index) => (self.action_title(*index), Selection::Action(*index), true),
        };
        self.chat.set_chat_title(&title, selection, can_run);
    }

    fn poll_discovery(&mut self) {
        let Some(discovery) = &self.discovery else {
            return;
        };
        let result = match discovery.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Could not look for the claude command.".to_string())
            }
        };
        if let Err(error) = &result {
            eprintln!("meow: {error}");
            self.error_in(&ChatId::Main, error);
            self.unread = true;
            self.unread_chat = ChatId::Main;
        }
        self.claude = Some(result);
        self.discovery = None;
        self.discover_superset();
    }

    /// Redraws conversation `id` if it is the one on screen.
    fn refresh(&self, id: &ChatId, scroll_to_end: bool) {
        if self.conversations.active() == id {
            self.chat
                .chat()
                .render(&self.conversations.current().transcript, scroll_to_end);
        }
    }

    fn transcript(&mut self, id: &ChatId) -> &mut Transcript {
        &mut self.conversations.get_mut(id).transcript
    }

    fn info_in(&mut self, id: &ChatId, text: &str) {
        self.transcript(id).info(text);
        self.refresh(id, true);
    }

    fn error_in(&mut self, id: &ChatId, text: &str) {
        self.transcript(id).error(text);
        self.refresh(id, true);
    }

    /// A note in the conversation on screen.
    fn info(&mut self, text: &str) {
        let id = self.conversations.active().clone();
        self.info_in(&id, text);
    }

    fn toggle_steps(&mut self, turn: usize) {
        let id = self.conversations.active().clone();
        self.transcript(&id).toggle(turn);
        self.refresh(&id, false);
    }
}
