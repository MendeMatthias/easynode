//! The six-hourly self-update check, on a timer nothing throttles.
//!
//! Until this module the periodic check was a `setInterval` in the webview
//! (`src/main.ts`, next to the launch-time `updateCheck()`), and the launch
//! check plus the "Check now" button still are: they run in JavaScript and
//! record through `record_update_check`. The recheck now runs here, in Rust,
//! through the updater plugin's own API, and records through the same
//! `update_log::record` with the same five words, so `update-check.log` reads
//! the same whichever side ran the check.
//!
//! WHY IT MOVED, STATED AS A HYPOTHESIS. Measured 2026-09-15, and only this
//! much: this project's own signer box ran 0.6.21 for eight hours after the
//! feed served 0.6.22; two six-hourly checks fell due in that window; nothing
//! updated and nothing left a trace. The launch-time check on the same box did
//! work once, on 09-10, right after a relaunch. The hypothesis that fits both
//! facts is that WebKitGTK throttles or suspends JavaScript timers in a window
//! that is hidden or minimized, which is how a node app that "sits quietly in
//! the menu bar" spends its life, so the interval never fired while the launch
//! check, which needs no timer, did. That is a hypothesis, not a finding: the
//! webview's timers were never instrumented, and the branch that added
//! `update-check.log` did so precisely because nothing could tell. This timer
//! runs on tokio, which knows nothing about window visibility. The log is what
//! confirms or refutes the hypothesis on the next release: six-hourly
//! `automatic:` lines on a box that sat in the tray confirm the timer fires
//! where the interval did not; their absence says the cause was elsewhere.
//!
//! What this does not address, said so nobody expects it to: a machine that
//! sleeps. tokio's clock is monotonic and does not run through suspend, so a
//! laptop closed for a day gets its next check a period after it wakes, not
//! at once. The launch check covers the common case of a machine that was off.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

use crate::state::node_datadir;
use crate::update_log;

/// The first check waits this long after the timer is armed at launch. The
/// front end checks at launch already (`main.ts`, boot); a second check at
/// once would race it, and two minutes is enough for a launch check that found
/// something to have downloaded, recorded, and restarted the process, in which
/// case this timer never reaches its first tick at all.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(2 * 60);

/// Six hours: the period the webview's `setInterval` used until this module
/// took the job, and the period the Settings pane's copy promises ("Checks
/// automatically on launch and every 6 hours"). A test below reads that copy.
pub const CHECK_PERIOD: Duration = Duration::from_secs(6 * 60 * 60);

/// The event the webview listens for (`main.ts`), carrying one settled check.
/// `update-check.test.ts` reads this constant to keep the two names equal.
pub const UPDATE_CHECK_EVENT: &str = "update-check";

/// The schedule as two numbers, so a test can pin it without an event loop:
/// tick 0 fires at `first_delay`, tick n at `first_delay + n * period`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    pub first_delay: Duration,
    pub period: Duration,
}

/// The arithmetic the tests pin. The timer itself is tokio's `interval_at`,
/// which does the same sums; nothing outside the tests needs them spelled out.
#[cfg(test)]
impl Schedule {
    /// When tick `n` (zero-based) fires, measured from the moment the timer
    /// was armed, if every tick fires on time.
    pub fn tick_at(&self, n: u32) -> Duration {
        self.first_delay + self.period * n
    }

    /// How many ticks fall within `window` of arming: the number of checks a
    /// box that sat in the tray that long owes the log.
    pub fn ticks_within(&self, window: Duration) -> u32 {
        if window < self.first_delay {
            return 0;
        }
        let after_first = window - self.first_delay;
        1 + (after_first.as_secs() / self.period.as_secs()) as u32
    }
}

/// The one schedule the app runs.
pub fn schedule() -> Schedule {
    Schedule {
        first_delay: FIRST_CHECK_DELAY,
        period: CHECK_PERIOD,
    }
}

/// What the webview receives when a check the timer ran has settled: the same
/// `outcome` and `detail` the record got, the version offered where there was
/// one (empty otherwise), and the record's timestamp, so the pane can paint
/// its "Last check" line from this exactly as it paints it from the settings
/// file on the next status tick.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheckEvent<'a> {
    pub outcome: &'a str,
    pub version: &'a str,
    pub detail: &'a str,
    pub at: String,
}

/// Arm the timer. Called once from `lib.rs` setup; the task lives as long as
/// the process does. See the module comment for the hypothesis it tests.
pub fn spawn(app: AppHandle) {
    let sched = schedule();
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + sched.first_delay,
            sched.period,
        );
        // A machine that was suspended through several periods gets ONE check
        // when the clock catches up and the next a full period after it, not
        // a burst of every check it missed.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            check_once(&app).await;
        }
    });
}

/// One automatic check, start to finish, mirroring `updateCheck()` in
/// `main.ts` branch for branch: the builder with no options is exactly what
/// the JavaScript `check()` with no options asks the plugin for (its `check`
/// command calls `updater_builder()` and sets only what it was passed), so
/// the endpoints, the public key and the target come from `tauri.conf.json`
/// on both paths. On `found` the record is written BEFORE the download, so a
/// check that found something and died mid-download still left the finding
/// behind. On `installed` the restart is requested the way the front end's
/// `relaunch()` requests it (the process plugin's `restart` command is
/// `app.request_restart()`), which `lib.rs` recognises by `RESTART_EXIT_CODE`
/// and lets through without stopping btxd.
async fn check_once(app: &AppHandle) {
    let datadir = node_datadir();
    let current = app.package_info().version.to_string();

    let checked = match app.updater_builder().build() {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    let update = match checked {
        Err(e) => {
            settle(app, &datadir, "check-failed", "", &check_failure_detail(&e));
            return;
        }
        Ok(None) => {
            let detail = format!("automatic: v{current} is current");
            settle(app, &datadir, "no-update", "", &detail);
            return;
        }
        Ok(Some(update)) => update,
    };

    let version = update.version.clone();
    let detail = format!("automatic: v{version} offered, downloading");
    settle(app, &datadir, "found", &version, &detail);

    if let Err(e) = update.download_and_install(|_, _| {}, || {}).await {
        let detail = format!("automatic: v{version}: {}", error_text(&e));
        settle(app, &datadir, "install-failed", &version, &detail);
        return;
    }

    // Written synchronously, so it is on disk before the restart is asked for:
    // the front end has to race its record against a two-second bound here,
    // this side does not.
    let detail = format!("automatic: v{version}, restarting");
    settle(app, &datadir, "installed", &version, &detail);
    app.request_restart();
}

/// Write the outcome down and tell the webview, in that order, and let
/// neither change what happens next: a record that fails is a line on stderr,
/// and an emit nobody is listening to is dropped by Tauri. The detail is
/// flattened once here so the log line and the event carry the same text.
fn settle(app: &AppHandle, datadir: &Path, outcome: &str, version: &str, detail: &str) {
    let now = update_log::now_secs();
    let detail = update_log::one_line(detail);
    if let Err(e) = update_log::record(datadir, now, outcome, &detail) {
        eprintln!("[update-timer] could not record {outcome}: {e}");
    }
    let _ = app.emit(
        UPDATE_CHECK_EVENT,
        UpdateCheckEvent {
            outcome,
            version,
            detail: &detail,
            at: update_log::rfc3339_utc(now),
        },
    );
}

/// The detail for a failed CHECK, classified the way `classifyCheckFailure`
/// in `src/update-check.ts` classifies it, but on the plugin's error variants
/// rather than its wording: a release that has no build for this platform is
/// the safe state and the normal cadence here (0.6.18 through 0.6.20 were
/// Linux-only at some point), and the pane renders it as "no build for this
/// platform yet" from these exact words.
fn check_failure_detail(e: &tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error;
    match e {
        Error::TargetNotFound(_) | Error::TargetsNotFound(_) => {
            "automatic: no build for this platform".to_string()
        }
        _ => format!("automatic: {}", error_text(e)),
    }
}

/// An updater error can run to a stack of URLs; the log keeps this much, the
/// same bound `errorText` in `src/update-check.ts` applies.
const DETAIL_ERROR_CHARS: usize = 200;

/// The plugin's error as one line of at most [`DETAIL_ERROR_CHARS`].
fn error_text(e: &tauri_plugin_updater::Error) -> String {
    e.to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(DETAIL_ERROR_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_updater::Error;

    const HOUR: Duration = Duration::from_secs(60 * 60);

    #[test]
    fn the_first_check_waits_for_the_launch_check_and_then_keeps_the_period() {
        let s = schedule();
        // Not at once: the front end checks at launch and must not be raced.
        assert!(
            s.first_delay >= Duration::from_secs(60),
            "{:?}",
            s.first_delay
        );
        // But soon, so a box that never sits six hours still gets one.
        assert!(
            s.first_delay <= Duration::from_secs(5 * 60),
            "{:?}",
            s.first_delay
        );
        assert_eq!(s.period, 6 * HOUR);
        assert_eq!(s.tick_at(0), s.first_delay);
        assert_eq!(s.tick_at(1), s.first_delay + s.period);
        assert_eq!(s.tick_at(4) - s.tick_at(3), s.period);
    }

    /// The measurement behind this module: eight hours on 0.6.21 with the feed
    /// serving 0.6.22, and two checks due in that window. This schedule owes
    /// the log two `automatic:` lines for such a window, and none for a box
    /// that was up for a minute.
    #[test]
    fn eight_hours_in_the_tray_owe_the_log_two_checks() {
        let s = schedule();
        assert_eq!(s.ticks_within(8 * HOUR), 2);
        assert_eq!(s.ticks_within(Duration::from_secs(60)), 0);
        assert_eq!(s.ticks_within(s.first_delay), 1);
        assert_eq!(s.ticks_within(s.tick_at(1) - Duration::from_secs(1)), 1);
        assert_eq!(s.ticks_within(s.tick_at(1)), 2);
        assert_eq!(s.ticks_within(24 * HOUR), 4);
    }

    /// The Settings pane promises a period in words; the timer keeps it.
    #[test]
    fn the_settings_copy_promises_the_same_period() {
        let html = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../index.html"))
            .expect("apps/node/index.html");
        let hours = CHECK_PERIOD.as_secs() / 3600;
        assert!(
            html.contains(&format!("on launch and every {hours} hours")),
            "index.html does not promise every {hours} hours"
        );
    }

    /// Every word this side writes is in the vocabulary, so the timer can never
    /// be refused unwritten by `record`, and every word in the vocabulary is
    /// one this side can write. (`update-check.test.ts` pins the same from the
    /// TypeScript side by reading this file.)
    #[test]
    fn the_timer_uses_the_whole_vocabulary_and_nothing_else() {
        let src = include_str!("update_timer.rs");
        let mut used: Vec<&str> = Vec::new();
        for line in src.lines().take_while(|l| !l.contains("mod tests")) {
            if let Some(rest) = line.trim_start().strip_prefix("settle(app, &datadir, \"") {
                let word = rest.split('"').next().unwrap();
                used.push(word);
            }
        }
        used.sort_unstable();
        used.dedup();
        let mut want = update_log::UPDATE_CHECK_OUTCOMES.to_vec();
        want.sort_unstable();
        assert_eq!(used, want);
    }

    #[test]
    fn a_missing_platform_is_named_not_quoted() {
        let e = Error::TargetNotFound("linux-x86_64".into());
        assert_eq!(
            check_failure_detail(&e),
            "automatic: no build for this platform"
        );
        let e = Error::TargetsNotFound(vec!["darwin-aarch64".into(), "darwin-x86_64".into()]);
        assert_eq!(
            check_failure_detail(&e),
            "automatic: no build for this platform"
        );
        // Anything else carries the plugin's words, prefixed like the front
        // end's `automatic: ` details.
        let e = Error::Network("dns error".into());
        assert_eq!(check_failure_detail(&e), "automatic: `dns error`");
        assert_eq!(
            check_failure_detail(&Error::EmptyEndpoints),
            "automatic: Updater does not have any endpoints set."
        );
    }

    #[test]
    fn the_error_text_is_one_line_and_bounded() {
        let e = Error::Network(format!(
            "first\nsecond\r\n\tthird   fourth {}",
            "x".repeat(1_000)
        ));
        let t = error_text(&e);
        assert!(t.starts_with("`first second third fourth x"), "{t}");
        assert!(!t.contains('\n'));
        assert_eq!(t.chars().count(), DETAIL_ERROR_CHARS);
        // Inside the record's own bound, so `record` flattens nothing further
        // and the event carries exactly what the log line does.
        assert!(DETAIL_ERROR_CHARS + "automatic: v0.6.23: ".len() <= update_log::DETAIL_MAX_CHARS);
    }
}
