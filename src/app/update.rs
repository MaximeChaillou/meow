//! Updates on the app side: every few hours, or from the cat's menu, a check
//! for new commits on `main`, built and installed in the background; then
//! Meow restarts into the new version once the user is away and nothing is
//! running. The cat's menu shows how it is going, and Activity what came in.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use meow::activity::{self, Kind, Link, Note};
use meow::update::{self, BRANCH, Build, Progress, Update};
use objc2::rc::Retained;
use objc2_app_kit::NSMenuItem;
use objc2_foundation::{NSBundle, NSNumber, NSString};

use crate::ui::{menus, notify};

use super::App;

const DEFAULT_HOURS: u64 = 6;
/// The first check waits a little after launch, so startup stays quick.
const FIRST_DELAY: Duration = Duration::from_secs(2 * 60);
/// An update found by itself restarts Meow only once the chat has been closed
/// this long, so a conversation in progress is not cut short.
const QUIET_BEFORE_RESTART: Duration = Duration::from_secs(10 * 60);

enum Stage {
    Idle,
    Checking,
    Building(Update),
    /// Installed; Meow restarts into it at the next quiet moment.
    Installed,
}

pub(super) struct UpdateState {
    build: Build,
    /// The running Meow.app; `None` for a bare `cargo run` binary.
    app: Option<PathBuf>,
    /// `None` when checks are turned off (`MEOW_UPDATE_HOURS=0`), or this
    /// build never updates.
    interval: Option<Duration>,
    next_at: Instant,
    job: Option<Receiver<Progress>>,
    stage: Stage,
    /// Asked for from the menu: says how the check went, and restarts without
    /// waiting for the user to be away.
    manual: bool,
    /// The last commit whose failed build was reported, so a retry that fails
    /// the same way stays quiet.
    reported_failure: Option<String>,
    /// When the chat was last on screen.
    chat_seen_at: Instant,
    /// The cat menu's item that checks, or restarts into an installed update.
    item: Retained<NSMenuItem>,
}

impl UpdateState {
    /// Reads `MEOW_UPDATE_HOURS` (default 6, 0 turns the checks off).
    pub(super) fn new(build: Build, app: Option<PathBuf>, item: Retained<NSMenuItem>) -> Self {
        let hours = std::env::var("MEOW_UPDATE_HOURS")
            .ok()
            .and_then(|h| h.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_HOURS);
        let updates = app.is_some() && build.source().is_ok();
        Self {
            build,
            app,
            interval: (updates && hours > 0).then(|| Duration::from_secs(hours * 60 * 60)),
            next_at: Instant::now() + FIRST_DELAY,
            job: None,
            stage: Stage::Idle,
            manual: false,
            reported_failure: None,
            chat_seen_at: Instant::now(),
            item,
        }
    }
}

/// What the running app was built from, as its Info.plist says, and where
/// its bundle is. A bare `cargo run` binary has neither.
pub(super) fn running_build() -> (Build, Option<PathBuf>) {
    let bundle = NSBundle::mainBundle();
    let value = |key: &str| bundle.objectForInfoDictionaryKey(&NSString::from_str(key));
    let text = |key: &str| {
        value(key)
            .and_then(|value| value.downcast::<NSString>().ok())
            .map(|text| text.to_string())
            .filter(|text| !text.is_empty())
    };
    let build = Build {
        version: text("CFBundleShortVersionString")
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        commit: text("MeowCommit"),
        remote: text("MeowRemote"),
        modified: value("MeowModified")
            .and_then(|value| value.downcast::<NSNumber>().ok())
            .is_some_and(|flag| flag.as_bool()),
    };
    let app = bundle
        .bundleIdentifier()
        .map(|_| PathBuf::from(bundle.bundlePath().to_string()));
    (build, app)
}

impl App {
    /// Starts a check when one is due, follows the one in flight, and
    /// restarts into an installed update when the moment is right.
    pub(super) fn poll_update(&mut self) {
        if self.chat.is_visible() {
            self.update.chat_seen_at = Instant::now();
        }
        while let Some(progress) = self.next_update_progress() {
            self.on_update_progress(progress);
        }
        match self.update.stage {
            Stage::Idle if self.update_due() => self.start_update(false),
            Stage::Installed if self.ready_to_restart() => self.restart_to_update(),
            _ => {}
        }
    }

    /// The menu's update item: checks now, or restarts into the installed update.
    pub(super) fn update_from_menu(&mut self) {
        match self.update.stage {
            Stage::Idle => self.start_update(true),
            Stage::Installed => self.restart_to_update(),
            Stage::Checking | Stage::Building(_) => {}
        }
    }

    fn update_due(&self) -> bool {
        self.update.interval.is_some() && Instant::now() >= self.update.next_at
    }

    fn start_update(&mut self, manual: bool) {
        if let Some(interval) = self.update.interval {
            self.update.next_at = Instant::now() + interval;
        }
        let state = &self.update;
        let job = match (&state.app, state.build.source(), update::default_dir()) {
            (_, Err(reason), _) => Err(reason),
            (Some(app), Ok((commit, remote)), Some(dir)) => Ok(update::start(
                dir,
                remote.to_string(),
                commit.to_string(),
                app.clone(),
            )),
            _ => Err("Only Meow.app built by scripts/bundle.sh updates itself."),
        };
        match job {
            Ok(job) => {
                self.update.job = Some(job);
                self.update.manual = manual;
                self.set_update_stage(Stage::Checking);
            }
            Err(reason) if manual => notify::post("Meow can't update itself", reason),
            Err(_) => {}
        }
    }

    fn next_update_progress(&mut self) -> Option<Progress> {
        let job = self.update.job.as_ref()?;
        match job.try_recv() {
            Ok(progress) => Some(progress),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.update.job = None;
                // Its last word always ends the update; without one it crashed.
                matches!(self.update.stage, Stage::Checking | Stage::Building(_))
                    .then(|| Progress::Failed("The update stopped early.".to_string()))
            }
        }
    }

    fn on_update_progress(&mut self, progress: Progress) {
        let manual = self.update.manual;
        match progress {
            Progress::UpToDate => {
                if manual {
                    notify::post("Meow is up to date", &self.update.build.label());
                }
                self.set_update_stage(Stage::Idle);
            }
            Progress::NotOnBranch => {
                if manual {
                    let body = format!(
                        "This build isn't on {BRANCH}. Install from {BRANCH} with \
                         scripts/bundle.sh --install to get updates."
                    );
                    notify::post("Meow wasn't updated", &body);
                }
                self.set_update_stage(Stage::Idle);
            }
            Progress::Building(update) => {
                if manual {
                    let body = format!(
                        "Building {}. Meow restarts when it's ready.",
                        count(update.changes.len(), "new change")
                    );
                    notify::post("Updating Meow", &body);
                }
                self.set_update_stage(Stage::Building(update));
            }
            Progress::Installed(update) => {
                self.notify(installed_note(&update, self.update.build.remote.as_deref()));
                self.set_update_stage(Stage::Installed);
            }
            Progress::Failed(error) => {
                eprintln!("meow: update failed: {error}");
                self.report_update_failure(error, manual);
                self.set_update_stage(Stage::Idle);
            }
        }
    }

    /// A failed build goes to Activity, once per commit; a check that could
    /// not reach the repository is only worth a word when it was asked for.
    fn report_update_failure(&mut self, error: String, manual: bool) {
        let building = match &self.update.stage {
            Stage::Building(update) => Some(update.commit.clone()),
            _ => None,
        };
        match building {
            Some(commit) if manual || self.update.reported_failure.as_ref() != Some(&commit) => {
                self.update.reported_failure = Some(commit);
                self.notify(Note {
                    kind: Kind::Update,
                    title: "Meow could not update".to_string(),
                    detail: Some(error),
                    link: None,
                });
            }
            Some(_) => {}
            None if manual => notify::post("Meow could not check for updates", &error),
            None => {}
        }
    }

    fn set_update_stage(&mut self, stage: Stage) {
        let (title, enabled) = match &stage {
            Stage::Idle => ("Check for Updates", true),
            Stage::Checking => ("Checking for Updates…", false),
            Stage::Building(_) => ("Building the Update…", false),
            Stage::Installed => ("Restart to Update", true),
        };
        menus::show_update(&self.update.item, title, enabled);
        if matches!(stage, Stage::Idle) {
            self.update.manual = false;
        }
        self.update.stage = stage;
    }

    /// The chat is closed (for a while, unless the update was asked for), the
    /// cat is not in the user's hands, and nothing a restart would cut short
    /// is running.
    fn ready_to_restart(&self) -> bool {
        let quiet = if self.update.manual {
            Duration::ZERO
        } else {
            QUIET_BEFORE_RESTART
        };
        !self.chat.is_visible()
            && self.update.chat_seen_at.elapsed() >= quiet
            && !self.cat.is_held()
            && !self.busy()
    }

    fn busy(&self) -> bool {
        self.run.is_some()
            || self.sync.busy()
            || self.agenda.busy()
            || self.rsvp.busy()
            || self.routines.running().is_some()
            || self.slack_assist.run.is_some()
            || self.voice.busy()
            || self.agents.busy()
    }

    fn restart_to_update(&mut self) {
        let Some(app) = self.update.app.clone() else {
            return;
        };
        match update::relaunch_after_exit(&app) {
            Ok(()) => self.quit(),
            Err(error) => {
                eprintln!("meow: {error}");
                // Tried again after the next quiet spell.
                self.update.manual = false;
                self.update.chat_seen_at = Instant::now();
            }
        }
    }
}

/// "Meow updated to 1a2b3c4", with what changed, linking to the commits.
fn installed_note(update: &Update, remote: Option<&str>) -> Note {
    let changes: Vec<&str> = update.changes.iter().map(String::as_str).collect();
    Note {
        kind: Kind::Update,
        title: format!("Meow updated to {}", update::short(&update.commit)),
        detail: (!changes.is_empty()).then(|| activity::list_titles(&changes, 3)),
        link: remote
            .and_then(|remote| update::compare_url(remote, &update.from, &update.commit))
            .map(|url| Link::Url { url }),
    }
}

fn count(n: usize, what: &str) -> String {
    match n {
        1 => format!("1 {what}"),
        n => format!("{n} {what}s"),
    }
}
