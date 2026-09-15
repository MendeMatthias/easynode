//! The outcome of every self-update check, written down.
//!
//! `updateCheck()` in `src/main.ts` painted its result into the Settings pane
//! and nowhere else, and on the automatic path a failed check returned in
//! silence. Measured 2026-09-15: this project's own signer box ran 0.6.21 for
//! eight hours after the feed served 0.6.22, with two six-hourly checks due
//! in that window, and nothing on the machine could say whether they ran,
//! failed, or found nothing. The release recipe's "observe a real upgrade"
//! step was not possible to carry out.
//!
//! This module is the durable answer, in two places: one line per check in
//! `<datadir>/update-check.log`, next to `setup.log` and through the same
//! private-append helper, and the LAST outcome in the settings file, which
//! `get_node_status` carries so the pane shows it without waiting for the
//! next check and after a relaunch. The front end calls `record_update_check`
//! at every exit of its `updateCheck()` (the launch check and the button),
//! fire-and-forget, and the six-hourly timer in `update_timer` calls `record`
//! directly at every exit of its own check: nothing written here may change
//! what the updater does, on either side.

use std::io::Write;
use std::path::Path;

use crate::state::NodeAppSettings;

/// One line per check, newest last, in the datadir beside `setup.log`.
pub const UPDATE_CHECK_LOG: &str = "update-check.log";

/// The closed set of things a check can end as. `UPDATE_CHECK_OUTCOMES` in
/// `src/update-check.ts` is the same five words, and `update-check.test.ts`
/// reads this file to keep the two lists equal. Anything else is refused
/// unwritten: a sixth word on one side is a bug, not a new outcome.
///
/// `found` is followed by `installed` or `install-failed` on the same check,
/// so a real upgrade leaves two lines; that is the point, the first says when
/// the feed offered it and the second says what happened next.
pub const UPDATE_CHECK_OUTCOMES: [&str; 5] = [
    "no-update",
    "check-failed",
    "found",
    "install-failed",
    "installed",
];

/// A check on launch and every six hours is under 1,500 lines a year, each
/// under 300 bytes. The cap keeps a node that runs for years bounded anyway.
pub const LOG_MAX_BYTES: u64 = 64 * 1024;

/// When the file grows past the cap it is cut back to the newest lines that
/// fit in this many bytes, on a line boundary, so it does not trim again on
/// the very next append.
pub const LOG_TRIM_TO_BYTES: usize = 32 * 1024;

/// The detail is free text from the front end, an error message sliced to
/// ~200 characters plus a short prefix. One line, this long at most.
pub const DETAIL_MAX_CHARS: usize = 240;

/// Write down how a check ended: the last outcome into the settings file and
/// one line into the log, in that order, because the pane's line is the
/// surface a person actually looks at and the log is the one they look at
/// afterwards. An unknown `outcome` is refused before either write.
pub fn record(datadir: &Path, now_secs: u64, outcome: &str, detail: &str) -> Result<(), String> {
    if !UPDATE_CHECK_OUTCOMES.contains(&outcome) {
        return Err(format!(
            "unknown update-check outcome {outcome:?}; expected one of {UPDATE_CHECK_OUTCOMES:?}"
        ));
    }
    let at = rfc3339_utc(now_secs);
    let detail = one_line(detail);

    NodeAppSettings::update(datadir, |s| {
        s.last_update_check_at = Some(at.clone());
        s.last_update_check_outcome = Some(outcome.to_string());
        s.last_update_check_detail = detail.clone();
    });

    let line = if detail.is_empty() {
        format!("{at} {outcome}\n")
    } else {
        format!("{at} {outcome} {detail}\n")
    };
    append_bounded(&datadir.join(UPDATE_CHECK_LOG), &line).map_err(|e| e.to_string())
}

/// Append `line` through the same private-append helper `setup.log` uses,
/// then cut the file back to its newest [`LOG_TRIM_TO_BYTES`] once it grows
/// past [`LOG_MAX_BYTES`]. The trim is a truncate-and-rewrite through the
/// matching private-write helper rather than `fsx::atomic_write`, which
/// creates its temp file with the default mode: this is a log in a private
/// directory and a torn trim costs history, not correctness.
fn append_bounded(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    {
        let mut f = btx_core::platform::open_private_append(path)?;
        f.write_all(line.as_bytes())?;
    }
    if std::fs::metadata(path)?.len() > LOG_MAX_BYTES {
        let contents = String::from_utf8_lossy(&std::fs::read(path)?).into_owned();
        let tail = newest_lines_within(&contents, LOG_TRIM_TO_BYTES);
        let mut f = btx_core::platform::open_private_write(path)?;
        f.write_all(tail.as_bytes())?;
    }
    Ok(())
}

/// The suffix of `contents` that fits in `max_bytes` and starts at a line
/// boundary, so the file never begins mid-line after a trim. Works on bytes
/// and cuts right after a newline, which is always a character boundary.
fn newest_lines_within(contents: &str, max_bytes: usize) -> &str {
    if contents.len() <= max_bytes {
        return contents;
    }
    let start = contents.len() - max_bytes;
    let bytes = contents.as_bytes();
    // `start >= 1` here. When the byte before the window is a newline the
    // window already begins a line; skipping to the next newline would throw
    // away a whole line that fits.
    if bytes[start - 1] == b'\n' {
        return &contents[start..];
    }
    match bytes[start..].iter().position(|&b| b == b'\n') {
        Some(nl) => &contents[start + nl + 1..],
        None => "",
    }
}

/// Seconds since the Unix epoch, now; 0 on a clock set before 1970, which the
/// timestamp then renders as such rather than refusing the record.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One line, printable, at most [`DETAIL_MAX_CHARS`] characters: an error
/// message from the updater can carry newlines, and a newline in a
/// line-per-record log is a forged record. `record` applies it; `update_timer`
/// applies it once more, up front, so the event it emits carries exactly the
/// text the log line got.
pub fn one_line(detail: &str) -> String {
    let flat: String = detail
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(DETAIL_MAX_CHARS)
        .collect()
}

/// Seconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`. The crate has no
/// calendar dependency and this is the one place it needs a date, so this is
/// the proleptic-Gregorian conversion by hand (Howard Hinnant's
/// `civil_from_days`), pinned against GNU `date -u` in the tests below.
pub fn rfc3339_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SETTINGS_FILE_NAME;

    fn log_lines(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join(UPDATE_CHECK_LOG))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Pinned against `date -u -d @N +%Y-%m-%dT%H:%M:%SZ` on 2026-09-15.
    #[test]
    fn the_timestamp_matches_gnu_date() {
        for (secs, want) in [
            (0, "1970-01-01T00:00:00Z"),
            (951_782_400, "2000-02-29T00:00:00Z"),
            (1_709_251_199, "2024-02-29T23:59:59Z"),
            (1_757_894_400, "2025-09-15T00:00:00Z"),
            (1_789_481_002, "2026-09-15T14:03:22Z"),
            (2_147_483_647, "2038-01-19T03:14:07Z"),
            (4_102_444_800, "2100-01-01T00:00:00Z"),
            (4_107_542_400, "2100-03-01T00:00:00Z"),
        ] {
            assert_eq!(rfc3339_utc(secs), want, "at {secs}");
        }
    }

    #[test]
    fn a_check_leaves_one_line_and_the_last_outcome() {
        let dir = tempfile::tempdir().unwrap();
        record(
            dir.path(),
            1_789_481_002,
            "check-failed",
            "automatic: dns error",
        )
        .unwrap();

        assert_eq!(
            log_lines(dir.path()),
            vec!["2026-09-15T14:03:22Z check-failed automatic: dns error"]
        );
        let s = NodeAppSettings::load(dir.path());
        assert_eq!(
            s.last_update_check_at.as_deref(),
            Some("2026-09-15T14:03:22Z")
        );
        assert_eq!(s.last_update_check_outcome.as_deref(), Some("check-failed"));
        assert_eq!(s.last_update_check_detail, "automatic: dns error");
        // In THIS app's file, not the miner's.
        assert!(dir.path().join(SETTINGS_FILE_NAME).exists());
        assert!(!dir.path().join("easybtx-state.json").exists());
    }

    #[test]
    fn a_later_check_appends_and_replaces_the_last_outcome() {
        let dir = tempfile::tempdir().unwrap();
        record(
            dir.path(),
            1_789_481_002,
            "found",
            "automatic: v0.6.23 offered",
        )
        .unwrap();
        record(
            dir.path(),
            1_789_481_040,
            "installed",
            "automatic: v0.6.23, restarting",
        )
        .unwrap();

        let lines = log_lines(dir.path());
        assert_eq!(lines.len(), 2, "one line per check, nothing overwritten");
        assert!(lines[0].starts_with("2026-09-15T14:03:22Z found "));
        assert!(lines[1].starts_with("2026-09-15T14:04:00Z installed "));
        let s = NodeAppSettings::load(dir.path());
        assert_eq!(s.last_update_check_outcome.as_deref(), Some("installed"));
        assert_eq!(
            s.last_update_check_at.as_deref(),
            Some("2026-09-15T14:04:00Z")
        );
    }

    #[test]
    fn an_empty_detail_leaves_no_trailing_space() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), 0, "no-update", "").unwrap();
        assert_eq!(
            log_lines(dir.path()),
            vec!["1970-01-01T00:00:00Z no-update"]
        );
    }

    #[test]
    fn recording_does_not_disturb_the_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        NodeAppSettings::update(dir.path(), |s| {
            s.setup_complete = true;
            s.node_nickname = "alice".into();
            s.on_close = "tray".into();
        });
        record(dir.path(), 1, "no-update", "manual: v0.6.22 is current").unwrap();
        let s = NodeAppSettings::load(dir.path());
        assert!(s.setup_complete);
        assert_eq!(s.node_nickname, "alice");
        assert_eq!(s.on_close, "tray");
    }

    /// The vocabulary is closed on this side too. A word the front end
    /// invented is refused before anything is written, so the two lists
    /// cannot drift into a log with six kinds of line in it.
    #[test]
    fn an_unknown_outcome_is_refused_unwritten() {
        let dir = tempfile::tempdir().unwrap();
        let err = record(dir.path(), 1, "skipped", "automatic").unwrap_err();
        assert!(err.contains("skipped"), "{err}");
        assert!(
            err.contains("no-update"),
            "the refusal names the vocabulary: {err}"
        );
        assert!(!dir.path().join(UPDATE_CHECK_LOG).exists());
        assert!(NodeAppSettings::load(dir.path())
            .last_update_check_at
            .is_none());
        // Every word in the vocabulary IS accepted, so a typo in the list
        // itself would show up here rather than in front of a user.
        for outcome in UPDATE_CHECK_OUTCOMES {
            record(dir.path(), 1, outcome, "").unwrap();
        }
        assert_eq!(log_lines(dir.path()).len(), UPDATE_CHECK_OUTCOMES.len());
    }

    /// A line-per-record log with a newline inside a record is a forged
    /// record; the updater's errors do carry newlines, and tabs.
    #[test]
    fn the_detail_is_one_line_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let nasty = format!("first\nsecond\r\n\tthird   fourth {}", "x".repeat(1_000));
        record(dir.path(), 1, "install-failed", &nasty).unwrap();
        let lines = log_lines(dir.path());
        assert_eq!(
            lines.len(),
            1,
            "a newline in the detail must not split the record"
        );
        assert!(
            lines[0].contains("first second third fourth x"),
            "{}",
            lines[0]
        );
        let detail = NodeAppSettings::load(dir.path()).last_update_check_detail;
        assert_eq!(detail.chars().count(), DETAIL_MAX_CHARS);
        assert!(!detail.contains('\n'));
    }

    /// The cap. A node that runs for years must not grow this file forever:
    /// past 64 KiB it is cut back to its newest 32 KiB, on a line boundary,
    /// with the newest line intact and the oldest gone.
    #[test]
    fn the_log_is_cut_back_to_its_newest_lines_once_it_passes_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(UPDATE_CHECK_LOG);
        let file_len = || std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        // Each line is ~230 bytes; ~290 of them cross 64 KiB.
        let detail = format!("automatic: {}", "d".repeat(200));

        // Append until the first trim. The trim happens INSIDE the append that
        // crosses the cap, so the file is never observed past it, and until
        // that append every line is kept: nothing is trimmed early.
        let mut n = 0u64;
        let lines_before_trim = loop {
            record(dir.path(), n, "no-update", &detail).unwrap();
            n += 1;
            assert!(file_len() <= LOG_MAX_BYTES, "past the cap after append {n}");
            let lines = log_lines(dir.path()).len() as u64;
            if lines < n {
                break n - 1;
            }
            assert_eq!(lines, n, "a line went missing before the cap");
            assert!(n < 10_000, "the cap never triggered");
        };
        assert!(
            lines_before_trim > 250,
            "trimmed after only {lines_before_trim} lines"
        );

        let len = file_len();
        assert!(len <= LOG_TRIM_TO_BYTES as u64, "trimmed to {len} bytes");
        assert!(
            len > LOG_TRIM_TO_BYTES as u64 / 2,
            "trimmed too far: {len} bytes"
        );

        let lines = log_lines(dir.path());
        assert!(lines.len() > 100, "{} lines survive", lines.len());
        // Every surviving line is whole: the cut landed on a line boundary.
        for l in &lines {
            assert!(
                l.starts_with("1970-01-01T"),
                "a partial line survived: {l:?}"
            );
            assert!(l.ends_with("dddd"), "a partial line survived: {l:?}");
        }
        let newest = rfc3339_utc(n - 1);
        assert!(
            lines.last().unwrap().starts_with(&newest),
            "the newest line survives: {}",
            lines.last().unwrap()
        );
        assert!(
            !lines.iter().any(|l| l.starts_with(&rfc3339_utc(0))),
            "the oldest line is gone"
        );
        // And the file keeps growing normally from there, one line per check.
        record(dir.path(), n, "no-update", &detail).unwrap();
        assert_eq!(log_lines(dir.path()).len(), lines.len() + 1);
    }

    #[test]
    fn a_trim_never_starts_mid_line_or_mid_character() {
        let s = "aaaa\nbbbb\ncccc\n";
        assert_eq!(newest_lines_within(s, 100), s, "under the limit: untouched");
        // Exactly two lines fit, and the window begins on a line boundary.
        assert_eq!(newest_lines_within(s, 10), "bbbb\ncccc\n");
        assert_eq!(
            newest_lines_within(s, 9),
            "cccc\n",
            "a partial line is dropped"
        );
        assert_eq!(newest_lines_within(s, 5), "cccc\n", "exactly one line fits");
        assert_eq!(newest_lines_within(s, 4), "", "no whole line fits");
        // Multi-byte text: the cut lands after a newline, never inside "é".
        let s = "ééé\nééé\n";
        assert_eq!(newest_lines_within(s, 7), "ééé\n");
        assert_eq!(
            newest_lines_within(s, 8),
            "ééé\n",
            "the window opened mid-character"
        );
        assert_eq!(newest_lines_within(s, 6), "", "no whole line fits");
    }
}
