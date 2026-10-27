# Autopilot

The sidebar's **Autopilot** page lets Meow start work by itself when a Slack
message asks you for something: "your PR #42 has conflicts", "can you look at
why the build fails", "send me the numbers for X". Turn it on in
**Settings › Autopilot** with **Prepare work from Slack messages** (it is off
by default); **Settings** on the Autopilot page goes there.

- **When to start:** in Settings › Autopilot, say which messages Autopilot
  should prepare work for, in your own words, e.g. "Only private messages
  about a coding task". The triage follows it over its own rules. It is told
  where each message was written (a direct message, a group one, a channel
  mentioning you, a thread) so it can tell private messages apart. Empty, it
  takes any message asking you for something.
- **How to work:** in the same place, say how the work should be done, e.g.
  "For coding tasks, open a new task in Superset in the right workspace, and
  open a draft PR at the end". The work follows it over its own instructions,
  except that what cannot be undone (opening the PR included) still waits for
  your approval. Return saves either one, as does leaving the field.

- **Triage:** once a DM, group message or mention (in a thread too) has been
  quiet for 30 seconds (so a request written in several messages is read
  whole), Claude (Haiku, with no tools) reads the conversation's last messages
  (or the thread's) and decides whether there is work to prepare. This works
  in any conversation, open in Meow or not, and leaves it unread. Greetings,
  news and decisions only you can make are left alone, as is a conversation
  you already answered. Rules in `~/.config/meow/autopilot.md`, if you keep
  that file, apply to both the triage and the work. The last messages it made no job of are
  listed under the jobs, each with why (no work, already answered, an answer
  it could not read, a failure).
- **Work:** one job at a time, with your usual tools and connectors. The
  triage says whether the work is hard: changing code, or a long, tricky
  investigation, runs on Opus; looking up an answer, explaining code or
  drafting a reply, on Sonnet (`MEOW_AUTOPILOT_MODEL` runs every job on one
  model). While it works, the card shows the model and its last steps. Code
  work happens in a new git worktree of your clone, at the tip of the PR's
  branch (found with `gh`, the clone found under `~/Developer` and the usual
  folders, or `MEOW_CODE_DIRS`), never in your own checkout. The agent
  merges, edits, builds, tests and commits there. A PR given by number alone
  ("PR #1727") is looked up among your open PRs and the ones you are asked to
  review. When no worktree can be made (no clone, the repository unknown), the
  work still runs, in an empty folder, and the agent is told why and to name
  the repository to `gh`.
- **Nothing irreversible without you:** every tool call goes through Meow's
  guard (the app's binary as a Claude Code `PreToolUse` hook, `meow --guard`).
  Reading, searching, and editing files inside the job's folder go through.
  Anything else is held: pushing, `gh pr comment/merge/…`, POST requests,
  publishing or deploying, deleting outside the folder, and every connector
  tool that does more than read (sending a Slack message or an email, saving a
  Linear issue…). A reply on Slack is prepared with Meow's own
  `slack_send_message`, in the request's thread (or its DM), so it goes out
  through your Slack session in Meow, not through the Slack connector. The
  held call is recorded exactly as Claude made it, shows on the job's card at
  once, and Claude carries on without it.
- **Approval:** the job's card shows its summary and each held call (the
  command, or the message with where it goes) with **Approve** and **Reject**.
  An approved call is made once the work has ended: approving resumes the
  job's Claude session, and the guard lets through that one call, with that
  exact input, once. Results land in Activity, and the
  sidebar shows how many jobs wait for you.

- **Feedback on one job:** once a job ran (done or failed, with no approved
  call next up), **Give feedback** on its card points the box at the bottom at it:
  type what it should do instead ("rebase, don't merge", "answer in
  English") and press Return or **Send**. Its calls still waiting are
  rejected, and the job queues again: it resumes its Claude session in the
  same worktree, told what you said and which calls were made, and holds
  again what it still wants to do. What you said shows on the card. Esc or
  **Cancel feedback** leaves the job as it is. Feedback once kept for every
  job now lives at the end of **How to work**.

`×` removes a job and its worktree; **Open folder** shows the worktree in
Finder. Jobs are saved in `~/Library/Application Support/Meow/autopilot.json`,
their worktrees under `…/Meow/autopilot/<id>/work`, and finished ones go after
7 days.

## Settings (environment variables)

| Variable | Default | Effect |
|---|---|---|
| `MEOW_AUTOPILOT_MODEL` | Opus when hard, else Sonnet | Claude model for every Autopilot job |
| `MEOW_CODE_DIRS`  | `~/Developer`, `~/Projects`, `~/Code`… | Where Autopilot looks for clones (colon-separated) |
