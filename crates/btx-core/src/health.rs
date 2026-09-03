use crate::node_api::{BlockchainInfo, ChainStates, MiningInfo};

#[derive(Debug, Clone, PartialEq)]
pub enum MineDecision {
    Mine,
    Pause(String),
}

/// Sync readiness derived from `getchainstates` + `getblockchaininfo`.
///
/// On an assumeutxo fast-start node `getblockchaininfo` reports the BACKGROUND
/// chainstate (height 0, progress ~0, initialblockdownload=true) for a long time
/// while the snapshot chainstate is already usable. This type lets the UI reflect
/// the BEST (snapshot) chainstate so it doesn't appear stuck at 0%.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncReadiness {
    /// Snapshot chainstate is loaded at/above the snapshot height (or the node is
    /// genuinely at tip): wallet + mining usable. Carries the best height + progress.
    NearTip { height: u64, progress: f64 },
    /// Still syncing — no usable snapshot yet. Carries best-known height + progress.
    Syncing { height: u64, progress: f64 },
}

impl SyncReadiness {
    pub fn is_near_tip(&self) -> bool {
        matches!(self, SyncReadiness::NearTip { .. })
    }
    pub fn height(&self) -> u64 {
        match self {
            SyncReadiness::NearTip { height, .. } | SyncReadiness::Syncing { height, .. } => {
                *height
            }
        }
    }
    pub fn progress(&self) -> f64 {
        match self {
            SyncReadiness::NearTip { progress, .. } | SyncReadiness::Syncing { progress, .. } => {
                *progress
            }
        }
    }
}

/// Decide sync readiness from chainstates + blockchain info.
///
/// The node is "near tip / ready to mine" when EITHER:
///   - an assumeutxo snapshot chainstate has loaded at/above `min_snapshot_height`
///     (the snapshot chainstate is usable even though a background chainstate is
///     still validating from 0 and `initialblockdownload` is still true), OR
///   - the node is genuinely out of initial block download (normal full sync).
///
/// `min_snapshot_height` is the manifest's snapshot height; pass 0 to accept any
/// loaded snapshot. `chain` provides the genuine-IBD fallback signal.
pub fn sync_readiness(
    chainstates: &ChainStates,
    chain: &BlockchainInfo,
    min_snapshot_height: u64,
) -> SyncReadiness {
    // Best (snapshot-preferred) numbers for display; fall back to chain info when
    // chainstates has nothing useful (shouldn't happen, but keep it robust).
    let height = chainstates
        .best_height()
        .max(if chainstates.chainstates.is_empty() {
            chain.blocks
        } else {
            0
        });
    let progress = if chainstates.chainstates.is_empty() {
        chain.verification_progress
    } else {
        chainstates.best_verification_progress()
    };

    let snapshot_ready = chainstates.snapshot_ready(min_snapshot_height);
    if snapshot_ready || !chain.initial_block_download {
        SyncReadiness::NearTip { height, progress }
    } else {
        SyncReadiness::Syncing { height, progress }
    }
}

/// Pure decision: mine only when synced (near tip) and the node's own chain
/// guard does not ask us to pause. Mirrors `contrib/mining/live-mining-loop.sh`.
///
/// `near_tip` is the assumeutxo-aware readiness signal (see [`sync_readiness`]).
/// On a fast-started node `chain.initial_block_download` stays true for a long
/// time while the BACKGROUND chainstate validates from 0, so we must NOT gate
/// solely on `initialblockdownload` — the snapshot chainstate is already usable.
/// The node's own `chain_guard` remains the authoritative "is it safe to mine
/// right now" signal (peer consensus, tip freshness), so we still honor it.
pub fn decide(chain: &BlockchainInfo, mining: &MiningInfo, near_tip: bool) -> MineDecision {
    // Only treat genuine IBD (no usable snapshot) as a pause-worthy sync state.
    if chain.initial_block_download && !near_tip {
        return MineDecision::Pause("syncing".into());
    }
    let cg = &mining.chain_guard;
    if cg.enabled && cg.should_pause_mining {
        let reason = if cg.reason.is_empty() {
            "chain guard pause".into()
        } else {
            cg.reason.clone()
        };
        return MineDecision::Pause(reason);
    }
    MineDecision::Mine
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_api::{ChainGuard, ChainStates, ChainstateEntry};

    fn chain(ibd: bool) -> BlockchainInfo {
        BlockchainInfo {
            blocks: 100,
            headers: 100,
            verification_progress: 1.0,
            initial_block_download: ibd,
            median_time: 0, // unknown, which tip_is_stale reads as "not stale"
            is_stale: false,
            behind_best_header: 0,
        }
    }
    fn mining(enabled: bool, pause: bool, reason: &str) -> MiningInfo {
        MiningInfo {
            blocks: 100,
            difficulty: 1.0,
            network_hashps: 1.0,
            chain: "main".into(),
            chain_guard: ChainGuard {
                enabled,
                healthy: !pause,
                should_pause_mining: pause,
                reason: reason.into(),
                peer_count: 8,
                near_tip_peers: 6,
                local_tip: 100,
            },
        }
    }

    // Helpers for chainstates fixtures.
    fn snapshot_entry(height: u64) -> ChainstateEntry {
        ChainstateEntry {
            blocks: height,
            bestblockhash: "88a7".into(),
            verification_progress: 0.999,
            snapshot_blockhash: Some("88a7".into()),
            validated: false,
        }
    }
    fn ibd_entry(height: u64) -> ChainstateEntry {
        ChainstateEntry {
            blocks: height,
            bestblockhash: "75a9".into(),
            verification_progress: 7.5e-6,
            snapshot_blockhash: None,
            validated: true,
        }
    }

    #[test]
    fn pauses_while_syncing() {
        // Genuine IBD, no usable snapshot → pause.
        assert_eq!(
            decide(&chain(true), &mining(true, false, "ok"), false),
            MineDecision::Pause("syncing".into())
        );
    }
    #[test]
    fn pauses_when_chain_guard_says_so() {
        assert_eq!(
            decide(
                &chain(false),
                &mining(true, true, "insufficient_peer_consensus"),
                true
            ),
            MineDecision::Pause("insufficient_peer_consensus".into())
        );
    }
    #[test]
    fn mines_when_synced_and_healthy() {
        assert_eq!(
            decide(&chain(false), &mining(true, false, "ok"), true),
            MineDecision::Mine
        );
    }
    #[test]
    fn mines_when_chain_guard_disabled() {
        assert_eq!(
            decide(&chain(false), &mining(false, true, "ignored"), true),
            MineDecision::Mine
        );
    }

    /// The assumeutxo fix: blockchaininfo still reports IBD, but `near_tip=true`
    /// (snapshot loaded) → mining must NOT be paused on the sync gate.
    #[test]
    fn mines_in_ibd_when_snapshot_makes_it_near_tip() {
        assert_eq!(
            decide(&chain(true), &mining(true, false, "ok"), true),
            MineDecision::Mine
        );
    }

    // ── sync_readiness ────────────────────────────────────────────────────────

    #[test]
    fn readiness_near_tip_when_snapshot_loaded_despite_ibd() {
        let cs = ChainStates {
            headers: 106875,
            chainstates: vec![ibd_entry(0), snapshot_entry(106875)],
        };
        let r = sync_readiness(&cs, &chain(true), 106875);
        assert!(r.is_near_tip(), "loaded snapshot → near tip even in IBD");
        assert_eq!(
            r.height(),
            106875,
            "height reflects the snapshot chainstate"
        );
        assert!(r.progress() > 0.99, "progress reflects snapshot, not 0%");
    }

    #[test]
    fn readiness_syncing_when_only_background_chainstate() {
        let cs = ChainStates {
            headers: 0,
            chainstates: vec![ibd_entry(0)],
        };
        let r = sync_readiness(&cs, &chain(true), 106875);
        assert!(!r.is_near_tip(), "no snapshot + IBD → still syncing");
        assert_eq!(r.height(), 0);
    }

    #[test]
    fn readiness_near_tip_on_genuine_full_sync() {
        // No snapshot at all, but the node is out of IBD (normal full sync).
        let cs = ChainStates {
            headers: 120000,
            chainstates: vec![ChainstateEntry {
                blocks: 120000,
                bestblockhash: "ffff".into(),
                verification_progress: 1.0,
                snapshot_blockhash: None,
                validated: true,
            }],
        };
        let r = sync_readiness(&cs, &chain(false), 106875);
        assert!(r.is_near_tip());
        assert_eq!(r.height(), 120000);
    }

    #[test]
    fn readiness_falls_back_to_chain_info_when_chainstates_empty() {
        // Defensive: getchainstates unavailable (empty) but node out of IBD.
        let cs = ChainStates::default();
        let r = sync_readiness(&cs, &chain(false), 106875);
        assert!(r.is_near_tip());
        // Falls back to chain.blocks (100 from the chain() fixture).
        assert_eq!(r.height(), 100);
    }
}
