//! Keeps the installed app current with [`BRANCH`].
//!
//! Meow is built from source and signed with this Mac's own "Meow Dev"
//! identity, which the Keychain trusts, so updates are built here too rather
//! than downloaded: a private clone of the repository the app was built from
//! follows [`BRANCH`]; when it moves past the running build,
//! `scripts/bundle.sh` builds the new app in the clone and it takes the
//! installed one's place. The app restarts into it later (`src/app/update.rs`).

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use crate::claude;
use crate::superset::{Output, run_with_timeout};

/// The branch installs follow.
pub const BRANCH: &str = "main";
/// Where `scripts/bundle.sh` leaves the app, in the clone.
const BUILT_APP: &str = "target/release/Meow.app";
/// The build's whole output, kept for when it fails.
const BUILD_LOG: &str = "target/update-build.log";
const GIT_TIMEOUT: Duration = Duration::from_secs(120);
/// The first build in a fresh clone compiles every crate.
const BUILD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Waits for the process `$1` to exit, then opens the app `$2`.
const RELAUNCH_SCRIPT: &str =
    r#"while /bin/kill -0 "$1" 2>/dev/null; do /bin/sleep 0.2; done; exec /usr/bin/open "$2""#;

/// What the running app was built from, as `scripts/bundle.sh` wrote it in
/// its Info.plist.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Build {
    pub version: String,
    pub commit: Option<String>,
    /// The `origin` of the checkout it was built in.
    pub remote: Option<String>,
    /// Built with uncommitted changes.
    pub modified: bool,
}

impl Build {
    /// "Meow 0.1.0 (b1a7899)", for the cat's menu.
    pub fn label(&self) -> String {
        match &self.commit {
            Some(commit) if self.modified => {
                format!("Meow {} ({}, modified)", self.version, short(commit))
            }
            Some(commit) => format!("Meow {} ({})", self.version, short(commit)),
            None => format!("Meow {}", self.version),
        }
    }

    /// The commit and remote it updates from, or why it is never replaced.
    pub fn source(&self) -> Result<(&str, &str), &'static str> {
        let (Some(commit), Some(remote)) = (&self.commit, &self.remote) else {
            return Err(
                "Only Meow.app built by scripts/bundle.sh from a git clone updates itself.",
            );
        };
        if self.modified {
            return Err("This build has uncommitted changes, so it is left as it is.");
        }
        Ok((commit, remote))
    }
}

/// Commits on [`BRANCH`] newer than the running build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    /// The running build's commit.
    pub from: String,
    /// The newest commit, the one built.
    pub commit: String,
    /// Their subjects, newest first, merges left out.
    pub changes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    UpToDate,
    /// The running build is not on [`BRANCH`] (built from another branch, or
    /// with commits not pushed): it is left alone.
    NotOnBranch,
    Available(Update),
}

/// How an update goes, step by step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    UpToDate,
    NotOnBranch,
    /// New commits found; building them.
    Building(Update),
    /// The new app took the installed one's place; it runs from the next launch.
    Installed(Update),
    Failed(String),
}

/// On a thread: checks `remote` for commits on [`BRANCH`] newer than
/// `built`, and if there are, builds them in the clone at `dir` and installs
/// the new app over `app`.
pub fn start(dir: PathBuf, remote: String, built: String, app: PathBuf) -> Receiver<Progress> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let updater = Updater {
            dir,
            remote,
            path_var: claude::shell_path(),
        };
        let last = match updater.check(&built) {
            Ok(Check::UpToDate) => Progress::UpToDate,
            Ok(Check::NotOnBranch) => Progress::NotOnBranch,
            Ok(Check::Available(update)) => {
                let _ = tx.send(Progress::Building(update.clone()));
                match updater
                    .build(&update.commit)
                    .and_then(|new_app| install(&new_app, &app))
                {
                    Ok(()) => Progress::Installed(update),
                    Err(error) => Progress::Failed(error),
                }
            }
            Err(error) => Progress::Failed(error),
        };
        let _ = tx.send(last);
    });
    rx
}

/// The clone updates are built in.
pub struct Updater {
    pub dir: PathBuf,
    /// The repository it follows.
    pub remote: String,
    /// The user's PATH, where `git` and `cargo` are.
    pub path_var: String,
}

impl Updater {
    /// Fetches [`BRANCH`] and compares it with the running build's commit.
    pub fn check(&self, built: &str) -> Result<Check, String> {
        self.fetch()?;
        let branch = format!("refs/remotes/origin/{BRANCH}^{{commit}}");
        let latest = self.git(&["rev-parse", "--verify", "--quiet", &branch])?;
        if latest == built {
            return Ok(Check::UpToDate);
        }
        // Fails too for a commit the clone does not have.
        if !self.git_succeeds(&["merge-base", "--is-ancestor", built, &latest])? {
            return Ok(Check::NotOnBranch);
        }
        let log = self.git(&[
            "log",
            "--no-merges",
            "--format=%s",
            &format!("{built}..{latest}"),
        ])?;
        Ok(Check::Available(Update {
            from: built.to_string(),
            changes: log.lines().map(str::to_string).collect(),
            commit: latest,
        }))
    }

    /// Builds `commit` with `scripts/bundle.sh`, at low priority, and returns
    /// the new app.
    pub fn build(&self, commit: &str) -> Result<PathBuf, String> {
        self.git(&["checkout", "--quiet", "--force", "--detach", commit])?;
        let mut command = self.command("/usr/bin/nice");
        command
            .args(["-n", "10", "/bin/sh", "scripts/bundle.sh"])
            .current_dir(&self.dir);
        let output = run_with_timeout(command, BUILD_TIMEOUT)
            .map_err(|e| format!("The build did not finish: {e}"))?;
        let log = self.dir.join(BUILD_LOG);
        if let Some(dir) = log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&log, format!("{}{}", output.stdout, output.stderr));
        if !output.success {
            return Err(format!(
                "The build failed ({}). Its log is in {}",
                last_line(&output.stderr),
                log.display()
            ));
        }
        Ok(self.dir.join(BUILT_APP))
    }

    /// Clones the repository the first time, then brings [`BRANCH`] up to date.
    fn fetch(&self) -> Result<(), String> {
        if self.dir.join(".git").is_dir() {
            // The app may have been rebuilt from a clone with another remote.
            self.git(&["remote", "set-url", "origin", &self.remote])?;
        } else {
            // Only ever a clone of ours, left half-made.
            let _ = std::fs::remove_dir_all(&self.dir);
            if let Some(parent) = self.dir.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
            }
            let mut clone = self.command("git");
            clone
                .args(["clone", "--quiet", "--no-checkout", &self.remote])
                .arg(&self.dir);
            checked(run_with_timeout(clone, GIT_TIMEOUT), "git clone")?;
        }
        let refspec = format!("+refs/heads/{BRANCH}:refs/remotes/origin/{BRANCH}");
        self.git(&["fetch", "--quiet", "origin", &refspec])
            .map(drop)
    }

    /// Runs git in the clone; its output, trimmed.
    fn git(&self, args: &[&str]) -> Result<String, String> {
        let what = format!("git {}", args.first().copied().unwrap_or_default());
        checked(run_with_timeout(self.git_command(args), GIT_TIMEOUT), &what)
    }

    /// Runs git in the clone; whether it succeeded.
    fn git_succeeds(&self, args: &[&str]) -> Result<bool, String> {
        run_with_timeout(self.git_command(args), GIT_TIMEOUT)
            .map(|output| output.success)
            .map_err(|e| format!("git {}: {e}", args.first().copied().unwrap_or_default()))
    }

    fn git_command(&self, args: &[&str]) -> Command {
        let mut command = self.command("git");
        command.arg("-C").arg(&self.dir).args(args);
        command
    }

    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .env("PATH", &self.path_var)
            // Fail rather than wait for a password nobody will type.
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
}

/// `what`'s output, trimmed, or what went wrong.
fn checked(output: Result<Output, String>, what: &str) -> Result<String, String> {
    let output = output.map_err(|e| format!("{what}: {e}"))?;
    if output.success {
        Ok(output.stdout.trim().to_string())
    } else {
        Err(format!("{what} failed: {}", last_line(&output.stderr)))
    }
}

/// The last line with something on it, where commands say what went wrong.
fn last_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("no details")
}

/// Puts `new_app` where `app` is. The copy is made next to it and the two are
/// swapped in one step, so `app` is never missing or half-copied; the old one
/// is then removed. A running app keeps the files it has open until it exits.
pub fn install(new_app: &Path, app: &Path) -> Result<(), String> {
    let fail = |e: &dyn std::fmt::Display| {
        format!("Could not install the update in {}: {e}", app.display())
    };
    let name = app.file_name().ok_or_else(|| fail(&"not an app"))?;
    let staged = app.with_file_name(format!(".{}.update", name.to_string_lossy()));
    let _ = std::fs::remove_dir_all(&staged);
    let copy = Command::new("/usr/bin/ditto")
        .arg(new_app)
        .arg(&staged)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| fail(&e))?;
    if !copy.status.success() {
        return Err(fail(&String::from_utf8_lossy(&copy.stderr).trim()));
    }
    swap(&staged, app).map_err(|e| fail(&e))?;
    // The old app now; a leftover is removed by the next update.
    let _ = std::fs::remove_dir_all(&staged);
    Ok(())
}

/// Exchanges what two paths name, atomically.
fn swap(a: &Path, b: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c_path =
        |path: &Path| CString::new(path.as_os_str().as_bytes()).map_err(std::io::Error::other);
    let (a, b) = (c_path(a)?, c_path(b)?);
    // SAFETY: both are NUL-terminated paths that outlive the call.
    match unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) } {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

/// Opens `app` again as soon as this process has exited.
pub fn relaunch_after_exit(app: &Path) -> Result<(), String> {
    Command::new("/bin/sh")
        .args(["-c", RELAUNCH_SCRIPT, "sh", &std::process::id().to_string()])
        .arg(app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own group, so nothing aimed at Meow's reaches it.
        .process_group(0)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Could not restart Meow: {e}"))
}

/// The page listing the commits between `from` and `to`, for a GitHub remote.
pub fn compare_url(remote: &str, from: &str, to: &str) -> Option<String> {
    // https://github.com/o/r(.git), git@github.com:o/r(.git), ssh://git@github.com/o/r
    let (_, path) = remote.trim().split_once("github.com")?;
    let path = path.strip_prefix(['/', ':'])?.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some(format!(
        "https://github.com/{owner}/{repo}/compare/{}...{}",
        short(from),
        short(to)
    ))
}

/// The first 7 characters of a commit hash.
pub fn short(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

/// `~/Library/Caches/Meow/source`.
pub fn default_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches/Meow/source"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("meow-update-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Runs git in `dir` without the user's config getting in the way; its output.
    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(["-c", "user.name=Meow", "-c", "user.email=meow@example.com"])
            .args(["-c", "commit.gpgsign=false", "-C"])
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn commit(dir: &Path, subject: &str) -> String {
        git(dir, &["commit", "--quiet", "--allow-empty", "-m", subject]);
        git(dir, &["rev-parse", "HEAD"])
    }

    #[test]
    fn labels_name_the_version_and_commit() {
        let mut build = Build {
            version: "0.1.0".into(),
            ..Build::default()
        };
        assert_eq!(build.label(), "Meow 0.1.0");
        assert!(build.source().is_err(), "no commit, no updates");
        build.commit = Some("b1a7899c0ffee".into());
        build.remote = Some("https://github.com/o/r".into());
        assert_eq!(build.label(), "Meow 0.1.0 (b1a7899)");
        assert_eq!(
            build.source(),
            Ok(("b1a7899c0ffee", "https://github.com/o/r"))
        );
        build.modified = true;
        assert_eq!(build.label(), "Meow 0.1.0 (b1a7899, modified)");
        assert!(build.source().is_err(), "local changes are kept");
    }

    #[test]
    fn compare_links_only_for_github() {
        let page = Some("https://github.com/o/r/compare/aaaaaaa...bbbbbbb".to_string());
        for remote in [
            "https://github.com/o/r",
            "https://github.com/o/r.git",
            "https://token@github.com/o/r/",
            "git@github.com:o/r.git",
            "ssh://git@github.com/o/r",
        ] {
            assert_eq!(
                compare_url(remote, "aaaaaaa111", "bbbbbbb222"),
                page,
                "{remote}"
            );
        }
        assert_eq!(compare_url("https://gitlab.com/o/r", "a", "b"), None);
        assert_eq!(
            compare_url("https://github.com.evil.com/o/r", "a", "b"),
            None
        );
        assert_eq!(compare_url("https://github.com/o", "a", "b"), None);
    }

    #[test]
    fn checks_follow_the_branch() {
        let root = temp_dir("check");
        let origin = root.join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "--quiet", "-b", BRANCH]);
        let first = commit(&origin, "first");
        git(&origin, &["checkout", "--quiet", "-b", "side"]);
        commit(&origin, "side work");
        git(&origin, &["checkout", "--quiet", BRANCH]);
        git(
            &origin,
            &[
                "merge",
                "--quiet",
                "--no-ff",
                "--no-edit",
                "-m",
                "Merge side",
                "side",
            ],
        );
        let second = commit(&origin, "second");
        let third = commit(&origin, "third");
        let updater = Updater {
            dir: root.join("clone"),
            remote: origin.display().to_string(),
            path_var: std::env::var("PATH").unwrap(),
        };

        assert_eq!(
            updater.check(&first),
            Ok(Check::Available(Update {
                from: first.clone(),
                commit: third.clone(),
                changes: vec!["third".into(), "second".into(), "side work".into()],
            }))
        );
        assert_eq!(updater.check(&third), Ok(Check::UpToDate));

        git(&origin, &["checkout", "--quiet", "-b", "feature", &second]);
        let feature = commit(&origin, "feature");
        assert_eq!(updater.check(&feature), Ok(Check::NotOnBranch));

        // The clone made by the first check follows the branch as it moves.
        git(&origin, &["checkout", "--quiet", BRANCH]);
        let fourth = commit(&origin, "fourth");
        let Ok(Check::Available(update)) = updater.check(&third) else {
            panic!("the new commit is found");
        };
        assert_eq!(
            (update.commit, update.changes),
            (fourth, vec!["fourth".to_string()])
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failures_say_what_failed() {
        let root = temp_dir("fail");
        let updater = Updater {
            dir: root.join("clone"),
            remote: root.join("missing").display().to_string(),
            path_var: std::env::var("PATH").unwrap(),
        };
        let error = updater.check("abc").unwrap_err();
        assert!(error.starts_with("git clone failed: "), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn install_swaps_the_app_in() {
        let root = temp_dir("install");
        let app = root.join("Applications/Meow.app");
        let new_app = root.join("build/Meow.app");
        for (dir, version) in [(&app, "old"), (&new_app, "new")] {
            std::fs::create_dir_all(dir.join("Contents")).unwrap();
            std::fs::write(dir.join("Contents/version"), version).unwrap();
        }
        // Left by an update that was interrupted.
        std::fs::create_dir_all(root.join("Applications/.Meow.app.update")).unwrap();

        install(&new_app, &app).unwrap();
        assert_eq!(
            std::fs::read_to_string(app.join("Contents/version")).unwrap(),
            "new"
        );
        let left: Vec<_> = std::fs::read_dir(root.join("Applications"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["Meow.app"]);
        assert!(new_app.exists(), "the build stays for the next one");
        assert!(install(&new_app, &root.join("Missing.app")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn short_commits() {
        assert_eq!(short("b1a7899c0ffee"), "b1a7899");
        assert_eq!(short("abc"), "abc");
    }
}
