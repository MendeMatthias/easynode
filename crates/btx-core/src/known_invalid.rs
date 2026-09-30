//! Blocks this app refuses by hash, whatever its engine or its signers say.
//!
//! # Why a list of hashes exists at all
//!
//! On 2026-09-23 the network split at height 227,313. The longer branch starts
//! from `b28c3e84…`, and btxscan.io, two pools and the signed-confirmation
//! mirrors followed it. BTX's developers (jpp, late on the 23rd) said it fails
//! ExactReplay on the processor and the graphics chip, and that 0.34.10 will
//! refuse it (btxchain/btx PR #203). The valid chain carries `d5f0e92f…` at
//! 227,313. Checked on our own v0.34.9 node that night: both sit on the same
//! 227,312 (`8c36f9a6…`).
//!
//! A node that checks blocks on 0.34.9 rules the longer branch out itself.
//! Everything else does not:
//!
//! * A **mirror** (an M5, a PC without an NVIDIA driver) follows whatever its
//!   pinned signers sign, and the mirrors the census reached were on the
//!   longer branch.
//! * A node that has the longer branch's **headers** but has never fetched its
//!   bodies ranks it first by work and shows it as a `headers-only` tip, so
//!   `crate::fork` warns that "a longer chain exists that this node cannot
//!   obtain". That is exactly backwards on the valid chain.
//!
//! `invalidateblock` on the first block of the branch fixes both, and it is
//! the one instruction BTX's developers gave every node operator that night.
//! It marks the block and every descendant failed in the block index, which
//! persists, so a restart does not undo it. A mirror that is ON the branch
//! rolls back to 227,312. Tips of a refused branch read `invalid`, which
//! `crate::fork` does not count.
//!
//! # What this list is not
//!
//! It is not a checkpoint system and must not become one. An entry is a block
//! upstream itself calls invalid, named in the changelog of the release that
//! adds it, and the release that ships an upstream engine which refuses the
//! block by itself should take the entry out again.
//!
//! Operators can switch it off with `EASYBTX_NODE_REFUSE_KNOWN_INVALID=0`.
//!
//! # Held branches: refused by decision, not because they are invalid
//!
//! 2026-09-27 needed a second kind of entry, and it must not be mistaken for
//! the first. Branch B leaves the valid chain at 228,145 (`8240c62e…` at
//! 228,146, mined from 24 Sep 20:54 UTC): about 3,280 blocks, one payout
//! address, bodies withheld until that day and then served by one peer. jpp
//! replayed its root on a qualified node and it connects, so it is NOT
//! invalid, and upstream has not ruled on it (0.34.12rc1: "does not choose
//! either child of that fork"). But every node this app starts runs
//! `-parkdeepreorg=0`, so a node that replays B's bodies follows it, and every
//! mirror follows its signers there. The 3060 (`02d5efca…`), the one key every
//! signature-following node pins since 0.6.30, is such a node.
//!
//! The same day btxscan.io and the 3060 stopped on a dead branch from 229,400
//! (`b3a099ad…`), lighter than the valid chain and no longer mined. A node on
//! it can reach the valid chain only as a competing tower, the 0.34.9
//! acquisition path upstream fixed in 0.34.10, and the 3060's RTX 30-series
//! card cannot run 0.34.10 (btxchain/btx#205). Refusing the dead branch's first
//! block drops such a node to 229,399, where the valid chain is a plain
//! extension.
//!
//! So [`HELD_BRANCHES`] holds every node off both, in that order: B first, so a
//! node leaving the dead branch never has B as the heaviest chain it can check.
//! A held branch is the owner's decision (Mende, 2026-09-27), taken because an
//! update of this app is the only way to reach these machines, and it is
//! temporary by construction. Lifting one is a release that moves its root to
//! [`LIFTED_BRANCHES`], which the app reconsiders on every run. Without that, a
//! removed entry would stay refused on every node that had applied it: this
//! app calls `reconsiderblock` nowhere else.
//!
//! The same switch turns holds off: `EASYBTX_NODE_REFUSE_KNOWN_INVALID=0`.

use crate::error::AppError;
use crate::rpc::Rpc;
use serde_json::json;

/// One block the app refuses, with the sibling that the valid chain has at the
/// same height, so the entry says what it is choosing between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownInvalidBlock {
    pub height: u64,
    /// The block to refuse. Lowercase hex, as btxd prints it.
    pub hash: &'static str,
    /// What the valid chain has at the same height.
    pub valid_sibling: &'static str,
}

/// Every block the app refuses. See the module docs for what earns an entry.
pub const KNOWN_INVALID_BLOCKS: &[KnownInvalidBlock] = &[KnownInvalidBlock {
    // The first block of the longer branch of the 2026-09-23 split.
    height: 227_313,
    hash: "b28c3e846344ba744ca1e360792ba4389063b440c260eeb4a8003724478322a0",
    valid_sibling: "d5f0e92fb9a1f551a927902376f105d649dfa6f296304905a0275c1a5cdf8aa2",
}];

/// A branch the app holds its nodes off by the owner's decision, not because
/// upstream calls it invalid. See the module docs for what earns an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldBranch {
    /// The height of the branch's first block.
    pub height: u64,
    /// The branch's first block, the one refused. Lowercase hex, as btxd prints it.
    pub root: &'static str,
    /// Why, in a few words, for the node log.
    pub why: &'static str,
}

/// Every branch the app holds its nodes off, in the order they are refused.
pub const HELD_BRANCHES: &[HeldBranch] = &[
    HeldBranch {
        // Branch B. First, so the dead branch below is never left toward it.
        height: 228_146,
        root: "8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c",
        why: "Branch B: one payout address, bodies withheld, not ruled on upstream",
    },
    HeldBranch {
        height: 229_400,
        root: "b3a099ad8467b1ae8e033646eba8b15d5b3c65bd0af6fe813ad1a57a72c287be",
        why: "the dead branch btxscan.io and the 3060 stopped on",
    },
];

/// Roots a release has lifted. The app reconsiders each on every run, so a node
/// that applied the hold follows the most-work chain again. Empty until one is.
pub const LIFTED_BRANCHES: &[&str] = &[];

/// The operator's word on refusing known-invalid blocks, read from
/// `EASYBTX_NODE_REFUSE_KNOWN_INVALID`. Refusal is the default; only an
/// explicit `0`, `false`, `no` or `off` turns it off.
pub fn refusal_enabled() -> bool {
    refusal_enabled_from(
        std::env::var("EASYBTX_NODE_REFUSE_KNOWN_INVALID")
            .ok()
            .as_deref(),
    )
}

/// Pure half of [`refusal_enabled`].
pub fn refusal_enabled_from(raw: Option<&str>) -> bool {
    !matches!(
        raw.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0" | "false" | "no" | "off")
    )
}

/// What one attempt to refuse a block came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The node has no header for the block yet, so there is nothing to refuse.
    /// A node that has not reached 227,313, or whose peers never showed it
    /// the branch. Asked again later.
    NotKnownYet,
    /// `invalidateblock` returned. The block and its descendants are failed in
    /// the node's block index, which persists across restarts.
    Refused,
    /// The node knows the block but `invalidateblock` failed, or it did not
    /// answer whether it knows the block. Asked again later.
    Failed(String),
    /// Not attempted: an earlier held branch failed, and the order is the
    /// point. Asked again later.
    Waiting,
}

/// Ask the node to refuse `block`. Checks for the header first, so a node that
/// has never heard of the block is left alone rather than handed an error.
///
/// `invalidateblock` can take minutes on a node whose ACTIVE chain contains the
/// block, because it disconnects every block above it (about 900 on a mirror
/// that followed the 2026-09-23 branch to its tip). The caller must not hold
/// anything the rest of the app is waiting on while this runs.
pub async fn refuse(rpc: &dyn Rpc, block: &KnownInvalidBlock) -> Refusal {
    refuse_hash(rpc, block.hash).await
}

/// Does the node have a header for `hash`? `Ok(false)` only on the engine's own
/// answer for a header it never saw, `RPC_INVALID_ADDRESS_OR_KEY` (-5), "Block
/// not found" (`src/rpc/blockchain.cpp:881` at 84b998b4, unchanged in
/// v0.34.12). The code decides, not the words: in `getblockheader` -5 means
/// only that (a malformed hash is -8), and a later engine that rewords the
/// message must not strand every node that never heard of B on the dead
/// branch. Any other error, a timeout or a node still warming up (-28), is
/// `Err`: it says nothing about the header, so it must not read as "never
/// heard of it". That reading would let a node leave the dead branch while B,
/// unanswered, stood open.
pub async fn header_known(rpc: &dyn Rpc, hash: &str) -> Result<bool, String> {
    match rpc.call("getblockheader", json!([hash, true])).await {
        Ok(_) => Ok(true),
        Err(AppError::Rpc { code: -5, .. }) => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

/// [`refuse`], for any block hash.
async fn refuse_hash(rpc: &dyn Rpc, hash: &str) -> Refusal {
    match header_known(rpc, hash).await {
        Ok(true) => {}
        Ok(false) => return Refusal::NotKnownYet,
        Err(e) => return Refusal::Failed(e),
    }
    match rpc.call("invalidateblock", json!([hash])).await {
        Ok(_) => Refusal::Refused,
        Err(e) => Refusal::Failed(e.to_string()),
    }
}

/// Refuse `branches` in order. Once one FAILS, the rest wait for the next
/// attempt: Branch B must be refused before the dead branch is left, or the
/// heaviest chain the node can check would be B. A branch the node has never
/// heard of does not hold the rest back, because a branch it does not know is
/// one it cannot follow, and the next attempt refuses it once the header lands.
/// "Never heard of" means the engine said so ([`header_known`]); a lookup it
/// did not answer is a failure and holds the rest back like any other.
///
/// On a node whose ACTIVE chain contains the root, `invalidateblock` outlasts
/// the RPC client's timeout (thousands of blocks for B), so it reads as Failed
/// once and the rest wait; the retry finds the block already failed.
pub async fn refuse_held_in_order(
    rpc: &dyn Rpc,
    branches: &[HeldBranch],
) -> Vec<(HeldBranch, Refusal)> {
    let mut out = Vec::with_capacity(branches.len());
    let mut failed = false;
    for branch in branches {
        let outcome = if failed {
            Refusal::Waiting
        } else {
            refuse_hash(rpc, branch.root).await
        };
        failed = failed || matches!(outcome, Refusal::Failed(_));
        out.push((*branch, outcome));
    }
    out
}

/// [`refuse_held_in_order`] over [`HELD_BRANCHES`].
pub async fn refuse_held(rpc: &dyn Rpc) -> Vec<(HeldBranch, Refusal)> {
    refuse_held_in_order(rpc, HELD_BRANCHES).await
}

/// What one attempt to lift a hold came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lift {
    /// The node has no header for the root, so it never held it.
    NotKnownYet,
    /// `reconsiderblock` returned. On a block that was not failed it changes
    /// nothing, which is why it is safe to ask on every run.
    Lifted,
    /// The node knows the root but `reconsiderblock` failed, or it did not
    /// answer whether it knows the root. Asked again later.
    Failed(String),
}

/// `reconsiderblock` every root in `roots`, in order.
pub async fn lift_roots(rpc: &dyn Rpc, roots: &[&'static str]) -> Vec<(&'static str, Lift)> {
    let mut out = Vec::with_capacity(roots.len());
    for root in roots {
        let outcome = match header_known(rpc, root).await {
            Ok(false) => Lift::NotKnownYet,
            Err(e) => Lift::Failed(e),
            Ok(true) => match rpc.call("reconsiderblock", json!([root])).await {
                Ok(_) => Lift::Lifted,
                Err(e) => Lift::Failed(e.to_string()),
            },
        };
        out.push((*root, outcome));
    }
    out
}

/// [`lift_roots`] over [`LIFTED_BRANCHES`].
pub async fn lift_all(rpc: &dyn Rpc) -> Vec<(&'static str, Lift)> {
    lift_roots(rpc, LIFTED_BRANCHES).await
}

/// [`refuse`] every entry in [`KNOWN_INVALID_BLOCKS`], returning each outcome
/// in list order.
pub async fn refuse_all(rpc: &dyn Rpc) -> Vec<(KnownInvalidBlock, Refusal)> {
    let mut out = Vec::with_capacity(KNOWN_INVALID_BLOCKS.len());
    for block in KNOWN_INVALID_BLOCKS {
        out.push((*block, refuse(rpc, block).await));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppResult;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::Mutex;

    fn is_block_hash(s: &str) -> bool {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    /// Every entry is a well-formed lowercase hash, refuses something other
    /// than the valid chain's own block, and the 2026-09-23 split is on the
    /// list with the two hashes our node read at 227,313.
    #[test]
    fn the_list_names_the_2026_09_23_branch_and_never_the_valid_chain() {
        for b in KNOWN_INVALID_BLOCKS {
            assert!(is_block_hash(b.hash), "{b:?}");
            assert!(is_block_hash(b.valid_sibling), "{b:?}");
            assert_ne!(b.hash, b.valid_sibling);
        }
        let split = KNOWN_INVALID_BLOCKS
            .iter()
            .find(|b| b.height == 227_313)
            .expect("the 227,313 split is on the list");
        assert!(split.hash.starts_with("b28c3e84"));
        assert!(split.valid_sibling.starts_with("d5f0e92f"));
    }

    #[test]
    fn refusal_is_on_unless_the_operator_says_otherwise() {
        assert!(refusal_enabled_from(None));
        assert!(refusal_enabled_from(Some("")));
        assert!(refusal_enabled_from(Some("1")));
        assert!(refusal_enabled_from(Some("yes")));
        for off in ["0", "false", "FALSE", " no ", "off"] {
            assert!(!refusal_enabled_from(Some(off)), "{off:?}");
        }
    }

    /// A node scripted per method, recording every call it receives.
    struct ScriptedNode {
        knows_header: bool,
        invalidate_fails: bool,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl ScriptedNode {
        fn new(knows_header: bool, invalidate_fails: bool) -> Self {
            Self {
                knows_header,
                invalidate_fails,
                calls: Mutex::new(Vec::new()),
            }
        }
        fn methods(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(m, _)| m.clone())
                .collect()
        }
    }

    #[async_trait]
    impl Rpc for ScriptedNode {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), params));
            match method {
                "getblockheader" if self.knows_header => Ok(json!({ "height": 227_313 })),
                "getblockheader" => Err(AppError::Rpc {
                    code: -5,
                    message: "Block not found".into(),
                }),
                "invalidateblock" if self.invalidate_fails => Err(AppError::Rpc {
                    code: -28,
                    message: "Work queue depth exceeded".into(),
                }),
                "invalidateblock" => Ok(Value::Null),
                other => panic!("refusing a block must never call {other}"),
            }
        }
    }

    const SPLIT: KnownInvalidBlock = KNOWN_INVALID_BLOCKS[0];

    /// A node that has never seen the branch is left alone: no invalidateblock,
    /// so a node below 227,313 is not handed an error every half minute.
    #[tokio::test]
    async fn a_node_without_the_header_is_not_asked_to_invalidate_it() {
        let node = ScriptedNode::new(false, false);
        assert_eq!(refuse(&node, &SPLIT).await, Refusal::NotKnownYet);
        assert_eq!(node.methods(), vec!["getblockheader"]);
    }

    #[tokio::test]
    async fn a_node_with_the_header_invalidates_exactly_that_block() {
        let node = ScriptedNode::new(true, false);
        assert_eq!(refuse(&node, &SPLIT).await, Refusal::Refused);
        let calls = node.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].0, "invalidateblock");
        assert_eq!(calls[1].1, json!([SPLIT.hash]));
    }

    #[tokio::test]
    async fn a_failed_invalidate_is_reported_for_a_retry() {
        let node = ScriptedNode::new(true, true);
        match refuse(&node, &SPLIT).await {
            Refusal::Failed(msg) => assert!(msg.contains("Work queue"), "{msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn refuse_all_answers_for_every_entry_in_order() {
        let node = ScriptedNode::new(true, false);
        let got = refuse_all(&node).await;
        assert_eq!(got.len(), KNOWN_INVALID_BLOCKS.len());
        assert!(got.iter().all(|(_, r)| *r == Refusal::Refused));
    }

    /// A node scripted per hash: which headers it has never seen and which
    /// `invalidateblock` calls fail. Records every call; answers reconsiderblock.
    struct PerHashNode {
        unknown: Vec<&'static str>,
        failing: Vec<&'static str>,
        /// Headers the node does not answer for: a timeout, not "not found".
        silent: Vec<&'static str>,
        /// Headers asked while the node warms up: the engine's -28.
        warming: Vec<&'static str>,
        /// Headers the node never saw, said in other words than today's.
        reworded: Vec<&'static str>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl PerHashNode {
        fn new(unknown: &[&'static str], failing: &[&'static str]) -> Self {
            Self {
                unknown: unknown.to_vec(),
                failing: failing.to_vec(),
                silent: Vec::new(),
                warming: Vec::new(),
                reworded: Vec::new(),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn silent(mut self, silent: &[&'static str]) -> Self {
            self.silent = silent.to_vec();
            self
        }
        fn warming(mut self, warming: &[&'static str]) -> Self {
            self.warming = warming.to_vec();
            self
        }
        fn reworded(mut self, reworded: &[&'static str]) -> Self {
            self.reworded = reworded.to_vec();
            self
        }
        fn calls(&self) -> Vec<(String, Value)> {
            self.calls.lock().unwrap().clone()
        }
        fn invalidated(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter(|(m, _)| m == "invalidateblock")
                .map(|(_, p)| p[0].as_str().unwrap_or_default().to_string())
                .collect()
        }
    }

    #[async_trait]
    impl Rpc for PerHashNode {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), params.clone()));
            let hash = params[0].as_str().unwrap_or_default();
            match method {
                "getblockheader" if self.silent.contains(&hash) => {
                    Err(AppError::Http("operation timed out".into()))
                }
                "getblockheader" if self.warming.contains(&hash) => Err(AppError::Rpc {
                    code: -28,
                    message: "Loading block index...".into(),
                }),
                "getblockheader" if self.reworded.contains(&hash) => Err(AppError::Rpc {
                    code: -5,
                    message: "No such block".into(),
                }),
                "getblockheader" if self.unknown.contains(&hash) => Err(AppError::Rpc {
                    code: -5,
                    message: "Block not found".into(),
                }),
                "getblockheader" => Ok(json!({ "height": 1 })),
                "invalidateblock" if self.failing.contains(&hash) => Err(AppError::Rpc {
                    code: -1,
                    message: "request timed out".into(),
                }),
                "invalidateblock" | "reconsiderblock" => Ok(Value::Null),
                other => panic!("a hold must never call {other}"),
            }
        }
    }

    const B: &str = "8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c";
    const DEAD: &str = "b3a099ad8467b1ae8e033646eba8b15d5b3c65bd0af6fe813ad1a57a72c287be";

    /// Branch B first, then the dead branch. Neither is on the invalid list,
    /// because neither is invalid, and neither is lifted yet.
    #[test]
    fn held_branches_are_b_then_the_dead_branch() {
        let roots: Vec<_> = HELD_BRANCHES.iter().map(|b| (b.height, b.root)).collect();
        assert_eq!(roots, vec![(228_146, B), (229_400, DEAD)]);
        for b in HELD_BRANCHES {
            assert!(is_block_hash(b.root), "{b:?}");
            assert!(!b.why.is_empty(), "{b:?}");
            assert!(
                KNOWN_INVALID_BLOCKS.iter().all(|k| k.hash != b.root),
                "{b:?} is held, not invalid"
            );
            assert!(
                !LIFTED_BRANCHES.contains(&b.root),
                "{b:?} is held and lifted"
            );
        }
        assert!(LIFTED_BRANCHES.iter().all(|r| is_block_hash(r)));
    }

    #[tokio::test]
    async fn both_are_refused_and_b_first() {
        let node = PerHashNode::new(&[], &[]);
        let got = refuse_held(&node).await;
        assert!(got.iter().all(|(_, r)| *r == Refusal::Refused), "{got:?}");
        assert_eq!(node.invalidated(), vec![B.to_string(), DEAD.to_string()]);
    }

    /// While B is not refused, the dead branch is not left: off it, B would be
    /// the heaviest chain the node can check.
    #[tokio::test]
    async fn the_dead_branch_waits_while_b_is_not_refused() {
        let node = PerHashNode::new(&[], &[B]);
        let got = refuse_held(&node).await;
        assert!(matches!(got[0].1, Refusal::Failed(_)), "{got:?}");
        assert_eq!(got[1].1, Refusal::Waiting);
        assert!(
            node.calls().iter().all(|(_, p)| p[0] != json!(DEAD)),
            "the dead branch was touched while B stood"
        );
    }

    /// A node that never heard of B cannot follow it, so it still leaves the
    /// dead branch.
    #[tokio::test]
    async fn an_unknown_b_does_not_hold_the_dead_branch_back() {
        let node = PerHashNode::new(&[B], &[]);
        let got = refuse_held(&node).await;
        assert_eq!(got[0].1, Refusal::NotKnownYet);
        assert_eq!(got[1].1, Refusal::Refused);
        assert_eq!(node.invalidated(), vec![DEAD.to_string()]);
    }

    /// A lookup the node does not answer is not "never heard of it". Were it
    /// read that way, a node that timed out on B would leave the dead branch
    /// with B still open, and B is then the heaviest chain it can check. Only
    /// the engine's "Block not found" (-5) says the node has no such header.
    #[tokio::test]
    async fn an_unanswered_lookup_for_b_holds_the_dead_branch_back() {
        let node = PerHashNode::new(&[], &[]).silent(&[B]);
        let got = refuse_held(&node).await;
        assert!(matches!(got[0].1, Refusal::Failed(_)), "{got:?}");
        assert_eq!(got[1].1, Refusal::Waiting);
        assert!(
            node.calls().iter().all(|(_, p)| p[0] != json!(DEAD)),
            "the dead branch was touched while B was unanswered"
        );
        assert!(node.invalidated().is_empty(), "nothing was refused");
    }

    /// An error that IS an engine answer, but not "not found", is no answer
    /// about the header either: the warm-up refusal (-28) every method returns
    /// while the node starts. It holds the dead branch back like a timeout.
    #[tokio::test]
    async fn a_warming_node_holds_the_dead_branch_back() {
        let node = PerHashNode::new(&[], &[]).warming(&[B]);
        let got = refuse_held(&node).await;
        assert!(matches!(got[0].1, Refusal::Failed(_)), "{got:?}");
        assert_eq!(got[1].1, Refusal::Waiting);
        assert!(node.invalidated().is_empty(), "nothing was refused");
    }

    /// The engine's code for a header it never saw is what counts, not its
    /// wording: a later engine that rewords "Block not found" must not strand
    /// every node that never heard of B on the dead branch.
    #[tokio::test]
    async fn not_found_is_the_code_not_the_words() {
        let node = PerHashNode::new(&[], &[]).reworded(&[B]);
        let got = refuse_held(&node).await;
        assert_eq!(got[0].1, Refusal::NotKnownYet);
        assert_eq!(got[1].1, Refusal::Refused);
    }

    /// A lift the node does not answer for is a failure, asked again, not a
    /// root the node never held.
    #[tokio::test]
    async fn an_unanswered_lookup_is_a_failed_lift() {
        let node = PerHashNode::new(&[], &[]).silent(&[DEAD]);
        let got = lift_roots(&node, &[DEAD]).await;
        assert!(matches!(got[0].1, Lift::Failed(_)), "{got:?}");
    }

    /// Lifting is reconsiderblock, asked only of a node that knows the root.
    #[tokio::test]
    async fn a_lifted_root_is_reconsidered() {
        let node = PerHashNode::new(&[DEAD], &[]);
        let got = lift_roots(&node, &[B, DEAD]).await;
        assert_eq!(got, vec![(B, Lift::Lifted), (DEAD, Lift::NotKnownYet)]);
        let methods: Vec<_> = node.calls().into_iter().map(|(m, _)| m).collect();
        assert_eq!(
            methods,
            vec!["getblockheader", "reconsiderblock", "getblockheader"]
        );
        assert!(lift_all(&node).await.is_empty(), "nothing is lifted yet");
    }
}
