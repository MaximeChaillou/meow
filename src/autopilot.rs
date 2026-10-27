//! Autopilot: work started by itself when a Slack message asks the user for
//! something. Claude (no tools) first reads the conversation and says whether
//! there is work to prepare ([`Plan`]); a run then does all of it that can be
//! taken back (looking things up, editing and committing in a worktree of the
//! repository, drafting a reply), while [`crate::guard`] holds every call
//! that cannot (sending, pushing, changing an issue). Those wait on the
//! Autopilot page as [`Action`]s until the user approves or rejects each.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::claude::STRONG_MODEL;
use crate::context;
use crate::guard::Call;
use crate::tasks::take_fence;

pub const PLAN_FENCE: &str = "meow-autopilot";
/// Finished jobs with nothing left to approve are kept this long.
pub const KEEP_SECS: u64 = 7 * 24 * 60 * 60;
const MAX_TEXT_CHARS: usize = 2000;
const MAX_SUMMARY_CHARS: usize = 4000;
const MAX_JOBS: usize = 50;
/// Longest a piece of feedback is kept.
const MAX_FEEDBACK_CHARS: usize = 500;
/// The work's model unless it is hard: quick, and enough to look things up
/// and draft a reply.
pub const EVERYDAY_MODEL: &str = "sonnet";
/// How many of a running job's last steps its card shows.
const MAX_STEPS: usize = 3;
/// How many of the messages the triage left alone are listed.
const MAX_LEFT_ALONE: usize = 5;

/// What the triage found to prepare.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Plan {
    /// A few words, for the Autopilot page.
    pub title: String,
    /// The work's prompt, self-contained.
    pub prompt: String,
    /// `owner/name` of the GitHub repository the work is in, if code.
    #[serde(default)]
    pub repo: Option<String>,
    /// The pull request it is about (URL or number), if any.
    #[serde(default)]
    pub pr: Option<String>,
    /// The branch to work on, when no pull request says it.
    #[serde(default)]
    pub branch: Option<String>,
    /// Whether the work changes code or is a long investigation, for the
    /// strong model; unsaid, work in a repository is.
    #[serde(default, deserialize_with = "loose_bool")]
    pub hard: Option<bool>,
}

/// A yes or no the triage may write as `true` or `"true"`; anything else is unsaid.
fn loose_bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<bool>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Bool(said) => Some(said),
        Value::String(said) => match said.trim().to_lowercase().as_str() {
            "true" | "yes" => Some(true),
            "false" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

/// A message the triage made no job of, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeftAlone {
    pub from: String,
    pub text: String,
    pub why: String,
    pub at: u64,
}

/// The Slack message that started a job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub channel: String,
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    /// Who wrote it, and where ("Axel", "Axel in #eng").
    pub from: String,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    /// Waiting for the run before it.
    Queued,
    /// Its worktree being made.
    Preparing,
    Working,
    /// The run ended; its actions may still wait for the user.
    Finished,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionState {
    /// Waiting for the user.
    Pending,
    /// Approved, waiting for its run.
    Approved,
    Running,
    Done,
    Rejected,
    Failed,
}

/// A held call: something the run wanted to do that cannot be taken back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub call: Call,
    pub state: ActionState,
    /// What happened when it was made, or why it was not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// Where code work happens: a worktree of the user's clone, detached at the
/// branch it is about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub path: PathBuf,
    /// The clone the worktree belongs to.
    pub clone: PathBuf,
    /// The remote the branch is pushed to.
    pub remote: String,
    pub branch: String,
    /// The branch a pull request merges into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The pull request's URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub title: String,
    pub origin: Origin,
    pub plan_prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Whether to run on the strong model: see [`Plan::hard`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard: Option<bool>,
    pub created_at: u64,
    pub stage: Stage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<Folder>,
    /// Why no worktree could be made: the work then runs without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_error: Option<String>,
    /// The Claude session, resumed to make approved calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The run's final message, or why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// The running work's last steps ("Bash: Run the tests"), newest last.
    #[serde(skip)]
    pub steps: Vec<String>,
    /// The model the work last ran on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ran_on: Option<String>,
}

impl Job {
    /// Actions the user has not decided on yet.
    pub fn pending(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| a.state == ActionState::Pending)
            .count()
    }

    /// Whether something is still to happen: work, or actions to decide or make.
    pub fn is_open(&self) -> bool {
        matches!(
            self.stage,
            Stage::Queued | Stage::Preparing | Stage::Working
        ) || self.actions.iter().any(|a| {
            matches!(
                a.state,
                ActionState::Pending | ActionState::Approved | ActionState::Running
            )
        })
    }

    /// Whether a worktree is still to be made before the work starts.
    pub fn needs_folder(&self) -> bool {
        (self.repo.is_some() || self.pr.is_some())
            && self.folder.is_none()
            && self.folder_error.is_none()
    }

    /// The Claude model the work runs on.
    pub fn model(&self) -> &'static str {
        let code = self.repo.is_some() || self.pr.is_some();
        if self.hard.unwrap_or(code) {
            STRONG_MODEL
        } else {
            EVERYDAY_MODEL
        }
    }

    /// The run's working folder, with the default when the job has none.
    pub fn cwd<'a>(&'a self, default: &'a Path) -> &'a Path {
        self.folder.as_ref().map_or(default, |f| f.path.as_path())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Jobs {
    /// Newest first.
    jobs: Vec<Job>,
    #[serde(default)]
    next_id: u64,
    /// Newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    left_alone: Vec<LeftAlone>,
}

impl Jobs {
    pub fn all(&self) -> &[Job] {
        &self.jobs
    }

    pub fn get(&self, id: u64) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    /// Jobs waiting on the user: actions to approve.
    pub fn waiting_count(&self) -> usize {
        self.jobs.iter().filter(|j| j.pending() > 0).count()
    }

    /// Whether a job was already started for this message.
    pub fn has_origin(&self, channel: &str, ts: &str) -> bool {
        self.jobs
            .iter()
            .any(|j| j.origin.channel == channel && j.origin.ts == ts)
    }

    /// The messages the triage last made no job of, newest first.
    pub fn left_alone(&self) -> &[LeftAlone] {
        &self.left_alone
    }

    /// Remembers that the triage made no job of `origin`'s message, and `why`.
    pub fn leave_alone(&mut self, origin: Origin, why: String, now: u64) {
        let entry = LeftAlone {
            from: origin.from,
            text: clip(&origin.text, 200),
            why,
            at: now,
        };
        self.left_alone.insert(0, entry);
        self.left_alone.truncate(MAX_LEFT_ALONE);
    }

    /// A new queued job for `plan`, started by `origin`.
    pub fn add(&mut self, plan: Plan, origin: Origin, now: u64) -> &Job {
        self.next_id += 1;
        let job = Job {
            id: self.next_id,
            title: clip(plan.title.trim(), 120),
            origin: Origin {
                text: clip(&origin.text, MAX_TEXT_CHARS),
                ..origin
            },
            plan_prompt: plan.prompt.trim().to_string(),
            repo: plan.repo.filter(|r| !r.trim().is_empty()),
            pr: plan.pr.filter(|p| !p.trim().is_empty()),
            branch: plan.branch.filter(|b| !b.trim().is_empty()),
            hard: plan.hard,
            created_at: now,
            stage: Stage::Queued,
            folder: None,
            folder_error: None,
            session: None,
            summary: None,
            actions: Vec::new(),
            finished_at: None,
            steps: Vec::new(),
            ran_on: None,
        };
        self.jobs.insert(0, job);
        &self.jobs[0]
    }

    /// The oldest job waiting to start.
    pub fn next_queued(&self) -> Option<&Job> {
        self.jobs.iter().rev().find(|j| j.stage == Stage::Queued)
    }

    /// The oldest approved action, as its job's id and the action's index.
    pub fn next_approved(&self) -> Option<(u64, usize)> {
        self.jobs.iter().rev().find_map(|job| {
            job.actions
                .iter()
                .position(|a| a.state == ActionState::Approved)
                .map(|index| (job.id, index))
        })
    }

    /// The run of job `id` ended: `held` are the calls it was refused.
    pub fn finish(&mut self, id: u64, outcome: Result<String, String>, held: Vec<Call>, now: u64) {
        let Some(job) = self.get_mut(id) else { return };
        let (stage, summary) = match outcome {
            Ok(summary) => (Stage::Finished, summary),
            Err(reason) => (Stage::Failed, reason),
        };
        job.stage = stage;
        job.summary = Some(clip(summary.trim(), MAX_SUMMARY_CHARS));
        job.finished_at = Some(now);
        job.steps.clear();
        self.hold(id, held);
    }

    /// Job `id`'s work took `step`: only the last [`MAX_STEPS`] are kept.
    pub fn step(&mut self, id: u64, step: &str) {
        let Some(job) = self.get_mut(id) else { return };
        job.steps.push(step.to_string());
        let extra = job.steps.len().saturating_sub(MAX_STEPS);
        job.steps.drain(..extra);
    }

    /// Adds the calls of job `id` held since, each once, for the user to
    /// decide on. Returns whether any was new.
    pub fn hold(&mut self, id: u64, held: Vec<Call>) -> bool {
        let Some(job) = self.get_mut(id) else {
            return false;
        };
        let before = job.actions.len();
        for call in held {
            if !job.actions.iter().any(|a| a.call.same_as(&call)) {
                job.actions.push(Action {
                    call,
                    state: ActionState::Pending,
                    result: None,
                });
            }
        }
        job.actions.len() > before
    }

    /// Sets action `index` of job `id` to `state`, with its result.
    pub fn set_action(
        &mut self,
        id: u64,
        index: usize,
        state: ActionState,
        result: Option<String>,
    ) {
        if let Some(action) = self.get_mut(id).and_then(|j| j.actions.get_mut(index)) {
            action.state = state;
            if result.is_some() {
                action.result = result.map(|r| clip(r.trim(), MAX_TEXT_CHARS));
            }
        }
    }

    pub fn remove(&mut self, id: u64) -> Option<Job> {
        let index = self.jobs.iter().position(|j| j.id == id)?;
        Some(self.jobs.remove(index))
    }

    /// Forgets old finished jobs, and anything past the newest [`MAX_JOBS`].
    /// Returns the forgotten ones, whose folders can go.
    pub fn prune(&mut self, now: u64) -> Vec<Job> {
        let (mut keep, mut gone) = (Vec::new(), Vec::new());
        for (index, job) in std::mem::take(&mut self.jobs).into_iter().enumerate() {
            let recent = job.finished_at.unwrap_or(job.created_at) + KEEP_SECS > now;
            if index < MAX_JOBS && (job.is_open() || recent) {
                keep.push(job);
            } else {
                gone.push(job);
            }
        }
        self.jobs = keep;
        gone
    }

    /// Jobs a restart interrupted start again from the beginning, forgetting
    /// the calls their run held (its session is gone; the new run holds them
    /// again), and actions that were being made wait for the user again.
    pub fn after_restart(&mut self) {
        for job in &mut self.jobs {
            if matches!(job.stage, Stage::Preparing | Stage::Working) {
                job.stage = Stage::Queued;
                job.actions.clear();
            }
            for action in &mut job.actions {
                if action.state == ActionState::Running {
                    action.state = ActionState::Pending;
                }
                if let Some(call) = through_meow(&action.call) {
                    action.call = call;
                }
            }
        }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("Could not read autopilot jobs from {}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Could not read {}: {e}", path.display())),
        }
    }

    /// Writes atomically, so a crash never leaves a half-written file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let fail = |e: &dyn std::fmt::Display| {
            format!("Could not save autopilot jobs to {}: {e}", path.display())
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| fail(&e))?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, json).map_err(|e| fail(&e))?;
        std::fs::rename(&temp, path).map_err(|e| fail(&e))
    }
}

/// `~/Library/Application Support/Meow`.
fn data_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/Meow"))
}

/// `~/Library/Application Support/Meow/autopilot.json`.
pub fn default_path() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("autopilot.json"))
}

/// Where job `id` keeps its held calls and its worktree.
pub fn job_dir(id: u64) -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("autopilot").join(id.to_string()))
}

/// The user's own rules for the triage, from `~/.config/meow/autopilot.md`.
pub fn user_rules() -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let text =
        std::fs::read_to_string(PathBuf::from(home).join(".config/meow/autopilot.md")).ok()?;
    Some(text.trim().to_string()).filter(|t| !t.is_empty())
}

/// `feedback` with `text` added at the end, on one line: unchanged when
/// `text` is blank or already there.
pub fn add_feedback(feedback: &[String], text: &str) -> Vec<String> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let line = clip(&line, MAX_FEEDBACK_CHARS);
    let mut all = feedback.to_vec();
    if !line.is_empty() && !feedback.contains(&line) {
        all.push(line);
    }
    all
}

/// What the triage and the work follow besides their own instructions: the
/// user's rules file, then each piece of feedback given on the Autopilot page.
pub fn rules(own: Option<&str>, feedback: &[String]) -> Option<String> {
    let lines: Vec<String> = own
        .map(str::to_string)
        .into_iter()
        .chain(feedback.iter().map(|f| format!("- {f}")))
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The triage request: is there work in what `from` just wrote to `me`?
/// `conversation` is the recent messages, oldest first, as `Name: text`.
pub fn triage_prompt(me: &str, from: &str, conversation: &str, rules: Option<&str>) -> String {
    let rules = rules
        .map(|r| format!("\n\n{me}'s own rules, which win over the ones above:\n{r}"))
        .unwrap_or_default();
    format!(
        "You triage {me}'s Slack messages while {me} is busy. {from} just wrote to {me}. Here \
is the conversation, oldest first; the last messages are the new ones:\n\n\
<conversation>\n{conversation}\n</conversation>\n\n\
Decide whether the new messages ask {me} to do something an assistant could prepare ahead: a \
coding agent with {me}'s tools (git clones of the repositories, the GitHub CLI, read access to \
Slack, Linear, Gmail, Notion, the calendar) works on it, and every action that cannot be undone \
(sending a message, pushing, commenting, changing an issue) waits for {me}'s approval. Fixing a \
pull request's conflicts, a bug, a review comment, looking up an answer and drafting the reply: \
yes. Greetings, thanks, news with no request, opinions or decisions only {me} can give, \
something {me} already answered: no.{rules}\n\n\
If there is no work, reply with the single word `none`. Otherwise reply only with a fenced code \
block with the info string `{PLAN_FENCE}` holding one JSON object: {{\"title\": 3 to 8 words, \
\"prompt\": the agent's prompt, self-contained (it sees nothing else): who asked what, the exact \
details (links, PR numbers, branch names, error messages), what done looks like, in English; \
\"repo\": the GitHub repository as owner/name when the work is in code and you can tell it \
(from a link, or a repository named in the conversation), else null; \"pr\": the pull request's \
URL when the conversation links it, else its number, else null; \"branch\": \
the branch to work on when named and there is no pull request, else null; \"hard\": true when \
the work writes or changes code (a fix, conflicts, a review comment, a feature) or is a long, \
tricky investigation, false for looking up an answer, explaining code or drafting a reply}}."
    )
}

/// The plan in a triage answer, else why there is no job: no work, or an
/// answer that could not be read.
pub fn read_triage(answer: &str) -> Result<Plan, &'static str> {
    match parse_plan(answer) {
        Some(plan) => Ok(plan),
        None if answer.contains(PLAN_FENCE) => Err("its answer couldn't be read"),
        None => Err("no work to prepare"),
    }
}

/// The plan in a triage answer; `None` for `none` or anything unreadable.
pub fn parse_plan(answer: &str) -> Option<Plan> {
    let (_, body) = take_fence(answer, PLAN_FENCE);
    let plan: Plan = serde_json::from_str(body?.trim()).ok()?;
    let plan = Plan {
        title: plan.title.trim().to_string(),
        prompt: plan.prompt.trim().to_string(),
        repo: plan
            .repo
            .map(|r| r.trim().trim_end_matches(".git").to_string()),
        pr: plan.pr.map(|p| p.trim().to_string()),
        branch: plan.branch.map(|b| b.trim().to_string()),
        ..plan
    };
    (!plan.title.is_empty() && !plan.prompt.is_empty()).then_some(plan)
}

/// What the run starts with: the plan, where it works, how irreversible
/// actions are handled, and the user's `rules` (see [`rules`]).
pub fn work_prompt(job: &Job, me: &str, rules: Option<&str>) -> String {
    let place = match &job.folder {
        Some(folder) => {
            let pr = folder
                .pr_url
                .as_ref()
                .map(|url| format!(" of the pull request {url}"))
                .unwrap_or_default();
            let base = folder
                .base
                .as_ref()
                .map(|base| format!(" It merges into `{base}` (remote `{}`).", folder.remote))
                .unwrap_or_default();
            format!(
                "\n\nYou are in a git worktree of {me}'s clone, at the tip of the branch `{branch}`{pr} \
(detached HEAD, fetched from `{remote}`).{base} Commit your work here. To publish it, the \
command is exactly `git push {remote} HEAD:{branch}` (add `--force-with-lease` only if you \
rewrote history): run it at the end.",
                branch = folder.branch,
                remote = folder.remote,
            )
        }
        None => match &job.folder_error {
            Some(error) => format!(
                "\n\nMeow could not make a git worktree for this work ({error}), so you are in \
an empty folder that is not a git repository. Find the repository from the request, and name it \
in every `gh` command (`--repo owner/name`). If you need the code, read it with `gh`, or clone \
it into this folder."
            ),
            None => String::new(),
        },
    };
    let rules = rules
        .map(|r| {
            format!(
                "\n\n{me}'s rules and feedback on earlier work, which win over the instructions \
above:\n{r}"
            )
        })
        .unwrap_or_default();
    format!(
        "{prompt}{place}\n\n---\n\
You work for {me}, who is busy: nobody watches this session, so make reasonable choices instead \
of asking, and finish the work. The request came from {from} on Slack: \"{text}\".\n\n\
Do everything that can be taken back yourself: read, search, edit files here, build, test, \
commit. Every action that cannot be undone (pushing, sending or replying to a message, posting \
a comment, changing a Linear issue or a pull request) is held by Meow for {me}'s approval: make \
the call exactly as you would if it were allowed, with its final content (for a reply, the \
message itself, in {me}'s voice and the conversation's language), and it will wait for {me} \
to approve it. Do not try to reach the same result another way. When a reply to {from} would \
help, prepare it that way too, with `{send}` ({reply_to}): Slack messages go through {me}'s own \
Slack in Meow, never through another Slack tool.{rules}\n\n\
End with a short summary for {me}, in the language of the request: what you did, and what is \
waiting for approval.",
        prompt = job.plan_prompt,
        send = context::tool_name(context::SLACK_SEND),
        reply_to = reply_args(&job.origin),
        from = job.origin.from,
        text = clip(&job.origin.text, 600).replace('"', "'"),
    )
}

/// A Slack connector send held before Slack messages went through Meow, as
/// Meow's own send: the connector's tools are no longer offered, so it could
/// not be made once approved.
fn through_meow(call: &Call) -> Option<Call> {
    if call.tool != "mcp__claude_ai_Slack__slack_send_message" {
        return None;
    }
    let input = &call.input;
    let mut moved = serde_json::Map::new();
    for (from, to) in [
        ("channel_id", "channel"),
        ("message", "text"),
        ("thread_ts", "thread_ts"),
    ] {
        if let Some(value) = input.get(from) {
            moved.insert(to.to_string(), value.clone());
        }
    }
    Some(Call {
        tool: context::tool_name(context::SLACK_SEND),
        input: Value::Object(moved),
    })
}

/// Where a reply to `origin` goes, as the send tool's arguments: its thread,
/// else the DM itself, else a thread on the message (in a channel).
fn reply_args(origin: &Origin) -> String {
    let channel = &origin.channel;
    match &origin.thread_ts {
        Some(thread) => format!("channel `{channel}`, thread_ts `{thread}`"),
        None if channel.starts_with('D') => format!("channel `{channel}`"),
        None => format!("channel `{channel}`, thread_ts `{}`", origin.ts),
    }
}

/// What resumes the session once the user approved `call`.
pub fn approve_prompt(call: &Call, me: &str) -> String {
    format!(
        "{me} approved this held call. Make it now, exactly as it is (same tool, same input), \
then stop; do nothing else. Reply in one line with how it went.\n\nTool: {}\nInput: {}",
        call.tool,
        serde_json::to_string(&call.input).unwrap_or_default()
    )
}

/// A held call in a few words, for the Autopilot page: what and where.
pub fn describe(call: &Call) -> (String, String) {
    let input = &call.input;
    let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_string);
    if call.tool == "Bash" {
        let command = text("command").unwrap_or_default();
        let what = text("description").unwrap_or_else(|| "Run a command".to_string());
        return (what, command);
    }
    let name = call
        .tool
        .rsplit("__")
        .next()
        .unwrap_or(&call.tool)
        .replace(['_', '-'], " ");
    let server = call
        .tool
        .strip_prefix("mcp__")
        .and_then(|rest| rest.split("__").next())
        .map(|s| s.trim_start_matches("claude_ai_").replace('_', " "));
    // Meow's own tools name their app first: `slack send message` is Slack's.
    let server = match server {
        Some(server) if server == context::SERVER_NAME => name.split(' ').next().map(capitalize),
        other => other,
    };
    let what = match server {
        // `Slack: slack send message` reads `Slack: send message`.
        Some(server) => {
            let prefix = format!("{} ", server.to_lowercase());
            let name = name.strip_prefix(&prefix).unwrap_or(&name);
            format!("{server}: {name}")
        }
        None => name,
    };
    // The text it sends is what matters most; the rest as compact JSON.
    let body = ["message", "text", "body", "content", "description", "title"]
        .iter()
        .find_map(|key| text(key));
    let detail = match body {
        Some(body) => {
            let rest: serde_json::Map<String, Value> = input
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter(|(_, v)| !v.is_string() || v.as_str().is_some_and(|s| s.len() < 80))
                        .filter(|(_, v)| v.as_str() != Some(body.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
                .unwrap_or_default();
            if rest.is_empty() {
                body
            } else {
                format!("{}\n{body}", Value::Object(rest))
            }
        }
        None => input.to_string(),
    };
    (what, detail)
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(max).collect();
    clipped.push('…');
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plan() -> Plan {
        Plan {
            title: "Fix conflicts on #12".into(),
            prompt: "Resolve the merge conflicts of PR #12.".into(),
            repo: Some("o/r".into()),
            pr: Some("12".into()),
            branch: None,
            hard: None,
        }
    }

    fn origin(ts: &str) -> Origin {
        Origin {
            channel: "D1".into(),
            ts: ts.into(),
            thread_ts: None,
            from: "Axel".into(),
            text: "Ta PR #12 a des conflits".into(),
        }
    }

    fn push() -> Call {
        Call {
            tool: "Bash".into(),
            input: json!({ "command": "git push fork HEAD:feat/x", "description": "Push the fix" }),
        }
    }

    #[test]
    fn plans_come_from_the_fence() {
        let answer = "```meow-autopilot\n{\"title\": \" Fix conflicts \", \"prompt\": \"Do it\", \
                      \"repo\": \"o/r.git\", \"pr\": \"https://github.com/o/r/pull/12\", \"branch\": null}\n```";
        let plan = parse_plan(answer).unwrap();
        assert_eq!(plan.title, "Fix conflicts");
        assert_eq!(plan.repo.as_deref(), Some("o/r"));
        assert_eq!(plan.branch, None);
        assert_eq!(parse_plan("none"), None);
        assert_eq!(
            parse_plan("```meow-autopilot\n{\"title\": \"\", \"prompt\": \"x\"}\n```"),
            None
        );
    }

    #[test]
    fn plans_quoting_code_from_the_message_are_read_whole() {
        let answer = "```meow-autopilot\n{\"title\": \"Answer Julien\", \"prompt\": \"He pasted:\\n\
                      ```kotlin\\nfun confirmOrder()\\n```\\nExplain it.\", \"hard\": false}\n```";
        let plan = parse_plan(answer).unwrap();
        assert!(plan.prompt.ends_with("```\nExplain it."), "{}", plan.prompt);
        assert_eq!(plan.hard, Some(false));
        let loose = |hard: &str| {
            let answer = format!(
                "```meow-autopilot\n{{\"title\": \"t\", \"prompt\": \"p\", \"hard\": {hard}}}\n```"
            );
            parse_plan(&answer).map(|p| p.hard)
        };
        assert_eq!(loose("\"true\""), Some(Some(true)));
        assert_eq!(loose("\"maybe\""), Some(None));
        assert_eq!(loose("null"), Some(None));
    }

    #[test]
    fn the_triage_says_why_it_made_no_job() {
        assert_eq!(read_triage("none"), Err("no work to prepare"));
        assert_eq!(
            read_triage("```meow-autopilot\n{\"title\": \"x\", \"prompt\": \"cut ```\n"),
            Err("its answer couldn't be read")
        );
        let answer = "```meow-autopilot\n{\"title\": \"x\", \"prompt\": \"y\"}\n```";
        assert_eq!(read_triage(answer).map(|p| p.title), Ok("x".to_string()));
    }

    #[test]
    fn hard_work_runs_on_the_strong_model() {
        let mut jobs = Jobs::default();
        let light = Plan {
            hard: Some(false),
            ..plan()
        };
        let hard = Plan {
            hard: Some(true),
            repo: None,
            pr: None,
            ..plan()
        };
        let unsaid = Plan {
            hard: None,
            ..plan()
        };
        let answer = Plan {
            hard: None,
            repo: None,
            pr: None,
            ..plan()
        };
        let ids: Vec<u64> = [light, hard, unsaid, answer]
            .into_iter()
            .enumerate()
            .map(|(i, p)| jobs.add(p, origin(&i.to_string()), 0).id)
            .collect();
        let models: Vec<&str> = ids
            .iter()
            .map(|id| jobs.get(*id).unwrap().model())
            .collect();
        // Unsaid, a job with a repository or a pull request is code work.
        assert_eq!(models, ["sonnet", "opus", "opus", "sonnet"]);
        let saved_before = serde_json::json!({
            "id": 9, "title": "t", "origin": origin("9"), "plan_prompt": "p",
            "repo": "o/r", "created_at": 0, "stage": "queued",
        });
        let old: Job = serde_json::from_value(saved_before).unwrap();
        assert_eq!(
            old.model(),
            "opus",
            "saved before jobs said whether they were hard"
        );
        assert!(triage_prompt("Axel", "Julien", "Julien: hi", None).contains("\"hard\""));
    }

    #[test]
    fn jobs_restarted_from_scratch_forget_what_their_run_held() {
        let mut jobs = Jobs::default();
        let id = jobs.add(plan(), origin("1"), 0).id;
        jobs.get_mut(id).unwrap().stage = Stage::Working;
        jobs.hold(id, vec![push()]);
        jobs.set_action(id, 0, ActionState::Approved, None);
        jobs.after_restart();
        let job = jobs.get(id).unwrap();
        assert_eq!(job.stage, Stage::Queued);
        assert!(job.actions.is_empty(), "its session is gone with the run");
        assert_eq!(jobs.next_approved(), None);
    }

    #[test]
    fn running_jobs_show_their_last_steps_until_they_end() {
        let mut jobs = Jobs::default();
        let id = jobs.add(plan(), origin("1"), 0).id;
        for step in [
            "Read: a.rs",
            "Bash: Run tests",
            "Edit: a.rs",
            "Bash: Commit",
        ] {
            jobs.step(id, step);
        }
        assert_eq!(
            jobs.get(id).unwrap().steps,
            ["Bash: Run tests", "Edit: a.rs", "Bash: Commit"]
        );
        jobs.finish(id, Ok("Done.".into()), vec![], 1);
        assert!(jobs.get(id).unwrap().steps.is_empty());
    }

    #[test]
    fn calls_held_while_working_wait_at_once_and_only_once() {
        let mut jobs = Jobs::default();
        let id = jobs.add(plan(), origin("1"), 0).id;
        jobs.get_mut(id).unwrap().stage = Stage::Working;
        assert!(jobs.hold(id, vec![push()]));
        assert!(!jobs.hold(id, vec![push()]), "already waiting");
        assert_eq!(jobs.waiting_count(), 1);
        jobs.finish(id, Ok("Pushed later.".into()), vec![push()], 1);
        assert_eq!(jobs.get(id).unwrap().actions.len(), 1);
    }

    #[test]
    fn messages_left_alone_are_listed_newest_first_and_capped() {
        let mut jobs = Jobs::default();
        for i in 0..MAX_LEFT_ALONE + 2 {
            jobs.leave_alone(origin(&i.to_string()), "no work found".into(), i as u64);
        }
        let left = jobs.left_alone();
        assert_eq!(left.len(), MAX_LEFT_ALONE);
        assert_eq!(left[0].at, (MAX_LEFT_ALONE + 1) as u64);
        assert_eq!(left[0].why, "no work found");
        assert_eq!(left[0].from, "Axel");
    }

    #[test]
    fn jobs_run_in_order_and_keep_held_calls_once() {
        let mut jobs = Jobs::default();
        jobs.add(plan(), origin("1"), 10);
        jobs.add(plan(), origin("2"), 20);
        assert!(jobs.has_origin("D1", "1"));
        let first = jobs.next_queued().unwrap().id;
        assert_eq!(first, 1);
        jobs.get_mut(first).unwrap().stage = Stage::Working;
        jobs.finish(first, Ok("Merged main.".into()), vec![push(), push()], 30);
        let job = jobs.get(first).unwrap();
        assert_eq!(job.stage, Stage::Finished);
        assert_eq!(job.actions.len(), 1);
        assert_eq!(jobs.waiting_count(), 1);
        assert_eq!(jobs.next_queued().unwrap().id, 2);

        jobs.set_action(first, 0, ActionState::Approved, None);
        assert_eq!(jobs.next_approved(), Some((first, 0)));
        jobs.set_action(first, 0, ActionState::Done, Some("Pushed.".into()));
        assert_eq!(jobs.waiting_count(), 0);
        assert!(!jobs.get(first).unwrap().is_open());
    }

    #[test]
    fn prune_keeps_open_jobs_and_restarts_interrupted_ones() {
        let mut jobs = Jobs::default();
        jobs.add(plan(), origin("1"), 0);
        jobs.add(plan(), origin("2"), 0);
        jobs.finish(1, Ok("done".into()), vec![], 0);
        jobs.finish(2, Ok("done".into()), vec![push()], 0);
        let gone = jobs.prune(KEEP_SECS + 1);
        assert_eq!(gone.iter().map(|j| j.id).collect::<Vec<_>>(), vec![1]);
        assert_eq!(jobs.all().len(), 1, "an action still waits on job 2");

        jobs.add(plan(), origin("3"), 0);
        jobs.get_mut(3).unwrap().stage = Stage::Working;
        jobs.set_action(2, 0, ActionState::Running, None);
        jobs.after_restart();
        assert_eq!(jobs.get(3).unwrap().stage, Stage::Queued);
        assert_eq!(jobs.get(2).unwrap().actions[0].state, ActionState::Pending);
    }

    #[test]
    fn the_work_prompt_says_where_and_how_to_push() {
        let mut jobs = Jobs::default();
        let id = jobs.add(plan(), origin("1"), 0).id;
        let job = jobs.get_mut(id).unwrap();
        job.folder = Some(Folder {
            path: "/w".into(),
            clone: "/c".into(),
            remote: "fork".into(),
            branch: "feat/x".into(),
            base: Some("main".into()),
            pr_url: Some("https://github.com/o/r/pull/12".into()),
        });
        let prompt = work_prompt(job, "Cyril", None);
        assert!(prompt.starts_with("Resolve the merge conflicts"));
        assert!(prompt.contains("`git push fork HEAD:feat/x`"), "{prompt}");
        assert!(prompt.contains("merges into `main`"));
        assert!(prompt.contains("from Axel on Slack: \"Ta PR #12 a des conflits\""));
        assert!(
            prompt.contains("with `mcp__meow__slack_send_message` (channel `D1`)"),
            "{prompt}"
        );
    }

    #[test]
    fn connector_sends_held_before_go_through_meow_after_a_restart() {
        let mut jobs = Jobs::default();
        jobs.add(plan(), origin("1"), 0);
        let old = Call {
            tool: "mcp__claude_ai_Slack__slack_send_message".into(),
            input: json!({ "channel_id": "D1", "message": "Fait", "thread_ts": "0.5" }),
        };
        jobs.finish(1, Ok("Done".into()), vec![old, push()], 0);
        jobs.after_restart();
        let actions = &jobs.get(1).unwrap().actions;
        assert_eq!(actions[0].call.tool, "mcp__meow__slack_send_message");
        assert_eq!(
            actions[0].call.input,
            json!({ "channel": "D1", "text": "Fait", "thread_ts": "0.5" })
        );
        assert_eq!(actions[1].call, push(), "other calls stay as they are");
    }

    #[test]
    fn replies_go_in_the_thread_or_start_one_in_a_channel() {
        let dm = origin("1.0");
        assert_eq!(reply_args(&dm), "channel `D1`");
        let in_thread = Origin {
            thread_ts: Some("0.5".into()),
            ..origin("1.0")
        };
        assert_eq!(reply_args(&in_thread), "channel `D1`, thread_ts `0.5`");
        let mention = Origin {
            channel: "C9".into(),
            ..origin("1.0")
        };
        assert_eq!(reply_args(&mention), "channel `C9`, thread_ts `1.0`");
    }

    #[test]
    fn held_calls_read_as_what_they_do() {
        assert_eq!(
            describe(&push()),
            (
                "Push the fix".to_string(),
                "git push fork HEAD:feat/x".to_string()
            )
        );
        let send = Call {
            tool: "mcp__meow__slack_send_message".into(),
            input: json!({ "channel": "D1", "text": "C'est réglé, je push." }),
        };
        let (what, detail) = describe(&send);
        assert_eq!(what, "Slack: send message");
        assert_eq!(detail, "{\"channel\":\"D1\"}\nC'est réglé, je push.");
        let connector = Call {
            tool: "mcp__claude_ai_Linear__save_comment".into(),
            input: json!({ "issueId": "ACC-1", "body": "Fixed" }),
        };
        assert_eq!(describe(&connector).0, "Linear: save comment");
    }

    #[test]
    fn jobs_without_a_worktree_still_run_and_say_why() {
        let mut jobs = Jobs::default();
        jobs.add(plan(), origin("1"), 0);
        let job = jobs.get_mut(1).unwrap();
        assert!(job.needs_folder());
        assert!(!work_prompt(job, "Axel", None).contains("could not make a git worktree"));

        job.folder_error = Some("No clone of o/r found".into());
        assert!(!job.needs_folder());
        let prompt = work_prompt(job, "Axel", None);
        assert!(prompt.contains("could not make a git worktree for this work (No clone of o/r"));
        assert!(prompt.contains("--repo owner/name"));
    }

    #[test]
    fn feedback_is_kept_once_on_one_line() {
        let feedback = add_feedback(&[], "  Open the PR,\n\ndon't   message on Slack. ");
        assert_eq!(feedback, ["Open the PR, don't message on Slack."]);
        assert_eq!(add_feedback(&feedback, "   \n"), feedback);
        assert_eq!(
            add_feedback(&feedback, "Open the PR, don't message on Slack."),
            feedback
        );
        let long = add_feedback(&[], &"a".repeat(MAX_FEEDBACK_CHARS + 10));
        assert_eq!(long[0].chars().count(), MAX_FEEDBACK_CHARS + 1);
    }

    #[test]
    fn rules_join_the_users_file_and_feedback() {
        assert_eq!(rules(None, &[]), None);
        assert_eq!(
            rules(Some("Only for messages from Axel"), &[]).as_deref(),
            Some("Only for messages from Axel")
        );
        let feedback = ["Only the PR, no Slack message".to_string()];
        assert_eq!(
            rules(None, &feedback).as_deref(),
            Some("- Only the PR, no Slack message")
        );
        assert_eq!(
            rules(Some("Only for Axel"), &feedback).as_deref(),
            Some("Only for Axel\n- Only the PR, no Slack message")
        );
    }

    #[test]
    fn the_work_prompt_follows_the_users_rules() {
        let mut jobs = Jobs::default();
        let id = jobs.add(plan(), origin("1"), 0).id;
        let job = jobs.get(id).unwrap();
        assert!(!work_prompt(job, "Axel", None).contains("Axel's rules"));
        let prompt = work_prompt(job, "Axel", Some("- Only the PR, no Slack message"));
        assert!(
            prompt.contains("Axel's rules and feedback on earlier work, which win over"),
            "{prompt}"
        );
        assert!(prompt.contains("- Only the PR, no Slack message\n\nEnd with a short summary"));
    }

    #[test]
    fn jobs_round_trip() {
        let dir = std::env::temp_dir().join(format!("meow-autopilot-{}", std::process::id()));
        let path = dir.join("autopilot.json");
        let mut jobs = Jobs::default();
        jobs.add(plan(), origin("1"), 5);
        jobs.finish(1, Err("no clone".into()), vec![push()], 6);
        jobs.leave_alone(origin("2"), "no work found".into(), 7);
        jobs.save(&path).unwrap();
        assert_eq!(Jobs::load(&path).unwrap(), jobs);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
