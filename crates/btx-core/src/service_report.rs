//! The local service report — the opt-in seed of the Keepers idea.
//!
//! A node that gives the network something (uptime, blocks served, signed
//! confirmations passed on) currently has no way to SHOW it. This module
//! writes a small JSON snapshot next to the datadir that a future dashboard,
//! leaderboard, or the user themselves can read. LOCAL FILE ONLY — nothing in
//! this module (or anywhere else in the app) phones home; publishing a report
//! somewhere is a separate, explicit, future feature with its own consent.
//!
//! Written atomically (tmp + rename) so a reader never sees a torn file.

use crate::error::{AppError, AppResult};
use crate::node_api::{ArchivePeerSummary, TxRelayHealth};
use crate::watchdog::StallVerdict;
use serde::Serialize;
use std::path::Path;

pub const SERVICE_REPORT_FILE: &str = "service-report.json";

/// Bump when the shape changes so readers can dispatch.
///
/// 2 adds `nickname`. 3 adds `tx_relay`.
pub const SERVICE_REPORT_SCHEMA: u32 = 3;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ServiceReport {
    pub schema: u32,
    /// Unix seconds when this snapshot was written.
    pub written_at_unix: u64,
    /// Seconds the current node run has been up.
    pub uptime_secs: u64,
    pub blocks: u64,
    pub headers: u64,
    pub peers: i64,
    /// Total bytes uploaded to peers this run (`getnettotals`).
    pub bytes_sent: Option<u64>,
    /// The trusted-mirror archive-peer summary, when measured.
    pub archive_peers: Option<ArchivePeerSummary>,
    /// Whether this node is actually exchanging TRANSACTIONS with its peers.
    ///
    /// Every other number here can look perfect while this one is `isolated`:
    /// synced, well connected, serving blocks, and never once receiving a
    /// transaction. Measured across two independent public BTX nodes on
    /// 2026-09-06, that was the real state, and nothing anywhere reported it
    /// because nothing was measuring it. `None` means the peer census did not
    /// run this tick; `unknown` means it ran and could not yet tell.
    pub tx_relay: Option<TxRelayHealth>,
    /// The stall discriminator's current verdict, if any.
    pub stall: Option<StallVerdict>,
    /// Whether this node runs as a trusted mirror.
    pub trusted_mirror: bool,
    /// Whether this node serves historical attestations
    /// (`matmulattestationserve=1` asserted by the app).
    pub serving_attestations: bool,
    /// The operator's chosen public nickname, empty when they have not set one.
    ///
    /// Already public by construction: it is broadcast to every peer in the user
    /// agent, so a local file that records it discloses nothing new. It is here
    /// because this report is the seed for a keepers dashboard that READS it,
    /// and a dashboard of anonymous rows is the problem the nickname exists to
    /// solve.
    pub nickname: String,
}

impl ServiceReport {
    pub fn new(written_at_unix: u64) -> Self {
        Self {
            schema: SERVICE_REPORT_SCHEMA,
            written_at_unix,
            uptime_secs: 0,
            blocks: 0,
            headers: 0,
            peers: 0,
            bytes_sent: None,
            archive_peers: None,
            tx_relay: None,
            stall: None,
            trusted_mirror: false,
            serving_attestations: false,
            nickname: String::new(),
        }
    }
}

/// Atomic write of the report into `datadir/service-report.json`.
pub fn write_service_report(datadir: &Path, report: &ServiceReport) -> AppResult<()> {
    let path = datadir.join(SERVICE_REPORT_FILE);
    let tmp = datadir.join(format!("{SERVICE_REPORT_FILE}.tmp"));
    let body = serde_json::to_vec_pretty(report)
        .map_err(|e| AppError::Config(format!("service report encode: {e}")))?;
    {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&tmp).map_err(|e| {
            AppError::Config(format!("service report create {}: {e}", tmp.display()))
        })?;
        f.write_all(&body).map_err(|e| {
            AppError::Config(format!("service report write {}: {e}", tmp.display()))
        })?;
        // fsync BEFORE the rename: rename is atomic for the NAME, not the
        // bytes — without this a crash can leave the report renamed into
        // place with empty/torn content on some filesystems.
        f.sync_all()
            .map_err(|e| AppError::Config(format!("service report sync {}: {e}", tmp.display())))?;
    }
    std::fs::rename(&tmp, &path)
        .map_err(|e| AppError::Config(format!("service report rename {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_atomically_and_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = ServiceReport::new(1_766_000_000);
        r.uptime_secs = 3600;
        r.blocks = 191_583;
        r.headers = 191_583;
        r.peers = 17;
        r.bytes_sent = Some(1_900_000);
        r.trusted_mirror = true;
        r.serving_attestations = true;
        r.nickname = "Byron Bay node".into();
        write_service_report(tmp.path(), &r).unwrap();

        let read: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join(SERVICE_REPORT_FILE)).unwrap(),
        )
        .unwrap();
        // Pinned against the constant, not a literal: a reader dispatches on
        // this, and the point of bumping it is that it moves with the shape.
        assert_eq!(read["schema"], SERVICE_REPORT_SCHEMA);
        assert_eq!(read["blocks"], 191_583);
        assert_eq!(read["serving_attestations"], true);
        assert_eq!(read["nickname"], "Byron Bay node");
        // No tmp file left behind.
        assert!(!tmp
            .path()
            .join(format!("{SERVICE_REPORT_FILE}.tmp"))
            .exists());
    }
}
