//! The day plan page of a plan quick action: showing it, ticking its items
//! (they are tasks) and asking about the day.

use meow::actions::Layout;
use meow::conversations::ChatId;
use meow::plan::DayPlan;
use meow::tasks;

use super::requests::RunKind;
use super::{App, now_secs};

impl App {
    /// The plan of quick action `index` and when it was made, if it has one.
    pub(super) fn plan_of(&self, index: usize) -> Option<(&DayPlan, u64)> {
        let action = self.actions.get(index)?;
        if action.layout != Layout::Plan {
            return None;
        }
        let result = self.action_results.get(&action.title)?;
        Some((result.plan.as_ref()?, result.ran_at))
    }

    /// The plan of the conversation on screen.
    fn shown_plan(&self) -> Option<(&DayPlan, u64)> {
        match self.conversations.active() {
            ChatId::Action(index) => self.plan_of(*index),
            ChatId::Main | ChatId::Task(_) => None,
        }
    }

    /// Redraws the plan page, e.g. after a task was ticked.
    pub(super) fn render_plan(&self) {
        if let Some((plan, made_at)) = self.shown_plan() {
            self.chat
                .plan()
                .render(plan, made_at, &self.tasks, now_secs());
        }
    }

    /// A question typed under the plan: asked in the action's conversation,
    /// which then shows on the chat page.
    pub(super) fn ask_about_plan(&mut self) {
        let text = self.chat.plan().take_input();
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let id = self.conversations.active().clone();
        self.open_conversation(id.clone(), false);
        self.start_request(&id, text, text, RunKind::Message);
    }

    /// Ticks a plan item off, adding it to the task list if it is not there.
    pub(super) fn toggle_plan_item(&mut self, row: usize) {
        let Some(item) = self.chat.plan().item_at(row) else {
            return;
        };
        let Some(id) = tasks::id_for(&item) else {
            return;
        };
        if self.tasks.get(&id).is_none() {
            self.tasks.merge(std::slice::from_ref(&item), now_secs());
        }
        self.toggle_task_id(&id);
    }

    /// Opens the item's link, or its task's conversation when it has none.
    pub(super) fn open_plan_item(&mut self, row: usize) {
        let Some(item) = self.chat.plan().item_at(row) else {
            return;
        };
        if let Some(url) = &item.url {
            open_url(url);
            return;
        }
        let Some(id) = tasks::id_for(&item) else {
            return;
        };
        if self.tasks.get(&id).is_none() {
            self.tasks.merge(std::slice::from_ref(&item), now_secs());
            self.tasks_changed();
        }
        self.chat_about_task_id(&id);
    }

    pub(super) fn open_plan_link(&mut self, index: usize) {
        if let Some(url) = self.chat.plan().link_at(index) {
            open_url(&url);
        }
    }
}

pub(super) fn open_url(url: &str) {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}
