//! Shared datadir resolution for every easyBTX app.
//!
//! DESIGN DECISION (easyBTX Node spec, risk #1): the miner and the standalone
//! node app share ONE datadir — the BTX chain is ~124 GiB of block payload
//! un-pruned (measured 2026-09-04, see `docs/archival-capacity.md`) and
//! duplicating it per-app is unacceptable. Both apps therefore resolve it through
//! this module: the default `~/.easybtx`, overridable via the `~/.easybtx-location`
//! file (written by the miner's Settings → "Move data" flow). Whichever app the
//! user relocates the data from, the other app follows automatically.
//!
//! Cross-app coordination for the btxd process itself lives in [`crate::node`]
//! (pidfile `easybtx-node.pid` + foreign-node reconciliation).

use std::path::PathBuf;

/// File in $HOME (OUTSIDE the datadir, so it survives a relocation) that records
/// a custom datadir path chosen via Settings. Absent/empty => default ~/.easybtx.
pub fn location_file() -> PathBuf {
    let home = crate::platform::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".easybtx-location")
}

/// The default datadir (`~/.easybtx`) used when no custom location is set.
pub fn default_datadir() -> PathBuf {
    crate::platform::data_dir().unwrap_or_else(|| PathBuf::from(".easybtx"))
}

/// The ACTIVE easyBTX datadir: a custom location from `.easybtx-location` if set
/// and non-empty, else the default `~/.easybtx`. Read fresh each call (cheap) so a
/// relocation (e.g. to an external SSD) takes effect for every datadir use.
pub fn easybtx_datadir() -> PathBuf {
    if let Ok(s) = std::fs::read_to_string(location_file()) {
        let p = s.trim();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    default_datadir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_datadir_ends_with_easybtx() {
        let d = default_datadir();
        assert!(
            d.to_string_lossy().to_lowercase().ends_with("easybtx"),
            "default datadir should end in (.)easybtx, got {d:?}"
        );
    }

    #[test]
    fn location_file_lives_in_home_not_datadir() {
        let f = location_file();
        assert!(
            f.to_string_lossy().ends_with(".easybtx-location"),
            "unexpected location file {f:?}"
        );
        // It must NOT live inside the datadir it points at (it has to survive a move).
        assert_ne!(f.parent(), Some(default_datadir().as_path()));
    }
}
