//! "Fetch a stuck block": which blocks to ask for, and which peer to ask.
//!
//! For a node that knows of blocks it does not have and is asking nobody for
//! them. The watchdog names the fix (`StallClass::BlockFetchGated`); on
//! 2026-09-29 asking an archive peer by name moved a stuck mirror at about 940
//! blocks a minute. This button is for a stuck tip, so it asks for at most
//! [`MAX_BLOCKS`].

use crate::fork::ChainTip;
use crate::node_api::PeerInfo;

pub const MAX_BLOCKS: usize = 16;
/// Farther than this and the header walk is not what a quick action is for.
pub const MAX_WALK: u64 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FetchRequest {
    pub height: u64,
    pub hash: String,
    pub peer_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchPlan {
    Ask(Vec<FetchRequest>),
    Nothing(String),
}

/// The header tip to follow: the highest one above `blocks` that is not invalid.
pub fn target_tip(tips: &[ChainTip], blocks: u64) -> Option<&ChainTip> {
    tips.iter()
        .filter(|t| t.height > blocks && matches!(t.status.as_str(), "headers-only" | "valid-headers" | "valid-fork"))
        .max_by_key(|t| t.height)
}

/// `missing`: (height, hash) above the tip on the followed chain, lowest first.
pub fn plan(missing: &[(u64, String)], peers: &[PeerInfo]) -> FetchPlan {
    if missing.is_empty() {
        return FetchPlan::Nothing("Your node has every block it knows of. Nothing to fetch.".into());
    }
    let mut ranked: Vec<&PeerInfo> = peers.iter().collect();
    ranked.sort_by_key(|p| rank(p));
    let reqs: Vec<FetchRequest> = missing
        .iter()
        .take(MAX_BLOCKS)
        .filter_map(|(height, hash)| {
            ranked
                .iter()
                .find(|p| p.synced_headers >= *height as i64)
                .map(|p| FetchRequest { height: *height, hash: hash.clone(), peer_id: p.id })
        })
        .collect();
    if reqs.is_empty() {
        FetchPlan::Nothing(
            "No connected peer has announced the next block yet. Give it a few minutes.".into(),
        )
    } else {
        FetchPlan::Ask(reqs)
    }
}

fn rank(p: &PeerInfo) -> u8 {
    if p.connection_type == "manual" {
        0
    } else if !p.inbound {
        1
    } else {
        2
    }
}

pub fn summary(reqs: &[FetchRequest]) -> String {
    let peers: std::collections::BTreeSet<i64> = reqs.iter().map(|r| r.peer_id).collect();
    let (np, nb) = (peers.len(), reqs.len());
    format!(
        "Asked {np} {} for {nb} {}.",
        if np == 1 { "peer" } else { "peers" },
        if nb == 1 { "block" } else { "blocks" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: i64, ct: &str, inbound: bool, headers: i64) -> PeerInfo {
        PeerInfo { id, connection_type: ct.into(), inbound, synced_headers: headers, ..Default::default() }
    }
    fn tip(height: u64, status: &str) -> ChainTip {
        ChainTip { height, hash: format!("{height:064x}"), branchlen: 1, status: status.into(), ..Default::default() }
    }
    fn missing(from: u64, n: u64) -> Vec<(u64, String)> {
        (from..from + n).map(|h| (h, format!("{h:064x}"))).collect()
    }

    #[test]
    fn asks_the_app_s_own_peers_first_then_outbound_then_inbound() {
        let peers = [peer(1, "inbound", true, 300), peer(2, "outbound-full-relay", false, 300), peer(3, "manual", false, 300)];
        let FetchPlan::Ask(reqs) = plan(&missing(101, 3), &peers) else { panic!() };
        assert!(reqs.iter().all(|r| r.peer_id == 3));
        let FetchPlan::Ask(reqs) = plan(&missing(101, 1), &peers[..2]) else { panic!() };
        assert_eq!(reqs[0].peer_id, 2);
    }

    #[test]
    fn only_peers_that_announced_the_block_are_asked() {
        let peers = [peer(3, "manual", false, 101), peer(2, "outbound-full-relay", false, 110)];
        let FetchPlan::Ask(reqs) = plan(&missing(101, 3), &peers) else { panic!() };
        assert_eq!(reqs.iter().map(|r| r.peer_id).collect::<Vec<_>>(), vec![3, 2, 2]);
    }

    #[test]
    fn at_most_sixteen_blocks() {
        let FetchPlan::Ask(reqs) = plan(&missing(101, 40), &[peer(3, "manual", false, 1_000)]) else { panic!() };
        assert_eq!(reqs.len(), MAX_BLOCKS);
        assert_eq!(reqs[0].height, 101);
    }

    #[test]
    fn says_so_when_there_is_nothing_to_do() {
        assert!(matches!(plan(&[], &[peer(3, "manual", false, 1)]), FetchPlan::Nothing(s) if s.contains("every block")));
        assert!(matches!(plan(&missing(101, 2), &[peer(3, "manual", false, 100)]), FetchPlan::Nothing(s) if s.contains("No connected peer")));
    }

    #[test]
    fn follows_the_highest_tip_that_is_not_invalid() {
        let tips = [tip(100, "active"), tip(233_453, "headers-only"), tip(233_467, "valid-headers"), tip(240_000, "invalid")];
        assert_eq!(target_tip(&tips, 100).unwrap().height, 233_467);
        assert!(target_tip(&[tip(100, "active")], 100).is_none());
    }

    #[test]
    fn the_summary_counts_peers_and_blocks() {
        let reqs = vec![
            FetchRequest { height: 1, hash: "a".into(), peer_id: 3 },
            FetchRequest { height: 2, hash: "b".into(), peer_id: 2 },
        ];
        assert_eq!(summary(&reqs), "Asked 2 peers for 2 blocks.");
        assert_eq!(summary(&reqs[..1]), "Asked 1 peer for 1 block.");
    }
}
