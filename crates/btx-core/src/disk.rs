//! Disk-reclaim maintenance for the EasyBTX datadir.
//!
//! EasyBTX runs btxd UN-PRUNED (`prune=0`) on purpose — pruning reintroduces the
//! shielded-rebuild SIGABRT on unclean shutdown (see `installer.rs`). That keeps
//! the full chain on disk, but several NON-chain things are pure waste we can
//! reclaim safely WITHOUT touching the chain or the prune posture:
//!   * `blockfilterindex` / `coinstatsindex` — btxd builds them but EasyBTX never
//!     queries them (no `gettxoutsetinfo` / `getblockfilter` caller). Dead weight.
//!   * `faststart/snapshot.dat` — the one-time assumeutxo bootstrap; dead once the
//!     snapshot has been loaded (btxd re-downloads only if it ever re-bootstraps).
//!   * `debug.log` — can balloon to GBs.
//!
//! These functions MUST run with btxd NOT holding the files (stopped, or launching
//! fresh): deleting a LevelDB index dir under a live btxd can crash it. Every
//! caller guarantees that (launch pre-spawn, or stop → reclaim → restart).

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Free-disk thresholds (megabytes) driving the UI's low-disk warning banner.
/// `WARN` shows a soft amber notice; `CRITICAL` upgrades to a red banner with
/// a one-click "Heal disk space" button (calls `reclaim_disk_now`). 500 MB at
/// the critical threshold gives btxd headroom for a few minutes of leveldb
/// writes — enough for the user to either click Heal or move data, without
/// btxd crashing from a "No space left on device" mid-flush (the failure mode
/// observed in the wild at debug.log 10:52:09).
///
/// The live threshold comparison runs in the frontend (`renderDiskWarning`),
/// so from the Rust lib's view these read as unused; a unit test guards the
/// `CRITICAL < WARN` ordering. `#[allow(dead_code)]` keeps them as the
/// documented canonical values without a spurious warning.
#[allow(dead_code)]
pub const DISK_WARN_MB: u64 = 1000;
#[allow(dead_code)]
pub const DISK_CRITICAL_MB: u64 = 500;

/// Low-disk thresholds (MB free) for easyBTX NODE. Higher than the miner's
/// `DISK_*_MB` because the node keeps the FULL un-pruned chain, which GROWS over
/// time — so it warns while there's still comfortable room to act (move data,
/// reclaim) before btxd hits "No space left on device" mid-flush. Shipped in the
/// node app's status payload so its frontend reads ONE definition instead of
/// hardcoding the numbers. `WARN` = soft amber; `CRITICAL` = red "may stop".
pub const NODE_DISK_WARN_MB: u64 = 10 * 1024; // 10 GB
pub const NODE_DISK_CRITICAL_MB: u64 = 2 * 1024; // 2 GB

/// Read the megabytes available on the filesystem holding `path`. Uses
/// `statvfs` so it works across APFS, ext4, exFAT, etc. Returns `0` on any
/// failure (path missing, statvfs syscall error, overflow). The caller treats
/// `0` as "not yet measured" so a transient failure never spuriously triggers
/// the critical banner.
pub fn free_disk_mb(path: &std::path::Path) -> u64 {
    // Platform-specific: statvfs on unix, GetDiskFreeSpaceExW on Windows.
    crate::platform::free_disk_mb(path)
}

/// Index subdirs btxd builds that EasyBTX never queries. Deleted + disabled in conf.
const UNUSED_INDEX_DIRS: &[&str] = &["blockfilter", "coinstats"];
/// Matching conf keys, removed so btxd stops rebuilding the indexes.
const UNUSED_INDEX_CONF_KEYS: &[&str] = &["blockfilterindex", "coinstatsindex"];
/// `debug.log` is truncated once it exceeds this many MiB.
const DEBUG_LOG_CAP_MB: u64 = 50;

const MB: u64 = 1024 * 1024;

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct ReclaimReport {
    /// Megabytes (MiB) freed across all categories.
    pub freed_mb: u64,
    /// Human-readable per-item lines (what was removed + how much).
    pub items: Vec<String>,
}

/// Sum the sizes (bytes) of all regular files under `dir`. The ONE walker for
/// both the miner and the node app (the status cards, install-size, and every
/// reclaim tally read it).
///
/// Iterative (an explicit stack, so a deeply-nested tree can't overflow the
/// call stack), `symlink_metadata` so symlinks are NOT followed (a
/// self-referential link can't loop us and a link into `blocks/` can't
/// double-count), and `saturating_add` so a pathological tree can't overflow.
/// A non-existent `dir` returns 0 rather than erroring.
pub fn dir_size_bytes(dir: &Path) -> u64 {
    let mut total: u64 = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue; // unreadable dir → skip rather than abort
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // symlink_metadata: do not traverse symlinks (avoids cycles + double counts).
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

/// Remove the unused-index lines from a faststart.conf string. Pure → testable.
/// Returns the rewritten conf (identical when no such lines are present).
pub fn strip_unused_index_conf_str(conf: &str) -> String {
    let kept: Vec<&str> = conf
        .lines()
        .filter(|line| {
            let key = line.trim().split('=').next().unwrap_or("").trim();
            !UNUSED_INDEX_CONF_KEYS.contains(&key)
        })
        .collect();
    let mut out = kept.join("\n");
    if conf.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Rewrite faststart.conf in place to drop the unused-index lines. Returns true if
/// the file changed (caller may need to (re)start btxd for it to take effect).
fn strip_unused_index_conf(conf_path: &Path) -> bool {
    let Ok(original) = std::fs::read_to_string(conf_path) else {
        return false;
    };
    let rewritten = strip_unused_index_conf_str(&original);
    if rewritten == original {
        return false;
    }
    std::fs::write(conf_path, rewritten).is_ok()
}

/// Reclaim disk in `datadir`. MUST be called with btxd NOT holding these files.
///
/// `snapshot_loaded` is the CALLER's own persisted "loaded" flag (miner's
/// EasyBtxState / node app's NodeAppSettings). `faststart/snapshot.dat` is
/// deleted only when that flag AND the SHARED cross-process marker
/// (`snapshot::snapshot_marker_present`) both agree — so a stale per-app flag
/// (e.g. the miner's from an old load) can never delete a snapshot the OTHER
/// app just downloaded and is still loading over the shared `~/.easybtx`
/// datadir (C3, cross-process). The marker is written on a confirmed load and
/// cleared whenever a fresh snapshot.dat is downloaded.
pub fn reclaim_disk(datadir: &Path, conf_path: &Path, snapshot_loaded: bool) -> ReclaimReport {
    let mut report = ReclaimReport::default();
    // Accumulate raw BYTES and divide to MB exactly once at the end. Dividing
    // per item (`bytes / MB`) truncated every sub-MB item to 0, so the reported
    // total undercounted the real reclaim — part of why Heal looked like it
    // "freed 1 GB" when more came back on restart.
    let mut freed_bytes: u64 = 0;

    // 1) Stop btxd rebuilding the unused indexes, then delete their dirs.
    strip_unused_index_conf(conf_path);
    for name in UNUSED_INDEX_DIRS {
        let dir = datadir.join("indexes").join(name);
        if dir.is_dir() {
            let bytes = dir_size_bytes(&dir);
            if std::fs::remove_dir_all(&dir).is_ok() {
                freed_bytes += bytes;
                report.items.push(format!("{name} index ({} MB)", bytes / MB));
            }
        }
    }

    // 2) Delete the post-load assumeutxo bootstrap snapshot (C3-gated).
    if let Some(bytes) = sweep_loaded_snapshot(datadir, snapshot_loaded) {
        freed_bytes += bytes;
        report.items.push(format!("assumeutxo snapshot ({} MB)", bytes / MB));
    }

    // 3) Cap debug.log (safe: btxd is stopped, so truncation can't race a write).
    let log = datadir.join("debug.log");
    if let Ok(meta) = std::fs::metadata(&log) {
        if meta.len() > DEBUG_LOG_CAP_MB * MB {
            let bytes = meta.len();
            if std::fs::write(&log, b"").is_ok() {
                freed_bytes += bytes;
                report.items.push(format!("debug.log ({} MB)", bytes / MB));
            }
        }
    }

    report.freed_mb = freed_bytes / MB;
    report
}

/// Delete the post-load assumeutxo bootstrap snapshot, returning the bytes
/// freed (None = nothing deleted). Deletes only when BOTH the caller's own
/// persisted "loaded" flag AND the shared cross-process marker
/// (`snapshot::snapshot_marker_present`) agree the CURRENT snapshot.dat has
/// been loaded (C3, cross-process) — a stale per-app flag can never delete a
/// snapshot the OTHER app just downloaded over the shared datadir. Safe with
/// btxd RUNNING: the daemon reads snapshot.dat once during `loadtxoutset` and
/// never holds it after, so a live app can sweep the ~450 MB the moment the
/// load is confirmed instead of leaving it until the next manual reclaim.
pub fn sweep_loaded_snapshot(datadir: &Path, caller_loaded_flag: bool) -> Option<u64> {
    if !(caller_loaded_flag && crate::snapshot::snapshot_marker_present(datadir)) {
        return None;
    }
    let snap = datadir.join("faststart").join("snapshot.dat");
    let bytes = std::fs::metadata(&snap).ok()?.len();
    std::fs::remove_file(&snap).ok()?;
    Some(bytes)
}

/// Chain/node directories that lite-pool can safely reclaim. NEVER includes
/// `wallets` (self-custody keys) or `easybtx-state.json` (settings/payout).
const NODE_DATA_DIRS: &[&str] = &["blocks", "chainstate", "indexes", "faststart"];

/// Remove the on-disk BTX node + chain to reclaim space, for users who only mine
/// in (node-less) Pool mode. Deletes the chain dirs in [`NODE_DATA_DIRS`] plus
/// `debug.log`, and PRESERVES `wallets/` and `easybtx-state.json`. MUST be called
/// with btxd NOT running (i.e. in Pool mode / after `stop_node`). Returns a
/// [`ReclaimReport`] with the bytes freed.
pub fn remove_node_data(datadir: &Path) -> ReclaimReport {
    let mut report = ReclaimReport::default();
    let mut freed_bytes: u64 = 0;
    for name in NODE_DATA_DIRS {
        let dir = datadir.join(name);
        if dir.is_dir() {
            let bytes = dir_size_bytes(&dir);
            if std::fs::remove_dir_all(&dir).is_ok() {
                freed_bytes += bytes;
                report.items.push(format!("{name} ({} MB)", bytes / MB));
            }
        }
    }
    let log = datadir.join("debug.log");
    if let Ok(meta) = std::fs::metadata(&log) {
        let bytes = meta.len();
        if std::fs::remove_file(&log).is_ok() {
            freed_bytes += bytes;
            report.items.push(format!("debug.log ({} MB)", bytes / MB));
        }
    }
    report.freed_mb = freed_bytes / MB;
    report
}

/// How long a repair-flow quarantine dir is kept on disk before being eligible
/// for auto-deletion. Long enough for the user to grab forensics on a failure
/// ("what blew up yesterday?"), short enough that a single Repair Node click
/// doesn't permanently leak its archived chain to disk forever. Field failure
/// this guards against: the maintainer's machine held 42 GB hostage in one
/// stale `_corrupt-*` dir because nothing ever cleaned it up post-repair.
pub const QUARANTINE_RETENTION_SECS: u64 = 7 * 24 * 60 * 60; // 7 days

/// How often the background sweeper re-runs [`prune_old_quarantines`] during a
/// long-lived session. `first_run_setup` (startup) and `repair_node` already
/// prune at their moments; this timer is the missing piece for an install that
/// stays open for DAYS without relaunching or repairing — exactly the warm-node
/// workflow the Pause/Resume control encourages — so archives can't linger past
/// retention. SAFE to run while btxd is live: quarantine dirs are never held open
/// by the node, unlike the leveldb indexes `reclaim_disk` deletes (which is why
/// that heavier reclaim stays pre-spawn-only).
pub const QUARANTINE_SWEEP_INTERVAL_SECS: u64 = 6 * 60 * 60; // every 6 hours

/// Dirname prefixes the repair flow produces under the datadir. Both leak the
/// same way — quarantine on repair, never auto-cleaned. `_corrupt-*` carries
/// archived chain data (multi-GB), `_preserve-*` carries wallet/state backups
/// (~hundreds of MB but accumulates on every repair).
const QUARANTINE_PREFIXES: &[&str] = &["_corrupt-", "_preserve-"];

#[derive(Debug, Default, Clone, PartialEq)]
pub struct QuarantinePruneReport {
    /// How many quarantine dirs were deleted (across all prefixes).
    pub removed_count: usize,
    /// Total bytes freed by the deletions.
    pub freed_bytes: u64,
    /// How many quarantine dirs were kept (newest-of-its-kind, or still
    /// inside the retention window).
    pub kept_count: usize,
}

/// Decide which quarantine entries to delete given a list of `(path, mtime_secs)`,
/// the current time `now_secs`, and the retention window. Pure → unit-testable
/// without touching the disk.
///
/// Rule: delete any entry older than the retention window EXCEPT the single
/// newest entry of its kind, which is ALWAYS kept regardless of age. The "keep
/// newest" rule guarantees the user always has yesterday's failure available
/// for forensics, even if "yesterday" was actually six months ago on a rarely
/// repaired install.
pub fn quarantine_entries_to_prune<P: Clone>(
    mut entries: Vec<(P, u64)>,
    now_secs: u64,
    retention_secs: u64,
) -> Vec<P> {
    // Sort newest-first so index 0 is the one we always keep.
    entries.sort_by_key(|(_, mtime)| std::cmp::Reverse(*mtime));
    let mut to_prune = Vec::new();
    for (idx, (path, mtime)) in entries.iter().enumerate() {
        if idx == 0 {
            continue; // always keep the newest, regardless of age
        }
        // saturating_sub handles the edge case where a future-dated mtime (clock
        // skew, fs preserved across timezone migration, etc.) would otherwise
        // wrap and the entry would be "infinitely old" → wrongly pruned.
        let age = now_secs.saturating_sub(*mtime);
        if age > retention_secs {
            to_prune.push(path.clone());
        }
    }
    to_prune
}

/// Return the directories in `datadir` whose name starts with `prefix`, paired
/// with their mtime (Unix seconds; 0 if unreadable). Helper for
/// [`prune_old_quarantines`]; factored out so the iteration is testable and
/// the prune-logic in [`quarantine_entries_to_prune`] sees a clean list.
fn collect_quarantine_dirs(datadir: &Path, prefix: &str) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(datadir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(prefix) {
            continue;
        }
        // Quarantines are always directories. A stray file with the same prefix
        // (e.g. a tester left `_corrupt-notes.txt`) is left alone — never ours
        // to delete.
        if !entry.path().is_dir() {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.push((entry.path(), mtime));
    }
    out
}

/// Delete `_corrupt-*` and `_preserve-*` quarantine directories under `datadir`
/// that are older than [`QUARANTINE_RETENTION_SECS`], keeping the single newest
/// match of each pattern as forensics. Best-effort: every error is logged but
/// never returned — disk reclaim must never block app startup or repair flow.
///
/// Why not use `mtime`'s relationship to the parsed dirname timestamp? Because
/// the dirname format has evolved (`_corrupt-<sec>` → `_corrupt-<sec>-<usec>`
/// → `_corrupt-<sec>-<usec>-<n>` for collision counters) and matching on mtime
/// covers every generation without per-format parsing.
///
/// Called from two places (commands.rs):
///   * `first_run_setup` — catches quarantines from past sessions on every cold
///     start, so the cleanup doesn't depend on the user clicking Repair again.
///   * `repair_node` — at the end of the repair, so prior repairs' archives
///     can't linger past their retention window even on an install that never
///     restarts (e.g. always running, daily repairs).
pub fn prune_old_quarantines(datadir: &Path) -> QuarantinePruneReport {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    prune_old_quarantines_at(datadir, now_secs)
}

/// Test-injectable variant of [`prune_old_quarantines`] that takes `now_secs`
/// instead of reading the system clock. Lets a test simulate "the present is
/// 30 days from now" without having to backdate mtimes (which would need a
/// non-std dep like `filetime`). Production code uses [`prune_old_quarantines`].
pub fn prune_old_quarantines_at(datadir: &Path, now_secs: u64) -> QuarantinePruneReport {
    let mut report = QuarantinePruneReport::default();
    for prefix in QUARANTINE_PREFIXES {
        let entries = collect_quarantine_dirs(datadir, prefix);
        let total = entries.len();
        let to_prune = quarantine_entries_to_prune(entries, now_secs, QUARANTINE_RETENTION_SECS);
        report.kept_count += total.saturating_sub(to_prune.len());
        for path in to_prune {
            // Measure BEFORE deletion so the freed_bytes tally is accurate.
            let bytes = dir_size_bytes(&path);
            match std::fs::remove_dir_all(&path) {
                Ok(()) => {
                    report.removed_count += 1;
                    report.freed_bytes += bytes;
                    eprintln!(
                        "[disk-quarantine] pruned {} ({} MB)",
                        path.display(),
                        bytes / MB
                    );
                }
                Err(e) => {
                    eprintln!(
                        "[disk-quarantine] could not prune {} (non-fatal): {e}",
                        path.display()
                    );
                }
            }
        }
    }
    if report.removed_count > 0 {
        eprintln!(
            "[disk-quarantine] pruned {} stale quarantine(s), freed {} MB",
            report.removed_count,
            report.freed_bytes / MB
        );
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── dir_size_bytes (the single shared walker) ───────────────────────────

    #[test]
    fn dir_size_sums_nested_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.bin"), vec![0u8; 100]).unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("b.bin"), vec![0u8; 250]).unwrap();
        std::fs::write(sub.join("c.bin"), vec![0u8; 5]).unwrap();

        assert_eq!(
            dir_size_bytes(dir.path()),
            100 + 250 + 5,
            "must sum files across nested dirs"
        );
    }

    #[test]
    fn dir_size_missing_dir_is_zero() {
        assert_eq!(dir_size_bytes(Path::new("/no/such/easybtx/path")), 0);
    }

    #[test]
    fn dir_size_empty_dir_is_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(dir_size_bytes(dir.path()), 0);
    }

    #[cfg(unix)]
    #[test]
    fn dir_size_does_not_follow_symlinks() {
        // The invariant the weaker recursive walker lacked: a symlink into a
        // sibling tree (or a self-referential loop) must NOT be traversed —
        // otherwise a datadir with a link into blocks/ would double-count, and a
        // cyclic link would recurse forever. symlink_metadata sees the link as a
        // non-dir, non-file entry, so it contributes nothing and is not followed.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("real.bin"), vec![0u8; 42]).unwrap();
        let target = dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("big.bin"), vec![0u8; 1000]).unwrap();
        // A symlink to target/ placed inside dir: a follow-symlinks walk would
        // add another 1000 bytes; ours must not.
        std::os::unix::fs::symlink(&target, dir.path().join("link")).unwrap();
        // real.bin (42) + target/big.bin (1000); the link adds nothing extra.
        assert_eq!(dir_size_bytes(dir.path()), 42 + 1000);
    }

    #[test]
    fn free_disk_mb_returns_nonzero_on_real_path() {
        // The temp dir lives on a real filesystem with statvfs support on every
        // platform we ship to (macOS, Linux). The exact MB count is environmental,
        // but it must be > 0 on a working system — and 0 specifically signals
        // "not measured" to the UI (which hides the warning banner). Catches
        // regressions where statvfs is mis-called and the field silently sticks
        // at 0 forever, leaving the user with no low-disk warning.
        let tmp = std::env::temp_dir();
        let mb = free_disk_mb(&tmp);
        assert!(
            mb > 0,
            "free_disk_mb({}) returned 0; statvfs must work on the temp dir",
            tmp.display()
        );
    }

    #[test]
    fn free_disk_mb_returns_zero_on_missing_path() {
        // A path that doesn't exist must NOT return random data — it must return
        // 0 so the UI shows "not measured" (banner hidden), never a spurious
        // critical warning.
        let missing = std::path::PathBuf::from("/this/path/should/not/exist/anywhere");
        assert_eq!(free_disk_mb(&missing), 0);
    }

    #[test]
    fn disk_thresholds_are_ordered() {
        // The UI assumes CRITICAL ≤ WARN. If someone flips them the banner
        // logic breaks (critical would never fire because warn would always
        // match first). Holds for both the miner and the node thresholds.
        assert!(DISK_CRITICAL_MB < DISK_WARN_MB);
        assert!(NODE_DISK_CRITICAL_MB < NODE_DISK_WARN_MB);
    }

    #[test]
    fn strip_removes_only_the_unused_index_lines() {
        let conf = "server=1\nprune=0\nblockfilterindex=1\ncoinstatsindex=1\nretainshieldedcommitmentindex=1\naddnode=1.2.3.4\n";
        let out = strip_unused_index_conf_str(conf);
        assert!(!out.contains("blockfilterindex"));
        assert!(!out.contains("coinstatsindex"));
        // The load-bearing lines survive.
        assert!(out.contains("prune=0"));
        assert!(out.contains("retainshieldedcommitmentindex=1"));
        assert!(out.contains("addnode=1.2.3.4"));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn strip_is_a_noop_when_indexes_absent() {
        let conf = "server=1\nprune=0\nretainshieldedcommitmentindex=1\n";
        assert_eq!(strip_unused_index_conf_str(conf), conf);
    }

    #[test]
    fn reclaim_deletes_indexes_snapshot_and_strips_conf() {
        let tmp = std::env::temp_dir().join(format!("ebtx-disk-test-{}", std::process::id()));
        let dd = tmp.join("dd");
        let fs_dir = dd.join("faststart");
        std::fs::create_dir_all(dd.join("indexes").join("blockfilter")).unwrap();
        std::fs::create_dir_all(dd.join("indexes").join("coinstats")).unwrap();
        std::fs::create_dir_all(dd.join("indexes").join("txindex")).unwrap(); // must survive
        std::fs::create_dir_all(&fs_dir).unwrap();
        std::fs::write(dd.join("indexes").join("blockfilter").join("000.ldb"), vec![0u8; 4096]).unwrap();
        std::fs::write(dd.join("indexes").join("coinstats").join("000.ldb"), vec![0u8; 4096]).unwrap();
        let conf = fs_dir.join("faststart.conf");
        std::fs::write(&conf, "prune=0\nblockfilterindex=1\ncoinstatsindex=1\n").unwrap();
        std::fs::write(fs_dir.join("snapshot.dat"), vec![0u8; 4096]).unwrap();
        // The cross-process marker must be present for snapshot.dat deletion
        // (a confirmed load wrote it); the per-app flag alone is not enough.
        crate::snapshot::mark_snapshot_marker(&dd);

        let report = reclaim_disk(&dd, &conf, true);

        assert!(!dd.join("indexes").join("blockfilter").exists());
        assert!(!dd.join("indexes").join("coinstats").exists());
        assert!(dd.join("indexes").join("txindex").exists(), "unrelated index must be preserved");
        assert!(!fs_dir.join("snapshot.dat").exists());
        let conf_after = std::fs::read_to_string(&conf).unwrap();
        assert!(!conf_after.contains("blockfilterindex"));
        assert!(conf_after.contains("prune=0"));
        // 3 items removed (two indexes + snapshot); freed_mb may be 0 (files < 1 MB).
        assert_eq!(report.items.len(), 3);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn snapshot_sweep_needs_both_flag_and_marker() {
        let tmp = std::env::temp_dir().join(format!("ebtx-sweep-test-{}", std::process::id()));
        let dd = tmp.join("dd");
        let fs_dir = dd.join("faststart");
        std::fs::create_dir_all(&fs_dir).unwrap();
        let snap = fs_dir.join("snapshot.dat");
        std::fs::write(&snap, vec![0u8; 2048]).unwrap();
        // Caller flag alone: no delete (the OTHER app may still be loading).
        assert_eq!(sweep_loaded_snapshot(&dd, true), None);
        assert!(snap.exists());
        // Marker alone: no delete (this caller hasn't confirmed its own load).
        crate::snapshot::mark_snapshot_marker(&dd);
        assert_eq!(sweep_loaded_snapshot(&dd, false), None);
        assert!(snap.exists());
        // Both agree → swept, bytes reported.
        assert_eq!(sweep_loaded_snapshot(&dd, true), Some(2048));
        assert!(!snap.exists());
        // Idempotent once gone.
        assert_eq!(sweep_loaded_snapshot(&dd, true), None);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── prune_old_quarantines / quarantine_entries_to_prune ────────────────

    #[test]
    fn quarantine_keeps_the_newest_regardless_of_age() {
        // Even an ancient archive must be kept if it's the only one of its kind —
        // forensics matter more than disk on a rarely-repaired install. The user
        // who hits Repair once a year still wants that single archive available.
        let now = 1_000_000_000u64;
        let week = 7 * 24 * 60 * 60;
        let entries = vec![("only", now.saturating_sub(week * 10))]; // very old
        let pruned = quarantine_entries_to_prune(entries, now, week);
        assert!(pruned.is_empty(), "the single newest entry must always be kept");
    }

    #[test]
    fn quarantine_prunes_older_than_retention_keeps_newest() {
        // The case from the field: one archive within the retention window
        // (kept as the newest), several stale ones (deleted). Verifies both
        // "keep newest" AND "delete >7d" hold simultaneously.
        let now = 1_000_000_000u64;
        let week = 7 * 24 * 60 * 60;
        let entries = vec![
            ("newest", now - 60),              // 1 min old — newest, always kept
            ("old1", now - (week + 100)),      // 7d+100s — prune
            ("old2", now - (week * 4)),        // 28d — prune
            ("recent_but_not_newest", now - (week / 2)), // 3.5d — within window, kept
        ];
        let mut pruned = quarantine_entries_to_prune(entries, now, week);
        pruned.sort();
        assert_eq!(pruned, vec!["old1", "old2"]);
    }

    #[test]
    fn quarantine_handles_clock_skew_safely() {
        // An mtime in the FUTURE (clock skew / restored backup with old time)
        // must not wrap subtraction and incorrectly prune. saturating_sub keeps
        // age = 0, so the entry survives. Verifies the future-dated entry is
        // even treated as "newest" (Reverse sort puts it on top → kept).
        let now = 1_000_000_000u64;
        let week = 7 * 24 * 60 * 60;
        let entries = vec![
            ("future", now + 10_000),     // mtime > now (clock skew)
            ("normal_old", now - (week + 1)),
        ];
        let pruned = quarantine_entries_to_prune(entries, now, week);
        // The future-dated entry sorts as newest → always kept. The normal old
        // one IS pruned (it's not the newest and is past retention).
        assert_eq!(pruned, vec!["normal_old"]);
    }

    #[test]
    fn prune_handles_missing_datadir_gracefully() {
        // First-run case: the datadir hasn't been created yet. Must return an
        // empty report without panicking — startup ordering can call this
        // before `first_run_setup` ever creates the directory.
        let missing = std::path::PathBuf::from("/this/path/does/not/exist/ebtx-test");
        let report = prune_old_quarantines(&missing);
        assert_eq!(report, QuarantinePruneReport::default());
    }

    #[test]
    fn prune_skips_non_quarantine_entries_and_files() {
        // The datadir contains plenty of dirs we must NEVER touch (blocks,
        // chainstate, wallets, faststart, etc.) — only `_corrupt-*` and
        // `_preserve-*` dirs are ours. Also: a stray FILE with a matching
        // prefix (e.g. a tester's `_corrupt-notes.txt`) must be left alone.
        let tmp = std::env::temp_dir().join(format!("ebtx-quar-skip-{}", std::process::id()));
        let dd = tmp.join("dd");
        std::fs::create_dir_all(dd.join("blocks")).unwrap();
        std::fs::create_dir_all(dd.join("chainstate")).unwrap();
        std::fs::create_dir_all(dd.join("miner")).unwrap();
        // Stray file with quarantine prefix — must survive (not a dir).
        std::fs::write(dd.join("_corrupt-notes.txt"), b"keep me").unwrap();

        let report = prune_old_quarantines(&dd);

        assert_eq!(report.removed_count, 0);
        assert!(dd.join("blocks").is_dir(), "chain data must be untouched");
        assert!(dd.join("chainstate").is_dir(), "chain data must be untouched");
        assert!(dd.join("miner").is_dir(), "wallet must be untouched");
        assert!(dd.join("_corrupt-notes.txt").is_file(), "stray file with prefix must survive");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn prune_deletes_stale_corrupt_and_preserve_dirs_end_to_end() {
        // Full e2e: create a datadir with several quarantine dirs of varying
        // ages, then call `prune_old_quarantines_at` with a synthetic "now" 30
        // days in the future. The newest of EACH pattern must survive — even
        // if the ancient corrupt one is older than the fresh preserve one.
        // Verifies per-pattern "keep newest", not a single global newest.
        //
        // We use `_at` (the time-injected variant) instead of backdating
        // mtimes: backdating would need `filetime` (non-std dep). Since all
        // dirs are freshly created, their mtimes are within seconds of each
        // other — the sort still produces a stable newest-per-pattern, and
        // jumping `now` forward by 30 days makes EVERY non-newest dir "old".
        let tmp = std::env::temp_dir().join(format!("ebtx-quar-e2e-{}", std::process::id()));
        let dd = tmp.join("dd");
        std::fs::create_dir_all(&dd).unwrap();

        // Create in order so sort-by-mtime puts each newer dir ahead of older.
        // Sleep 1.1s between dirs: macOS reports directory mtimes at SECOND
        // granularity via `Metadata::modified()` even on APFS (which stores
        // nanosecond mtimes natively but the std libc bridge often rounds).
        // 1.1s guarantees distinct integer-second mtimes, so the stable sort
        // doesn't fall back to read_dir order (non-deterministic on macOS).
        let mk = |name: &str| -> PathBuf {
            let p = dd.join(name);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("blob"), vec![0u8; 4096]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100));
            p
        };

        let old_corrupt = mk("_corrupt-1700000000-100"); // created first → oldest
        let new_corrupt = mk("_corrupt-1700000000-200"); // newest of _corrupt-*
        let old_preserve = mk("_preserve-1700000000-100");
        let new_preserve = mk("_preserve-1700000000-200"); // newest of _preserve-*
        std::fs::create_dir_all(dd.join("blocks")).unwrap(); // must NEVER be touched

        // Simulate "now" being 30 days from when these dirs were created. That
        // makes the non-newest ones (older by ~20ms) safely past the 7-day
        // retention threshold relative to our synthetic now.
        let synthetic_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 30 * 24 * 60 * 60;

        let report = prune_old_quarantines_at(&dd, synthetic_now);

        assert_eq!(report.removed_count, 2, "exactly 2 stale dirs should be pruned");
        assert_eq!(report.kept_count, 2, "exactly 2 newest dirs should be kept");
        assert!(new_corrupt.exists(), "newest _corrupt-* must survive");
        assert!(new_preserve.exists(), "newest _preserve-* must survive");
        assert!(!old_corrupt.exists(), "older _corrupt-* should be removed");
        assert!(!old_preserve.exists(), "older _preserve-* should be removed");
        assert!(dd.join("blocks").exists(), "chain data must NEVER be touched");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn remove_node_data_deletes_chain_but_keeps_wallets_and_state() {
        let tmp = std::env::temp_dir().join(format!("ebtx-remove-{}", std::process::id()));
        let dd = tmp.join("datadir");
        // Chain data (must be removed):
        std::fs::create_dir_all(dd.join("blocks")).unwrap();
        std::fs::create_dir_all(dd.join("chainstate")).unwrap();
        std::fs::create_dir_all(dd.join("indexes")).unwrap();
        std::fs::create_dir_all(dd.join("faststart")).unwrap();
        std::fs::write(dd.join("faststart").join("snapshot.dat"), b"xxxx").unwrap();
        std::fs::write(dd.join("debug.log"), b"logloglog").unwrap();
        // User data (must be preserved):
        std::fs::create_dir_all(dd.join("wallets").join("miner")).unwrap();
        std::fs::write(dd.join("wallets").join("miner").join("wallet.dat"), b"keys").unwrap();
        std::fs::write(dd.join("easybtx-state.json"), b"{}").unwrap();

        let report = remove_node_data(&dd);

        assert!(!dd.join("blocks").exists(), "blocks must be removed");
        assert!(!dd.join("chainstate").exists(), "chainstate must be removed");
        assert!(!dd.join("indexes").exists(), "indexes must be removed");
        assert!(!dd.join("faststart").exists(), "faststart dir must be removed");
        assert!(!dd.join("debug.log").exists(), "debug.log must be removed");
        assert!(dd.join("wallets").join("miner").join("wallet.dat").exists(), "WALLET MUST SURVIVE");
        assert!(dd.join("easybtx-state.json").exists(), "state must survive");
        assert!(!report.items.is_empty(), "at least one deletion should be recorded");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn reclaim_keeps_snapshot_until_loaded() {
        let tmp = std::env::temp_dir().join(format!("ebtx-disk-snap-{}", std::process::id()));
        let dd = tmp.join("dd");
        let fs_dir = dd.join("faststart");
        std::fs::create_dir_all(&fs_dir).unwrap();
        std::fs::write(fs_dir.join("faststart.conf"), "prune=0\n").unwrap();
        std::fs::write(fs_dir.join("snapshot.dat"), vec![0u8; 4096]).unwrap();

        let report = reclaim_disk(&dd, &fs_dir.join("faststart.conf"), false);
        assert!(fs_dir.join("snapshot.dat").exists(), "snapshot kept while not loaded");
        assert!(report.items.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn reclaim_keeps_snapshot_when_flag_true_but_marker_absent() {
        // The cross-process guard (C3): a stale per-app flag alone must NOT
        // delete snapshot.dat when the shared marker is absent — this is exactly
        // the case where the OTHER app just downloaded a fresh snapshot (which
        // clears the marker) while this app's persisted flag is still true from a
        // previous load.
        let tmp = std::env::temp_dir().join(format!("ebtx-disk-xproc-{}", std::process::id()));
        let dd = tmp.join("dd");
        let fs_dir = dd.join("faststart");
        std::fs::create_dir_all(&fs_dir).unwrap();
        std::fs::write(fs_dir.join("faststart.conf"), "prune=0\n").unwrap();
        std::fs::write(fs_dir.join("snapshot.dat"), vec![0u8; 4096]).unwrap();
        // No marker written (a fresh download cleared it). Flag says "loaded".
        assert!(!crate::snapshot::snapshot_marker_present(&dd));

        let report = reclaim_disk(&dd, &fs_dir.join("faststart.conf"), true);
        assert!(
            fs_dir.join("snapshot.dat").exists(),
            "stale flag alone must not delete a snapshot the other app is loading"
        );
        assert!(report.items.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
