//! Filesystem writes that cannot leave a half-written file behind.
//!
//! WHY THIS EXISTS. Several files in a datadir are read-modify-rewritten in
//! place: `faststart.conf` (three writers in [`crate::setup`], one in
//! [`crate::disk`]) and the node app's settings file. A plain `std::fs::write`
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
//!     closing it needs a lock around the whole read-modify-write, which is a
//!     separate decision.
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
