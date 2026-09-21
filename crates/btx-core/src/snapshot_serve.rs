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
//!   within 40 seconds and had been offered. So the cycle is dump, WAIT for
//!   [`CONFIRMATIONS_REQUIRED`] confirmations, re-verify the base is still on
//!   the active chain, THEN offer. Upstream parks reorgs deeper than 6; ten
//!   is that with margin. A base that reaches -1 confirmations is dumped
//!   again. The old offer stays live throughout, so serving never stops.
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

/// Confirmations a base needs before it is offered. See the module header:
/// the dump bases on the 0-conf tip, siblings arrive every ~25 blocks, and
/// upstream's own reorg park depth is 6.
pub const CONFIRMATIONS_REQUIRED: u64 = 10;

/// Re-export once the tip is this far past the offered base. About eleven
/// hours at the 45 blocks/h measured on 2026-09-21; an importer then catches
/// up at most this much, which a mirror does in minutes.
pub const REFRESH_BLOCKS: u64 = 500;

/// Headers ahead of blocks at which the node counts as "at the tip" for the
/// purpose of dumping or re-offering. Wider than
/// [`crate::role::HEADERS_AHEAD_IS_BEHIND`] on purpose: this gate decides
/// whether to ACT, and one block in flight is not a reason to skip a cycle.
pub const TIP_GATE_HEADERS_AHEAD: u64 = 5;

/// Snapshot pairs kept on disk: the offered one and the one before it, so a
/// failed swap always has something the keeper can fall back to.
pub const KEEP_PAIRS: usize = 2;

/// How often the maturation wait reads the base's confirmations. One block
/// is ~90 s, so anything faster only burns RPC.
pub const MATURE_POLL: Duration = Duration::from_secs(60);

/// How long a base may take to mature before the cycle gives up. Ninety
/// minutes is ~60 blocks; a base that has not reached ten confirmations by
/// then is on a chain that is not moving, and there is nothing to serve.
pub const MATURE_DEADLINE: Duration = Duration::from_secs(90 * 60);

/// Chunk the engine serves the file in. Its default and the one every
/// measured fetch used (9 chunks for a 9 MB file, 19 s over loopback).
pub const CHUNK_SIZE: u64 = 1 << 20;

/// Free disk below which the role refuses to start. A pair is ~9 MB and two
/// are kept, so this is headroom rather than a budget: a datadir this full
/// has bigger problems than a snapshot, and adding to it helps nobody.
pub const MIN_FREE_DISK_MB: u64 = 200;

/// Folder under the datadir holding the pairs and the offer record.
pub const SNAPSHOT_DIR: &str = "snapshots";
/// The record of what is offered, next to the pairs. JSON, ours, and the
/// only thing the keeper needs to re-offer after a restart.
pub const OFFER_RECORD: &str = "current-offer.json";
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
}

impl SnapshotBlocker {
    /// A transient blocker clears on its own (the node catches up); the keeper
    /// waits. A permanent one needs an operator (a setting, an engine), so the
    /// row asks for attention instead of quietly polling forever.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            SnapshotBlocker::Unmeasured
                | SnapshotBlocker::InitialBlockDownload
                | SnapshotBlocker::BehindTip { .. }
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
    } else if f.validation_mode.is_none() || f.local_signer.is_none() {
        Some(SnapshotBlocker::Unmeasured)
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
             9 MB and two are kept, but a disk this full needs space before it needs \
             another file."
        ),
    }
}

// ── Pure decisions ──────────────────────────────────────────────────────────

/// Is it time to export again? `base` is the offered base's height, `None`
/// when nothing has ever been offered from this folder.
pub fn refresh_due(tip: u64, base: Option<u64>) -> bool {
    match base {
        None => true,
        Some(b) => tip.saturating_sub(b) >= REFRESH_BLOCKS,
    }
}

/// Which heights to delete so that at most `keep` pairs remain, never the one
/// currently offered. Oldest first.
pub fn pairs_to_prune(heights: &[u64], keep: usize, current: Option<u64>) -> Vec<u64> {
    let mut sorted: Vec<u64> = heights.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let excess = sorted.len().saturating_sub(keep);
    sorted
        .into_iter()
        .take(excess)
        .filter(|h| Some(*h) != current)
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

/// Delete pairs beyond [`KEEP_PAIRS`], never the offered one. Returns what
/// went.
pub fn prune(dir: &Path, current: Option<u64>) -> Vec<u64> {
    let gone = pairs_to_prune(&pairs_on_disk(dir), KEEP_PAIRS, current);
    for h in &gone {
        let _ = std::fs::remove_file(dir.join(snapshot_file_name(*h)));
        let _ = std::fs::remove_file(dir.join(manifest_file_name(*h)));
    }
    gone
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
    let (status, chain, help) = tokio::join!(
        rpc.call("getmatmultrustedstatus", json!([])),
        rpc.call("getblockchaininfo", json!([])),
        rpc.call("help", json!(["dumptxoutsetattested"])),
    );
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

/// One full cycle: dump, wait, verify, withdraw the old, offer the new,
/// record, bounce, prune. Returns the record of what is now served.
///
/// * `poll` / `deadline` are [`MATURE_POLL`] / [`MATURE_DEADLINE`] in the app
///   and milliseconds in tests.
/// * `keep_going` is asked between polls; `false` aborts (the node stopped,
///   the role was switched off). Nothing is offered after an abort.
/// * `on_phase` receives every phase change, for the status row.
pub async fn run_cycle(
    rpc: &dyn Rpc,
    dir: &Path,
    poll: Duration,
    deadline: Duration,
    keep_going: &(dyn Fn() -> bool + Sync),
    on_phase: &(dyn Fn(CyclePhase) + Sync),
) -> Result<OfferRecord, String> {
    let started = std::time::Instant::now();
    on_phase(CyclePhase::Dumping);
    let mut base = dump(rpc, dir)
        .await
        .map_err(|e| format!("export failed: {e}"))?;
    loop {
        if !keep_going() {
            return Err("stopped while the base was maturing".into());
        }
        tokio::time::sleep(poll).await;
        let Ok(c) = confirmations(rpc, &base.base_hash).await else {
            continue; // one lost read is not a verdict
        };
        let tip = rpc
            .call("getblockcount", json!([]))
            .await
            .ok()
            .and_then(|v| v.as_u64())
            .unwrap_or(base.base_height);
        if c < 0 {
            // Orphaned. Exactly what happened to the first one ever taken.
            on_phase(CyclePhase::Dumping);
            base = dump(rpc, dir)
                .await
                .map_err(|e| format!("re-export failed: {e}"))?;
            continue;
        }
        on_phase(CyclePhase::Maturing {
            base: base.base_height,
            confirmations: c as u64,
            tip,
        });
        if c as u64 >= CONFIRMATIONS_REQUIRED {
            break;
        }
        if started.elapsed() > deadline {
            return Err(format!(
                "base {} did not reach {CONFIRMATIONS_REQUIRED} confirmations in time",
                base.base_height
            ));
        }
    }
    // Final re-verify, immediately before anything is offered.
    match base_is_canonical(rpc, base.base_height, &base.base_hash).await {
        Ok(true) => {}
        Ok(false) => {
            return Err(format!(
                "base {} left the active chain just before the offer",
                base.base_height
            ))
        }
        Err(e) => return Err(format!("could not re-verify the base: {e}")),
    }
    let dat = dir.join(snapshot_file_name(base.base_height));
    let man = dir.join(manifest_file_name(base.base_height));
    // Renaming is safe: the manifest embeds no path.
    std::fs::rename(dir.join(STAGING_DAT), &dat).map_err(|e| format!("rename: {e}"))?;
    std::fs::rename(dir.join(STAGING_MANIFEST), &man).map_err(|e| format!("rename: {e}"))?;
    let (file_size, sha256) = sha256_of_file(&dat)
        .await
        .map_err(|e| format!("hashing the snapshot: {e}"))?;
    let (_, manifest_sha256) = sha256_of_file(&man)
        .await
        .map_err(|e| format!("hashing the manifest: {e}"))?;

    on_phase(CyclePhase::Offering {
        base: base.base_height,
    });
    // Offering on top of a live offer is untested; withdraw first. The gap is
    // one RPC round trip, and a failure here leaves the previous record for
    // the keeper to re-offer from.
    let _ = withdraw(rpc).await;
    let offered = offer(rpc, &dat, &man)
        .await
        .map_err(|e| format!("offer failed: {e}"))?;
    let record = OfferRecord {
        height: offered.height,
        block_hash: offered.block_hash,
        txoutset_hash: base.txoutset_hash,
        file_size,
        sha256,
        manifest_sha256,
        file_hash: offered.file_hash,
        chunk_count: offered.chunk_count,
        signatures: offered.signatures,
        offered_at: now_unix(),
    };
    save_record(dir, &record).map_err(|e| format!("recording the offer: {e}"))?;
    on_phase(CyclePhase::Live {
        base: record.height,
    });
    bounce_mirror_links(rpc).await;
    prune(dir, Some(record.height));
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
    /// offered; the next refresh replaces it.
    BaseNotCanonical {
        height: u64,
    },
    Reoffered {
        height: u64,
    },
}

/// Re-assert the recorded offer after a node start. Idempotent, never dumps,
/// refuses when the base is no longer canonical.
pub async fn reoffer(
    rpc: &dyn Rpc,
    dir: &Path,
    blocks: Option<u64>,
    headers: Option<u64>,
    initial_block_download: Option<bool>,
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
        let next = stale
            .map(|n| REFRESH_BLOCKS.saturating_sub(n))
            .map(|n| format!(" A fresh one is taken in {n} blocks."))
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
        }
    }

    #[test]
    fn the_producing_node_is_allowed() {
        assert!(check(&rig()).is_allowed(), "{:?}", check(&rig()));
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

    #[test]
    fn refresh_is_due_at_exactly_five_hundred_blocks_or_with_no_base() {
        assert!(refresh_due(226_640, Some(226_140)));
        assert!(!refresh_due(226_639, Some(226_140)));
        assert!(refresh_due(10, None));
        assert!(
            !refresh_due(5, Some(10)),
            "a base ahead of the tip is not stale"
        );
    }

    #[test]
    fn pruning_keeps_the_two_newest_and_never_the_offered_one() {
        assert_eq!(
            pairs_to_prune(&[225_186, 225_927, 226_140], 2, Some(226_140)),
            vec![225_186]
        );
        assert_eq!(
            pairs_to_prune(&[225_927, 226_140], 2, Some(226_140)),
            Vec::<u64>::new()
        );
        // A record pointing at the oldest pair (a swap that failed after the
        // rename) must not have its files deleted from under it.
        assert_eq!(
            pairs_to_prune(&[225_186, 225_927, 226_140], 2, Some(225_186)),
            Vec::<u64>::new()
        );
        assert_eq!(pairs_to_prune(&[3, 1, 2, 2], 1, None), vec![1, 2]);
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

    #[test]
    fn the_serving_sentence_names_height_age_and_the_next_refresh() {
        let r = OfferRecord {
            height: 226_140,
            block_hash: String::new(),
            txoutset_hash: String::new(),
            file_size: 9_059_813,
            sha256: String::new(),
            manifest_sha256: String::new(),
            file_hash: String::new(),
            chunk_count: 9,
            signatures: 1,
            offered_at: 0,
        };
        let m = ServeStatus::serving_message(&r, Some(226_150), Some(0));
        assert!(m.contains("226140"), "{m}");
        assert!(m.contains("10 blocks old"), "{m}");
        assert!(m.contains("in 490 blocks"), "{m}");
        assert!(m.contains("No other node"), "{m}");
        let m = ServeStatus::serving_message(&r, Some(226_140), None);
        assert!(m.contains("at the tip"), "{m}");
    }

    // ── A scripted node ─────────────────────────────────────────────────────

    /// A node that answers the seven RPCs the cycle uses, with a scripted
    /// confirmation count per read and a switchable canonical hash.
    struct ScriptedNode {
        calls: Mutex<Vec<(String, Value)>>,
        confirmations: Mutex<Vec<i64>>,
        canonical: Mutex<String>,
        /// When set, `getblockhash` answers this instead of the dumped hash:
        /// a sibling took the base's height after the dump.
        sibling_at_verify: Mutex<Option<String>>,
        dumps: Mutex<u64>,
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
                dumps: Mutex::new(0),
                offering: Mutex::new(false),
                tip: 226_150,
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
        fn params_of(&self, method: &str) -> Vec<Value> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == method)
                .map(|(_, p)| p.clone())
                .collect()
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
                "dumptxoutsetattested" => {
                    let n = {
                        let mut d = self.dumps.lock().unwrap();
                        *d += 1;
                        *d
                    };
                    // Write the staging pair the way the engine would.
                    let dat = params[0].as_str().unwrap();
                    let man = params[1].as_str().unwrap();
                    std::fs::write(dat, format!("snapshot bytes {n}")).unwrap();
                    std::fs::write(man, [0x02, 0x01, 0x46, 0xfa]).unwrap();
                    *self.canonical.lock().unwrap() = format!("hash-{n}");
                    json!({
                        "base_height": 226_140 + n - 1,
                        "base_hash": format!("hash-{n}"),
                        "txoutset_hash": "dcf828e4",
                        "coins_written": 140_936,
                        "nchaintx": 328_424,
                        "max_cs_main_hold_us": 7385
                    })
                }
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

    fn fast() -> (Duration, Duration) {
        (Duration::from_millis(1), Duration::from_secs(5))
    }

    #[tokio::test]
    async fn a_cycle_waits_for_ten_confirmations_then_swaps_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        // Two old pairs already on disk, the older one is what should go.
        for h in [225_186u64, 225_927] {
            std::fs::write(d.join(snapshot_file_name(h)), b"old").unwrap();
            std::fs::write(d.join(manifest_file_name(h)), b"old").unwrap();
        }
        let node = ScriptedNode::new(&[3, 4, 7, 9, 10]);
        let phases = Mutex::new(Vec::new());
        let (poll, deadline) = fast();
        let record = run_cycle(&node, d, poll, deadline, &|| true, &|p| {
            phases.lock().unwrap().push(p)
        })
        .await
        .unwrap();

        assert_eq!(record.height, 226_140);
        assert_eq!(record.block_hash, "hash-1");
        assert_eq!(record.sha256, sha256_hex(b"snapshot bytes 1"));
        assert_eq!(
            load_record(d),
            Some(record.clone()),
            "recorded after the offer"
        );
        assert!(
            d.join(snapshot_file_name(226_140)).is_file(),
            "renamed into place"
        );
        assert!(!d.join(STAGING_DAT).exists());
        assert!(
            !d.join(snapshot_file_name(225_186)).exists(),
            "oldest pair pruned"
        );
        assert!(
            d.join(snapshot_file_name(225_927)).is_file(),
            "previous pair kept"
        );

        // Offered exactly once, after a withdraw, and only once matured.
        assert_eq!(node.count("offerattestedutxosnapshot"), 1);
        assert_eq!(node.count("withdrawattestedutxosnapshot"), 1);
        assert_eq!(
            node.count("getblockheader"),
            5,
            "one read per poll until 10"
        );
        assert_eq!(
            node.count("getblockhash"),
            1,
            "re-verified right before the offer"
        );
        // Both mirror links were bounced, the unrelated peer was not.
        let dropped = node.params_of("disconnectnode");
        assert_eq!(dropped.len(), 2, "{dropped:?}");
        assert!(dropped
            .iter()
            .all(|p| p[0].as_str().unwrap().starts_with("20.86.181.203")));
        // The row saw the whole story.
        let seen = phases.lock().unwrap().clone();
        assert!(matches!(seen.first(), Some(CyclePhase::Dumping)));
        assert!(seen.iter().any(|p| matches!(
            p,
            CyclePhase::Maturing {
                confirmations: 10,
                ..
            }
        )));
        assert!(matches!(
            seen.last(),
            Some(CyclePhase::Live { base: 226_140 })
        ));
    }

    #[tokio::test]
    async fn an_orphaned_base_is_dumped_again_and_the_first_is_never_offered() {
        // The 2026-09-20 shape: the first base is orphaned within a block.
        let dir = tempfile::tempdir().unwrap();
        let node = ScriptedNode::new(&[2, -1, 5, 10]);
        let (poll, deadline) = fast();
        let record = run_cycle(&node, dir.path(), poll, deadline, &|| true, &|_| {})
            .await
            .unwrap();
        assert_eq!(node.count("dumptxoutsetattested"), 2);
        assert_eq!(record.height, 226_141, "the second dump's base");
        assert_eq!(record.block_hash, "hash-2");
        assert_eq!(node.count("offerattestedutxosnapshot"), 1);
    }

    #[tokio::test]
    async fn a_base_that_leaves_the_chain_at_the_last_moment_is_not_offered() {
        // Ten confirmations reported, but by the time of the final re-verify
        // the active chain has a sibling at that height. Confirmations and
        // canonicality are read separately, and the last word is the latter.
        let dir = tempfile::tempdir().unwrap();
        let node = ScriptedNode::new(&[10]);
        *node.sibling_at_verify.lock().unwrap() = Some("some-sibling".into());
        let (poll, deadline) = fast();
        let err = run_cycle(&node, dir.path(), poll, deadline, &|| true, &|_| {})
            .await
            .unwrap_err();
        assert!(err.contains("left the active chain"), "{err}");
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
        assert_eq!(load_record(dir.path()), None);
    }

    #[tokio::test]
    async fn a_stopped_role_aborts_the_wait_and_offers_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let node = ScriptedNode::new(&[1, 2, 3]);
        let (poll, deadline) = fast();
        let err = run_cycle(&node, dir.path(), poll, deadline, &|| false, &|_| {})
            .await
            .unwrap_err();
        assert!(err.contains("stopped"), "{err}");
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
        assert_eq!(load_record(dir.path()), None);
    }

    #[tokio::test]
    async fn a_base_that_never_matures_times_out_without_offering() {
        let dir = tempfile::tempdir().unwrap();
        let node = ScriptedNode::new(&[3]);
        let err = run_cycle(
            &node,
            dir.path(),
            Duration::from_millis(1),
            Duration::from_millis(20),
            &|| true,
            &|_| {},
        )
        .await
        .unwrap_err();
        assert!(err.contains("did not reach"), "{err}");
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
        let out = reoffer(&node, dir.path(), Some(226_150), Some(226_150), Some(false))
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
        let again = reoffer(&node, dir.path(), Some(226_150), Some(226_150), Some(false))
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
        let out = reoffer(&node, dir.path(), Some(226_150), Some(226_150), Some(false))
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
        let out = reoffer(&node, dir.path(), Some(226_100), Some(226_160), Some(false))
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
            reoffer(&node, dir.path(), Some(1), Some(1), Some(false))
                .await
                .unwrap(),
            ReofferOutcome::NoRecord
        );
        let r = recorded(dir.path(), 226_140, "hash-1");
        std::fs::remove_file(dir.path().join(snapshot_file_name(r.height))).unwrap();
        assert_eq!(
            reoffer(&node, dir.path(), Some(1), Some(1), Some(false))
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
