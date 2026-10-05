//! Serve an attested UTXO snapshot to the network: the PRODUCER half.
//!
//! ── WHY ─────────────────────────────────────────────────────────────────────
//! Every new node bootstraps from a snapshot compiled into the engine, and the
//! newest published one lags the chain by thousands of blocks. BTX has had a
//! complete attested-snapshot mechanism since 0.34 (`dumptxoutsetattested`,
//! `offerattestedutxosnapshot`, `fetchattestedutxosnapshot`,
//! `loadtxoutsetattested`) and, measured on 2026-09-20 across 62 reachable
//! peers, nobody used it: zero advertised `NODE_ATTESTED_UTXO_SNAPSHOT`. On
//! 2026-09-21 one home RTX 3060 produced, served and round-tripped the first
//! one, and everything it took was four shell scripts and a person watching
//! them. This module is those scripts, as a role the app runs on its own.
//!
//! [`crate::snapshot`] is the other half, the CONSUMER: it downloads the
//! compiled pin and calls `loadtxoutset`. Nothing here changes what a node
//! loads. Loading an attested snapshot means running as a trusted mirror and
//! taking the signers' word for the UTXO set, which is a trust decision this
//! module does not make for anyone.
//!
//! ── WHAT WAS MEASURED, AND WHAT IT DECIDES ──────────────────────────────────
//! * The export is effectively free. `dumptxoutsetattested` wrote 140,936
//!   coins in 0.15 s and held `cs_main` for 7.4 ms at most (5.8 to 15.9 ms
//!   across five runs). It is safe on a live signer; the pause is invisible.
//! * **The dump always bases on the 0-conf tip.** The RPC takes only two
//!   paths, no height and no rollback, and BTX mints a competing sibling
//!   about every 25 blocks. The first snapshot ever produced was orphaned
//!   within 40 seconds and had been offered. So nothing is offered from the
//!   dump: it WAITS, then the base is re-verified on the active chain, then
//!   the producer's checks have their word, THEN it is offered. The old
//!   offer stays live throughout, so serving never stops.
//!
//! ── ON THE GRID, 144 DEEP (docs/decisions/2026-09-29-every-node-starts-
//! near-the-tip.md, sections 2 and 4) ─────────────────────────────────────────
//! * **Exports land only on the grid** ([`EXPORT_GRID`], every multiple of
//!   100), where every validating node's diary has the same height to compare
//!   with, and only above the last export ([`export_due`]). The dump takes the
//!   tip, so a tip that moved off the grid before the dump throws the export
//!   away, and so does a base orphaned while it waits: a second dump would
//!   base on today's tip, off the grid. The next chance is the next grid
//!   height. (Before the design: every 500 blocks from the last base, and an
//!   orphaned base was dumped again at once.)
//! * **Nothing leaves the node before the base is 144 deep**
//!   ([`CONFIRMATIONS_REQUIRED`], the design's depth for sending and signing;
//!   it was 10). Not the P2P offer, not the upload: the design supersedes
//!   the 10-confirmation wait outright and counts "two waiting" pairs on disk
//!   beside the offered one, and a producer "sends nothing" until then.
//! * **Each export waits on its own** ([`WaitingPair`], [`WAITING_FILE`]).
//!   144 is more than 100, so the next grid height comes while the last
//!   export still waits; a cycle that blocked until its base matured would
//!   miss it. So exporting ([`export_on_grid`]) and maturing ([`mature`]) are
//!   separate steps of the keeper's tick, and the waiting list is on disk, so
//!   a restart forgets nothing. Up to six hours each ([`MATURE_DEADLINE`]),
//!   four pairs on disk ([`KEEP_PAIRS`]).
//! * **The producer's checks have the last word** ([`BeforeOffer`],
//!   `crate::snapshot_producer::ProducerChecks`): the held blocks refused on
//!   the node, the diary entry at that height field by field, every
//!   chainstate validated. A pair that fails is neither offered nor sent,
//!   and the same checks run before a re-offer after a restart.
//! * **Service bits are sent once, in the VERSION handshake.**
//!   `PushNodeVersion` sends `peer.m_our_services`, snapshotted when the
//!   socket is made, and nothing re-announces a change. A peer connected
//!   BEFORE the offer never learns about bit 32. Measured: the mirror link was
//!   made 14:30Z, the offer re-asserted 14:34Z, the mirror still saw no offer
//!   at 16:46Z. So after every offer the links to the mirrors that would fetch
//!   are re-made ([`bounce_mirror_links`]), the same hosts the signer role
//!   already dials.
//! * **The offer lives in the running process only.** A btxd restart drops it
//!   and the bit with it, silently, and did so four times in one day. So the
//!   keeper re-offers the recorded pair on every node start, after checking
//!   the base is still canonical ([`reoffer`]).
//! * **Never dump while behind the tip.** A dump taken 500 blocks behind
//!   produces a worse base than the matured one already on disk. The gate is
//!   out of IBD and headers within [`TIP_GATE_HEADERS_AHEAD`] of blocks.
//! * The manifest is BINARY despite the `.json` name the engine's own
//!   examples use (335 bytes, starts `02 01 46 fa`). It is never parsed here.
//!   `offerattestedutxosnapshot` returns a `file_hash` that is the
//!   byte-reversed double SHA-256, NOT the plain sha256; both are recorded.
//!
//! Facts in, verdict out, the contract [`crate::esplora`] uses, and the pure
//! decisions are separate from the RPC driver so each has a test.

use crate::error::{AppError, AppResult};
use crate::rpc::Rpc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `NODE_ATTESTED_UTXO_SNAPSHOT`, service bit 32 (`1ULL << 32` in upstream
/// `src/protocol.h` at 3013c2c). On the wire while an offer is active.
pub const NODE_ATTESTED_UTXO_SNAPSHOT_BIT: u64 = 1 << 32;

/// Confirmations a base needs before it is offered on P2P or sent to the
/// website: the design's 144 (section 4), counted as `getblockheader`'s
/// `confirmations` counts them (tip minus height plus one), the same depth
/// [`crate::statement_check`] demands of a statement. 144 keeps the blocks
/// after a snapshot inside the 288 that limited peers serve, and is twice
/// the depth at which the engine's archive profile alarms about a reorg. It
/// was 10 (upstream's park depth of 6 with margin) before the design.
pub const CONFIRMATIONS_REQUIRED: u64 = crate::statement_check::DEPTH;

/// Snapshots are exported where the tip is a multiple of this (section 2):
/// the statements' own grid, on every chain, and the diary's. It divides the
/// engine's compiled heights (219,000, 228,000).
pub const EXPORT_GRID: u64 = crate::confirmed_snapshot::SNAPSHOT_GRID as u64;

/// Headers ahead of blocks at which the node counts as "at the tip" for the
/// purpose of dumping or re-offering. Wider than
/// [`crate::role::HEADERS_AHEAD_IS_BEHIND`] on purpose: this gate decides
/// whether to ACT, and one block in flight is not a reason to skip a cycle.
pub const TIP_GATE_HEADERS_AHEAD: u64 = 5;

/// Snapshot pairs kept on disk: the offered one, the one before it (so a
/// failed swap always has something the keeper can fall back to) and two
/// waiting, since 144 is more than 100 (section 4). About 36 MB at today's
/// 9 MB file. The offered pair and every waiting one are never pruned.
pub const KEEP_PAIRS: usize = 4;

/// How long a base may take to reach [`CONFIRMATIONS_REQUIRED`] before it is
/// dropped. 144 blocks take about three and a half hours at 40 an hour; a
/// base still short of them after six is on a chain that is not moving, and
/// there is nothing to serve. It was 90 minutes for 10 confirmations.
pub const MATURE_DEADLINE: Duration = Duration::from_secs(6 * 3600);

/// Chunk the engine serves the file in. Its default and the one every
/// measured fetch used (9 chunks for a 9 MB file, 19 s over loopback).
pub const CHUNK_SIZE: u64 = 1 << 20;

/// Free disk below which the role refuses to start. A pair is ~9 MB and four
/// are kept, so this is headroom rather than a budget: a datadir this full
/// has bigger problems than a snapshot, and adding to it helps nobody.
pub const MIN_FREE_DISK_MB: u64 = 200;

/// Folder under the datadir holding the pairs and the offer record.
pub const SNAPSHOT_DIR: &str = "snapshots";
/// The record of what is offered, next to the pairs. JSON, ours, and the
/// only thing the keeper needs to re-offer after a restart.
pub const OFFER_RECORD: &str = "current-offer.json";
/// The exports waiting to be 144 deep, next to the pairs. JSON, ours.
pub const WAITING_FILE: &str = "waiting.json";
const STAGING_DAT: &str = "staging.dat";
const STAGING_MANIFEST: &str = "staging.manifest";

/// The mirrors a signer already dials (`btx_core::signer`). These are the
/// peers that fetch attested snapshots, and the links that have to be re-made
/// after an offer so their handshake carries bit 32.
pub const MIRROR_HOSTS: &[&str] = crate::signer::BTX_MIRROR_WHITELIST_IPS;

pub fn snapshot_dir(datadir: &Path) -> PathBuf {
    datadir.join(SNAPSHOT_DIR)
}

/// The names upstream's own releases use, so a pair copied out of this folder
/// is recognisable to anyone who has seen the compiled-in assets.
pub fn snapshot_file_name(height: u64) -> String {
    format!("utxo-btx-main-{height}.dat")
}

pub fn manifest_file_name(height: u64) -> String {
    format!("snapshot-manifest-{height}.json")
}

/// The height a snapshot file name carries, or `None` for anything else in
/// the folder (the record, the staging files, a stray).
pub fn height_from_file_name(name: &str) -> Option<u64> {
    name.strip_prefix("utxo-btx-main-")?
        .strip_suffix(".dat")?
        .parse()
        .ok()
}

// ── The gate ────────────────────────────────────────────────────────────────

/// What the app knows when it decides whether this node can produce a
/// snapshot. `None` means "could not measure", never a default.
#[derive(Debug, Clone, Default)]
pub struct SnapshotFacts {
    /// `getmatmultrustedstatus.matmul_validation_mode`.
    pub validation_mode: Option<String>,
    /// `getmatmultrustedstatus.local_signer`. Only a signer can dump: the
    /// export carries this node's signature, and an unsigned manifest is
    /// nothing an importer can verify.
    pub local_signer: Option<bool>,
    pub blocks: Option<u64>,
    pub headers: Option<u64>,
    pub initial_block_download: Option<bool>,
    /// Whether `help dumptxoutsetattested` names a real command. The quartet
    /// exists from v0.34.6; an older engine answers "unknown command".
    pub engine_knows_rpc: Option<bool>,
    pub free_disk_mb: Option<u64>,
    /// `getchainstates` shows every chainstate at `"validated": true`
    /// ([`crate::node_api::chainstates_validated`]). `Some(false)` on a node
    /// that loaded a snapshot (upstream's plain one or a signed one) and has
    /// not finished checking the history below it.
    pub chainstates_validated: Option<bool>,
}

/// Why the role cannot act right now. Each carries what it measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum SnapshotBlocker {
    /// Nothing could be measured, usually because the node is not running.
    Unmeasured,
    /// The engine does not know the attested-snapshot RPCs.
    EngineTooOld,
    /// The node follows signed attestations instead of validating. A mirror
    /// holds a UTXO set it never verified; there is nothing to attest.
    NotValidating {
        mode: String,
    },
    /// No signing key, so the manifest would carry no signature.
    NoSigningKey,
    /// Still in initial block download.
    InitialBlockDownload,
    /// Headers run ahead of blocks by more than [`TIP_GATE_HEADERS_AHEAD`].
    BehindTip {
        blocks_behind: u64,
    },
    DiskLow {
        free_mb: u64,
    },
    /// `getchainstates` shows a snapshot chainstate still at `"validated":
    /// false`: the node's UTXO set rests on a snapshot it has not finished
    /// checking, so exporting it would let one snapshot vouch for the next
    /// (sections 3, 4 and 8 of the design).
    UnvalidatedChainstate,
}

impl SnapshotBlocker {
    /// A transient blocker clears on its own (the node catches up, the
    /// background check finishes); the keeper waits. A permanent one needs an
    /// operator (a setting, an engine), so the row asks for attention instead
    /// of quietly polling forever.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            SnapshotBlocker::Unmeasured
                | SnapshotBlocker::InitialBlockDownload
                | SnapshotBlocker::BehindTip { .. }
                | SnapshotBlocker::UnvalidatedChainstate
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotVerdict {
    pub blocker: Option<SnapshotBlocker>,
}

impl SnapshotVerdict {
    pub fn is_allowed(&self) -> bool {
        self.blocker.is_none()
    }
}

/// The tip gate on its own: the same rule the keeper applies before dumping
/// and before re-offering. `None` means the node may act.
pub fn tip_gate(
    blocks: Option<u64>,
    headers: Option<u64>,
    initial_block_download: Option<bool>,
) -> Option<SnapshotBlocker> {
    if initial_block_download == Some(true) {
        return Some(SnapshotBlocker::InitialBlockDownload);
    }
    match (blocks, headers) {
        (Some(b), Some(h)) if h.saturating_sub(b) > TIP_GATE_HEADERS_AHEAD => {
            Some(SnapshotBlocker::BehindTip {
                blocks_behind: h - b,
            })
        }
        (Some(_), Some(_)) => None,
        _ => Some(SnapshotBlocker::Unmeasured),
    }
}

/// Decide whether this node may produce and serve a snapshot right now.
///
/// Permanent blockers come first, so an operator whose node is both behind
/// and a mirror is told the thing that will still be true in an hour.
pub fn check(f: &SnapshotFacts) -> SnapshotVerdict {
    let blocker = if f.engine_knows_rpc == Some(false) {
        Some(SnapshotBlocker::EngineTooOld)
    } else if let Some(mode) = f
        .validation_mode
        .as_deref()
        .filter(|m| !m.trim().eq_ignore_ascii_case("consensus"))
    {
        Some(SnapshotBlocker::NotValidating {
            mode: mode.trim().to_string(),
        })
    } else if f.local_signer == Some(false) {
        Some(SnapshotBlocker::NoSigningKey)
    } else if let Some(mb) = f.free_disk_mb.filter(|mb| *mb < MIN_FREE_DISK_MB) {
        Some(SnapshotBlocker::DiskLow { free_mb: mb })
    } else if f.validation_mode.is_none()
        || f.local_signer.is_none()
        || f.chainstates_validated.is_none()
    {
        Some(SnapshotBlocker::Unmeasured)
    } else if f.chainstates_validated == Some(false) {
        Some(SnapshotBlocker::UnvalidatedChainstate)
    } else {
        tip_gate(f.blocks, f.headers, f.initial_block_download)
    };
    SnapshotVerdict { blocker }
}

/// The operator-facing sentence: what is true, why it blocks, what changes it.
pub fn explain(b: &SnapshotBlocker) -> String {
    match b {
        SnapshotBlocker::Unmeasured => {
            "Waiting for the node to answer. Nothing is measured yet, so nothing is \
             refused yet either."
                .to_string()
        }
        SnapshotBlocker::EngineTooOld => {
            "This node engine does not know the attested-snapshot commands. They exist \
             from BTX 0.34.6; the next engine update brings them."
                .to_string()
        }
        SnapshotBlocker::NotValidating { mode } => format!(
            "This node follows signed attestations ({mode} mode) instead of checking \
             blocks itself, so it holds a chain state it never verified and there is \
             nothing it could honestly attest. Only a node that validates can produce \
             a snapshot."
        ),
        SnapshotBlocker::NoSigningKey => {
            "This node has no signing key, so the snapshot's manifest would carry no \
             signature and no importer could verify it. Turn on \"Sign confirmations \
             for mirrors\" and restart the node first."
                .to_string()
        }
        SnapshotBlocker::InitialBlockDownload => {
            "Still downloading the chain. A snapshot is only worth taking at the tip; \
             the role starts on its own once the node is caught up."
                .to_string()
        }
        SnapshotBlocker::BehindTip { blocks_behind } => format!(
            "{blocks_behind} blocks behind the best header. A snapshot taken now would \
             be older than the one already on disk; waiting for the tip."
        ),
        SnapshotBlocker::DiskLow { free_mb } => format!(
            "Only {free_mb} MB free on the data folder's disk. Each snapshot is about \
             9 MB and up to four are kept, but a disk this full needs space before it \
             needs another file."
        ),
        SnapshotBlocker::UnvalidatedChainstate => {
            "This node started from a snapshot and is still checking the older history \
             in the background. Until that check is done its chain state rests partly on \
             that snapshot, so it does not export one of its own. It starts on its own \
             once the check finishes."
                .to_string()
        }
    }
}

// ── Pure decisions ──────────────────────────────────────────────────────────

/// Is it time to export? Only when the tip is exactly on the grid and above
/// `last`, the newest base already exported, offered or still waiting
/// ([`last_exported`]; `None` when there is none). A tip that moved past a
/// grid height before this was asked waits for the next one; the genesis
/// block is never a base.
pub fn export_due(tip: u64, grid: u64, last: Option<u64>) -> bool {
    tip > 0 && grid > 0 && tip.is_multiple_of(grid) && last.is_none_or(|b| tip > b)
}

/// Blocks until the tip reaches the next grid height, 1 to `grid`. `grid`
/// is never 0 in this app ([`EXPORT_GRID`]); 0 reads as "no grid", 0 blocks.
pub fn blocks_to_grid(tip: u64, grid: u64) -> u64 {
    if grid == 0 {
        return 0;
    }
    grid - tip % grid
}

/// The newest base exported from this folder: the offered one or a waiting
/// one, whichever is higher. What [`export_due`] compares the tip with, so
/// a height that already waits is not exported twice.
pub fn last_exported(record: Option<&OfferRecord>, waiting: &[WaitingPair]) -> Option<u64> {
    record
        .map(|r| r.height)
        .into_iter()
        .chain(waiting.iter().map(|w| w.height))
        .max()
}

/// Where one waiting pair stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Maturity {
    /// Still short of [`CONFIRMATIONS_REQUIRED`], in time.
    Waiting { confirmations: u64 },
    /// Deep enough: re-verify, check, offer.
    Ready,
    /// Its base left the active chain (`confirmations` -1). Dropped; the next
    /// chance is the next grid height.
    Orphaned,
    /// Short of the depth after `deadline`. Dropped.
    Expired,
}

/// One waiting pair, from its base's `confirmations` and how long it has
/// waited. Off the chain says more than late, and deep enough is ready
/// however long it took (a keeper that was off for a while). A clock that
/// went backwards is not late.
pub fn maturity(confirmations: i64, exported_at: u64, now: u64, deadline: Duration) -> Maturity {
    if confirmations < 0 {
        Maturity::Orphaned
    } else if confirmations as u64 >= CONFIRMATIONS_REQUIRED {
        Maturity::Ready
    } else if now.saturating_sub(exported_at) > deadline.as_secs() {
        Maturity::Expired
    } else {
        Maturity::Waiting {
            confirmations: confirmations as u64,
        }
    }
}

/// Which heights to delete so that at most `keep` pairs remain, never one in
/// `protect` (the offered pair and every waiting one). Oldest first.
pub fn pairs_to_prune(heights: &[u64], keep: usize, protect: &[u64]) -> Vec<u64> {
    let mut sorted: Vec<u64> = heights.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let excess = sorted.len().saturating_sub(keep);
    sorted
        .into_iter()
        .take(excess)
        .filter(|h| !protect.contains(h))
        .collect()
}

/// Does `getnetworkinfo` say an offer is live? Decided from the bits, with
/// the exact name as the fallback for an engine that omitted `localservices`,
/// the rule [`crate::role`] uses and for the same reason.
pub fn advertises_offer(localservices_hex: &str, names: &[String]) -> bool {
    let t = localservices_hex.trim().trim_start_matches("0x");
    match u64::from_str_radix(t, 16) {
        Ok(bits) if !t.is_empty() => bits & NODE_ATTESTED_UTXO_SNAPSHOT_BIT != 0,
        _ => names.iter().any(|n| n == "ATTESTED_UTXO_SNAPSHOT"),
    }
}

// ── The record ──────────────────────────────────────────────────────────────

/// What is offered, written only AFTER an offer succeeded. A failed swap
/// leaves the previous record in place, so the keeper falls back to the
/// previous pair instead of to nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferRecord {
    pub height: u64,
    pub block_hash: String,
    pub txoutset_hash: String,
    pub file_size: u64,
    /// Plain SHA-256 of the snapshot file, what a person checks a download
    /// against.
    pub sha256: String,
    pub manifest_sha256: String,
    /// The engine's own commitment to the file bytes: byte-reversed double
    /// SHA-256. Not the sha256 above, and both are kept because both get
    /// asked for.
    pub file_hash: String,
    pub chunk_count: u64,
    pub signatures: u64,
    /// Unix seconds when the offer went live.
    pub offered_at: u64,
}

pub fn load_record(dir: &Path) -> Option<OfferRecord> {
    let raw = std::fs::read_to_string(dir.join(OFFER_RECORD)).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn save_record(dir: &Path, r: &OfferRecord) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(r)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    crate::fsx::atomic_write(&dir.join(OFFER_RECORD), json.as_bytes())
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Heights of the pairs on disk (a `.dat` is enough to count; a manifest
/// without its file is nothing to serve).
pub fn pairs_on_disk(dir: &Path) -> Vec<u64> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| height_from_file_name(&e.file_name().to_string_lossy()))
        .collect()
}

/// Delete pairs beyond [`KEEP_PAIRS`], never one in `protect`. Returns what
/// went.
pub fn prune(dir: &Path, protect: &[u64]) -> Vec<u64> {
    let gone = pairs_to_prune(&pairs_on_disk(dir), KEEP_PAIRS, protect);
    for h in &gone {
        remove_pair(dir, *h);
    }
    gone
}

fn remove_pair(dir: &Path, height: u64) {
    let _ = std::fs::remove_file(dir.join(snapshot_file_name(height)));
    let _ = std::fs::remove_file(dir.join(manifest_file_name(height)));
}

// ── The waiting list ────────────────────────────────────────────────────────

/// An export waiting to be [`CONFIRMATIONS_REQUIRED`] deep. Its files are
/// already under its height's names; nothing about it has left the node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingPair {
    pub height: u64,
    /// The base as the dump named it, display hex.
    pub block_hash: String,
    pub txoutset_hash: String,
    /// Unix seconds, for [`MATURE_DEADLINE`].
    pub exported_at: u64,
}

/// The waiting list, ascending by height. A missing or unreadable file is
/// an empty list: the pairs it named are pruned like any other, and the
/// next grid height starts again.
pub fn load_waiting(dir: &Path) -> Vec<WaitingPair> {
    let mut w: Vec<WaitingPair> = std::fs::read(dir.join(WAITING_FILE))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default();
    w.sort_by_key(|p| p.height);
    w
}

/// Write the waiting list atomically.
pub fn save_waiting(dir: &Path, waiting: &[WaitingPair]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(waiting).map_err(std::io::Error::other)?;
    crate::fsx::atomic_write(&dir.join(WAITING_FILE), &json)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

async fn sha256_of_file(path: &Path) -> std::io::Result<(u64, String)> {
    let p = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(&p)?;
        Ok((bytes.len() as u64, sha256_hex(&bytes)))
    })
    .await
    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?
}

// ── Wire reads ──────────────────────────────────────────────────────────────

fn u64_field(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(|x| x.as_u64())
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

/// Read every fact the gate needs from a running node. Each read fails
/// independently and leaves its field `None`.
pub async fn read_facts(rpc: &dyn Rpc, free_disk_mb: Option<u64>) -> SnapshotFacts {
    let (status, chain, help, chainstates) = tokio::join!(
        rpc.call("getmatmultrustedstatus", json!([])),
        rpc.call("getblockchaininfo", json!([])),
        rpc.call("help", json!(["dumptxoutsetattested"])),
        rpc.call("getchainstates", json!([])),
    );
    // A failed call is "could not measure", not "unvalidated".
    let chainstates_validated = chainstates
        .ok()
        .map(|v| crate::node_api::chainstates_validated(Some(&v)));
    let (validation_mode, local_signer) = match status {
        Ok(v) => (
            str_field(&v, "matmul_validation_mode"),
            v.get("local_signer").and_then(|x| x.as_bool()),
        ),
        Err(_) => (None, None),
    };
    let (blocks, headers, initial_block_download) = match chain {
        Ok(v) => (
            u64_field(&v, "blocks"),
            u64_field(&v, "headers"),
            v.get("initialblockdownload").and_then(|x| x.as_bool()),
        ),
        Err(_) => (None, None, None),
    };
    // `help <unknown>` is not an error on this engine: it answers the string
    // "help: unknown command: ...". A method-not-found error means the same.
    let engine_knows_rpc = match help {
        Ok(Value::String(s)) => Some(!s.trim_start().starts_with("help: unknown command")),
        Ok(_) => None,
        Err(AppError::Rpc { code: -32601, .. }) => Some(false),
        Err(_) => None,
    };
    SnapshotFacts {
        validation_mode,
        local_signer,
        blocks,
        headers,
        initial_block_download,
        engine_knows_rpc,
        free_disk_mb,
        chainstates_validated,
    }
}

/// Is an offer live on THIS node, from `getnetworkinfo`? `None` when the node
/// did not answer.
pub async fn offer_live(rpc: &dyn Rpc) -> Option<bool> {
    let v = rpc.call("getnetworkinfo", json!([])).await.ok()?;
    let hex = str_field(&v, "localservices").unwrap_or_default();
    let names: Vec<String> = v
        .get("localservicesnames")
        .and_then(|n| n.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    Some(advertises_offer(&hex, &names))
}

/// How many connected peers advertise bit 32 themselves. Zero across the
/// whole network on 2026-09-20; the number a person watches to see the role
/// spread.
pub async fn peers_offering(rpc: &dyn Rpc) -> Option<u64> {
    let v = rpc.call("getpeerinfo", json!([])).await.ok()?;
    let peers = v.as_array()?;
    Some(
        peers
            .iter()
            .filter(|p| {
                let hex = str_field(p, "services").unwrap_or_default();
                advertises_offer(&hex, &[])
            })
            .count() as u64,
    )
}

// ── Actions ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpResult {
    pub base_height: u64,
    pub base_hash: String,
    pub txoutset_hash: String,
    pub coins_written: u64,
    pub max_cs_main_hold_us: u64,
}

fn decode_err(what: &str) -> AppError {
    AppError::Decode(format!("{what}: field missing from the engine's answer"))
}

/// `dumptxoutsetattested` into the staging pair. Bases on the 0-conf tip;
/// see the module header for why nothing is offered from here.
pub async fn dump(rpc: &dyn Rpc, dir: &Path) -> AppResult<DumpResult> {
    std::fs::create_dir_all(dir).map_err(|e| AppError::Disk(e.to_string()))?;
    let dat = dir.join(STAGING_DAT);
    let man = dir.join(STAGING_MANIFEST);
    let _ = std::fs::remove_file(&dat);
    let _ = std::fs::remove_file(&man);
    let v = rpc
        .call(
            "dumptxoutsetattested",
            json!([dat.to_string_lossy(), man.to_string_lossy()]),
        )
        .await?;
    Ok(DumpResult {
        base_height: u64_field(&v, "base_height").ok_or_else(|| decode_err("base_height"))?,
        base_hash: str_field(&v, "base_hash").ok_or_else(|| decode_err("base_hash"))?,
        txoutset_hash: str_field(&v, "txoutset_hash").unwrap_or_default(),
        coins_written: u64_field(&v, "coins_written").unwrap_or(0),
        max_cs_main_hold_us: u64_field(&v, "max_cs_main_hold_us").unwrap_or(0),
    })
}

/// Confirmations of a block by hash: -1 once it is off the active chain.
pub async fn confirmations(rpc: &dyn Rpc, hash: &str) -> AppResult<i64> {
    let v = rpc.call("getblockheader", json!([hash, true])).await?;
    v.get("confirmations")
        .and_then(|c| c.as_i64())
        .ok_or_else(|| decode_err("confirmations"))
}

/// Is `hash` what the active chain has at `height`?
pub async fn base_is_canonical(rpc: &dyn Rpc, height: u64, hash: &str) -> AppResult<bool> {
    let v = rpc.call("getblockhash", json!([height])).await?;
    Ok(v.as_str() == Some(hash))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferResult {
    pub height: u64,
    pub block_hash: String,
    pub file_size: u64,
    pub chunk_count: u64,
    pub file_hash: String,
    pub signatures: u64,
}

pub async fn offer(rpc: &dyn Rpc, dat: &Path, man: &Path) -> AppResult<OfferResult> {
    let v = rpc
        .call(
            "offerattestedutxosnapshot",
            json!([dat.to_string_lossy(), man.to_string_lossy(), CHUNK_SIZE]),
        )
        .await?;
    Ok(OfferResult {
        height: u64_field(&v, "height").ok_or_else(|| decode_err("height"))?,
        block_hash: str_field(&v, "block_hash").ok_or_else(|| decode_err("block_hash"))?,
        file_size: u64_field(&v, "file_size").unwrap_or(0),
        chunk_count: u64_field(&v, "chunk_count").unwrap_or(0),
        file_hash: str_field(&v, "file_hash").unwrap_or_default(),
        signatures: u64_field(&v, "signatures").unwrap_or(0),
    })
}

/// Stop serving. `Ok(false)` when there was nothing to withdraw.
pub async fn withdraw(rpc: &dyn Rpc) -> AppResult<bool> {
    let v = rpc.call("withdrawattestedutxosnapshot", json!([])).await?;
    Ok(v.get("withdrawn")
        .and_then(|w| w.as_bool())
        .unwrap_or(false))
}

/// Re-make the links to the mirror hosts so their handshake carries the
/// current service bits (module header, third bullet). Disconnects every
/// connected peer on a mirror host, then dials each host's known port once.
/// Returns the addresses it disconnected, for the log.
pub async fn bounce_mirror_links(rpc: &dyn Rpc) -> Vec<String> {
    let mut dropped = Vec::new();
    if let Ok(v) = rpc.call("getpeerinfo", json!([])).await {
        for p in v.as_array().into_iter().flatten() {
            let Some(addr) = str_field(p, "addr") else {
                continue;
            };
            let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(&addr);
            if MIRROR_HOSTS.contains(&host)
                && rpc.call("disconnectnode", json!([addr])).await.is_ok()
            {
                dropped.push(addr);
            }
        }
    }
    // `disconnectnode` marks the socket; the net thread closes it a moment
    // later, and a dial while the old socket lives is refused as a duplicate.
    if !dropped.is_empty() {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    for host in crate::signer::BTX_MIRRORS_FED_BY_SIGNERS {
        let _ = rpc.call("addnode", json!([host, "onetry"])).await;
    }
    for addr in &dropped {
        let _ = rpc.call("addnode", json!([addr, "onetry"])).await;
    }
    dropped
}

// ── The cycle ───────────────────────────────────────────────────────────────

/// Where a cycle is, for the row in Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CyclePhase {
    Dumping,
    Maturing {
        base: u64,
        confirmations: u64,
        tip: u64,
    },
    Offering {
        base: u64,
    },
    Live {
        base: u64,
    },
}

impl CyclePhase {
    /// The sentence beside the switch while a cycle runs.
    pub fn message(&self) -> String {
        match self {
            CyclePhase::Dumping => "Exporting the chain state at the tip.".to_string(),
            CyclePhase::Maturing {
                base,
                confirmations,
                tip,
            } => format!(
                "Snapshot at block {base} exported; waiting for it to mature, {confirmations} of \
                 {CONFIRMATIONS_REQUIRED} confirmations (tip {tip}). The previous one stays on offer."
            ),
            CyclePhase::Offering { base } => format!("Offering block {base} to the network."),
            CyclePhase::Live { base } => format!("Offering a snapshot of the chain at block {base}."),
        }
    }
}

/// Why a pair was not offered. `retry` says whether asking again can change
/// the answer: the node did not answer, or its background check is still
/// running. Otherwise the pair is dropped for good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotOffered {
    pub why: String,
    pub retry: bool,
}

/// The producer's word, around the keeper's own steps
/// (`crate::snapshot_producer::ProducerChecks` in the app, [`NoChecks`]
/// where there is none).
#[async_trait::async_trait]
pub trait BeforeOffer: Sync {
    /// Right before the dump, at the tip the dump will take. The producer
    /// writes its diary here, so the check 144 blocks later has this height
    /// to compare with even when the status refresher missed it.
    async fn before_export(&self, _rpc: &dyn Rpc) {}

    /// The last word before a matured pair is offered, after the keeper's
    /// own re-verify of the base. `manifest` is the export's own manifest.
    /// `Err` keeps the pair off the wire and the previous offer live.
    async fn check(
        &self,
        rpc: &dyn Rpc,
        base_height: u64,
        manifest: &[u8],
    ) -> Result<(), NotOffered>;
}

/// No check beyond the keeper's own: what it did before the diary.
pub struct NoChecks;

#[async_trait::async_trait]
impl BeforeOffer for NoChecks {
    async fn check(&self, _: &dyn Rpc, _: u64, _: &[u8]) -> Result<(), NotOffered> {
        Ok(())
    }
}

/// Export at the tip, which must be a grid height: the hook's
/// `before_export`, `dumptxoutsetattested` into the staging pair, then the
/// pair under its height's names and on the waiting list. Nothing is
/// offered. The caller has checked [`export_due`] and the gate ([`check`]).
///
/// The dump takes the tip whatever it is by then, so a base off the grid
/// (the tip moved in between) is thrown away, files and all: the next chance
/// is the next grid height.
pub async fn export_on_grid(
    rpc: &dyn Rpc,
    dir: &Path,
    grid: u64,
    before: &dyn BeforeOffer,
    on_phase: &(dyn Fn(CyclePhase) + Sync),
) -> Result<WaitingPair, String> {
    before.before_export(rpc).await;
    on_phase(CyclePhase::Dumping);
    let base = dump(rpc, dir)
        .await
        .map_err(|e| format!("export failed: {e}"))?;
    if grid == 0 || base.base_height == 0 || !base.base_height.is_multiple_of(grid) {
        let _ = std::fs::remove_file(dir.join(STAGING_DAT));
        let _ = std::fs::remove_file(dir.join(STAGING_MANIFEST));
        return Err(format!(
            "the tip moved to {} before the export; the next one is at the next multiple of {grid}",
            base.base_height
        ));
    }
    // Renaming is safe: the manifest embeds no path.
    std::fs::rename(
        dir.join(STAGING_DAT),
        dir.join(snapshot_file_name(base.base_height)),
    )
    .map_err(|e| format!("rename: {e}"))?;
    std::fs::rename(
        dir.join(STAGING_MANIFEST),
        dir.join(manifest_file_name(base.base_height)),
    )
    .map_err(|e| format!("rename: {e}"))?;
    let pair = WaitingPair {
        height: base.base_height,
        block_hash: base.base_hash.to_ascii_lowercase(),
        txoutset_hash: base.txoutset_hash,
        exported_at: now_unix(),
    };
    // A sibling at a height that waited before replaces that entry.
    let mut waiting = load_waiting(dir);
    waiting.retain(|w| w.height != pair.height);
    waiting.push(pair.clone());
    waiting.sort_by_key(|w| w.height);
    save_waiting(dir, &waiting).map_err(|e| format!("recording the export: {e}"))?;
    Ok(pair)
}

/// What [`mature`] did with one waiting pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatureEvent {
    /// Short of [`CONFIRMATIONS_REQUIRED`]; asked again next round.
    Waiting {
        base: u64,
        confirmations: u64,
        tip: u64,
    },
    /// Re-verified, checked and offered: what is served now.
    Offered(OfferRecord),
    /// Kept waiting and asked again next round: a read the node did not
    /// answer, a check that cannot decide yet, a failed offer, or a second
    /// pair ready in the same round.
    NotYet { base: u64, why: String },
    /// Gone with its files, never offered: off the chain, out of time, or
    /// refused by the checks.
    Dropped { base: u64, why: String },
}

/// One round over the waiting list, oldest first, each pair on its own: a
/// pair whose base left the chain or that ran out of time is dropped; one
/// that is [`CONFIRMATIONS_REQUIRED`] deep is re-verified on the active
/// chain, checked by `before`, and offered in place of the old offer
/// (withdraw, offer, record, bounce the mirror links, prune). At most one
/// pair is offered per round, so each one that comes of age is offered,
/// and sent by the producer, in turn. Never blocks on the chain: the keeper
/// calls it every tick.
pub async fn mature(
    rpc: &dyn Rpc,
    dir: &Path,
    deadline: Duration,
    before: &dyn BeforeOffer,
    on_phase: &(dyn Fn(CyclePhase) + Sync),
) -> Vec<MatureEvent> {
    let mut waiting = load_waiting(dir);
    let mut events = Vec::new();
    let mut keep = Vec::new();
    let mut offered = false;
    let now = now_unix();
    for w in std::mem::take(&mut waiting) {
        let base = w.height;
        let state = match confirmations(rpc, &w.block_hash).await {
            Ok(c) => maturity(c, w.exported_at, now, deadline),
            // One lost read is not a verdict, unless the time is up anyway.
            Err(e) => match maturity(0, w.exported_at, now, deadline) {
                Maturity::Expired => Maturity::Expired,
                _ => {
                    events.push(MatureEvent::NotYet {
                        base,
                        why: format!("could not read its confirmations: {e}"),
                    });
                    keep.push(w);
                    continue;
                }
            },
        };
        let drop = |why: String, events: &mut Vec<MatureEvent>| {
            remove_pair(dir, base);
            events.push(MatureEvent::Dropped { base, why });
        };
        match state {
            Maturity::Orphaned => drop(
                "its block left the chain while it waited; the next export is at the next \
                 grid height"
                    .into(),
                &mut events,
            ),
            Maturity::Expired => drop(
                format!(
                    "it did not reach {CONFIRMATIONS_REQUIRED} confirmations within {} hours",
                    deadline.as_secs() / 3600
                ),
                &mut events,
            ),
            Maturity::Waiting { confirmations } => {
                let tip = rpc
                    .call("getblockcount", json!([]))
                    .await
                    .ok()
                    .and_then(|v| v.as_u64())
                    .unwrap_or(base);
                on_phase(CyclePhase::Maturing {
                    base,
                    confirmations,
                    tip,
                });
                events.push(MatureEvent::Waiting {
                    base,
                    confirmations,
                    tip,
                });
                keep.push(w);
            }
            Maturity::Ready if offered => {
                events.push(MatureEvent::NotYet {
                    base,
                    why: "one pair is offered per round".into(),
                });
                keep.push(w);
            }
            Maturity::Ready => match offer_matured(rpc, dir, &w, before, on_phase).await {
                Ok(record) => {
                    offered = true;
                    events.push(MatureEvent::Offered(record));
                }
                Err(n) if n.retry => {
                    events.push(MatureEvent::NotYet { base, why: n.why });
                    keep.push(w);
                }
                Err(n) => drop(n.why, &mut events),
            },
        }
    }
    if let Err(e) = save_waiting(dir, &keep) {
        eprintln!("[snapshot] could not write the waiting list: {e}");
    }
    if offered {
        let mut protect: Vec<u64> = keep.iter().map(|w| w.height).collect();
        protect.extend(load_record(dir).map(|r| r.height));
        prune(dir, &protect);
    }
    events
}

/// One matured pair: re-verify, check, then swap the offer. `Err` with
/// `retry` keeps it waiting; without, it is dropped.
async fn offer_matured(
    rpc: &dyn Rpc,
    dir: &Path,
    w: &WaitingPair,
    before: &dyn BeforeOffer,
    on_phase: &(dyn Fn(CyclePhase) + Sync),
) -> Result<OfferRecord, NotOffered> {
    let retry = |why: String| NotOffered { why, retry: true };
    let base = w.height;
    // Final re-verify, immediately before anything is offered: confirmations
    // and canonicality are read separately, and the last word is the latter.
    match base_is_canonical(rpc, base, &w.block_hash).await {
        Ok(true) => {}
        Ok(false) => {
            return Err(NotOffered {
                why: format!("base {base} left the active chain just before the offer"),
                retry: false,
            })
        }
        Err(e) => return Err(retry(format!("could not re-verify the base: {e}"))),
    }
    let dat = dir.join(snapshot_file_name(base));
    let man = dir.join(manifest_file_name(base));
    let manifest = std::fs::read(&man).map_err(|e| NotOffered {
        why: format!("reading the export's manifest: {e}"),
        retry: false,
    })?;
    before
        .check(rpc, base, &manifest)
        .await
        .map_err(|n| NotOffered {
            why: format!("not offered: {}", n.why),
            retry: n.retry,
        })?;
    let (file_size, sha256) = sha256_of_file(&dat).await.map_err(|e| NotOffered {
        why: format!("hashing the snapshot: {e}"),
        retry: false,
    })?;
    let manifest_sha256 = sha256_hex(&manifest);

    on_phase(CyclePhase::Offering { base });
    // Offering on top of a live offer is untested; withdraw first. The gap is
    // one RPC round trip, and a failure here leaves the previous record for
    // the keeper to re-offer from, and this pair waiting for the next round.
    let _ = withdraw(rpc).await;
    let offered = offer(rpc, &dat, &man)
        .await
        .map_err(|e| retry(format!("offer failed: {e}")))?;
    let record = OfferRecord {
        height: offered.height,
        block_hash: offered.block_hash,
        txoutset_hash: w.txoutset_hash.clone(),
        file_size,
        sha256,
        manifest_sha256,
        file_hash: offered.file_hash,
        chunk_count: offered.chunk_count,
        signatures: offered.signatures,
        offered_at: now_unix(),
    };
    save_record(dir, &record).map_err(|e| retry(format!("recording the offer: {e}")))?;
    on_phase(CyclePhase::Live {
        base: record.height,
    });
    bounce_mirror_links(rpc).await;
    Ok(record)
}

/// What [`reoffer`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReofferOutcome {
    AlreadyLive,
    NoRecord,
    FilesMissing {
        height: u64,
    },
    Blocked(SnapshotBlocker),
    /// The recorded base is no longer on the active chain. Nothing is
    /// offered; the next export replaces it.
    BaseNotCanonical {
        height: u64,
    },
    /// The producer's checks refused the recorded pair (a held block now on
    /// the chain, a diary that disagrees, a background check still running,
    /// a pair off the grid from before the design). Nothing is offered;
    /// asked again on the next tick, and the next export replaces it.
    Refused {
        height: u64,
        why: String,
    },
    Reoffered {
        height: u64,
    },
}

/// Re-assert the recorded offer after a node start. Idempotent, never dumps,
/// refuses when the base is no longer canonical or the producer's checks
/// (`before`) refuse the pair: a re-offer sends it again, and a producer
/// never sends a pair that failed any check.
pub async fn reoffer(
    rpc: &dyn Rpc,
    dir: &Path,
    blocks: Option<u64>,
    headers: Option<u64>,
    initial_block_download: Option<bool>,
    before: &dyn BeforeOffer,
) -> Result<ReofferOutcome, String> {
    if offer_live(rpc).await == Some(true) {
        return Ok(ReofferOutcome::AlreadyLive);
    }
    let Some(record) = load_record(dir) else {
        return Ok(ReofferOutcome::NoRecord);
    };
    let dat = dir.join(snapshot_file_name(record.height));
    let man = dir.join(manifest_file_name(record.height));
    if !dat.is_file() || !man.is_file() {
        return Ok(ReofferOutcome::FilesMissing {
            height: record.height,
        });
    }
    if let Some(b) = tip_gate(blocks, headers, initial_block_download) {
        return Ok(ReofferOutcome::Blocked(b));
    }
    match base_is_canonical(rpc, record.height, &record.block_hash).await {
        Ok(true) => {}
        Ok(false) => {
            return Ok(ReofferOutcome::BaseNotCanonical {
                height: record.height,
            })
        }
        Err(e) => return Err(format!("could not verify the base: {e}")),
    }
    let manifest = std::fs::read(&man).map_err(|e| format!("reading the manifest: {e}"))?;
    if let Err(n) = before.check(rpc, record.height, &manifest).await {
        return Ok(ReofferOutcome::Refused {
            height: record.height,
            why: n.why,
        });
    }
    offer(rpc, &dat, &man)
        .await
        .map_err(|e| format!("offer failed: {e}"))?;
    bounce_mirror_links(rpc).await;
    Ok(ReofferOutcome::Reoffered {
        height: record.height,
    })
}

// ── What the row shows ──────────────────────────────────────────────────────

/// The role's state for the Settings row, kept by the keeper on every tick.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct ServeStatus {
    /// `Some(true)` while bit 32 is on the wire; `None` when unmeasured.
    pub offering: Option<bool>,
    pub base_height: Option<u64>,
    pub base_hash: Option<String>,
    /// Blocks the tip has moved past the offered base.
    pub stale_by: Option<u64>,
    pub file_size: Option<u64>,
    pub sha256: Option<String>,
    /// Peers that advertise bit 32 themselves.
    pub peers_offering: Option<u64>,
    /// Where a cycle is, while one runs.
    pub phase: Option<CyclePhase>,
    /// The sentence beside the switch.
    pub message: String,
    /// The message names something an operator has to change.
    pub needs_attention: bool,
}

impl ServeStatus {
    /// The one-line summary for a serving node.
    pub fn serving_message(
        record: &OfferRecord,
        tip: Option<u64>,
        peers_offering: Option<u64>,
    ) -> String {
        let stale = tip.map(|t| t.saturating_sub(record.height));
        let age = match stale {
            Some(0) => "at the tip".to_string(),
            Some(n) => format!("{n} blocks old"),
            None => "age unknown".to_string(),
        };
        let next = tip
            .map(|t| blocks_to_grid(t, EXPORT_GRID))
            .map(|n| {
                format!(
                    " The next one is taken in {n} blocks and offered once it is \
                     {CONFIRMATIONS_REQUIRED} blocks deep."
                )
            })
            .unwrap_or_default();
        let others = match peers_offering {
            Some(0) => " No other node offers one yet.".to_string(),
            Some(1) => " One other node offers one.".to_string(),
            Some(n) => format!(" {n} other nodes offer one."),
            None => String::new(),
        };
        format!(
            "Offering a snapshot of the chain at block {} ({age}, {:.1} MB, {} chunks).{next}{others}",
            record.height,
            record.file_size as f64 / 1_048_576.0,
            record.chunk_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    // The facts measured on the producing node on 2026-09-21 17:16Z.
    fn rig() -> SnapshotFacts {
        SnapshotFacts {
            validation_mode: Some("consensus".into()),
            local_signer: Some(true),
            blocks: Some(226_135),
            headers: Some(226_135),
            initial_block_download: Some(false),
            engine_knows_rpc: Some(true),
            free_disk_mb: Some(121 * 1024),
            chainstates_validated: Some(true),
        }
    }

    #[test]
    fn the_producing_node_is_allowed() {
        assert!(check(&rig()).is_allowed(), "{:?}", check(&rig()));
    }

    /// Section 3 and 4: a node whose `getchainstates` shows a snapshot
    /// chainstate at `"validated": false` (upstream's plain assumeutxo
    /// snapshot or a signed one) exports nothing, so no snapshot vouches for
    /// the next. It clears on its own when the background check finishes,
    /// so it waits rather than asking for attention. An unanswered
    /// `getchainstates` is not an open gate.
    #[test]
    fn an_unvalidated_chainstate_produces_nothing_until_the_check_finishes() {
        let f = SnapshotFacts {
            chainstates_validated: Some(false),
            ..rig()
        };
        let b = check(&f).blocker.unwrap();
        assert_eq!(b, SnapshotBlocker::UnvalidatedChainstate);
        assert!(b.is_transient());
        let msg = explain(&b);
        assert!(msg.contains("background"), "{msg}");
        assert!(!msg.contains('\u{2014}'), "{msg}");
        let f = SnapshotFacts {
            chainstates_validated: None,
            ..rig()
        };
        assert_eq!(check(&f).blocker, Some(SnapshotBlocker::Unmeasured));
        // A mirror is told it is a mirror first: the permanent reason.
        let f = SnapshotFacts {
            chainstates_validated: Some(false),
            validation_mode: Some("trusted".into()),
            ..rig()
        };
        assert!(matches!(
            check(&f).blocker,
            Some(SnapshotBlocker::NotValidating { .. })
        ));
    }

    #[test]
    fn a_mirror_is_refused_because_it_verified_nothing() {
        let f = SnapshotFacts {
            validation_mode: Some("trusted".into()),
            ..rig()
        };
        let v = check(&f);
        assert_eq!(
            v.blocker,
            Some(SnapshotBlocker::NotValidating {
                mode: "trusted".into()
            })
        );
        assert!(!v.blocker.as_ref().unwrap().is_transient());
        let msg = explain(v.blocker.as_ref().unwrap());
        assert!(msg.contains("trusted"), "name the mode: {msg}");
        assert!(msg.contains("never verified"), "say why: {msg}");
    }

    #[test]
    fn no_key_is_refused_and_told_which_switch() {
        let f = SnapshotFacts {
            local_signer: Some(false),
            ..rig()
        };
        let v = check(&f);
        assert_eq!(v.blocker, Some(SnapshotBlocker::NoSigningKey));
        assert!(explain(v.blocker.as_ref().unwrap()).contains("Sign confirmations"));
    }

    #[test]
    fn an_old_engine_is_refused_before_anything_else() {
        let f = SnapshotFacts {
            engine_knows_rpc: Some(false),
            validation_mode: Some("trusted".into()),
            ..rig()
        };
        assert_eq!(check(&f).blocker, Some(SnapshotBlocker::EngineTooOld));
    }

    #[test]
    fn behind_the_tip_is_transient_and_says_by_how_much() {
        // The crash-drill shape: headers 500 ahead, blocks catching up at
        // 250/h. A dump then would be worse than the pair on disk.
        let f = SnapshotFacts {
            blocks: Some(225_317),
            headers: Some(225_799),
            ..rig()
        };
        let v = check(&f);
        assert_eq!(
            v.blocker,
            Some(SnapshotBlocker::BehindTip { blocks_behind: 482 })
        );
        assert!(v.blocker.as_ref().unwrap().is_transient());
        assert!(explain(v.blocker.as_ref().unwrap()).contains("482"));
    }

    #[test]
    fn the_tip_gate_allows_five_headers_ahead_and_not_six() {
        assert_eq!(tip_gate(Some(100), Some(105), Some(false)), None);
        assert_eq!(
            tip_gate(Some(100), Some(106), Some(false)),
            Some(SnapshotBlocker::BehindTip { blocks_behind: 6 })
        );
        assert_eq!(
            tip_gate(Some(100), Some(100), Some(true)),
            Some(SnapshotBlocker::InitialBlockDownload)
        );
        assert_eq!(
            tip_gate(None, Some(100), Some(false)),
            Some(SnapshotBlocker::Unmeasured)
        );
    }

    #[test]
    fn nothing_measured_waits_rather_than_refusing_for_good() {
        let v = check(&SnapshotFacts::default());
        assert_eq!(v.blocker, Some(SnapshotBlocker::Unmeasured));
        assert!(v.blocker.unwrap().is_transient());
    }

    #[test]
    fn a_full_disk_is_refused_with_the_number() {
        let f = SnapshotFacts {
            free_disk_mb: Some(150),
            ..rig()
        };
        assert_eq!(
            check(&f).blocker,
            Some(SnapshotBlocker::DiskLow { free_mb: 150 })
        );
    }

    /// Section 4: an export only where the tip is exactly on the grid, and
    /// only above the newest base already exported (offered or waiting).
    /// Replaces "every 500 blocks from the last base" (`refresh_due`), which
    /// the design supersedes.
    #[test]
    fn an_export_is_due_only_on_the_grid_and_above_the_last_export() {
        assert!(export_due(226_200, 100, None));
        assert!(export_due(226_300, 100, Some(226_200)));
        assert!(!export_due(226_201, 100, None), "one block past the grid");
        assert!(!export_due(226_200, 100, Some(226_200)), "already exported");
        assert!(
            !export_due(226_200, 100, Some(226_300)),
            "below the last export"
        );
        assert!(!export_due(0, 100, None), "the genesis block");
        assert!(!export_due(226_200, 0, None), "no grid");
        // Upstream's compiled heights are on it.
        assert!(export_due(219_000, EXPORT_GRID, None));
        assert!(export_due(228_000, EXPORT_GRID, None));
        assert_eq!(blocks_to_grid(226_150, 100), 50);
        assert_eq!(blocks_to_grid(226_199, 100), 1);
        assert_eq!(blocks_to_grid(226_200, 100), 100);
        // The last export counts a waiting pair as well as the offered one.
        let w = |height| WaitingPair {
            height,
            block_hash: String::new(),
            txoutset_hash: String::new(),
            exported_at: 0,
        };
        let r = record_at(226_000);
        assert_eq!(last_exported(None, &[]), None);
        assert_eq!(last_exported(Some(&r), &[]), Some(226_000));
        assert_eq!(
            last_exported(Some(&r), &[w(226_100), w(226_200)]),
            Some(226_200)
        );
        assert_eq!(last_exported(None, &[w(226_100)]), Some(226_100));
    }

    /// Section 4's numbers: 144 deep before anything is offered or sent, the
    /// grid of 100, six hours to get there, four pairs on disk.
    #[test]
    fn the_wait_and_the_disk_follow_section_four() {
        assert_eq!(CONFIRMATIONS_REQUIRED, 144);
        assert_eq!(EXPORT_GRID, 100);
        assert_eq!(MATURE_DEADLINE, Duration::from_secs(6 * 3600));
        assert_eq!(KEEP_PAIRS, 4);
    }

    /// Each waiting pair on its own: off the chain is dropped, 144 deep is
    /// ready even past the deadline, short of it past six hours is dropped.
    #[test]
    fn maturity_is_read_per_pair() {
        let d = MATURE_DEADLINE.as_secs();
        assert_eq!(maturity(-1, 0, 10, MATURE_DEADLINE), Maturity::Orphaned);
        assert_eq!(
            maturity(143, 0, 10, MATURE_DEADLINE),
            Maturity::Waiting { confirmations: 143 }
        );
        assert_eq!(maturity(144, 0, 10, MATURE_DEADLINE), Maturity::Ready);
        assert_eq!(maturity(400, 0, d + 1, MATURE_DEADLINE), Maturity::Ready);
        assert_eq!(
            maturity(143, 0, d, MATURE_DEADLINE),
            Maturity::Waiting { confirmations: 143 },
            "exactly six hours is still in time"
        );
        assert_eq!(maturity(143, 0, d + 1, MATURE_DEADLINE), Maturity::Expired);
        assert_eq!(
            maturity(-1, 0, d + 1, MATURE_DEADLINE),
            Maturity::Orphaned,
            "off the chain says more than late"
        );
        // A clock that went backwards is not late.
        assert_eq!(
            maturity(3, 100, 50, MATURE_DEADLINE),
            Maturity::Waiting { confirmations: 3 }
        );
    }

    /// Four pairs on disk now (the offered one, the one before it and two
    /// waiting), where two were kept before; the protected ones (the offered
    /// pair and every waiting one) never go.
    #[test]
    fn pruning_keeps_four_and_never_the_offered_or_a_waiting_one() {
        let on_disk = [225_800, 225_900, 226_000, 226_100, 226_200, 226_300];
        assert_eq!(
            pairs_to_prune(&on_disk, 4, &[226_100, 226_200, 226_300]),
            vec![225_800, 225_900]
        );
        assert_eq!(
            pairs_to_prune(&on_disk[2..], 4, &[226_100]),
            Vec::<u64>::new()
        );
        // A record pointing at the oldest pair (a swap that failed after the
        // rename) must not have its files deleted from under it.
        assert_eq!(pairs_to_prune(&on_disk, 4, &[225_800]), vec![225_900]);
        assert_eq!(pairs_to_prune(&[3, 1, 2, 2], 1, &[]), vec![1, 2]);
    }

    #[test]
    fn file_names_round_trip_and_reject_strays() {
        assert_eq!(
            height_from_file_name(&snapshot_file_name(226_140)),
            Some(226_140)
        );
        assert_eq!(height_from_file_name("staging.dat"), None);
        assert_eq!(height_from_file_name("current-offer.json"), None);
        assert_eq!(height_from_file_name("snapshot-manifest-226140.json"), None);
    }

    #[test]
    fn the_offer_bit_is_read_from_the_bits_with_the_name_as_fallback() {
        // The producing node's localservices with and without the offer.
        assert!(advertises_offer("0000000188000d08", &[]));
        assert!(!advertises_offer("0000000088000d08", &[]));
        assert!(advertises_offer("", &["ATTESTED_UTXO_SNAPSHOT".into()]));
        assert!(!advertises_offer("", &["UNKNOWN[2^32]".into()]));
    }

    #[test]
    fn the_record_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let r = OfferRecord {
            height: 226_140,
            block_hash: "64e144d5".into(),
            txoutset_hash: "dcf828e4".into(),
            file_size: 9_059_813,
            sha256: "73ecb7a1".into(),
            manifest_sha256: "a226bb09".into(),
            file_hash: "f4607b19".into(),
            chunk_count: 9,
            signatures: 1,
            offered_at: 1_790_000_000,
        };
        assert_eq!(load_record(dir.path()), None);
        save_record(dir.path(), &r).unwrap();
        assert_eq!(load_record(dir.path()), Some(r));
    }

    fn record_at(height: u64) -> OfferRecord {
        OfferRecord {
            height,
            block_hash: String::new(),
            txoutset_hash: String::new(),
            file_size: 9_059_813,
            sha256: String::new(),
            manifest_sha256: String::new(),
            file_hash: String::new(),
            chunk_count: 9,
            signatures: 1,
            offered_at: 0,
        }
    }

    /// The next export is counted to the next grid height now, not 500
    /// blocks from the base, and the sentence says it waits 144 deep.
    #[test]
    fn the_serving_sentence_names_height_age_and_the_next_export() {
        let r = record_at(226_000);
        let m = ServeStatus::serving_message(&r, Some(226_150), Some(0));
        assert!(m.contains("226000"), "{m}");
        assert!(m.contains("150 blocks old"), "{m}");
        assert!(m.contains("in 50 blocks"), "{m}");
        assert!(m.contains("144 blocks deep"), "{m}");
        assert!(m.contains("No other node"), "{m}");
        assert!(!m.contains('\u{2014}'), "{m}");
        let m = ServeStatus::serving_message(&r, Some(226_000), None);
        assert!(m.contains("at the tip"), "{m}");
    }

    // ── A scripted node ─────────────────────────────────────────────────────

    /// A node that answers the RPCs maturing and re-offering use, with a
    /// scripted confirmation count per read and a switchable canonical hash:
    /// for the cases a consistent chain cannot show (a header that says 144
    /// deep while `getblockhash` already names a sibling). The export and
    /// the ordinary flow run on `crate::fake_node::FakeNode`.
    struct ScriptedNode {
        calls: Mutex<Vec<(String, Value)>>,
        confirmations: Mutex<Vec<i64>>,
        canonical: Mutex<String>,
        /// When set, `getblockhash` answers this instead of the dumped hash:
        /// a sibling took the base's height after the dump.
        sibling_at_verify: Mutex<Option<String>>,
        offering: Mutex<bool>,
        tip: u64,
    }

    impl ScriptedNode {
        fn new(confs: &[i64]) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                confirmations: Mutex::new(confs.to_vec()),
                canonical: Mutex::new("hash-1".into()),
                sibling_at_verify: Mutex::new(None),
                offering: Mutex::new(false),
                tip: 226_350,
            }
        }
        fn count(&self, method: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == method)
                .count()
        }
    }

    #[async_trait]
    impl Rpc for ScriptedNode {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), params.clone()));
            Ok(match method {
                "getblockheader" => {
                    let mut c = self.confirmations.lock().unwrap();
                    let v = if c.len() > 1 { c.remove(0) } else { c[0] };
                    json!({ "confirmations": v })
                }
                "getblockcount" => json!(self.tip),
                "getblockhash" => match self.sibling_at_verify.lock().unwrap().clone() {
                    Some(s) => json!(s),
                    None => json!(self.canonical.lock().unwrap().clone()),
                },
                "withdrawattestedutxosnapshot" => {
                    let was = std::mem::replace(&mut *self.offering.lock().unwrap(), false);
                    json!({ "withdrawn": was })
                }
                "offerattestedutxosnapshot" => {
                    *self.offering.lock().unwrap() = true;
                    let dat = params[0].as_str().unwrap();
                    let h = height_from_file_name(
                        Path::new(dat).file_name().unwrap().to_str().unwrap(),
                    )
                    .unwrap();
                    json!({
                        "block_hash": self.canonical.lock().unwrap().clone(),
                        "height": h,
                        "file_size": std::fs::metadata(dat).unwrap().len(),
                        "chunk_size": CHUNK_SIZE,
                        "chunk_count": 1,
                        "file_hash": "f4607b19",
                        "signatures": 1
                    })
                }
                "getnetworkinfo" => {
                    let on = *self.offering.lock().unwrap();
                    json!({
                        "localservices": if on { "0000000188000d08" } else { "0000000088000d08" },
                        "localservicesnames": []
                    })
                }
                "getpeerinfo" => json!([
                    { "addr": "20.86.181.203:19338", "services": "0000000088000d08" },
                    { "addr": "20.86.181.203:19335", "services": "0000000088000d08" },
                    { "addr": "89.85.40.184:19335", "services": "0000000088000d08" }
                ]),
                "disconnectnode" | "addnode" => Value::Null,
                other => panic!("the cycle called {other}, which the script does not know"),
            })
        }
    }

    // ── Exporting on the grid and maturing per height ──────────────────────

    use crate::fake_node::{synthetic_hash, FakeNode};
    use crate::operators::MAINNET_GENESIS;

    /// A hook that writes down what it was asked and answers `verdict`. Its
    /// `before_export` asks the node for its tip, so the order of the calls
    /// shows it ran before the dump.
    struct Hook {
        seen: Mutex<Vec<(u64, Vec<u8>)>>,
        exports: Mutex<u32>,
        verdict: Option<NotOffered>,
    }

    impl Hook {
        fn passing() -> Self {
            Self::answering(None)
        }
        fn answering(verdict: Option<NotOffered>) -> Self {
            Self {
                seen: Mutex::new(Vec::new()),
                exports: Mutex::new(0),
                verdict,
            }
        }
    }

    #[async_trait]
    impl BeforeOffer for Hook {
        async fn before_export(&self, rpc: &dyn Rpc) {
            *self.exports.lock().unwrap() += 1;
            let _ = rpc.call("getblockcount", json!([])).await;
        }
        async fn check(&self, _: &dyn Rpc, base: u64, manifest: &[u8]) -> Result<(), NotOffered> {
            self.seen.lock().unwrap().push((base, manifest.to_vec()));
            self.verdict.clone().map_or(Ok(()), Err)
        }
    }

    fn mainnet_at(tip: u64) -> FakeNode {
        FakeNode::new(MAINNET_GENESIS, tip)
    }

    async fn export(
        node: &FakeNode,
        dir: &Path,
        hook: &dyn BeforeOffer,
    ) -> Result<WaitingPair, String> {
        export_on_grid(node, dir, EXPORT_GRID, hook, &|_| {}).await
    }

    async fn mature_now(node: &dyn Rpc, dir: &Path, hook: &dyn BeforeOffer) -> Vec<MatureEvent> {
        mature(node, dir, MATURE_DEADLINE, hook, &|_| {}).await
    }

    /// A pair written down as waiting, the way an export leaves it.
    fn waiting_on_disk(dir: &Path, height: u64, hash: &str, exported_at: u64) {
        std::fs::write(dir.join(snapshot_file_name(height)), b"bytes").unwrap();
        std::fs::write(
            dir.join(manifest_file_name(height)),
            [0x02, 0x01, 0x46, 0xfa],
        )
        .unwrap();
        let mut w = load_waiting(dir);
        w.push(WaitingPair {
            height,
            block_hash: hash.into(),
            txoutset_hash: String::new(),
            exported_at,
        });
        save_waiting(dir, &w).unwrap();
    }

    /// The export lands on the grid, under its own height's names, and only
    /// waits: nothing is offered, and the waiting list is on disk for the
    /// next start. The hook ran before the dump, from the same tip (the
    /// producer writes its diary there).
    #[tokio::test]
    async fn an_export_on_the_grid_waits_and_is_written_down() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let node = mainnet_at(226_200);
        let hook = Hook::passing();
        let w = export(&node, d, &hook).await.unwrap();
        assert_eq!(
            (w.height, w.block_hash.as_str()),
            (226_200, synthetic_hash(226_200).as_str())
        );
        assert_eq!(*hook.exports.lock().unwrap(), 1);
        assert_eq!(
            node.methods(),
            vec!["getblockcount", "dumptxoutsetattested"],
            "the hook first, then the dump, nothing offered"
        );
        assert!(d.join(snapshot_file_name(226_200)).is_file());
        assert!(d.join(manifest_file_name(226_200)).is_file());
        assert!(!d.join(STAGING_DAT).exists());
        assert!(!d.join(STAGING_MANIFEST).exists());
        assert_eq!(load_waiting(d), vec![w], "survives a restart");
        assert_eq!(load_record(d), None);
    }

    /// The dump always takes the tip. If the tip moved off the grid between
    /// the keeper's look and the dump, the export is thrown away: nothing
    /// waits, nothing is left on disk, and the next chance is the next grid
    /// height.
    #[tokio::test]
    async fn an_export_that_landed_off_the_grid_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let node = mainnet_at(226_201);
        let err = export(&node, d, &Hook::passing()).await.unwrap_err();
        assert!(err.contains("the tip moved to 226201"), "{err}");
        assert!(load_waiting(d).is_empty());
        assert!(pairs_on_disk(d).is_empty());
        assert!(!d.join(STAGING_DAT).exists());
        assert!(!d.join(STAGING_MANIFEST).exists());
    }

    /// The whole story on one pair: nothing is offered at 143 deep, at 144
    /// the base is re-verified, the hook has its word on the export's own
    /// manifest, the old offer is withdrawn, the new one offered and
    /// recorded, the mirror links bounced, and the disk pruned to four.
    #[tokio::test]
    async fn a_pair_waits_until_144_deep_then_swaps_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        // Five older pairs, the newest of them offered. Two must go.
        for h in [225_600u64, 225_700, 225_800, 225_900, 226_000] {
            std::fs::write(d.join(snapshot_file_name(h)), b"old").unwrap();
            std::fs::write(d.join(manifest_file_name(h)), b"old").unwrap();
        }
        save_record(d, &record_at(226_000)).unwrap();
        let node = mainnet_at(226_200);
        node.with(|s| {
            s.offering = Some(("old".into(), "old".into()));
            s.peers = vec![
                json!({ "addr": "20.86.181.203:19338", "services": "0000000088000d08" }),
                json!({ "addr": "20.86.181.203:19335", "services": "0000000088000d08" }),
                json!({ "addr": "89.85.40.184:19335", "services": "0000000088000d08" }),
            ];
        });
        let hook = Hook::passing();
        export(&node, d, &hook).await.unwrap();

        node.with(|s| s.extend_to(226_342));
        let phases = Mutex::new(Vec::new());
        let events = mature(&node, d, MATURE_DEADLINE, &hook, &|p| {
            phases.lock().unwrap().push(p)
        })
        .await;
        assert_eq!(
            events,
            vec![MatureEvent::Waiting {
                base: 226_200,
                confirmations: 143,
                tip: 226_342
            }]
        );
        assert!(hook.seen.lock().unwrap().is_empty(), "not checked yet");
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
        assert_eq!(node.count("withdrawattestedutxosnapshot"), 0);
        assert_eq!(
            load_record(d).unwrap().height,
            226_000,
            "the old offer stays"
        );

        node.with(|s| s.extend_to(226_343));
        let events = mature(&node, d, MATURE_DEADLINE, &hook, &|p| {
            phases.lock().unwrap().push(p)
        })
        .await;
        let [MatureEvent::Offered(record)] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert_eq!(record.height, 226_200);
        assert_eq!(record.block_hash, synthetic_hash(226_200));
        assert_eq!(record.sha256, sha256_hex(b"snapshot bytes 226200"));
        assert_eq!(
            load_record(d).as_ref(),
            Some(record),
            "recorded after the offer"
        );
        assert!(load_waiting(d).is_empty(), "no longer waiting");
        assert_eq!(
            hook.seen.lock().unwrap().clone(),
            vec![(226_200, vec![0x02, 0x01, 0x46, 0xfa])],
            "the hook saw the export's own manifest"
        );
        assert_eq!(node.count("withdrawattestedutxosnapshot"), 1);
        assert_eq!(node.count("offerattestedutxosnapshot"), 1);
        assert_eq!(offer_live(&node).await, Some(true));
        let mut kept = pairs_on_disk(d);
        kept.sort_unstable();
        assert_eq!(kept, vec![225_800, 225_900, 226_000, 226_200]);
        // Both mirror links were bounced, the unrelated peer was not.
        let dropped: Vec<String> = node
            .methods()
            .into_iter()
            .filter(|m| m == "disconnectnode")
            .collect();
        assert_eq!(dropped.len(), 2);
        // The row saw the story.
        let seen = phases.lock().unwrap().clone();
        assert!(seen.iter().any(|p| matches!(
            p,
            CyclePhase::Maturing {
                confirmations: 143,
                ..
            }
        )));
        assert!(matches!(
            seen.last(),
            Some(CyclePhase::Live { base: 226_200 })
        ));
    }

    /// Because 144 is more than 100, the next grid height comes while the
    /// last export still waits. Both wait, each on its own height, and each
    /// is offered when it is itself 144 deep.
    #[tokio::test]
    async fn two_exports_wait_at_once_each_on_its_own_height() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let node = mainnet_at(226_200);
        let hook = Hook::passing();
        export(&node, d, &hook).await.unwrap();
        node.with(|s| s.extend_to(226_300));
        let last = last_exported(load_record(d).as_ref(), &load_waiting(d));
        assert!(export_due(226_300, EXPORT_GRID, last));
        export(&node, d, &hook).await.unwrap();
        let heights: Vec<u64> = load_waiting(d).iter().map(|w| w.height).collect();
        assert_eq!(heights, vec![226_200, 226_300]);

        node.with(|s| s.extend_to(226_343));
        let events = mature_now(&node, d, &hook).await;
        assert!(
            matches!(&events[..], [MatureEvent::Offered(r), MatureEvent::Waiting { base: 226_300, confirmations: 44, .. }] if r.height == 226_200),
            "{events:?}"
        );
        node.with(|s| s.extend_to(226_443));
        let events = mature_now(&node, d, &hook).await;
        assert!(
            matches!(&events[..], [MatureEvent::Offered(r)] if r.height == 226_300),
            "{events:?}"
        );
        assert_eq!(load_record(d).unwrap().height, 226_300);
        assert!(load_waiting(d).is_empty());
        assert!(
            d.join(snapshot_file_name(226_200)).is_file(),
            "the one before it stays"
        );
    }

    /// Two pairs that came of age together (a keeper that was off for a
    /// while): one is offered per round, the older first, so each is
    /// offered, and sent by the producer, in turn.
    #[tokio::test]
    async fn two_pairs_ready_at_once_are_offered_one_per_round() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let node = mainnet_at(226_200);
        let hook = Hook::passing();
        export(&node, d, &hook).await.unwrap();
        node.with(|s| s.extend_to(226_300));
        export(&node, d, &hook).await.unwrap();
        node.with(|s| s.extend_to(226_500));
        let events = mature_now(&node, d, &hook).await;
        assert!(
            matches!(&events[..], [MatureEvent::Offered(r), MatureEvent::NotYet { base: 226_300, .. }] if r.height == 226_200),
            "{events:?}"
        );
        let events = mature_now(&node, d, &hook).await;
        assert!(
            matches!(&events[..], [MatureEvent::Offered(r)] if r.height == 226_300),
            "{events:?}"
        );
    }

    /// The 2026-09-20 shape: the base is orphaned while it waits. It is
    /// dropped with its files, never offered, and no second dump is made (it
    /// would base on today's tip, off the grid): the next chance is the next
    /// grid height. Before the design, the keeper dumped again at once.
    #[tokio::test]
    async fn a_base_that_leaves_the_chain_while_it_waits_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let node = mainnet_at(226_200);
        let hook = Hook::passing();
        export(&node, d, &hook).await.unwrap();
        node.with(|s| s.reorg_from(226_195, 226_250));
        let events = mature_now(&node, d, &hook).await;
        assert!(
            matches!(&events[..], [MatureEvent::Dropped { base: 226_200, why }] if why.contains("left the chain")),
            "{events:?}"
        );
        assert_eq!(node.count("dumptxoutsetattested"), 1, "no second dump");
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
        assert!(hook.seen.lock().unwrap().is_empty());
        assert!(load_waiting(d).is_empty());
        assert!(pairs_on_disk(d).is_empty(), "its files went with it");
        assert_eq!(load_record(d), None);
        let last = last_exported(None, &load_waiting(d));
        assert!(!export_due(226_250, EXPORT_GRID, last));
        assert!(export_due(226_300, EXPORT_GRID, last));
    }

    #[tokio::test]
    async fn a_base_that_leaves_the_chain_at_the_last_moment_is_not_offered() {
        // 144 confirmations reported, but by the time of the final re-verify
        // the active chain has a sibling at that height. Confirmations and
        // canonicality are read separately, and the last word is the latter.
        let dir = tempfile::tempdir().unwrap();
        waiting_on_disk(dir.path(), 226_200, "hash-1", now_unix());
        let node = ScriptedNode::new(&[144]);
        *node.sibling_at_verify.lock().unwrap() = Some("some-sibling".into());
        let hook = Hook::passing();
        let events = mature_now(&node, dir.path(), &hook).await;
        assert!(
            matches!(&events[..], [MatureEvent::Dropped { base: 226_200, why }] if why.contains("left the active chain")),
            "{events:?}"
        );
        assert!(hook.seen.lock().unwrap().is_empty(), "never got as far");
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
        assert_eq!(load_record(dir.path()), None);
    }

    /// Six hours and still short of 144: a chain that is not moving, and
    /// nothing to serve. Dropped. One that is still in time keeps waiting,
    /// and a lost read is not a verdict.
    #[tokio::test]
    async fn a_base_that_never_matures_is_dropped_after_six_hours() {
        let dir = tempfile::tempdir().unwrap();
        waiting_on_disk(dir.path(), 226_200, "hash-1", 0);
        let node = ScriptedNode::new(&[3]);
        let events = mature_now(&node, dir.path(), &Hook::passing()).await;
        assert!(
            matches!(&events[..], [MatureEvent::Dropped { base: 226_200, why }] if why.contains("144")),
            "{events:?}"
        );
        assert!(load_waiting(dir.path()).is_empty());
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);

        let dir = tempfile::tempdir().unwrap();
        waiting_on_disk(dir.path(), 226_200, "unknown-to-the-node", now_unix());
        let node = FakeNode::new(MAINNET_GENESIS, 226_210);
        let events = mature_now(&node, dir.path(), &Hook::passing()).await;
        assert!(
            matches!(&events[..], [MatureEvent::NotYet { base: 226_200, .. }]),
            "{events:?}"
        );
        assert_eq!(load_waiting(dir.path()).len(), 1);
    }

    /// Section 4: a producer never sends a pair that failed any check. The
    /// hook's refusal keeps it off the wire for good (dropped with its
    /// files), and the previous offer stays live and recorded.
    #[tokio::test]
    async fn a_pair_the_checks_refuse_is_never_offered_and_the_old_offer_stays() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join(snapshot_file_name(226_000)), b"old").unwrap();
        std::fs::write(d.join(manifest_file_name(226_000)), b"old").unwrap();
        save_record(d, &record_at(226_000)).unwrap();
        let node = mainnet_at(226_200);
        node.with(|s| s.offering = Some(("old".into(), "old".into())));
        let hook = Hook::answering(Some(NotOffered {
            why: "coin count differs from the diary (diary 7, statement 8)".into(),
            retry: false,
        }));
        export(&node, d, &hook).await.unwrap();
        node.with(|s| s.extend_to(226_343));
        let events = mature_now(&node, d, &hook).await;
        assert_eq!(
            events,
            vec![MatureEvent::Dropped {
                base: 226_200,
                why: "not offered: coin count differs from the diary (diary 7, statement 8)".into()
            }]
        );
        assert_eq!(hook.seen.lock().unwrap().len(), 1);
        assert_eq!(node.count("withdrawattestedutxosnapshot"), 0);
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
        assert_eq!(offer_live(&node).await, Some(true), "the old offer is live");
        assert_eq!(load_record(d).unwrap().height, 226_000);
        assert!(load_waiting(d).is_empty());
        assert!(!d.join(snapshot_file_name(226_200)).exists());
    }

    /// A check that cannot decide yet (the node did not answer, the
    /// background check is still running) keeps the pair waiting, files and
    /// all, and asks again on the next round, until the six hours are up.
    #[tokio::test]
    async fn a_check_that_cannot_decide_yet_keeps_the_pair_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let node = mainnet_at(226_200);
        let hook = Hook::answering(Some(NotOffered {
            why: "the node did not answer".into(),
            retry: true,
        }));
        export(&node, d, &hook).await.unwrap();
        node.with(|s| s.extend_to(226_343));
        let events = mature_now(&node, d, &hook).await;
        assert!(
            matches!(&events[..], [MatureEvent::NotYet { base: 226_200, why }] if why.contains("did not answer")),
            "{events:?}"
        );
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
        assert_eq!(load_waiting(d).len(), 1);
        assert!(d.join(snapshot_file_name(226_200)).is_file());
        // The next round the check passes.
        let events = mature_now(&node, d, &Hook::passing()).await;
        assert!(
            matches!(&events[..], [MatureEvent::Offered(_)]),
            "{events:?}"
        );
    }

    /// The pair that was offered is checked again before it is re-offered
    /// after a node start: a pair that no longer passes is not sent.
    #[tokio::test]
    async fn reoffer_runs_the_checks_and_refuses_a_pair_that_fails_them() {
        let dir = tempfile::tempdir().unwrap();
        recorded(dir.path(), 226_140, "hash-1");
        let node = ScriptedNode::new(&[144]);
        let hook = Hook::answering(Some(NotOffered {
            why: "a held block is on this node's chain".into(),
            retry: false,
        }));
        let out = reoffer(
            &node,
            dir.path(),
            Some(226_150),
            Some(226_150),
            Some(false),
            &hook,
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            ReofferOutcome::Refused {
                height: 226_140,
                why: "a held block is on this node's chain".into()
            }
        );
        assert_eq!(
            hook.seen.lock().unwrap().clone(),
            vec![(226_140, b"m".to_vec())]
        );
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
    }

    fn recorded(dir: &Path, height: u64, hash: &str) -> OfferRecord {
        std::fs::write(dir.join(snapshot_file_name(height)), b"bytes").unwrap();
        std::fs::write(dir.join(manifest_file_name(height)), b"m").unwrap();
        let r = OfferRecord {
            height,
            block_hash: hash.into(),
            txoutset_hash: String::new(),
            file_size: 5,
            sha256: String::new(),
            manifest_sha256: String::new(),
            file_hash: String::new(),
            chunk_count: 1,
            signatures: 1,
            offered_at: 0,
        };
        save_record(dir, &r).unwrap();
        r
    }

    #[tokio::test]
    async fn reoffer_after_a_restart_offers_the_recorded_pair_and_bounces_the_mirror() {
        let dir = tempfile::tempdir().unwrap();
        recorded(dir.path(), 226_140, "hash-1");
        let node = ScriptedNode::new(&[10]);
        let out = reoffer(
            &node,
            dir.path(),
            Some(226_150),
            Some(226_150),
            Some(false),
            &NoChecks,
        )
        .await
        .unwrap();
        assert_eq!(out, ReofferOutcome::Reoffered { height: 226_140 });
        assert_eq!(node.count("offerattestedutxosnapshot"), 1);
        assert_eq!(
            node.count("disconnectnode"),
            2,
            "the handshake has to carry the bit"
        );
        // And it is idempotent: the second call sees the bit and does nothing.
        let again = reoffer(
            &node,
            dir.path(),
            Some(226_150),
            Some(226_150),
            Some(false),
            &NoChecks,
        )
        .await
        .unwrap();
        assert_eq!(again, ReofferOutcome::AlreadyLive);
        assert_eq!(node.count("offerattestedutxosnapshot"), 1);
    }

    #[tokio::test]
    async fn reoffer_refuses_a_base_that_is_no_longer_canonical() {
        let dir = tempfile::tempdir().unwrap();
        recorded(dir.path(), 226_140, "hash-1");
        let node = ScriptedNode::new(&[10]);
        *node.canonical.lock().unwrap() = "a-sibling".into();
        let out = reoffer(
            &node,
            dir.path(),
            Some(226_150),
            Some(226_150),
            Some(false),
            &NoChecks,
        )
        .await
        .unwrap();
        assert_eq!(out, ReofferOutcome::BaseNotCanonical { height: 226_140 });
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
    }

    #[tokio::test]
    async fn reoffer_waits_while_the_node_is_behind() {
        let dir = tempfile::tempdir().unwrap();
        recorded(dir.path(), 226_140, "hash-1");
        let node = ScriptedNode::new(&[10]);
        let out = reoffer(
            &node,
            dir.path(),
            Some(226_100),
            Some(226_160),
            Some(false),
            &NoChecks,
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            ReofferOutcome::Blocked(SnapshotBlocker::BehindTip { blocks_behind: 60 })
        );
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
    }

    #[tokio::test]
    async fn reoffer_with_nothing_recorded_or_files_gone_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let node = ScriptedNode::new(&[10]);
        assert_eq!(
            reoffer(&node, dir.path(), Some(1), Some(1), Some(false), &NoChecks)
                .await
                .unwrap(),
            ReofferOutcome::NoRecord
        );
        let r = recorded(dir.path(), 226_140, "hash-1");
        std::fs::remove_file(dir.path().join(snapshot_file_name(r.height))).unwrap();
        assert_eq!(
            reoffer(&node, dir.path(), Some(1), Some(1), Some(false), &NoChecks)
                .await
                .unwrap(),
            ReofferOutcome::FilesMissing { height: 226_140 }
        );
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
    }

    #[tokio::test]
    async fn facts_read_the_help_answer_the_way_this_engine_gives_it() {
        struct OldEngine;
        #[async_trait]
        impl Rpc for OldEngine {
            async fn call(&self, method: &str, _p: Value) -> AppResult<Value> {
                Ok(match method {
                    "help" => json!("help: unknown command: dumptxoutsetattested"),
                    "getmatmultrustedstatus" => {
                        json!({"local_signer": true, "matmul_validation_mode": "consensus"})
                    }
                    _ => json!({"blocks": 5, "headers": 5, "initialblockdownload": false}),
                })
            }
        }
        let f = read_facts(&OldEngine, Some(10_000)).await;
        assert_eq!(f.engine_knows_rpc, Some(false));
        assert_eq!(check(&f).blocker, Some(SnapshotBlocker::EngineTooOld));
    }

    #[tokio::test]
    async fn peers_offering_counts_bit_32_on_peers() {
        let node = ScriptedNode::new(&[1]);
        assert_eq!(peers_offering(&node).await, Some(0));
        assert_eq!(offer_live(&node).await, Some(false));
    }
}
