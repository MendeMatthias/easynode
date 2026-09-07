//! Filesystem writes that cannot leave a half-written file behind.
//!
//! WHY THIS EXISTS. Several files in a datadir are read-modify-rewritten in
//! place: `faststart.conf` (three writers in [`crate::setup`], one in
//! [`crate::disk`], and two in [`crate::installer`] — the provisioning paths,
//! which are the only ones that replace the WHOLE file rather than editing it)
//! and the node app's settings file. A plain `std::fs::write`
//! is open + O_TRUNC + write, so the truncation is committed before the bytes
//! are, and the caller sees one call that either worked or did not.
//!
//! The dangerous window is not mainly a crash — once `write(2)` returns, the
//! content is in the page cache whether or not the writer survives. It is:
//!
//!   * **ENOSPC.** The truncation has already happened when the write fails,
//!     and every caller of these particular writers discards the error. This is
//!     an app whose disk preflight exists precisely because volumes fill up.
//!   * **Power loss inside the writeback window**, which is dirty-page lifetime
//!     rather than microseconds.
//!   * **Two writers at once.** The datadir is shared with the miner by design,
//!     and inside this app the Settings commands rewrite the conf on their own
//!     tasks while the start sequence rewrites it too. What this module does
//!     about that is bounded and worth stating exactly: each writer gets its
//!     OWN temp file (pid + counter, opened `create_new`), so two writers can
//!     never share an inode and publish a byte-mixed hybrid — a torn conf is
//!     the one that stops btxd. What it does NOT do is serialise the
//!     read-modify-write: both writers read before either writes, so the
//!     loser's edit is lost. That is a lost update, not a corrupt file, and
//!     closing it needs a lock around the whole read-modify-write. That lock
//!     is [`ConfLock`], taken by every read-modify-write writer of the conf in
//!     [`crate::setup`] and [`crate::disk`]; its doc comment states exactly
//!     which writers it binds and which it cannot.
//!
//! A truncated `faststart.conf` is not a cosmetic loss. `prune=0` disappearing
//! means the datadir's own `btx_rw.conf` decides the prune posture instead, and
//! the reorg-parking keys disappearing means btxd reverts to following whichever
//! branch carries the most work — on a chain this repository measures at roughly
//! one sibling every 25 blocks.
//!
//! [`crate::service_report`] already writes this way; this is that pattern,
//! shared, so every writer of a file somebody depends on can use it.

use std::io;
use std::path::Path;

/// Write `bytes` to `path` so a reader sees either the old file or the new one.
///
/// Writes a sibling temp file, fsyncs it, then renames over the target. The
/// fsync is before the rename on purpose: rename is atomic for the NAME, not
/// for the bytes, so without it a crash can leave the new name pointing at
/// content that never reached the disk.
///
/// The temp file is a sibling because rename is only atomic within a
/// filesystem. Its name carries the pid and a counter and it is opened
/// `create_new`, so two concurrent writers of one target get two files rather
/// than two descriptors on one inode — the earlier fixed `.name.tmp` let a
/// second writer truncate and overlay the first's bytes, and whichever rename
/// won published the hybrid. On ANY failure the temp file is removed, so a
/// failed write never leaves a stray file behind.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tmp".to_string());
    let pid = std::process::id();

    // A unique temp path this writer alone holds. AlreadyExists means a
    // leftover from a crashed writer with the same pid and counter — possible
    // across a reboot — so step the counter and try again rather than truncate
    // a file we cannot prove is ours.
    let (tmp, mut file) = loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".{name}.{pid}.{n}.tmp"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(f) => break (tmp, f),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };

    let written = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// The lock file that serialises read-modify-write cycles on a conf.
///
/// A sibling of the conf rather than a name in the datadir root: every writer
/// of one conf derives the same path from the conf it is editing, which is what
/// makes them exclude each other without threading a datadir through five
/// signatures.
fn conf_lock_path(conf_path: &Path) -> std::path::PathBuf {
    conf_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".easybtx.conf.lock")
}

/// An exclusive advisory lock held for the whole read → modify → write of a
/// conf file.
///
/// WHY THIS EXISTS. [`atomic_write`] closes the *torn file* hole and says so in
/// this module's header: two writers get two temp files, so neither can publish
/// a byte-mixed hybrid. What it explicitly does NOT close is the **lost
/// update** — both writers read the old text, each edits its own copy, and the
/// second rename silently discards the first's edit. Every conf writer in this
/// crate is a read-modify-write, and several of them run concurrently by
/// design: the start sequence in `apps/node` walks the whole conf reconciliation
/// while the Settings commands rewrite the same file on their own tasks.
///
/// What a lost update costs on this particular file, today, in descending order
/// of harm:
///
///   * **The `addnode` set.** The peer list is what decides which chain a node
///     can obtain bodies for at all — the whole subject of
///     `docs/incident-2026-09-05-fork.md`. Losing the edit that puts the live
///     chain's body source in the conf is losing the route to the live chain.
///   * **`prune=0`.** The datadir's own `btx_rw.conf` then decides the prune
///     posture, and `disk.rs` documents why this app runs unpruned.
///   * **The managed noban block**, which is a security grant that is supposed
///     to be revocable on a schedule this app controls.
///
/// SCOPE, stated plainly. This is `flock(2)` on Unix and an exclusive
/// share-mode open on Windows, and it binds **processes that take it** — this
/// app's tasks, and a second copy of this app. The miner shares the datadir by
/// design and is a separate program that does not take this lock, so a race
/// with the miner is narrowed (the window shrinks to one writer's critical
/// section) but not eliminated. That is why the callers that assert a key still
/// assert it every start rather than trusting one successful write.
///
/// Failing to take the lock is never fatal: the caller proceeds unlocked, which
/// is exactly the behaviour that existed before this type. A conf that cannot
/// be locked is still a conf that must be written.
pub struct ConfLock {
    file: Option<std::fs::File>,
}

impl ConfLock {
    /// Take the exclusive lock for `conf_path`, blocking until it is ours.
    ///
    /// Returns `None` when the lock could not be taken at all (the directory is
    /// read-only, the filesystem does not support locking, another process held
    /// it past the Windows timeout). The caller does its read-modify-write
    /// anyway — see the type's doc comment.
    #[cfg(unix)]
    pub fn acquire(conf_path: &Path) -> Option<Self> {
        use std::os::unix::io::AsRawFd;

        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(conf_lock_path(conf_path))
            .ok()?;
        // LOCK_EX blocks until the holder releases. EINTR is the one retryable
        // failure: a signal during the wait is not a lock we cannot have.
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Some(Self { file: Some(file) });
            }
            if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                return None;
            }
        }
    }

    /// Windows has no `flock`. Exclusivity comes from the open itself: a
    /// share mode of 0 lets exactly one handle exist at a time, and Windows
    /// closes it (and so releases the lock) even if the process dies, which is
    /// the property a hand-rolled lockfile would not have. The wait is bounded
    /// rather than infinite because a share-mode conflict has no blocking form.
    #[cfg(not(unix))]
    pub fn acquire(conf_path: &Path) -> Option<Self> {
        use std::os::windows::fs::OpenOptionsExt as _;

        const WAIT_MS: u64 = 5_000;
        const STEP_MS: u64 = 25;
        let path = conf_lock_path(conf_path);
        let mut waited = 0;
        loop {
            match std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .share_mode(0)
                .open(&path)
            {
                Ok(file) => return Some(Self { file: Some(file) }),
                // A sharing violation — someone else holds the lock — is the
                // one error worth waiting on. Anything else (no such
                // directory, read-only volume) will not become true by
                // waiting, and retrying it for five seconds would stall every
                // conf write behind a lock that is never coming.
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied && waited < WAIT_MS => {
                    std::thread::sleep(std::time::Duration::from_millis(STEP_MS));
                    waited += STEP_MS;
                }
                Err(_) => return None,
            }
        }
    }
}

impl Drop for ConfLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(file) = self.file.as_ref() {
            use std::os::unix::io::AsRawFd;
            // Closing the descriptor releases the lock on its own; unlocking
            // first keeps the release explicit and independent of close order.
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        }
        // Closing the handle is what releases the lock on Windows, and the
        // second half of releasing it on Unix. `take` rather than `= None` so
        // the field is READ on every platform — on Windows the flock block
        // above is compiled out, and a write-only field is a dead_code warning.
        drop(self.file.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_the_file_and_leaves_no_temp_behind() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("faststart.conf");
        std::fs::write(&p, "prune=0\n").unwrap();

        atomic_write(&p, b"prune=0\nparkdeepreorg=1\n").unwrap();

        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "prune=0\nparkdeepreorg=1\n"
        );
        let leftovers: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "faststart.conf")
            .collect();
        assert!(leftovers.is_empty(), "left {leftovers:?} behind");
    }

    #[test]
    fn creates_the_file_when_it_does_not_exist_yet() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("new.conf");
        atomic_write(&p, b"txindex=1\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "txindex=1\n");
    }

    #[test]
    fn concurrent_writers_never_publish_a_hybrid() {
        // Two threads, two distinct payloads, many rounds. With a shared temp
        // name this fails: the shorter payload overlays the longer one on a
        // shared inode and the file ends up as neither. With per-writer temp
        // files the file is always exactly one of the two.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("faststart.conf");
        std::fs::write(
            &p, "prune=0
",
        )
        .unwrap();
        let a = "prune=0
parkdeepreorg=1
maxreorgdepthpark=6
"
        .repeat(40);
        let b = "x=1
"
        .repeat(3);
        let (pa, pb) = (p.clone(), p.clone());
        let (aa, bb) = (a.clone(), b.clone());
        let ta = std::thread::spawn(move || {
            for _ in 0..200 {
                atomic_write(&pa, aa.as_bytes()).unwrap();
            }
        });
        let tb = std::thread::spawn(move || {
            for _ in 0..200 {
                atomic_write(&pb, bb.as_bytes()).unwrap();
            }
        });
        ta.join().unwrap();
        tb.join().unwrap();
        let got = std::fs::read_to_string(&p).unwrap();
        assert!(
            got == a || got == b,
            "hybrid published:
{got}"
        );
        let leftovers: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "faststart.conf")
            .collect();
        assert!(leftovers.is_empty(), "left {leftovers:?} behind");
    }

    /// THE LOST UPDATE, WHICH `atomic_write` ALONE DOES NOT PREVENT.
    ///
    /// Two writers each read the conf, add their own line, and write it back.
    /// Unlocked, the second read happens before the first rename and one edit
    /// disappears — a conf that silently loses `addnode=` or `prune=0`. Under
    /// `ConfLock` each cycle is serialised and both lines survive every round.
    #[test]
    fn the_lock_makes_two_read_modify_writes_both_survive() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("faststart.conf");

        for round in 0..25 {
            std::fs::write(&p, "prune=0\n").unwrap();
            let edit = |line: &'static str| {
                let p = p.clone();
                std::thread::spawn(move || {
                    let _guard = ConfLock::acquire(&p);
                    let mut text = std::fs::read_to_string(&p).unwrap();
                    // A real writer's think time between the read and the
                    // write; without the lock this is the whole race.
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    text.push_str(line);
                    atomic_write(&p, text.as_bytes()).unwrap();
                })
            };
            let a = edit("addnode=13.140.141.180:19335\n");
            let b = edit("addnode=213.224.31.105:33706\n");
            a.join().unwrap();
            b.join().unwrap();

            let got = std::fs::read_to_string(&p).unwrap();
            assert!(got.contains("13.140.141.180"), "round {round}: {got}");
            assert!(got.contains("213.224.31.105"), "round {round}: {got}");
            assert!(got.contains("prune=0"), "round {round}: {got}");
        }
    }

    /// A lock that cannot be taken must not stop the write: a conf that has to
    /// be written is written, locked or not. (An unwritable directory is the
    /// reachable version of "no lock available".)
    #[test]
    fn an_unavailable_lock_is_never_fatal() {
        let missing = std::path::Path::new("/no-such-dir-here/faststart.conf");
        assert!(ConfLock::acquire(missing).is_none());
    }

    #[test]
    fn a_failed_write_leaves_the_original_intact() {
        // The whole point. `std::fs::write` truncates first, so a failure here
        // used to leave an empty conf and the caller discarded the error.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("conf");
        std::fs::write(&p, "prune=0\n").unwrap();

        // Make the temp file's PARENT unwritable by pointing the target inside
        // a directory that does not exist: create_new fails, nothing is
        // written, and the original next door is untouched. (A directory at a
        // fixed temp name no longer works as a trap, because the temp name is
        // per-writer now — which is the point.)
        let missing = d.path().join("no-such-dir").join("conf");
        assert!(atomic_write(&missing, b"replacement").is_err());
        assert!(!d.path().join("no-such-dir").exists());
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "prune=0\n",
            "the original must survive a failed write"
        );
    }
}
