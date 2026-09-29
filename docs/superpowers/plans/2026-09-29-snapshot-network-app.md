# Snapshot Network, app side: diary, producers and confirmers (Implementation Plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every validating node writes down its own chain state at every multiple of 200; a producer on the operator list exports exactly there, checks its export against its node and sends it to easybtx.com; a confirmer on the list co-signs a waiting statement only when its own diary, chain and holds agree field by field; and an end-to-end rehearsal on regtest proves the refusals and the happy path against real engines and the website's own route code.

**Architecture:** `diary.rs` records `gettxoutsetinfo` and `getchaintxstats` at grid heights into `<datadir>/snapshot-diary.json`. `statement_check.rs` compares a statement with the running node (chain, replay context, active chain, 10 confirmations, held roots) and with the diary (block hash, UTXO hash, coins, transaction count), read-only. `snapshot_site.rs` is the client of the website's routes. `snapshot_serve.rs` exports only on the grid and asks a `BeforeOffer` hook before anything is offered; `snapshot_producer.rs` is that hook plus the submission. `snapshot_confirmer.rs` reads `pending`, checks, signs a copy with `signutxosnapshotmanifest`, and sends back its one signature. The Tauri shell drives the diary and the confirmer from the status refresher and the producer from the snapshot keeper, and Copy diagnostics gains a "Snapshots" section. `tests/snapshot_network_regtest.rs` runs five regtest engines and the website's handlers (`site/scripts/snapshot-rendezvous-local.mjs`) together.

**Tech Stack:** Rust (btx-core, Tauri 2 shell), k256 0.13.4 (`ecdsa`), reqwest, serde_json, tokio, mockito (tests); btxd v0.34.9 on regtest and Node 22 for the rehearsal.

**This plan builds on the confirmed-snapshots plans** (`2026-09-29-confirmed-snapshots-core.md` and `2026-09-29-confirmed-snapshots-fast-forward.md`, same folder) and uses their interfaces as they define them: `operators::{Chain, OperatorList, MAINNET_OPERATORS, MAINNET_GENESIS, REGTEST_GENESIS, for_chain, regtest_env, parse_key, hex, hex_decode}`, `confirmed_snapshot::{parse, Manifest, Signed, Statement, Hash32, ChainRules, Refusal, confirming_operators, signature_is_valid, check_with, MAINNET_GRID, REGTEST_GRID, REGTEST_REPLAY_CONTEXT}`, `confirmed_load::{Holds, node_view, load, CliRunner, trimmed_manifest_path}`, `attested_snapshot::{prepare_confirmed, ReadyPair}`, `node::attested_snapshot_record`, `node_api::MatmulTrustedStatus::replay_authority_context`. The website half is `2026-09-29-snapshot-network-site.md`; its handlers answer this plan's client, and its stand-in serves the rehearsal.

## Global Constraints

- Design: `docs/decisions/2026-09-29-every-node-starts-near-the-tip.md` on `origin/claude/cosigned-snapshots` (read it with `git -C /Users/m2promende/repos/easynode show origin/claude/cosigned-snapshots:docs/decisions/2026-09-29-every-node-starts-near-the-tip.md`), approved 2026-09-29 with its choices as proposed. Sections implemented here: 2 (grid of 200), 3 (the diary), 4 (producers), 5 (confirmers), and the app's side of 6 (the website's client). Section 6's routes are the website plan.
- Branch: `claude/snapshot-network`, created from `claude/confirmed-snapshots` after both confirmed-snapshots plans are done (that branch is itself based on `claude/tools-command-window`, head `cb0316b` on origin today). Worktree: `git -C /Users/m2promende/repos/easynode worktree add ../easynode-snapshot-network -b claude/snapshot-network claude/confirmed-snapshots`. All paths below are relative to that worktree.
- Grid: **200** on mainnet (`confirmed_snapshot::MAINNET_GRID`), **100** on regtest (`REGTEST_GRID`), chosen by the chain's genesis hash. A height the tip passed before the app read it is skipped.
- Confirmations: **10** (`snapshot_serve::CONFIRMATIONS_REQUIRED`), as `getblockheader`'s `confirmations` counts them (tip minus height plus one).
- Diary: `<datadir>/snapshot-diary.json`, JSON `{"version":1,"chain_id":"<genesis display hex>","entries":[{"height","block_hash","hash_serialized","coins","chain_tx","recorded_at"}]}`, ascending, newest **100** heights, written with `fsx::atomic_write`. Recorded only when `getmatmultrustedstatus.matmul_validation_mode` is `consensus` and `node::attested_snapshot_record(<datadir>)` does not exist. The fields come from `gettxoutsetinfo` (`height`, `bestblock`, `txouts`, `hash_serialized_3`), kept only when `getblockhash(height) == bestblock` and `height` is on the grid, and from `getchaintxstats 1 <bestblock>` (`txcount`). Measured on regtest with v0.34.9 on 2026-09-29: at 100 these equal the statement `dumptxoutsetattested` writes (block, UTXO hash, coins 101, transaction count 101).
- Statement checks, in this order (`statement_check::check_against_node`), each a refusal with its own reason: shape (`confirmed_snapshot::check_shape`: version, chain id, replay context compiled, shielded commitment, chunk geometry, file size, grid); node's genesis equals the chain id; node's `replay_authority_context` present and equal; the base is `getblockhash(height)`; at least 10 confirmations; no known-invalid block or held root (`confirmed_load::Holds`) at or below the base on the active chain; the diary entry at the height exists and equals the statement in block hash, UTXO hash, coin count, transaction count. Read-only.
- Producer: a node with "Serve a chain snapshot" on and a key, validating (as today); exports only when its tip is exactly on the grid and above the last offered base; waits 10 confirmations; an orphaned base is abandoned until the next grid height (no re-export off the grid); before offering on P2P or sending anywhere it runs `snapshot_producer::check_before_send` (refuse every known-invalid block and held root on this node with `invalidateblock` through `known_invalid`, then the statement checks). Only a producer whose key is on the list (`operators::for_chain(..).operator_of`) sends to easybtx.com; a pair that failed any check is neither offered nor sent. The submission is recorded in `<datadir>/snapshots/submitted.json` (`{"height","statement_hash","at"}`) and retried every 10 minutes until it is.
- Confirmer: a node that validates, signs (`local_signer`), whose key is on the list, and whose diary is allowed (no `attested_assumeutxo`); every **600 s** (200 refresher ticks of 3 s, the first about 2 minutes after the node starts) it reads `GET /api/snapshots/pending?chain=main`, skips statements its operator already signed, checks each, signs a copy at `<datadir>/snapshots/confirm/<statement_hash>.manifest` with `signutxosnapshotmanifest`, verifies the one new signature, and posts a manifest carrying the statement and only that signature. It never signs without a diary match. Refusals are logged once per statement per run and counted in Copy diagnostics.
- The website (routes, replies and limits exactly as the website plan's Global Constraints): `https://easybtx.com`, overridable with `EASYNODE_SNAPSHOT_SITE` only to another `https://` origin or to `http://127.0.0.1:<port>` / `http://localhost:<port>` (no path, no user info); header `x-ebtx-node: ebtx-snapshot-v1` on every write; every body `content-type: application/octet-stream`; parts of at most 4,194,304 bytes; client timeout 120 s. Nothing the website says is trusted: loading still goes through `attested_snapshot::prepare_confirmed` and `confirmed_load::load`, which check everything again.
- Operator list: mainnet **Mende only** (`02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675`) until the owner confirms Aleksander and jpp agreed. Published byte for byte as `crates/btx-core/snapshot-operators.json`, which a test keeps equal to `MAINNET_OPERATORS` and the website's CI compares with its copy. Test chains: `EASYNODE_REGTEST_OPERATORS` (`name=key[,key];name=key`), only for regtest statements.
- CI gates, all must pass before each commit that touches them: in `crates/btx-core` and `apps/node/src-tauri`: `cargo fmt --all --check`, `cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious`, `cargo test --locked`; in `apps/node`: `npx tsc --noEmit`, `npm test`, `npx vite build`.
- The Tauri crate needs `apps/node/src-tauri/resources/node-pkg/` to hold one file for `cargo check`/`test` (CI writes `CI-PLACEHOLDER`): `mkdir -p apps/node/src-tauri/resources/node-pkg && echo placeholder > apps/node/src-tauri/resources/node-pkg/CI-PLACEHOLDER` if it is empty (the folder is gitignored).
- Known flake, not caused by this plan: `node::tests::launch_watch_detects_an_immediate_child_death` fails now and then on macOS when the suite runs in parallel. Rerun it alone: `cargo test --locked --lib -- --exact node::tests::launch_watch_detects_an_immediate_child_death`.
- Commits end with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (`git commit -m "<subject>" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"`).
- User-facing copy: friendly, simple, no hype, no guarantees, no em-dashes.
- Shared test vectors: the fixtures the core plan adds (`crates/btx-core/tests/fixtures/confirmed_snapshot/`). The regtest statement: height 100, block `bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46`, UTXO hash `e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527`, coins 101, transaction count 101, statement hash `11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194`; the mainnet one: 225,927, statement hash `d3ee9312…0482`, file hash `f234192d…eb2c`.

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `crates/btx-core/snapshot-operators.json` | create | The compiled mainnet list as JSON, for the website's copy |
| `crates/btx-core/src/operators.rs` | modify | `published_json()` and the test that keeps the file equal |
| `crates/btx-core/src/fake_node.rs` | create | Test-only scripted node (chain, UTXO answers, `invalidateblock`, a real `signutxosnapshotmanifest`) |
| `crates/btx-core/src/diary.rs` | create | The diary: record at grid heights, load, save, who may record |
| `crates/btx-core/src/confirmed_snapshot.rs` | modify | `check_shape` split out of `check_with`, same rules |
| `crates/btx-core/src/statement_check.rs` | create | A statement against this node and its diary, read-only; `Mismatch` |
| `crates/btx-core/src/snapshot_site.rs` | create | Client of `/api/snapshots/*` |
| `crates/btx-core/src/snapshot_serve.rs` | modify | Export on the grid, abandon an orphaned base, `BeforeOffer` hook |
| `crates/btx-core/src/snapshot_producer.rs` | create | The producer's checks, the submission and its record |
| `crates/btx-core/src/snapshot_confirmer.rs` | create | Read pending, check, sign a copy, send one signature; tally |
| `crates/btx-core/src/diagnostics.rs` | modify | A "Snapshots" section in Copy diagnostics |
| `crates/btx-core/src/lib.rs` | modify | Register the modules |
| `apps/node/src-tauri/src/state.rs` | modify | `snapshot_network` status on `AppState` |
| `apps/node/src-tauri/src/commands.rs` | modify | Refresher: diary and confirmer; keeper: grid, checks, submission; copy |
| `apps/node/src-tauri/src/tools.rs` | modify | Feed the "Snapshots" section |
| `apps/node/index.html`, `apps/node/src/main.ts` | modify | "refreshed every 500 blocks" becomes "every 200 blocks" |
| `crates/btx-core/tests/snapshot_network_regtest.rs` | create | The end-to-end rehearsal (opt-in) |
| `apps/node/CHANGELOG.md` | modify | One entry |

## Tasks

### Task 1: Publish the operator list for the website (`operators.rs`)

**Files:**
- Create: `crates/btx-core/snapshot-operators.json`
- Modify: `crates/btx-core/src/operators.rs` (`published_json`, one test)

**Interfaces:**
- Consumes: `operators::MAINNET_OPERATORS` (core plan, Task 1).
- Produces: `pub fn operators::published_json() -> String`; the file `crates/btx-core/snapshot-operators.json`, which the website plan's `scripts/check-snapshot-operators.mjs` fetches from `https://raw.githubusercontent.com/MendeMatthias/easynode/main/crates/btx-core/snapshot-operators.json`. This task can land on `main` on its own, before the rest of this plan, so the website's check has a file to compare with.

- [ ] **Step 1: Create the branch**

````bash
cd /Users/m2promende/repos/easynode
git fetch origin
git worktree add ../easynode-snapshot-network -b claude/snapshot-network claude/confirmed-snapshots
cd ../easynode-snapshot-network
git log --oneline -1
mkdir -p apps/node/src-tauri/resources/node-pkg && [ -n "$(ls apps/node/src-tauri/resources/node-pkg)" ] || echo placeholder > apps/node/src-tauri/resources/node-pkg/CI-PLACEHOLDER
````

Expected: the last commit is the confirmed-snapshots work (its changelog or engine-check commit). If `claude/confirmed-snapshots` exists only on origin, use `origin/claude/confirmed-snapshots`.

- [ ] **Step 2: Write the failing test**

In `crates/btx-core/src/operators.rs`, the test module ends with `chains_are_named_by_their_genesis_hash`; add the new test after it:

*Test: the published list is the compiled one.* Replace:

````rust
        assert_eq!(Chain::from_genesis_hex(&"00".repeat(32)), None);
    }
}
````

with:

````rust
        assert_eq!(Chain::from_genesis_hex(&"00".repeat(32)), None);
    }

    /// The file the website copies is the compiled list, byte for byte. When
    /// an operator is added, this fails and prints the file to write.
    #[test]
    fn the_published_list_is_the_compiled_one() {
        let expected = published_json();
        assert_eq!(
            include_str!("../snapshot-operators.json"),
            expected,
            "write this to crates/btx-core/snapshot-operators.json:\n{expected}"
        );
    }
}
````

- [ ] **Step 3: Run it to see it fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- operators::`
Expected: compile errors `` cannot find function `published_json` in this scope `` and `` couldn't read `…/crates/btx-core/src/../snapshot-operators.json` ``.

- [ ] **Step 4: Write the function and the file**

*Published_json, above the tests.* Replace:

````rust
#[cfg(test)]
mod tests {
````

with:

````rust
/// The compiled mainnet list as the JSON this repository publishes at
/// `crates/btx-core/snapshot-operators.json`. The website keeps a byte-for-byte
/// copy (`site/src/data/snapshot-operators.json` in the EasyBTX repository)
/// to decide which signatures it stores, and its CI compares the two
/// (`scripts/check-snapshot-operators.mjs` there). Two-space indent, one
/// newline at the end.
pub fn published_json() -> String {
    #[derive(serde::Serialize)]
    struct Entry<'a> {
        name: &'a str,
        keys: &'a [&'a str],
    }
    #[derive(serde::Serialize)]
    struct Published<'a> {
        version: u32,
        main: Vec<Entry<'a>>,
    }
    let published = Published {
        version: 1,
        main: MAINNET_OPERATORS
            .iter()
            .map(|(name, keys)| Entry { name, keys })
            .collect(),
    };
    let mut json = serde_json::to_string_pretty(&published).expect("plain data serializes");
    json.push('\n');
    json
}

#[cfg(test)]
mod tests {
````

Create `crates/btx-core/snapshot-operators.json` (two-space indent, one newline at the end; the website keeps the same bytes):

````json
{
  "version": 1,
  "main": [
    {
      "name": "Mende",
      "keys": [
        "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675"
      ]
    }
  ]
}
````

- [ ] **Step 5: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- operators::`
Expected: `9 passed; 0 failed`.

Sabotage, then undo: change `Mende` to `Mende2` in the JSON file, run the same command, expect `the_published_list_is_the_compiled_one` to FAIL with the message `write this to crates/btx-core/snapshot-operators.json:` followed by the right file; put `Mende` back.

- [ ] **Step 6: Format, lint, commit**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cd ../..
git add crates/btx-core/snapshot-operators.json crates/btx-core/src/operators.rs
git commit -m "core: publish the snapshot operator list for easybtx.com's copy" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 2: The diary (`diary.rs`), and a scripted node for the tests (`fake_node.rs`)

**Files:**
- Create: `crates/btx-core/src/fake_node.rs` (test-only)
- Create: `crates/btx-core/src/diary.rs`
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: `confirmed_snapshot::{MAINNET_GRID, REGTEST_GRID, parse, Signed}`, `operators::{Chain, MAINNET_GENESIS, REGTEST_GENESIS, hex}`, `node::attested_snapshot_record` (core plan, Task 6), `fsx::atomic_write` (exists), `rpc::Rpc` (exists).
- Produces (Tasks 3, 5, 6, 7, 8):
  - `diary::{DIARY_FILE, DIARY_KEEP, DIARY_VERSION}`, `pub struct DiaryEntry { height: u64, block_hash: String, hash_serialized: String, coins: u64, chain_tx: u64, recorded_at: u64 }`, `pub struct Diary { version: u32, chain_id: String, entries: Vec<DiaryEntry> }` with `new(&str)`, `at(u64) -> Option<&DiaryEntry>`, `newest()`, `record(DiaryEntry)`
  - `diary_path(&Path) -> PathBuf`, `load(&Path, chain_id: &str) -> Diary`, `save(&Path, &Diary) -> io::Result<()>`, `summary(&Path) -> Option<String>`, `grid_for(chain_id: &str) -> Option<u64>`, `may_record(validation_mode: Option<&str>, network_dir: &Path) -> bool`
  - `pub enum DiaryOutcome { Recorded(DiaryEntry), AlreadyRecorded(u64), NotOnGrid, TipMoved, NotAllowed }`
  - `pub async fn chain_id(&dyn Rpc) -> Option<String>`, `pub async fn read_at_tip(&dyn Rpc, grid: u64) -> Result<Option<DiaryEntry>, String>`, `pub async fn record_at_tip(&dyn Rpc, dir: &Path, network_dir: &Path, validation_mode: Option<&str>) -> Result<DiaryOutcome, String>`
  - test-only `crate::fake_node::{FakeNode, NodeState, synthetic_hash}`: `FakeNode::new(genesis, tip)`, `with(|&mut NodeState| ..)`, `methods()`, `count(method)`; `NodeState { chain: BTreeMap<u64, String>, side: HashSet<String>, utxo: Option<Value>, chain_tx: u64, replay_context: Option<String>, mode: String, signer: Option<SigningKey>, invalidate_fails: bool }`. It answers `getblockcount`, `getblockhash`, `getblockheader` (confirmations as the engine counts them, -1 off the chain), `gettxoutsetinfo`, `getchaintxstats`, `getmatmultrustedstatus`, `invalidateblock` (moves the chain as the engine does) and `signutxosnapshotmanifest` (appends a real low-S signature to the file), and panics on anything else.

- [ ] **Step 1: Write the scripted node and the failing tests**

Create `crates/btx-core/src/fake_node.rs`:

````rust
//! A scripted node for unit tests of the snapshot network (the diary, the
//! producer's checks, the confirmer): a chain of block hashes, the answers
//! those modules read, `invalidateblock` that moves the chain the way the
//! engine does, and a signer that appends a real signature to a manifest
//! file the way `signutxosnapshotmanifest` does. Test-only.

use crate::confirmed_snapshot as cs;
use crate::error::{AppError, AppResult};
use crate::rpc::Rpc;
use async_trait::async_trait;
use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{Signature, SigningKey};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

/// What the scripted node knows. Change it with [`FakeNode::with`].
pub(crate) struct NodeState {
    /// The active chain, height to hash; height 0 is the genesis hash.
    pub chain: BTreeMap<u64, String>,
    /// Headers the node knows off its active chain (confirmations -1).
    pub side: HashSet<String>,
    /// What `gettxoutsetinfo` answers, `None` for an error.
    pub utxo: Option<Value>,
    /// What `getchaintxstats` answers as `txcount`.
    pub chain_tx: u64,
    pub replay_context: Option<String>,
    pub mode: String,
    /// The key `signutxosnapshotmanifest` signs with.
    pub signer: Option<SigningKey>,
    pub invalidate_fails: bool,
}

pub(crate) struct FakeNode {
    state: Mutex<NodeState>,
    calls: Mutex<Vec<(String, Value)>>,
}

/// A made-up block hash for a height, distinct per height.
pub(crate) fn synthetic_hash(height: u64) -> String {
    format!("{height:064x}")
}

fn rpc_err(code: i64, message: &str) -> AppError {
    AppError::Rpc {
        code,
        message: message.to_string(),
    }
}

impl FakeNode {
    /// A validating node on `genesis` with a synthetic chain up to `tip`.
    pub fn new(genesis: &str, tip: u64) -> Self {
        let mut chain = BTreeMap::new();
        chain.insert(0, genesis.to_string());
        for h in 1..=tip {
            chain.insert(h, synthetic_hash(h));
        }
        Self {
            state: Mutex::new(NodeState {
                chain,
                side: HashSet::new(),
                utxo: None,
                chain_tx: 0,
                replay_context: None,
                mode: "consensus".into(),
                signer: None,
                invalidate_fails: false,
            }),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut NodeState) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    pub fn methods(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(m, _)| m.clone())
            .collect()
    }

    pub fn count(&self, method: &str) -> usize {
        self.methods().iter().filter(|m| *m == method).count()
    }
}

fn tip(s: &NodeState) -> u64 {
    *s.chain.keys().next_back().unwrap_or(&0)
}

#[async_trait]
impl Rpc for FakeNode {
    async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((method.to_string(), params.clone()));
        let mut s = self.state.lock().unwrap();
        match method {
            "getblockcount" => Ok(json!(tip(&s))),
            "getblockhash" => {
                let h = params[0].as_u64().unwrap_or(u64::MAX);
                s.chain
                    .get(&h)
                    .map(|hash| json!(hash))
                    .ok_or_else(|| rpc_err(-8, "Block height out of range"))
            }
            "getblockheader" => {
                let hash = params[0].as_str().unwrap_or_default().to_string();
                let top = tip(&s);
                if let Some((h, _)) = s.chain.iter().find(|(_, v)| **v == hash) {
                    Ok(json!({ "hash": hash, "height": h, "confirmations": top - h + 1 }))
                } else if s.side.contains(&hash) {
                    Ok(json!({ "hash": hash, "confirmations": -1 }))
                } else {
                    Err(rpc_err(-5, "Block not found"))
                }
            }
            "gettxoutsetinfo" => s
                .utxo
                .clone()
                .ok_or_else(|| rpc_err(-1, "Unable to read UTXO set")),
            "getchaintxstats" => Ok(json!({ "txcount": s.chain_tx })),
            "getmatmultrustedstatus" => Ok(json!({
                "local_signer": s.signer.is_some(),
                "serves_attestations": true,
                "matmul_validation_mode": s.mode,
                "trusted_mirror": s.mode == "trusted",
                "replay_authority_context": s.replay_context,
            })),
            "invalidateblock" => {
                if s.invalidate_fails {
                    return Err(rpc_err(-1, "Failed to invalidate"));
                }
                let hash = params[0].as_str().unwrap_or_default().to_string();
                let at = s.chain.iter().find(|(_, v)| **v == hash).map(|(h, _)| *h);
                match at {
                    Some(h) => {
                        let gone: Vec<u64> = s.chain.range(h..).map(|(k, _)| *k).collect();
                        for k in gone {
                            let v = s.chain.remove(&k).unwrap();
                            s.side.insert(v);
                        }
                        Ok(Value::Null)
                    }
                    None if s.side.contains(&hash) => Ok(Value::Null),
                    None => Err(rpc_err(-5, "Block not found")),
                }
            }
            "signutxosnapshotmanifest" => {
                let path = params[0].as_str().unwrap_or_default().to_string();
                let key = s
                    .signer
                    .clone()
                    .ok_or_else(|| rpc_err(-1, "requires a configured local signer"))?;
                let bytes = std::fs::read(&path).map_err(|e| rpc_err(-22, &e.to_string()))?;
                let mut m = cs::parse(&bytes).map_err(|e| rpc_err(-22, &e.to_string()))?;
                let pubkey: [u8; 33] = key
                    .verifying_key()
                    .to_encoded_point(true)
                    .as_bytes()
                    .try_into()
                    .unwrap();
                if m.signatures.iter().any(|x| x.key == pubkey) {
                    return Err(rpc_err(-1, "Local signer has already signed this manifest"));
                }
                let sig: Signature = key.sign_prehash(&m.statement.hash().0).unwrap();
                m.signatures.push(cs::Signed {
                    key: pubkey,
                    der: sig.to_der().as_bytes().to_vec(),
                });
                std::fs::write(&path, m.to_bytes()).map_err(|e| rpc_err(-1, &e.to_string()))?;
                Ok(json!({
                    "manifest_path": path,
                    "signatures": m.signatures.len(),
                    "signer": crate::operators::hex(&pubkey),
                }))
            }
            other => panic!("the fake node was asked {other}, which it does not script"),
        }
    }
}
````

Create `crates/btx-core/src/diary.rs` with only the test module:

````rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_node::FakeNode;
    use crate::operators::{MAINNET_GENESIS, REGTEST_GENESIS};

    // The spike's regtest export at 100 (the fixture regtest-*.manifest).
    const R_BLOCK: &str = "bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46";
    const R_UTXO: &str = "e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527";

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

    fn entry(height: u64) -> DiaryEntry {
        DiaryEntry {
            height,
            block_hash: format!("{height:064x}"),
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
        let out = record_at_tip(&node, dir.path(), dir.path(), Some("consensus"))
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
        let d = load(dir.path(), REGTEST_GENESIS);
        assert_eq!(d.at(100), Some(&e));
        assert_eq!(
            node.methods(),
            vec![
                "getblockhash",
                "getblockcount",
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
        record_at_tip(&node, dir.path(), dir.path(), Some("consensus"))
            .await
            .unwrap();
        let again = record_at_tip(&node, dir.path(), dir.path(), Some("consensus"))
            .await
            .unwrap();
        assert_eq!(again, DiaryOutcome::AlreadyRecorded(100));
        assert_eq!(node.count("gettxoutsetinfo"), 1);
    }

    #[tokio::test]
    async fn off_the_grid_nothing_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let node = FakeNode::new(MAINNET_GENESIS, 226_001);
        let out = record_at_tip(&node, dir.path(), dir.path(), Some("consensus"))
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::NotOnGrid);
        assert_eq!(node.count("gettxoutsetinfo"), 0);
    }

    #[tokio::test]
    async fn an_answer_for_a_block_that_moved_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        // The tip moved on while the engine read: it answers for 101.
        let node = regtest_at_100();
        node.with(|s| {
            s.utxo.as_mut().unwrap()["height"] = json!(101);
        });
        let out = record_at_tip(&node, dir.path(), dir.path(), Some("consensus"))
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::TipMoved);
        // A reorg while the engine read: the answer names a block that is no
        // longer the one at 100.
        let node = regtest_at_100();
        node.with(|s| {
            s.utxo.as_mut().unwrap()["bestblock"] = json!("ff".repeat(32));
        });
        let out = record_at_tip(&node, dir.path(), dir.path(), Some("consensus"))
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::TipMoved);
        assert!(!diary_path(dir.path()).exists());
    }

    #[tokio::test]
    async fn a_mirror_and_a_node_on_a_signed_snapshot_write_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let node = regtest_at_100();
        for mode in [Some("trusted"), None] {
            let out = record_at_tip(&node, dir.path(), dir.path(), mode)
                .await
                .unwrap();
            assert_eq!(out, DiaryOutcome::NotAllowed);
        }
        let record = crate::node::attested_snapshot_record(dir.path());
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, b"v2").unwrap();
        let out = record_at_tip(&node, dir.path(), dir.path(), Some("consensus"))
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::NotAllowed);
        assert!(node.methods().is_empty(), "not even a read");
        std::fs::remove_file(&record).unwrap();
        assert!(may_record(Some("consensus"), dir.path()));
    }

    #[test]
    fn the_diary_keeps_the_newest_hundred_heights_one_each() {
        let mut d = Diary::new(MAINNET_GENESIS);
        for i in (1..=120u64).rev() {
            d.record(entry(i * 200));
        }
        d.record(DiaryEntry {
            coins: 7,
            ..entry(24_000)
        });
        assert_eq!(d.entries.len(), DIARY_KEEP);
        assert_eq!(d.entries.first().unwrap().height, 4_200);
        assert_eq!(d.newest().unwrap().height, 24_000);
        assert_eq!(d.at(24_000).unwrap().coins, 7);
        assert!(d.at(4_000).is_none());
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
        assert_eq!(grid_for(MAINNET_GENESIS), Some(200));
        assert_eq!(grid_for(REGTEST_GENESIS), Some(100));
        assert_eq!(grid_for(&"00".repeat(32)), None);
    }
}
````

In `crates/btx-core/src/lib.rs`, after `pub mod diagnostics;` add `pub mod diary;`, and after `pub mod esplora_sidecar;` add:

````rust
#[cfg(test)]
pub(crate) mod fake_node;
````

- [ ] **Step 2: Run them to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- diary::`
Expected: compile errors, among them `` cannot find function `record_at_tip` in this scope `` and `` cannot find struct, variant or union type `DiaryEntry` ``.

- [ ] **Step 3: Write the diary**

Insert above `#[cfg(test)]` in `crates/btx-core/src/diary.rs`:

````rust
//! The diary: what this node's own chain state was at every grid height.
//!
//! A snapshot statement names a block and the UTXO set after it: the set's
//! hash (`hash_serialized_3`), its coin count and the chain's transaction
//! count. A confirmer signs a statement only when its OWN node wrote down
//! the same four things at that height (`crate::statement_check`), because
//! `signutxosnapshotmanifest` signs blindly: a node at height 0 co-signed a
//! statement about block 100 in the spike of 2026-09-29. This diary is the
//! only real check a co-signature carries.
//!
//! WHEN. `gettxoutsetinfo` answers only at the tip, so the app reads it when
//! the tip is on the grid (every 200 blocks on mainnet, section 2 of
//! docs/decisions/2026-09-29-every-node-starts-near-the-tip.md) and keeps the
//! answer only if the block it names is still the one at its height. A height
//! the tip passed before the app read it is skipped; the next chance is 200
//! blocks later.
//!
//! WHO. Only a node whose chain state rests on its own checks: it validates
//! (a mirror holds a UTXO set it never checked), and it does not run on a
//! signed snapshot whose background check is still going
//! (`node::attested_snapshot_record`). Otherwise one snapshot could vouch for
//! the next.
//!
//! WHERE. `<datadir>/snapshot-diary.json`, the newest 100 heights, written
//! atomically, for one chain: a diary from another chain reads as empty.

use crate::confirmed_snapshot as cs;
use crate::operators::Chain;
use crate::rpc::Rpc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

pub const DIARY_FILE: &str = "snapshot-diary.json";
/// Heights kept: 100 heights of 200 blocks is about a month of mainnet.
pub const DIARY_KEEP: usize = 100;
pub const DIARY_VERSION: u32 = 1;

/// One height, as this node's engine reported it. Hashes are display hex.
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

pub fn save(dir: &Path, diary: &Diary) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(diary).map_err(std::io::Error::other)?;
    crate::fsx::atomic_write(&diary_path(dir), &json)
}

/// One line for Copy diagnostics: how many heights the diary holds and the
/// newest, whatever chain it is for. `None` when there is no diary.
pub fn summary(dir: &Path) -> Option<String> {
    let d: Diary = serde_json::from_slice(&std::fs::read(diary_path(dir)).ok()?).ok()?;
    let newest = d.newest()?;
    let n = d.entries.len();
    Some(format!(
        "{n} {}, newest {} (block {})",
        if n == 1 { "height" } else { "heights" },
        newest.height,
        &newest.block_hash[..16.min(newest.block_hash.len())]
    ))
}

/// The grid of the chain a genesis hash names: 200 on mainnet, 100 on regtest.
pub fn grid_for(chain_id: &str) -> Option<u64> {
    match Chain::from_genesis_hex(chain_id)? {
        Chain::Main => Some(cs::MAINNET_GRID as u64),
        Chain::Regtest => Some(cs::REGTEST_GRID as u64),
    }
}

/// Whether this node may write in its diary: it validates, and it does not
/// run on a signed snapshot whose background check has not finished.
/// `network_dir` is the datadir on mainnet.
pub fn may_record(validation_mode: Option<&str>, network_dir: &Path) -> bool {
    validation_mode.is_some_and(|m| m.trim().eq_ignore_ascii_case("consensus"))
        && !crate::node::attested_snapshot_record(network_dir).exists()
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
    /// height any more, or off the grid: the tip moved during the read.
    TipMoved,
    /// A mirror, or a node on a signed snapshot still being checked.
    NotAllowed,
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

async fn hash_at(rpc: &dyn Rpc, height: u64) -> Result<String, String> {
    rpc.call("getblockhash", json!([height]))
        .await
        .map_err(|e| format!("getblockhash {height}: {e}"))?
        .as_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "getblockhash answered no hash".to_string())
}

/// The chain this node is on: its genesis hash, display hex. `None` when it
/// did not answer.
pub async fn chain_id(rpc: &dyn Rpc) -> Option<String> {
    hash_at(rpc, 0).await.ok()
}

/// Read the UTXO set at the tip. `Ok(None)` when the answer is not one to
/// keep: off the grid, or for a block no longer at its height.
pub async fn read_at_tip(rpc: &dyn Rpc, grid: u64) -> Result<Option<DiaryEntry>, String> {
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
    if height == 0 || height % grid != 0 || hash_at(rpc, height).await? != block {
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

/// One diary step, cheap when there is nothing to do: the status refresher
/// calls it on a tick whose tip is on the grid, and the producer right
/// before it exports. `dir` holds the diary (the datadir); `network_dir` is
/// where the engine keeps its chain state (the same folder on mainnet).
pub async fn record_at_tip(
    rpc: &dyn Rpc,
    dir: &Path,
    network_dir: &Path,
    validation_mode: Option<&str>,
) -> Result<DiaryOutcome, String> {
    if !may_record(validation_mode, network_dir) {
        return Ok(DiaryOutcome::NotAllowed);
    }
    let genesis = chain_id(rpc)
        .await
        .ok_or_else(|| "getblockhash 0 did not answer".to_string())?;
    let grid = grid_for(&genesis).ok_or_else(|| format!("chain {genesis} has no grid"))?;
    let tip = rpc
        .call("getblockcount", json!([]))
        .await
        .map_err(|e| format!("getblockcount: {e}"))?
        .as_u64()
        .ok_or_else(|| "getblockcount answered no number".to_string())?;
    if tip == 0 || tip % grid != 0 {
        return Ok(DiaryOutcome::NotOnGrid);
    }
    let mut diary = load(dir, &genesis);
    if let Some(known) = diary.at(tip).map(|e| e.block_hash.clone()) {
        if hash_at(rpc, tip).await? == known {
            return Ok(DiaryOutcome::AlreadyRecorded(tip));
        }
    }
    let Some(entry) = read_at_tip(rpc, grid).await? else {
        return Ok(DiaryOutcome::TipMoved);
    };
    diary.record(entry.clone());
    save(dir, &diary).map_err(|e| format!("writing the diary: {e}"))?;
    Ok(DiaryOutcome::Recorded(entry))
}
````

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- diary::`
Expected: `7 passed; 0 failed`. The first test reads the spike's regtest values as the engine reports them (block `bd23c642…2c46`, UTXO hash `e611efee…d527`, 101 coins, 101 transactions) and checks the exact RPC sequence: `getblockhash`, `getblockcount`, `gettxoutsetinfo`, `getblockhash`, `getchaintxstats`.

- [ ] **Step 5: Format, lint, commit**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cd ../..
git add crates/btx-core/src/diary.rs crates/btx-core/src/fake_node.rs crates/btx-core/src/lib.rs
git commit -m "core: the diary, this node's own chain state at every grid height" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 3: A statement against this node and its diary (`statement_check.rs`)

**Files:**
- Modify: `crates/btx-core/src/confirmed_snapshot.rs` (`check_shape` split out of `check_with`, same rules and order of refusals for every existing test)
- Create: `crates/btx-core/src/statement_check.rs`
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: Task 2 (`Diary`, `DiaryEntry`, `FakeNode`), `confirmed_snapshot::{ChainRules, Statement, Refusal, parse, REGTEST_REPLAY_CONTEXT}`, `confirmed_load::Holds` (core plan, Task 4), `known_invalid::HeldBranch`, `node_api::get_matmul_trusted_status` with `replay_authority_context` (core plan, Task 4).
- Produces (Tasks 5, 6, 8):
  - `pub fn confirmed_snapshot::check_shape(&Statement, &ChainRules) -> Result<u64, Refusal>` (the height)
  - `pub enum statement_check::Mismatch { Unreadable(String), ChainId { node }, ReplayContext { node: Option<String> }, BaseNotOnActiveChain { height }, TooFewConfirmations { have: i64, need: u64 }, HeldRootOnActiveChain { height, root }, HeldNotRefused(String), NoDiaryEntry { height }, BlockHash { diary, statement }, HashSerialized { diary, statement }, Coins { diary, statement }, ChainTx { diary, statement }, NodeUnanswered(String) }` (`Display` one line; `From<Refusal>`)
  - `pub fn compare_with_diary(&Statement, &Diary) -> Result<(), Mismatch>`
  - `pub async fn check_against_node(&dyn Rpc, &Statement, &ChainRules, &Diary, &Holds<'_>, min_confirmations: u64) -> Result<(), Mismatch>`

- [ ] **Step 1: Write the failing tests**

Create `crates/btx-core/src/statement_check.rs` with only the test module. It has one sabotage per diary field (block hash, UTXO hash, coin count, transaction count), a node on another chain, another and a missing replay context, nine confirmations, a sibling at the base's height, a node below the base, a refused block under the base, and an off-grid statement that is refused before the node is asked anything:

````rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::diary::DiaryEntry;
    use crate::fake_node::FakeNode;
    use crate::known_invalid::HeldBranch;
    use crate::operators::REGTEST_GENESIS;

    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_BLOCK: &str = "bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46";
    const R_UTXO: &str = "e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527";
    const ROOT: &str = "8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c";
    const HOLD_AT_50: &[HeldBranch] = &[HeldBranch {
        height: 50,
        root: ROOT,
        why: "a test hold",
    }];

    type Edit = Box<dyn Fn(&mut DiaryEntry)>;

    fn statement() -> Statement {
        cs::parse(R_P).unwrap().statement
    }

    fn rules() -> ChainRules {
        ChainRules::for_statement(&statement(), None).unwrap()
    }

    fn diary() -> Diary {
        let mut d = Diary::new(REGTEST_GENESIS);
        d.record(DiaryEntry {
            height: 100,
            block_hash: R_BLOCK.into(),
            hash_serialized: R_UTXO.into(),
            coins: 101,
            chain_tx: 101,
            recorded_at: 0,
        });
        d
    }

    /// The node that exported the spike's statement, ten blocks later.
    fn node() -> FakeNode {
        let n = FakeNode::new(REGTEST_GENESIS, 110);
        n.with(|s| {
            s.chain.insert(100, R_BLOCK.into());
            s.replay_context = Some(cs::REGTEST_REPLAY_CONTEXT.into());
        });
        n
    }

    async fn check(n: &FakeNode, d: &Diary, holds: &Holds<'_>) -> Result<(), Mismatch> {
        check_against_node(n, &statement(), &rules(), d, holds, 10).await
    }

    #[tokio::test]
    async fn the_spikes_statement_matches_the_node_that_made_it() {
        assert_eq!(check(&node(), &diary(), &Holds::none()).await, Ok(()));
    }

    /// One sabotage per field: each is the only thing wrong, and each is
    /// refused with its own reason.
    #[tokio::test]
    async fn a_diary_that_differs_in_any_one_field_is_refused() {
        let cases: Vec<(&str, Edit, Mismatch)> = vec![
            (
                "block hash",
                Box::new(|e| e.block_hash = "11".repeat(32)),
                Mismatch::BlockHash {
                    diary: "11".repeat(32),
                    statement: R_BLOCK.into(),
                },
            ),
            (
                "UTXO hash",
                Box::new(|e| e.hash_serialized = "22".repeat(32)),
                Mismatch::HashSerialized {
                    diary: "22".repeat(32),
                    statement: R_UTXO.into(),
                },
            ),
            (
                "coin count",
                Box::new(|e| e.coins = 102),
                Mismatch::Coins {
                    diary: 102,
                    statement: 101,
                },
            ),
            (
                "transaction count",
                Box::new(|e| e.chain_tx = 100),
                Mismatch::ChainTx {
                    diary: 100,
                    statement: 101,
                },
            ),
        ];
        for (what, edit, want) in cases {
            let mut d = diary();
            let mut e = d.at(100).unwrap().clone();
            edit(&mut e);
            d.record(e);
            assert_eq!(
                check(&node(), &d, &Holds::none()).await,
                Err(want),
                "{what}"
            );
        }
        assert_eq!(
            check(&node(), &Diary::new(REGTEST_GENESIS), &Holds::none()).await,
            Err(Mismatch::NoDiaryEntry { height: 100 })
        );
    }

    #[tokio::test]
    async fn a_node_on_another_chain_or_replay_context_is_refused() {
        let n = node();
        n.with(|s| {
            s.chain.insert(0, crate::operators::MAINNET_GENESIS.into());
        });
        assert_eq!(
            check(&n, &diary(), &Holds::none()).await,
            Err(Mismatch::ChainId {
                node: crate::operators::MAINNET_GENESIS.into()
            })
        );
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
    }

    #[tokio::test]
    async fn a_base_off_the_active_chain_or_under_ten_confirmations_is_refused() {
        // At tip 108 the base has 9 confirmations.
        let n = node();
        n.with(|s| {
            s.chain.remove(&110);
            s.chain.remove(&109);
        });
        assert_eq!(
            check(&n, &diary(), &Holds::none()).await,
            Err(Mismatch::TooFewConfirmations { have: 9, need: 10 })
        );
        // Exactly ten (tip 109) is enough.
        let n = node();
        n.with(|s| {
            s.chain.remove(&110);
        });
        assert_eq!(check(&n, &diary(), &Holds::none()).await, Ok(()));
        // A sibling at 100 on the active chain.
        let n = node();
        n.with(|s| {
            s.chain.insert(100, "44".repeat(32));
        });
        assert_eq!(
            check(&n, &diary(), &Holds::none()).await,
            Err(Mismatch::BaseNotOnActiveChain { height: 100 })
        );
        // A node that has not reached 100 at all.
        let n = FakeNode::new(REGTEST_GENESIS, 60);
        n.with(|s| s.replay_context = Some(cs::REGTEST_REPLAY_CONTEXT.into()));
        assert_eq!(
            check(&n, &diary(), &Holds::none()).await,
            Err(Mismatch::BaseNotOnActiveChain { height: 100 })
        );
    }

    #[tokio::test]
    async fn a_refused_block_under_the_base_is_refused() {
        let n = node();
        n.with(|s| {
            s.chain.insert(50, ROOT.into());
        });
        let holds = Holds {
            invalid: &[],
            held: HOLD_AT_50,
        };
        assert_eq!(
            check(&n, &diary(), &holds).await,
            Err(Mismatch::HeldRootOnActiveChain {
                height: 50,
                root: ROOT.into()
            })
        );
        // The same hold, not on this chain: nothing to say.
        assert_eq!(check(&node(), &diary(), &holds).await, Ok(()));
        assert_eq!(n.count("invalidateblock"), 0, "read-only");
    }

    #[tokio::test]
    async fn an_off_grid_statement_is_refused_before_the_node_is_asked() {
        let mut raw = *statement().raw();
        raw[65..69].copy_from_slice(&150i32.to_le_bytes());
        let st = Statement::from_raw(raw);
        let n = node();
        let got = check_against_node(&n, &st, &rules(), &diary(), &Holds::none(), 10).await;
        assert_eq!(
            got,
            Err(Mismatch::Unreadable(
                "height 150 is not a multiple of 100".into()
            ))
        );
        assert!(n.methods().is_empty());
    }
}
````

In `crates/btx-core/src/lib.rs`, after `pub mod snapshot_serve;` add `pub mod statement_check;`.

- [ ] **Step 2: Run them to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- statement_check::`
Expected: compile errors, among them `` cannot find function `check_against_node` in this scope `` and `` cannot find type `Mismatch` in this scope ``.

- [ ] **Step 3: Split the shape out of `check_with`**

In `crates/btx-core/src/confirmed_snapshot.rs`:

*Check_shape out of check_with.* Replace:

````rust
/// Every rule of section 7, step 1.
pub fn check_with(
    manifest: &Manifest,
    node: &NodeView,
    rules: &ChainRules,
) -> Result<Confirmed, Refusal> {
    let st = &manifest.statement;
    if st.version() != STATEMENT_VERSION {
        return Err(Refusal::UnsupportedVersion(st.version()));
    }
    if st.chain_id() != rules.genesis {
        return Err(Refusal::UnknownChain(st.chain_id().display_hex()));
    }
    if st.replay_context() != rules.replay_context {
        return Err(Refusal::WrongReplayContext);
    }
    node_agrees(st, node)?;
    if st.shielded().is_null() {
        return Err(Refusal::MissingShieldedCommitment);
    }
    let size = st.file_size();
    let chunk = st.chunk_size();
    if size == 0
        || st.file_hash().is_null()
        || !(MIN_CHUNK..=MAX_CHUNK).contains(&chunk)
        || st.chunk_count() as u64 != 1 + (size - 1) / chunk as u64
    {
        return Err(Refusal::BadGeometry);
    }
    if size > crate::attested_snapshot::MAX_SNAPSHOT_BYTES {
        return Err(Refusal::FileTooLarge(size));
    }
    let height = st.height() as i64;
    if height <= 0 || height % rules.grid as i64 != 0 {
        return Err(Refusal::OffGrid {
            height,
            grid: rules.grid,
        });
    }
    if height as u64 <= node.start_height {
        return Err(Refusal::NotAboveStart {
            height,
            start: node.start_height,
        });
    }
    let operators = confirming_operators(manifest, &rules.operators)?;
    if operators.len() < 2 {
        return Err(Refusal::TooFewOperators(operators));
    }
    if !manifest
        .signatures
        .iter()
        .any(|s| node.pinned.contains(&s.key))
    {
        return Err(Refusal::NoPinnedSigner);
    }
    Ok(Confirmed {
        manifest: manifest.clone(),
        chain: rules.chain,
        height: height as u64,
        statement_hash: st.hash(),
        operators,
    })
}

````

with:

````rust
/// The statement on its own, under its chain's rules: version 2, the chain's
/// genesis and replay context, a shielded commitment, the engine's chunk
/// geometry, a file this app downloads, a height on the grid. Returns the
/// height. Neither the signatures nor the node are looked at; the producer
/// and the confirmer (`crate::statement_check`) use this before comparing
/// the statement with their own node.
pub fn check_shape(st: &Statement, rules: &ChainRules) -> Result<u64, Refusal> {
    if st.version() != STATEMENT_VERSION {
        return Err(Refusal::UnsupportedVersion(st.version()));
    }
    if st.chain_id() != rules.genesis {
        return Err(Refusal::UnknownChain(st.chain_id().display_hex()));
    }
    if st.replay_context() != rules.replay_context {
        return Err(Refusal::WrongReplayContext);
    }
    if st.shielded().is_null() {
        return Err(Refusal::MissingShieldedCommitment);
    }
    let size = st.file_size();
    let chunk = st.chunk_size();
    if size == 0
        || st.file_hash().is_null()
        || !(MIN_CHUNK..=MAX_CHUNK).contains(&chunk)
        || st.chunk_count() as u64 != 1 + (size - 1) / chunk as u64
    {
        return Err(Refusal::BadGeometry);
    }
    if size > crate::attested_snapshot::MAX_SNAPSHOT_BYTES {
        return Err(Refusal::FileTooLarge(size));
    }
    let height = st.height() as i64;
    if height <= 0 || height % rules.grid as i64 != 0 {
        return Err(Refusal::OffGrid {
            height,
            grid: rules.grid,
        });
    }
    Ok(height as u64)
}

/// Every rule of section 7, step 1.
pub fn check_with(
    manifest: &Manifest,
    node: &NodeView,
    rules: &ChainRules,
) -> Result<Confirmed, Refusal> {
    let st = &manifest.statement;
    let height = check_shape(st, rules)?;
    node_agrees(st, node)?;
    if height <= node.start_height {
        return Err(Refusal::NotAboveStart {
            height: height as i64,
            start: node.start_height,
        });
    }
    let operators = confirming_operators(manifest, &rules.operators)?;
    if operators.len() < 2 {
        return Err(Refusal::TooFewOperators(operators));
    }
    if !manifest
        .signatures
        .iter()
        .any(|s| node.pinned.contains(&s.key))
    {
        return Err(Refusal::NoPinnedSigner);
    }
    Ok(Confirmed {
        manifest: manifest.clone(),
        chain: rules.chain,
        height,
        statement_hash: st.hash(),
        operators,
    })
}

````

Run: `cd crates/btx-core && cargo test --locked --lib -- confirmed_snapshot:: attested_snapshot:: confirmed_load::`
Expected: every test still passes (measured: `41 passed; 0 failed; 1 ignored`). The refusals and their order are unchanged for every case the core plan tests; only a manifest that is both off in shape and on another chain than the node now names the shape first.

- [ ] **Step 4: Write the check**

Insert above `#[cfg(test)]` in `crates/btx-core/src/statement_check.rs`:

````rust
//! Does a snapshot statement say what THIS node saw? The one check behind
//! every co-signature and every submission.
//!
//! `signutxosnapshotmanifest` signs whatever it is handed (a node at height 0
//! co-signed a statement about block 100 in the spike of 2026-09-29), so a
//! signature means something only because the app compares first. A producer
//! runs this on its own export before it sends it anywhere (section 4 of
//! docs/decisions/2026-09-29-every-node-starts-near-the-tip.md); a confirmer
//! runs it on every statement waiting on easybtx.com before it signs
//! (section 5). In this order, each a reason to refuse:
//!
//! 1. the statement's shape under its chain's rules
//!    (`confirmed_snapshot::check_shape`);
//! 2. the node is on the statement's chain and reports its replay context;
//! 3. the base is the block at that height on the node's ACTIVE chain, with
//!    at least 10 confirmations;
//! 4. no block the app refuses (`known_invalid`) is on that chain at or
//!    below the base;
//! 5. the node's own diary (`crate::diary`) has that height, and every field
//!    matches: block hash, UTXO set hash, coin count, transaction count.
//!
//! Read-only: nothing here changes the node.

use crate::confirmed_load::Holds;
use crate::confirmed_snapshot::{self as cs, ChainRules, Statement};
use crate::diary::Diary;
use crate::rpc::Rpc;
use serde_json::json;

/// Why a statement is not signed or sent. `Display` is one plain line for
/// the log and for Copy diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    /// The manifest does not read, or breaks its chain's rules.
    Unreadable(String),
    /// The node is on another chain than the statement names.
    ChainId {
        node: String,
    },
    /// The node reports no replay context, or another one.
    ReplayContext {
        node: Option<String>,
    },
    BaseNotOnActiveChain {
        height: u64,
    },
    TooFewConfirmations {
        have: i64,
        need: u64,
    },
    HeldRootOnActiveChain {
        height: u64,
        root: String,
    },
    /// Producer only: a refused block could not be refused on this node.
    HeldNotRefused(String),
    NoDiaryEntry {
        height: u64,
    },
    BlockHash {
        diary: String,
        statement: String,
    },
    HashSerialized {
        diary: String,
        statement: String,
    },
    Coins {
        diary: u64,
        statement: u64,
    },
    ChainTx {
        diary: u64,
        statement: u64,
    },
    /// The node did not answer a read.
    NodeUnanswered(String),
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mismatch::Unreadable(e) => write!(f, "the statement does not check out: {e}"),
            Mismatch::ChainId { node } => {
                write!(f, "this node is on chain {node}, not the statement's")
            }
            Mismatch::ReplayContext { node: None } => {
                write!(f, "this node reports no replay context")
            }
            Mismatch::ReplayContext { node: Some(c) } => {
                write!(f, "this node's replay context is {c}, not the statement's")
            }
            Mismatch::BaseNotOnActiveChain { height } => write!(
                f,
                "the statement's block is not the one at {height} on this node's chain"
            ),
            Mismatch::TooFewConfirmations { have, need } => {
                write!(
                    f,
                    "the block has {have} confirmations here, {need} are needed"
                )
            }
            Mismatch::HeldRootOnActiveChain { height, root } => write!(
                f,
                "block {root} at {height}, which the app refuses, is on this node's chain"
            ),
            Mismatch::HeldNotRefused(e) => write!(f, "a refused block could not be refused: {e}"),
            Mismatch::NoDiaryEntry { height } => {
                write!(f, "this node's diary has nothing at {height}")
            }
            Mismatch::BlockHash { diary, statement } => write!(
                f,
                "block hash differs from the diary (diary {diary}, statement {statement})"
            ),
            Mismatch::HashSerialized { diary, statement } => write!(
                f,
                "UTXO set hash differs from the diary (diary {diary}, statement {statement})"
            ),
            Mismatch::Coins { diary, statement } => write!(
                f,
                "coin count differs from the diary (diary {diary}, statement {statement})"
            ),
            Mismatch::ChainTx { diary, statement } => write!(
                f,
                "transaction count differs from the diary (diary {diary}, statement {statement})"
            ),
            Mismatch::NodeUnanswered(e) => write!(f, "the node did not answer: {e}"),
        }
    }
}

impl From<cs::Refusal> for Mismatch {
    fn from(r: cs::Refusal) -> Self {
        Mismatch::Unreadable(r.to_string())
    }
}

/// Field by field against the diary entry at the statement's height.
pub fn compare_with_diary(st: &Statement, diary: &Diary) -> Result<(), Mismatch> {
    let height = st.height().max(0) as u64;
    let e = diary.at(height).ok_or(Mismatch::NoDiaryEntry { height })?;
    let block = st.block_hash().display_hex();
    if !e.block_hash.eq_ignore_ascii_case(&block) {
        return Err(Mismatch::BlockHash {
            diary: e.block_hash.clone(),
            statement: block,
        });
    }
    let utxo = st.hash_serialized().display_hex();
    if !e.hash_serialized.eq_ignore_ascii_case(&utxo) {
        return Err(Mismatch::HashSerialized {
            diary: e.hash_serialized.clone(),
            statement: utxo,
        });
    }
    if e.coins != st.coins() {
        return Err(Mismatch::Coins {
            diary: e.coins,
            statement: st.coins(),
        });
    }
    if e.chain_tx != st.chain_tx() {
        return Err(Mismatch::ChainTx {
            diary: e.chain_tx,
            statement: st.chain_tx(),
        });
    }
    Ok(())
}

async fn block_at(rpc: &dyn Rpc, height: u64) -> Option<String> {
    rpc.call("getblockhash", json!([height]))
        .await
        .ok()
        .and_then(|v| v.as_str().map(str::to_ascii_lowercase))
}

/// Steps 1 to 5 of the module doc against the running node, read-only.
/// `min_confirmations` is `snapshot_serve::CONFIRMATIONS_REQUIRED` (10).
pub async fn check_against_node(
    rpc: &dyn Rpc,
    st: &Statement,
    rules: &ChainRules,
    diary: &Diary,
    holds: &Holds<'_>,
    min_confirmations: u64,
) -> Result<(), Mismatch> {
    let height = cs::check_shape(st, rules)?;

    let genesis = block_at(rpc, 0)
        .await
        .ok_or_else(|| Mismatch::NodeUnanswered("getblockhash 0".into()))?;
    if genesis != st.chain_id().display_hex() {
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

    let block = st.block_hash().display_hex();
    if block_at(rpc, height).await.as_deref() != Some(block.as_str()) {
        return Err(Mismatch::BaseNotOnActiveChain { height });
    }
    let have = rpc
        .call("getblockheader", json!([block, true]))
        .await
        .map_err(|e| Mismatch::NodeUnanswered(format!("getblockheader: {e}")))?["confirmations"]
        .as_i64()
        .unwrap_or(-1);
    if have < min_confirmations as i64 {
        return Err(Mismatch::TooFewConfirmations {
            have,
            need: min_confirmations,
        });
    }

    let roots = holds
        .invalid
        .iter()
        .map(|b| (b.height, b.hash))
        .chain(holds.held.iter().map(|h| (h.height, h.root)));
    for (root_height, root) in roots.filter(|(h, _)| *h <= height) {
        if block_at(rpc, root_height).await.as_deref() == Some(root) {
            return Err(Mismatch::HeldRootOnActiveChain {
                height: root_height,
                root: root.to_string(),
            });
        }
    }

    compare_with_diary(st, diary)
}
````

- [ ] **Step 5: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- statement_check::`
Expected: `6 passed; 0 failed`.

- [ ] **Step 6: Format, lint, commit**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cd ../..
git add crates/btx-core/src/confirmed_snapshot.rs crates/btx-core/src/statement_check.rs crates/btx-core/src/lib.rs
git commit -m "core: compare a snapshot statement with this node and its diary, field by field" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 4: The website's client (`snapshot_site.rs`)

**Files:**
- Create: `crates/btx-core/src/snapshot_site.rs`
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: `operators::Chain`; the website plan's routes and replies (its Global Constraints).
- Produces (Tasks 5, 6, 7, 8): `SITE`, `SITE_ENV` (`EASYNODE_SNAPSHOT_SITE`), `NODE_HEADER`, `NODE_HEADER_VALUE`, `MAX_PART_BYTES`, `TIMEOUT`; `site_base_from(Option<&str>) -> String`, `site_base() -> String`, `client() -> Result<reqwest::Client, String>`, `chain_name(Chain) -> &'static str`; `StatementReply { statement_hash, chain, height, signers, operators, added, file }`, `PendingStatement { statement_hash, height, block_hash, manifest_hex, signers, operators, file, first_seen, confirmed, disputed }`, `Pending { version, chain, statements }`, `FileStored { stored, file_url, file_sha256, confirmed }`; `enum SiteError { Unreachable(String), Refused { status: u16, reason: String }, Unreadable(String) }`; `post_statement(&Client, base, &[u8]) -> Result<StatementReply, SiteError>`, `get_pending(&Client, base, chain: &str) -> Result<Pending, SiteError>`, `upload_file(&Client, base, statement_hash, &Path) -> Result<Option<FileStored>, SiteError>` (`None`: the website had the file).

- [ ] **Step 1: Write the failing tests**

Create `crates/btx-core/src/snapshot_site.rs` with only the test module (mockito stands in for the website; the parts are checked byte for byte):

````rust
#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Matcher;

    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_DAT: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-100.dat");
    const H: &str = "11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194";

    #[test]
    fn the_site_is_easybtx_unless_a_safe_override_says_otherwise() {
        assert_eq!(site_base_from(None), SITE);
        assert_eq!(site_base_from(Some("  ")), SITE);
        assert_eq!(
            site_base_from(Some("http://127.0.0.1:29650/")),
            "http://127.0.0.1:29650"
        );
        assert_eq!(
            site_base_from(Some("https://preview.vercel.app")),
            "https://preview.vercel.app"
        );
        for refused in [
            "http://example.com",
            "ftp://127.0.0.1",
            "https://easybtx.com/api",
            "https://user:pw@easybtx.com",
            "not a url",
        ] {
            assert_eq!(site_base_from(Some(refused)), SITE, "{refused}");
        }
    }

    #[tokio::test]
    async fn a_statement_goes_as_bytes_with_the_node_header() {
        let mut server = mockito::Server::new_async().await;
        let m = server
            .mock("POST", "/api/snapshots/statement")
            .match_header(NODE_HEADER, NODE_HEADER_VALUE)
            .match_header("content-type", "application/octet-stream")
            .match_body(R_P.to_vec())
            .with_body(format!(
                r#"{{"statement_hash":"{H}","chain":"regtest","height":100,"signers":["03"],"operators":["producer"],"added":1,"file":"missing"}}"#
            ))
            .create_async()
            .await;
        let reply = post_statement(&client().unwrap(), &server.url(), R_P)
            .await
            .unwrap();
        m.assert_async().await;
        assert_eq!(
            (reply.height, reply.operators, reply.file.as_str()),
            (100, vec!["producer".to_string()], "missing")
        );
    }

    #[tokio::test]
    async fn a_refusal_carries_the_websites_reason() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/statement")
            .with_status(422)
            .with_body(r#"{"error":"height 150 is not a multiple of 100"}"#)
            .create_async()
            .await;
        let err = post_statement(&client().unwrap(), &server.url(), R_P)
            .await
            .unwrap_err();
        assert_eq!(
            err,
            SiteError::Refused {
                status: 422,
                reason: "height 150 is not a multiple of 100".into()
            }
        );
        let err = post_statement(&client().unwrap(), "http://127.0.0.1:9", R_P)
            .await
            .unwrap_err();
        assert!(matches!(err, SiteError::Unreachable(_)), "{err}");
    }

    #[tokio::test]
    async fn the_file_goes_up_in_the_parts_the_website_asks_for() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("snap.dat");
        std::fs::write(&file, R_DAT).unwrap();
        let mut server = mockito::Server::new_async().await;
        let q = |pairs: &[(&str, &str)]| {
            Matcher::AllOf(
                pairs
                    .iter()
                    .map(|(k, v)| Matcher::UrlEncoded(k.to_string(), v.to_string()))
                    .collect(),
            )
        };
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(q(&[("statement", H), ("action", "start")]))
            .match_header(NODE_HEADER, NODE_HEADER_VALUE)
            .with_body(
                r#"{"upload":"0123456789abcdef0123456789abcdef","part_bytes":3000,"parts":3}"#,
            )
            .create_async()
            .await;
        let mut parts = Vec::new();
        for (n, range) in [(1, 0..3000), (2, 3000..6000), (3, 6000..8055)] {
            parts.push(
                server
                    .mock("PUT", "/api/snapshots/file")
                    .match_query(q(&[
                        ("statement", H),
                        ("upload", "0123456789abcdef0123456789abcdef"),
                        ("part", &n.to_string()),
                    ]))
                    .match_header("content-type", "application/octet-stream")
                    .match_body(R_DAT[range].to_vec())
                    .with_body(format!(r#"{{"part":{n},"bytes":1}}"#))
                    .create_async()
                    .await,
            );
        }
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(q(&[("statement", H), ("action", "complete")]))
            .with_body(r#"{"stored":true,"file_url":"https://x.public.blob.vercel-storage.com/f.dat","file_sha256":"b2c5","confirmed":false}"#)
            .create_async()
            .await;
        let stored = upload_file(&client().unwrap(), &server.url(), H, &file)
            .await
            .unwrap()
            .unwrap();
        for p in parts {
            p.assert_async().await;
        }
        assert!(stored.stored);
    }

    #[tokio::test]
    async fn a_file_already_there_is_not_sent_again_and_odd_part_plans_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("snap.dat");
        std::fs::write(&file, R_DAT).unwrap();
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(Matcher::Any)
            .with_status(409)
            .with_body(r#"{"error":"the file is already stored"}"#)
            .create_async()
            .await;
        assert_eq!(
            upload_file(&client().unwrap(), &server.url(), H, &file)
                .await
                .unwrap(),
            None
        );
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(Matcher::Any)
            .with_body(r#"{"upload":"u","part_bytes":8388608,"parts":1}"#)
            .create_async()
            .await;
        let err = upload_file(&client().unwrap(), &server.url(), H, &file)
            .await
            .unwrap_err();
        assert!(matches!(err, SiteError::Unreadable(_)), "{err}");
    }

    #[tokio::test]
    async fn pending_reads_the_websites_list() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(format!(
                r#"{{"version":1,"chain":"regtest","statements":[{{"statement_hash":"{H}","height":100,"block_hash":"bd23","manifest_hex":"02","signers":["03"],"operators":["producer"],"file":"stored","first_seen":"2026-10-01T12:00:00.000Z","confirmed":false,"disputed":false}}]}}"#
            ))
            .create_async()
            .await;
        let p = get_pending(&client().unwrap(), &server.url(), "regtest")
            .await
            .unwrap();
        assert_eq!(p.statements.len(), 1);
        assert_eq!(p.statements[0].operators, vec!["producer".to_string()]);
    }
}
````

In `crates/btx-core/src/lib.rs`, after `pub mod snapshot_serve;` add `pub mod snapshot_site;`.

- [ ] **Step 2: Run them to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot_site::`
Expected: compile errors, among them `` cannot find function `post_statement` in this scope `` and `` cannot find function `site_base_from` in this scope ``.

- [ ] **Step 3: Write the client**

Insert above `#[cfg(test)]` in `crates/btx-core/src/snapshot_site.rs`:

````rust
//! The app's side of the snapshot meeting point on easybtx.com (section 6 of
//! docs/decisions/2026-09-29-every-node-starts-near-the-tip.md): a producer
//! sends its statement and file, a confirmer reads what waits and sends back
//! one signature. The routes live in the EasyBTX repository
//! (`site/src/lib/snapshotRoutes.mjs`); this is their client.
//!
//! NOTHING the website answers is trusted. A producer only learns whether its
//! upload was kept; a confirmer re-reads every statement from its bytes and
//! checks it against its own node (`crate::statement_check`) before it signs.
//! Loading reads `latest` through `crate::attested_snapshot`, not through
//! this module.
//!
//! The file goes up in parts of at most 4 MiB because a Vercel function takes
//! a request body of about 4.5 MB. Every body is `application/octet-stream`.

use serde::Deserialize;
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncReadExt;

/// The website. `EASYNODE_SNAPSHOT_SITE` overrides it (see [`site_base_from`]),
/// which is how the regtest rehearsal points a node at a local stand-in.
pub const SITE: &str = "https://easybtx.com";
pub const SITE_ENV: &str = "EASYNODE_SNAPSHOT_SITE";
/// A junk filter the routes require on every write. Not authentication:
/// what authenticates a statement is its signatures.
pub const NODE_HEADER: &str = "x-ebtx-node";
pub const NODE_HEADER_VALUE: &str = "ebtx-snapshot-v1";
/// The largest part the routes take.
pub const MAX_PART_BYTES: u64 = 4 * 1024 * 1024;
/// Per request. A 4 MiB part on a slow home uplink needs the room.
pub const TIMEOUT: Duration = Duration::from_secs(120);

/// The site this run talks to: `raw` when it is `https://<host>[:port]` or a
/// loopback `http://127.0.0.1:<port>` / `http://localhost:<port>` with no
/// path, else [`SITE`]. A trailing slash is dropped.
pub fn site_base_from(raw: Option<&str>) -> String {
    let Some(v) = raw
        .map(|s| s.trim().trim_end_matches('/'))
        .filter(|s| !s.is_empty())
    else {
        return SITE.to_string();
    };
    let Ok(u) = reqwest::Url::parse(v) else {
        return SITE.to_string();
    };
    let loopback = matches!(u.host_str(), Some("127.0.0.1") | Some("localhost"));
    let ok = (u.scheme() == "https" || (u.scheme() == "http" && loopback))
        && u.username().is_empty()
        && u.password().is_none()
        && u.path() == "/"
        && u.query().is_none();
    if ok {
        v.to_string()
    } else {
        SITE.to_string()
    }
}

/// [`site_base_from`] with this process's `EASYNODE_SNAPSHOT_SITE`.
pub fn site_base() -> String {
    site_base_from(std::env::var(SITE_ENV).ok().as_deref())
}

pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| format!("snapshot site client: {e}"))
}

/// `main` or `regtest`, as the routes name chains.
pub fn chain_name(chain: crate::operators::Chain) -> &'static str {
    match chain {
        crate::operators::Chain::Main => "main",
        crate::operators::Chain::Regtest => "regtest",
    }
}

/// `POST /api/snapshots/statement`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StatementReply {
    pub statement_hash: String,
    pub chain: String,
    pub height: u64,
    pub signers: Vec<String>,
    pub operators: Vec<String>,
    /// Signatures this request added.
    pub added: u64,
    /// `missing`, `uploading` or `stored`.
    pub file: String,
}

/// One statement in `GET /api/snapshots/pending`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PendingStatement {
    pub statement_hash: String,
    pub height: u64,
    pub block_hash: String,
    /// The merged manifest, every signature the website took.
    pub manifest_hex: String,
    pub signers: Vec<String>,
    pub operators: Vec<String>,
    pub file: String,
    pub first_seen: String,
    pub confirmed: bool,
    pub disputed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Pending {
    pub version: u32,
    pub chain: String,
    pub statements: Vec<PendingStatement>,
}

/// `complete`'s answer when the website kept the file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FileStored {
    pub stored: bool,
    pub file_url: String,
    pub file_sha256: String,
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
struct UploadStart {
    upload: String,
    part_bytes: u64,
    parts: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteError {
    Unreachable(String),
    /// The website answered, and not with success. `reason` is its `error`.
    Refused {
        status: u16,
        reason: String,
    },
    Unreadable(String),
}

impl std::fmt::Display for SiteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SiteError::Unreachable(e) => write!(f, "easybtx.com could not be reached ({e})"),
            SiteError::Refused { status, reason } if reason.is_empty() => {
                write!(f, "easybtx.com answered {status}")
            }
            SiteError::Refused { status, reason } => {
                write!(f, "easybtx.com answered {status}: {reason}")
            }
            SiteError::Unreadable(e) => write!(f, "easybtx.com's answer did not read: {e}"),
        }
    }
}

async fn answer<T: serde::de::DeserializeOwned>(
    sent: Result<reqwest::Response, reqwest::Error>,
) -> Result<T, SiteError> {
    let resp = sent.map_err(|e| SiteError::Unreachable(e.to_string()))?;
    let status = resp.status().as_u16();
    let body = resp
        .bytes()
        .await
        .map_err(|e| SiteError::Unreachable(e.to_string()))?;
    if status != 200 {
        let reason = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or_default();
        return Err(SiteError::Refused { status, reason });
    }
    serde_json::from_slice(&body).map_err(|e| SiteError::Unreadable(e.to_string()))
}

/// Send a manifest: a new statement, or co-signatures for one already there.
pub async fn post_statement(
    client: &reqwest::Client,
    base: &str,
    manifest: &[u8],
) -> Result<StatementReply, SiteError> {
    answer(
        client
            .post(format!("{base}/api/snapshots/statement"))
            .header(NODE_HEADER, NODE_HEADER_VALUE)
            .header("content-type", "application/octet-stream")
            .body(manifest.to_vec())
            .send()
            .await,
    )
    .await
}

/// What waits for confirmers on `chain` (`main` or `regtest`).
pub async fn get_pending(
    client: &reqwest::Client,
    base: &str,
    chain: &str,
) -> Result<Pending, SiteError> {
    answer(
        client
            .get(format!("{base}/api/snapshots/pending?chain={chain}"))
            .send()
            .await,
    )
    .await
}

/// Upload the file of a statement the website holds, in the parts it asks
/// for. `Ok(None)` when it already had the file.
pub async fn upload_file(
    client: &reqwest::Client,
    base: &str,
    statement_hash: &str,
    file: &Path,
) -> Result<Option<FileStored>, SiteError> {
    let url = format!("{base}/api/snapshots/file");
    let started: Result<UploadStart, SiteError> = answer(
        client
            .post(format!("{url}?statement={statement_hash}&action=start"))
            .header(NODE_HEADER, NODE_HEADER_VALUE)
            .send()
            .await,
    )
    .await;
    let start = match started {
        Ok(s) => s,
        Err(SiteError::Refused { status: 409, .. }) => return Ok(None),
        Err(e) => return Err(e),
    };
    let len = std::fs::metadata(file)
        .map_err(|e| SiteError::Unreadable(format!("{}: {e}", file.display())))?
        .len();
    if start.part_bytes == 0
        || start.part_bytes > MAX_PART_BYTES
        || start.parts != len.div_ceil(start.part_bytes).max(1)
    {
        return Err(SiteError::Unreadable(format!(
            "asked for {} parts of {} bytes for a {len}-byte file",
            start.parts, start.part_bytes
        )));
    }
    let mut f = tokio::fs::File::open(file)
        .await
        .map_err(|e| SiteError::Unreadable(format!("{}: {e}", file.display())))?;
    for n in 1..=start.parts {
        let size = start.part_bytes.min(len - (n - 1) * start.part_bytes) as usize;
        let mut part = vec![0u8; size];
        f.read_exact(&mut part)
            .await
            .map_err(|e| SiteError::Unreadable(format!("{}: {e}", file.display())))?;
        let _: serde_json::Value = answer(
            client
                .put(format!(
                    "{url}?statement={statement_hash}&upload={}&part={n}",
                    start.upload
                ))
                .header(NODE_HEADER, NODE_HEADER_VALUE)
                .header("content-type", "application/octet-stream")
                .body(part)
                .send()
                .await,
        )
        .await?;
    }
    answer(
        client
            .post(format!(
                "{url}?statement={statement_hash}&upload={}&action=complete",
                start.upload
            ))
            .header(NODE_HEADER, NODE_HEADER_VALUE)
            .send()
            .await,
    )
    .await
    .map(Some)
}
````

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot_site::`
Expected: `6 passed; 0 failed`.

- [ ] **Step 5: Format, lint, commit**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cd ../..
git add crates/btx-core/src/snapshot_site.rs crates/btx-core/src/lib.rs
git commit -m "core: the client of easybtx.com's snapshot routes" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 5: Producers export on the grid and check before they offer or send (`snapshot_serve.rs`, `snapshot_producer.rs`)

**Files:**
- Modify: `crates/btx-core/src/snapshot_serve.rs` (`EXPORT_GRID`, `export_due`, `blocks_to_grid`, `BeforeOffer`, `NoChecks`, `run_cycle` takes the grid and the hook, an orphaned base is abandoned; tests)
- Create: `crates/btx-core/src/snapshot_producer.rs`
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: Tasks 2 to 4; `known_invalid::{refuse, refuse_held_in_order, Refusal, HeldBranch}` (exist); `operators::{for_chain, parse_key, Chain}`.
- Produces (Tasks 7, 8):
  - `snapshot_serve::EXPORT_GRID` (200), `export_due(tip, grid, base: Option<u64>) -> bool`, `blocks_to_grid(tip, grid) -> u64`
  - `#[async_trait] pub trait snapshot_serve::BeforeOffer: Sync { async fn check(&self, rpc: &dyn Rpc, base_height: u64, manifest: &[u8]) -> Result<(), String>; }`, `pub struct NoChecks`
  - `snapshot_serve::run_cycle(rpc, dir, grid: u64, poll, deadline, keep_going, on_phase, before_offer: &dyn BeforeOffer) -> Result<OfferRecord, String>`; `REFRESH_BLOCKS` and `refresh_due` are removed (only the keeper used them)
  - `snapshot_producer::{SUBMITTED_FILE, RESUBMIT_EVERY}` (600 s), `pub struct Submitted { height, statement_hash, at }`, `load_submitted(&Path)`, `save_submitted(&Path, &Submitted)`, `submission_due(&OfferRecord, Option<&Submitted>, grid) -> bool`, `key_is_listed(pubkey_hex, genesis, regtest_env) -> bool`, `pub async fn check_before_send(&dyn Rpc, manifest: &[u8], diary_dir: &Path, &Holds<'_>, regtest_env: Option<&str>) -> Result<(), Mismatch>`, `pub struct ProducerChecks { diary_dir: PathBuf, holds: Holds<'static>, regtest_env: Option<String> }` (implements `BeforeOffer`), `pub async fn submit(&Client, site: &str, dir: &Path, height: u64) -> Result<Submitted, String>`

- [ ] **Step 1: Change the keeper's tests to the grid, and add the new ones**

In `crates/btx-core/src/snapshot_serve.rs`, in the test module:

*Test: export_due replaces refresh_due.* Replace:

````rust
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
````

with:

````rust
    #[test]
    fn an_export_is_due_only_on_the_grid_and_above_the_offered_base() {
        assert!(export_due(226_200, 200, None));
        assert!(export_due(226_400, 200, Some(226_200)));
        assert!(!export_due(226_201, 200, None), "one block past the grid");
        assert!(!export_due(226_200, 200, Some(226_200)), "already offered");
        assert!(
            !export_due(226_200, 200, Some(226_400)),
            "below the offered base"
        );
        assert!(!export_due(0, 200, None));
        assert!(export_due(100, 100, None), "regtest's grid");
        assert_eq!(blocks_to_grid(226_150, 200), 50);
        assert_eq!(blocks_to_grid(226_200, 200), 200);
    }
````

*Test: the serving line.* Replace:

````rust
        assert!(m.contains("in 490 blocks"), "{m}");
````

with:

````rust
        assert!(m.contains("in 50 blocks"), "{m}");
````

*Test: the scripted node exports on the grid.* Replace:

````rust
                        "base_height": 226_140 + n - 1,
````

with:

````rust
                        "base_height": 226_200 + n - 1,
````

*Test: the full cycle.* Replace:

````rust
        let node = ScriptedNode::new(&[3, 4, 7, 9, 10]);
        let phases = Mutex::new(Vec::new());
        let (poll, deadline) = fast();
        let record = run_cycle(&node, d, poll, deadline, &|| true, &|p| {
            phases.lock().unwrap().push(p)
        })
        .await
        .unwrap();

        assert_eq!(record.height, 226_140);
````

with:

````rust
        let node = ScriptedNode::new(&[3, 4, 7, 9, 10]);
        let phases = Mutex::new(Vec::new());
        let (poll, deadline) = fast();
        let record = run_cycle(
            &node,
            d,
            200,
            poll,
            deadline,
            &|| true,
            &|p| phases.lock().unwrap().push(p),
            &NoChecks,
        )
        .await
        .unwrap();

        assert_eq!(record.height, 226_200);
````

*Test: the full cycle's snapshot name and last phase.* Replace:

````rust
        assert!(
            d.join(snapshot_file_name(226_140)).is_file(),
            "renamed into place"
        );
````

with:

````rust
        assert!(
            d.join(snapshot_file_name(226_200)).is_file(),
            "renamed into place"
        );
````

*Test: the full cycle's last phase.* Replace:

````rust
        assert!(matches!(
            seen.last(),
            Some(CyclePhase::Live { base: 226_140 })
        ));
````

with:

````rust
        assert!(matches!(
            seen.last(),
            Some(CyclePhase::Live { base: 226_200 })
        ));
````

*Test: an orphaned base is abandoned.* Replace:

````rust
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
````

with:

````rust
    #[tokio::test]
    async fn an_orphaned_base_is_abandoned_until_the_next_grid_height() {
        // The 2026-09-20 shape: the base is orphaned within a block. A second
        // dump would base on the tip, off the grid, so none is made.
        let dir = tempfile::tempdir().unwrap();
        let node = ScriptedNode::new(&[2, -1, 5, 10]);
        let (poll, deadline) = fast();
        let err = run_cycle(
            &node,
            dir.path(),
            200,
            poll,
            deadline,
            &|| true,
            &|_| {},
            &NoChecks,
        )
        .await
        .unwrap_err();
        assert!(err.contains("left the chain while it matured"), "{err}");
        assert_eq!(node.count("dumptxoutsetattested"), 1);
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
        assert_eq!(load_record(dir.path()), None);
    }

    #[tokio::test]
    async fn an_export_that_landed_off_the_grid_is_not_matured() {
        // The tip moved between the keeper's look and the dump: 226,200 is
        // not a multiple of 400.
        let dir = tempfile::tempdir().unwrap();
        let node = ScriptedNode::new(&[10]);
        let (poll, deadline) = fast();
        let err = run_cycle(
            &node,
            dir.path(),
            400,
            poll,
            deadline,
            &|| true,
            &|_| {},
            &NoChecks,
        )
        .await
        .unwrap_err();
        assert!(err.contains("the tip moved to 226200"), "{err}");
        assert_eq!(node.count("getblockheader"), 0, "nothing matured");
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
    }

    #[tokio::test]
    async fn a_pair_the_checks_refuse_is_not_offered_and_the_old_offer_stays() {
        struct Refuses(Mutex<Vec<(u64, Vec<u8>)>>);
        #[async_trait]
        impl BeforeOffer for Refuses {
            async fn check(&self, _: &dyn Rpc, base: u64, manifest: &[u8]) -> Result<(), String> {
                self.0.lock().unwrap().push((base, manifest.to_vec()));
                Err("coin count differs from the diary (diary 7, statement 8)".into())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let node = ScriptedNode::new(&[10]);
        let hook = Refuses(Mutex::new(Vec::new()));
        let (poll, deadline) = fast();
        let err = run_cycle(
            &node,
            dir.path(),
            200,
            poll,
            deadline,
            &|| true,
            &|_| {},
            &hook,
        )
        .await
        .unwrap_err();
        assert_eq!(
            err,
            "base 226200 was not offered: coin count differs from the diary (diary 7, statement 8)"
        );
        assert_eq!(
            hook.0.lock().unwrap().clone(),
            vec![(226_200, vec![0x02, 0x01, 0x46, 0xfa])],
            "the hook saw the export's own manifest"
        );
        assert_eq!(node.count("withdrawattestedutxosnapshot"), 0);
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
        assert_eq!(load_record(dir.path()), None);
    }
````

*Test: the last-moment test.* Replace:

````rust
        let err = run_cycle(&node, dir.path(), poll, deadline, &|| true, &|_| {})
            .await
            .unwrap_err();
        assert!(err.contains("left the active chain"), "{err}");
````

with:

````rust
        let err = run_cycle(
            &node,
            dir.path(),
            200,
            poll,
            deadline,
            &|| true,
            &|_| {},
            &NoChecks,
        )
        .await
        .unwrap_err();
        assert!(err.contains("left the active chain"), "{err}");
````

*Test: a stopped role.* Replace:

````rust
        let err = run_cycle(&node, dir.path(), poll, deadline, &|| false, &|_| {})
            .await
            .unwrap_err();
````

with:

````rust
        let err = run_cycle(
            &node,
            dir.path(),
            200,
            poll,
            deadline,
            &|| false,
            &|_| {},
            &NoChecks,
        )
        .await
        .unwrap_err();
````

*Test: a base that never matures.* Replace:

````rust
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
````

with:

````rust
        let err = run_cycle(
            &node,
            dir.path(),
            200,
            Duration::from_millis(1),
            Duration::from_millis(20),
            &|| true,
            &|_| {},
            &NoChecks,
        )
        .await
        .unwrap_err();
````

- [ ] **Step 2: Write the producer's failing tests**

Create `crates/btx-core/src/snapshot_producer.rs` with only the test module:

````rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::diary::{Diary, DiaryEntry};
    use crate::fake_node::FakeNode;
    use crate::known_invalid::HeldBranch;
    use crate::operators::{MAINNET_GENESIS, REGTEST_GENESIS};
    use mockito::Matcher;

    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_DAT: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-100.dat");
    const R_BLOCK: &str = "bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46";
    const R_UTXO: &str = "e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527";
    const H: &str = "11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194";
    const P: &str = "0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464";
    const ROOT: &str = "8240c62e62b47fc675610908c03045c244de1dfc06246209830ba9d98468952c";
    const HOLD_AT_105: &[HeldBranch] = &[HeldBranch {
        height: 105,
        root: ROOT,
        why: "a test hold above the base",
    }];
    const HOLD_AT_50: &[HeldBranch] = &[HeldBranch {
        height: 50,
        root: ROOT,
        why: "a test hold below the base",
    }];

    fn producer() -> (FakeNode, tempfile::TempDir) {
        let n = FakeNode::new(REGTEST_GENESIS, 110);
        n.with(|s| {
            s.chain.insert(100, R_BLOCK.into());
            s.replay_context = Some(cs::REGTEST_REPLAY_CONTEXT.into());
        });
        let dir = tempfile::tempdir().unwrap();
        let mut d = Diary::new(REGTEST_GENESIS);
        d.record(DiaryEntry {
            height: 100,
            block_hash: R_BLOCK.into(),
            hash_serialized: R_UTXO.into(),
            coins: 101,
            chain_tx: 101,
            recorded_at: 0,
        });
        crate::diary::save(dir.path(), &d).unwrap();
        (n, dir)
    }

    #[tokio::test]
    async fn the_spikes_export_passes_on_the_node_that_made_it() {
        let (n, dir) = producer();
        assert_eq!(
            check_before_send(&n, R_P, dir.path(), &Holds::none(), None).await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn a_producer_whose_diary_disagrees_sends_nothing() {
        let (n, dir) = producer();
        let mut d = crate::diary::load(dir.path(), REGTEST_GENESIS);
        let mut e = d.at(100).unwrap().clone();
        e.hash_serialized = "55".repeat(32);
        d.record(e);
        crate::diary::save(dir.path(), &d).unwrap();
        let got = check_before_send(&n, R_P, dir.path(), &Holds::none(), None).await;
        assert!(
            matches!(got, Err(Mismatch::HashSerialized { .. })),
            "{got:?}"
        );
    }

    #[tokio::test]
    async fn a_held_block_is_refused_first_and_a_base_above_it_is_not_sent() {
        // The held root is at 50 on this node's chain: refusing it takes the
        // chain down to 49, and the base at 100 is no longer on it.
        let (n, dir) = producer();
        n.with(|s| {
            s.chain.insert(50, ROOT.into());
        });
        let holds = Holds {
            invalid: &[],
            held: HOLD_AT_50,
        };
        let got = check_before_send(&n, R_P, dir.path(), &holds, None).await;
        assert_eq!(got, Err(Mismatch::BaseNotOnActiveChain { height: 100 }));
        assert_eq!(n.count("invalidateblock"), 1);
        // A hold the node cannot refuse stops everything.
        let (n, dir) = producer();
        n.with(|s| {
            s.chain.insert(50, ROOT.into());
            s.invalidate_fails = true;
        });
        let got = check_before_send(&n, R_P, dir.path(), &holds, None).await;
        assert!(matches!(got, Err(Mismatch::HeldNotRefused(_))), "{got:?}");
        // A hold above the base, on another branch the node has seen, does not
        // concern this statement.
        let (n, dir) = producer();
        n.with(|s| {
            s.side.insert(ROOT.into());
        });
        let holds = Holds {
            invalid: &[],
            held: HOLD_AT_105,
        };
        assert_eq!(
            check_before_send(&n, R_P, dir.path(), &holds, None).await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn a_manifest_with_a_bad_signature_is_not_sent() {
        let (n, dir) = producer();
        let mut bad = R_P.to_vec();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        let got = check_before_send(&n, &bad, dir.path(), &Holds::none(), None).await;
        assert!(matches!(got, Err(Mismatch::Unreadable(_))), "{got:?}");
    }

    #[test]
    fn only_a_listed_key_submits_and_only_once_per_pair() {
        assert!(key_is_listed(
            "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675",
            MAINNET_GENESIS,
            None
        ));
        assert!(!key_is_listed(
            P,
            MAINNET_GENESIS,
            Some(&format!("producer={P}"))
        ));
        assert!(key_is_listed(
            P,
            REGTEST_GENESIS,
            Some(&format!("producer={P}"))
        ));
        assert!(!key_is_listed(P, REGTEST_GENESIS, None));
        let record = |height| OfferRecord {
            height,
            block_hash: String::new(),
            txoutset_hash: String::new(),
            file_size: 0,
            sha256: String::new(),
            manifest_sha256: String::new(),
            file_hash: String::new(),
            chunk_count: 0,
            signatures: 1,
            offered_at: 0,
        };
        let done = Submitted {
            height: 226_200,
            statement_hash: H.into(),
            at: 0,
        };
        assert!(submission_due(&record(226_200), None, 200));
        assert!(!submission_due(&record(226_200), Some(&done), 200));
        assert!(submission_due(&record(226_400), Some(&done), 200));
        assert!(
            !submission_due(&record(226_140), None, 200),
            "an old off-grid pair"
        );
    }

    #[tokio::test]
    async fn submit_sends_the_statement_then_the_file_and_writes_it_down() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(manifest_file_name(100)), R_P).unwrap();
        std::fs::write(dir.path().join(snapshot_file_name(100)), R_DAT).unwrap();
        let mut server = mockito::Server::new_async().await;
        let statement = server
            .mock("POST", "/api/snapshots/statement")
            .match_body(R_P.to_vec())
            .with_body(format!(
                r#"{{"statement_hash":"{H}","chain":"regtest","height":100,"signers":["{P}"],"operators":["producer"],"added":1,"file":"missing"}}"#
            ))
            .create_async()
            .await;
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(Matcher::UrlEncoded("action".into(), "start".into()))
            .with_body(
                r#"{"upload":"0123456789abcdef0123456789abcdef","part_bytes":4194304,"parts":1}"#,
            )
            .create_async()
            .await;
        let part = server
            .mock("PUT", "/api/snapshots/file")
            .match_query(Matcher::Any)
            .match_body(R_DAT.to_vec())
            .with_body(r#"{"part":1,"bytes":8055}"#)
            .create_async()
            .await;
        server
            .mock("POST", "/api/snapshots/file")
            .match_query(Matcher::UrlEncoded("action".into(), "complete".into()))
            .with_body(r#"{"stored":true,"file_url":"u","file_sha256":"s","confirmed":false}"#)
            .create_async()
            .await;
        let client = crate::snapshot_site::client().unwrap();
        let done = submit(&client, &server.url(), dir.path(), 100)
            .await
            .unwrap();
        statement.assert_async().await;
        part.assert_async().await;
        assert_eq!((done.height, done.statement_hash.as_str()), (100, H));
        assert_eq!(load_submitted(dir.path()), Some(done));
    }
}
````

In `crates/btx-core/src/lib.rs`, before `pub mod snapshot_serve;` add `pub mod snapshot_producer;`.

- [ ] **Step 3: Run them to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot_serve:: snapshot_producer::`
Expected: compile errors, among them `` cannot find function `export_due` in this scope ``, `` cannot find trait `BeforeOffer` in this scope ``, `` cannot find value `NoChecks` in this scope ``, `` cannot find function `check_before_send` in this scope ``.

- [ ] **Step 4: Export on the grid, and let the hook have the last word**

In `crates/btx-core/src/snapshot_serve.rs`, above the test module:

*The refresh constant becomes the grid.* Replace:

````rust
/// Re-export once the tip is this far past the offered base. About eleven
/// hours at the 45 blocks/h measured on 2026-09-21; an importer then catches
/// up at most this much, which a mirror does in minutes.
pub const REFRESH_BLOCKS: u64 = 500;
````

with:

````rust
/// Snapshots are exported where mainnet's tip is a multiple of this (section
/// 2 of docs/decisions/2026-09-29-every-node-starts-near-the-tip.md). 200
/// keeps the blocks after a snapshot within the 288 recent blocks almost every
/// peer serves, and divides the engine's compiled heights (219,000, 228,000),
/// so every node's diary has the heights a snapshot can name.
pub const EXPORT_GRID: u64 = crate::confirmed_snapshot::MAINNET_GRID as u64;
````

*Refresh_due becomes export_due.* Replace:

````rust
/// Is it time to export again? `base` is the offered base's height, `None`
/// when nothing has ever been offered from this folder.
pub fn refresh_due(tip: u64, base: Option<u64>) -> bool {
    match base {
        None => true,
        Some(b) => tip.saturating_sub(b) >= REFRESH_BLOCKS,
    }
}
````

with:

````rust
/// Is it time to export? Only when the tip is exactly on the grid and above
/// the base already offered (`None` when nothing ever was). A tip that moved
/// past a grid height before this was asked waits for the next one.
pub fn export_due(tip: u64, grid: u64, base: Option<u64>) -> bool {
    tip > 0 && grid > 0 && tip.is_multiple_of(grid) && base.is_none_or(|b| tip > b)
}

/// Blocks until the tip reaches the next grid height, 1 to `grid`.
pub fn blocks_to_grid(tip: u64, grid: u64) -> u64 {
    grid - tip % grid
}
````

*The hook, before the cycle.* Replace:

````rust
// ── The cycle ───────────────────────────────────────────────────────────────
````

with:

````rust
// ── The cycle ───────────────────────────────────────────────────────────────

/// The last word before a matured pair is offered or sent anywhere: the
/// producer's checks (`crate::snapshot_producer::ProducerChecks`). `Err`
/// keeps the pair off the wire and the previous offer live.
#[async_trait::async_trait]
pub trait BeforeOffer: Sync {
    async fn check(&self, rpc: &dyn Rpc, base_height: u64, manifest: &[u8]) -> Result<(), String>;
}

/// No check beyond the canonical one: what the cycle did before the diary.
pub struct NoChecks;

#[async_trait::async_trait]
impl BeforeOffer for NoChecks {
    async fn check(&self, _: &dyn Rpc, _: u64, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
}
````

*Run_cycle's doc and signature.* Replace:

````rust
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
````

with:

````rust
/// One full cycle: dump, wait, verify, check, withdraw the old, offer the
/// new, record, bounce, prune. Returns the record of what is now served.
///
/// * `grid`: the export must land on a multiple of it; a tip that moved
///   before the dump, or a base orphaned while it matured, ends the cycle
///   until the next grid height (the dump always bases on the tip, so a
///   second try would be off the grid).
/// * `poll` / `deadline` are [`MATURE_POLL`] / [`MATURE_DEADLINE`] in the app
///   and milliseconds in tests.
/// * `keep_going` is asked between polls; `false` aborts (the node stopped,
///   the role was switched off). Nothing is offered after an abort.
/// * `on_phase` receives every phase change, for the status row.
/// * `before_offer` has the last word, after the canonical re-verify.
#[allow(clippy::too_many_arguments)]
pub async fn run_cycle(
    rpc: &dyn Rpc,
    dir: &Path,
    grid: u64,
    poll: Duration,
    deadline: Duration,
    keep_going: &(dyn Fn() -> bool + Sync),
    on_phase: &(dyn Fn(CyclePhase) + Sync),
    before_offer: &dyn BeforeOffer,
) -> Result<OfferRecord, String> {
    let started = std::time::Instant::now();
    on_phase(CyclePhase::Dumping);
    let base = dump(rpc, dir)
        .await
        .map_err(|e| format!("export failed: {e}"))?;
    if grid == 0 || base.base_height % grid != 0 {
        return Err(format!(
            "the tip moved to {} before the export; the next one is at the next multiple of {grid}",
            base.base_height
        ));
    }
````

*An orphaned base is abandoned, not dumped again.* Replace:

````rust
        if c < 0 {
            // Orphaned. Exactly what happened to the first one ever taken.
            on_phase(CyclePhase::Dumping);
            base = dump(rpc, dir)
                .await
                .map_err(|e| format!("re-export failed: {e}"))?;
            continue;
        }
````

with:

````rust
        if c < 0 {
            // Orphaned, as the first one ever taken was. A new dump would
            // base on today's tip, off the grid, so wait for the next height.
            return Err(format!(
                "base {} left the chain while it matured; the next export is at the next multiple of {grid}",
                base.base_height
            ));
        }
````

*The hook runs after the canonical re-verify.* Replace:

````rust
        Err(e) => return Err(format!("could not re-verify the base: {e}")),
    }
    let dat = dir.join(snapshot_file_name(base.base_height));
````

with:

````rust
        Err(e) => return Err(format!("could not re-verify the base: {e}")),
    }
    let manifest = std::fs::read(dir.join(STAGING_MANIFEST))
        .map_err(|e| format!("reading the export's manifest: {e}"))?;
    before_offer
        .check(rpc, base.base_height, &manifest)
        .await
        .map_err(|e| format!("base {} was not offered: {e}", base.base_height))?;
    let dat = dir.join(snapshot_file_name(base.base_height));
````

*The serving line counts to the next grid height.* Replace:

````rust
        let next = stale
            .map(|n| REFRESH_BLOCKS.saturating_sub(n))
            .map(|n| format!(" A fresh one is taken in {n} blocks."))
            .unwrap_or_default();
````

with:

````rust
        let next = tip
            .map(|t| blocks_to_grid(t, EXPORT_GRID))
            .map(|n| format!(" A fresh one is taken in {n} blocks."))
            .unwrap_or_default();
````

- [ ] **Step 5: Write the producer**

Insert above `#[cfg(test)]` in `crates/btx-core/src/snapshot_producer.rs`:

````rust
//! The producer (section 4 of docs/decisions/2026-09-29-every-node-starts-
//! near-the-tip.md): a validating node with "Serve a chain snapshot" on
//! exports where its tip is a multiple of 200 (`crate::snapshot_serve`),
//! waits 10 confirmations, and then, before the pair is offered on the
//! network or sent anywhere, checks it once more against its own node:
//!
//! * every block the app refuses (`known_invalid`) is refused on this node,
//!   so the base cannot sit on a held branch;
//! * the base is still the block at that height on its active chain, with 10
//!   confirmations, and no refused block is below it
//!   (`crate::statement_check`);
//! * its own diary entry for that height matches the statement field by
//!   field.
//!
//! A pair that fails any of it is neither offered nor sent. A producer whose
//! key is on the operator list then sends the statement and the file to
//! easybtx.com (`crate::snapshot_site`), and writes down that it did, so a
//! failed upload is tried again (every [`RESUBMIT_EVERY`]) and a finished one
//! never is.

use crate::confirmed_load::Holds;
use crate::confirmed_snapshot::{self as cs, ChainRules};
use crate::known_invalid::{self, Refusal};
use crate::operators::{self, Chain};
use crate::rpc::Rpc;
use crate::snapshot_serve::{
    manifest_file_name, snapshot_file_name, BeforeOffer, OfferRecord, CONFIRMATIONS_REQUIRED,
};
use crate::statement_check::{check_against_node, Mismatch};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Beside the pairs: which offered pair the website has.
pub const SUBMITTED_FILE: &str = "submitted.json";
/// How long a failed submission waits before the keeper tries again.
pub const RESUBMIT_EVERY: Duration = Duration::from_secs(600);

/// The pair the website took, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submitted {
    pub height: u64,
    pub statement_hash: String,
    /// Unix seconds.
    pub at: u64,
}

pub fn load_submitted(dir: &Path) -> Option<Submitted> {
    serde_json::from_slice(&std::fs::read(dir.join(SUBMITTED_FILE)).ok()?).ok()
}

pub fn save_submitted(dir: &Path, s: &Submitted) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?;
    crate::fsx::atomic_write(&dir.join(SUBMITTED_FILE), &json)
}

/// Whether the offered pair still has to go to the website: it is on the
/// grid and the website has not taken it yet.
pub fn submission_due(record: &OfferRecord, submitted: Option<&Submitted>, grid: u64) -> bool {
    grid > 0
        && record.height.is_multiple_of(grid)
        && submitted.map(|s| s.height) != Some(record.height)
}

/// Whether `pubkey_hex` is on the operator list of the chain `genesis` names
/// (the compiled one on mainnet, `EASYNODE_REGTEST_OPERATORS` on regtest).
pub fn key_is_listed(pubkey_hex: &str, genesis: &str, regtest_env: Option<&str>) -> bool {
    let (Some(chain), Some(key)) = (
        Chain::from_genesis_hex(genesis),
        operators::parse_key(pubkey_hex),
    ) else {
        return false;
    };
    operators::for_chain(chain, regtest_env)
        .operator_of(&key)
        .is_some()
}

fn refused_ok(r: &Refusal) -> bool {
    matches!(r, Refusal::Refused | Refusal::NotKnownYet)
}

/// Section 4's checks on this node's own export. `diary_dir` holds the diary
/// (the datadir). Refusing held blocks changes the node (`invalidateblock`,
/// as the fork check does every 30 seconds anyway); everything else reads.
pub async fn check_before_send(
    rpc: &dyn Rpc,
    manifest: &[u8],
    diary_dir: &Path,
    holds: &Holds<'_>,
    regtest_env: Option<&str>,
) -> Result<(), Mismatch> {
    let m = cs::parse(manifest)?;
    let rules = ChainRules::for_statement(&m.statement, regtest_env)?;
    cs::confirming_operators(&m, &rules.operators)?;
    for block in holds.invalid {
        let r = known_invalid::refuse(rpc, block).await;
        if !refused_ok(&r) {
            return Err(Mismatch::HeldNotRefused(format!(
                "{} at {}: {r:?}",
                block.hash, block.height
            )));
        }
    }
    for (branch, r) in known_invalid::refuse_held_in_order(rpc, holds.held).await {
        if !refused_ok(&r) {
            return Err(Mismatch::HeldNotRefused(format!(
                "{} at {}: {r:?}",
                branch.root, branch.height
            )));
        }
    }
    let diary = crate::diary::load(diary_dir, &m.statement.chain_id().display_hex());
    check_against_node(
        rpc,
        &m.statement,
        &rules,
        &diary,
        holds,
        CONFIRMATIONS_REQUIRED,
    )
    .await
}

/// [`check_before_send`] as the keeper's [`BeforeOffer`] hook.
pub struct ProducerChecks {
    pub diary_dir: PathBuf,
    pub holds: Holds<'static>,
    pub regtest_env: Option<String>,
}

#[async_trait::async_trait]
impl BeforeOffer for ProducerChecks {
    async fn check(&self, rpc: &dyn Rpc, _base_height: u64, manifest: &[u8]) -> Result<(), String> {
        check_before_send(
            rpc,
            manifest,
            &self.diary_dir,
            &self.holds,
            self.regtest_env.as_deref(),
        )
        .await
        .map_err(|m| m.to_string())
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Send the offered pair in `dir` to the website: the statement, then the
/// file unless the website has it. Writes [`SUBMITTED_FILE`] on success. The
/// caller has run [`check_before_send`] on this pair.
pub async fn submit(
    client: &reqwest::Client,
    site: &str,
    dir: &Path,
    height: u64,
) -> Result<Submitted, String> {
    let manifest = std::fs::read(dir.join(manifest_file_name(height)))
        .map_err(|e| format!("reading the manifest: {e}"))?;
    let reply = crate::snapshot_site::post_statement(client, site, &manifest)
        .await
        .map_err(|e| e.to_string())?;
    if reply.file != "stored" {
        crate::snapshot_site::upload_file(
            client,
            site,
            &reply.statement_hash,
            &dir.join(snapshot_file_name(height)),
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    let done = Submitted {
        height,
        statement_hash: reply.statement_hash,
        at: now_unix(),
    };
    save_submitted(dir, &done).map_err(|e| format!("recording the submission: {e}"))?;
    Ok(done)
}
````

- [ ] **Step 6: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot_serve:: snapshot_producer::`
Expected: `33 passed; 0 failed` (27 in `snapshot_serve`, 6 in `snapshot_producer`). Among them: `an_export_is_due_only_on_the_grid_and_above_the_offered_base`, `an_orphaned_base_is_abandoned_until_the_next_grid_height` (one dump, no offer), `an_export_that_landed_off_the_grid_is_not_matured`, `a_pair_the_checks_refuse_is_not_offered_and_the_old_offer_stays` (no withdraw, no offer, no record), `a_producer_whose_diary_disagrees_sends_nothing`, `a_held_block_is_refused_first_and_a_base_above_it_is_not_sent`, `only_a_listed_key_submits_and_only_once_per_pair`.

The Tauri crate does not build between this task and Task 7 (the keeper still calls `refresh_due` and the old `run_cycle`); run only the btx-core gates here.

- [ ] **Step 7: Format, lint, commit**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cargo test --locked --lib && cd ../..
git add crates/btx-core/src/snapshot_serve.rs crates/btx-core/src/snapshot_producer.rs crates/btx-core/src/lib.rs
git commit -m "core: producers export at every 200 blocks and check against their diary before offering or sending" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 6: Confirmers (`snapshot_confirmer.rs`)

**Files:**
- Create: `crates/btx-core/src/snapshot_confirmer.rs`
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: Tasks 2 to 5; `confirmed_snapshot::{parse, Manifest, Signed, ChainRules, confirming_operators, signature_is_valid}`, `operators::{hex_decode, parse_key, Chain}`, `node_api::MatmulTrustedStatus`.
- Produces (Tasks 7, 8):
  - `CONFIRM_EVERY_SECS` (600), `WORK_DIR` (`confirm`)
  - `pub enum Verdict { Signed { height, statement_hash, operators }, AlreadySigned { height, statement_hash }, Refused { height, statement_hash, why: Mismatch }, Failed { height, statement_hash, error } }` with `statement_hash()` and `line()`
  - `pub fn why_not(&MatmulTrustedStatus, our_key_hex: Option<&str>, genesis: &str, regtest_env: Option<&str>, network_dir: &Path) -> Option<&'static str>`
  - `pub struct Confirmer<'a> { rpc: &'a dyn Rpc, client: &'a reqwest::Client, site: &'a str, diary_dir: &'a Path, work_dir: &'a Path, our_key: [u8; 33], holds: Holds<'a>, regtest_env: Option<&'a str> }` with `async fn round(&self, Chain) -> Result<Vec<Verdict>, String>` and `async fn confirm_one(&self, &PendingStatement) -> Verdict`
  - `pub struct Tally { rounds, signed, refused, failed, last_refusal, last_error }` with `add(&[Verdict], &mut HashSet<String>) -> Vec<String>` and `report() -> Vec<String>`
  - `pub struct NetworkReport { diary: Option<String>, confirmer: Tally, confirmer_off: Option<String>, seen: HashSet<String>, producer: Option<String> }` (`Default`) with `lines(diary_summary: Option<String>) -> Vec<String>`

- [ ] **Step 1: Write the failing tests**

Create `crates/btx-core/src/snapshot_confirmer.rs` with only the test module. The scripted node signs with key 7 the way the engine does, and the website is mockito: a statement the diary agrees with gets exactly one signature (the POST body is checked byte for byte), a diary one coin off gets none and nothing is posted, a statement this operator signed is left alone, a listing whose hash lies and a manifest with a broken signature are refused unsigned, an engine that signs with another key sends nothing, a refusal is logged once per run, and only a validating, signing, listed node with a diary confirms:

````rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::diary::{Diary, DiaryEntry};
    use crate::fake_node::FakeNode;
    use crate::operators::REGTEST_GENESIS;
    use k256::ecdsa::signature::hazmat::PrehashSigner;
    use k256::ecdsa::{Signature, SigningKey};

    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_PC: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-PC.manifest");
    const R_BLOCK: &str = "bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46";
    const R_UTXO: &str = "e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527";
    const H: &str = "11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194";
    const P: &str = "0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464";
    const C: &str = "02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f";

    fn key(n: u8) -> SigningKey {
        SigningKey::from_slice(&[n; 32]).unwrap()
    }

    fn pubkey(sk: &SigningKey) -> [u8; 33] {
        sk.verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .try_into()
            .unwrap()
    }

    /// Our node: validating, at 110, key 7, a diary that matches the spike's
    /// statement at 100.
    fn setup(coins_in_diary: u64) -> (FakeNode, tempfile::TempDir, String) {
        let n = FakeNode::new(REGTEST_GENESIS, 110);
        n.with(|s| {
            s.chain.insert(100, R_BLOCK.into());
            s.replay_context = Some(cs::REGTEST_REPLAY_CONTEXT.into());
            s.signer = Some(key(7));
        });
        let dir = tempfile::tempdir().unwrap();
        let mut d = Diary::new(REGTEST_GENESIS);
        d.record(DiaryEntry {
            height: 100,
            block_hash: R_BLOCK.into(),
            hash_serialized: R_UTXO.into(),
            coins: coins_in_diary,
            chain_tx: 101,
            recorded_at: 0,
        });
        crate::diary::save(dir.path(), &d).unwrap();
        let env = format!(
            "producer={P};confirmer={}",
            operators::hex(&pubkey(&key(7)))
        );
        (n, dir, env)
    }

    fn pending_json(manifest: &[u8], hash: &str) -> String {
        format!(
            r#"{{"version":1,"chain":"regtest","statements":[{{"statement_hash":"{hash}","height":100,"block_hash":"{R_BLOCK}","manifest_hex":"{}","signers":[],"operators":[],"file":"stored","first_seen":"2026-10-01T12:00:00.000Z","confirmed":false,"disputed":false}}]}}"#,
            operators::hex(manifest)
        )
    }

    /// The manifest this node should send: the statement and its one signature.
    fn ours_only(sk: &SigningKey) -> Vec<u8> {
        let st = cs::parse(R_P).unwrap().statement;
        let sig: Signature = sk.sign_prehash(&st.hash().0).unwrap();
        Manifest {
            statement: st,
            signatures: vec![cs::Signed {
                key: pubkey(sk),
                der: sig.to_der().as_bytes().to_vec(),
            }],
        }
        .to_bytes()
    }

    async fn round(
        n: &FakeNode,
        dir: &Path,
        env: &str,
        our_key: [u8; 33],
        server: &mockito::ServerGuard,
    ) -> Vec<Verdict> {
        let client = site::client().unwrap();
        let work = dir.join("snapshots").join(WORK_DIR);
        let c = Confirmer {
            rpc: n,
            client: &client,
            site: &server.url(),
            diary_dir: dir,
            work_dir: &work,
            our_key,
            holds: Holds::none(),
            regtest_env: Some(env),
        };
        c.round(Chain::Regtest).await.unwrap()
    }

    #[tokio::test]
    async fn a_statement_the_diary_agrees_with_gets_this_nodes_one_signature() {
        let (n, dir, env) = setup(101);
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(R_P, H))
            .create_async()
            .await;
        let post = server
            .mock("POST", "/api/snapshots/statement")
            .match_body(ours_only(&key(7)))
            .with_body(format!(
                r#"{{"statement_hash":"{H}","chain":"regtest","height":100,"signers":[],"operators":["producer","confirmer"],"added":1,"file":"stored"}}"#
            ))
            .create_async()
            .await;
        let v = round(&n, dir.path(), &env, pubkey(&key(7)), &server).await;
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Verdict::Signed {
                height: 100,
                statement_hash: H.into(),
                operators: vec!["producer".into(), "confirmer".into()]
            }]
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 1);
        let work = dir.path().join("snapshots").join(WORK_DIR);
        assert_eq!(
            std::fs::read_dir(&work).unwrap().count(),
            0,
            "the copy is gone"
        );
    }

    #[tokio::test]
    async fn a_diary_that_disagrees_is_never_signed() {
        let (n, dir, env) = setup(102);
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(R_P, H))
            .create_async()
            .await;
        let post = server
            .mock("POST", "/api/snapshots/statement")
            .expect(0)
            .create_async()
            .await;
        let v = round(&n, dir.path(), &env, pubkey(&key(7)), &server).await;
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Verdict::Refused {
                height: 100,
                statement_hash: H.into(),
                why: Mismatch::Coins {
                    diary: 102,
                    statement: 101
                }
            }]
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 0);
    }

    #[tokio::test]
    async fn a_statement_this_operator_signed_is_left_alone() {
        let (n, dir, _) = setup(101);
        let env = format!("producer={P};confirmer={C}");
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(R_PC, H))
            .create_async()
            .await;
        let v = round(
            &n,
            dir.path(),
            &env,
            operators::parse_key(C).unwrap(),
            &server,
        )
        .await;
        assert_eq!(
            v,
            vec![Verdict::AlreadySigned {
                height: 100,
                statement_hash: H.into()
            }]
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 0);
    }

    #[tokio::test]
    async fn a_listing_that_lies_or_a_bad_signature_is_refused_unsigned() {
        let (n, dir, env) = setup(101);
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(R_P, &"00".repeat(32)))
            .create_async()
            .await;
        let v = round(&n, dir.path(), &env, pubkey(&key(7)), &server).await;
        assert!(
            matches!(
                &v[0],
                Verdict::Refused {
                    why: Mismatch::Unreadable(_),
                    ..
                }
            ),
            "{v:?}"
        );
        let mut bad = R_P.to_vec();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(&bad, H))
            .create_async()
            .await;
        let v = round(&n, dir.path(), &env, pubkey(&key(7)), &server).await;
        assert!(
            matches!(&v[0], Verdict::Refused { why: Mismatch::Unreadable(e), .. } if e.contains("is not valid")),
            "{v:?}"
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 0);
    }

    #[tokio::test]
    async fn an_engine_that_signs_with_another_key_sends_nothing() {
        let (n, dir, env) = setup(101);
        n.with(|s| s.signer = Some(key(8)));
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(R_P, H))
            .create_async()
            .await;
        let post = server
            .mock("POST", "/api/snapshots/statement")
            .expect(0)
            .create_async()
            .await;
        let v = round(&n, dir.path(), &env, pubkey(&key(7)), &server).await;
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Verdict::Failed {
                height: 100,
                statement_hash: H.into(),
                error: "the engine did not sign with this node's key".into()
            }]
        );
    }

    #[test]
    fn a_refusal_is_counted_and_logged_once_per_run() {
        let refused = Verdict::Refused {
            height: 100,
            statement_hash: H.into(),
            why: Mismatch::NoDiaryEntry { height: 100 },
        };
        let already = Verdict::AlreadySigned {
            height: 100,
            statement_hash: H.into(),
        };
        let mut t = Tally::default();
        let mut seen = HashSet::new();
        assert_eq!(
            t.add(std::slice::from_ref(&refused), &mut seen),
            vec![
                "did not sign the snapshot at 100: this node's diary has nothing at 100"
                    .to_string()
            ]
        );
        assert!(t.add(std::slice::from_ref(&refused), &mut seen).is_empty());
        assert!(t.add(&[already], &mut seen).is_empty());
        assert_eq!((t.rounds, t.refused, t.signed), (3, 1, 0));
        assert_eq!(
            t.report(),
            vec![
                "confirmer: 3 rounds, co-signed 0, refused 1, failed 0".to_string(),
                "confirmer, last refusal: did not sign the snapshot at 100: this node's diary has nothing at 100".to_string()
            ]
        );
    }

    #[test]
    fn the_diagnostics_lines_say_what_ran_and_what_did_not() {
        let mut r = NetworkReport::default();
        assert_eq!(
            r.lines(None),
            vec![
                "diary: empty".to_string(),
                "confirmer: no round yet in this run".to_string()
            ]
        );
        r.confirmer_off = Some("its key is not on the operator list".into());
        r.producer = Some("sent block 226200 to easybtx.com".into());
        r.diary = Some("wrote 226200".into());
        assert_eq!(
            r.lines(Some(
                "3 heights, newest 226200 (block 0123456789abcdef)".into()
            )),
            vec![
                "diary: 3 heights, newest 226200 (block 0123456789abcdef)".to_string(),
                "diary, last step: wrote 226200".to_string(),
                "confirmer: off, its key is not on the operator list".to_string(),
                "producer: sent block 226200 to easybtx.com".to_string(),
            ]
        );
    }

    #[test]
    fn only_a_validating_signing_listed_node_with_a_diary_confirms() {
        let dir = tempfile::tempdir().unwrap();
        let env = format!("confirmer={C}");
        let status = |mode: &str, signer: bool| crate::node_api::MatmulTrustedStatus {
            local_signer: signer,
            serves_attestations: true,
            matmul_validation_mode: mode.into(),
            trusted_mirror: mode == "trusted",
            replay_authority_context: None,
        };
        let ok = status("consensus", true);
        let mirror = status("trusted", true);
        let keyless = status("consensus", false);
        let run = |s, k: Option<&str>| why_not(s, k, REGTEST_GENESIS, Some(&env), dir.path());
        assert_eq!(run(&ok, Some(C)), None);
        assert_eq!(
            run(&mirror, Some(C)),
            Some("it follows signatures instead of checking blocks")
        );
        assert_eq!(run(&keyless, Some(C)), Some("it does not sign"));
        assert_eq!(
            run(&ok, Some(P)),
            Some("its key is not on the operator list")
        );
        assert_eq!(run(&ok, None), Some("its signing key could not be read"));
        let record = crate::node::attested_snapshot_record(dir.path());
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, b"v2").unwrap();
        assert_eq!(
            run(&ok, Some(C)),
            Some("it runs on a signed snapshot whose history check is still going")
        );
    }
}
````

In `crates/btx-core/src/lib.rs`, after `pub mod snapshot;` add `pub mod snapshot_confirmer;`.

- [ ] **Step 2: Run them to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot_confirmer::`
Expected: compile errors, among them `` cannot find struct, variant or union type `Confirmer` in this scope `` and `` cannot find type `Verdict` in this scope ``.

- [ ] **Step 3: Write the confirmer**

Insert above `#[cfg(test)]` in `crates/btx-core/src/snapshot_confirmer.rs`:

````rust
//! The confirmer (section 5 of docs/decisions/2026-09-29-every-node-starts-
//! near-the-tip.md): a node that validates, signs, and whose key is on the
//! operator list reads the statements waiting on easybtx.com about every ten
//! minutes, and co-signs one only when its OWN node agrees with it field by
//! field (`crate::statement_check`: chain, replay context, the base on its
//! active chain with 10 confirmations, no refused block below it, and its
//! diary's block hash, UTXO hash, coin count and transaction count).
//!
//! `signutxosnapshotmanifest` signs blindly, so it only ever sees a copy of a
//! statement that already passed; the app then checks that the one new
//! signature is this node's and valid, and sends back a manifest carrying
//! the statement and that signature alone. A refusal is logged once per
//! statement per run and counted for Copy diagnostics ([`Tally`]). Nothing
//! the website says is trusted: every statement is read from its own bytes.

use crate::confirmed_load::Holds;
use crate::confirmed_snapshot::{self as cs, ChainRules, Manifest};
use crate::operators::{self, Chain};
use crate::rpc::Rpc;
use crate::snapshot_serve::CONFIRMATIONS_REQUIRED;
use crate::snapshot_site::{self as site, PendingStatement};
use crate::statement_check::{check_against_node, Mismatch};
use serde::Serialize;
use serde_json::json;
use std::collections::HashSet;
use std::path::Path;

/// How often a confirmer reads what waits.
pub const CONFIRM_EVERY_SECS: u64 = 600;
/// Under `<datadir>/snapshots/`: where a copy is signed and removed again.
pub const WORK_DIR: &str = "confirm";

/// What one statement came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Signed {
        height: u64,
        statement_hash: String,
        operators: Vec<String>,
    },
    /// This node's operator signed it already, with this key or another.
    AlreadySigned { height: u64, statement_hash: String },
    /// This node does not agree with it. Never signed.
    Refused {
        height: u64,
        statement_hash: String,
        why: Mismatch,
    },
    /// Something failed on the way (the engine, the file, the website).
    /// Nothing was sent; the next round tries again.
    Failed {
        height: u64,
        statement_hash: String,
        error: String,
    },
}

impl Verdict {
    pub fn statement_hash(&self) -> &str {
        match self {
            Verdict::Signed { statement_hash, .. }
            | Verdict::AlreadySigned { statement_hash, .. }
            | Verdict::Refused { statement_hash, .. }
            | Verdict::Failed { statement_hash, .. } => statement_hash,
        }
    }

    /// One plain line for the node log.
    pub fn line(&self) -> String {
        match self {
            Verdict::Signed {
                height, operators, ..
            } => format!(
                "co-signed the snapshot at {height}; signed by {} now",
                operators.join(", ")
            ),
            Verdict::AlreadySigned { height, .. } => {
                format!("the snapshot at {height} carries this operator's signature already")
            }
            Verdict::Refused { height, why, .. } => {
                format!("did not sign the snapshot at {height}: {why}")
            }
            Verdict::Failed { height, error, .. } => {
                format!("could not co-sign the snapshot at {height} yet: {error}")
            }
        }
    }
}

/// Why this node does not confirm, or `None` when it does: it validates,
/// signs, its key is on the list of its chain, and it keeps a diary.
pub fn why_not(
    status: &crate::node_api::MatmulTrustedStatus,
    our_key_hex: Option<&str>,
    genesis: &str,
    regtest_env: Option<&str>,
    network_dir: &Path,
) -> Option<&'static str> {
    if !status
        .matmul_validation_mode
        .trim()
        .eq_ignore_ascii_case("consensus")
    {
        return Some("it follows signatures instead of checking blocks");
    }
    if !status.local_signer {
        return Some("it does not sign");
    }
    let Some(key) = our_key_hex else {
        return Some("its signing key could not be read");
    };
    if !crate::snapshot_producer::key_is_listed(key, genesis, regtest_env) {
        return Some("its key is not on the operator list");
    }
    if crate::node::attested_snapshot_record(network_dir).exists() {
        return Some("it runs on a signed snapshot whose history check is still going");
    }
    None
}

/// One confirmer, for one round.
pub struct Confirmer<'a> {
    pub rpc: &'a dyn Rpc,
    pub client: &'a reqwest::Client,
    pub site: &'a str,
    /// Holds the diary (the datadir).
    pub diary_dir: &'a Path,
    /// Where copies are signed; the engine must be able to write there, so
    /// inside the datadir (`<datadir>/snapshots/confirm`).
    pub work_dir: &'a Path,
    pub our_key: [u8; 33],
    pub holds: Holds<'a>,
    pub regtest_env: Option<&'a str>,
}

impl Confirmer<'_> {
    /// Every statement waiting on `chain`, newest first, as the website
    /// lists them.
    pub async fn round(&self, chain: Chain) -> Result<Vec<Verdict>, String> {
        let pending = site::get_pending(self.client, self.site, site::chain_name(chain))
            .await
            .map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(pending.statements.len());
        for p in &pending.statements {
            out.push(self.confirm_one(p).await);
        }
        Ok(out)
    }

    pub async fn confirm_one(&self, p: &PendingStatement) -> Verdict {
        let (height, statement_hash) = (p.height, p.statement_hash.to_ascii_lowercase());
        let refused = |why: Mismatch| Verdict::Refused {
            height,
            statement_hash: statement_hash.clone(),
            why,
        };
        let failed = |error: String| Verdict::Failed {
            height,
            statement_hash: statement_hash.clone(),
            error,
        };
        let Some(bytes) = operators::hex_decode(&p.manifest_hex) else {
            return refused(Mismatch::Unreadable("the manifest is not hex".into()));
        };
        let m = match cs::parse(&bytes) {
            Ok(m) => m,
            Err(e) => return refused(e.into()),
        };
        if m.statement.hash().display_hex() != statement_hash {
            return refused(Mismatch::Unreadable(
                "the manifest is not the statement it is listed as".into(),
            ));
        }
        let rules = match ChainRules::for_statement(&m.statement, self.regtest_env) {
            Ok(r) => r,
            Err(e) => return refused(e.into()),
        };
        let signed_by = match cs::confirming_operators(&m, &rules.operators) {
            Ok(o) => o,
            Err(e) => return refused(e.into()),
        };
        let Some(ours) = rules.operators.operator_of(&self.our_key) else {
            return failed("this node's key is not on the operator list".into());
        };
        if signed_by.iter().any(|o| o == ours) || m.signatures.iter().any(|s| s.key == self.our_key)
        {
            return Verdict::AlreadySigned {
                height,
                statement_hash,
            };
        }
        let diary = crate::diary::load(self.diary_dir, &m.statement.chain_id().display_hex());
        if let Err(why) = check_against_node(
            self.rpc,
            &m.statement,
            &rules,
            &diary,
            &self.holds,
            CONFIRMATIONS_REQUIRED,
        )
        .await
        {
            return refused(why);
        }
        let one = match self.sign_copy(&m, &statement_hash).await {
            Ok(one) => one,
            Err(e) => return failed(e),
        };
        match site::post_statement(self.client, self.site, &one.to_bytes()).await {
            Ok(reply) => Verdict::Signed {
                height,
                statement_hash,
                operators: reply.operators,
            },
            Err(e) => failed(e.to_string()),
        }
    }

    /// Sign a copy with the engine, and keep only this node's new signature,
    /// checked.
    async fn sign_copy(&self, m: &Manifest, statement_hash: &str) -> Result<Manifest, String> {
        std::fs::create_dir_all(self.work_dir).map_err(|e| format!("work folder: {e}"))?;
        let path = self.work_dir.join(format!("{statement_hash}.manifest"));
        std::fs::write(&path, m.to_bytes()).map_err(|e| format!("writing the copy: {e}"))?;
        let signed = self
            .rpc
            .call("signutxosnapshotmanifest", json!([path.to_string_lossy()]))
            .await;
        let after = std::fs::read(&path);
        let _ = std::fs::remove_file(&path);
        signed.map_err(|e| format!("the engine did not sign: {e}"))?;
        let after = cs::parse(&after.map_err(|e| format!("reading the copy: {e}"))?)
            .map_err(|e| format!("the signed copy does not read: {e}"))?;
        let ours = after
            .signatures
            .into_iter()
            .find(|s| s.key == self.our_key)
            .ok_or("the engine did not sign with this node's key")?;
        if after.statement != m.statement
            || !cs::signature_is_valid(&m.statement.hash(), &ours.key, &ours.der)
        {
            return Err("the engine's signature does not check out".into());
        }
        Ok(Manifest {
            statement: m.statement.clone(),
            signatures: vec![ours],
        })
    }
}

/// What the confirmer did this run, for Copy diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tally {
    pub rounds: u64,
    pub signed: u64,
    pub refused: u64,
    pub failed: u64,
    pub last_refusal: Option<String>,
    /// Why the last round could not run, or its error.
    pub last_error: Option<String>,
}

/// What the snapshot network did in this run of the app, for Copy
/// diagnostics. The status refresher writes the diary and confirmer parts,
/// the snapshot keeper the producer part.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkReport {
    /// The last diary step worth saying.
    pub diary: Option<String>,
    pub confirmer: Tally,
    /// Why this node does not confirm ([`why_not`]).
    pub confirmer_off: Option<String>,
    /// What the refresher already logged in this run (see [`Tally::add`]).
    pub seen: HashSet<String>,
    /// The producer's last word.
    pub producer: Option<String>,
}

impl NetworkReport {
    /// The "Snapshots" section of Copy diagnostics. `diary_summary` is
    /// `diary::summary` of the datadir.
    pub fn lines(&self, diary_summary: Option<String>) -> Vec<String> {
        let mut out = vec![format!(
            "diary: {}",
            diary_summary.unwrap_or_else(|| "empty".into())
        )];
        if let Some(d) = &self.diary {
            out.push(format!("diary, last step: {d}"));
        }
        match &self.confirmer_off {
            Some(why) => out.push(format!("confirmer: off, {why}")),
            None if self.confirmer.rounds == 0 && self.confirmer.last_error.is_none() => {
                out.push("confirmer: no round yet in this run".into())
            }
            None => out.extend(self.confirmer.report()),
        }
        if let Some(p) = &self.producer {
            out.push(format!("producer: {p}"));
        }
        out
    }
}

fn kind(v: &Verdict) -> &'static str {
    match v {
        Verdict::Signed { .. } => "signed",
        Verdict::AlreadySigned { .. } => "already",
        Verdict::Refused { .. } => "refused",
        Verdict::Failed { .. } => "failed",
    }
}

impl Tally {
    /// Count a round. Returns the log lines for what this run has not
    /// reported yet (`seen` lives as long as the run), so a refusal is
    /// logged and counted once, not every ten minutes. A statement already
    /// signed is the quiet, normal state and is not logged.
    pub fn add(&mut self, verdicts: &[Verdict], seen: &mut HashSet<String>) -> Vec<String> {
        self.rounds += 1;
        self.last_error = None;
        let mut lines = Vec::new();
        for v in verdicts {
            let first = seen.insert(format!("{}:{}", kind(v), v.statement_hash()));
            match v {
                Verdict::Signed { .. } => self.signed += 1,
                Verdict::Refused { .. } => {
                    self.last_refusal = Some(v.line());
                    if first {
                        self.refused += 1;
                    }
                }
                Verdict::Failed { .. } if first => self.failed += 1,
                Verdict::Failed { .. } | Verdict::AlreadySigned { .. } => {}
            }
            if first && !matches!(v, Verdict::AlreadySigned { .. }) {
                lines.push(v.line());
            }
        }
        lines
    }

    /// The lines Copy diagnostics shows.
    pub fn report(&self) -> Vec<String> {
        let mut out = vec![format!(
            "confirmer: {} rounds, co-signed {}, refused {}, failed {}",
            self.rounds, self.signed, self.refused, self.failed
        )];
        if let Some(r) = &self.last_refusal {
            out.push(format!("confirmer, last refusal: {r}"));
        }
        if let Some(e) = &self.last_error {
            out.push(format!("confirmer, last round: {e}"));
        }
        out
    }
}
````

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot_confirmer::`
Expected: `8 passed; 0 failed`.

- [ ] **Step 5: Format, lint, commit**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cargo test --locked --lib && cd ../..
git add crates/btx-core/src/snapshot_confirmer.rs crates/btx-core/src/lib.rs
git commit -m "core: confirmers co-sign a waiting snapshot only when their own diary agrees" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 7: The app drives it, and Copy diagnostics says what it did

**Files:**
- Modify: `crates/btx-core/src/diagnostics.rs` (`snapshots` on `DiagnosticsInput`, a "Snapshots" section; tests)
- Modify: `apps/node/src-tauri/src/state.rs` (`snapshot_network` on `AppState`; the settings comment)
- Modify: `apps/node/src-tauri/src/commands.rs` (refresher: diary step and confirmer round; keeper: grid, diary first, producer checks, submission; `diary_step`, `confirm_round`, `submit_offered_pair`; copy and comments)
- Modify: `apps/node/src-tauri/src/tools.rs` (the section's lines)
- Modify: `apps/node/index.html`, `apps/node/src/main.ts` (one string each)
- Modify: `apps/node/CHANGELOG.md`

**Interfaces:**
- Consumes: Tasks 2 to 6 (`diary::{chain_id, grid_for, record_at_tip, summary, DiaryOutcome}`, `snapshot_serve::{export_due, blocks_to_grid, EXPORT_GRID, run_cycle}`, `snapshot_producer::{ProducerChecks, check_before_send, submit, submission_due, load_submitted, key_is_listed, RESUBMIT_EVERY}`, `snapshot_confirmer::{Confirmer, why_not, NetworkReport, CONFIRM_EVERY_SECS, WORK_DIR}`, `snapshot_site::{client, site_base}`), `confirmed_load::Holds::compiled()`, `operators::{regtest_env, parse_key, Chain}`; the app's existing `AppState` slots `matmul_trusted`, `signer_pubkey`, `node_datadir()`.
- Produces: `AppState::snapshot_network: Arc<tokio::sync::Mutex<NetworkReport>>`; `DiagnosticsInput::snapshots: Vec<String>`.

- [ ] **Step 1: Write the failing diagnostics tests**

In `crates/btx-core/src/diagnostics.rs`, in the test module:

*Test literal: the private-values test.* Replace:

````rust
            attested_tip: None,
            stall: None,
            log_warnings: vec![format!(
````

with:

````rust
            attested_tip: None,
            stall: None,
            snapshots: vec![],
            log_warnings: vec![format!(
````

*Test literal: every section.* Replace:

````rust
            attested_tip: None,
            stall: None,
            log_warnings: vec!["[warning] something".into()],
        };
        let r = render(&input);
        for part in [
````

with:

````rust
            attested_tip: None,
            stall: None,
            snapshots: vec![
                "diary: 12 heights, newest 233,400".into(),
                "confirmer: 3 rounds, co-signed 1, refused 0, failed 0".into(),
            ],
            log_warnings: vec!["[warning] something".into()],
        };
        let r = render(&input);
        for part in [
            "Snapshots\n  diary: 12 heights, newest 233,400\n  confirmer: 3 rounds",
````

*Test: an empty section says so.* Replace:

````rust
    #[test]
    fn the_report_has_every_section_and_no_em_dash() {
````

with:

````rust
    #[test]
    fn a_node_that_did_nothing_for_the_snapshot_network_says_so() {
        let r = render(&DiagnosticsInput::default());
        assert!(r.contains("Snapshots\n  nothing to report\n"), "{r}");
    }

    #[test]
    fn the_report_has_every_section_and_no_em_dash() {
````

- [ ] **Step 2: Run them to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- diagnostics::`
Expected: compile errors `` struct `DiagnosticsInput` has no field named `snapshots` ``.

- [ ] **Step 3: Add the section**

In `crates/btx-core/src/diagnostics.rs`, above the test module:

*The input carries the snapshot lines.* Replace:

````rust
    pub stall: Option<String>,
    pub log_warnings: Vec<String>,
}
````

with:

````rust
    pub stall: Option<String>,
    /// What the snapshot network did in this run: the diary, the confirmer,
    /// the producer (`snapshot_confirmer::NetworkReport::lines`).
    pub snapshots: Vec<String>,
    pub log_warnings: Vec<String>,
}
````

*The section, before the engine notices.* Replace:

````rust
    let notices = i
        .chain
````

with:

````rust
    o.push("Snapshots".into());
    if i.snapshots.is_empty() {
        o.push("  nothing to report".into());
    }
    for l in &i.snapshots {
        o.push(format!("  {l}"));
    }
    let notices = i
        .chain
````

Run: `cd crates/btx-core && cargo test --locked --lib -- diagnostics::`
Expected: `40 passed; 0 failed`.

- [ ] **Step 4: The report slot on `AppState`**

In `apps/node/src-tauri/src/state.rs`:

*The settings comment.* Replace:

````rust
    /// with this node's key, wait for it to mature, offer it over P2P, refresh
    /// it every 500 blocks, and re-offer it after every node start. Off by
````

with:

````rust
    /// with this node's key, wait for it to mature, offer it over P2P, take a
    /// fresh one at every multiple of 200 blocks, and re-offer it after every
    /// node start. Off by
````

*The field.* Replace:

````rust
    pub snapshot_serve_gen: Arc<AtomicU64>,
}
````

with:

````rust
    pub snapshot_serve_gen: Arc<AtomicU64>,
    /// What the snapshot network did in this run (the diary, the confirmer,
    /// the producer), for Copy diagnostics. Written by the status refresher
    /// and the snapshot keeper.
    pub snapshot_network: Arc<Mutex<btx_core::snapshot_confirmer::NetworkReport>>,
}
````

*The initial value.* Replace:

````rust
            snapshot_serve_gen: Arc::new(AtomicU64::new(0)),
        }
````

with:

````rust
            snapshot_serve_gen: Arc::new(AtomicU64::new(0)),
            snapshot_network: Arc::new(Mutex::new(Default::default())),
        }
````

- [ ] **Step 5: The refresher, the keeper and the copy in `commands.rs`**

In `apps/node/src-tauri/src/commands.rs`, in this order (each old text appears exactly once):

*Start path comment.* Replace:

````rust
    // node is at the tip (the offer never survives a restart), and refreshes
    // it every 500 blocks. Gated against the LIVE node on every tick.
````

with:

````rust
    // node is at the tip (the offer never survives a restart), and takes a
    // fresh one at every multiple of 200 blocks. Gated against the LIVE node
    // on every tick.
````

*Refresher: the report slot.* Replace:

````rust
    let engine_warnings_slot = state.engine_warnings.clone();
````

with:

````rust
    let engine_warnings_slot = state.engine_warnings.clone();
    let snapshot_network_slot = state.snapshot_network.clone();
````

*Refresher: its state.* Replace:

````rust
        let mut bootstrap_moved_at = std::time::Instant::now();
````

with:

````rust
        let mut bootstrap_moved_at = std::time::Instant::now();
        // The snapshot network (btx_core::diary, btx_core::snapshot_confirmer):
        // the chain's genesis, read once; whether a diary step or a confirmer
        // round is running; the confirmer's counter, which starts near the
        // top so the first round goes out about two minutes in.
        let mut snapshot_genesis: Option<String> = None;
        let diary_in_flight = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let confirm_in_flight = Arc::new(std::sync::atomic::AtomicBool::new(false));
        const CONFIRM_EVERY: u32 = (btx_core::snapshot_confirmer::CONFIRM_EVERY_SECS / 3) as u32;
        let mut confirm_tick: u32 = CONFIRM_EVERY.saturating_sub(40);
````

*Refresher: the diary and the confirmer, after the check-in.* Replace:

````rust
                                *signer_offer_slot.lock().await = Some(status);
                            }
                        }
                    }
                }
                Err(AppError::Rpc { code: -28, .. }) => {
````

with:

````rust
                                *signer_offer_slot.lock().await = Some(status);
                            }
                        }
                    }

                    // ── The diary and the confirmer (snapshot network) ──────
                    //
                    // The diary writes down this node's own chain state when
                    // the tip is on the grid (btx_core::diary); the confirmer
                    // co-signs a statement waiting on easybtx.com only when
                    // that diary agrees (btx_core::snapshot_confirmer). Both
                    // run off the tick: gettxoutsetinfo and a round of HTTP
                    // can outlast it.
                    if snapshot_genesis.is_none() {
                        snapshot_genesis = btx_core::diary::chain_id(&rpc).await;
                    }
                    let snapshot_grid = snapshot_genesis
                        .as_deref()
                        .and_then(btx_core::diary::grid_for);
                    if snapshot_grid.is_some_and(|g| chain.blocks > 0 && chain.blocks % g == 0)
                        && !diary_in_flight.swap(true, Ordering::SeqCst)
                    {
                        let rpc = rpc.clone();
                        let in_flight = diary_in_flight.clone();
                        let report = snapshot_network_slot.clone();
                        let mode = matmul_trusted_slot
                            .lock()
                            .await
                            .as_ref()
                            .map(|s| s.matmul_validation_mode.clone());
                        tauri::async_runtime::spawn(async move {
                            diary_step(&rpc, mode, &report).await;
                            in_flight.store(false, Ordering::SeqCst);
                        });
                    }
                    confirm_tick += 1;
                    if confirm_tick >= CONFIRM_EVERY
                        && !confirm_in_flight.swap(true, Ordering::SeqCst)
                    {
                        confirm_tick = 0;
                        let rpc = rpc.clone();
                        let in_flight = confirm_in_flight.clone();
                        let report = snapshot_network_slot.clone();
                        let trusted = matmul_trusted_slot.lock().await.clone();
                        let key = signer_pubkey_slot.lock().await.clone();
                        let genesis = snapshot_genesis.clone();
                        tauri::async_runtime::spawn(async move {
                            confirm_round(&rpc, trusted, key, genesis, &report).await;
                            in_flight.store(false, Ordering::SeqCst);
                        });
                    }
                }
                Err(AppError::Rpc { code: -28, .. }) => {
````

*Keeper comment.* Replace:

````rust
// the mirrors because service bits travel only in the handshake, refreshes
// every 500 blocks, and re-offers after every node start because the offer
````

with:

````rust
// the mirrors because service bits travel only in the handshake, takes a
// fresh one at every multiple of 200 blocks (checked against the node's own
// diary before anything is offered, and sent to easybtx.com when the node's
// key is on the operator list), and re-offers after every node
// start because the offer
````

*Keeper: the diary step and the confirmer round, as plain functions.* Replace:

````rust
/// The keeper: one loop per node start, superseded by the generation counter
````

with:

````rust
/// One diary step (btx_core::diary), from the status refresher when the tip
/// is on the grid. Says in the report what came of it when that is news.
async fn diary_step(
    rpc: &RpcClient,
    validation_mode: Option<String>,
    report: &Arc<tokio::sync::Mutex<btx_core::snapshot_confirmer::NetworkReport>>,
) {
    use btx_core::diary::{record_at_tip, DiaryOutcome};
    let dd = node_datadir();
    let note = match record_at_tip(rpc, &dd, &dd, validation_mode.as_deref()).await {
        Ok(DiaryOutcome::Recorded(e)) => {
            eprintln!(
                "[snapshot] diary: wrote {} (block {}, {} coins)",
                e.height, e.block_hash, e.coins
            );
            Some(format!(
                "wrote {} (block {})",
                e.height,
                &e.block_hash[..16]
            ))
        }
        Ok(DiaryOutcome::TipMoved) => Some(
            "the tip moved while the chain state was read; the next chance is the next grid \
             height"
                .to_string(),
        ),
        Ok(DiaryOutcome::NotAllowed) => Some(
            "not kept: this node follows signatures, or runs on a signed snapshot whose \
             history check is still going"
                .to_string(),
        ),
        Ok(DiaryOutcome::AlreadyRecorded(_) | DiaryOutcome::NotOnGrid) => None,
        Err(e) => {
            eprintln!("[snapshot] diary: {e}");
            Some(e)
        }
    };
    if let Some(n) = note {
        report.lock().await.diary = Some(n);
    }
}

/// One confirmer round (btx_core::snapshot_confirmer), from the status
/// refresher every ten minutes. When the node does not confirm, the report
/// says why and nothing is asked of easybtx.com.
async fn confirm_round(
    rpc: &RpcClient,
    trusted: Option<btx_core::node_api::MatmulTrustedStatus>,
    key: Option<String>,
    genesis: Option<String>,
    report: &Arc<tokio::sync::Mutex<btx_core::snapshot_confirmer::NetworkReport>>,
) {
    use btx_core::snapshot_confirmer as confirmer;
    let dd = node_datadir();
    let env = btx_core::operators::regtest_env();
    let (Some(trusted), Some(genesis)) = (trusted, genesis) else {
        report.lock().await.confirmer_off = Some("the node has not answered yet".into());
        return;
    };
    if let Some(why) = confirmer::why_not(&trusted, key.as_deref(), &genesis, env.as_deref(), &dd) {
        report.lock().await.confirmer_off = Some(why.into());
        return;
    }
    let (Some(our_key), Some(chain)) = (
        key.as_deref().and_then(btx_core::operators::parse_key),
        btx_core::operators::Chain::from_genesis_hex(&genesis),
    ) else {
        return;
    };
    let client = match btx_core::snapshot_site::client() {
        Ok(c) => c,
        Err(e) => {
            report.lock().await.confirmer.last_error = Some(e);
            return;
        }
    };
    let site = btx_core::snapshot_site::site_base();
    let work = snap::snapshot_dir(&dd).join(confirmer::WORK_DIR);
    let c = confirmer::Confirmer {
        rpc,
        client: &client,
        site: &site,
        diary_dir: &dd,
        work_dir: &work,
        our_key,
        holds: btx_core::confirmed_load::Holds::compiled(),
        regtest_env: env.as_deref(),
    };
    let round = c.round(chain).await;
    let mut guard = report.lock().await;
    let r = &mut *guard;
    r.confirmer_off = None;
    match round {
        Ok(verdicts) => {
            for line in r.confirmer.add(&verdicts, &mut r.seen) {
                eprintln!("[snapshot] {line}");
            }
        }
        Err(e) => {
            eprintln!("[snapshot] confirmer: {e}");
            r.confirmer.last_error = Some(e);
        }
    }
}

/// Section 4's last step: check the offered pair once more and send it to
/// easybtx.com. Only for a node whose key is on the operator list. The line
/// it returns goes to the log and to Copy diagnostics.
async fn submit_offered_pair(rpc: &RpcClient, datadir: &Path, height: u64) -> String {
    use btx_core::snapshot_producer as producer;
    let dir = snap::snapshot_dir(datadir);
    let manifest = match std::fs::read(dir.join(snap::manifest_file_name(height))) {
        Ok(m) => m,
        Err(e) => return format!("block {height}: the manifest could not be read ({e})"),
    };
    let env = btx_core::operators::regtest_env();
    let holds = btx_core::confirmed_load::Holds::compiled();
    if let Err(why) =
        producer::check_before_send(rpc, &manifest, datadir, &holds, env.as_deref()).await
    {
        return format!("not sending block {height}: {why}");
    }
    let client = match btx_core::snapshot_site::client() {
        Ok(c) => c,
        Err(e) => return e,
    };
    match producer::submit(&client, &btx_core::snapshot_site::site_base(), &dir, height).await {
        Ok(s) => format!(
            "sent block {height} to easybtx.com (statement {})",
            &s.statement_hash[..16.min(s.statement_hash.len())]
        ),
        Err(e) => format!(
            "sending block {height} to easybtx.com failed; trying again in ten minutes: {e}"
        ),
    }
}

/// The keeper: one loop per node start, superseded by the generation counter
````

*Keeper: slots.* Replace:

````rust
    let quitting = state.quitting.clone();
    tauri::async_runtime::spawn(async move {
        let dir = snap::snapshot_dir(&datadir);
````

with:

````rust
    let quitting = state.quitting.clone();
    let pubkey_slot = state.signer_pubkey.clone();
    let report_slot = state.snapshot_network.clone();
    tauri::async_runtime::spawn(async move {
        let dir = snap::snapshot_dir(&datadir);
        // The chain's genesis (read once) names its grid; the last attempt
        // to send the offered pair to easybtx.com paces the retries.
        let mut genesis: Option<String> = None;
        let mut last_submit: Option<(u64, std::time::Instant)> = None;
````

*Keeper: the grid, and a closer look near it.* Replace:

````rust
            let stale_by = tip.zip(base).map(|(t, b)| t.saturating_sub(b));
````

with:

````rust
            let stale_by = tip.zip(base).map(|(t, b)| t.saturating_sub(b));
            if genesis.is_none() {
                genesis = btx_core::diary::chain_id(&rpc).await;
            }
            let grid = genesis
                .as_deref()
                .and_then(btx_core::diary::grid_for)
                .unwrap_or(snap::EXPORT_GRID);
            // Close to a grid height, look every few seconds: the export has
            // to happen while the tip is exactly there.
            if tip.is_some_and(|t| snap::blocks_to_grid(t, grid) <= 2) {
                wait = std::time::Duration::from_secs(3);
            }
````

*Keeper: export on the grid, diary first.* Replace:

````rust
            if snap::refresh_due(tip.unwrap_or(0), base) {
                eprintln!(
                    "[snapshot] tip {} is {} blocks past base {}; exporting a fresh snapshot",
                    tip.unwrap_or(0),
                    stale_by.unwrap_or(0),
                    base.map(|b| b.to_string()).unwrap_or_else(|| "none".into())
                );
````

with:

````rust
            if snap::export_due(tip.unwrap_or(0), grid, base) {
                eprintln!(
                    "[snapshot] the tip is at {}, a multiple of {grid}; exporting a fresh snapshot",
                    tip.unwrap_or(0)
                );
                // The diary first, from the same tip, so the check after the
                // base matures has this height to compare with.
                match btx_core::diary::record_at_tip(
                    &rpc,
                    &datadir,
                    &datadir,
                    facts.validation_mode.as_deref(),
                )
                .await
                {
                    Ok(btx_core::diary::DiaryOutcome::Recorded(e)) => {
                        eprintln!(
                            "[snapshot] diary: wrote {} (block {})",
                            e.height, e.block_hash
                        )
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("[snapshot] diary: {e}"),
                }
````

*Keeper: the cycle on the grid, with the producer's checks.* Replace:

````rust
                let keep_going = alive.clone();
                match snap::run_cycle(
                    &rpc,
                    &dir,
                    snap::MATURE_POLL,
                    snap::MATURE_DEADLINE,
                    &keep_going,
                    &on_phase,
                )
                .await
````

with:

````rust
                let keep_going = alive.clone();
                let checks = btx_core::snapshot_producer::ProducerChecks {
                    diary_dir: datadir.clone(),
                    holds: btx_core::confirmed_load::Holds::compiled(),
                    regtest_env: btx_core::operators::regtest_env(),
                };
                match snap::run_cycle(
                    &rpc,
                    &dir,
                    grid,
                    snap::MATURE_POLL,
                    snap::MATURE_DEADLINE,
                    &keep_going,
                    &on_phase,
                    &checks,
                )
                .await
````

*Keeper: what a failed cycle says.* Replace:

````rust
                            message: format!("The last export did not complete: {e}. Trying again."),
````

with:

````rust
                            message: format!(
                                "The last export did not complete: {e}. The next one is taken at \
                                 the next multiple of {grid} blocks."
                            ),
````

*Keeper: send the offered pair.* Replace:

````rust
            let offering = snap::offer_live(&rpc).await;
            let peers_offering = snap::peers_offering(&rpc).await;
````

with:

````rust
            // ── Send the offered pair to easybtx.com (section 4) ────────────
            // Once per pair when the key is not on the list (to say so), and
            // for a listed key every ten minutes until the website has it.
            if let (Some(r), Some(g)) = (record.as_ref(), genesis.as_deref()) {
                use btx_core::snapshot_producer as producer;
                let key = pubkey_slot.lock().await.clone();
                let env = btx_core::operators::regtest_env();
                let listed = key
                    .as_deref()
                    .is_some_and(|k| producer::key_is_listed(k, g, env.as_deref()));
                let first_look = last_submit.is_none_or(|(h, _)| h != r.height);
                let retry = last_submit.is_some_and(|(h, at)| {
                    h == r.height && at.elapsed() >= producer::RESUBMIT_EVERY
                });
                if (first_look || (listed && retry))
                    && producer::submission_due(r, producer::load_submitted(&dir).as_ref(), grid)
                {
                    last_submit = Some((r.height, std::time::Instant::now()));
                    let line = if listed {
                        submit_offered_pair(&rpc, &datadir, r.height).await
                    } else {
                        format!(
                            "offering block {} on the network; this node's key is not on the \
                             operator list, so nothing is sent to easybtx.com",
                            r.height
                        )
                    };
                    eprintln!("[snapshot] {line}");
                    report_slot.lock().await.producer = Some(line);
                }
            }

            let offering = snap::offer_live(&rpc).await;
            let peers_offering = snap::peers_offering(&rpc).await;
````

*The switch's answer.* Replace:

````rust
        "On. The first snapshot is exported at the tip and offered once it has ten \
         confirmations, about fifteen minutes; after that it is refreshed every 500 blocks."
````

with:

````rust
        "On. A snapshot is exported when the tip reaches the next multiple of 200 blocks \
         and offered once it has ten confirmations, about fifteen minutes later. After that, \
         every 200 blocks."
````

- [ ] **Step 6: Copy diagnostics reads the report**

In `apps/node/src-tauri/src/tools.rs`, in `tools_diagnostics`:

*The Snapshots section.* Replace:

````rust
    let ctx = RedactionContext {
        home: home_dir_display(),
````

with:

````rust
    input.snapshots = state
        .snapshot_network
        .lock()
        .await
        .lines(btx_core::diary::summary(&datadir));
    let ctx = RedactionContext {
        home: home_dir_display(),
````

- [ ] **Step 7: The Settings row's words**

In `apps/node/index.html` and in `apps/node/src/main.ts` (`SNAPSHOT_SERVE_STATIC_COPY`), the same change:

*The static copy.* Replace:

````text
About 9 MB, refreshed every 500 blocks
````

with:

````text
About 9 MB, taken every 200 blocks
````

- [ ] **Step 8: The changelog**

In `apps/node/CHANGELOG.md`, under `## [Unreleased]`:

*The entry, after the confirmed-snapshots one.* Replace:

````markdown
the list, so for now every node takes that second path.
````

with:

````markdown
the list, so for now every node takes that second path.

**Nodes that check blocks now help confirm snapshots.** A node that checks
blocks itself now writes down what its chain looked like every 200 blocks. If
you serve a chain snapshot, a fresh one is taken at every multiple of 200
blocks, compared with that record before it is offered, and sent to
easybtx.com when your key is on the operator list. A node on the list also
co-signs snapshots other operators sent, but only when its own record matches
every detail. Copy diagnostics has a new "Snapshots" section that says what
your node did.
````

(If the confirmed-snapshots entry reads differently by now, put this paragraph right after it.)

- [ ] **Step 9: Every gate**

````bash
for c in crates/btx-core apps/node/src-tauri; do (cd $c && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cargo test --locked); done
(cd apps/node && npx tsc --noEmit && npm test && npx vite build)
````

Expected: all pass (measured on this plan's dry run over the confirmed-snapshots core: btx-core `689 passed; 0 failed; 2 ignored` in the library tests, the Tauri crate `108 passed; 0 failed; 1 ignored`, `npm test` `128 passed`, `tsc` and `vite build` clean). Known flake: see Global Constraints.

- [ ] **Step 10: Check by hand on the owner's Mac (mainnet, a validating node that signs)**

Start the app, open Tools, press Copy diagnostics. Expected in the "Snapshots" section: `diary: empty` until the tip reaches a multiple of 200, then `diary: 1 height, newest <height> (block <16 hex>)` and `diary, last step: wrote <height> (block …)`; `confirmer: no round yet in this run` for the first two minutes, then either `confirmer: off, …` with the reason (on this Mac: `it follows signatures instead of checking blocks` if it runs as a mirror, since its device fails the engine's self-test) or `confirmer: 1 rounds, co-signed 0, refused 0, failed 0`. The node log shows `[snapshot] diary: wrote …` once per grid height. Nothing is sent to easybtx.com from a node whose key is not on the list.

- [ ] **Step 11: Commit**

````bash
git add crates/btx-core/src/diagnostics.rs apps/node/src-tauri/src/state.rs apps/node/src-tauri/src/commands.rs apps/node/src-tauri/src/tools.rs apps/node/index.html apps/node/src/main.ts apps/node/CHANGELOG.md
git commit -m "node: keep the diary, confirm and send snapshots, and say so in Copy diagnostics" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 8: The rehearsal: five engines and the website's own code on regtest

**Files:**
- Create: `crates/btx-core/tests/snapshot_network_regtest.rs`

**Interfaces:**
- Consumes: everything above through the public API, plus `attested_snapshot::prepare_confirmed` and `confirmed_load::{node_view, load, CliRunner, Holds}` (core plan, Tasks 3 and 4), and the website plan's `site/scripts/snapshot-rendezvous-local.mjs` (Task 6 there).
- Produces: `producers_confirmers_the_website_and_a_mirror_on_regtest`, `#[ignore]`, opt-in by `EASYNODE_TEST_BTXD` and `EASYNODE_TEST_SITE_DIR`.

**Which website:** the local stand-in, `node scripts/snapshot-rendezvous-local.mjs`, not `vercel dev`. It serves the same handler functions the Astro routes wrap (`site/src/lib/snapshotRoutes.mjs`), starts in under a second with nothing but `npm ci`, and needs no Vercel login or Blob token (a local Vercel would still talk to the real Blob store). `astro dev` serves the real route files too and was checked by hand with the same requests (website plan, Task 6, Step 2); the stand-in is what the test runs.

What it proves, in order, each against real v0.34.9 engines on regtest (measured 2026-09-29: passed 3 runs in a row, about 65 s each):

1. P, C and D share a chain to 100 and each writes its diary there; P exports at 100, matures to 110, and its own export passes `check_before_send` (the diary and the export agree on block, coins and transactions).
2. **Refused, a wrong file hash:** the website answers 422 `the file does not match the statement` to a file with one bit flipped, and keeps nothing.
3. **Refused, one operator:** P's real statement and file are stored; `latest` stays 404.
4. **Refused, a diary mismatch:** D's diary is edited one coin up; D's round refuses with `Mismatch::Coins` and signs nothing; `latest` stays 404.
5. **Works:** C co-signs; `latest` names 100 with operators `producer, confirmer1`; D (diary restored) co-signs too.
6. **Works, the load:** mirror M, pinning only P, gets headers from P, and `prepare_confirmed` plus `confirmed_load::load` load the confirmed pair, trimmed to P's one signature; its chain state's snapshot base is P's block 100.
7. **Refused, a held branch:** R takes the chain at 110, disconnects, and builds its own branch to 210, with D following it (D's diary records R's block at 200). R exports at 200 and sends it without checks, as a rogue would. With a hold on R's block 111: D, whose diary matches R's statement and which has 10 confirmations on it, refuses with `HeldRootOnActiveChain { height: 111 }`; C, off that branch, refuses with `BaseNotOnActiveChain { height: 200 }`; `check_before_send` on R with the hold refuses too (an honest producer would not send it); `latest` still names 100; `pending` shows 200 signed by `rogue` alone.

- [ ] **Step 1: Write the test**

Create `crates/btx-core/tests/snapshot_network_regtest.rs`:

````rust
//! The snapshot network end to end, on regtest: five real engines and the
//! website's own route code, through the app's own functions. Opt-in:
//!
//! ```text
//! EASYNODE_TEST_BTXD=/path/to/btxd \
//! EASYNODE_TEST_SITE_DIR=/path/to/EasyBTX/site \
//!   cargo test --test snapshot_network_regtest -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `EASYNODE_TEST_SITE_DIR` is the `site/` folder of an EasyBTX checkout that
//! has `scripts/snapshot-rendezvous-local.mjs` (the website plan) and its
//! `node_modules` installed (`npm ci`). The test runs that stand-in: the same
//! handlers the Astro routes on easybtx.com run, over `node:http`, with a
//! folder instead of Vercel Blob. btx-cli is taken from beside btxd, or from
//! `EASYNODE_TEST_BTX_CLI`. Keys are made fresh for each run.
//!
//! The cast: P, the producer; C and D, two confirmers; M, a mirror that pins
//! only P, as mainnet mirrors pin the 3060; R, a rogue producer whose key is
//! on the list. The list is `producer=P;confirmer1=C;confirmer2=D;rogue=R`,
//! in `EASYNODE_REGTEST_OPERATORS`'s format, handed to the app's functions
//! and to the website.
//!
//! Proven refused: a file whose hash is not the statement's (the website
//! keeps nothing); a statement one operator signed (`latest` stays 404 and
//! the mirror's loader finds nothing to load); a confirmer whose diary
//! disagrees (it signs nothing); a statement on a held branch (an honest
//! producer does not send it; the confirmer on that branch and the one off it
//! both refuse it; `latest` never points at it). Proven working: a producer's
//! export matching its own diary, two confirmers co-signing, `latest`
//! pointing at the statement, and the mirror loading it through the app's
//! loader, trimmed to the one key it pins.
//!
//! Regtest facts this test relies on, measured with v0.34.9 on 2026-09-29:
//! blocks above 100 connect with `-allowunverifiablematmulconsensus=1` on a
//! device that fails the engine's self-test; the miner's own block 100
//! connects only after a restart; a validating node that syncs from the
//! producer connects 100 without one; at 100 `gettxoutsetinfo` and
//! `getchaintxstats` give the statement's UTXO hash, coin count (101) and
//! transaction count (101).

use btx_core::confirmed_load::{self, CliRunner, Holds};
use btx_core::confirmed_snapshot as cs;
use btx_core::diary::{self, DiaryOutcome};
use btx_core::known_invalid::HeldBranch;
use btx_core::operators::Chain;
use btx_core::rpc::{Rpc, RpcClient};
use btx_core::snapshot_confirmer::{Confirmer, Verdict, WORK_DIR};
use btx_core::snapshot_serve::{self as serve, manifest_file_name, snapshot_file_name};
use btx_core::snapshot_site::{self as site, SiteError};
use btx_core::statement_check::Mismatch;
use btx_core::{attested_snapshot, snapshot_producer as producer};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SITE_PORT: u16 = 29650;

/// A fresh key: (regtest WIF, compressed public key hex).
fn regtest_key() -> (String, String) {
    let sk = k256::SecretKey::random(&mut rand_core::OsRng);
    let mut payload = vec![0xef];
    payload.extend_from_slice(&sk.to_bytes());
    payload.push(0x01);
    let wif = bs58::encode(payload).with_check().into_string();
    let hex: String = sk
        .public_key()
        .to_encoded_point(true)
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    (wif, hex)
}

/// One regtest btxd in its own folder, killed and reaped on drop.
struct Node {
    btxd: PathBuf,
    dir: PathBuf,
    rpc_port: u16,
    p2p_port: u16,
    args: Vec<String>,
    child: Option<std::process::Child>,
}

impl Node {
    fn new(btxd: &Path, dir: PathBuf, rpc_port: u16, args: Vec<String>) -> Self {
        std::fs::create_dir_all(dir.join("regtest")).unwrap();
        Self {
            btxd: btxd.to_path_buf(),
            dir,
            rpc_port,
            p2p_port: rpc_port + 100,
            args,
            child: None,
        }
    }

    /// Where the engine keeps regtest's chain: also where the app keeps the
    /// diary and the snapshot pairs in this test.
    fn net(&self) -> PathBuf {
        self.dir.join("regtest")
    }

    fn snapshots(&self) -> PathBuf {
        serve::snapshot_dir(&self.net())
    }

    fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.p2p_port)
    }

    fn log_tail(&self) -> String {
        let log = std::fs::read_to_string(self.net().join("debug.log")).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        lines[lines.len().saturating_sub(40)..].join("\n")
    }

    async fn start(&mut self) -> RpcClient {
        let _ = std::fs::remove_file(self.net().join(".cookie"));
        let child = std::process::Command::new(&self.btxd)
            .arg("-regtest")
            .arg(format!("-datadir={}", self.dir.display()))
            .arg(format!("-rpcport={}", self.rpc_port))
            .arg(format!("-port={}", self.p2p_port))
            .args([
                "-server=1",
                "-bind=127.0.0.1",
                "-listen=1",
                "-discover=0",
                "-dnsseed=0",
                "-fixedseeds=0",
                "-upnp=0",
                "-natpmp=0",
                "-printtoconsole=0",
                "-daemon=0",
            ])
            .args(&self.args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn btxd");
        self.child = Some(child);
        let url = format!("http://127.0.0.1:{}", self.rpc_port);
        for _ in 0..240 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                panic!("btxd exited with {status}:\n{}", self.log_tail());
            }
            if let Ok(c) = RpcClient::from_cookie(url.clone(), &self.net().join(".cookie")) {
                if c.call("getblockcount", json!([])).await.is_ok() {
                    return c;
                }
            }
        }
        panic!("no RPC within 120 s:\n{}", self.log_tail());
    }

    async fn stop(&mut self, rpc: &RpcClient) {
        let _ = rpc.call("stop", json!([])).await;
        if let Some(mut child) = self.child.take() {
            for _ in 0..120 {
                if child.try_wait().unwrap().is_some() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The website's handlers, served locally, killed on drop.
struct Site(std::process::Child);

impl Site {
    async fn start(site_dir: &Path, store: &Path, operators: &str) -> Self {
        let child = std::process::Command::new("node")
            .current_dir(site_dir)
            .args(["scripts/snapshot-rendezvous-local.mjs", "--port"])
            .arg(SITE_PORT.to_string())
            .arg("--dir")
            .arg(store)
            .env("SNAPSHOT_REGTEST_OPERATORS", operators)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("node scripts/snapshot-rendezvous-local.mjs");
        let s = Site(child);
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if latest().await.is_some() {
                return s;
            }
        }
        panic!("the website stand-in did not answer on {SITE_PORT}");
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn base() -> String {
    format!("http://127.0.0.1:{SITE_PORT}")
}

fn latest_url() -> String {
    format!("{}/api/snapshots/latest?chain=regtest", base())
}

/// `GET latest`: (status, body).
async fn latest() -> Option<(u16, Value)> {
    let r = reqwest::get(latest_url()).await.ok()?;
    let status = r.status().as_u16();
    Some((status, r.json().await.ok()?))
}

/// The stand-in serves its files over plain HTTP on loopback; the app's own
/// rule (HTTPS on easybtx.com or the Blob store) stays for real runs.
fn loopback_site(url: &str) -> bool {
    url.starts_with(&format!("http://127.0.0.1:{SITE_PORT}/"))
}

fn validating(pubkey: &str) -> Vec<String> {
    vec![
        "-matmulvalidation=consensus".into(),
        "-matmulattestationsignerkeyfile=signer.wif".into(),
        format!("-matmultrustedpubkey={pubkey}"),
        "-matmultrustedthreshold=1".into(),
        "-matmulattestationserve=0".into(),
        "-allowunverifiablematmulconsensus=1".into(),
    ]
}

async fn call(rpc: &RpcClient, method: &str, params: Value) -> Value {
    rpc.call(method, params)
        .await
        .unwrap_or_else(|e| panic!("{method}: {e}"))
}

async fn height(rpc: &RpcClient) -> u64 {
    call(rpc, "getblockcount", json!([]))
        .await
        .as_u64()
        .unwrap()
}

/// Mine on `rpc` until it has a header at `to`, ten a call. Headers, not
/// blocks: the miner's own block 100 stays unconnected until a restart, and
/// counting blocks would mine sibling after sibling at 100.
async fn mine_to(rpc: &RpcClient, to: u64) {
    for _ in 0..200 {
        let headers = call(rpc, "getblockchaininfo", json!([])).await["headers"]
            .as_u64()
            .unwrap_or(0);
        if headers >= to {
            return;
        }
        let _ = rpc
            .call(
                "generatetodescriptor",
                json!([(to - headers).min(10), "raw(51)"]),
            )
            .await;
    }
    panic!("did not reach {to}");
}

async fn wait_for(rpc: &RpcClient, to: u64) {
    for _ in 0..240 {
        if height(rpc).await >= to {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("the node did not reach {to}");
}

async fn connect(from: &RpcClient, to: &Node) {
    call(from, "addnode", json!([to.addr(), "onetry"])).await;
}

async fn record_diary(rpc: &RpcClient, node: &Node) -> diary::DiaryEntry {
    match diary::record_at_tip(rpc, &node.net(), &node.net(), Some("consensus"))
        .await
        .unwrap()
    {
        DiaryOutcome::Recorded(e) => e,
        other => panic!("{other:?}"),
    }
}

/// `dump` writes the staging pair; the keeper renames it into place after
/// the base matures. Do the same, and return the manifest.
fn put_in_place(node: &Node, height: u64) -> Vec<u8> {
    let dir = node.snapshots();
    std::fs::rename(
        dir.join("staging.dat"),
        dir.join(snapshot_file_name(height)),
    )
    .unwrap();
    std::fs::rename(
        dir.join("staging.manifest"),
        dir.join(manifest_file_name(height)),
    )
    .unwrap();
    std::fs::read(dir.join(manifest_file_name(height))).unwrap()
}

async fn confirm(
    rpc: &RpcClient,
    node: &Node,
    key: &str,
    holds: Holds<'_>,
    env: &str,
) -> Vec<Verdict> {
    let client = site::client().unwrap();
    let work = node.snapshots().join(WORK_DIR);
    let base = base();
    let c = Confirmer {
        rpc,
        client: &client,
        site: &base,
        diary_dir: &node.net(),
        work_dir: &work,
        our_key: btx_core::operators::parse_key(key).unwrap(),
        holds,
        regtest_env: Some(env),
    };
    c.round(Chain::Regtest).await.unwrap()
}

fn verdict_at(v: &[Verdict], height: u64) -> &Verdict {
    v.iter()
        .find(|x| match x {
            Verdict::Signed { height: h, .. }
            | Verdict::AlreadySigned { height: h, .. }
            | Verdict::Refused { height: h, .. }
            | Verdict::Failed { height: h, .. } => *h == height,
        })
        .unwrap_or_else(|| panic!("no verdict at {height}: {v:?}"))
}

fn snapshot_base(chainstates: &Value) -> Option<String> {
    chainstates["chainstates"]
        .as_array()?
        .iter()
        .find_map(|c| c["snapshot_blockhash"].as_str().map(str::to_string))
}

#[tokio::test]
#[ignore]
async fn producers_confirmers_the_website_and_a_mirror_on_regtest() {
    let (Some(btxd), Some(site_dir)) = (
        std::env::var_os("EASYNODE_TEST_BTXD").map(PathBuf::from),
        std::env::var_os("EASYNODE_TEST_SITE_DIR").map(PathBuf::from),
    ) else {
        eprintln!("EASYNODE_TEST_BTXD or EASYNODE_TEST_SITE_DIR unset; nothing to test against");
        return;
    };
    let cli = std::env::var_os("EASYNODE_TEST_BTX_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| btxd.with_file_name("btx-cli"));
    let root = tempfile::tempdir().unwrap();
    let (p_wif, p_pub) = regtest_key();
    let (c_wif, c_pub) = regtest_key();
    let (d_wif, d_pub) = regtest_key();
    let (r_wif, r_pub) = regtest_key();
    let env = format!("producer={p_pub};confirmer1={c_pub};confirmer2={d_pub};rogue={r_pub}");
    let _site = Site::start(&site_dir, &root.path().join("site"), &env).await;
    let client = site::client().unwrap();
    let web = base();
    assert_eq!(latest().await.unwrap().0, 404);

    // ── The shared chain to 100, and every validating node's diary there ──
    let mut p = Node::new(&btxd, root.path().join("p"), 29621, validating(&p_pub));
    let mut c = Node::new(&btxd, root.path().join("c"), 29622, validating(&c_pub));
    let mut d = Node::new(&btxd, root.path().join("d"), 29623, validating(&d_pub));
    for (n, wif) in [(&p, &p_wif), (&c, &c_wif), (&d, &d_wif)] {
        std::fs::write(n.net().join("signer.wif"), format!("{wif}\n")).unwrap();
    }
    let rp = p.start().await;
    mine_to(&rp, 100).await;
    if height(&rp).await < 100 {
        // The miner's own block 100 connects at its next start.
        p.stop(&rp).await;
    }
    let rp = if p.child.is_none() {
        p.start().await
    } else {
        rp
    };
    assert_eq!(height(&rp).await, 100);
    let rc = c.start().await;
    let rd = d.start().await;
    connect(&rc, &p).await;
    connect(&rd, &p).await;
    wait_for(&rc, 100).await;
    wait_for(&rd, 100).await;
    let at_100 = record_diary(&rp, &p).await;
    assert_eq!(record_diary(&rc, &c).await.block_hash, at_100.block_hash);
    assert_eq!(record_diary(&rd, &d).await.block_hash, at_100.block_hash);

    // ── The producer exports at 100, matures to 110, checks its own export ──
    let dump = serve::dump(&rp, &p.snapshots()).await.unwrap();
    assert_eq!(dump.base_height, 100);
    mine_to(&rp, 110).await;
    wait_for(&rc, 110).await;
    wait_for(&rd, 110).await;
    let manifest = put_in_place(&p, 100);
    producer::check_before_send(&rp, &manifest, &p.net(), &Holds::none(), Some(&env))
        .await
        .expect("the producer's own export matches its diary");
    let st = cs::parse(&manifest).unwrap().statement;
    let hash = st.hash().display_hex();
    assert_eq!(st.block_hash().display_hex(), at_100.block_hash);
    assert_eq!(
        (st.coins(), st.chain_tx()),
        (at_100.coins, at_100.chain_tx),
        "the diary and the export agree on coins and transactions"
    );

    // ── Refused: a file whose hash is not the statement's ──────────────────
    let first = site::post_statement(&client, &web, &manifest)
        .await
        .unwrap();
    assert_eq!(
        (first.operators.clone(), first.file.as_str()),
        (vec!["producer".to_string()], "missing")
    );
    let tampered = root.path().join("tampered.dat");
    let mut bytes = std::fs::read(p.snapshots().join(snapshot_file_name(100))).unwrap();
    bytes[1000] ^= 1;
    std::fs::write(&tampered, &bytes).unwrap();
    assert_eq!(
        site::upload_file(&client, &web, &hash, &tampered).await,
        Err(SiteError::Refused {
            status: 422,
            reason: "the file does not match the statement".into()
        })
    );

    // ── Refused: a statement one operator signed ───────────────────────────
    let sent = producer::submit(&client, &web, &p.snapshots(), 100)
        .await
        .unwrap();
    assert_eq!(sent.statement_hash, hash);
    assert_eq!(
        latest().await.unwrap().0,
        404,
        "one operator, file stored: latest names nothing"
    );

    // ── Refused: a confirmer whose diary disagrees ─────────────────────────
    let d_diary = std::fs::read(diary::diary_path(&d.net())).unwrap();
    let mut wrong = diary::load(&d.net(), btx_core::operators::REGTEST_GENESIS);
    let mut e = wrong.at(100).unwrap().clone();
    e.coins += 1;
    wrong.record(e);
    diary::save(&d.net(), &wrong).unwrap();
    let v = confirm(&rd, &d, &d_pub, Holds::none(), &env).await;
    assert!(
        matches!(
            verdict_at(&v, 100),
            Verdict::Refused {
                why: Mismatch::Coins { .. },
                ..
            }
        ),
        "{v:?}"
    );
    std::fs::write(diary::diary_path(&d.net()), d_diary).unwrap();
    assert_eq!(latest().await.unwrap().0, 404);

    // ── Two confirmers co-sign; latest points at it ────────────────────────
    let v = confirm(&rc, &c, &c_pub, Holds::none(), &env).await;
    assert!(
        matches!(verdict_at(&v, 100), Verdict::Signed { .. }),
        "{v:?}"
    );
    let (status, pointer) = latest().await.unwrap();
    assert_eq!(status, 200, "{pointer}");
    assert_eq!(pointer["height"], json!(100));
    assert_eq!(pointer["operators"], json!(["producer", "confirmer1"]));
    let v = confirm(&rd, &d, &d_pub, Holds::none(), &env).await;
    assert!(
        matches!(verdict_at(&v, 100), Verdict::Signed { .. }),
        "{v:?}"
    );
    assert_eq!(
        latest().await.unwrap().1["operators"],
        json!(["producer", "confirmer1", "confirmer2"])
    );

    // ── Works: a mirror pinning only P loads it through the app's loader ──
    let mut m = Node::new(
        &btxd,
        root.path().join("m"),
        29624,
        vec![
            "-matmulvalidation=trusted".into(),
            "-connect=0".into(),
            format!("-matmultrustedpubkey={p_pub}"),
            "-matmultrustedthreshold=1".into(),
        ],
    );
    let rm = m.start().await;
    connect(&rm, &p).await;
    for _ in 0..120 {
        if call(&rm, "getblockchaininfo", json!([])).await["headers"].as_u64() >= Some(100) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let view = confirmed_load::node_view(&rm, &[p_pub.as_str()], 0).await;
    let pair = attested_snapshot::prepare_confirmed(
        &client,
        &latest_url(),
        &m.net(),
        &view,
        Some(&env),
        loopback_site,
    )
    .await
    .expect("the confirmed pair downloads and checks out");
    assert_eq!(pair.height, 100);
    let runner = CliRunner {
        btx_cli: cli.clone(),
        args: vec![
            "-regtest".into(),
            format!("-datadir={}", m.dir.display()),
            format!("-rpcport={}", m.rpc_port),
        ],
    };
    let loaded = confirmed_load::load(&rm, &runner, &pair, &view, &Holds::none(), Some(&env))
        .await
        .expect("the mirror loads the confirmed snapshot");
    assert_eq!((loaded.height, loaded.signatures), (100, 1));
    assert_eq!(
        snapshot_base(&call(&rm, "getchainstates", json!([])).await),
        Some(at_100.block_hash.clone())
    );
    m.stop(&rm).await;

    // ── Refused: a statement on a held branch ──────────────────────────────
    // R, the rogue, takes the chain at 110 from P, then builds its own branch
    // with D following it. C stays with P.
    let mut r = Node::new(&btxd, root.path().join("r"), 29625, validating(&r_pub));
    std::fs::write(r.net().join("signer.wif"), format!("{r_wif}\n")).unwrap();
    let rr = r.start().await;
    connect(&rr, &p).await;
    wait_for(&rr, 110).await;
    let _ = rr.call("disconnectnode", json!([p.addr()])).await;
    let _ = rd.call("disconnectnode", json!([p.addr()])).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    connect(&rd, &r).await;
    mine_to(&rr, 200).await;
    wait_for(&rd, 200).await;
    let held_root = call(&rr, "getblockhash", json!([111])).await;
    let held_root: &'static str =
        Box::leak(held_root.as_str().unwrap().to_string().into_boxed_str());
    let d_at_200 = record_diary(&rd, &d).await;
    let rogue_dump = serve::dump(&rr, &r.snapshots()).await.unwrap();
    assert_eq!(rogue_dump.base_height, 200);
    assert_eq!(rogue_dump.base_hash, d_at_200.block_hash, "D follows R");
    mine_to(&rr, 210).await;
    wait_for(&rd, 210).await;
    let rogue_manifest = put_in_place(&r, 200);
    // The rogue sends without the checks an honest producer runs.
    producer::submit(&client, &web, &r.snapshots(), 200)
        .await
        .unwrap();
    let holds_list: &'static [HeldBranch] = Box::leak(Box::new([HeldBranch {
        height: 111,
        root: held_root,
        why: "the rehearsal's held branch",
    }]));
    let holds = Holds {
        invalid: &[],
        held: holds_list,
    };
    // The confirmer ON the held branch: its diary matches, 10 confirmations,
    // and still it refuses, because a refused block is under the base.
    let v = confirm(&rd, &d, &d_pub, holds, &env).await;
    assert_eq!(
        verdict_at(&v, 200),
        &Verdict::Refused {
            height: 200,
            statement_hash: cs::parse(&rogue_manifest)
                .unwrap()
                .statement
                .hash()
                .display_hex(),
            why: Mismatch::HeldRootOnActiveChain {
                height: 111,
                root: held_root.into()
            }
        }
    );
    assert!(matches!(verdict_at(&v, 100), Verdict::AlreadySigned { .. }));
    // The confirmer off it: the base is not on its chain at all.
    let v = confirm(&rc, &c, &c_pub, holds, &env).await;
    assert!(
        matches!(
            verdict_at(&v, 200),
            Verdict::Refused {
                why: Mismatch::BaseNotOnActiveChain { height: 200 },
                ..
            }
        ),
        "{v:?}"
    );
    // An honest producer on that branch, knowing the hold, does not send it.
    let honest =
        producer::check_before_send(&rr, &rogue_manifest, &r.net(), &holds, Some(&env)).await;
    assert!(honest.is_err(), "{honest:?}");
    // latest still names 100; the held branch has one operator.
    let (status, pointer) = latest().await.unwrap();
    assert_eq!((status, pointer["height"].clone()), (200, json!(100)));
    let pending = site::get_pending(&client, &web, "regtest").await.unwrap();
    let h200 = pending.statements.iter().find(|s| s.height == 200).unwrap();
    assert_eq!(h200.operators, vec!["rogue".to_string()]);

    r.stop(&rr).await;
    d.stop(&rd).await;
    c.stop(&rc).await;
    p.stop(&rp).await;
}
````

- [ ] **Step 2: Run it without an engine (it must skip, not fail)**

Run: `cd crates/btx-core && cargo test --locked --test snapshot_network_regtest -- --ignored --nocapture`
Expected: `1 passed` and `EASYNODE_TEST_BTXD or EASYNODE_TEST_SITE_DIR unset; nothing to test against`.

- [ ] **Step 3: Prepare the website stand-in**

The website plan's Tasks 1 to 6 must be done on `claude/snapshot-rendezvous`. Use its worktree, or make one for this run:

````bash
git -C /Users/m2promende/repos/EasyBTX fetch -q origin claude/snapshot-rendezvous
git -C /Users/m2promende/repos/EasyBTX worktree add --detach /private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad/easybtx-rehearsal origin/claude/snapshot-rendezvous
(cd /private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad/easybtx-rehearsal/site && PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm ci --no-audit --no-fund)
````

Expected: `added … packages`. Never use `/Users/m2promende/repos/EasyBTX` itself (the owner's working tree).

- [ ] **Step 4: Run it against the shipped engine**

The v0.34.9 btxd and btx-cli used for this plan are at `/private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad/spike/bin/`; otherwise use the staged package's (`apps/node/src-tauri/resources/node-pkg/.../btxd`, after `apps/node/scripts/stage-node-pkg.sh`). RPC ports 29621 to 29625, P2P ports 29721 to 29725 and port 29650 must be free; Node 22 must be on the PATH.

````bash
cd crates/btx-core
EASYNODE_TEST_BTXD=/path/to/btxd \
EASYNODE_TEST_SITE_DIR=/private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad/easybtx-rehearsal/site \
  cargo test --locked --test snapshot_network_regtest -- --ignored --nocapture --test-threads=1
````

Expected: `test producers_confirmers_the_website_and_a_mirror_on_regtest ... ok` and `test result: ok. 1 passed` in about 65 s. No btxd or node process of the test is left running (`pgrep -fl 'btxd -regtest'` and `pgrep -fl snapshot-rendezvous-local` show none from this run). If it stops at "did not reach 100", the miner's block 100 did not connect after the restart: read the tail of `p/regtest/debug.log` in the temp folder the panic names.

- [ ] **Step 5: Format, lint, commit, and clean up**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cd ../..
git add crates/btx-core/tests/snapshot_network_regtest.rs
git commit -m "core: rehearse the snapshot network on regtest: producer, two confirmers, a mirror and the website's own code" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git -C /Users/m2promende/repos/EasyBTX worktree remove --force /private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad/easybtx-rehearsal
````

## Risks and open points (for the owner)

1. **Nothing is confirmed while Mende is alone on the list.** Everything here runs, but a statement from the 3060 stays at one operator, `latest` stays 404, and every node uses the fallbacks. The first confirmation needs Aleksander or jpp on both lists, with a validating, signing node each, whose diaries cover the same heights.
2. **A confirmer's diary must have the height.** The refresher reads the tip every 3 seconds and blocks come about every 90, so a grid height is usually caught, but a node that was down or catching up at that moment has no entry and refuses that statement (`NoDiaryEntry`). With two or three confirmers, some heights may never reach two operators; the next one is 200 blocks later. The keeper also records right before it exports, so the producer itself never misses its own height.
3. **The keeper's poll.** An export happens only while the tip is exactly on the grid; the keeper looks every 3 s when the tip is 1 or 2 blocks short and every 30 s otherwise. A block that arrives within those 3 s moves the tip on and that height is skipped.
4. **The producer's checks now gate the P2P offer as well**, not only the upload: a keeper whose diary is off (a mirror, or a node on a signed snapshot still being checked) stops offering new pairs. That is the design's "a producer never sends a pair that failed any check" applied to both channels; the 0.6.x mirrors that fetch over P2P lose nothing while the 3060 validates.
5. **An orphaned base is abandoned** until the next grid height (up to about 5 hours), where the keeper used to dump again at once; a second dump would base on the tip, off the grid.
6. **The rehearsal needs `-allowunverifiablematmulconsensus=1`** for the validating regtest nodes to connect blocks above 100 on a device the engine does not qualify (measured on the owner's M2 Pro). On a qualified card the flag is harmless.
7. **The confirmer trusts the engine's `getmatmultrustedstatus.replay_authority_context`** for the node side; a signing node always reports one (a node that does not is refused, `ReplayContext { node: None }`).
8. **The design's upload path does not fit Vercel as written** (website plan, Risks 1): parts of at most 4 MiB go up as the website's own private blobs, not as Blob multipart parts, which must be at least 5 MB.

## Self-review

- Spec coverage: section 2, the grid of 200 (`EXPORT_GRID`, `export_due`, `grid_for`, Task 5; regtest's 100 in Tasks 2 and 8); section 3, the diary on every validating node, only at grid heights, only when `height` and `bestblock` are that block, newest 100, atomic, nothing while `attested_assumeutxo` exists (Task 2; the refresher, Task 7); section 4, producers export exactly on the grid, wait 10 confirmations, re-check canonical, held roots refused, diary field by field, send only when listed, never send a failed pair, keep offering over P2P (Tasks 5 and 7); section 5, confirmers every ten minutes, diary match per field, chain id and replay context against the node, 10 confirmations on the active chain, `signutxosnapshotmanifest` on a copy, one new signature back, never without a diary match, mismatch logged and counted in Copy diagnostics (Tasks 6 and 7); section 6's app side, the client (Task 4). The website is its own plan.
- Sabotage tests, one per rule: diary mismatch per field, never signed (`a_diary_that_differs_in_any_one_field_is_refused`, `a_diary_that_disagrees_is_never_signed`, rehearsal step 4); chain id and replay context (`a_node_on_another_chain_or_replay_context_is_refused`); a base off the active chain or under 10 confirmations (`a_base_off_the_active_chain_or_under_ten_confirmations_is_refused`, rehearsal step 7 for C); off-grid (`an_off_grid_statement_is_refused_before_the_node_is_asked`, `an_export_that_landed_off_the_grid_is_not_matured`, `an_export_is_due_only_on_the_grid_and_above_the_offered_base`); a held branch (`a_refused_block_under_the_base_is_refused`, `a_held_block_is_refused_first_and_a_base_above_it_is_not_sent`, rehearsal step 7); a file whose hash does not match, discarded (rehearsal step 2; the website plan's routes test; the core plan's loader tests); `latest` never pointing at a one-operator statement (rehearsal steps 3 and 7; the website plan's rules and routes tests); a statement from an unlisted key and a merged co-signature from an unlisted key (the website plan); a producer that fails its checks offers and sends nothing (`a_pair_the_checks_refuse_is_not_offered_and_the_old_offer_stays`, `a_producer_whose_diary_disagrees_sends_nothing`); an engine that signs with the wrong key (`an_engine_that_signs_with_another_key_sends_nothing`); the published list drifting from the compiled one (`the_published_list_is_the_compiled_one`).
- Placeholders: none. Every code step is the code as it passed on 2026-09-29, applied on top of a copy of `cb0316b` with the core plan's Tasks 1 to 4 and its `attested_snapshot_record` in place: btx-core 689 library tests, the Tauri crate 108, the frontend gates, fmt and clippy (correctness, suspicious) clean, and the rehearsal against v0.34.9 and the website's handlers passing. The only value an engineer supplies is the path to btxd, as in the core plan.
- Consistency: names checked across tasks and against the core plan (`Holds`, `ChainRules`, `check_shape`, `Mismatch`, `Verdict`, `NetworkReport`, `ProducerChecks`, `BeforeOffer`, `EXPORT_GRID`, `record_at_tip`, `check_before_send`, `submit`) and against the website plan (`x-ebtx-node: ebtx-snapshot-v1`, the reply fields, `part_bytes`, `parts`, `manifest_hex`).
