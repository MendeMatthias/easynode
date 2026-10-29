//! How high the chain this node is on reaches, as far as the node can tell
//! from things nobody can fake: the one number every "blocks behind" reader
//! uses (docs/decisions/2026-10-12-network-height.md).
//!
//! WHY IT EXISTS. From mainnet block 244,000 the engine (v0.34.15) no longer
//! lets a header that has only passed the cheap compact-target check become
//! the best header (`AddToBlockIndex`, `phase1_must_not_lead`;
//! `MatMulDigestBelowTargetAuthenticatesWork` always says no). Only a header
//! whose block was checked, or that a pinned key signed, can lead. So while a
//! node catches up, `getblockchaininfo.headers` stays close to `blocks`, and
//! `headers - blocks` under-counts: on a test install the app said 4,888
//! blocks behind when the real gap was about 5,260. Every reader of that gap
//! went quiet with it: the catch-up help, the stall diagnosis, the warning
//! about falling behind the signers.
//!
//! THE RULE. The highest of three heights, each one something a stranger
//! cannot invent:
//!
//! * `blocks`, the node's own tip;
//! * `headers`, the engine's best header: since 244,000 one it checked or one
//!   a pinned key signed;
//! * the signed frontier (`getblockchaininfo.matmul_signed_frontier`, or
//!   `getmatmulattestedtip.signed_frontier`), the highest height a pinned key
//!   signed, counted only while `on_active_chain` says it is on this node's
//!   own chain (an ancestor, the tip, or a block ahead of it). The engine
//!   reports it on every node that pins a key: every node that follows
//!   signatures, every validating node that started from a signed snapshot,
//!   and every signer. Measured on btxscan's btxd2 (0.34.15) on 2026-10-11 at
//!   21:5x UTC: frontier 245,546, `on_active_chain` true, the same object in
//!   both RPCs.
//!
//! WHAT IT REFUSES TO COUNT, AND WHY. A header that only passed the cheap
//! check costs nothing to make, so any peer can hand a node a tower of them
//! on top of its own tip. Peers' `synced_headers` and `headers-only` chain
//! tips both count such headers, and the 2026-09-23 split (the false tower
//! after 227,313) and the 2026-09-06 header tower are what reading them
//! unfiltered looks like. They are not sources here. They are also poor
//! witnesses when honest: on btxd2, 8 of 27 peers reported `synced_headers`
//! -1, the 3060 among them, and `getchaintips` listed 2,905 tips.
//!
//! A node that pins no key (one that started long ago from the snapshot
//! compiled into the engine and signs nothing) has no frontier and reads as
//! it did before: `headers`.

use crate::node_api::AttestedTip;

/// The frontier's height when it is on this node's own chain, else `None`.
/// A frontier on another branch is a signer somewhere else, never a height
/// this node is behind.
pub fn signed_height_on_chain(frontier: Option<&AttestedTip>) -> Option<u64> {
    let f = frontier?;
    if f.on_active_chain == Some(true) {
        f.height
    } else {
        None
    }
}

/// The network height (module doc).
pub fn network_height(blocks: u64, headers: u64, frontier: Option<&AttestedTip>) -> u64 {
    blocks
        .max(headers)
        .max(signed_height_on_chain(frontier).unwrap_or(0))
}

/// How many blocks `height` stands below the network height.
pub fn blocks_behind(height: u64, network: u64) -> u64 {
    network.saturating_sub(height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frontier(height: u64, on_active_chain: Option<bool>) -> AttestedTip {
        AttestedTip {
            height: Some(height),
            blocks_behind: None,
            on_active_chain,
            hash: None,
        }
    }

    #[test]
    fn a_signed_frontier_ahead_on_our_chain_is_the_network_height() {
        // The shape behind the 4,888-versus-5,260 reading: headers held
        // near the tip of what the node has checked, the signers far ahead.
        let f = frontier(245_260, Some(true));
        assert_eq!(network_height(240_000, 244_888, Some(&f)), 245_260);
        assert_eq!(
            blocks_behind(240_000, network_height(240_000, 244_888, Some(&f))),
            5_260
        );
    }

    #[test]
    fn a_frontier_off_our_chain_or_of_unknown_place_counts_for_nothing() {
        for on in [Some(false), None] {
            let f = frontier(250_000, on);
            assert_eq!(network_height(240_000, 240_010, Some(&f)), 240_010);
        }
        let no_height = AttestedTip {
            on_active_chain: Some(true),
            ..Default::default()
        };
        assert_eq!(network_height(240_000, 240_010, Some(&no_height)), 240_010);
    }

    #[test]
    fn without_a_frontier_it_reads_as_before() {
        assert_eq!(network_height(240_000, 240_500, None), 240_500);
        assert_eq!(network_height(240_000, 0, None), 240_000);
    }

    #[test]
    fn it_never_reads_below_the_node_itself() {
        // A signer behind us (an ancestor on our chain) and a best header
        // the engine has not moved yet: the node's own tip stands.
        let f = frontier(239_900, Some(true));
        assert_eq!(network_height(240_000, 239_000, Some(&f)), 240_000);
        assert_eq!(blocks_behind(240_000, 240_000), 0);
        assert_eq!(blocks_behind(240_010, 240_000), 0);
    }
}
