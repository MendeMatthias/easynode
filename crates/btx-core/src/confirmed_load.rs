//! The one loading path for a signed snapshot, the same for setup,
//! Fast-forward and every kind of node (docs/decisions/2026-09-29-every-node-
//! starts-near-the-tip.md, section 7, steps 3 to 6).
//!
//! By the time a pair gets here, `crate::attested_snapshot` has downloaded
//! and checked it. This checks it again against the node that is about to
//! load it (its chain, its replay context, its pins), because the engine
//! checks only what it pins and nothing else:
//!
//! 1. The pair on disk passes every check again: a confirmed pair
//!    [`crate::confirmed_snapshot::check`], its height and its file's double
//!    SHA-256, the pinned pair its compiled SHA-256s; and at least one of its
//!    signatures is from a key the engine trusts (step 4's trim, worked out
//!    here). Every refusal that needs no engine happens here, before any
//!    side effect.
//! 2. Every block the app refuses (`crate::known_invalid`) is refused now,
//!    in order, as the fork check does every 30 seconds. The engine then
//!    refuses a snapshot whose base sits above any of them (proven on regtest
//!    by `tests/confirmed_snapshot_regtest.rs`). If one cannot be refused,
//!    nothing is loaded. Fails closed: only the engine's "Block not found"
//!    lets a block pass as one the node has never seen; a lookup with any
//!    other answer, or none, stops the load ([`LoadError::HeldNotRefused`]).
//! 3. The start record (`crate::snapshot_start`): the height, the base and
//!    the operators whose signatures step 1 verified, written before the
//!    trim drops their signatures. If the load then does not happen, the
//!    record that was there before goes back.
//! 4. A trimmed manifest with every signature from a key this node pins, in
//!    order, and no other: the engine refuses a manifest carrying any key it
//!    does not pin. "Pins" are the app's pins that the running engine
//!    reports trusting ([`node_view`]), not the keys it was meant to start
//!    with.
//! 5. `loadtxoutsetattested`, which the engine allows only in mirror mode.
//!    The engine's own refusal (btx-cli's `error code:` reply) leaves the
//!    chainstate untouched ([`LoadError::Engine`]). No answer at all (btx-cli
//!    could not reach the node, lost the connection partway, was killed)
//!    may still have loaded it: it counts only when the node then shows this
//!    snapshot active, and goes on to step 6; otherwise
//!    [`LoadError::EngineUnanswered`].
//! 6. After the load, the block at each refused height must not be the
//!    refused block. If it ever were, the caller stops the node and discards
//!    the snapshot ([`set_aside_snapshot_chainstate`]). Fails closed: only a
//!    block hash or the engine's "Block height out of range" is an answer;
//!    a question left unanswered is asked again a few times, then it is
//!    [`LoadError::PostLoadCheckUnavailable`].
//!
//! [`LoadError::HeldRootOnChain`], [`LoadError::PostLoadCheckUnavailable`]
//! and [`LoadError::EngineUnanswered`] all mean the engine may hold a
//! snapshot the app refuses: callers treat the last two exactly like the
//! first, stop the node and restore the chain data
//! ([`LoadError::restore_chain_data`]). On every error after step 4 the
//! previous start record goes back and the trimmed manifest is removed.
//!
//! Every [`LoadError`] is for the log: it can carry text from the manifest
//! the website served or from the engine, so no caller shows it as is.

use crate::attested_snapshot::{self, PairKind, ReadyPair};
use crate::confirmed_snapshot::{self as cs, NodeView};
use crate::error::AppError;
use crate::known_invalid::{self, HeldBranch, KnownInvalidBlock};
use crate::rpc::Rpc;
use crate::snapshot::LoadOutcome;
use crate::snapshot_start::{self, StartRecord, StartSource};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Runs `loadtxoutsetattested`. The app's is [`CliRunner`]; tests script one.
#[async_trait::async_trait]
pub trait LoadRunner: Send + Sync {
    async fn load_attested(&self, file: &Path, manifest: &Path) -> LoadOutcome;
}

/// `btx-cli` with no client timeout, the way `crate::snapshot` has always
/// loaded: a load reads the whole file and can take minutes.
pub struct CliRunner {
    pub btx_cli: PathBuf,
    /// Everything before the method: `-datadir=...` for the app, plus
    /// `-regtest` and `-rpcport=...` in the regtest test.
    pub args: Vec<String>,
}

impl CliRunner {
    pub fn for_datadir(btx_cli: &Path, datadir: &Path) -> Self {
        Self {
            btx_cli: btx_cli.to_path_buf(),
            args: vec![format!("-datadir={}", datadir.display())],
        }
    }
}

#[async_trait::async_trait]
impl LoadRunner for CliRunner {
    async fn load_attested(&self, file: &Path, manifest: &Path) -> LoadOutcome {
        crate::snapshot::run_cli_load(
            &self.btx_cli,
            &self.args,
            "loadtxoutsetattested",
            &[file, manifest],
        )
        .await
    }
}

/// The blocks the app refuses, as this load sees them.
#[derive(Debug, Clone, Copy)]
pub struct Holds<'a> {
    pub invalid: &'a [KnownInvalidBlock],
    pub held: &'a [HeldBranch],
}

impl Holds<'static> {
    /// The compiled lists, or none when the operator switched refusal off
    /// (`EASYBTX_NODE_REFUSE_KNOWN_INVALID=0`).
    pub fn compiled() -> Self {
        if known_invalid::refusal_enabled() {
            Self {
                invalid: known_invalid::KNOWN_INVALID_BLOCKS,
                held: known_invalid::HELD_BRANCHES,
            }
        } else {
            Self::none()
        }
    }

    pub fn none() -> Self {
        Self {
            invalid: &[],
            held: &[],
        }
    }
}

impl Holds<'_> {
    /// (height, block) of everything refused, invalid blocks first.
    fn roots(&self) -> Vec<(u64, &'static str)> {
        self.invalid
            .iter()
            .map(|b| (b.height, b.hash))
            .chain(self.held.iter().map(|h| (h.height, h.root)))
            .collect()
    }
}

/// Why a pair was not loaded. `Display` is one line for the log, never for
/// the screen (see the module doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The pair no longer passes the checks against the live node.
    NotConfirmed(String),
    /// A refused block could not be refused, so nothing was loaded.
    HeldNotRefused(String),
    /// The engine refused the load. The chainstate is untouched.
    Engine(String),
    /// After the load a refused block is on this node's chain. The caller
    /// stops the node and discards the snapshot.
    HeldRootOnChain {
        height: u64,
        root: String,
    },
    /// After the load the node did not say which block it has at a refused
    /// height: no answer, or an answer that is neither a block hash nor the
    /// engine's "Block height out of range". The app cannot vouch for the
    /// chain it just loaded, so callers treat this exactly like
    /// [`LoadError::HeldRootOnChain`]: stop the node and restore the chain
    /// data ([`LoadError::restore_chain_data`]).
    PostLoadCheckUnavailable {
        height: u64,
        root: String,
        why: String,
    },
    /// btx-cli brought back no answer to the load (it could not reach the
    /// node, lost the connection partway or was killed), and the node does
    /// not show the snapshot active afterwards: the engine may have loaded
    /// it, or may still be loading it. Callers treat this exactly like
    /// [`LoadError::HeldRootOnChain`]: stop the node and restore the chain
    /// data ([`LoadError::restore_chain_data`]).
    EngineUnanswered(String),
    Io(String),
}

impl LoadError {
    /// The engine loaded the snapshot and the app refuses the result: the
    /// caller stops the node and sets the snapshot chainstate aside
    /// ([`set_aside_snapshot_chainstate`]). [`LoadError::HeldRootOnChain`],
    /// [`LoadError::PostLoadCheckUnavailable`] and
    /// [`LoadError::EngineUnanswered`], and nothing else: every other error
    /// leaves the chainstate as it was.
    pub fn restore_chain_data(&self) -> bool {
        matches!(
            self,
            LoadError::HeldRootOnChain { .. }
                | LoadError::PostLoadCheckUnavailable { .. }
                | LoadError::EngineUnanswered(_)
        )
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NotConfirmed(e) => write!(f, "the snapshot no longer checks out: {e}"),
            LoadError::HeldNotRefused(e) => write!(f, "a refused block could not be refused yet: {e}"),
            LoadError::Engine(e) => write!(f, "the engine did not load it: {e}"),
            LoadError::HeldRootOnChain { height, root } => write!(
                f,
                "after the load, block {root} at {height} is on this node's chain, which the app refuses"
            ),
            LoadError::PostLoadCheckUnavailable { height, root, why } => write!(
                f,
                "after the load, the node did not say which block it has at {height}, so the app cannot tell whether {root} is on its chain: {why}"
            ),
            LoadError::EngineUnanswered(e) => write!(
                f,
                "the engine gave no answer to the load and the node does not show the snapshot active, so the app cannot tell whether it loaded: {e}"
            ),
            LoadError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// A finished load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub height: u64,
    /// Signatures in the trimmed manifest the engine read.
    pub signatures: usize,
    /// The node's chain was already past the base ("Work does not exceed
    /// active chainstate"). Nothing changed; the caller counts it as loaded.
    pub superseded: bool,
}

/// What the running node says about itself, for [`cs::check`] and the trim.
///
/// Always asked of the node, never assumed: `genesis` is its `getblockhash
/// 0`, `replay_context` its `getmatmultrustedstatus.replay_authority_context`,
/// and a node that does not answer leaves them `None`, which [`load`]
/// refuses. [`NodeView::pinned`] is those of the app's `pinned` keys, in
/// their order, that the engine lists in `trusted_signer_pubkeys`: the
/// engine refuses a manifest carrying any key it does not trust, so the trim
/// keeps only these, and a node that does not list its keys pins none.
/// `start_height` is where the node starts without this pair:
/// `crate::attested_snapshot::fallback_start` of the compiled anchor.
pub async fn node_view(rpc: &dyn Rpc, pinned: &[&str], start_height: u64) -> NodeView {
    let genesis = rpc
        .call("getblockhash", json!([0]))
        .await
        .ok()
        .and_then(|v| v.as_str().map(str::to_string));
    // One answer for both: the replay context and the keys the engine trusts.
    let status = rpc
        .call("getmatmultrustedstatus", json!([]))
        .await
        .unwrap_or_default();
    let replay_context =
        serde_json::from_value::<crate::node_api::MatmulTrustedStatus>(status.clone())
            .ok()
            .and_then(|s| s.replay_authority_context);
    let trusted = engine_trusted_keys(&status);
    NodeView {
        genesis,
        replay_context,
        start_height,
        pinned: cs::pinned_keys(pinned)
            .into_iter()
            .filter(|k| trusted.contains(k))
            .collect(),
    }
}

/// The view a load at this node's next launch will have, for a check made
/// now on the running node: Fast-forward's offer, and its step 1, which
/// judge the confirmed snapshot before any launch. Genesis and replay
/// context are the running node's ([`node_view`]). The pins are the keys
/// that launch's engine will list: on a node that follows signatures
/// (`follows_signatures`), the running engine's own, since it loads in
/// place; on a node that checks blocks, every one of `compiled`, which its
/// one mirror launch pins (`crate::node`'s mirror arm), whatever the
/// running engine pins now. A node that checks blocks and pins nothing (the
/// shipped conf) lists no key at all, and judged by that it would refuse
/// every manifest.
pub async fn launch_view(
    rpc: &dyn Rpc,
    compiled: &[&str],
    start_height: u64,
    follows_signatures: bool,
) -> NodeView {
    let mut view = node_view(rpc, compiled, start_height).await;
    if !follows_signatures {
        view.pinned = cs::pinned_keys(compiled);
    }
    view
}

/// The compressed keys in a `getmatmultrustedstatus` answer's
/// `trusted_signer_pubkeys`: the keys the engine verifies a snapshot
/// manifest against. Empty when the answer has no such list.
fn engine_trusted_keys(status: &Value) -> Vec<[u8; 33]> {
    let hexes: Vec<&str> = status["trusted_signer_pubkeys"]
        .as_array()
        .map(|keys| keys.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    cs::pinned_keys(&hexes)
}

/// Where the trimmed manifest goes: beside the pair, under a name the sweep
/// removes with it.
pub fn trimmed_manifest_path(pair: &ReadyPair) -> PathBuf {
    pair.manifest
        .with_file_name(format!("loaded-{}.manifest", pair.height))
}

/// Section 7, step 1 again, against the node about to load: the trimmed
/// manifest the engine will read, and what the start record will say. Every
/// refusal that needs no engine happens here, before any side effect.
fn recheck(
    pair: &ReadyPair,
    view: &NodeView,
    regtest_env: Option<&str>,
) -> Result<(cs::Manifest, StartRecord), LoadError> {
    let bytes =
        std::fs::read(&pair.manifest).map_err(|e| LoadError::Io(format!("manifest: {e}")))?;
    let m = cs::parse(&bytes).map_err(|e| LoadError::NotConfirmed(e.to_string()))?;
    // `cs::node_agrees` skips what the node did not say; the loader does not.
    if view.genesis.is_none() {
        return Err(LoadError::NotConfirmed(
            "the node did not say which chain it is on".into(),
        ));
    }
    if view.replay_context.is_none() {
        return Err(LoadError::NotConfirmed(
            "the node did not say which replay context it runs with".into(),
        ));
    }
    // Section 7, step 4's trim, worked out now: a manifest with no signature
    // the engine trusts is refused before anything is touched.
    let trimmed = cs::trim_to_pinned(&m, &view.pinned);
    if trimmed.signatures.is_empty() {
        return Err(LoadError::NotConfirmed(
            "no signature is from a key this node pins".into(),
        ));
    }
    let block_hash = m.statement.block_hash().display_hex();
    let start = match pair.kind {
        PairKind::Confirmed => {
            let confirmed = cs::check(&m, view, regtest_env)
                .map_err(|e| LoadError::NotConfirmed(e.to_string()))?;
            if confirmed.height != pair.height {
                return Err(LoadError::NotConfirmed(format!(
                    "the pair is kept as height {} but its statement is for height {}",
                    pair.height, confirmed.height
                )));
            }
            let file =
                std::fs::read(&pair.file).map_err(|e| LoadError::Io(format!("snapshot: {e}")))?;
            let mut h = cs::FileHasher::default();
            h.update(&file);
            let (len, _, double) = h.finish();
            if !cs::file_matches(&m.statement, len, &double) {
                return Err(LoadError::NotConfirmed(
                    "the file is not the one the statement signs".into(),
                ));
            }
            StartRecord {
                height: confirmed.height,
                block_hash,
                source: StartSource::Confirmed,
                // Verified here, grouped by operator, in the list's order.
                operators: confirmed.operators,
            }
        }
        PairKind::Pinned => {
            let pin = attested_snapshot::pinned_pair();
            let same = |p: &Path, sha: &str| {
                matches!(crate::snapshot::verify_file_sha256(p, sha), Ok(true))
            };
            if pair.height != pin.height
                || !same(&pair.manifest, &pin.manifest_sha256)
                || !same(&pair.file, &pin.sha256)
            {
                return Err(LoadError::NotConfirmed(
                    "the pinned pair on disk is not the one compiled into the app".into(),
                ));
            }
            cs::node_agrees(&m.statement, view)
                .map_err(|e| LoadError::NotConfirmed(e.to_string()))?;
            StartRecord {
                height: pair.height,
                block_hash,
                source: StartSource::Pinned,
                operators: Vec::new(),
            }
        }
    };
    Ok((trimmed, start))
}

/// Section 7, steps 3 to 6. See the module doc. `datadir` is where the start
/// record goes: the datadir on mainnet, the network folder on regtest.
pub async fn load(
    rpc: &dyn Rpc,
    runner: &dyn LoadRunner,
    pair: &ReadyPair,
    view: &NodeView,
    holds: &Holds<'_>,
    regtest_env: Option<&str>,
    datadir: &Path,
) -> Result<Loaded, LoadError> {
    let (trimmed, start) = recheck(pair, view, regtest_env)?;
    refuse_holds(rpc, holds).await?;

    // Step 4: who confirmed it, before the trimmed manifest drops their
    // signatures. Put back as it was if the load does not happen.
    let previous = snapshot_start::replace(datadir, &start).map_err(|e| {
        LoadError::Io(format!(
            "write {}: {e}",
            snapshot_start::path(datadir).display()
        ))
    })?;
    let result = write_and_load(rpc, runner, pair, holds, &trimmed).await;
    if !matches!(
        result,
        Ok(Loaded {
            superseded: false,
            ..
        })
    ) {
        snapshot_start::put_back(datadir, previous);
    }
    result
}

/// Steps 4 to 6 after the record: write the trimmed manifest, load, and look
/// for a refused block. The trimmed manifest goes again on any error.
async fn write_and_load(
    rpc: &dyn Rpc,
    runner: &dyn LoadRunner,
    pair: &ReadyPair,
    holds: &Holds<'_>,
    trimmed: &cs::Manifest,
) -> Result<Loaded, LoadError> {
    let path = trimmed_manifest_path(pair);
    crate::fsx::atomic_write(&path, &trimmed.to_bytes())
        .map_err(|e| LoadError::Io(format!("write {}: {e}", path.display())))?;
    let result = load_and_check(rpc, runner, pair, holds, trimmed, &path).await;
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result
}

async fn load_and_check(
    rpc: &dyn Rpc,
    runner: &dyn LoadRunner,
    pair: &ReadyPair,
    holds: &Holds<'_>,
    trimmed: &cs::Manifest,
    path: &Path,
) -> Result<Loaded, LoadError> {
    let superseded = match runner.load_attested(&pair.file, path).await {
        LoadOutcome::Loaded => false,
        LoadOutcome::Superseded => true,
        LoadOutcome::Failed(e) => return Err(LoadError::Engine(e)),
        // The engine may have loaded it anyway. It counts only if the node
        // shows this very snapshot active, and then goes through step 6.
        LoadOutcome::NoAnswer(e) => {
            let base = trimmed.statement.block_hash().display_hex();
            if !snapshot_active(rpc, &base).await {
                return Err(LoadError::EngineUnanswered(e));
            }
            false
        }
    };
    check_holds_after_load(rpc, holds).await?;
    Ok(Loaded {
        height: pair.height,
        signatures: trimmed.signatures.len(),
        superseded,
    })
}

/// How many times a question after the load is asked before the app gives
/// up on an answer, and the pause between: one RPC error right after a good
/// load must not discard it.
const POST_LOAD_ATTEMPTS: u32 = 3;
const POST_LOAD_PAUSE: Duration = if cfg!(test) {
    Duration::from_millis(1)
} else {
    Duration::from_secs(2)
};

/// Section 7, step 6: the block at each refused height is not the refused
/// block. A question the node leaves unanswered is asked again, up to
/// [`POST_LOAD_ATTEMPTS`] times; then [`LoadError::PostLoadCheckUnavailable`].
/// `crate::snapshot` also asks it of a snapshot a signed-only load finds
/// already there, which the run that loaded it may never have checked.
pub(crate) async fn check_holds_after_load(
    rpc: &dyn Rpc,
    holds: &Holds<'_>,
) -> Result<(), LoadError> {
    for (height, root) in holds.roots() {
        let mut attempt = 1;
        loop {
            match block_at(rpc, height).await {
                Ok(Some(at)) if at.eq_ignore_ascii_case(root) => {
                    return Err(LoadError::HeldRootOnChain {
                        height,
                        root: root.to_string(),
                    })
                }
                Ok(_) => break,
                Err(_) if attempt < POST_LOAD_ATTEMPTS => {
                    attempt += 1;
                    tokio::time::sleep(POST_LOAD_PAUSE).await;
                }
                Err(why) => {
                    return Err(LoadError::PostLoadCheckUnavailable {
                        height,
                        root: root.to_string(),
                        why,
                    })
                }
            }
        }
    }
    Ok(())
}

/// The block the node's active chain has at `height`: `Some(hash)`, `None`
/// when the chain does not reach it (the engine's "Block height out of
/// range"), or why the node did not say.
async fn block_at(rpc: &dyn Rpc, height: u64) -> Result<Option<String>, String> {
    match rpc.call("getblockhash", json!([height])).await {
        Ok(Value::String(at)) if at.len() == 64 && at.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Ok(Some(at))
        }
        Ok(v) => Err(format!("the answer {v} is not a block hash")),
        Err(e) if is_height_out_of_range(&e) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// After a load btx-cli brought back no answer for: whether the node shows
/// the snapshot based on `base` as one of its chainstates. Asked up to
/// [`POST_LOAD_ATTEMPTS`] times, since the load may still be finishing.
async fn snapshot_active(rpc: &dyn Rpc, base: &str) -> bool {
    for attempt in 1..=POST_LOAD_ATTEMPTS {
        if let Ok(states) = crate::node_api::get_chainstates(rpc).await {
            let active = states
                .snapshot()
                .and_then(|c| c.snapshot_blockhash.as_deref())
                .is_some_and(|h| h.eq_ignore_ascii_case(base));
            if active {
                return true;
            }
        }
        if attempt < POST_LOAD_ATTEMPTS {
            tokio::time::sleep(POST_LOAD_PAUSE).await;
        }
    }
    false
}

/// What a compiled `loadtxoutset` btx-cli brought back no answer for came
/// to, when the app can say. See [`unanswered_compiled_load`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnansweredCompiledLoad {
    /// The snapshot based at the anchor is active and no refused block is on
    /// the chain: it loaded.
    Loaded,
    /// The node answers and shows no snapshot chainstate at all: nothing was
    /// loaded, and there is nothing to set aside.
    NothingLoaded,
}

/// A compiled `loadtxoutset` btx-cli brought back no answer for
/// (`LoadOutcome::NoAnswer`). It has no manifest, so it never comes through
/// [`load`], but it counts the same way: only if the node then shows the
/// snapshot active, the one based at `base_height` (the app does not compile
/// the engine's base hash, so the base's own header says where it is), and
/// step 6 finds no refused block on its chain.
///
/// Unlike a signed load, a node that answers and shows no snapshot chainstate
/// at all loaded nothing ([`UnansweredCompiledLoad::NothingLoaded`]): a
/// btx-cli that never reaches the engine (a refused login, "Could not
/// connect") would otherwise have the node set aside and restarted on every
/// launch. That is safe because the compiled snapshot is the engine's own and
/// its base sits below every refused block (a test in `crate::snapshot`
/// holds the pin to that). A snapshot chainstate based anywhere else, a
/// refused block on the chain, or a node that does not say is an error, and
/// every error here has [`LoadError::restore_chain_data`]: the engine may hold
/// a snapshot the app cannot vouch for.
pub(crate) async fn unanswered_compiled_load(
    rpc: &dyn Rpc,
    base_height: u64,
    holds: &Holds<'_>,
    why: &str,
) -> Result<UnansweredCompiledLoad, LoadError> {
    match compiled_snapshot_seen(rpc, base_height).await {
        Seen::Active => {
            check_holds_after_load(rpc, holds).await?;
            Ok(UnansweredCompiledLoad::Loaded)
        }
        Seen::NoSnapshot => Ok(UnansweredCompiledLoad::NothingLoaded),
        Seen::Unvouched(what) => Err(LoadError::EngineUnanswered(format!("{why}; {what}"))),
    }
}

/// What the node shows after a compiled load, for [`unanswered_compiled_load`].
enum Seen {
    /// A snapshot chainstate based at the anchor.
    Active,
    /// The node answers, and shows no snapshot chainstate.
    NoSnapshot,
    /// Anything else, and what it was, for the log.
    Unvouched(String),
}

/// Asked up to [`POST_LOAD_ATTEMPTS`] times until the node shows the snapshot
/// based at `base_height`, since the load may still be finishing. Short of
/// that, the last answer stands.
async fn compiled_snapshot_seen(rpc: &dyn Rpc, base_height: u64) -> Seen {
    let mut seen = Seen::Unvouched("the node was not asked".into());
    for attempt in 1..=POST_LOAD_ATTEMPTS {
        seen = match crate::node_api::get_chainstates(rpc).await {
            Err(e) => Seen::Unvouched(format!("getchainstates did not answer: {e}")),
            Ok(states) => match states.snapshot().and_then(|c| c.snapshot_blockhash.clone()) {
                None => Seen::NoSnapshot,
                Some(base) => match rpc.call("getblockheader", json!([base, true])).await {
                    Ok(header) if header["height"].as_u64() == Some(base_height) => {
                        return Seen::Active
                    }
                    Ok(header) => Seen::Unvouched(format!(
                        "the snapshot chainstate is based on {base} at height {}, not at {base_height}",
                        header["height"]
                    )),
                    Err(e) => Seen::Unvouched(format!(
                        "the node did not say where the snapshot base {base} is: {e}"
                    )),
                },
            },
        };
        if attempt < POST_LOAD_ATTEMPTS {
            tokio::time::sleep(POST_LOAD_PAUSE).await;
        }
    }
    seen
}

/// Section 7, step 3: refuse every block the app refuses, invalid blocks
/// first, then the held branches in order, as `crate::known_invalid` does
/// for the fork check. Stricter than the fork check: only "Block not found"
/// lets a block pass as one the node has never seen (a node cannot follow a
/// block it does not know); a lookup with any other answer, or none, stops
/// the load, and so does a failed `invalidateblock`. Stopping at the first
/// also keeps the held order: nothing after a hold that stands is touched.
async fn refuse_holds(rpc: &dyn Rpc, holds: &Holds<'_>) -> Result<(), LoadError> {
    for (height, block) in holds.roots() {
        match rpc.call("getblockheader", json!([block, true])).await {
            Ok(_) => {}
            Err(e) if is_block_not_found(&e) => continue,
            Err(e) => {
                return Err(LoadError::HeldNotRefused(format!(
                    "{block} at {height}: the node did not say whether it has it: {e}"
                )))
            }
        }
        if let Err(e) = rpc.call("invalidateblock", json!([block])).await {
            return Err(LoadError::HeldNotRefused(format!(
                "{block} at {height}: {e}"
            )));
        }
    }
    Ok(())
}

/// `getblockheader` for a block the node has no header for: v0.34.9
/// (84b998b4) `src/rpc/blockchain.cpp:881`, `RPC_INVALID_ADDRESS_OR_KEY`
/// (-5), "Block not found". The one answer that says the node never saw it.
fn is_block_not_found(e: &AppError) -> bool {
    matches!(e, AppError::Rpc { code: -5, message } if message == "Block not found")
}

/// `getblockhash` for a height above the active tip: v0.34.9 (84b998b4)
/// `src/rpc/blockchain.cpp:774`, `RPC_INVALID_PARAMETER` (-8), "Block height
/// out of range". The one answer that says no block at that height is on
/// the active chain.
fn is_height_out_of_range(e: &AppError) -> bool {
    matches!(e, AppError::Rpc { code: -8, message } if message == "Block height out of range")
}

/// What a set-aside snapshot chainstate is called, before the time it was
/// set aside (Unix seconds). `crate::disk` sweeps them.
pub const REFUSED_CHAINSTATE_PREFIX: &str = "chainstate_snapshot.refused-";

/// Move a snapshot chainstate the app refuses out of the engine's way, as the
/// engine itself does with one that fails its background check. `network_dir`
/// is the datadir on mainnet. Call only with the node stopped. `Ok(None)`
/// when there was none. It goes to [`REFUSED_CHAINSTATE_PREFIX`] and the
/// time; the launch sweep removes it after a week, Remove node data at once
/// (`crate::disk`).
pub fn set_aside_snapshot_chainstate(
    network_dir: &Path,
    now_unix: u64,
) -> std::io::Result<Option<PathBuf>> {
    let from = network_dir.join("chainstate_snapshot");
    if !from.exists() {
        return Ok(None);
    }
    let to = network_dir.join(format!("{REFUSED_CHAINSTATE_PREFIX}{now_unix}"));
    std::fs::rename(&from, &to)?;
    Ok(Some(to))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{AppError, AppResult};
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    const R_PC: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PC.manifest");
    const R_PCD: &[u8] =
        include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PCD.manifest");
    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_DAT: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-100.dat");
    /// The published 239,111 manifest: the pinned pair's, signed by the 3060.
    const PINNED_MANIFEST: &[u8] =
        include_bytes!("../tests/fixtures/confirmed_snapshot/mainnet-239111.manifest");
    const THE_3060: &str = "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675";
    const P: &str = "0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464";
    const C: &str = "02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f";
    /// The regtest statements' base block, display order.
    const BASE_100: &str = "bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46";
    const ROOT: &str = "8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c";
    const HELD: &[HeldBranch] = &[HeldBranch {
        height: 50,
        root: ROOT,
        why: "a test hold",
    }];

    /// A node scripted per method. `chain` answers getblockhash by height.
    struct Node {
        genesis: &'static str,
        chain: HashMap<u64, String>,
        knows_root: bool,
        invalidate_fails: bool,
        /// The keys the engine says it trusts (`trusted_signer_pubkeys`).
        trusted: Vec<&'static str>,
        /// Whether `getmatmultrustedstatus` names a replay context.
        reports_context: bool,
        /// Methods the node does not answer, as a node that went away.
        silent: Vec<&'static str>,
        /// What getblockhash answers for a height not in `chain`: the
        /// engine's error, or `None` for a `null` answer.
        off_chain: Option<(i64, &'static str)>,
        /// An error getblockheader answers instead of the usual.
        header_error: Option<(i64, &'static str)>,
        /// The `snapshot_blockhash` getchainstates reports, if any.
        snapshot_base: Option<&'static str>,
        /// getblockhash above 0 goes unanswered this many times first.
        hash_failures: Mutex<u32>,
        /// Every call, shared with the [`Runner`] so the load is in sequence.
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl Node {
        fn regtest() -> Self {
            Self {
                genesis: crate::operators::REGTEST_GENESIS,
                chain: HashMap::new(),
                knows_root: true,
                invalidate_fails: false,
                trusted: vec![P],
                reports_context: true,
                silent: Vec::new(),
                off_chain: Some((-8, "Block height out of range")),
                header_error: None,
                snapshot_base: None,
                hash_failures: Mutex::new(0),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
        fn methods(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
        fn count(&self, method: &str) -> usize {
            self.methods().iter().filter(|m| *m == method).count()
        }
    }

    #[async_trait::async_trait]
    impl Rpc for Node {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            self.calls.lock().unwrap().push(method.to_string());
            if self.silent.contains(&method) {
                return Err(AppError::Http("connection refused".into()));
            }
            let not_found = || AppError::Rpc {
                code: -5,
                message: "Block not found".into(),
            };
            match method {
                "getblockhash" => {
                    let h = params[0].as_u64().unwrap_or(u64::MAX);
                    if h == 0 {
                        return Ok(json!(self.genesis));
                    }
                    {
                        let mut left = self.hash_failures.lock().unwrap();
                        if *left > 0 {
                            *left -= 1;
                            return Err(AppError::Http("connection reset".into()));
                        }
                    }
                    match (self.chain.get(&h), self.off_chain) {
                        (Some(s), _) => Ok(json!(s)),
                        (None, Some((code, message))) => Err(AppError::Rpc {
                            code,
                            message: message.into(),
                        }),
                        (None, None) => Ok(Value::Null),
                    }
                }
                "getmatmultrustedstatus" => {
                    let mut status = json!({
                        "matmul_validation_mode": "trusted",
                        "trusted_mirror": true,
                        "trusted_signer_pubkeys": self.trusted,
                    });
                    if self.reports_context {
                        status["replay_authority_context"] = json!(cs::REGTEST_REPLAY_CONTEXT);
                    }
                    Ok(status)
                }
                "getblockheader" if self.header_error.is_some() => {
                    let (code, message) = self.header_error.unwrap();
                    Err(AppError::Rpc {
                        code,
                        message: message.into(),
                    })
                }
                "getblockheader" if self.knows_root => Ok(json!({"height": 50})),
                "getblockheader" => Err(not_found()),
                "invalidateblock" if self.invalidate_fails => Err(AppError::Rpc {
                    code: -1,
                    message: "request timed out".into(),
                }),
                "invalidateblock" => Ok(Value::Null),
                "getchainstates" => {
                    let mut states = vec![json!({"blocks": 40, "validated": true})];
                    if let Some(base) = self.snapshot_base {
                        states.push(json!({
                            "blocks": 100,
                            "snapshot_blockhash": base,
                            "validated": false,
                        }));
                    }
                    Ok(json!({"headers": 100, "chainstates": states}))
                }
                other => panic!("the loader must never call {other}"),
            }
        }
    }

    /// Records the manifest it was handed and answers as told. Built
    /// [`Runner::watching`] a node and a datadir, it also puts the load in
    /// the node's call log and notes the start record on disk at that moment.
    struct Runner {
        answer: LoadOutcome,
        seen: Mutex<Option<Vec<u8>>>,
        dir: Option<PathBuf>,
        record_then: Mutex<Option<Option<StartRecord>>>,
        log: Option<Arc<Mutex<Vec<String>>>>,
    }

    impl Runner {
        fn new(answer: LoadOutcome) -> Self {
            Self {
                answer,
                seen: Mutex::new(None),
                dir: None,
                record_then: Mutex::new(None),
                log: None,
            }
        }
        fn watching(answer: LoadOutcome, node: &Node, dir: &Path) -> Self {
            Self {
                dir: Some(dir.to_path_buf()),
                log: Some(node.calls.clone()),
                ..Self::new(answer)
            }
        }
        /// The start record as it stood when the engine was asked.
        fn record_then(&self) -> Option<StartRecord> {
            self.record_then
                .lock()
                .unwrap()
                .clone()
                .expect("the engine was asked")
        }
    }

    #[async_trait::async_trait]
    impl LoadRunner for Runner {
        async fn load_attested(&self, _file: &Path, manifest: &Path) -> LoadOutcome {
            if let Some(log) = &self.log {
                log.lock().unwrap().push("loadtxoutsetattested".into());
            }
            if let Some(dir) = &self.dir {
                *self.record_then.lock().unwrap() = Some(snapshot_start::read(dir));
            }
            *self.seen.lock().unwrap() = Some(std::fs::read(manifest).unwrap());
            self.answer.clone()
        }
    }

    fn confirmed_record(operators: &[&str]) -> StartRecord {
        StartRecord {
            height: 100,
            block_hash: BASE_100.into(),
            source: StartSource::Confirmed,
            operators: operators.iter().map(|n| n.to_string()).collect(),
        }
    }

    fn pair_on_disk(dir: &Path, manifest: &[u8], file: &[u8]) -> ReadyPair {
        let (f, m) = attested_snapshot::pair_paths(dir, 100);
        std::fs::create_dir_all(attested_snapshot::pair_dir(dir)).unwrap();
        std::fs::write(&f, file).unwrap();
        std::fs::write(&m, manifest).unwrap();
        ReadyPair {
            kind: PairKind::Confirmed,
            height: 100,
            file: f,
            manifest: m,
        }
    }

    fn env() -> String {
        format!("producer={P};confirmer={C}")
    }

    async fn view(node: &Node) -> NodeView {
        node_view(node, &[P], 0).await
    }

    #[tokio::test]
    async fn a_confirmed_pair_is_trimmed_to_the_pins_and_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PCD, R_DAT);
        let node = Node::regtest();
        let runner = Runner::watching(LoadOutcome::Loaded, &node, tmp.path());
        let got = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds {
                invalid: &[],
                held: HELD,
            },
            Some(&env()),
            tmp.path(),
        )
        .await
        .unwrap();
        assert_eq!(
            got,
            Loaded {
                height: 100,
                signatures: 1,
                superseded: false
            }
        );
        assert_eq!(
            runner.seen.lock().unwrap().as_deref(),
            Some(R_P),
            "the engine saw the producer's own file, byte for byte"
        );
        // Refused before the load, checked after it.
        let calls = node.methods();
        let at = |m: &str| calls.iter().position(|c| c == m).unwrap();
        let checked = calls.iter().rposition(|c| c == "getblockhash").unwrap();
        assert!(
            at("getblockheader") < at("invalidateblock")
                && at("invalidateblock") < at("loadtxoutsetattested")
                && at("loadtxoutsetattested") < checked,
            "{calls:?}"
        );
        // Section 7, step 4: the names survive the trim. Only verified
        // signers on the list count (D signed too and is on no list), in the
        // list's order. On disk before the engine is asked, and after.
        let record = confirmed_record(&["producer", "confirmer"]);
        assert_eq!(runner.record_then(), Some(record.clone()));
        assert_eq!(snapshot_start::read(tmp.path()), Some(record));
        assert!(trimmed_manifest_path(&pair).is_file(), "kept for the sweep");
    }

    #[tokio::test]
    async fn a_hold_that_cannot_be_refused_stops_the_load() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let mut node = Node::regtest();
        node.invalidate_fails = true;
        let runner = Runner::new(LoadOutcome::Loaded);
        let err = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds {
                invalid: &[],
                held: HELD,
            },
            Some(&env()),
            tmp.path(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, LoadError::HeldNotRefused(_)), "{err}");
        assert!(
            runner.seen.lock().unwrap().is_none(),
            "the engine was never asked"
        );
        assert_eq!(snapshot_start::read(tmp.path()), None, "nothing recorded");
    }

    /// A node that never heard of the root cannot follow it: the load goes on.
    #[tokio::test]
    async fn an_unknown_root_does_not_stop_the_load() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let mut node = Node::regtest();
        node.knows_root = false;
        let runner = Runner::new(LoadOutcome::Loaded);
        assert!(load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds {
                invalid: &[],
                held: HELD
            },
            Some(&env()),
            tmp.path(),
        )
        .await
        .is_ok());
    }

    #[tokio::test]
    async fn a_refused_block_on_the_chain_after_the_load_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let mut node = Node::regtest();
        node.chain.insert(50, ROOT.to_string());
        let runner = Runner::new(LoadOutcome::Loaded);
        let err = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds {
                invalid: &[],
                held: HELD,
            },
            Some(&env()),
            tmp.path(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            err,
            LoadError::HeldRootOnChain {
                height: 50,
                root: ROOT.into()
            }
        );
        assert!(err.restore_chain_data());
        assert_eq!(
            snapshot_start::read(tmp.path()),
            None,
            "a load the app discards leaves no record"
        );
    }

    #[tokio::test]
    async fn the_engines_refusal_is_passed_on() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let node = Node::regtest();
        let why = "The base block header (x) is part of an invalid chain";
        let runner = Runner::new(LoadOutcome::Failed(why.into()));
        let err = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds::none(),
            Some(&env()),
            tmp.path(),
        )
        .await
        .unwrap_err();
        assert_eq!(err, LoadError::Engine(why.into()));
        assert!(!err.restore_chain_data(), "the chainstate is untouched");
    }

    /// The record is written before the engine is asked, and put back as it
    /// was when the engine refuses.
    #[tokio::test]
    async fn the_start_record_is_put_back_when_the_engine_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let earlier = StartRecord {
            height: 225_927,
            block_hash: "06780445dae193010e099e6425c5430f121416b067b8d68a8a5c3b52e8a4b932".into(),
            source: StartSource::Pinned,
            operators: vec![],
        };
        snapshot_start::write(tmp.path(), &earlier).unwrap();
        let node = Node::regtest();
        let runner = Runner::watching(LoadOutcome::Failed("no".into()), &node, tmp.path());
        let err = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds::none(),
            Some(&env()),
            tmp.path(),
        )
        .await
        .unwrap_err();
        assert_eq!(err, LoadError::Engine("no".into()));
        assert_eq!(
            runner.record_then(),
            Some(confirmed_record(&["producer", "confirmer"])),
            "written before the engine was asked"
        );
        assert_eq!(snapshot_start::read(tmp.path()), Some(earlier));
    }

    #[tokio::test]
    async fn a_pair_that_no_longer_checks_out_is_not_loaded() {
        let node = Node::regtest();
        let runner = Runner::new(LoadOutcome::Loaded);
        let v = view(&node).await;
        // One operator.
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_P, R_DAT);
        let err = load(
            &node,
            &runner,
            &pair,
            &v,
            &Holds::none(),
            Some(&env()),
            tmp.path(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, LoadError::NotConfirmed(_)), "{err}");
        // A file changed on disk since it was downloaded.
        let tmp = tempfile::tempdir().unwrap();
        let mut changed = R_DAT.to_vec();
        changed[7] ^= 1;
        let pair = pair_on_disk(tmp.path(), R_PC, &changed);
        let err = load(
            &node,
            &runner,
            &pair,
            &v,
            &Holds::none(),
            Some(&env()),
            tmp.path(),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("not the one the statement signs"),
            "{err}"
        );
        // A node on another chain.
        let mut mainnet = Node::regtest();
        mainnet.genesis = crate::operators::MAINNET_GENESIS;
        let v = view(&mainnet).await;
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let err = load(
            &mainnet,
            &runner,
            &pair,
            &v,
            &Holds::none(),
            Some(&env()),
            tmp.path(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, LoadError::NotConfirmed(_)), "{err}");
        assert!(runner.seen.lock().unwrap().is_none());
        assert_eq!(snapshot_start::read(tmp.path()), None);
    }

    /// A dissent never reaches the engine, however many operators signed it
    /// and whichever keys the node pins (section 6a).
    #[tokio::test]
    async fn a_dissent_offered_for_loading_is_refused() {
        use k256::ecdsa::signature::hazmat::PrehashSigner;
        use k256::ecdsa::{Signature, SigningKey};
        let (a, b) = (
            SigningKey::from_slice(&[7; 32]).unwrap(),
            SigningKey::from_slice(&[8; 32]).unwrap(),
        );
        let pubkey = |sk: &SigningKey| -> [u8; 33] {
            sk.verifying_key()
                .to_encoded_point(true)
                .as_bytes()
                .try_into()
                .unwrap()
        };
        let d = cs::parse(R_PC).unwrap().statement.chain_facts().dissent();
        let sign = |sk: &SigningKey| {
            let s: Signature = sk.sign_prehash(&d.hash().0).unwrap();
            cs::Signed {
                key: pubkey(sk),
                der: s.to_der().as_bytes().to_vec(),
            }
        };
        let m = cs::Manifest {
            signatures: vec![sign(&a), sign(&b)],
            statement: d.clone(),
        };
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), &m.to_bytes(), R_DAT);
        let env = format!(
            "a={};b={}",
            crate::operators::hex(&pubkey(&a)),
            crate::operators::hex(&pubkey(&b))
        );
        let node = Node::regtest();
        let mut v = view(&node).await;
        v.pinned = vec![pubkey(&a)];
        let runner = Runner::new(LoadOutcome::Loaded);
        let err = load(
            &node,
            &runner,
            &pair,
            &v,
            &Holds::none(),
            Some(&env),
            tmp.path(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, LoadError::NotConfirmed(e) if e.contains("dissent")),
            "{err}"
        );
        assert!(
            runner.seen.lock().unwrap().is_none(),
            "the engine was never asked"
        );
        assert_eq!(snapshot_start::read(tmp.path()), None, "nothing recorded");
    }

    #[tokio::test]
    async fn a_pinned_pair_is_held_to_its_compiled_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut pair = pair_on_disk(tmp.path(), R_P, R_DAT);
        pair.kind = PairKind::Pinned;
        let node = Node::regtest();
        let runner = Runner::new(LoadOutcome::Loaded);
        let err = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &Holds::none(),
            None,
            tmp.path(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("compiled into the app"), "{err}");
    }

    /// The view is the running node's own: its genesis and replay context,
    /// and of the app's pins only those the engine says it trusts.
    #[tokio::test]
    async fn the_view_is_the_running_nodes_own() {
        let mut node = Node::regtest();
        node.trusted = vec![C];
        let v = node_view(&node, &[P, C], 225_927).await;
        assert_eq!(
            v.genesis.as_deref(),
            Some(crate::operators::REGTEST_GENESIS)
        );
        assert_eq!(
            v.replay_context.as_deref(),
            Some(cs::REGTEST_REPLAY_CONTEXT)
        );
        assert_eq!(v.start_height, 225_927);
        assert_eq!(
            v.pinned,
            cs::pinned_keys(&[C]),
            "only keys the engine trusts"
        );
        // A key the engine trusts and the app does not pin is not taken up.
        node.trusted = vec![P, C];
        assert_eq!(
            node_view(&node, &[P], 0).await.pinned,
            cs::pinned_keys(&[P])
        );
    }

    /// Fast-forward judges the confirmed snapshot before the launch that
    /// loads it, so with the pins that launch will have. A node that checks
    /// blocks and pins nothing (a consensus node lists no key, and reports
    /// no replay context) is judged with every compiled key, which its one
    /// mirror launch pins: the snapshot passes. Judged by what the running
    /// engine lists, it would not. A node that follows signatures loads in
    /// place, so its own engine's pins decide, both ways.
    #[tokio::test]
    async fn a_run_judges_the_snapshot_with_the_pins_its_launch_will_have() {
        let manifest = cs::parse(R_PC).unwrap();
        let env = env();
        let confirmed = |v: &NodeView| cs::check(&manifest, v, Some(&env)).map(|c| c.operators);
        let names = || vec!["producer".to_string(), "confirmer".to_string()];

        let mut consensus = Node::regtest();
        consensus.trusted = vec![];
        consensus.reports_context = false;
        let running = node_view(&consensus, &[P, C], 0).await;
        assert!(running.pinned.is_empty(), "the engine lists none");
        assert_eq!(confirmed(&running), Err(cs::Refusal::NoPinnedSigner));
        let v = launch_view(&consensus, &[P, C], 7, false).await;
        assert_eq!(v.pinned, cs::pinned_keys(&[P, C]), "every compiled key");
        assert_eq!(
            v.genesis.as_deref(),
            Some(crate::operators::REGTEST_GENESIS),
            "still the running node's chain"
        );
        assert_eq!(v.replay_context, None);
        assert_eq!(v.start_height, 7);
        assert_eq!(confirmed(&v), Ok(names()), "a node that checks blocks");

        let mut mirror = Node::regtest();
        mirror.trusted = vec![P, C];
        let v = launch_view(&mirror, &[P, C], 0, true).await;
        assert_eq!(v.pinned, cs::pinned_keys(&[P, C]));
        assert_eq!(confirmed(&v), Ok(names()), "a node that follows signatures");
        let v = launch_view(&consensus, &[P, C], 0, true).await;
        assert!(v.pinned.is_empty(), "its own engine's pins, none");
        assert_eq!(confirmed(&v), Err(cs::Refusal::NoPinnedSigner));
    }

    /// The app pins P and C, the engine was started with P alone: the engine
    /// is handed P's signature and nothing it would refuse.
    #[tokio::test]
    async fn a_pin_the_engine_does_not_trust_is_trimmed_away() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PCD, R_DAT);
        let node = Node::regtest();
        let runner = Runner::new(LoadOutcome::Loaded);
        let got = load(
            &node,
            &runner,
            &pair,
            &node_view(&node, &[P, C], 0).await,
            &Holds::none(),
            Some(&env()),
            tmp.path(),
        )
        .await
        .unwrap();
        assert_eq!(got.signatures, 1);
        assert_eq!(runner.seen.lock().unwrap().as_deref(), Some(R_P));
    }

    /// A node that does not say which chain it is on, which replay context
    /// it runs with or which keys it trusts is not loaded: the checks
    /// against the running node are never skipped.
    #[tokio::test]
    async fn a_node_that_does_not_say_where_it_stands_is_not_loaded() {
        let mut no_chain = Node::regtest();
        no_chain.silent = vec!["getblockhash"];
        let mut no_context = Node::regtest();
        no_context.reports_context = false;
        let mut no_keys = Node::regtest();
        no_keys.trusted = vec![];
        for (why, node) in [
            ("which chain", no_chain),
            ("replay context", no_context),
            ("pins", no_keys),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
            let runner = Runner::new(LoadOutcome::Loaded);
            let err = load(
                &node,
                &runner,
                &pair,
                &view(&node).await,
                &Holds::none(),
                Some(&env()),
                tmp.path(),
            )
            .await
            .unwrap_err();
            assert!(
                matches!(&err, LoadError::NotConfirmed(e) if e.contains(why)),
                "{why}: {err}"
            );
            assert!(runner.seen.lock().unwrap().is_none(), "{why}");
            assert_eq!(snapshot_start::read(tmp.path()), None, "{why}");
        }
        // A node that does not answer at all pins nothing and says nothing.
        let mut node = Node::regtest();
        node.silent = vec!["getblockhash", "getmatmultrustedstatus"];
        let v = view(&node).await;
        assert_eq!(
            (v.genesis, v.replay_context, v.pinned),
            (None, None, vec![])
        );
    }

    /// Loads the PC pair under [`HELD`] on `node`: what it came to, and
    /// whether the engine was asked.
    async fn load_held(
        node: &Node,
        v: &NodeView,
        invalid: &[KnownInvalidBlock],
        dir: &Path,
    ) -> (Result<Loaded, LoadError>, bool) {
        let pair = pair_on_disk(dir, R_PC, R_DAT);
        let runner = Runner::new(LoadOutcome::Loaded);
        let holds = Holds {
            invalid,
            held: HELD,
        };
        let got = load(node, &runner, &pair, v, &holds, Some(&env()), dir).await;
        let asked = runner.seen.lock().unwrap().is_some();
        (got, asked)
    }

    /// Section 7, step 6 fails closed. Only the engine's "Block height out of
    /// range" says nothing at a refused height is on the chain; with any
    /// other answer, or none, the app cannot vouch for the chain it loaded.
    #[tokio::test]
    async fn a_check_after_the_load_that_gets_no_answer_fails_closed() {
        // The engine's own answer for a height above its tip.
        let tmp = tempfile::tempdir().unwrap();
        let node = Node::regtest();
        let v = view(&node).await;
        let (got, asked) = load_held(&node, &v, &[], tmp.path()).await;
        assert!(got.is_ok() && asked, "{got:?}");
        assert!(snapshot_start::read(tmp.path()).is_some());

        let mut gone = Node::regtest();
        let v = view(&gone).await;
        gone.silent = vec!["getblockhash"];
        let mut other_code = Node::regtest();
        other_code.off_chain = Some((-1, "request timed out"));
        let mut other_words = Node::regtest();
        other_words.off_chain = Some((-8, "Invalid parameter"));
        let mut no_hash = Node::regtest();
        no_hash.off_chain = None;
        let mut garbled = Node::regtest();
        garbled.chain.insert(50, format!(" {ROOT}"));
        for (what, node) in [
            ("no answer", gone),
            ("another code", other_code),
            ("another message", other_words),
            ("not a hash", no_hash),
            ("not only a hash", garbled),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let (got, asked) = load_held(&node, &v, &[], tmp.path()).await;
            assert!(
                matches!(
                    &got,
                    Err(LoadError::PostLoadCheckUnavailable { height: 50, root, .. })
                        if root == ROOT
                ),
                "{what}: {got:?}"
            );
            assert!(got.unwrap_err().restore_chain_data(), "{what}");
            assert!(asked, "{what}: the engine loaded it");
            assert_eq!(
                snapshot_start::read(tmp.path()),
                None,
                "{what}: a load the app discards leaves no record"
            );
        }
    }

    /// Section 7, step 3 fails closed too. Only "Block not found" says the
    /// node has never seen a refused block; a lookup that gets any other
    /// answer, or none, stops the load before anything is written.
    #[tokio::test]
    async fn a_refusal_that_gets_no_answer_stops_the_load() {
        let split = known_invalid::KNOWN_INVALID_BLOCKS;
        let mut gone = Node::regtest();
        gone.silent = vec!["getblockheader"];
        let mut warming = Node::regtest();
        warming.header_error = Some((-28, "Loading block index..."));
        let mut gone_again = Node::regtest();
        gone_again.silent = vec!["getblockheader"];
        for (what, node, invalid) in [
            ("held, no answer", gone, &[][..]),
            ("held, another code", warming, &[][..]),
            ("invalid list, no answer", gone_again, split),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let v = view(&node).await;
            let (got, asked) = load_held(&node, &v, invalid, tmp.path()).await;
            assert!(
                matches!(&got, Err(LoadError::HeldNotRefused(_))),
                "{what}: {got:?}"
            );
            assert!(!asked, "{what}: the engine was never asked");
            assert_eq!(snapshot_start::read(tmp.path()), None, "{what}");
            assert!(
                !node.methods().contains(&"invalidateblock".to_string()),
                "{what}"
            );
        }
    }

    fn held() -> Holds<'static> {
        Holds {
            invalid: &[],
            held: HELD,
        }
    }

    fn no_answer() -> LoadOutcome {
        LoadOutcome::NoAnswer("error: Could not connect to the server 127.0.0.1:19443".into())
    }

    /// btx-cli brought back no answer, so the engine may have loaded it
    /// anyway. It counts only when the node shows this very snapshot active
    /// and the check after the load passes; a refused block on the chain, or
    /// a check with no answer, restores the chain data as after any load.
    #[tokio::test]
    async fn a_load_with_no_answer_counts_only_when_the_node_shows_it() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let mut node = Node::regtest();
        node.snapshot_base = Some(BASE_100);
        let v = view(&node).await;
        let runner = Runner::new(no_answer());
        let got = load(&node, &runner, &pair, &v, &held(), Some(&env()), tmp.path()).await;
        assert_eq!(
            got,
            Ok(Loaded {
                height: 100,
                signatures: 1,
                superseded: false
            })
        );
        assert!(node.count("getchainstates") >= 1, "asked the node");
        assert_eq!(
            snapshot_start::read(tmp.path()),
            Some(confirmed_record(&["producer", "confirmer"]))
        );

        let mut on_chain = Node::regtest();
        on_chain.snapshot_base = Some(BASE_100);
        on_chain.chain.insert(50, ROOT.into());
        let mut unanswered = Node::regtest();
        unanswered.snapshot_base = Some(BASE_100);
        unanswered.off_chain = Some((-1, "request timed out"));
        for (what, node) in [("on the chain", on_chain), ("unanswered", unanswered)] {
            let tmp = tempfile::tempdir().unwrap();
            let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
            let v = view(&node).await;
            let runner = Runner::new(no_answer());
            let err = load(&node, &runner, &pair, &v, &held(), Some(&env()), tmp.path())
                .await
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    LoadError::HeldRootOnChain { .. } | LoadError::PostLoadCheckUnavailable { .. }
                ),
                "{what}: {err}"
            );
            assert!(err.restore_chain_data(), "{what}");
            assert_eq!(snapshot_start::read(tmp.path()), None, "{what}");
        }
    }

    /// No answer from the load, and the node does not show this snapshot
    /// active (or does not answer): nobody can say whether it loaded, so the
    /// chain data is restored.
    #[tokio::test]
    async fn a_load_nobody_can_account_for_restores_the_chain_data() {
        let mut not_active = Node::regtest();
        not_active.snapshot_base = None;
        let mut another = Node::regtest();
        another.snapshot_base = Some(ROOT);
        let mut quiet = Node::regtest();
        quiet.silent = vec!["getchainstates"];
        for (what, node) in [
            ("not active", not_active),
            ("another base", another),
            ("no answer", quiet),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
            let earlier = StartRecord {
                source: StartSource::Pinned,
                operators: vec![],
                ..confirmed_record(&[])
            };
            snapshot_start::write(tmp.path(), &earlier).unwrap();
            let v = view(&node).await;
            let runner = Runner::new(no_answer());
            let err = load(&node, &runner, &pair, &v, &held(), Some(&env()), tmp.path())
                .await
                .unwrap_err();
            assert!(
                matches!(err, LoadError::EngineUnanswered(_)),
                "{what}: {err}"
            );
            assert!(err.restore_chain_data(), "{what}");
            assert_eq!(snapshot_start::read(tmp.path()), Some(earlier), "{what}");
            assert!(!trimmed_manifest_path(&pair).exists(), "{what}");
        }
    }

    /// One RPC error right after a good load does not discard it: the check
    /// is asked again, up to [`POST_LOAD_ATTEMPTS`] times, then fails closed.
    #[tokio::test]
    async fn an_unanswered_check_after_a_good_load_is_asked_again() {
        // (unanswered first, loaded, times the height was asked)
        for (failures, loads, asked) in [(1, true, 2), (2, true, 3), (3, false, 3)] {
            let tmp = tempfile::tempdir().unwrap();
            let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
            let node = Node::regtest();
            let v = view(&node).await;
            *node.hash_failures.lock().unwrap() = failures;
            let runner = Runner::new(LoadOutcome::Loaded);
            let got = load(&node, &runner, &pair, &v, &held(), Some(&env()), tmp.path()).await;
            // Less the view's getblockhash 0.
            assert_eq!(node.count("getblockhash") - 1, asked, "{failures}");
            if loads {
                assert!(got.is_ok(), "{failures}: {got:?}");
            } else {
                assert!(
                    matches!(got, Err(LoadError::PostLoadCheckUnavailable { .. })),
                    "{failures}: then it fails closed: {got:?}"
                );
            }
        }
    }

    /// `loaded-<h>.manifest` stays only beside a load that counts.
    #[tokio::test]
    async fn the_trimmed_manifest_goes_when_the_load_does_not_count() {
        let refused = (
            Node::regtest(),
            LoadOutcome::Failed("error code: -32603".into()),
        );
        let mut on_chain = Node::regtest();
        on_chain.chain.insert(50, ROOT.into());
        let mut unanswered = Node::regtest();
        unanswered.off_chain = None;
        for (what, (node, answer)) in [
            ("refused", refused),
            ("held root", (on_chain, LoadOutcome::Loaded)),
            ("check unanswered", (unanswered, LoadOutcome::Loaded)),
            ("load unanswered", (Node::regtest(), no_answer())),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
            let v = view(&node).await;
            let runner = Runner::new(answer);
            let got = load(&node, &runner, &pair, &v, &held(), Some(&env()), tmp.path()).await;
            assert!(got.is_err(), "{what}: {got:?}");
            assert!(
                runner.seen.lock().unwrap().is_some(),
                "{what}: it was written"
            );
            assert!(!trimmed_manifest_path(&pair).exists(), "{what}");
        }
    }

    /// Every refusal that needs no engine comes before any side effect: a
    /// pinned pair none of whose signers the engine trusts touches no hold,
    /// writes no record and no trimmed manifest.
    #[tokio::test]
    async fn a_pinned_pair_with_no_pin_the_engine_trusts_is_refused_first() {
        let tmp = tempfile::tempdir().unwrap();
        let pin = attested_snapshot::pinned_pair();
        let (file, manifest) = attested_snapshot::pair_paths(tmp.path(), pin.height);
        std::fs::create_dir_all(attested_snapshot::pair_dir(tmp.path())).unwrap();
        std::fs::write(&file, b"not the pinned file").unwrap();
        std::fs::write(&manifest, PINNED_MANIFEST).unwrap();
        let pair = ReadyPair {
            kind: PairKind::Pinned,
            height: pin.height,
            file,
            manifest,
        };
        let mut node = Node::regtest();
        node.trusted = vec![];
        let v = node_view(&node, &[THE_3060], 0).await;
        let runner = Runner::new(LoadOutcome::Loaded);
        let err = load(&node, &runner, &pair, &v, &held(), None, tmp.path())
            .await
            .unwrap_err();
        assert!(
            matches!(&err, LoadError::NotConfirmed(e) if e.contains("pins")),
            "{err}"
        );
        assert_eq!(node.count("getblockheader"), 0, "no hold was touched");
        assert!(runner.seen.lock().unwrap().is_none());
        assert_eq!(snapshot_start::read(tmp.path()), None);
        assert!(!trimmed_manifest_path(&pair).exists());
        // With the 3060 trusted, the same pair gets as far as its file.
        node.trusted = vec![THE_3060];
        let v = node_view(&node, &[THE_3060], 0).await;
        let err = load(&node, &runner, &pair, &v, &held(), None, tmp.path())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("compiled into the app"), "{err}");
    }

    /// "Work does not exceed active chainstate": counted as loaded, and the
    /// record goes back to what it was, since this snapshot is not where the
    /// node's chain started.
    #[tokio::test]
    async fn a_superseded_load_counts_and_puts_the_record_back() {
        let tmp = tempfile::tempdir().unwrap();
        let pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        let earlier = StartRecord {
            height: 228_000,
            block_hash: ROOT.into(),
            source: StartSource::Engine,
            operators: vec![],
        };
        snapshot_start::write(tmp.path(), &earlier).unwrap();
        let node = Node::regtest();
        let runner = Runner::watching(LoadOutcome::Superseded, &node, tmp.path());
        let got = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &held(),
            Some(&env()),
            tmp.path(),
        )
        .await;
        assert_eq!(
            got,
            Ok(Loaded {
                height: 100,
                signatures: 1,
                superseded: true
            })
        );
        assert_eq!(
            runner.record_then(),
            Some(confirmed_record(&["producer", "confirmer"]))
        );
        assert_eq!(snapshot_start::read(tmp.path()), Some(earlier));
    }

    #[tokio::test]
    async fn a_pair_whose_height_is_not_its_statements_is_not_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let mut pair = pair_on_disk(tmp.path(), R_PC, R_DAT);
        pair.height = 200;
        let node = Node::regtest();
        let runner = Runner::new(LoadOutcome::Loaded);
        let err = load(
            &node,
            &runner,
            &pair,
            &view(&node).await,
            &held(),
            Some(&env()),
            tmp.path(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, LoadError::NotConfirmed(e) if e.contains("height")),
            "{err}"
        );
        assert!(runner.seen.lock().unwrap().is_none());
        assert_eq!(node.count("getblockheader"), 0);
        assert_eq!(snapshot_start::read(tmp.path()), None);
    }

    #[test]
    fn a_refused_snapshot_chainstate_is_moved_aside_not_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(set_aside_snapshot_chainstate(tmp.path(), 1).unwrap(), None);
        let cs_dir = tmp.path().join("chainstate_snapshot");
        std::fs::create_dir_all(&cs_dir).unwrap();
        std::fs::write(cs_dir.join("attested_assumeutxo"), b"x").unwrap();
        let moved = set_aside_snapshot_chainstate(tmp.path(), 1_790_000_000)
            .unwrap()
            .unwrap();
        assert!(!cs_dir.exists());
        assert!(moved.join("attested_assumeutxo").exists());
        assert!(moved.ends_with("chainstate_snapshot.refused-1790000000"));
    }
}
