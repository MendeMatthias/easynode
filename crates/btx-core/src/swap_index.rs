//! The in-memory index behind the witness's five swap routes.
//!
//! ── WHY A WITNESS NEEDS AN INDEX AT ALL ──────────────────────────────────────
//! An HTLC swap client asks five questions about one transaction: is it
//! confirmed and where (`/tx/<txid>/status`), what are its bytes (`/hex`), how
//! does it hash into its block (`/merkle-proof`), has this output been spent and
//! by what (`/outspend/<vout>`), and what is that block's header
//! (`/block/<hash>/header`). A node answers the last one from its block index,
//! pruned or not. The first four need to know which block holds a txid, and a
//! node without `-txindex` (the whole easyNode fleet runs `prune=4096`, no
//! txindex) cannot say. Nor can any node say what spent an output once the
//! spend is confirmed: the UTXO set forgets spent outputs.
//!
//! So this keeps exactly those two facts, for the blocks the node still holds:
//! txid → (height, position) and outpoint → (spending txid, input index,
//! height). BTX carries very little transaction data (one ~200-byte coinbase a
//! block for most of its history), so this is megabytes, not gigabytes; the
//! numbers measured on a real pruned mainnet datadir are in
//! `docs/esplora-mode.md`.
//!
//! ── THE WINDOW, AND WHY "UNSPENT" IS ONLY SAID INSIDE IT ─────────────────────
//! The index covers a contiguous run of the active chain, from a bottom height
//! up to the node's tip. That contiguity is what makes one answer possible that
//! is otherwise never safe: if the transaction that CREATED an output is in the
//! window, every block that could have spent it is in the window too, so a
//! spend the index has not seen does not exist (up to the node's tip). For an
//! output created below the window the index knows nothing, and the route says
//! so rather than guessing. See `witness::answer_outspend` for the whole rule.
//!
//! ── REORGS ──────────────────────────────────────────────────────────────────
//! Every block is connected only onto the current tip (its `previousblockhash`
//! must be the tip's hash, its height the tip's plus one), and disconnecting
//! removes exactly what that block added. The sync step asks the node whether
//! the index's tip is still on the active chain; while it is not, the tip is
//! disconnected, one block at a time, until it is. A reorg deeper than the
//! window empties the index and it is rebuilt. Answers that report a block are
//! re-checked against the node at request time (`getblockhash` at that height),
//! so a reorg the sync has not reached yet produces a 503, never a stale "yes".
//!
//! ── WHAT IT DOES NOT HOLD ───────────────────────────────────────────────────
//! No transaction bytes (fetched on demand with `getrawtransaction … <block>`,
//! which works pruned while the block is on disk), no amounts, no scripts, no
//! addresses. Nothing here could become an address index, and nothing should.

use crate::error::{AppError, AppResult};
use crate::rpc::Rpc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

/// A 32-byte id (txid or block hash) in DISPLAY order: the bytes of the hex
/// string as the node prints it. Reversed only for hashing.
pub type Id = [u8; 32];

/// How many blocks the index covers by default, counted down from the tip.
///
/// About 104 days at BTX's 90-second blocks: far longer than any HTLC refund
/// timelock a swap would use, and enough that a client checking a funding
/// transaction weeks later still gets an answer. A pruned node holds fewer
/// blocks than this only if its prune target is small; on the fleet's
/// `prune=4096` the node held 105,640 blocks on 2026-11-28 (pruneheight
/// 184,942), because post-fork BTX blocks are ~8 kB. On an archival node the
/// cap is what stops the startup scan from walking the ~1 MB-a-block MatMul
/// era (heights ~62,000 to ~185,000), which is 123 GiB of disk reads for a
/// window no swap needs.
pub const DEFAULT_WINDOW: u32 = 100_000;

/// The smallest window accepted. Below this a swap's own lifetime would not
/// fit, and the "funding is in the window" rule would rarely apply.
pub const MIN_WINDOW: u32 = 144;

/// How often the index asks the node whether its tip moved, when nothing else
/// wakes it. One `getblockheader` call per poll.
pub const POLL: Duration = Duration::from_secs(2);

/// After reading a block, wait this many times as long as the read took.
///
/// The index reads one block at a time, so unpaced it would keep one of the
/// node's RPC threads busy for the whole first build (100,000 blocks, about
/// four minutes on the 3060 signer), and `getblock` takes the node's main lock
/// to find the block. A node that signs blocks must not be slowed by a witness
/// beside it: measured 2026-11-28, an unpaced build took that machine's load
/// average from 4 to 7. At 2 the build uses at most a third of one RPC thread
/// and takes about three times as long, once, at start.
pub const BUILD_PACE: u32 = 2;

/// Parse exactly 64 lowercase hex characters. Uppercase is refused on
/// purpose: one id has one spelling, on the wire and in the node's output.
pub fn parse_id(s: &str) -> Option<Id> {
    let b = s.as_bytes();
    if b.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in b.chunks_exact(2).enumerate() {
        out[i] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(out)
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Lowercase hex of an id, in the order it is stored (display order).
pub fn id_hex(id: &Id) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for b in id {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

fn sha256d(data: &[u8]) -> [u8; 32] {
    let once = Sha256::digest(data);
    Sha256::digest(once).into()
}

fn reversed(id: &Id) -> Id {
    let mut r = *id;
    r.reverse();
    r
}

/// The merkle root of a block's txids and the branch for the one at `pos`,
/// all in display order — the same computation as electrs'
/// `create_merkle_branch_and_root`, so the branch is byte-for-byte what
/// `api.btxscan.io` serves: at each level an odd last node is paired with
/// itself, the sibling is `pos ^ 1`, and `pos` halves.
///
/// Panics if `txids` is empty or `pos` is out of range; callers hold a block
/// that has at least its coinbase and a position read from that block.
pub fn merkle_root_and_branch(txids: &[Id], pos: usize) -> (Id, Vec<Id>) {
    assert!(pos < txids.len(), "merkle position out of range");
    let mut level: Vec<Id> = txids.iter().map(reversed).collect();
    let mut idx = pos;
    let mut branch = Vec::new();
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            let last = *level.last().expect("non-empty");
            level.push(last);
        }
        branch.push(reversed(&level[idx ^ 1]));
        idx >>= 1;
        level = level
            .chunks_exact(2)
            .map(|p| {
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(&p[0]);
                buf[32..].copy_from_slice(&p[1]);
                sha256d(&buf)
            })
            .collect();
    }
    (reversed(&level[0]), branch)
}

/// One transaction, as much of it as the index keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTx {
    pub txid: Id,
    /// `(input index, previous txid, previous vout)` for every input that
    /// spends an output. A coinbase input spends nothing and is absent, but
    /// still counts in the input index, so `vin` matches the transaction.
    pub spends: Vec<(u32, Id, u32)>,
}

/// One block, read from `getblock <hash> 2` and checked for shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBlock {
    pub hash: Id,
    pub height: u32,
    /// The header's own timestamp, which is what electrs reports as
    /// `block_time` (not the median time).
    pub time: u32,
    pub prev: Option<Id>,
    pub next: Option<Id>,
    pub merkle_root: Id,
    pub txs: Vec<ParsedTx>,
}

fn field_id(v: &Value, k: &str) -> Result<Id, String> {
    v.get(k)
        .and_then(Value::as_str)
        .and_then(parse_id)
        .ok_or_else(|| format!("missing or malformed '{k}'"))
}

fn opt_id(v: &Value, k: &str) -> Result<Option<Id>, String> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => x
            .as_str()
            .and_then(parse_id)
            .map(Some)
            .ok_or_else(|| format!("malformed '{k}'")),
    }
}

fn field_u32(v: &Value, k: &str) -> Result<u32, String> {
    v.get(k)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| format!("missing or malformed '{k}'"))
}

/// Read a `getblock <hash> 2` answer, strictly.
///
/// Every input must be either a coinbase or a `(txid, vout)` pair. An input of
/// any other shape refuses the whole block, and with it every block after: an
/// index that skipped an input it could not read would later say "unspent"
/// about an output that input spent, which is the one answer this must never
/// get wrong. A stalled index costs a 503; a silently incomplete one costs
/// someone's swap.
pub fn parse_block(v: &Value) -> Result<ParsedBlock, String> {
    let hash = field_id(v, "hash")?;
    let height = field_u32(v, "height")?;
    let time = field_u32(v, "time")?;
    let prev = opt_id(v, "previousblockhash")?;
    let next = opt_id(v, "nextblockhash")?;
    let merkle_root = field_id(v, "merkleroot")?;
    let txs_v = v
        .get("tx")
        .and_then(Value::as_array)
        .ok_or("missing 'tx' (is this verbosity 2?)")?;
    if txs_v.is_empty() {
        return Err("a block with no transactions".into());
    }
    let mut txs = Vec::with_capacity(txs_v.len());
    for t in txs_v {
        if !t.is_object() {
            return Err(
                "a transaction that is not an object (verbosity 1 gives txids only)".into(),
            );
        }
        let txid = field_id(t, "txid")?;
        let vin = t
            .get("vin")
            .and_then(Value::as_array)
            .ok_or("a transaction without 'vin'")?;
        let mut spends = Vec::new();
        for (i, input) in vin.iter().enumerate() {
            let i = u32::try_from(i).map_err(|_| "too many inputs".to_string())?;
            if input.get("coinbase").is_some() {
                if input.get("txid").is_some() {
                    return Err("an input that is both coinbase and a spend".into());
                }
                continue;
            }
            let prev_txid = field_id(input, "txid")
                .map_err(|e| format!("input {i} of {}: {e}", id_hex(&txid)))?;
            let prev_vout = field_u32(input, "vout")
                .map_err(|e| format!("input {i} of {}: {e}", id_hex(&txid)))?;
            spends.push((i, prev_txid, prev_vout));
        }
        txs.push(ParsedTx { txid, spends });
    }
    Ok(ParsedBlock {
        hash,
        height,
        time,
        prev,
        next,
        merkle_root,
        txs,
    })
}

/// Where a confirmed transaction is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockRef {
    pub height: u32,
    pub hash: Id,
    pub time: u32,
}

/// A confirmed spend of an outpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spend {
    pub txid: Id,
    pub vin: u32,
    pub block: BlockRef,
}

/// A confirmed transaction's merkle proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    pub block: BlockRef,
    pub branch: Vec<Id>,
    pub pos: u32,
}

struct BlockEntry {
    hash: Id,
    time: u32,
    txids: Vec<Id>,
    /// The outpoints this block spends, so disconnecting it can remove them.
    spent: Vec<(Id, u32)>,
    /// Whether the txids hash to the header's merkle root. A block that does
    /// not is still indexed (its spends are real) but no proof is served from
    /// it: a proof that does not verify is worse than none.
    merkle_ok: bool,
}

/// Why a block could not be connected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// It does not extend the current tip: a reorg or a race. Not a fault.
    NotOnTip,
}

/// The index itself. Pure data and pure methods, so every rule is tested
/// without a node; `SwapIndex` wraps it with the sync loop.
#[derive(Default)]
pub struct IndexState {
    /// True once the first build reached the node's tip. Until then nothing
    /// is answered from the index (503): a half-built index would call an
    /// output created in its window "unspent" while the spend sits in a block
    /// it has not read yet.
    pub ready: bool,
    bottom: u32,
    blocks: VecDeque<BlockEntry>,
    by_hash: HashMap<Id, u32>,
    txs: HashMap<Id, (u32, u32)>,
    /// outpoint → (height, position of the spending tx in that block, input
    /// index). The spender's txid is read from the block's txids rather than
    /// stored again: on mainnet this map is the largest thing here.
    spends: HashMap<(Id, u32), (u32, u32, u32)>,
}

/// Counts for logs and for the measurements in the PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndexStats {
    pub blocks: usize,
    pub txs: usize,
    pub spends: usize,
    pub bottom: u32,
    pub tip: u32,
    /// What the index's own structures occupy, from their capacities: the
    /// number to compare with a process's RSS, which also holds the runtime,
    /// the HTTP server and whatever the allocator has not handed back.
    pub approx_bytes: usize,
}

impl IndexState {
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// `(height, hash)` of the newest block held.
    pub fn tip(&self) -> Option<(u32, Id)> {
        let last = self.blocks.back()?;
        Some((self.bottom + self.blocks.len() as u32 - 1, last.hash))
    }

    pub fn bottom(&self) -> Option<u32> {
        (!self.blocks.is_empty()).then_some(self.bottom)
    }

    pub fn stats(&self) -> IndexStats {
        IndexStats {
            blocks: self.blocks.len(),
            txs: self.txs.len(),
            spends: self.spends.len(),
            bottom: self.bottom().unwrap_or(0),
            tip: self.tip().map(|t| t.0).unwrap_or(0),
            approx_bytes: self.approx_bytes(),
        }
    }

    fn approx_bytes(&self) -> usize {
        use std::mem::size_of;
        // hashbrown: one control byte per bucket beside each slot.
        fn map<K, V>(m: &HashMap<K, V>) -> usize {
            m.capacity() * (size_of::<K>() + size_of::<V>() + 1)
        }
        let blocks = self.blocks.capacity() * size_of::<BlockEntry>()
            + self
                .blocks
                .iter()
                .map(|b| {
                    b.txids.capacity() * size_of::<Id>()
                        + b.spent.capacity() * size_of::<(Id, u32)>()
                })
                .sum::<usize>();
        blocks + map(&self.by_hash) + map(&self.txs) + map(&self.spends)
    }

    fn entry(&self, height: u32) -> Option<&BlockEntry> {
        let i = height.checked_sub(self.bottom)?;
        self.blocks.get(i as usize)
    }

    fn block_ref(&self, height: u32) -> Option<BlockRef> {
        let e = self.entry(height)?;
        Some(BlockRef {
            height,
            hash: e.hash,
            time: e.time,
        })
    }

    pub fn has_block(&self, hash: &Id) -> bool {
        self.by_hash.contains_key(hash)
    }

    /// Add a block on top of the tip. An empty index accepts any block as its
    /// first; after that a block must name the tip as its parent.
    pub fn connect(&mut self, b: &ParsedBlock) -> Result<(), ConnectError> {
        match self.tip() {
            None => self.bottom = b.height,
            Some((h, hash)) => {
                if b.height != h + 1 || b.prev != Some(hash) {
                    return Err(ConnectError::NotOnTip);
                }
            }
        }
        let txids: Vec<Id> = b.txs.iter().map(|t| t.txid).collect();
        let (root, _) = merkle_root_and_branch(&txids, 0);
        let merkle_ok = root == b.merkle_root;
        if !merkle_ok {
            eprintln!(
                "[witness] block {} at {}: txids do not hash to its merkle root; no proofs will be served from it",
                id_hex(&b.hash),
                b.height
            );
        }
        let mut spent = Vec::with_capacity(b.txs.iter().map(|t| t.spends.len()).sum());
        for (pos, t) in b.txs.iter().enumerate() {
            self.txs.insert(t.txid, (b.height, pos as u32));
            for &(vin, prev, vout) in &t.spends {
                self.spends
                    .insert((prev, vout), (b.height, pos as u32, vin));
                spent.push((prev, vout));
            }
        }
        self.by_hash.insert(b.hash, b.height);
        self.blocks.push_back(BlockEntry {
            hash: b.hash,
            time: b.time,
            txids,
            spent,
            merkle_ok,
        });
        Ok(())
    }

    fn forget(&mut self, height: u32, e: &BlockEntry) {
        // Remove only what THIS height put there. A txid or outpoint that a
        // later block re-used (never on a valid chain, but this is not the
        // place to assume it) keeps the later entry.
        for t in &e.txids {
            if self.txs.get(t).map(|x| x.0) == Some(height) {
                self.txs.remove(t);
            }
        }
        for op in &e.spent {
            if self.spends.get(op).map(|x| x.0) == Some(height) {
                self.spends.remove(op);
            }
        }
        if self.by_hash.get(&e.hash) == Some(&height) {
            self.by_hash.remove(&e.hash);
        }
    }

    /// Remove the tip block and everything it added.
    pub fn disconnect_tip(&mut self) -> Option<Id> {
        let (h, _) = self.tip()?;
        let e = self.blocks.pop_back()?;
        self.forget(h, &e);
        Some(e.hash)
    }

    /// Drop blocks from the bottom until at most `window` remain. Safe for the
    /// "funding is in the window" rule: a spend is never in an earlier block
    /// than the output it spends (the same block, or a later one), so
    /// trimming from below never keeps a funding transaction while dropping
    /// its spend.
    pub fn trim(&mut self, window: u32) {
        while self.blocks.len() > window as usize {
            let h = self.bottom;
            if let Some(e) = self.blocks.pop_front() {
                self.forget(h, &e);
            }
            self.bottom += 1;
        }
    }

    /// Forget everything; the next sync rebuilds.
    pub fn clear(&mut self) {
        *self = IndexState::default();
    }

    /// Where a txid is confirmed, if it is in the window.
    pub fn tx(&self, txid: &Id) -> Option<BlockRef> {
        let (h, _) = self.txs.get(txid)?;
        self.block_ref(*h)
    }

    /// The confirmed spend of an outpoint, if it is in the window.
    pub fn spend(&self, txid: &Id, vout: u32) -> Option<Spend> {
        let &(h, pos, vin) = self.spends.get(&(*txid, vout))?;
        let spender = *self.entry(h)?.txids.get(pos as usize)?;
        Some(Spend {
            txid: spender,
            vin,
            block: self.block_ref(h)?,
        })
    }

    /// The merkle proof of a txid in the window. `Err(())` when the tx is
    /// there but its block's txids did not hash to its root.
    pub fn proof(&self, txid: &Id) -> Option<Result<Proof, ()>> {
        let (h, pos) = self.txs.get(txid)?;
        let e = self.entry(*h)?;
        if !e.merkle_ok {
            return Some(Err(()));
        }
        let (_, branch) = merkle_root_and_branch(&e.txids, *pos as usize);
        Some(Ok(Proof {
            block: self.block_ref(*h)?,
            branch,
            pos: *pos,
        }))
    }
}

/// Hand memory the allocator holds but no longer uses back to the system.
///
/// The first build parses a hundred thousand `getblock` answers, some of them
/// hundreds of kilobytes of JSON (post-quantum witnesses are large), and glibc
/// keeps the freed pages in its per-thread arenas. Without this the process
/// keeps its build-time peak as RSS for its whole life, which on a desktop
/// running the app reads as a leak. Elsewhere it is a no-op.
fn release_free_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim only returns free memory to the OS; it touches no
    // live allocation and is safe to call from any thread at any time.
    unsafe {
        libc::malloc_trim(0);
    }
}

/// What one sync step did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOutcome {
    /// The index's tip is the node's tip.
    Current,
    /// Blocks were connected or disconnected; call again.
    Moved,
    /// The index was emptied (a reorg below its bottom) and will rebuild.
    Reset,
}

/// The index and the loop that keeps it on the node's active chain.
pub struct SwapIndex {
    pub state: tokio::sync::RwLock<IndexState>,
    pub window: u32,
    /// Woken by a request that found the index behind the node, so it need
    /// not wait for the next poll.
    pub wake: tokio::sync::Notify,
}

/// Every RPC the index makes takes one permit from the server's in-flight
/// bound first, so the index and the requests together never hold more than
/// `MAX_INFLIGHT` calls open against the node.
async fn call(
    rpc: &dyn Rpc,
    gate: &tokio::sync::Semaphore,
    method: &str,
    params: Value,
) -> AppResult<Value> {
    let _permit = gate
        .acquire()
        .await
        .map_err(|_| AppError::Config("the witness is shutting down".into()))?;
    rpc.call(method, params).await
}

impl SwapIndex {
    pub fn new(window: u32) -> Self {
        Self {
            state: tokio::sync::RwLock::new(IndexState::default()),
            window: window.max(MIN_WINDOW),
            wake: tokio::sync::Notify::new(),
        }
    }

    async fn fetch(
        &self,
        rpc: &dyn Rpc,
        gate: &tokio::sync::Semaphore,
        hash: &Id,
    ) -> AppResult<ParsedBlock> {
        let started = tokio::time::Instant::now();
        let v = call(rpc, gate, "getblock", json!([id_hex(hash), 2])).await;
        // Paced whether it worked or not: a failing node is not asked faster.
        tokio::time::sleep(started.elapsed() * BUILD_PACE).await;
        let v = v?;
        let b = parse_block(&v).map_err(AppError::Decode)?;
        if b.hash != *hash {
            return Err(AppError::Decode(format!(
                "asked for block {} and got {}",
                id_hex(hash),
                id_hex(&b.hash)
            )));
        }
        Ok(b)
    }

    /// The first build: from the bottom of the window to the tip, following
    /// `nextblockhash`. Returns once it has reached a block with no next.
    async fn build(&self, rpc: &dyn Rpc, gate: &tokio::sync::Semaphore) -> AppResult<()> {
        let info = call(rpc, gate, "getblockchaininfo", json!([])).await?;
        let tip = info
            .get("blocks")
            .and_then(Value::as_u64)
            .ok_or_else(|| AppError::Decode("getblockchaininfo without 'blocks'".into()))?;
        let pruned = info.get("pruned").and_then(Value::as_bool).unwrap_or(false);
        let lowest = if pruned {
            info.get("pruneheight").and_then(Value::as_u64).unwrap_or(0)
        } else {
            0
        };
        let mut height = lowest.max((tip + 1).saturating_sub(self.window as u64));
        let started = std::time::Instant::now();
        eprintln!(
            "[witness] swap index: reading blocks {height}..{tip} (window {}, pruned={pruned})",
            self.window
        );
        // The node may prune the bottom while this starts; step past it.
        let mut next = loop {
            let h = call(rpc, gate, "getblockhash", json!([height])).await?;
            let hash = h
                .as_str()
                .and_then(parse_id)
                .ok_or_else(|| AppError::Decode("getblockhash gave no hash".into()))?;
            match self.fetch(rpc, gate, &hash).await {
                Ok(b) => break b,
                Err(AppError::Rpc { message, .. })
                    if message.contains("pruned") && height < tip =>
                {
                    height += 1;
                }
                Err(e) => return Err(e),
            }
        };
        let mut n = 0u64;
        loop {
            {
                let mut s = self.state.write().await;
                if s.connect(&next).is_err() {
                    // Not reached by a reorg in practice: a block fetched by
                    // its parent's `nextblockhash` names that parent. A reorg
                    // under the scan instead ends it on a block with no next,
                    // which the incremental sync then finds off the active
                    // chain and disconnects.
                    break;
                }
            }
            n += 1;
            // Through the build too, not only after it: the peak is what an
            // operator's machine has to have free, and without this it is set
            // by every arena the runtime's worker threads touched.
            if n.is_multiple_of(2_000) {
                release_free_memory();
            }
            if n.is_multiple_of(10_000) {
                eprintln!(
                    "[witness] swap index: {n} blocks read, at {} ({:.0}s)",
                    next.height,
                    started.elapsed().as_secs_f64()
                );
            }
            let Some(h) = next.next else { break };
            next = self.fetch(rpc, gate, &h).await?;
        }
        // Not ready yet: where the scan stopped is the node's tip only if the
        // node still says so. The next sync step asks, disconnects a block a
        // reorg left behind, and marks the index ready at the active tip.
        let mut s = self.state.write().await;
        s.trim(self.window);
        let st = s.stats();
        drop(s);
        release_free_memory();
        eprintln!(
            "[witness] swap index read: {} blocks ({}..{}), {} txs, {} spent outpoints, ~{:.1} MiB, in {:.1}s",
            st.blocks,
            st.bottom,
            st.tip,
            st.txs,
            st.spends,
            st.approx_bytes as f64 / 1048576.0,
            started.elapsed().as_secs_f64()
        );
        Ok(())
    }

    /// One step toward the node's active tip. Disconnects one stale block, or
    /// connects every block from the tip forward, or builds from nothing.
    pub async fn sync_once(
        &self,
        rpc: &dyn Rpc,
        gate: &tokio::sync::Semaphore,
    ) -> AppResult<SyncOutcome> {
        let tip = self.state.read().await.tip();
        let Some((tip_h, tip_hash)) = tip else {
            self.build(rpc, gate).await?;
            return Ok(SyncOutcome::Moved);
        };
        let hdr = match call(
            rpc,
            gate,
            "getblockheader",
            json!([id_hex(&tip_hash), true]),
        )
        .await
        {
            Ok(h) => h,
            // The node does not know our tip at all (a -reindex, or another
            // datadir behind the same port): as far off the active chain as
            // a block can be. Disconnecting heals it; erroring forever would
            // leave every index route at 503 until a restart.
            Err(AppError::Rpc { code: -5, .. }) => json!({"confirmations": -1}),
            Err(e) => return Err(e),
        };
        let confirmations = hdr
            .get("confirmations")
            .and_then(Value::as_i64)
            .unwrap_or(-1);
        if confirmations < 1 {
            // Our tip is no longer on the active chain: take it off.
            let mut s = self.state.write().await;
            if s.tip() == Some((tip_h, tip_hash)) {
                s.disconnect_tip();
            }
            if s.is_empty() {
                eprintln!("[witness] swap index: a reorg reached below the window; rebuilding");
                s.clear();
                return Ok(SyncOutcome::Reset);
            }
            return Ok(SyncOutcome::Moved);
        }
        let mut next = match opt_id(&hdr, "nextblockhash").map_err(AppError::Decode)? {
            None => {
                // At the tip. A first build that failed part-way (the node
                // stopped answering) resumes here, through the incremental
                // path, and becomes ready only now that it has caught up.
                let mut s = self.state.write().await;
                if !s.ready && s.tip() == Some((tip_h, tip_hash)) {
                    s.ready = true;
                    let st = s.stats();
                    eprintln!(
                        "[witness] swap index ready: {} blocks ({}..{}), {} txs, {} spent outpoints, ~{:.1} MiB",
                        st.blocks,
                        st.bottom,
                        st.tip,
                        st.txs,
                        st.spends,
                        st.approx_bytes as f64 / 1048576.0
                    );
                }
                return Ok(SyncOutcome::Current);
            }
            Some(n) => n,
        };
        loop {
            let b = self.fetch(rpc, gate, &next).await?;
            {
                let mut s = self.state.write().await;
                if s.connect(&b).is_err() {
                    // Raced with a reorg; the next step re-checks the tip.
                    return Ok(SyncOutcome::Moved);
                }
                s.trim(self.window);
            }
            match b.next {
                Some(n) => next = n,
                None => return Ok(SyncOutcome::Moved),
            }
        }
    }

    /// Keep the index on the node's tip until the task is aborted.
    pub async fn run(&self, rpc: &dyn Rpc, gate: &tokio::sync::Semaphore) {
        loop {
            match self.sync_once(rpc, gate).await {
                Ok(SyncOutcome::Moved) | Ok(SyncOutcome::Reset) => continue,
                Ok(SyncOutcome::Current) => {}
                Err(e) => {
                    eprintln!("[witness] swap index sync failed: {e}");
                    // A node that is down should not be asked again at once.
                    tokio::time::sleep(POLL * 2).await;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(POLL) => {}
                _ = self.wake.notified() => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> Id {
        let mut a = [0u8; 32];
        a[31] = n;
        a[0] = 0xaa;
        a
    }

    fn block(height: u32, hash: Id, prev: Option<Id>, txs: Vec<ParsedTx>) -> ParsedBlock {
        let txids: Vec<Id> = txs.iter().map(|t| t.txid).collect();
        let (root, _) = merkle_root_and_branch(&txids, 0);
        ParsedBlock {
            hash,
            height,
            time: 1_000 + height,
            prev,
            next: None,
            merkle_root: root,
            txs,
        }
    }

    fn coinbase(txid: Id) -> ParsedTx {
        ParsedTx {
            txid,
            spends: vec![],
        }
    }

    #[test]
    fn ids_have_exactly_one_spelling() {
        let s = "2c4797cf2748908b1afba52a49e279289a05261f9aedae411f5db038836206fc";
        let i = parse_id(s).unwrap();
        assert_eq!(id_hex(&i), s);
        for bad in [
            "",
            &s[..63],
            &format!("{s}0"),
            &s.to_uppercase(),
            "2C4797cf2748908b1afba52a49e279289a05261f9aedae411f5db038836206fc",
            "zz4797cf2748908b1afba52a49e279289a05261f9aedae411f5db038836206fc",
            " c4797cf2748908b1afba52a49e279289a05261f9aedae411f5db038836206fc",
        ] {
            assert_eq!(parse_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_single_transaction_is_its_own_root_with_an_empty_branch() {
        let (root, branch) = merkle_root_and_branch(&[id(1)], 0);
        assert_eq!(root, id(1));
        assert!(branch.is_empty());
    }

    /// Every branch, for every position, of every block size up to 9, folds
    /// back to the root. This is the property a client checks.
    #[test]
    fn every_branch_folds_back_to_the_root() {
        for n in 1..=9u8 {
            let txids: Vec<Id> = (0..n).map(id).collect();
            let (root, _) = merkle_root_and_branch(&txids, 0);
            for pos in 0..n as usize {
                let (r2, branch) = merkle_root_and_branch(&txids, pos);
                assert_eq!(r2, root);
                let mut acc = reversed(&txids[pos]);
                let mut p = pos;
                for sib in &branch {
                    let sib = reversed(sib);
                    let mut buf = [0u8; 64];
                    if p & 1 == 0 {
                        buf[..32].copy_from_slice(&acc);
                        buf[32..].copy_from_slice(&sib);
                    } else {
                        buf[..32].copy_from_slice(&sib);
                        buf[32..].copy_from_slice(&acc);
                    }
                    acc = sha256d(&buf);
                    p >>= 1;
                }
                assert_eq!(reversed(&acc), root, "n={n} pos={pos}");
            }
        }
    }

    #[test]
    fn connect_disconnect_and_trim_leave_nothing_behind() {
        let mut s = IndexState::default();
        let b1 = block(10, id(101), Some(id(100)), vec![coinbase(id(1))]);
        let spend = ParsedTx {
            txid: id(3),
            spends: vec![(0, id(1), 0), (1, id(9), 4)],
        };
        let b2 = block(11, id(102), Some(id(101)), vec![coinbase(id(2)), spend]);
        s.connect(&b1).unwrap();
        s.connect(&b2).unwrap();
        assert_eq!(s.tip(), Some((11, id(102))));
        assert_eq!(s.tx(&id(3)).unwrap().height, 11);
        let sp = s.spend(&id(1), 0).unwrap();
        assert_eq!((sp.txid, sp.vin, sp.block.height), (id(3), 0, 11));
        assert_eq!(s.spend(&id(9), 4).unwrap().vin, 1);
        // A block that does not extend the tip is refused, whatever it holds.
        let wrong_parent = block(12, id(103), Some(id(77)), vec![coinbase(id(5))]);
        assert_eq!(s.connect(&wrong_parent), Err(ConnectError::NotOnTip));
        let wrong_height = block(13, id(103), Some(id(102)), vec![coinbase(id(5))]);
        assert_eq!(s.connect(&wrong_height), Err(ConnectError::NotOnTip));
        // Disconnecting the tip removes its txs AND its spends.
        assert_eq!(s.disconnect_tip(), Some(id(102)));
        assert_eq!(s.tx(&id(3)), None);
        assert_eq!(s.tx(&id(2)), None);
        assert_eq!(s.spend(&id(1), 0), None);
        assert_eq!(s.spend(&id(9), 4), None);
        assert!(!s.has_block(&id(102)));
        assert_eq!(s.tx(&id(1)).unwrap().height, 10);
        // Trim from below.
        s.connect(&b2).unwrap();
        s.trim(1);
        assert_eq!(s.bottom(), Some(11));
        assert_eq!(s.tx(&id(1)), None);
        assert!(
            s.spend(&id(1), 0).is_some(),
            "the spend is in the kept block"
        );
        let st = s.stats();
        assert_eq!((st.blocks, st.txs, st.spends), (1, 2, 2));
    }

    #[test]
    fn a_block_whose_txids_miss_its_root_serves_no_proof() {
        let mut s = IndexState::default();
        let mut b = block(5, id(50), None, vec![coinbase(id(1)), coinbase(id(2))]);
        b.merkle_root = id(99);
        s.connect(&b).unwrap();
        assert_eq!(s.proof(&id(2)), Some(Err(())));
        // The tx is still known to be there; only the proof is withheld.
        assert!(s.tx(&id(2)).is_some());
    }

    #[test]
    fn parse_block_refuses_an_input_it_cannot_read() {
        let txid = "aeaa3afef5d168e266ef13162d278cc4478bcc20742ac923af712d4378d17071";
        let good = json!({
            "hash": id_hex(&id(1)), "height": 7, "time": 9,
            "merkleroot": id_hex(&id(2)), "previousblockhash": id_hex(&id(3)),
            "tx": [
                {"txid": id_hex(&id(4)), "vin": [{"coinbase": "00", "sequence": 1}]},
                {"txid": txid, "vin": [{"txid": id_hex(&id(5)), "vout": 8}]}
            ]
        });
        let b = parse_block(&good).unwrap();
        assert_eq!(b.txs[1].spends, vec![(0, id(5), 8)]);
        assert_eq!(b.next, None);
        for (path, bad) in [
            ("/tx/1/vin/0/vout", json!(null)),
            ("/tx/1/vin/0/vout", json!(-1)),
            ("/tx/1/vin/0/vout", json!(4294967296u64)),
            ("/tx/1/vin/0/txid", json!("XYZ")),
            ("/tx/1/vin/0", json!({"shielded": "?"})),
            ("/tx/1/txid", json!(txid.to_uppercase())),
            ("/tx/0", json!("a txid alone, as verbosity 1 gives")),
            ("/hash", json!(null)),
            ("/merkleroot", json!(7)),
        ] {
            let mut v = good.clone();
            *v.pointer_mut(path).unwrap() = bad.clone();
            assert!(
                parse_block(&v).is_err(),
                "{path} = {bad} must refuse the block"
            );
        }
        let mut both = good.clone();
        both["tx"][0]["vin"][0]["txid"] = json!(id_hex(&id(6)));
        assert!(parse_block(&both).is_err());
    }
}
