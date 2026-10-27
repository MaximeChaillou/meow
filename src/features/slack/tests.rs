use meow::activity::Link;
use meow::autopilot::{ActionState, Origin, Plan, Stage};
use meow::guard::Call;

use super::{Slack, SlackSettings};
use crate::app::feature::Feature;
use crate::features::testing::FakeHost;
use crate::ui::sidebar::Page;
use crate::ui::{autopilot_page, slack_page};

fn slack() -> Slack {
    Slack::signed_out(Ok(SlackSettings::default()))
}

#[test]
fn slack_and_autopilot_pages_keep_their_place_in_the_sidebar() {
    let pages: Vec<_> = slack()
        .pages()
        .iter()
        .map(|page| (page.id, page.title, page.order))
        .collect();
    assert_eq!(
        pages,
        [
            (slack_page::PAGE_ID, "Slack", 30),
            (autopilot_page::PAGE_ID, "Autopilot", 50),
        ]
    );
}

#[test]
fn autopilot_news_opens_the_autopilot_page() {
    let mut feature = slack();
    let mut host = FakeHost::default();
    assert!(feature.open_link(&Link::Autopilot, &mut host));
    assert_eq!(host.opened, [Page::Feature(autopilot_page::PAGE_ID)]);
}

#[test]
fn other_links_are_left_to_the_app() {
    let mut feature = slack();
    let mut host = FakeHost::default();
    assert!(!feature.open_link(&Link::Tasks, &mut host));
    assert!(host.opened.is_empty());
}

#[test]
fn esc_away_from_the_slack_page_is_left_to_the_app() {
    let mut feature = slack();
    assert!(!feature.escape(&mut FakeHost::default()));
}

#[test]
fn autopilot_feedback_is_saved_and_can_be_removed() {
    let mut feature = slack();
    let mut host = FakeHost::default();
    feature.add_autopilot_feedback("Only open the PR,\n no Slack message", &mut host);
    feature.add_autopilot_feedback("Answer in French", &mut host);
    feature.add_autopilot_feedback("   ", &mut host);
    let saved: SlackSettings = host.settings.get().unwrap();
    assert_eq!(
        saved.autopilot_feedback,
        ["Only open the PR, no Slack message", "Answer in French"]
    );

    feature.remove_autopilot_feedback(0, &mut host);
    feature.remove_autopilot_feedback(5, &mut host);
    let saved: SlackSettings = host.settings.get().unwrap();
    assert_eq!(saved.autopilot_feedback, ["Answer in French"]);
    assert_eq!(feature.settings, saved);
}

/// `feature` with a job that ran and waits for a push to be approved.
fn with_a_finished_job(feature: &mut Slack) -> u64 {
    let plan = Plan {
        title: "Fix the conflicts on PR #42".into(),
        prompt: "Merge main into the PR".into(),
        repo: None,
        pr: None,
        branch: None,
        hard: None,
    };
    let origin = Origin {
        channel: "D1".into(),
        ts: "1".into(),
        thread_ts: None,
        from: "Axel".into(),
        text: "Ta PR #42 a des conflits".into(),
    };
    let jobs = &mut feature.autopilot.jobs;
    let id = jobs.add(plan, origin, 0).id;
    let push = Call {
        tool: "Bash".into(),
        input: serde_json::json!({ "command": "git push fork HEAD:feat/x" }),
    };
    jobs.finish(id, Ok("Merged main".into()), vec![push], 1);
    id
}

#[test]
fn feedback_given_while_replying_to_a_job_goes_to_that_job_only() {
    let mut feature = slack();
    let mut host = FakeHost::default();
    let id = with_a_finished_job(&mut feature);
    feature.reply_to_autopilot(id);
    assert_eq!(feature.autopilot.replying, Some(id));

    feature.send_autopilot_feedback("Rebase instead of merging", &mut host);
    let job = feature.autopilot.jobs.get(id).unwrap();
    assert_eq!(job.feedback, ["Rebase instead of merging"]);
    assert_eq!(job.stage, Stage::Queued);
    assert_eq!(feature.autopilot.replying, None);
    assert!(feature.settings.autopilot_feedback.is_empty());

    feature.send_autopilot_feedback("Answer in French", &mut host);
    assert_eq!(feature.settings.autopilot_feedback, ["Answer in French"]);
    assert_eq!(feature.autopilot.jobs.get(id).unwrap().feedback.len(), 1);
}

#[test]
fn replying_to_a_job_is_toggled_and_only_once_it_ran() {
    let mut feature = slack();
    let id = with_a_finished_job(&mut feature);
    feature.reply_to_autopilot(id);
    feature.reply_to_autopilot(id);
    assert_eq!(feature.autopilot.replying, None, "pressed again, it stops");

    feature.autopilot.jobs.get_mut(id).unwrap().stage = Stage::Working;
    feature.reply_to_autopilot(id);
    assert_eq!(feature.autopilot.replying, None, "still at work");
    feature.reply_to_autopilot(99);
    assert_eq!(feature.autopilot.replying, None);
}

#[test]
fn feedback_goes_to_every_job_once_the_job_replied_to_makes_a_call() {
    let mut feature = slack();
    let mut host = FakeHost::default();
    let id = with_a_finished_job(&mut feature);
    feature.reply_to_autopilot(id);
    let jobs = &mut feature.autopilot.jobs;
    jobs.set_action(id, 0, ActionState::Approved, None);
    feature.send_autopilot_feedback("Answer in French", &mut host);
    assert_eq!(feature.settings.autopilot_feedback, ["Answer in French"]);
    assert!(feature.autopilot.jobs.get(id).unwrap().feedback.is_empty());
    assert_eq!(feature.autopilot.replying, None);
}

#[test]
fn blank_feedback_to_a_job_keeps_replying_to_it() {
    let mut feature = slack();
    let mut host = FakeHost::default();
    let id = with_a_finished_job(&mut feature);
    feature.reply_to_autopilot(id);
    feature.send_autopilot_feedback("  ", &mut host);
    assert_eq!(feature.autopilot.replying, Some(id));
    assert_eq!(
        feature.autopilot.jobs.get(id).unwrap().stage,
        Stage::Finished
    );
}

#[test]
fn unreadable_settings_are_told_at_launch() {
    let feature = Slack::signed_out(Err("Could not read settings".to_string()));
    assert_eq!(
        feature.startup_error().as_deref(),
        Some("Could not read settings. Settings changes won't be saved.")
    );
    assert_eq!(feature.settings, SlackSettings::default());
}

#[test]
fn snapshots_show_samples_of_both_pages() {
    assert_eq!(
        slack().snapshot_steps(),
        [
            "autopilot",
            "slack-list",
            "slack-conversation",
            "slack-focus",
            "slack-thread",
            "slack-assist",
        ]
    );
}

#[test]
fn nothing_runs_until_asked() {
    assert!(!slack().is_busy());
}
