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
    /// The node knows the block but `invalidateblock` failed. Asked again later.
    Failed(String),
}

/// Ask the node to refuse `block`. Checks for the header first, so a node that
/// has never heard of the block is left alone rather than handed an error.
///
/// `invalidateblock` can take minutes on a node whose ACTIVE chain contains the
/// block, because it disconnects every block above it (about 900 on a mirror
/// that followed the 2026-09-23 branch to its tip). The caller must not hold
/// anything the rest of the app is waiting on while this runs.
pub async fn refuse(rpc: &dyn Rpc, block: &KnownInvalidBlock) -> Refusal {
    if rpc
        .call("getblockheader", json!([block.hash, true]))
        .await
        .is_err()
    {
        return Refusal::NotKnownYet;
    }
    match rpc.call("invalidateblock", json!([block.hash])).await {
        Ok(_) => Refusal::Refused,
        Err(e) => Refusal::Failed(e.to_string()),
    }
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
    use crate::error::{AppError, AppResult};
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
}
