//! The diary: what this node's own chain state was at every grid height.
//!
//! A snapshot statement names a block and the UTXO set after it: the set's
//! hash (`hash_serialized_3`), its coin count and the chain's transaction
//! count. A confirmer signs a statement only when its OWN node wrote down the
//! same four things at that height, because `signutxosnapshotmanifest` signs
//! blindly: a node at height 0 co-signed a statement about block 100 in the
//! spike of 2026-09-29. This diary is the only real check a co-signature
//! carries, and a producer compares its own export with it before sending.
//! docs/decisions/2026-09-29-every-node-starts-near-the-tip.md, section 3.
//!
//! WHEN. `gettxoutsetinfo` answers only at the tip, so the app reads it when
//! the tip is on the grid (every multiple of 100, section 2, the same
//! [`cs::SNAPSHOT_GRID`] statements are checked against) and keeps the answer
//! only if the block it names is still the one at its height. A height the
//! tip passed before the app could read it is skipped; the next chance is
//! 100 blocks later.
//!
//! WHO. Only a node whose chain state rests on its own checks: it validates
//! (a mirror holds a UTXO set it never checked), and `getchainstates` shows
//! no chainstate at `"validated": false` ([`node_api::chainstates_validated`]).
//! That covers upstream's plain assumeutxo snapshot, which writes no
//! `attested_assumeutxo` file, as well as a signed one. Otherwise one
//! snapshot could vouch for the next. Once the background check has finished
//! and every chainstate is validated, the node writes again.
//!
//! WHAT COUNTS. The tip is read the moment it reaches the grid, and a sibling
//! can still replace that block (BTX forks about every 25 blocks). So an
//! entry counts only while its block is still the block at that height on the
//! node's active chain: [`entry_on_chain`] is the only way a comparison
//! should read the diary, and recording a new height drops every entry whose
//! block has left the chain ([`drop_entries_off_chain`]). A node that does not
//! answer proves nothing either way, so it drops nothing.
//!
//! WHERE. `<datadir>/snapshot-diary.json`, the newest 100 heights (10,000
//! blocks, about ten days at 40 an hour, longer than the seven days the
//! website's `pending` covers), written atomically, for one chain: a diary
//! from another chain reads as empty. No shielded commitment: no read-only
//! RPC returns one, and near the tip it is the compiled constant.

use crate::confirmed_snapshot as cs;
use crate::error::AppError;
use crate::node_api;
use crate::operators::Chain;
use crate::role::ValidationMode;
use crate::rpc::Rpc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

pub const DIARY_FILE: &str = "snapshot-diary.json";
/// Heights kept: 100 heights of 100 blocks.
pub const DIARY_KEEP: usize = 100;
pub const DIARY_VERSION: u32 = 1;
/// The grid the diary writes on: the statements' own, on every chain.
pub const DIARY_GRID: u64 = cs::SNAPSHOT_GRID as u64;

/// Whether `height` is a grid height. The genesis block is not.
pub fn on_grid(height: u64) -> bool {
    height > 0 && height.is_multiple_of(DIARY_GRID)
}

/// One height, as this node's engine reported it. Hashes are display hex,
/// lowercase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiaryEntry {
    pub height: u64,
    pub block_hash: String,
    /// `gettxoutsetinfo.hash_serialized_3`.
    pub hash_serialized: String,
    /// `gettxoutsetinfo.txouts`.
    pub coins: u64,
    /// `getchaintxstats 1 <block>`'s `txcount`.
    pub chain_tx: u64,
    /// Unix seconds.
    pub recorded_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diary {
    pub version: u32,
    /// The chain's genesis hash, display hex.
    pub chain_id: String,
    /// Ascending by height, one per height.
    pub entries: Vec<DiaryEntry>,
}

impl Diary {
    pub fn new(chain_id: &str) -> Self {
        Self {
            version: DIARY_VERSION,
            chain_id: chain_id.to_ascii_lowercase(),
            entries: Vec::new(),
        }
    }

    /// The entry at `height`, whether or not its block is still on the
    /// chain. A comparison goes through [`entry_on_chain`] instead.
    pub fn at(&self, height: u64) -> Option<&DiaryEntry> {
        self.entries.iter().find(|e| e.height == height)
    }

    pub fn newest(&self) -> Option<&DiaryEntry> {
        self.entries.last()
    }

    /// Keep `entry`, replacing what the diary had at its height, and only the
    /// newest [`DIARY_KEEP`] heights.
    pub fn record(&mut self, entry: DiaryEntry) {
        self.entries.retain(|e| e.height != entry.height);
        self.entries.push(entry);
        self.entries.sort_by_key(|e| e.height);
        let excess = self.entries.len().saturating_sub(DIARY_KEEP);
        self.entries.drain(..excess);
    }
}

/// `<dir>/snapshot-diary.json`; `dir` is the datadir.
pub fn diary_path(dir: &Path) -> PathBuf {
    dir.join(DIARY_FILE)
}

/// The diary for `chain_id`. A missing, unreadable, other-version or
/// other-chain file reads as an empty diary, which confirms nothing.
pub fn load(dir: &Path, chain_id: &str) -> Diary {
    let empty = Diary::new(chain_id);
    let Ok(raw) = std::fs::read(diary_path(dir)) else {
        return empty;
    };
    match serde_json::from_slice::<Diary>(&raw) {
        Ok(d) if d.version == DIARY_VERSION && d.chain_id.eq_ignore_ascii_case(chain_id) => d,
        _ => empty,
    }
}

/// Write the diary atomically (`fsx::atomic_write`): a reader sees the old
/// file or the new one, never half of either.
pub fn save(dir: &Path, diary: &Diary) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(diary).map_err(std::io::Error::other)?;
    crate::fsx::atomic_write(&diary_path(dir), &json)
}

/// One line for Copy diagnostics: how many heights the diary holds and the
/// newest, whatever chain it is for. `None` when there is no readable diary
/// or it is empty.
pub fn summary(dir: &Path) -> Option<String> {
    let d: Diary = serde_json::from_slice(&std::fs::read(diary_path(dir)).ok()?).ok()?;
    let newest = d.newest()?;
    let n = d.entries.len();
    Some(format!(
        "{n} {}, newest {} (block {})",
        if n == 1 { "height" } else { "heights" },
        newest.height,
        newest.block_hash.get(..16).unwrap_or(&newest.block_hash)
    ))
}

/// Whether this node may write in its diary now: it validates, and
/// `chainstates` (its `getchainstates` answer, `None` when the call failed)
/// shows every chainstate validated.
pub fn may_record(mode: ValidationMode, chainstates: Option<&serde_json::Value>) -> bool {
    mode == ValidationMode::Consensus && node_api::chainstates_validated(chainstates)
}

/// What one diary step came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiaryOutcome {
    Recorded(DiaryEntry),
    /// This height is in the diary already, for the block at it now.
    AlreadyRecorded(u64),
    /// The tip is not on the grid.
    NotOnGrid,
    /// The engine answered for a block that is not the one at that grid
    /// height any more, or for a height off the grid: the tip moved during
    /// the read.
    TipMoved,
    /// A mirror, a relay, or a node whose mode is unknown: it does not check
    /// blocks itself. Nothing was asked.
    NotValidating,
    /// `getchainstates` shows a snapshot chainstate still at
    /// `"validated": false`, or did not answer.
    Unvalidated,
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The block at `height` on the active chain, lowercase display hex.
/// `Ok(None)` when the chain does not reach that height (the engine's -8,
/// "Block height out of range"); `Err` when the node did not answer, which
/// says nothing about the chain.
async fn block_at(rpc: &dyn Rpc, height: u64) -> Result<Option<String>, String> {
    match rpc.call("getblockhash", json!([height])).await {
        Ok(v) => v
            .as_str()
            .map(|s| Some(s.to_ascii_lowercase()))
            .ok_or_else(|| format!("getblockhash {height} answered {v}")),
        Err(AppError::Rpc { code: -8, .. }) => Ok(None),
        Err(e) => Err(format!("getblockhash {height}: {e}")),
    }
}

/// The chain this node is on: its genesis hash, display hex. `None` when it
/// did not answer.
pub async fn chain_id(rpc: &dyn Rpc) -> Option<String> {
    block_at(rpc, 0).await.ok().flatten()
}

/// The diary's entry at `height`, only while its block is still the block at
/// that height on the node's active chain. `Ok(None)` for no entry, or an
/// entry whose block has left the chain (never compared). `Err` when the node
/// did not answer.
pub async fn entry_on_chain<'a>(
    rpc: &dyn Rpc,
    diary: &'a Diary,
    height: u64,
) -> Result<Option<&'a DiaryEntry>, String> {
    let Some(e) = diary.at(height) else {
        return Ok(None);
    };
    let now = block_at(rpc, height).await?;
    Ok((now.as_deref() == Some(e.block_hash.as_str())).then_some(e))
}

/// Drop every entry whose block is no longer the block at its height on the
/// node's active chain, and return their heights. Stops at the first height
/// the node does not answer for and keeps it and the rest: an unanswered
/// call proves nothing, and [`entry_on_chain`] checks again before any
/// comparison anyway.
pub async fn drop_entries_off_chain(rpc: &dyn Rpc, diary: &mut Diary) -> Vec<u64> {
    let mut gone = Vec::new();
    for e in &diary.entries {
        match block_at(rpc, e.height).await {
            Ok(now) if now.as_deref() == Some(e.block_hash.as_str()) => {}
            Ok(_) => gone.push(e.height),
            Err(_) => break,
        }
    }
    diary.entries.retain(|e| !gone.contains(&e.height));
    gone
}

/// Read the UTXO set at the tip. `Ok(None)` when the answer is not one to
/// keep: off the grid, or for a block no longer at its height.
pub async fn read_at_tip(rpc: &dyn Rpc) -> Result<Option<DiaryEntry>, String> {
    let v = rpc
        .call("gettxoutsetinfo", json!([]))
        .await
        .map_err(|e| format!("gettxoutsetinfo: {e}"))?;
    let (Some(height), Some(block), Some(coins), Some(hash)) = (
        v["height"].as_u64(),
        v["bestblock"].as_str(),
        v["txouts"].as_u64(),
        v["hash_serialized_3"].as_str(),
    ) else {
        return Err(format!("gettxoutsetinfo answered without the fields: {v}"));
    };
    let block = block.to_ascii_lowercase();
    if !on_grid(height) || block_at(rpc, height).await?.as_deref() != Some(block.as_str()) {
        return Ok(None);
    }
    let stats = rpc
        .call("getchaintxstats", json!([1, block]))
        .await
        .map_err(|e| format!("getchaintxstats: {e}"))?;
    let chain_tx = stats["txcount"]
        .as_u64()
        .ok_or_else(|| format!("getchaintxstats answered no txcount: {stats}"))?;
    Ok(Some(DiaryEntry {
        height,
        block_hash: block,
        hash_serialized: hash.to_ascii_lowercase(),
        coins,
        chain_tx,
        recorded_at: now_unix(),
    }))
}

/// One diary step, cheap when there is nothing to do: a mode that does not
/// validate asks nothing, and a tip off the grid costs one `getblockcount`.
/// The status refresher calls it every tick, the producer right before it
/// exports and `btx-confirmer` every few seconds. `dir` holds the diary (the
/// datadir, or the confirmer's `--state`); `mode` is
/// [`crate::role::validation_mode`] of the node's `getmatmultrustedstatus`.
///
/// In order: the mode; the tip on the grid; `getchainstates` all validated;
/// the chain (mainnet or regtest, else an error); the height not already
/// written for the block at it now; `gettxoutsetinfo` for that block; then
/// the entry is kept, entries whose block left the chain are dropped, and the
/// diary is written.
pub async fn record_at_tip(
    rpc: &dyn Rpc,
    dir: &Path,
    mode: ValidationMode,
) -> Result<DiaryOutcome, String> {
    if mode != ValidationMode::Consensus {
        return Ok(DiaryOutcome::NotValidating);
    }
    let tip = rpc
        .call("getblockcount", json!([]))
        .await
        .map_err(|e| format!("getblockcount: {e}"))?
        .as_u64()
        .ok_or_else(|| "getblockcount answered no number".to_string())?;
    if !on_grid(tip) {
        return Ok(DiaryOutcome::NotOnGrid);
    }
    let chainstates = rpc.call("getchainstates", json!([])).await.ok();
    if !may_record(mode, chainstates.as_ref()) {
        return Ok(DiaryOutcome::Unvalidated);
    }
    let genesis = chain_id(rpc)
        .await
        .ok_or_else(|| "getblockhash 0 did not answer".to_string())?;
    if Chain::from_genesis_hex(&genesis).is_none() {
        return Err(format!(
            "chain {genesis} is neither mainnet nor regtest, so it has no diary"
        ));
    }
    let mut diary = load(dir, &genesis);
    if let Some(known) = diary.at(tip).map(|e| e.block_hash.clone()) {
        if block_at(rpc, tip).await?.as_deref() == Some(known.as_str()) {
            return Ok(DiaryOutcome::AlreadyRecorded(tip));
        }
    }
    let Some(entry) = read_at_tip(rpc).await? else {
        return Ok(DiaryOutcome::TipMoved);
    };
    diary.record(entry.clone());
    // The new entry was just checked against the chain; the older ones are
    // checked now, once every 100 blocks rather than on every read.
    let mut older = Diary {
        entries: diary
            .entries
            .iter()
            .filter(|e| e.height != entry.height)
            .cloned()
            .collect(),
        ..diary.clone()
    };
    let gone = drop_entries_off_chain(rpc, &mut older).await;
    if !gone.is_empty() {
        eprintln!(
            "[diary] dropped {} entr{} whose block left the active chain: {gone:?}",
            gone.len(),
            if gone.len() == 1 { "y" } else { "ies" }
        );
    }
    diary.entries.retain(|e| !gone.contains(&e.height));
    save(dir, &diary).map_err(|e| format!("writing the diary: {e}"))?;
    Ok(DiaryOutcome::Recorded(entry))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_node::{sibling_hash, synthetic_hash, FakeNode};
    use crate::operators::{MAINNET_GENESIS, REGTEST_GENESIS};
    use serde_json::json;

    // The spike's regtest export at 100 (the fixture regtest-*.manifest).
    const R_BLOCK: &str = "bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46";
    const R_UTXO: &str = "e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527";
    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");

    fn regtest_at_100() -> FakeNode {
        let node = FakeNode::new(REGTEST_GENESIS, 100);
        node.with(|s| {
            s.chain.insert(100, R_BLOCK.into());
            s.utxo = Some(json!({
                "height": 100, "bestblock": R_BLOCK, "txouts": 101,
                "hash_serialized_3": R_UTXO, "transactions": 101
            }));
            s.chain_tx = 101;
        });
        node
    }

    /// The node's UTXO answer at its tip, for a synthetic chain.
    fn utxo_at_tip(node: &FakeNode) {
        node.with(|s| {
            let tip = s.tip();
            s.utxo = Some(json!({
                "height": tip, "bestblock": s.chain[&tip], "txouts": tip + 1,
                "hash_serialized_3": "cd".repeat(32), "transactions": tip + 1
            }));
            s.chain_tx = tip + 1;
        });
    }

    fn entry(height: u64) -> DiaryEntry {
        DiaryEntry {
            height,
            block_hash: synthetic_hash(height),
            hash_serialized: "ab".repeat(32),
            coins: height,
            chain_tx: height,
            recorded_at: 0,
        }
    }

    #[tokio::test]
    async fn a_grid_height_is_written_down_as_the_engine_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let node = regtest_at_100();
        let out = record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        let DiaryOutcome::Recorded(e) = out else {
            panic!("{out:?}")
        };
        assert_eq!(
            (
                e.height,
                e.block_hash.as_str(),
                e.hash_serialized.as_str(),
                e.coins,
                e.chain_tx
            ),
            (100, R_BLOCK, R_UTXO, 101, 101)
        );
        // The same four facts the producer's statement carries at 100, as
        // `dumptxoutsetattested` wrote it in the spike.
        let st = cs::parse(R_P).unwrap().statement;
        assert_eq!(
            (
                st.height() as u64,
                st.block_hash().display_hex(),
                st.hash_serialized().display_hex(),
                st.coins(),
                st.chain_tx()
            ),
            (
                e.height,
                e.block_hash.clone(),
                e.hash_serialized.clone(),
                e.coins,
                e.chain_tx
            )
        );
        let d = load(dir.path(), REGTEST_GENESIS);
        assert_eq!(d.at(100), Some(&e));
        assert_eq!(
            node.methods(),
            vec![
                "getblockcount",
                "getchainstates",
                "getblockhash",
                "gettxoutsetinfo",
                "getblockhash",
                "getchaintxstats"
            ]
        );
    }

    #[tokio::test]
    async fn the_same_height_twice_reads_the_utxo_set_once() {
        let dir = tempfile::tempdir().unwrap();
        let node = regtest_at_100();
        record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        let again = record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        assert_eq!(again, DiaryOutcome::AlreadyRecorded(100));
        assert_eq!(node.count("gettxoutsetinfo"), 1);
    }

    /// A sibling replaced the block at 100 after it was written down: the
    /// tip is on the grid again, at the same height, with another block, so
    /// the height is read afresh and the old entry replaced.
    #[tokio::test]
    async fn a_new_block_at_a_recorded_height_is_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let node = regtest_at_100();
        record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        node.with(|s| s.reorg_from(100, 100));
        utxo_at_tip(&node);
        let out = record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        let DiaryOutcome::Recorded(e) = out else {
            panic!("{out:?}")
        };
        assert_eq!(e.block_hash, sibling_hash(100));
        let d = load(dir.path(), REGTEST_GENESIS);
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.at(100), Some(&e));
    }

    #[tokio::test]
    async fn off_the_grid_nothing_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let node = FakeNode::new(MAINNET_GENESIS, 226_001);
        let out = record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::NotOnGrid);
        assert_eq!(node.methods(), vec!["getblockcount"]);
    }

    /// Section 2: every multiple of 100, so upstream's compiled heights are
    /// on it. Height 0 (the genesis block) is not a snapshot.
    #[test]
    fn the_grid_is_every_hundred_blocks() {
        assert_eq!(DIARY_GRID, 100);
        for h in [100, 219_000, 225_900, 228_000, 233_800] {
            assert!(on_grid(h), "{h}");
        }
        for h in [0, 50, 150, 200_001, 225_927, 226_001] {
            assert!(!on_grid(h), "{h}");
        }
    }

    #[tokio::test]
    async fn an_answer_for_a_block_that_moved_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        // The tip moved on while the engine read: it answers for 101.
        let node = regtest_at_100();
        node.with(|s| {
            s.utxo.as_mut().unwrap()["height"] = json!(101);
        });
        let out = record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::TipMoved);
        // A reorg while the engine read: the answer names a block that is no
        // longer the one at 100.
        let node = regtest_at_100();
        node.with(|s| {
            s.utxo.as_mut().unwrap()["bestblock"] = json!("ff".repeat(32));
        });
        let out = record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::TipMoved);
        assert!(!diary_path(dir.path()).exists());
    }

    /// A mirror holds a UTXO set it never checked: it writes nothing and
    /// asks the node nothing.
    #[tokio::test]
    async fn a_node_that_does_not_validate_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let node = regtest_at_100();
        for mode in [
            ValidationMode::Trusted,
            ValidationMode::Relay,
            ValidationMode::Unknown,
        ] {
            let out = record_at_tip(&node, dir.path(), mode).await.unwrap();
            assert_eq!(out, DiaryOutcome::NotValidating);
        }
        assert!(node.methods().is_empty(), "not even a read");
        assert!(!diary_path(dir.path()).exists());
    }

    /// Section 3's gate. Upstream's plain assumeutxo snapshot writes no
    /// `attested_assumeutxo` file and a signed one does; both show a
    /// snapshot chainstate at `"validated": false`, and on both nothing is
    /// written. Once every chainstate is validated the node writes again,
    /// although the file is still there (it stays until the next start).
    #[tokio::test]
    async fn an_unvalidated_snapshot_chainstate_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let node = regtest_at_100();
        node.with(|s| s.on_unchecked_snapshot(&synthetic_hash(50)));
        // Plain assumeutxo: no file.
        let out = record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::Unvalidated);
        // Signed: the file exists.
        let record = crate::node::attested_snapshot_record(dir.path());
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, b"v2").unwrap();
        let out = record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::Unvalidated);
        // A getchainstates that does not answer is not an open gate.
        node.with(|s| {
            s.chainstates = None;
            s.silent.insert("getchainstates");
        });
        let out = record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::Unvalidated);
        assert_eq!(node.count("gettxoutsetinfo"), 0);
        assert!(!diary_path(dir.path()).exists());
        // The background check finished.
        node.with(|s| {
            s.silent.clear();
        });
        let out = record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        assert!(matches!(out, DiaryOutcome::Recorded(_)), "{out:?}");
        assert!(record.exists());
    }

    /// An entry counts only while its block is still the block at that
    /// height on the active chain. Recording a new height drops every entry
    /// whose block left it: one replaced by a sibling, one above the tip
    /// after the chain got shorter.
    #[tokio::test]
    async fn an_entry_whose_block_left_the_chain_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = Diary::new(REGTEST_GENESIS);
        for h in [100, 200, 400] {
            d.record(entry(h));
        }
        save(dir.path(), &d).unwrap();
        let node = FakeNode::new(REGTEST_GENESIS, 400);
        node.with(|s| s.reorg_from(150, 300));
        utxo_at_tip(&node);
        assert_eq!(
            entry_on_chain(&node, &d, 100).await.unwrap(),
            Some(&entry(100))
        );
        assert_eq!(entry_on_chain(&node, &d, 200).await.unwrap(), None);
        assert_eq!(entry_on_chain(&node, &d, 400).await.unwrap(), None);
        assert_eq!(
            entry_on_chain(&node, &d, 300).await.unwrap(),
            None,
            "no entry"
        );
        let out = record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .unwrap();
        assert!(matches!(out, DiaryOutcome::Recorded(_)), "{out:?}");
        let d = load(dir.path(), REGTEST_GENESIS);
        let heights: Vec<u64> = d.entries.iter().map(|e| e.height).collect();
        assert_eq!(heights, vec![100, 300]);
        assert_eq!(d.at(300).unwrap().block_hash, sibling_hash(300));
    }

    /// A node that stops answering proves nothing about the chain: nothing
    /// is dropped, and asking for an entry is an error, not a "no".
    #[tokio::test]
    async fn a_node_that_stops_answering_drops_nothing() {
        let mut d = Diary::new(REGTEST_GENESIS);
        for h in [100, 200] {
            d.record(DiaryEntry {
                block_hash: "ee".repeat(32),
                ..entry(h)
            });
        }
        let node = FakeNode::new(REGTEST_GENESIS, 300);
        node.with(|s| {
            s.silent.insert("getblockhash");
        });
        assert!(drop_entries_off_chain(&node, &mut d).await.is_empty());
        assert_eq!(d.entries.len(), 2);
        assert!(entry_on_chain(&node, &d, 100).await.is_err());
        // Answering again, both are off the chain (another block at each).
        node.with(|s| s.silent.clear());
        assert_eq!(drop_entries_off_chain(&node, &mut d).await, vec![100, 200]);
        assert!(d.entries.is_empty());
    }

    #[test]
    fn the_diary_keeps_the_newest_hundred_heights_one_each() {
        let mut d = Diary::new(MAINNET_GENESIS);
        for i in (1..=120u64).rev() {
            d.record(entry(i * 100));
        }
        d.record(DiaryEntry {
            coins: 7,
            ..entry(12_000)
        });
        assert_eq!(DIARY_KEEP, 100);
        assert_eq!(d.entries.len(), DIARY_KEEP);
        assert_eq!(d.entries.first().unwrap().height, 2_100);
        assert_eq!(d.newest().unwrap().height, 12_000);
        assert_eq!(d.at(12_000).unwrap().coins, 7);
        assert!(d.at(2_000).is_none());
        assert!(d.entries.windows(2).all(|w| w[0].height < w[1].height));
    }

    #[test]
    fn a_diary_from_another_chain_or_a_broken_file_reads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = Diary::new(REGTEST_GENESIS);
        d.record(entry(100));
        save(dir.path(), &d).unwrap();
        assert_eq!(load(dir.path(), REGTEST_GENESIS), d);
        assert_eq!(
            load(dir.path(), MAINNET_GENESIS),
            Diary::new(MAINNET_GENESIS)
        );
        std::fs::write(diary_path(dir.path()), b"{ not json").unwrap();
        assert_eq!(
            load(dir.path(), REGTEST_GENESIS),
            Diary::new(REGTEST_GENESIS)
        );
        assert_eq!(
            summary(dir.path()),
            None,
            "a broken file summarises as nothing"
        );
        save(dir.path(), &d).unwrap();
        assert_eq!(
            summary(dir.path()).as_deref(),
            Some("1 height, newest 100 (block 0000000000000000)")
        );
        assert_eq!(
            diary_path(dir.path()),
            dir.path().join("snapshot-diary.json")
        );
    }

    /// A node on neither mainnet nor regtest has no diary: an error, and
    /// nothing written.
    #[tokio::test]
    async fn a_node_on_an_unknown_chain_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let node = FakeNode::new(&"00".repeat(32), 100);
        utxo_at_tip(&node);
        assert!(record_at_tip(&node, dir.path(), ValidationMode::Consensus)
            .await
            .is_err());
        assert!(!diary_path(dir.path()).exists());
    }
}
