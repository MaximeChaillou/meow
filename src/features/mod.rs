//! Every feature of the app, one folder each (see ARCHITECTURE.md). Adding a
//! feature adds its folder plus one `mod` line and one entry in `all`, kept in
//! alphabetical order so parallel branches rarely touch the same lines.

#[cfg(test)]
mod fake_host;
mod routines;
mod update;

use crate::app::feature::Feature;

/// The features, in the order they are polled and their menu items shown.
// One entry per line, even when rustfmt would join them: each branch then
// adds its own line, which git merges without a conflict.
#[rustfmt::skip]
pub(crate) fn all() -> Vec<Box<dyn Feature>> {
    vec![
        Box::new(routines::ScheduledTasks::from_env()),
        Box::new(update::Update::from_env()),
    ]
}
