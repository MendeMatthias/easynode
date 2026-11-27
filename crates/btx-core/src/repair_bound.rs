//! How often the app repairs something by itself, kept in a file in the node
//! folder so the bound holds across app restarts.
//!
//! Two repairs use it: putting a deleted node engine back (about 700 MB
//! copied) and downloading a damaged swarm engine again (about 500 MB). Both
//! are made at most once per [`WINDOW_SECS`] by themselves. The owner's own
//! action (pressing Start node, flipping the swarm switch) is always allowed
//! and starts the count again. Without this, antivirus software that keeps
//! deleting the engine, or a download that keeps failing, repeated the whole
//! copy or download on every start of the app.
//!
//! The record is two lines: the unix time of the last attempt, and why it
//! failed (empty when it did not fail, or when it is still under way).

use std::path::Path;

/// One automatic attempt per this long.
pub const WINDOW_SECS: u64 = 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// Unix seconds.
    pub at: u64,
    /// Why it failed, when it did.
    pub failed: Option<String>,
}

/// The record in `path`, or `None` when there is none or it does not read.
pub fn read(path: &Path) -> Option<Attempt> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let at = lines.next()?.trim().parse::<u64>().ok()?;
    let failed = lines.collect::<Vec<_>>().join(" ").trim().to_string();
    Some(Attempt {
        at,
        failed: (!failed.is_empty()).then_some(failed),
    })
}

/// Write the record of an attempt made at `at`, atomically.
pub fn record(path: &Path, at: u64, failed: Option<&str>) -> std::io::Result<()> {
    let why = failed.unwrap_or("").replace(['\n', '\r'], " ");
    let why: String = why.chars().take(600).collect();
    crate::fsx::atomic_write(path, format!("{at}\n{why}\n").as_bytes())
}

/// Forget the record: the owner asked.
pub fn clear(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// May an attempt be made now? Always when the owner asked; otherwise only
/// when none was made in the last [`WINDOW_SECS`]. A record from the future
/// (a clock set back) does not block for longer than the window.
pub fn allows(last: Option<&Attempt>, now: u64, owner_asked: bool) -> bool {
    match last {
        _ if owner_asked => true,
        None => true,
        Some(a) => {
            now >= a.at.saturating_add(WINDOW_SECS) || a.at > now.saturating_add(WINDOW_SECS)
        }
    }
}

/// When the next automatic attempt is allowed, as unix seconds.
pub fn next_allowed(last: &Attempt) -> u64 {
    last.at.saturating_add(WINDOW_SECS)
}

/// "2026-11-27 14:05 UTC" for a unix time, for the log and the window. The
/// date is part of it: "not before 20:42" read at 20:45 means tomorrow, and
/// a bare time said the opposite.
pub fn clock(unix: u64) -> String {
    let secs = unix % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = (unix / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        secs / 3600,
        (secs % 3600) / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bound is in the file: a new run of the app (a new read) sees the
    /// attempt, and only the window or the owner lets the next one through.
    #[test]
    fn an_attempt_holds_across_a_restart_until_the_window_or_the_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("easynode-swarm-repair");
        assert_eq!(read(&path), None);
        assert!(allows(read(&path).as_ref(), 1_000, false));

        record(
            &path,
            1_000,
            Some("The swarm engine download failed (HTTP 404).\nagain"),
        )
        .unwrap();
        // "Restart": nothing in memory, only the file.
        let last = read(&path).unwrap();
        assert_eq!(last.at, 1_000);
        assert_eq!(
            last.failed.as_deref(),
            Some("The swarm engine download failed (HTTP 404). again")
        );
        assert!(!allows(Some(&last), 1_000 + 3_600, false));
        assert!(!allows(Some(&last), 1_000 + WINDOW_SECS - 1, false));
        assert!(allows(Some(&last), 1_000 + WINDOW_SECS, false));
        assert!(allows(Some(&last), 1_001, true), "the owner always may");
        assert_eq!(next_allowed(&last), 1_000 + WINDOW_SECS);

        // A success is an attempt too: something that keeps deleting the
        // engine does not get a fresh 700 MB copy on every start.
        record(&path, 5_000, None).unwrap();
        let last = read(&path).unwrap();
        assert_eq!(last.failed, None);
        assert!(!allows(Some(&last), 5_060, false));

        // A clock set far back does not lock it for longer than the window.
        let future = Attempt {
            at: 10 * WINDOW_SECS,
            failed: None,
        };
        assert!(allows(Some(&future), 8 * WINDOW_SECS, false));

        clear(&path);
        assert_eq!(read(&path), None);
        // Garbage reads as no record.
        std::fs::write(&path, "not a time\n").unwrap();
        assert_eq!(read(&path), None);
        assert_eq!(clock(1_795_720_459), "2026-11-26 19:14 UTC");
        assert_eq!(clock(0), "1970-01-01 00:00 UTC");
        assert_eq!(clock(951_782_400), "2000-02-29 00:00 UTC");
    }
}
