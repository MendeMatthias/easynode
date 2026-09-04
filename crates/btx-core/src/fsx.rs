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
//!   * **Two processes rewriting one file.** The datadir is shared with the
//!     miner by design, and `commands.rs` says so explicitly where it re-asserts
//!     `txindex`: that re-assertion exists to "self-heal a conf the MINER
//!     rewrote". Two unsynchronised read-modify-rewrites lose content with no
//!     crash at all.
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
/// filesystem. A best-effort cleanup removes it if the rename fails, so a
/// failed write does not litter the datadir.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tmp".to_string());
    let tmp = dir.join(format!(".{name}.tmp"));

    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
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
    fn a_failed_write_leaves_the_original_intact() {
        // The whole point. `std::fs::write` truncates first, so a failure here
        // used to leave an empty conf and the caller discarded the error.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("conf");
        std::fs::write(&p, "prune=0\n").unwrap();

        // A directory in the temp file's place makes File::create fail without
        // needing a full disk or a permissions dance that root ignores.
        let tmp = d.path().join(".conf.tmp");
        std::fs::create_dir(&tmp).unwrap();

        assert!(atomic_write(&p, b"replacement").is_err());
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "prune=0\n",
            "the original must survive a failed write"
        );
    }
}
