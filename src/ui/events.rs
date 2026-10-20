//! Queue between AppKit callbacks and the app tick.
//!
//! Callbacks only enqueue; the tick handles everything, so no callback ever
//! runs while app state is borrowed.

use std::cell::RefCell;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiEvent {
    CatClicked,
    /// The cat was picked up with the mouse.
    Grab,
    /// Dragged cat window origin, in screen coordinates.
    DragTo {
        x: f64,
        y: f64,
    },
    Drop,
    /// The mouse moved this far over the cat (petting).
    Stroke(f64),
    /// Index into `Coat::ALL`.
    SetCoat(usize),
    /// Put down a bowl of kibble for the cat.
    Feed,
    /// Let a mouse run across the screen for the cat to chase.
    ReleaseMouse,
    /// Index into the loaded quick actions.
    QuickAction(usize),
    /// Run the quick action shown on the chat page again.
    RunAction,
    /// Fold or unfold the working steps of a reply (transcript entry index).
    ToggleSteps(usize),
    ShowChat,
    ShowTasks,
    /// Add the text typed in the task window.
    AddTask,
    /// Row index in the task window, as last rendered.
    ToggleTask(usize),
    /// Show or hide the completed tasks.
    ToggleCompleted,
    OpenTask(usize),
    /// Open the conversation about the task in this row.
    ChatAboutTask(usize),
    /// Ask about the day plan with the text typed under it.
    AskPlan,
    /// Item index on the plan page, as last rendered.
    TogglePlanItem(usize),
    OpenPlanItem(usize),
    /// Agenda link index on the plan page.
    OpenPlanLink(usize),
    /// Accept or decline the invitation of this agenda event on the plan page.
    AcceptPlanEvent(usize),
    DeclinePlanEvent(usize),
    ToggleChat,
    /// Open the Activity page.
    ShowActivity,
    /// Open what the activity item in this row is about.
    OpenActivity(usize),
    /// Dismiss the activity item in this row.
    DismissActivity(usize),
    ClearActivity,
    /// Open the Scheduled page.
    ShowRoutines,
    /// Delete the scheduled task in this row.
    DeleteRoutine(usize),
    /// Open the Slack page.
    ShowSlack,
    /// Start the Slack sign-in flow.
    SignInSlack,
    /// Open the Slack conversation in this row, as last listed.
    OpenSlackConversation(usize),
    /// Show the Slack list's tab of direct messages, or of channels.
    SlackTab(crate::ui::slack_page::SlackTab),
    /// Go back from a Slack conversation to the list.
    SlackBack,
    /// Send the reply typed in the open Slack conversation.
    SendSlack,
    /// Ask Claude, privately, about the open Slack conversation.
    AskSlack,
    /// Have Claude research and draft the reply, with the user's tools.
    AutocompleteSlack,
    /// Turn drafting in the user's own style on or off.
    ToggleWriteAsMe,
    /// Learn the user's style from their Slack messages.
    LearnMyStyle,
    /// Put Claude's draft in the Slack reply box.
    UseSlackDraft,
    /// Create the calendar event Claude proposed.
    CreateSlackEvent,
    /// Close the private Claude panel.
    CloseSlackAssist,
    /// Have Claude propose a coding agent for the open Slack conversation.
    AgentSlack,
    /// Start the coding agent Claude proposed.
    StartSlackAgent,
    Submit,
    Escape,
    /// The bubble stopped being the focused window.
    FocusLost,
    NewConversation,
    /// Check for an update now, or restart into one already installed.
    CheckForUpdates,
    Quit,
}

thread_local! {
    static QUEUE: RefCell<VecDeque<UiEvent>> = const { RefCell::new(VecDeque::new()) };
}

pub fn push(event: UiEvent) {
    QUEUE.with(|queue| queue.borrow_mut().push_back(event));
}

pub fn drain() -> Vec<UiEvent> {
    QUEUE.with(|queue| queue.borrow_mut().drain(..).collect())
}
