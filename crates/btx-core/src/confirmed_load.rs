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
//!    [`crate::confirmed_snapshot::check`] and its file's double SHA-256,
//!    the pinned pair its compiled SHA-256s.
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
//! 6. After the load, the block at each refused height must not be the
//!    refused block. If it ever were, the caller stops the node and discards
//!    the snapshot ([`set_aside_snapshot_chainstate`]). Fails closed: only a
//!    block hash or the engine's "Block height out of range" is an answer;
//!    anything else, or no answer, is [`LoadError::PostLoadCheckUnavailable`],
//!    which callers treat exactly like [`LoadError::HeldRootOnChain`] (see
//!    [`LoadError::restore_chain_data`]).
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
    Io(String),
}

impl LoadError {
    /// The engine loaded the snapshot and the app refuses the result: the
    /// caller stops the node and sets the snapshot chainstate aside
    /// ([`set_aside_snapshot_chainstate`]). [`LoadError::HeldRootOnChain`]
    /// and [`LoadError::PostLoadCheckUnavailable`], and nothing else: every
    /// other error leaves the chainstate as it was.
    pub fn restore_chain_data(&self) -> bool {
        matches!(
            self,
            LoadError::HeldRootOnChain { .. } | LoadError::PostLoadCheckUnavailable { .. }
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

/// Section 7, step 1 again, against the node about to load: the manifest
/// and what the start record will say about it.
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
    let block_hash = m.statement.block_hash().display_hex();
    let start = match pair.kind {
        PairKind::Confirmed => {
            let confirmed = cs::check(&m, view, regtest_env)
                .map_err(|e| LoadError::NotConfirmed(e.to_string()))?;
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
    Ok((m, start))
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
    let (m, start) = recheck(pair, view, regtest_env)?;
    refuse_holds(rpc, holds).await?;

    // Step 4: who confirmed it, before the trimmed manifest drops their
    // signatures. Put back as it was if the load does not happen.
    let previous = snapshot_start::replace(datadir, &start).map_err(|e| {
        LoadError::Io(format!(
            "write {}: {e}",
            snapshot_start::path(datadir).display()
        ))
    })?;
    let result = trim_and_load(rpc, runner, pair, view, holds, &m).await;
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

/// Steps 4 to 6 after the record: trim, load, and look for a refused block.
async fn trim_and_load(
    rpc: &dyn Rpc,
    runner: &dyn LoadRunner,
    pair: &ReadyPair,
    view: &NodeView,
    holds: &Holds<'_>,
    m: &cs::Manifest,
) -> Result<Loaded, LoadError> {
    let trimmed = cs::trim_to_pinned(m, &view.pinned);
    if trimmed.signatures.is_empty() {
        return Err(LoadError::NotConfirmed(
            "no signature is from a key this node pins".into(),
        ));
    }
    let path = trimmed_manifest_path(pair);
    crate::fsx::atomic_write(&path, &trimmed.to_bytes())
        .map_err(|e| LoadError::Io(format!("write {}: {e}", path.display())))?;

    let superseded = match runner.load_attested(&pair.file, &path).await {
        LoadOutcome::Loaded => false,
        LoadOutcome::Superseded => true,
        LoadOutcome::Failed(e) => return Err(LoadError::Engine(e)),
    };

    for (height, root) in holds.roots() {
        let unavailable = |why: String| LoadError::PostLoadCheckUnavailable {
            height,
            root: root.to_string(),
            why,
        };
        match rpc.call("getblockhash", json!([height])).await {
            Ok(v) => match v.as_str() {
                Some(at) if at.eq_ignore_ascii_case(root) => {
                    return Err(LoadError::HeldRootOnChain {
                        height,
                        root: root.to_string(),
                    })
                }
                Some(at) if at.len() == 64 && at.bytes().all(|b| b.is_ascii_hexdigit()) => {}
                _ => return Err(unavailable(format!("the answer {v} is not a block hash"))),
            },
            Err(e) if is_height_out_of_range(&e) => {}
            Err(e) => return Err(unavailable(e.to_string())),
        }
    }
    Ok(Loaded {
        height: pair.height,
        signatures: trimmed.signatures.len(),
        superseded,
    })
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

/// Move a snapshot chainstate the app refuses out of the engine's way, as the
/// engine itself does with one that fails its background check. `network_dir`
/// is the datadir on mainnet. Call only with the node stopped. `Ok(None)`
/// when there was none.
pub fn set_aside_snapshot_chainstate(
    network_dir: &Path,
    now_unix: u64,
) -> std::io::Result<Option<PathBuf>> {
    let from = network_dir.join("chainstate_snapshot");
    if !from.exists() {
        return Ok(None);
    }
    let to = network_dir.join(format!("chainstate_snapshot.refused-{now_unix}"));
    std::fs::rename(&from, &to)?;
    Ok(Some(to))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{AppError, AppResult};
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const R_PC: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PC.manifest");
    const R_PCD: &[u8] =
        include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PCD.manifest");
    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_DAT: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-100.dat");
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
        calls: Mutex<Vec<String>>,
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
                calls: Mutex::new(Vec::new()),
            }
        }
        fn methods(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
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
                other => panic!("the loader must never call {other}"),
            }
        }
    }

    /// Records the manifest it was handed and answers as told.
    struct Runner {
        answer: LoadOutcome,
        seen: Mutex<Option<Vec<u8>>>,
    }

    impl Runner {
        fn new(answer: LoadOutcome) -> Self {
            Self {
                answer,
                seen: Mutex::new(None),
            }
        }
    }

    #[async_trait::async_trait]
    impl LoadRunner for Runner {
        async fn load_attested(&self, _file: &Path, manifest: &Path) -> LoadOutcome {
            *self.seen.lock().unwrap() = Some(std::fs::read(manifest).unwrap());
            self.answer.clone()
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
        let runner = Runner::new(LoadOutcome::Loaded);
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
        let calls = node.methods();
        let invalidate = calls.iter().position(|m| m == "invalidateblock").unwrap();
        assert!(
            invalidate < calls.len() - 1,
            "refused before the load: {calls:?}"
        );
        // Section 7, step 4: the names survive the trim. Only verified
        // signers on the list count (D signed too and is on no list), in the
        // list's order.
        assert_eq!(
            snapshot_start::read(tmp.path()),
            Some(StartRecord {
                height: 100,
                block_hash: BASE_100.into(),
                source: StartSource::Confirmed,
                operators: vec!["producer".into(), "confirmer".into()],
            })
        );
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
        let runner = Runner::new(LoadOutcome::Failed("no".into()));
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
        assert!(
            runner.seen.lock().unwrap().is_some(),
            "the engine was asked"
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
