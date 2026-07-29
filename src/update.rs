//! Self-update and passive update notification.
//!
//! Two entry points:
//! - [`self_update`] performs an in-place upgrade, backed by the install receipt
//!   written by the dist shell/PowerShell installers.
//! - [`notify_if_outdated`] prints a one-line hint to stderr when a newer release
//!   exists, at most once a day.
//!
//! Installations owned by a package manager (Homebrew, `cargo install`) have no
//! receipt. Those are never replaced in place; the user is told which command to
//! run instead.

use anyhow::{Result, anyhow};
use axoupdater::{AxoUpdater, AxoupdateError, ReleaseSource, ReleaseSourceType, UpdateRequest};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::env;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::debug;

/// The dist app name, which is the cargo package name rather than the binary name.
const APP_NAME: &str = "changelog-x";
const REPO_OWNER: &str = "maikbasel";
const REPO_NAME: &str = "changelog-x";

/// Minimum time between two background version checks.
const CHECK_INTERVAL: Duration = Duration::from_hours(24);

/// Hard limit on the background check so a slow network never delays a command.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Set to any value to suppress the background version check entirely.
const OPT_OUT_VAR: &str = "CGX_NO_UPDATE_CHECK";

fn release_source() -> ReleaseSource {
    ReleaseSource {
        release_type: ReleaseSourceType::GitHub,
        owner: REPO_OWNER.to_owned(),
        name: REPO_NAME.to_owned(),
        app_name: APP_NAME.to_owned(),
    }
}

/// How this binary was installed, which decides whether it may replace itself.
enum InstallKind {
    /// Installed by a dist installer; an install receipt describes the install.
    Receipt(Box<AxoUpdater>),
    /// Installed by something else, which owns the binary and must do the upgrade.
    Unmanaged,
}

/// Build an updater and determine whether this install owns its own binary.
///
/// A missing or foreign receipt is not an error. It means some package manager
/// put the binary there, so self-replacement is the wrong operation.
fn detect_install() -> Result<InstallKind> {
    let mut updater = AxoUpdater::new_for(APP_NAME);
    updater.set_release_source(release_source());

    match updater.load_receipt() {
        Ok(_) => {}
        Err(AxoupdateError::NoReceipt { .. } | AxoupdateError::ReceiptLoadFailed { .. }) => {
            return Ok(InstallKind::Unmanaged);
        }
        Err(err) => return Err(anyhow!(err).context("Failed to read the cgx install receipt")),
    }

    // A receipt can exist while the running binary came from somewhere else, for
    // example a Homebrew cgx shadowing an older installer-managed one on PATH.
    if updater.check_receipt_is_for_this_executable()? {
        Ok(InstallKind::Receipt(Box::new(updater)))
    } else {
        Ok(InstallKind::Unmanaged)
    }
}

/// Instructions for installs this binary must not replace itself.
fn unmanaged_hint() -> String {
    format!(
        "cgx was not installed by the official installer, so it cannot update itself.\n\
         Use the tool that installed it:\n  \
         Homebrew:      brew upgrade {APP_NAME}\n  \
         Cargo:         cargo install --git https://github.com/{REPO_OWNER}/{REPO_NAME}\n  \
         Installer:     https://github.com/{REPO_OWNER}/{REPO_NAME}/releases/latest"
    )
}

/// Upgrade this binary in place to the newest stable release.
///
/// With `force`, the newest release is reinstalled even when it matches the
/// running version.
///
/// # Errors
///
/// Returns an error when the receipt is unreadable, when the release cannot be
/// queried, or when the installer fails.
pub async fn self_update(force: bool) -> Result<Option<String>> {
    let mut updater = match detect_install()? {
        InstallKind::Receipt(updater) => updater,
        InstallKind::Unmanaged => return Err(anyhow!(unmanaged_hint())),
    };

    updater.configure_version_specifier(UpdateRequest::Latest);
    updater.always_update(force);

    match updater.run().await {
        Ok(Some(result)) => Ok(Some(result.new_version.to_string())),
        Ok(None) => Ok(None),
        Err(AxoupdateError::NoStableReleases { .. }) => {
            Err(anyhow!("No stable release of {APP_NAME} is available yet"))
        }
        Err(err) => Err(anyhow!(err).context("Update failed")),
    }
}

/// Cached result of the last background version check.
#[derive(Debug, Serialize, Deserialize)]
struct CheckState {
    /// Unix timestamp of the last completed check.
    last_check: u64,
    /// Newest version seen at that check, if any.
    latest: Option<String>,
}

fn state_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "cgx").map(|dirs| dirs.cache_dir().join("update-check.json"))
}

fn read_state() -> Option<CheckState> {
    let raw = std::fs::read_to_string(state_path()?).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_state(state: &CheckState) {
    let Some(path) = state_path() else { return };
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    if let Ok(raw) = serde_json::to_string(state) {
        // A failed cache write only costs an extra check next run.
        let _ = std::fs::write(&path, raw);
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Whether enough time has passed since `last_check` to check again.
///
/// A cache written in the future, which happens when the clock is adjusted
/// backwards, counts as due rather than blocking checks until the clock catches up.
fn interval_elapsed(last_check: Option<u64>, now: u64) -> bool {
    last_check.is_none_or(|last| now < last || now - last >= CHECK_INTERVAL.as_secs())
}

/// Whether this run may check for and report a new version.
///
/// This gates the notice as a whole, not just the network call. Opting out has to
/// silence a cached result too, otherwise the last check keeps talking for a day.
fn notifications_enabled() -> bool {
    env::var_os(OPT_OUT_VAR).is_none()
        && env::var_os("CI").is_none()
        && std::io::stderr().is_terminal()
}

/// Compare the running version against the newest release, ignoring pre-releases.
fn is_newer(latest: &str, current: &str) -> bool {
    match (semver_parse(latest), semver_parse(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

/// Parse `major.minor.patch`, discarding any pre-release or build suffix.
fn semver_parse(version: &str) -> Option<(u64, u64, u64)> {
    let core = version
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or_default();
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// The hint printed when a newer release exists, tailored to the install source.
fn upgrade_hint() -> &'static str {
    match detect_install() {
        Ok(InstallKind::Receipt(_)) => "run `cgx self update`",
        _ => "upgrade with your package manager, e.g. `brew upgrade changelog-x`",
    }
}

/// Query the newest release, bounded by [`CHECK_TIMEOUT`].
async fn query_latest() -> Option<String> {
    let mut updater = AxoUpdater::new_for(APP_NAME);
    updater.set_release_source(release_source());
    updater
        .set_current_version(env!("CARGO_PKG_VERSION").parse().ok()?)
        .ok()?;

    let query = tokio::time::timeout(CHECK_TIMEOUT, updater.query_new_version());
    match query.await {
        Ok(Ok(version)) => version.map(ToString::to_string),
        Ok(Err(err)) => {
            debug!("Update check failed: {err}");
            None
        }
        Err(_) => {
            debug!("Update check timed out after {CHECK_TIMEOUT:?}");
            None
        }
    }
}

/// Print a one-line notice to stderr when a newer release is available.
///
/// Never fails and never blocks longer than `CHECK_TIMEOUT`. Call this after a
/// command has produced its output so the notice cannot interleave with results.
pub async fn notify_if_outdated() {
    if !notifications_enabled() {
        return;
    }

    let current = env!("CARGO_PKG_VERSION");
    let cached = read_state();

    let latest = if interval_elapsed(cached.as_ref().map(|state| state.last_check), now_secs()) {
        let latest = query_latest().await;
        write_state(&CheckState {
            last_check: now_secs(),
            latest: latest.clone(),
        });
        latest
    } else {
        // Re-notify from cache so the hint persists between checks.
        cached.and_then(|state| state.latest)
    };

    let Some(latest) = latest else { return };
    if !is_newer(&latest, current) {
        return;
    }

    eprintln!(
        "\ncgx {latest} is available (you have {current}), {}",
        upgrade_hint()
    );
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_versions() {
        assert_eq!(semver_parse("1.2.3"), Some((1, 2, 3)));
    }

    #[test]
    fn parses_tag_and_prerelease_forms() {
        assert_eq!(semver_parse("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(semver_parse("1.2.3-rc.1"), Some((1, 2, 3)));
        assert_eq!(semver_parse("1.2.3+build.5"), Some((1, 2, 3)));
    }

    #[test]
    fn rejects_garbage_versions() {
        assert_eq!(semver_parse("not-a-version"), None);
        assert_eq!(semver_parse(""), None);
    }

    #[test]
    fn detects_newer_releases() {
        assert!(is_newer("0.2.0", "0.1.1"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.1.2", "0.1.1"));
    }

    #[test]
    fn ignores_same_or_older_releases() {
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("v0.1.1", "0.1.1"));
    }

    #[test]
    fn unparseable_versions_never_notify() {
        assert!(!is_newer("garbage", "0.1.1"));
        assert!(!is_newer("0.2.0", "garbage"));
    }

    /// Hits the real GitHub API, so it is excluded from the default run.
    /// Run with `cargo test -- --ignored` to verify the release source still resolves.
    #[tokio::test]
    #[ignore = "requires network access"]
    async fn queries_the_real_release_source() {
        let mut updater = AxoUpdater::new_for(APP_NAME);
        updater.set_release_source(release_source());
        updater
            .set_current_version("0.0.1".parse().expect("valid semver"))
            .expect("version accepted");

        let latest = updater
            .query_new_version()
            .await
            .expect("release query succeeds")
            .expect("a release exists");

        assert!(is_newer(&latest.to_string(), "0.0.1"));
    }

    #[test]
    fn first_run_checks_immediately() {
        assert!(interval_elapsed(None, 1_000_000));
    }

    #[test]
    fn check_is_skipped_within_the_interval() {
        let now = 1_000_000;
        let an_hour_ago = now - 3600;
        assert!(!interval_elapsed(Some(an_hour_ago), now));
    }

    #[test]
    fn check_runs_once_the_interval_elapses() {
        let now = 1_000_000;
        let a_day_ago = now - CHECK_INTERVAL.as_secs();
        assert!(interval_elapsed(Some(a_day_ago), now));
    }

    #[test]
    fn a_cache_from_the_future_does_not_block_checks() {
        let now = 1_000_000;
        assert!(interval_elapsed(Some(now + 5000), now));
    }
}
