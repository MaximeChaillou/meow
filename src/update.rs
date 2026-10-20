//! Updates from the GitHub releases: each push to `main` publishes the app
//! under `commit-<hash>` and as `latest` (see .github/workflows/release.yml).
//!
//! A release build knows its commit (`MEOW_COMMIT`, set by the workflow). It
//! reads `latest`'s `version.json`; when the commit differs, it downloads the
//! zip, checks its SHA-256, unpacks it and stages it next to the installed
//! app. The app swaps the two and relaunches once nothing is going on.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

/// Where `latest` keeps its files; `MEOW_UPDATE_URL` points elsewhere (a
/// fork, or a local server to try an update).
pub const LATEST_URL: &str = "https://github.com/axellaffite/meow/releases/download/latest";
/// Checks run this often unless `MEOW_UPDATE_HOURS` says otherwise.
const DEFAULT_HOURS: u64 = 6;
const BUNDLE_ID: &str = "dev.meow.cat";
const EXECUTABLE: &str = "Contents/MacOS/meow";
/// Far above the app's few megabytes; a bigger download is not ours.
const MAX_ZIP_BYTES: u64 = 200 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);

/// What `version.json` says about a release.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    /// The full commit hash the release was built from.
    pub commit: String,
    /// Of `Meow.zip`, in lowercase hex.
    pub sha256: String,
}

impl Manifest {
    /// Reads `version.json`, refusing anything that does not look like one.
    pub fn parse(text: &str) -> Result<Self, String> {
        let manifest: Self =
            serde_json::from_str(text).map_err(|e| format!("bad version.json: {e}"))?;
        let hex = |s: &str, len: usize| s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit());
        if !hex(&manifest.commit, 40) || !hex(&manifest.sha256, 64) {
            return Err("bad version.json: commit or sha256 malformed".to_string());
        }
        Ok(Self {
            commit: manifest.commit.to_lowercase(),
            sha256: manifest.sha256.to_lowercase(),
        })
    }

    pub fn short(&self) -> &str {
        short(&self.commit)
    }
}

/// The commit this build comes from; `None` for local builds, which never update.
pub fn own_commit() -> Option<&'static str> {
    option_env!("MEOW_COMMIT").filter(|commit| !commit.trim().is_empty())
}

/// "90220bc" for a full hash.
pub fn short(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

/// Whether `latest` is another build than `own`.
pub fn is_update(own: &str, latest: &Manifest) -> bool {
    !own.trim().eq_ignore_ascii_case(&latest.commit)
}

/// From `MEOW_UPDATE_HOURS` (default 6); `None` turns checks off.
pub fn check_interval(hours: Option<&str>) -> Option<Duration> {
    let hours = hours
        .and_then(|h| h.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_HOURS);
    (hours > 0).then(|| Duration::from_secs(hours * 60 * 60))
}

/// The `.app` holding `exe`, when the app runs from a bundle.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let bundle = macos.parent()?.parent()?;
    let is_bundle =
        macos.ends_with("Contents/MacOS") && bundle.extension().is_some_and(|ext| ext == "app");
    is_bundle.then(|| bundle.to_path_buf())
}

/// Where a new version waits, beside the installed `bundle` so the swap is
/// a rename on the same volume.
pub fn staged_path(bundle: &Path) -> PathBuf {
    sibling(bundle, "update")
}

fn sibling(bundle: &Path, what: &str) -> PathBuf {
    let name = bundle
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Meow".to_string());
    bundle.with_file_name(format!(".{name}-{what}.app"))
}

/// Reads `latest`'s manifest.
pub fn fetch_latest() -> Result<Manifest, String> {
    let mut response = agent()
        .get(&format!("{}/version.json", latest_url()))
        .call()
        .map_err(|e| format!("update check failed: {e}"))?;
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("update check failed: {e}"))?;
    Manifest::parse(&text)
}

/// Downloads, checks and unpacks `latest` into [`staged_path`] of `bundle`,
/// using `work` (emptied first) for the download.
pub fn stage(latest: &Manifest, bundle: &Path, work: &Path) -> Result<PathBuf, String> {
    let fail = |what: &str, e: &dyn std::fmt::Display| format!("update: {what}: {e}");
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(work).map_err(|e| fail("cache folder", &e))?;
    let zip = work.join("Meow.zip");
    download(&format!("{}/Meow.zip", latest_url()), &zip)?;
    let sum = sha256(&zip)?;
    if sum != latest.sha256 {
        return Err(format!(
            "update: Meow.zip checksum {sum} is not {}",
            latest.sha256
        ));
    }
    let unpacked = work.join("unpacked");
    run(
        "/usr/bin/ditto",
        &["-x", "-k", path_str(&zip)?, path_str(&unpacked)?],
    )?;
    let app = unpacked.join("Meow.app");
    verify(&app)?;
    let staged = staged_path(bundle);
    let _ = std::fs::remove_dir_all(&staged);
    run("/usr/bin/ditto", &[path_str(&app)?, path_str(&staged)?])?;
    let _ = std::fs::remove_dir_all(work);
    Ok(staged)
}

/// Puts `staged` in place of `bundle`, putting the old one back on failure.
pub fn swap(staged: &Path, bundle: &Path) -> Result<(), String> {
    let old = sibling(bundle, "old");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(bundle, &old).map_err(|e| format!("update: move the old app aside: {e}"))?;
    if let Err(e) = std::fs::rename(staged, bundle) {
        let _ = std::fs::rename(&old, bundle);
        return Err(format!("update: move the new app in: {e}"));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Opens `bundle` again a moment after this process has quit.
pub fn relaunch(bundle: &Path) -> Result<(), String> {
    Command::new("/bin/sh")
        .args([
            "-c",
            "sleep 1; exec /usr/bin/open \"$0\"",
            path_str(bundle)?,
        ])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("update: relaunch: {e}"))
}

fn latest_url() -> String {
    std::env::var("MEOW_UPDATE_URL")
        .ok()
        .map(|url| url.trim().trim_end_matches('/').to_string())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| LATEST_URL.to_string())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .new_agent()
}

fn download(url: &str, file: &Path) -> Result<(), String> {
    let mut response = agent()
        .get(url)
        .call()
        .map_err(|e| format!("update download failed: {e}"))?;
    let mut out =
        std::fs::File::create(file).map_err(|e| format!("update: {}: {e}", file.display()))?;
    std::io::copy(
        &mut response.body_mut().as_reader().take(MAX_ZIP_BYTES),
        &mut out,
    )
    .map_err(|e| format!("update download failed: {e}"))?;
    Ok(())
}

fn sha256(file: &Path) -> Result<String, String> {
    let output = run("/usr/bin/shasum", &["-a", "256", path_str(file)?])?;
    output
        .split_whitespace()
        .next()
        .map(str::to_lowercase)
        .ok_or_else(|| "update: shasum printed nothing".to_string())
}

/// The unpacked app is a whole, validly signed Meow.
fn verify(app: &Path) -> Result<(), String> {
    if !app.join(EXECUTABLE).is_file() {
        return Err("update: the download holds no Meow.app".to_string());
    }
    let plist = app.join("Contents/Info.plist");
    let id = run(
        "/usr/libexec/PlistBuddy",
        &["-c", "Print :CFBundleIdentifier", path_str(&plist)?],
    )?;
    if id.trim() != BUNDLE_ID {
        return Err(format!("update: unexpected bundle id {}", id.trim()));
    }
    run(
        "/usr/bin/codesign",
        &["--verify", "--deep", "--strict", path_str(app)?],
    )
    .map(|_| ())
}

/// Runs `program`, returning what it printed; an error if it failed.
fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("update: {program}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("update: {program} failed: {}", stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("update: path is not UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "90220bc5d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6";
    const SUM: &str = "4f8b42c22dd3729b519ba6f68d2da7cc5b2d606d05daed5ad5128cc03e6c6358";

    #[test]
    fn manifests_must_carry_a_commit_and_a_checksum() {
        let text = format!(
            r#"{{"commit":"{}","sha256":"{SUM}","built_at":"x"}}"#,
            COMMIT.to_uppercase()
        );
        let manifest = Manifest::parse(&text).unwrap();
        assert_eq!(manifest.commit, COMMIT, "lowercased");
        assert_eq!(manifest.short(), "90220bc");
        assert!(Manifest::parse(r#"{"commit":"abc","sha256":"def"}"#).is_err());
        assert!(Manifest::parse("<html>Not Found</html>").is_err());
    }

    #[test]
    fn another_commit_is_an_update() {
        let latest = Manifest {
            commit: COMMIT.into(),
            sha256: SUM.into(),
        };
        assert!(!is_update(COMMIT, &latest));
        assert!(!is_update(&COMMIT.to_uppercase(), &latest));
        assert!(is_update(
            "35c510e2e91305a84afd157dd9fb1c8b28779fd1",
            &latest
        ));
    }

    #[test]
    fn checks_run_every_few_hours_unless_turned_off() {
        assert_eq!(check_interval(None), Some(Duration::from_secs(6 * 3600)));
        assert_eq!(check_interval(Some("1")), Some(Duration::from_secs(3600)));
        assert_eq!(check_interval(Some("0")), None);
        assert_eq!(
            check_interval(Some("soon")),
            Some(Duration::from_secs(6 * 3600))
        );
    }

    #[test]
    fn only_a_bundled_app_has_a_bundle_to_replace() {
        let exe = Path::new("/Applications/Meow.app/Contents/MacOS/meow");
        assert_eq!(
            bundle_of(exe),
            Some(PathBuf::from("/Applications/Meow.app"))
        );
        assert_eq!(bundle_of(Path::new("/src/meow/target/debug/meow")), None);
        assert_eq!(
            staged_path(Path::new("/Applications/Meow.app")),
            PathBuf::from("/Applications/.Meow-update.app")
        );
    }

    #[test]
    fn swapping_puts_the_new_app_in_place() {
        let dir = std::env::temp_dir().join(format!("meow-update-{}", std::process::id()));
        let bundle = dir.join("Meow.app");
        let staged = staged_path(&bundle);
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("v"), "old").unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(staged.join("v"), "new").unwrap();
        swap(&staged, &bundle).unwrap();
        assert_eq!(std::fs::read_to_string(bundle.join("v")).unwrap(), "new");
        assert!(!staged.exists() && !sibling(&bundle, "old").exists());
        assert!(swap(&staged, &bundle).is_err(), "nothing staged");
        assert_eq!(
            std::fs::read_to_string(bundle.join("v")).unwrap(),
            "new",
            "rolled back"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
