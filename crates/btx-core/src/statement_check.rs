//! Does a snapshot statement say what THIS node saw? The one check behind
//! every co-signature, every dissent and every submission.
//!
//! `signutxosnapshotmanifest` signs whatever it is handed (a node at height 0
//! co-signed a statement about block 100 in the spike of 2026-09-29), so a
//! signature means something only because the app compares first. A producer
//! runs this on its own export before it sends it anywhere (section 4 of
//! docs/decisions/2026-09-29-every-node-starts-near-the-tip.md); a confirmer
//! runs it on every statement waiting on easybtx.com before it signs or
//! dissents (section 5). Read-only: nothing here changes the node.
//!
//! IN THIS ORDER, each a [`Mismatch`] whose [`Mismatch::next`] says what the
//! confirmer does about it:
//!
//! 1. The statement on its own: version, the chain's genesis, replay context
//!    and compiled shielded commitment, not a dissent, the engine's file
//!    geometry, a height on the grid ([`cs::check_shape`]). No RPC. Skip.
//! 2. The running node is on the statement's chain (`getblockhash 0`) and
//!    reports the statement's replay context (`getmatmultrustedstatus`). With
//!    1, section 5's check 3. Skip.
//! 3. The height is at least [`cs::SNAPSHOT_DEPTH`] (144) blocks deep on the
//!    node's active chain, counted as `confirmations` counts them (tip minus
//!    height plus one). Section 5's check 1. Wait.
//! 4. The diary has an entry at that height whose block is STILL the block at
//!    that height on the active chain ([`diary::entry_on_chain`]). Section 5's
//!    check 2. Skip.
//! 5. No block the app refuses (`crate::known_invalid`, the [`Holds`]) is the
//!    block at its height on the active chain at or below the statement's.
//!    A node on a refused branch keeps a diary of that branch. Skip.
//! 6. Height, block hash, `hash_serialized_3`, coin count and chain
//!    transaction count equal that diary entry. Section 5's check 4: the only
//!    failure a confirmer answers with a dissent (section 6a).
//!
//! The order differs from section 5's numbering (chain before depth) so that
//! a statement for another chain is skipped at once instead of waited on;
//! for any one failing check the verdict is the same.
//!
//! WHY NO "BASE ON THE ACTIVE CHAIN" STEP. Step 4 already proved the diary's
//! block is the block at that height on the active chain, so step 6's
//! block-hash comparison is that check: a statement naming any other block
//! (a sibling, a branch this node never saw) differs from the diary, which
//! for a confirmer is a dissent and for a producer a pair it does not send.
//! Asking for the statement's block separately would turn a real chain
//! disagreement into a silent skip.

use crate::confirmed_load::Holds;
use crate::confirmed_snapshot::{self as cs, ChainRules, Refusal, Statement};
use crate::diary::{self, Diary, DiaryEntry};
use crate::rpc::Rpc;
use serde_json::json;

/// The depth a statement's height must have on the node's own chain before
/// it is sent or signed.
pub const DEPTH: u64 = cs::SNAPSHOT_DEPTH as u64;

/// A chain field a statement and a diary entry can disagree on, the ones a
/// dispute compares (section 6a; the website's `CHAIN_FIELDS`). The height
/// is equal by construction: the entry is looked up by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    BlockHash,
    HashSerialized,
    Coins,
    ChainTx,
}

impl Field {
    /// The words the website's alert uses for the field.
    pub fn words(self) -> &'static str {
        match self {
            Field::BlockHash => "block hash",
            Field::HashSerialized => "UTXO hash",
            Field::Coins => "coin count",
            Field::ChainTx => "transaction count",
        }
    }
}

/// What a confirmer does about a [`Mismatch`] (section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Not yet: the block is not deep enough, or the node did not answer.
    /// Ask again on the next round.
    Wait,
    /// Neither sign nor dissent: nothing this node can vouch for either way.
    Skip,
    /// The diary disagrees: send a dissent built from the diary entry.
    Dissent,
}

/// Why a statement is not signed or sent. `Display` is one plain line for
/// the log and for Copy diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    /// Step 1: the statement breaks its chain's rules on its own.
    Shape(Refusal),
    /// Step 2: the node is on another chain (its genesis, display hex).
    ChainId { node: String },
    /// Step 2: the node reports no replay context, or another one.
    ReplayContext { node: Option<String> },
    /// Step 3: `depth` blocks deep on the node's active chain (0 when the
    /// node has not reached the height), `need` required.
    TooShallow { depth: u64, need: u64 },
    /// Step 4: no entry at the height, or one whose block has left the
    /// active chain, or a diary for another chain.
    NoDiaryEntry { height: u64 },
    /// Step 5: a refused block is on the node's active chain.
    HeldRootOnActiveChain { height: u64, root: String },
    /// Step 6: the statement differs from the diary entry in `fields` (in
    /// the website's order, never empty). `entry` is the diary's account, the
    /// one a dissent carries.
    Differs {
        entry: DiaryEntry,
        statement: Box<Statement>,
        fields: Vec<Field>,
    },
    /// The node did not answer a read. Says nothing about the statement.
    NodeUnanswered(String),
}

impl Mismatch {
    pub fn next(&self) -> Next {
        match self {
            Mismatch::TooShallow { .. } | Mismatch::NodeUnanswered(_) => Next::Wait,
            Mismatch::Differs { .. } => Next::Dissent,
            Mismatch::Shape(_)
            | Mismatch::ChainId { .. }
            | Mismatch::ReplayContext { .. }
            | Mismatch::NoDiaryEntry { .. }
            | Mismatch::HeldRootOnActiveChain { .. } => Next::Skip,
        }
    }
}

/// `(diary, statement)` for one field, as the log shows them.
fn values(field: Field, e: &DiaryEntry, st: &Statement) -> (String, String) {
    match field {
        Field::BlockHash => (e.block_hash.clone(), st.block_hash().display_hex()),
        Field::HashSerialized => (
            e.hash_serialized.clone(),
            st.hash_serialized().display_hex(),
        ),
        Field::Coins => (e.coins.to_string(), st.coins().to_string()),
        Field::ChainTx => (e.chain_tx.to_string(), st.chain_tx().to_string()),
    }
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mismatch::Shape(r) => write!(f, "the statement does not check out: {r}"),
            Mismatch::ChainId { node } => {
                write!(f, "this node is on chain {node}, not the statement's")
            }
            Mismatch::ReplayContext { node: None } => {
                write!(f, "this node reports no replay context")
            }
            Mismatch::ReplayContext { node: Some(c) } => {
                write!(f, "this node's replay context is {c}, not the statement's")
            }
            Mismatch::TooShallow { depth, need } => write!(
                f,
                "the block is {depth} blocks deep on this node's chain, {need} are needed"
            ),
            Mismatch::NoDiaryEntry { height } => write!(
                f,
                "this node's diary has nothing at {height} for the block on its chain now"
            ),
            Mismatch::HeldRootOnActiveChain { height, root } => write!(
                f,
                "block {root} at {height}, which the app refuses, is on this node's chain"
            ),
            Mismatch::Differs {
                entry,
                statement,
                fields,
            } => {
                write!(
                    f,
                    "the statement differs from this node's diary at {}:",
                    entry.height
                )?;
                for (i, field) in fields.iter().enumerate() {
                    let (d, s) = values(*field, entry, statement);
                    let sep = if i == 0 { "" } else { "," };
                    write!(f, "{sep} {} (diary {d}, statement {s})", field.words())?;
                }
                Ok(())
            }
            Mismatch::NodeUnanswered(e) => write!(f, "the node did not answer: {e}"),
        }
    }
}

impl From<Refusal> for Mismatch {
    fn from(r: Refusal) -> Self {
        Mismatch::Shape(r)
    }
}

/// Step 6 alone, pure: every chain field of `st` against `entry`. Hashes
/// compare case-insensitively (the diary keeps lowercase, as the engine
/// prints). The caller has already matched the heights.
pub fn compare_with_entry(st: &Statement, entry: &DiaryEntry) -> Result<(), Mismatch> {
    let mut fields = Vec::new();
    for field in [
        Field::BlockHash,
        Field::HashSerialized,
        Field::Coins,
        Field::ChainTx,
    ] {
        let (d, s) = values(field, entry, st);
        if !d.eq_ignore_ascii_case(&s) {
            fields.push(field);
        }
    }
    if fields.is_empty() {
        Ok(())
    } else {
        Err(Mismatch::Differs {
            entry: entry.clone(),
            statement: Box::new(st.clone()),
            fields,
        })
    }
}

/// Steps 1 to 6 of the module doc against the running node and its diary,
/// read-only. `Ok` is the diary entry the statement matched. `rules` are
/// the statement's chain's (`ChainRules::for_statement`); `holds` the
/// blocks the app refuses (`Holds::compiled()`).
pub async fn check_against_node(
    rpc: &dyn Rpc,
    st: &Statement,
    rules: &ChainRules,
    diary: &Diary,
    holds: &Holds<'_>,
) -> Result<DiaryEntry, Mismatch> {
    // 1. The statement alone, before the node is asked anything.
    let height = cs::check_shape(st, rules)?;
    let unanswered = |e: String| Mismatch::NodeUnanswered(e);

    // 2. The node's chain and replay context.
    let genesis = diary::block_at(rpc, 0)
        .await
        .map_err(unanswered)?
        .ok_or_else(|| Mismatch::NodeUnanswered("getblockhash 0 found no genesis".into()))?;
    if !genesis.eq_ignore_ascii_case(&st.chain_id().display_hex()) {
        return Err(Mismatch::ChainId { node: genesis });
    }
    let context = crate::node_api::get_matmul_trusted_status(rpc)
        .await
        .map_err(|e| Mismatch::NodeUnanswered(format!("getmatmultrustedstatus: {e}")))?
        .replay_authority_context
        .map(|c| c.to_ascii_lowercase());
    if context.as_deref() != Some(st.replay_context().display_hex().as_str()) {
        return Err(Mismatch::ReplayContext { node: context });
    }

    // 3. Deep enough on the active chain. The block at `height` counts as
    // one deep, as the engine's `confirmations` counts it.
    let tip = rpc
        .call("getblockcount", json!([]))
        .await
        .map_err(|e| Mismatch::NodeUnanswered(format!("getblockcount: {e}")))?
        .as_u64()
        .ok_or_else(|| Mismatch::NodeUnanswered("getblockcount answered no number".into()))?;
    let depth = (tip + 1).saturating_sub(height);
    if depth < DEPTH {
        return Err(Mismatch::TooShallow { depth, need: DEPTH });
    }

    // 4. The diary's entry, only while its block is still on the chain. A
    // diary filed under another chain has nothing for this statement.
    let no_entry = Mismatch::NoDiaryEntry { height };
    if !diary
        .chain_id
        .eq_ignore_ascii_case(&st.chain_id().display_hex())
    {
        return Err(no_entry);
    }
    let entry = diary::entry_on_chain(rpc, diary, height)
        .await
        .map_err(unanswered)?
        .ok_or(no_entry)?
        .clone();

    // 5. No refused block on the chain that diary entry rests on.
    for (root_height, root) in holds.roots().into_iter().filter(|(h, _)| *h <= height) {
        let at = diary::block_at(rpc, root_height)
            .await
            .map_err(unanswered)?;
        if at.is_some_and(|b| b.eq_ignore_ascii_case(root)) {
            return Err(Mismatch::HeldRootOnActiveChain {
                height: root_height,
                root: root.to_string(),
            });
        }
    }

    // 6. Field by field.
    compare_with_entry(st, &entry)?;
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_node::{sibling_hash, FakeNode};
    use crate::known_invalid::{HeldBranch, KnownInvalidBlock};
    use crate::operators::{MAINNET_GENESIS, REGTEST_GENESIS};

    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_BLOCK: &str = "bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46";
    const R_UTXO: &str = "e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527";
    const ROOT: &str = "8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c";
    const HOLD_AT_50: &[HeldBranch] = &[HeldBranch {
        height: 50,
        root: ROOT,
        why: "a test hold",
    }];
    /// The spike's statement is at 100: 144 deep at tip 243.
    const DEEP_ENOUGH: u64 = 100 + DEPTH - 1;

    fn statement() -> Statement {
        cs::parse(R_P).unwrap().statement
    }

    /// The statement with bytes `at..at+bytes.len()` replaced.
    fn edited(at: usize, bytes: &[u8]) -> Statement {
        let mut raw = *statement().raw();
        raw[at..at + bytes.len()].copy_from_slice(bytes);
        Statement::from_raw(raw)
    }

    fn rules() -> ChainRules {
        ChainRules::for_statement(&statement(), None).unwrap()
    }

    fn spike_entry() -> DiaryEntry {
        DiaryEntry {
            height: 100,
            block_hash: R_BLOCK.into(),
            hash_serialized: R_UTXO.into(),
            coins: 101,
            chain_tx: 101,
            recorded_at: 0,
        }
    }

    fn diary_with(e: DiaryEntry) -> Diary {
        let mut d = Diary::new(REGTEST_GENESIS);
        d.record(e);
        d
    }

    fn diary() -> Diary {
        diary_with(spike_entry())
    }

    /// The node that exported the spike's statement, 144 blocks later.
    fn node_at(tip: u64) -> FakeNode {
        let n = FakeNode::new(REGTEST_GENESIS, tip);
        n.with(|s| {
            if tip >= 100 {
                s.chain.insert(100, R_BLOCK.into());
            }
            s.replay_context = Some(cs::REGTEST_REPLAY_CONTEXT.into());
        });
        n
    }

    fn node() -> FakeNode {
        node_at(DEEP_ENOUGH)
    }

    async fn check(n: &FakeNode, d: &Diary, holds: &Holds<'_>) -> Result<DiaryEntry, Mismatch> {
        check_against_node(n, &statement(), &rules(), d, holds).await
    }

    /// Only reads: never a refusal, a signature or anything else that
    /// changes the node.
    fn read_only(n: &FakeNode) -> bool {
        n.methods().iter().all(|m| {
            matches!(
                m.as_str(),
                "getblockhash" | "getblockcount" | "getmatmultrustedstatus"
            )
        })
    }

    #[tokio::test]
    async fn the_spikes_statement_matches_the_node_that_made_it() {
        let n = node();
        assert_eq!(check(&n, &diary(), &Holds::none()).await, Ok(spike_entry()));
        assert!(read_only(&n), "{:?}", n.methods());
    }

    /// Section 5, check 4: one sabotage per field, each the only thing wrong,
    /// each answered with a dissent that carries the diary's account.
    #[tokio::test]
    async fn a_diary_that_differs_in_any_one_field_is_a_dissent() {
        type Edit = fn(&mut DiaryEntry);
        let cases: [(Field, Edit); 4] = [
            (Field::BlockHash, |e| e.block_hash = "11".repeat(32)),
            (Field::HashSerialized, |e| {
                e.hash_serialized = "22".repeat(32)
            }),
            (Field::Coins, |e| e.coins = 102),
            (Field::ChainTx, |e| e.chain_tx = 100),
        ];
        for (field, edit) in cases {
            let mut e = spike_entry();
            edit(&mut e);
            // The node's chain agrees with its diary, as a recording node's
            // does: the block hash case is a node on another block at 100.
            let n = node();
            let block = e.block_hash.clone();
            n.with(|s| {
                s.chain.insert(100, block);
            });
            let got = check(&n, &diary_with(e.clone()), &Holds::none()).await;
            let want = Mismatch::Differs {
                entry: e,
                statement: Box::new(statement()),
                fields: vec![field],
            };
            assert_eq!(got, Err(want.clone()), "{}", field.words());
            assert_eq!(want.next(), Next::Dissent);
            assert!(read_only(&n));
        }
    }

    #[test]
    fn every_differing_field_is_named_in_the_websites_order() {
        let e = DiaryEntry {
            coins: 7,
            hash_serialized: "22".repeat(32),
            ..spike_entry()
        };
        let got = compare_with_entry(&statement(), &e).unwrap_err();
        let Mismatch::Differs { ref fields, .. } = got else {
            panic!("{got:?}")
        };
        assert_eq!(fields, &vec![Field::HashSerialized, Field::Coins]);
        assert_eq!(
            got.to_string(),
            format!(
                "the statement differs from this node's diary at 100: UTXO hash (diary {}, \
                 statement {R_UTXO}), coin count (diary 7, statement 101)",
                "22".repeat(32)
            )
        );
        assert!(!got.to_string().contains('\u{2014}'));
        // Case does not count: the engine prints lowercase, a diary might not.
        let upper = DiaryEntry {
            block_hash: R_BLOCK.to_ascii_uppercase(),
            ..spike_entry()
        };
        assert_eq!(compare_with_entry(&statement(), &upper), Ok(()));
    }

    /// The design's "a base off the node's chain": the statement names a
    /// block this node does not have at 100, and the diary has the one it
    /// does. That is a disagreement about the chain, so a dissent, not a skip.
    #[tokio::test]
    async fn a_statement_for_a_block_off_this_nodes_chain_is_a_dissent() {
        let n = node();
        n.with(|s| {
            s.chain.insert(100, sibling_hash(100));
        });
        let ours = DiaryEntry {
            block_hash: sibling_hash(100),
            ..spike_entry()
        };
        let got = check(&n, &diary_with(ours), &Holds::none())
            .await
            .unwrap_err();
        assert!(
            matches!(&got, Mismatch::Differs { fields, .. } if fields == &vec![Field::BlockHash]),
            "{got:?}"
        );
        assert_eq!(got.next(), Next::Dissent);
    }

    /// Section 5, check 2: no entry, an entry whose block has left the active
    /// chain, or a diary for another chain: neither sign nor dissent.
    #[tokio::test]
    async fn without_a_diary_entry_on_the_active_chain_nothing_is_said() {
        let none = Mismatch::NoDiaryEntry { height: 100 };
        assert_eq!(
            check(&node(), &Diary::new(REGTEST_GENESIS), &Holds::none()).await,
            Err(none.clone())
        );
        assert_eq!(none.next(), Next::Skip);
        // The diary wrote down the spike's block, and a sibling replaced it.
        let n = node();
        n.with(|s| s.reorg_from(100, DEEP_ENOUGH));
        assert_eq!(check(&n, &diary(), &Holds::none()).await, Err(none.clone()));
        // The same entries, filed under another chain.
        let mut other = diary();
        other.chain_id = MAINNET_GENESIS.into();
        assert_eq!(check(&node(), &other, &Holds::none()).await, Err(none));
    }

    /// Section 5, check 1: fewer than 144 blocks deep waits.
    #[tokio::test]
    async fn a_block_under_144_deep_waits() {
        let got = check(&node_at(DEEP_ENOUGH - 1), &diary(), &Holds::none()).await;
        assert_eq!(
            got,
            Err(Mismatch::TooShallow {
                depth: 143,
                need: 144
            })
        );
        assert_eq!(got.unwrap_err().next(), Next::Wait);
        // Exactly 144 is enough.
        assert!(check(&node_at(DEEP_ENOUGH), &diary(), &Holds::none())
            .await
            .is_ok());
        // A node that has not reached 100 at all: 0 deep, and its diary is
        // not even read.
        let n = node_at(60);
        assert_eq!(
            check(&n, &diary(), &Holds::none()).await,
            Err(Mismatch::TooShallow {
                depth: 0,
                need: 144
            })
        );
        assert_eq!(n.count("getblockhash"), 1, "only the genesis");
    }

    /// Section 5, check 3, the node's half: its chain and replay context.
    #[tokio::test]
    async fn a_node_on_another_chain_or_replay_context_says_nothing() {
        let n = node();
        n.with(|s| {
            s.chain.insert(0, MAINNET_GENESIS.into());
        });
        let got = check(&n, &diary(), &Holds::none()).await.unwrap_err();
        assert_eq!(
            got,
            Mismatch::ChainId {
                node: MAINNET_GENESIS.into()
            }
        );
        assert_eq!(got.next(), Next::Skip);
        let n = node();
        n.with(|s| s.replay_context = Some("33".repeat(32)));
        assert_eq!(
            check(&n, &diary(), &Holds::none()).await,
            Err(Mismatch::ReplayContext {
                node: Some("33".repeat(32))
            })
        );
        let n = node();
        n.with(|s| s.replay_context = None);
        assert_eq!(
            check(&n, &diary(), &Holds::none()).await,
            Err(Mismatch::ReplayContext { node: None })
        );
        // The engine prints lowercase; an uppercase answer is the same context.
        let n = node();
        n.with(|s| s.replay_context = Some(cs::REGTEST_REPLAY_CONTEXT.to_ascii_uppercase()));
        assert!(check(&n, &diary(), &Holds::none()).await.is_ok());
    }

    /// Section 5, check 3, the statement's half, and the rest of its shape:
    /// refused before the node is asked anything.
    #[tokio::test]
    async fn a_statement_that_breaks_its_chains_rules_is_refused_unasked() {
        let mut offgrid = *statement().raw();
        offgrid[65..69].copy_from_slice(&150i32.to_le_bytes());
        let cases = [
            (
                Statement::from_raw(offgrid),
                Refusal::OffGrid {
                    height: 150,
                    grid: 100,
                },
            ),
            (edited(117, &[0x44; 32]), Refusal::WrongShieldedCommitment),
            (edited(149, &[0x55; 32]), Refusal::WrongReplayContext),
            (edited(0, &[1]), Refusal::UnsupportedVersion(1)),
            // A dissent is never signed: the confirmer skips it before this.
            (edited(181, &[0; 48]), Refusal::Dissent),
        ];
        for (st, want) in cases {
            let n = node();
            let got = check_against_node(&n, &st, &rules(), &diary(), &Holds::none()).await;
            assert_eq!(got, Err(Mismatch::Shape(want.clone())), "{want}");
            assert_eq!(got.unwrap_err().next(), Next::Skip);
            assert!(n.methods().is_empty(), "{want}: {:?}", n.methods());
        }
    }

    /// A refused block at or below the statement's height on this node's
    /// chain: its diary is of a branch the app refuses, so nothing is said.
    #[tokio::test]
    async fn a_refused_block_under_the_base_says_nothing() {
        let held = Holds {
            invalid: &[],
            held: HOLD_AT_50,
        };
        let n = node();
        n.with(|s| {
            s.chain.insert(50, ROOT.into());
        });
        let got = check(&n, &diary(), &held).await.unwrap_err();
        assert_eq!(
            got,
            Mismatch::HeldRootOnActiveChain {
                height: 50,
                root: ROOT.into()
            }
        );
        assert_eq!(got.next(), Next::Skip);
        assert!(read_only(&n), "never refused here: {:?}", n.methods());
        // The same hold, not on this chain: nothing to say.
        assert!(check(&node(), &diary(), &held).await.is_ok());
        // A known-invalid block counts the same way.
        let invalid = [KnownInvalidBlock {
            height: 70,
            hash: ROOT,
            valid_sibling: R_BLOCK,
        }];
        let n = node();
        n.with(|s| {
            s.chain.insert(70, ROOT.into());
        });
        let holds = Holds {
            invalid: &invalid,
            held: &[],
        };
        assert_eq!(
            check(&n, &diary(), &holds).await,
            Err(Mismatch::HeldRootOnActiveChain {
                height: 70,
                root: ROOT.into()
            })
        );
        // One above the statement's height is not this statement's business.
        let above = [HeldBranch {
            height: 150,
            root: ROOT,
            why: "above",
        }];
        let n = node();
        n.with(|s| {
            s.chain.insert(150, ROOT.into());
        });
        let holds = Holds {
            invalid: &[],
            held: &above,
        };
        assert!(check(&n, &diary(), &holds).await.is_ok());
    }

    /// A node that does not answer proves nothing: wait, never skip or
    /// dissent.
    #[tokio::test]
    async fn a_node_that_does_not_answer_waits() {
        for method in ["getblockhash", "getmatmultrustedstatus", "getblockcount"] {
            let n = node();
            n.with(|s| {
                s.silent.insert(method);
            });
            let got = check(&n, &diary(), &Holds::none()).await.unwrap_err();
            assert!(
                matches!(got, Mismatch::NodeUnanswered(_)),
                "{method}: {got:?}"
            );
            assert_eq!(got.next(), Next::Wait);
        }
    }
}
