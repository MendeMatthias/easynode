//! Twin-freeze recovery: free a GPU-less trusted mirror stuck on a losing
//! one-block twin.
//!
//! **REMOVE WHEN 0.35 SHIPS.** This works around two engine bugs that
//! btxchain/btx#280 and #285 fix in 0.35. [`engine_needs_it`] turns it off on
//! any 0.35 engine, and `engine_pin_still_needs_twin_freeze` in the app fails
//! on the bump that reaches 0.35, so the module and its wiring get deleted
//! then (docs/btx-update-protocol.md, Part B).
//!
//! What happens. A trusted mirror (no GPU, no signer of its own) on v0.34.15
//! connects one block of a 1-2 block race, and the signers then sign the
//! other branch. Two engine paths can keep it on its losing block for hours,
//! while headers keep coming and `signed_frontier.blocks_behind` grows with
//! `on_active_chain=false`:
//!
//! - **#280, dual quorum.** Two signed candidates on incomparable branches
//!   make `FindUniqueCompetingAttestedIndex` fail closed on every call. The
//!   engine logs `dual-quorum incomparable siblings heights A/B` every second.
//!   Lars's archive, 26 Oct, 259,363 `ff8322…`, 2.5 hours.
//! - **#285, header-only skip.** The network block at our height was dropped
//!   HEADER_ONLY and is never asked for again. The engine logs
//!   `select=root_header_only_skip` every 30 seconds and a "Convergence note:
//!   tip-critical block … is not in flight". Lars's archive, 27 Oct, 259,983
//!   `dcc8d8d1…`, six hours and 255 signed blocks.
//!
//! The fix Jpp gave and Lars confirmed on both: `invalidateblock <our tip>`.
//! The tip drops one block, the engine follows the signed branch and catches
//! up within a minute. If it does not move, `reconsiderblock <same hash>`
//! puts everything back as it was.
//!
//! This module does that once per stuck block, and only when every one of
//! these holds ([`TwinFreeze::decide`]):
//! - the engine is older than 0.35 and the node runs as a trusted mirror
//!   without a signing key of its own, past initial block download;
//! - the tip has not changed for [`STUCK`];
//! - the signed frontier is [`MIN_BEHIND`] or more blocks ahead and NOT on
//!   our active chain;
//! - the engine's log shows one of the two freezes at our tip's height
//!   ([`log_shape`]);
//! - and, read from the node just before acting ([`verify`]): our tip is
//!   still the same block, the frontier's chain has a different block at our
//!   height, and that block has the same parent as ours. So the block set
//!   aside is never on the signed frontier, and only ever a one-block twin.
//!
//! Then it watches for [`ADVANCE_WAIT`]: a higher tip on another block is a
//! recovery; otherwise it puts the block back. Every step goes to the log,
//! and into a small record in the data folder ([`RECORD_FILE`]) that the
//! status card reads and that a restart mid-watch picks up again.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::node_api::AttestedTip;
use crate::rpc::Rpc;

/// The one switch. Off, nothing here reads or sends anything.
pub const ENABLED: bool = true;
/// The tip must have stood still this long. A normal race settles within a
/// block or two (about 90 seconds each).
pub const STUCK: Duration = Duration::from_secs(5 * 60);
/// The signed frontier must be at least this far ahead. At 0 or 1 the mirror
/// is level with the signers (Jpp, 26 Oct: "should be 0 or 1 at the tip").
pub const MIN_BEHIND: i64 = 2;
/// After the twin is set aside, the node has this long to connect a block at
/// its old height. Lars's archive took under a minute both times.
pub const ADVANCE_WAIT: Duration = Duration::from_secs(3 * 60);
/// At least this long between two attempts, on different twins.
pub const COOLDOWN: Duration = Duration::from_secs(30 * 60);
/// The frontier's headers walked down to our height at most. #285 was 255.
pub const MAX_WALK: u64 = 2_000;
/// The record in the data folder.
pub const RECORD_FILE: &str = "easynode-twin-freeze.json";
/// How long the status card mentions a finished recovery.
pub const NOTE_FOR_SECS: u64 = 24 * 3600;

/// False on a 0.35 or later engine, and on a tag that does not read as a
/// version: then this module does nothing.
pub fn engine_needs_it(tag: &str) -> bool {
    let head = tag.trim().split('-').next().unwrap_or("");
    let digits = head.strip_prefix('v').unwrap_or(head);
    let mut parts = digits.split('.').map(|p| {
        let n: String = p.chars().take_while(char::is_ascii_digit).collect();
        n.parse::<u64>().ok()
    });
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(major), Some(minor)) => (major, minor) < (0, 35),
        _ => false,
    }
}

/// Which of the two freezes the engine's log shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shape {
    /// btxchain/btx#280.
    DualQuorum,
    /// btxchain/btx#285.
    HeaderOnlySkip,
}

impl Shape {
    fn label(self) -> &'static str {
        match self {
            Shape::DualQuorum => "dual-quorum fail-closed, btxchain/btx#280",
            Shape::HeaderOnlySkip => "header-only skip, btxchain/btx#285",
        }
    }
}

const DUAL_QUORUM: &str = "dual-quorum incomparable siblings heights ";
const ROOT_SKIP: &str = "select=root_header_only_skip";
const CONVERGENCE: &str = "Convergence note: tip-critical block";
const NOT_IN_FLIGHT: &str = "not in flight";

/// The freeze the log tail shows at `tip_height`, if any. A line about
/// another height does not count: it is an older race.
pub fn log_shape(tail: &str, tip_height: u64) -> Option<Shape> {
    let at_tip = |line: &str, key: &str| {
        line.split_whitespace()
            .any(|w| w.strip_prefix(key).and_then(|n| n.parse::<u64>().ok()) == Some(tip_height))
    };
    let mut skip = false;
    for line in tail.lines() {
        if let Some(rest) = line.split(DUAL_QUORUM).nth(1) {
            let pair = rest.split_whitespace().next().unwrap_or("");
            if pair
                .split('/')
                .any(|n| n.parse::<u64>().ok() == Some(tip_height))
            {
                return Some(Shape::DualQuorum);
            }
        }
        if (line.contains(ROOT_SKIP) && at_tip(line, "tip="))
            || (line.contains(CONVERGENCE)
                && line.contains(NOT_IN_FLIGHT)
                && at_tip(line, "height="))
        {
            skip = true;
        }
    }
    skip.then_some(Shape::HeaderOnlySkip)
}

/// What one refresher tick saw.
#[derive(Debug, Clone)]
pub struct Seen<'a> {
    pub now: Instant,
    /// [`engine_needs_it`] for the engine that runs.
    pub engine_needs_it: bool,
    pub trusted_mirror: bool,
    pub own_signing_key: bool,
    pub initial_block_download: bool,
    pub blocks: u64,
    pub tip_hash: &'a str,
    pub frontier: Option<&'a AttestedTip>,
    /// [`log_shape`] of a recent log tail at `blocks`.
    pub shape: Option<Shape>,
}

/// Why a tick does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Off,
    NotMirror,
    OwnSigner,
    Syncing,
    /// The frontier is fewer than [`MIN_BEHIND`] ahead, or unknown.
    NotBehind,
    /// The frontier is on our chain (or the node did not say).
    FrontierOnOurChain,
    /// The tip changed within [`STUCK`].
    Moving,
    /// The log shows neither freeze at our height.
    NoLogSign,
    /// This block was already handled once this run.
    AlreadyTried,
    Cooldown,
    /// A block is set aside and the node has not had [`ADVANCE_WAIT`] yet.
    Watching,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Wait(Why),
    /// Check the chain ([`verify`]), then set `twin` aside.
    Check {
        twin: String,
        height: u64,
        frontier_hash: String,
        shape: Shape,
    },
    /// The node moved past the twin's height on another block.
    Recovered {
        twin: String,
        height: u64,
        now: u64,
    },
    /// It did not: put the twin back.
    PutBack {
        twin: String,
        height: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Watch {
    twin: String,
    height: u64,
    shape: Shape,
    since: Instant,
}

/// The state for one node run.
#[derive(Debug, Clone, Default)]
pub struct TwinFreeze {
    /// The tip, and since when it has stood still.
    still: Option<(String, Instant)>,
    /// The twins handled this run, set aside or refused.
    tried: Vec<String>,
    last_try: Option<Instant>,
    watching: Option<Watch>,
    /// The record was read at the first tick.
    loaded: bool,
    /// `reconsiderblock` failures in a row, so the log says the first only.
    put_back_failures: u32,
}

impl TwinFreeze {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pure apart from the tip clock: what this tick should do.
    pub fn decide(&mut self, s: &Seen) -> Decision {
        let moved = self.still.as_ref().map(|(h, _)| h.as_str()) != Some(s.tip_hash);
        if moved {
            self.still = Some((s.tip_hash.to_string(), s.now));
        }
        if let Some(w) = &self.watching {
            if s.blocks >= w.height && s.tip_hash != w.twin {
                return Decision::Recovered {
                    twin: w.twin.clone(),
                    height: w.height,
                    now: s.blocks,
                };
            }
            if s.now.saturating_duration_since(w.since) >= ADVANCE_WAIT {
                return Decision::PutBack {
                    twin: w.twin.clone(),
                    height: w.height,
                };
            }
            return Decision::Wait(Why::Watching);
        }
        if !ENABLED || !s.engine_needs_it {
            return Decision::Wait(Why::Off);
        }
        if !s.trusted_mirror {
            return Decision::Wait(Why::NotMirror);
        }
        if s.own_signing_key {
            return Decision::Wait(Why::OwnSigner);
        }
        if s.initial_block_download || s.blocks == 0 || s.tip_hash.is_empty() {
            return Decision::Wait(Why::Syncing);
        }
        let Some(f) = s.frontier else {
            return Decision::Wait(Why::NotBehind);
        };
        let Some(frontier_hash) = f.hash.as_deref().filter(|h| !h.is_empty()) else {
            return Decision::Wait(Why::NotBehind);
        };
        if f.blocks_behind.unwrap_or(0) < MIN_BEHIND {
            return Decision::Wait(Why::NotBehind);
        }
        if f.on_active_chain != Some(false) {
            return Decision::Wait(Why::FrontierOnOurChain);
        }
        let still_for = self
            .still
            .as_ref()
            .map(|(_, t)| s.now.saturating_duration_since(*t))
            .unwrap_or_default();
        if still_for < STUCK {
            return Decision::Wait(Why::Moving);
        }
        let Some(shape) = s.shape else {
            return Decision::Wait(Why::NoLogSign);
        };
        if self.tried.iter().any(|t| t == s.tip_hash) {
            return Decision::Wait(Why::AlreadyTried);
        }
        if self
            .last_try
            .is_some_and(|t| s.now.saturating_duration_since(t) < COOLDOWN)
        {
            return Decision::Wait(Why::Cooldown);
        }
        Decision::Check {
            twin: s.tip_hash.to_string(),
            height: s.blocks,
            frontier_hash: frontier_hash.to_string(),
            shape,
        }
    }

    fn set_aside(&mut self, twin: &str, height: u64, shape: Shape, now: Instant) {
        self.tried.push(twin.to_string());
        self.last_try = Some(now);
        self.watching = Some(Watch {
            twin: twin.to_string(),
            height,
            shape,
            since: now,
        });
    }

    fn refused(&mut self, twin: &str, now: Instant) {
        self.tried.push(twin.to_string());
        self.last_try = Some(now);
    }

    fn done(&mut self) {
        self.watching = None;
    }

    /// The block set aside and not yet concluded, if any.
    pub fn watching(&self) -> Option<(&str, u64)> {
        self.watching.as_ref().map(|w| (w.twin.as_str(), w.height))
    }

    /// Whether this tick should read the tip and the log at all: the first
    /// tick (a record may say a block is set aside), while watching, or when
    /// the cheap facts already point at a twin freeze. Every other node, every
    /// other tick, pays nothing.
    pub fn worth_a_look(&self, f: &NodeFacts) -> bool {
        !self.loaded
            || self.watching.is_some()
            || (ENABLED
                && f.trusted_mirror
                && !f.own_signing_key
                && !f.initial_block_download
                && engine_needs_it(f.engine_tag)
                && f.frontier.is_some_and(|t| {
                    t.blocks_behind.unwrap_or(0) >= MIN_BEHIND && t.on_active_chain == Some(false)
                }))
    }
}

/// What the app's refresher already knows this tick.
#[derive(Debug, Clone)]
pub struct NodeFacts<'a> {
    pub trusted_mirror: bool,
    pub own_signing_key: bool,
    /// The engine's release tag (`v0.34.15`).
    pub engine_tag: &'a str,
    pub initial_block_download: bool,
    pub blocks: u64,
    pub frontier: Option<&'a AttestedTip>,
}

/// The refresher's whole step: when [`TwinFreeze::worth_a_look`], read the
/// tip hash and the log tail (`log_tail` is only called then) and [`tick`].
pub async fn run(
    rpc: &dyn Rpc,
    datadir: &Path,
    tf: &mut TwinFreeze,
    f: &NodeFacts<'_>,
    log_tail: impl FnOnce() -> String,
    now: Instant,
    now_unix: u64,
) -> Vec<String> {
    if !tf.worth_a_look(f) {
        return Vec::new();
    }
    let Ok(best) = rpc.call("getbestblockhash", json!([])).await else {
        return Vec::new();
    };
    let tip_hash = best.as_str().unwrap_or("").to_string();
    let shape = log_shape(&log_tail(), f.blocks);
    let s = Seen {
        now,
        engine_needs_it: engine_needs_it(f.engine_tag),
        trusted_mirror: f.trusted_mirror,
        own_signing_key: f.own_signing_key,
        initial_block_download: f.initial_block_download,
        blocks: f.blocks,
        tip_hash: &tip_hash,
        frontier: f.frontier,
        shape,
    };
    tick(rpc, datadir, tf, &s, now_unix).await
}

/// Why [`verify`] would not let a block be set aside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    TipMoved,
    /// The signed frontier's chain runs through our tip.
    OnSignedFrontier,
    /// The two branches part lower than our tip's parent.
    NotOneBlockTwin,
    /// The frontier header is not above our tip, or too far to walk.
    FrontierOutOfReach,
}

impl Refusal {
    fn say(&self) -> &'static str {
        match self {
            Refusal::TipMoved => "the tip moved before the check",
            Refusal::OnSignedFrontier => "the signed frontier runs through this block",
            Refusal::NotOneBlockTwin => "the branches part lower than one block",
            Refusal::FrontierOutOfReach => "the signed frontier's headers are out of reach",
        }
    }
}

async fn header(rpc: &dyn Rpc, hash: &str) -> AppResult<(u64, String)> {
    let v = rpc.call("getblockheader", json!([hash])).await?;
    let height = v.get("height").and_then(|h| h.as_u64()).unwrap_or(0);
    let prev = v
        .get("previousblockhash")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    Ok((height, prev))
}

/// Reads the node: may `twin`, at `height`, be set aside? Only when it is
/// still the tip, and the signed frontier's chain has another block at
/// `height` with the same parent.
pub async fn verify(
    rpc: &dyn Rpc,
    twin: &str,
    height: u64,
    frontier_hash: &str,
) -> AppResult<Result<(), Refusal>> {
    let best = rpc.call("getbestblockhash", json!([])).await?;
    if best.as_str() != Some(twin) {
        return Ok(Err(Refusal::TipMoved));
    }
    let (twin_height, twin_parent) = header(rpc, twin).await?;
    if twin_height != height || twin_parent.is_empty() {
        return Ok(Err(Refusal::TipMoved));
    }
    let (mut h, mut prev) = header(rpc, frontier_hash).await?;
    let mut at = frontier_hash.to_string();
    if h <= height || h - height > MAX_WALK {
        return Ok(Err(Refusal::FrontierOutOfReach));
    }
    while h > height {
        if prev.is_empty() {
            return Ok(Err(Refusal::FrontierOutOfReach));
        }
        at = prev;
        (h, prev) = header(rpc, &at).await?;
    }
    if h != height {
        return Ok(Err(Refusal::FrontierOutOfReach));
    }
    if at == twin {
        return Ok(Err(Refusal::OnSignedFrontier));
    }
    if prev != twin_parent {
        return Ok(Err(Refusal::NotOneBlockTwin));
    }
    Ok(Ok(()))
}

/// What happened to the last twin, kept in the data folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Watching,
    Recovered,
    PutBack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub twin: String,
    pub height: u64,
    pub shape: Shape,
    pub state: State,
    /// When the state was written.
    pub at_unix: u64,
}

pub fn record_path(datadir: &Path) -> PathBuf {
    datadir.join(RECORD_FILE)
}

pub fn read_record(datadir: &Path) -> Option<Record> {
    let text = std::fs::read_to_string(record_path(datadir)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_record(datadir: &Path, r: &Record) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(r).map_err(std::io::Error::other)?;
    crate::fsx::atomic_write(&record_path(datadir), &bytes)
}

fn short(hash: &str) -> &str {
    &hash[..hash.len().min(12)]
}

/// One refresher tick: decide, read and act. Returns the lines for the log.
/// The only commands it sends that change anything are `invalidateblock` and
/// `reconsiderblock`, each on the one block [`verify`] approved.
pub async fn tick(
    rpc: &dyn Rpc,
    datadir: &Path,
    tf: &mut TwinFreeze,
    s: &Seen<'_>,
    now_unix: u64,
) -> Vec<String> {
    let mut out = Vec::new();
    if !tf.loaded {
        tf.loaded = true;
        // A run that ended while a block was set aside: the node gets a
        // full ADVANCE_WAIT from now (it may itself have just restarted),
        // and the block goes back if it has not moved on by then.
        if let Some(r) = read_record(datadir).filter(|r| r.state == State::Watching) {
            tf.set_aside(&r.twin, r.height, r.shape, s.now);
            out.push(format!(
                "picked up the set-aside block {} at {} from the last run",
                short(&r.twin),
                r.height
            ));
        }
    }
    let keep = |r: Record, out: &mut Vec<String>| {
        if let Err(e) = write_record(datadir, &r) {
            out.push(format!("could not write {RECORD_FILE}: {e}"));
        }
    };
    match tf.decide(s) {
        Decision::Wait(_) => {}
        Decision::Check {
            twin,
            height,
            frontier_hash,
            shape,
        } => match verify(rpc, &twin, height, &frontier_hash).await {
            Ok(Ok(())) => {
                // The record goes first: an app that dies right after the
                // invalidateblock still knows to put the block back.
                let record = Record {
                    twin: twin.clone(),
                    height,
                    shape,
                    state: State::Watching,
                    at_unix: now_unix,
                };
                if let Err(e) = write_record(datadir, &record) {
                    tf.refused(&twin, s.now);
                    out.push(format!(
                        "could not write {RECORD_FILE} ({e}), so left block {} alone.",
                        short(&twin)
                    ));
                    return out;
                }
                // Watched even when the call fails: a timeout may still have
                // set it aside, and reconsiderblock on a valid block does
                // nothing.
                tf.set_aside(&twin, height, shape, s.now);
                match rpc.call("invalidateblock", json!([twin])).await {
                    Ok(_) => out.push(format!(
                        "stuck {} min on block {} at {height} while the signed frontier is \
                         {} blocks ahead on another branch ({}). Set it aside with \
                         invalidateblock; it goes back with reconsiderblock if the node \
                         does not move on within {} min.",
                        STUCK.as_secs() / 60,
                        twin,
                        s.frontier.and_then(|f| f.blocks_behind).unwrap_or(0),
                        shape.label(),
                        ADVANCE_WAIT.as_secs() / 60,
                    )),
                    Err(e) => out.push(format!(
                        "invalidateblock {} failed: {e}. Watching anyway; it is put back \
                         if the node does not move on.",
                        short(&twin)
                    )),
                }
            }
            Ok(Err(why)) => {
                tf.refused(&twin, s.now);
                out.push(format!(
                    "looked stuck on block {} at {height}, left it alone: {}",
                    short(&twin),
                    why.say()
                ));
            }
            Err(e) => {
                // A read failed: try again after the cooldown.
                tf.last_try = Some(s.now);
                out.push(format!("could not check the chain: {e}"));
            }
        },
        Decision::Recovered { twin, height, now } => {
            let shape = tf.watching.as_ref().map(|w| w.shape);
            tf.done();
            out.push(format!(
                "recovered: the node left block {} at {height} and is at {now} on the signed \
                 branch.",
                short(&twin)
            ));
            keep(
                Record {
                    twin,
                    height,
                    shape: shape.unwrap_or(Shape::DualQuorum),
                    state: State::Recovered,
                    at_unix: now_unix,
                },
                &mut out,
            );
        }
        Decision::PutBack { twin, height } => {
            match rpc.call("reconsiderblock", json!([twin])).await {
                // The node does not know the block at all (its data folder was
                // made again since): nothing is set aside, nothing to put back.
                Err(AppError::Rpc { code: -5, .. }) => {
                    tf.done();
                    let _ = std::fs::remove_file(record_path(datadir));
                    out.push(format!(
                        "the node no longer knows block {} at {height}: nothing to put back.",
                        short(&twin)
                    ));
                }
                Ok(_) => {
                    tf.put_back_failures = 0;
                    let shape = tf.watching.as_ref().map(|w| w.shape);
                    tf.done();
                    out.push(format!(
                        "the node did not move on in {} min: put block {} at {height} back \
                         with reconsiderblock. Not trying it again this run.",
                        ADVANCE_WAIT.as_secs() / 60,
                        short(&twin)
                    ));
                    keep(
                        Record {
                            twin,
                            height,
                            shape: shape.unwrap_or(Shape::DualQuorum),
                            state: State::PutBack,
                            at_unix: now_unix,
                        },
                        &mut out,
                    );
                }
                // Tried again next tick; said once.
                Err(e) => {
                    tf.put_back_failures += 1;
                    if tf.put_back_failures == 1 {
                        out.push(format!(
                            "reconsiderblock {} failed: {e}. Trying again every few seconds.",
                            short(&twin)
                        ));
                    }
                }
            }
        }
    }
    out
}

/// The status card's line, and whether it needs the person: a step being
/// watched, or one that ended within [`NOTE_FOR_SECS`].
pub fn note(record: Option<&Record>, now_unix: u64) -> Option<(String, bool)> {
    let r = record?;
    if r.state != State::Watching && now_unix.saturating_sub(r.at_unix) > NOTE_FOR_SECS {
        return None;
    }
    let h = r.height;
    Some(match r.state {
        State::Watching => (
            format!(
                "Your node was stuck on the losing side of a one-block race at height {h}, a \
                 known 0.34.15 freeze. easyNode set that block aside and is checking that the \
                 node follows the signed chain."
            ),
            false,
        ),
        State::Recovered => (
            format!(
                "Your node got stuck on the losing side of a one-block race at height {h}, a \
                 known 0.34.15 freeze. easyNode set that block aside and the node follows the \
                 signed chain again."
            ),
            false,
        ),
        State::PutBack => (
            format!(
                "Your node looked stuck on the losing side of a one-block race at height {h}. \
                 easyNode set that block aside, the node did not move on, so it put the block \
                 back exactly as it was. If it stays stuck, ask in the BTX group."
            ),
            true,
        ),
    })
}

pub fn status_note(datadir: &Path, now_unix: u64) -> Option<(String, bool)> {
    note(read_record(datadir).as_ref(), now_unix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    /// A block tree. `invalidateblock` drops the block and everything above
    /// it from the active chain, the way the engine does; it does not pick a
    /// new branch: the test moves the tip to say what the engine did.
    #[derive(Default)]
    struct Tree {
        /// hash -> (height, parent)
        blocks: HashMap<String, (u64, String)>,
        active: Vec<String>,
        invalid: HashSet<String>,
        calls: Vec<String>,
        fail_invalidate: bool,
    }

    struct Fake(Mutex<Tree>);

    impl Fake {
        fn calls(&self) -> Vec<String> {
            self.0.lock().unwrap().calls.clone()
        }
        fn changes(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter(|c| c.starts_with("invalidateblock") || c.starts_with("reconsiderblock"))
                .collect()
        }
        fn tip(&self) -> (u64, String) {
            let t = self.0.lock().unwrap();
            let h = t.active.last().unwrap().clone();
            (t.blocks[&h].0, h)
        }
        /// The engine follows `branch`'s chain up to its tip.
        fn follow(&self, top: &str) {
            let mut t = self.0.lock().unwrap();
            let mut chain = vec![top.to_string()];
            let mut cur = top.to_string();
            while let Some((_, p)) = t.blocks.get(&cur) {
                if p.is_empty() {
                    break;
                }
                chain.push(p.clone());
                cur = p.clone();
            }
            chain.reverse();
            t.active = chain;
        }
    }

    #[async_trait]
    impl Rpc for Fake {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            let mut t = self.0.lock().unwrap();
            let arg = params
                .get(0)
                .and_then(|a| a.as_str())
                .unwrap_or("")
                .to_string();
            t.calls.push(format!("{method} {arg}"));
            match method {
                "getbestblockhash" => Ok(json!(t.active.last().unwrap())),
                "getblockheader" => {
                    let (h, p) = t.blocks.get(&arg).cloned().ok_or(AppError::Rpc {
                        code: -5,
                        message: "Block not found".into(),
                    })?;
                    let mut v = json!({ "hash": arg, "height": h });
                    if !p.is_empty() {
                        v["previousblockhash"] = json!(p);
                    }
                    Ok(v)
                }
                "invalidateblock" => {
                    if t.fail_invalidate {
                        return Err(AppError::Http("timed out".into()));
                    }
                    t.invalid.insert(arg.clone());
                    if let Some(i) = t.active.iter().position(|b| *b == arg) {
                        t.active.truncate(i);
                    }
                    Ok(Value::Null)
                }
                "reconsiderblock" => {
                    if !t.blocks.contains_key(&arg) {
                        return Err(AppError::Rpc {
                            code: -5,
                            message: "Block not found".into(),
                        });
                    }
                    t.invalid.remove(&arg);
                    Ok(Value::Null)
                }
                m => panic!("unscripted {m}"),
            }
        }
    }

    /// A shared chain to `fork`, then the network branch `n<h>` up to
    /// `network_top` and our one-block twin `t<fork+1>`, which is our tip.
    fn race(fork: u64, network_top: u64) -> (Fake, String) {
        let mut t = Tree::default();
        let base = fork - 3;
        let mut prev = String::new();
        for h in base..=fork {
            let hash = format!("c{h}");
            t.blocks.insert(hash.clone(), (h, prev.clone()));
            t.active.push(hash.clone());
            prev = hash;
        }
        let twin = format!("t{}", fork + 1);
        t.blocks.insert(twin.clone(), (fork + 1, prev.clone()));
        t.active.push(twin.clone());
        let mut p = prev;
        for h in fork + 1..=network_top {
            let hash = format!("n{h}");
            t.blocks.insert(hash.clone(), (h, p));
            p = hash;
        }
        (Fake(Mutex::new(t)), twin)
    }

    fn frontier(top: u64, behind: i64) -> AttestedTip {
        AttestedTip {
            height: Some(top),
            blocks_behind: Some(behind),
            on_active_chain: Some(false),
            hash: Some(format!("n{top}")),
        }
    }

    fn seen<'a>(
        now: Instant,
        fake_tip: &'a (u64, String),
        f: &'a AttestedTip,
        shape: Option<Shape>,
    ) -> Seen<'a> {
        Seen {
            now,
            engine_needs_it: true,
            trusted_mirror: true,
            own_signing_key: false,
            initial_block_download: false,
            blocks: fake_tip.0,
            tip_hash: &fake_tip.1,
            frontier: Some(f),
            shape,
        }
    }

    const MIN: Duration = Duration::from_secs(60);

    // The 26 Oct #280 lines from Lars's archive, at our tip's height.
    fn dual_quorum_log(h: u64) -> String {
        format!(
            "2026-10-26T10:17:03Z UpdateTip: new best=ff8322 height={h}\n\
             2026-10-26T10:18:01Z matmul: dual-quorum incomparable siblings heights {}/{h} tip=ff8322 \
             fail-closed (no unique competing attested index)\n",
            h - 5
        )
    }

    // The 27 Oct #285 lines.
    fn header_only_log(h: u64) -> String {
        format!(
            "01:08:32 MatMul admission HEADER_ONLY for block hash=50c61fcb height={h} from peer=6\n\
             01:12:23 Block download root-first: peer=4 tip={h} last_common={} lowest_missing=50c61fcb \
             missing_height={h} select=root_header_only_skip clamp=no reason=ok in_flight_global=1\n\
             01:14:23 Convergence note: tip-critical block 50c61fcb height={h} is the selected root but \
             is not in flight. This is a local scheduling or fork-choice hold\n",
            h - 1
        )
    }

    /// Runs the detector over a frozen tip until it acts, then lets the
    /// engine follow `follow` (or not) and runs it until it concludes.
    async fn freeze_and_recover(
        fake: &Fake,
        f: &AttestedTip,
        log: &str,
        follow: Option<&str>,
    ) -> (Vec<String>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut tf = TwinFreeze::new();
        let t0 = Instant::now();
        let mut lines = Vec::new();
        for m in 0..=5 {
            let tip = fake.tip();
            let shape = log_shape(log, tip.0);
            let s = seen(t0 + MIN * m, &tip, f, shape);
            lines.extend(tick(fake, dir.path(), &mut tf, &s, 1_000 + m as u64).await);
        }
        if let Some(top) = follow {
            fake.follow(top);
        }
        for m in 6..=10 {
            let tip = fake.tip();
            let s = seen(t0 + MIN * m, &tip, f, log_shape(log, tip.0));
            lines.extend(tick(fake, dir.path(), &mut tf, &s, 1_000 + m as u64).await);
        }
        (lines, dir)
    }

    #[tokio::test]
    async fn dual_quorum_freeze_280_is_set_aside_and_the_node_recovers() {
        let (fake, twin) = race(259_362, 259_365);
        let f = frontier(259_365, 2);
        let (lines, dir) =
            freeze_and_recover(&fake, &f, &dual_quorum_log(259_363), Some("n259365")).await;
        assert_eq!(fake.changes(), vec![format!("invalidateblock {twin}")]);
        assert_eq!(fake.tip().0, 259_365);
        assert!(
            lines.iter().any(|l| l.contains("btxchain/btx#280")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.starts_with("recovered")),
            "{lines:?}"
        );
        let r = read_record(dir.path()).unwrap();
        assert_eq!(
            (r.state, r.height, r.shape),
            (State::Recovered, 259_363, Shape::DualQuorum)
        );
    }

    #[tokio::test]
    async fn header_only_freeze_285_is_set_aside_and_the_node_recovers() {
        // 27 Oct: stuck at 259,983 while the signers went 255 blocks on.
        let (fake, twin) = race(259_982, 260_238);
        let f = frontier(260_238, 255);
        let (lines, dir) =
            freeze_and_recover(&fake, &f, &header_only_log(259_983), Some("n260238")).await;
        assert_eq!(fake.changes(), vec![format!("invalidateblock {twin}")]);
        assert!(
            lines.iter().any(|l| l.contains("btxchain/btx#285")),
            "{lines:?}"
        );
        assert_eq!(read_record(dir.path()).unwrap().state, State::Recovered);
    }

    #[tokio::test]
    async fn a_node_that_does_not_move_on_gets_its_block_back_once() {
        let (fake, twin) = race(259_362, 259_365);
        let f = frontier(259_365, 2);
        let (lines, dir) = freeze_and_recover(&fake, &f, &dual_quorum_log(259_363), None).await;
        assert_eq!(
            fake.changes(),
            vec![
                format!("invalidateblock {twin}"),
                format!("reconsiderblock {twin}")
            ]
        );
        assert!(lines.iter().any(|l| l.contains("put block")), "{lines:?}");
        let r = read_record(dir.path()).unwrap();
        assert_eq!(r.state, State::PutBack);
        assert!(
            note(Some(&r), r.at_unix).unwrap().1,
            "a put-back asks for attention"
        );
    }

    #[tokio::test]
    async fn a_put_back_twin_is_not_tried_again_this_run() {
        let (fake, twin) = race(100, 104);
        let f = frontier(104, 3);
        let dir = tempfile::tempdir().unwrap();
        let mut tf = TwinFreeze::new();
        let t0 = Instant::now();
        let log = dual_quorum_log(101);
        for m in 0..120 {
            // The engine goes back to the twin after reconsiderblock.
            if !fake.0.lock().unwrap().active.contains(&twin)
                && !fake.0.lock().unwrap().invalid.contains(&twin)
            {
                fake.follow(&twin);
            }
            let tip = fake.tip();
            let s = seen(t0 + MIN * m, &tip, &f, log_shape(&log, tip.0));
            tick(&fake, dir.path(), &mut tf, &s, m as u64).await;
        }
        assert_eq!(fake.changes().len(), 2, "{:?}", fake.changes());
    }

    #[tokio::test]
    async fn never_sets_aside_a_block_the_signed_frontier_runs_through() {
        // The engine says the frontier is off our chain, but its header
        // chain runs through our tip (a stale or racing answer).
        let (fake, twin) = race(100, 101);
        {
            let mut t = fake.0.lock().unwrap();
            t.blocks.insert("s102".into(), (102, twin.clone()));
            t.blocks.insert("s103".into(), (103, "s102".into()));
        }
        let f = AttestedTip {
            hash: Some("s103".into()),
            ..frontier(103, 2)
        };
        let (lines, _dir) = freeze_and_recover(&fake, &f, &dual_quorum_log(101), None).await;
        assert!(fake.changes().is_empty(), "{:?}", fake.changes());
        assert!(
            lines
                .iter()
                .any(|l| l.contains("signed frontier runs through")),
            "{lines:?}"
        );
    }

    #[tokio::test]
    async fn a_fork_deeper_than_one_block_is_left_alone() {
        // Our tip t101 sits on c100; the frontier branch parts at c99.
        let (fake, _twin) = race(100, 101);
        {
            let mut t = fake.0.lock().unwrap();
            t.blocks.insert("d100".into(), (100, "c99".into()));
            t.blocks.insert("d101".into(), (101, "d100".into()));
            t.blocks.insert("d102".into(), (102, "d101".into()));
            t.blocks.insert("d103".into(), (103, "d102".into()));
        }
        let f = AttestedTip {
            hash: Some("d103".into()),
            ..frontier(103, 2)
        };
        let (lines, _dir) = freeze_and_recover(&fake, &f, &dual_quorum_log(101), None).await;
        assert!(fake.changes().is_empty());
        assert!(
            lines.iter().any(|l| l.contains("lower than one block")),
            "{lines:?}"
        );
    }

    #[test]
    fn every_strict_condition_must_hold() {
        let (fake, _) = race(100, 104);
        let tip = fake.tip();
        let log = dual_quorum_log(101);
        let good = frontier(104, 3);
        let one_behind = frontier(104, 1);
        let on_chain = AttestedTip {
            on_active_chain: Some(true),
            ..frontier(104, 3)
        };
        let unsaid = AttestedTip {
            on_active_chain: None,
            ..frontier(104, 3)
        };
        type Tweak = fn(&mut Seen);
        let cases: Vec<(&str, &AttestedTip, Option<Shape>, Tweak)> = vec![
            ("one behind", &one_behind, log_shape(&log, 101), |_| {}),
            (
                "frontier on our chain",
                &on_chain,
                log_shape(&log, 101),
                |_| {},
            ),
            (
                "frontier chain unsaid",
                &unsaid,
                log_shape(&log, 101),
                |_| {},
            ),
            ("no log line", &good, None, |_| {}),
            (
                "log line at another height",
                &good,
                log_shape(&log, 102),
                |_| {},
            ),
            ("not a mirror", &good, log_shape(&log, 101), |s| {
                s.trusted_mirror = false
            }),
            ("own signer", &good, log_shape(&log, 101), |s| {
                s.own_signing_key = true
            }),
            ("0.35 engine", &good, log_shape(&log, 101), |s| {
                s.engine_needs_it = false
            }),
            ("syncing", &good, log_shape(&log, 101), |s| {
                s.initial_block_download = true
            }),
        ];
        for (name, f, shape, tweak) in cases {
            let mut tf = TwinFreeze::new();
            let t0 = Instant::now();
            for m in 0..=30 {
                let mut s = seen(t0 + MIN * m, &tip, f, shape);
                tweak(&mut s);
                assert!(
                    matches!(tf.decide(&s), Decision::Wait(_)),
                    "{name}: acted at minute {m}"
                );
            }
        }
        // And with all of them, not before STUCK.
        let mut tf = TwinFreeze::new();
        let t0 = Instant::now();
        let shape = log_shape(&log, 101);
        assert_eq!(
            tf.decide(&seen(t0, &tip, &good, shape)),
            Decision::Wait(Why::Moving)
        );
        assert_eq!(
            tf.decide(&seen(
                t0 + STUCK - Duration::from_secs(1),
                &tip,
                &good,
                shape
            )),
            Decision::Wait(Why::Moving)
        );
        assert!(matches!(
            tf.decide(&seen(t0 + STUCK, &tip, &good, shape)),
            Decision::Check { .. }
        ));
    }

    #[tokio::test]
    async fn a_tip_that_moved_before_the_check_is_left_alone() {
        let (fake, _) = race(100, 104);
        let r = verify(&fake, "c100", 100, "n104").await.unwrap();
        assert_eq!(r, Err(Refusal::TipMoved));
        assert!(fake.changes().is_empty());
    }

    #[tokio::test]
    async fn a_restart_mid_watch_concludes_from_the_record() {
        // Moved on while the app was down: recovered, nothing sent.
        let (fake, twin) = race(100, 104);
        let dir = tempfile::tempdir().unwrap();
        let rec = |state| Record {
            twin: twin.clone(),
            height: 101,
            shape: Shape::HeaderOnlySkip,
            state,
            at_unix: 5,
        };
        write_record(dir.path(), &rec(State::Watching)).unwrap();
        fake.follow("n104");
        let f = frontier(104, 0);
        let tip = fake.tip();
        let mut tf = TwinFreeze::new();
        tick(
            &fake,
            dir.path(),
            &mut tf,
            &seen(Instant::now(), &tip, &f, None),
            9,
        )
        .await;
        assert!(fake.changes().is_empty());
        assert_eq!(read_record(dir.path()).unwrap().state, State::Recovered);

        // Still one below right after the restart: it gets a full wait.
        let (fake, _) = race(100, 104);
        fake.0.lock().unwrap().active.pop();
        write_record(dir.path(), &rec(State::Watching)).unwrap();
        let tip = fake.tip();
        let mut tf = TwinFreeze::new();
        let t0 = Instant::now();
        for secs in [0, 60, 170] {
            let s = seen(t0 + Duration::from_secs(secs), &tip, &f, None);
            tick(&fake, dir.path(), &mut tf, &s, 9).await;
        }
        assert!(fake.changes().is_empty(), "too early: {:?}", fake.changes());
        // ...and if it catches up within it, the fix stays.
        fake.follow("n104");
        let up = fake.tip();
        let s = seen(t0 + Duration::from_secs(175), &up, &f, None);
        tick(&fake, dir.path(), &mut tf, &s, 9).await;
        assert!(fake.changes().is_empty());
        assert_eq!(read_record(dir.path()).unwrap().state, State::Recovered);

        // Still one below after the full wait: put back.
        let (fake, _) = race(100, 104);
        fake.0.lock().unwrap().active.pop();
        write_record(dir.path(), &rec(State::Watching)).unwrap();
        let tip = fake.tip();
        let mut tf = TwinFreeze::new();
        for secs in [0, ADVANCE_WAIT.as_secs()] {
            let s = seen(t0 + Duration::from_secs(secs), &tip, &f, None);
            tick(&fake, dir.path(), &mut tf, &s, 9).await;
        }
        assert_eq!(fake.changes(), vec![format!("reconsiderblock {twin}")]);
        assert_eq!(read_record(dir.path()).unwrap().state, State::PutBack);
    }

    #[tokio::test]
    async fn a_healthy_node_pays_one_read_per_run_and_a_frozen_one_is_freed() {
        let (fake, twin) = race(259_982, 260_238);
        let dir = tempfile::tempdir().unwrap();
        let mut tf = TwinFreeze::new();
        let level = AttestedTip {
            on_active_chain: Some(true),
            ..frontier(259_983, 0)
        };
        let t0 = Instant::now();
        let mut tails = 0;
        for m in 0..10 {
            let f = NodeFacts {
                trusted_mirror: true,
                own_signing_key: false,
                engine_tag: "v0.34.15",
                initial_block_download: false,
                blocks: 259_983,
                frontier: Some(&level),
            };
            run(
                &fake,
                dir.path(),
                &mut tf,
                &f,
                || {
                    tails += 1;
                    String::new()
                },
                t0 + MIN * m,
                0,
            )
            .await;
        }
        assert_eq!(
            (fake.calls().len(), tails),
            (1, 1),
            "only the first tick looks"
        );

        let stuck = frontier(260_238, 255);
        let log = header_only_log(259_983);
        for m in 10..12 {
            let f = NodeFacts {
                trusted_mirror: true,
                own_signing_key: false,
                engine_tag: "v0.34.15",
                initial_block_download: false,
                blocks: fake.tip().0,
                frontier: Some(&stuck),
            };
            run(
                &fake,
                dir.path(),
                &mut tf,
                &f,
                || log.clone(),
                t0 + MIN * m,
                0,
            )
            .await;
        }
        assert_eq!(fake.changes(), vec![format!("invalidateblock {twin}")]);
    }

    #[tokio::test]
    async fn a_failed_invalidateblock_is_still_watched_and_put_back() {
        let (fake, twin) = race(259_362, 259_365);
        fake.0.lock().unwrap().fail_invalidate = true;
        let f = frontier(259_365, 2);
        let (_lines, dir) = freeze_and_recover(&fake, &f, &dual_quorum_log(259_363), None).await;
        assert_eq!(
            fake.changes(),
            vec![
                format!("invalidateblock {twin}"),
                format!("reconsiderblock {twin}")
            ]
        );
        assert_eq!(read_record(dir.path()).unwrap().state, State::PutBack);
    }

    #[tokio::test]
    async fn a_record_for_a_block_the_node_no_longer_knows_is_dropped() {
        let (fake, _) = race(100, 104);
        fake.0.lock().unwrap().active.pop();
        let dir = tempfile::tempdir().unwrap();
        write_record(
            dir.path(),
            &Record {
                twin: "gone".into(),
                height: 101,
                shape: Shape::DualQuorum,
                state: State::Watching,
                at_unix: 5,
            },
        )
        .unwrap();
        let f = frontier(104, 3);
        let tip = fake.tip();
        let mut tf = TwinFreeze::new();
        let t0 = Instant::now();
        for m in 0..5 {
            tick(
                &fake,
                dir.path(),
                &mut tf,
                &seen(t0 + MIN * m, &tip, &f, None),
                9,
            )
            .await;
        }
        assert_eq!(fake.changes(), vec!["reconsiderblock gone".to_string()]);
        assert!(read_record(dir.path()).is_none());
        assert!(tf.watching().is_none());
    }

    #[test]
    fn the_log_shapes_are_read_at_our_height_only() {
        assert_eq!(
            log_shape(&dual_quorum_log(259_363), 259_363),
            Some(Shape::DualQuorum)
        );
        // The other sibling's height counts too: it is the same pair.
        assert_eq!(
            log_shape(&dual_quorum_log(259_363), 259_358),
            Some(Shape::DualQuorum)
        );
        assert_eq!(log_shape(&dual_quorum_log(259_363), 259_364), None);
        assert_eq!(
            log_shape(&header_only_log(259_983), 259_983),
            Some(Shape::HeaderOnlySkip)
        );
        assert_eq!(log_shape(&header_only_log(259_983), 259_984), None);
        // The HEADER_ONLY admission alone is an ordinary race.
        assert_eq!(
            log_shape(
                "MatMul admission HEADER_ONLY for block hash=50c6 height=7 from peer=6",
                7
            ),
            None
        );
        assert_eq!(log_shape("", 7), None);
    }

    #[test]
    fn only_engines_before_0_35_need_it() {
        assert!(engine_needs_it("v0.34.15"));
        assert!(engine_needs_it("v0.34.15-476f3f23"));
        assert!(!engine_needs_it("v0.35.0"));
        assert!(!engine_needs_it("v0.35.0rc1"));
        assert!(!engine_needs_it("v0.35.0-rc1"));
        assert!(!engine_needs_it("v1.0.0"));
        assert!(!engine_needs_it(""));
        assert!(!engine_needs_it("custom"));
    }

    #[test]
    fn the_card_says_what_happened_for_a_day() {
        let r = Record {
            twin: "t".into(),
            height: 259_983,
            shape: Shape::HeaderOnlySkip,
            state: State::Recovered,
            at_unix: 100,
        };
        let (msg, attention) = note(Some(&r), 100 + 3600).unwrap();
        assert!(msg.contains("259983") && !attention);
        assert!(note(Some(&r), 100 + NOTE_FOR_SECS + 1).is_none());
        let watching = Record {
            state: State::Watching,
            ..r
        };
        assert!(note(Some(&watching), u64::MAX / 2).is_some());
    }
}
