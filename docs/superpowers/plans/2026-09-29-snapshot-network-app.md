# Snapshot Network, app side: diary, producers and confirmers (Implementation Plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Amended 2026-09-29 (night)**, after jpp's review and the owner's decisions of that night, against the amended design (`docs/decisions/2026-09-29-every-node-starts-near-the-tip.md` as committed in `12e44c3` on `claude/cosigned-snapshots`) and the contracts the three amended plans share. What changed: the grid is 100 and the depth 144 everywhere; the validated gate (`getchainstates`, no chainstate with `"validated": false`) runs before every diary entry, export, send and signature, and replaces the `attested_assumeutxo` file test; diary entries are dropped when their block leaves the active chain; `statement_check.rs` holds the checks and the one pure verdict, Sign, Dissent or Neither, that the app and `btx-confirmer` both call; a confirmer whose diary disagrees on a chain field sends a dissent built from its diary and signed by its node; the keeper waits per exported height (six-hour deadline, four pairs on disk); the website client reads the disputed `latest`, the dissent flag in `pending` and the closed-height refusal; Copy diagnostics counts mismatches, dissents sent and gate-closed skips; `btx-confirmer` and its unit are new (Tasks 7 and 8); the rehearsal adds `btx-confirmer`, a real disagreement and an unvalidated snapshot chainstate. The old Task 1 (publish the operator list for the website) is folded away: the list is now the core plan's `crates/btx-core/snapshot-operators.json`, which the website copies byte for byte, and this plan writes it nowhere else. Tasks are renumbered: old Tasks 2 to 7 are Tasks 1 to 6, old Task 8 is Task 9. Rebased on `origin/main` after the Tools merge (`aed8755`; `origin/main` is `b330e3d` on 29 September); every file anchor below was re-read there.

> **Amended 2026-09-30, after the site code and the core crate were checked against this plan.** Two corrections, no design change: the route list now also carries the site's `409` on the file upload's `PUT` and `complete`, exactly as on `start`, when the file is already stored or the upload was let go, and its `400` on any query to `latest`, `pending` or `disputes` other than empty or exactly `?chain=main` / `?chain=regtest`; `upload_file` maps a `409` on `PUT` and `complete` to `Ok(None)` the same as it already does on `start`. And `confirmed_snapshot::dissent_statement` now returns `Option<[u8; STATEMENT_LEN]>` (`None` when the height does not fit the statement's 32-bit field), so `dissent_from` maps `None` to `Mismatch::Unreadable`.

**Goal:** Every validating node writes down its own chain state at every multiple of 100, but only while its chain rests on its own checks; a producer on the operator list exports exactly there, waits until that block is 144 deep, checks its export against its node and its diary, and sends it to easybtx.com; a confirmer on the list, in the app or beside a plain btxd (`btx-confirmer`), co-signs a waiting statement only when its own diary, chain and holds agree field by field, and sends a signed dissent when its diary disagrees on a chain field; an end-to-end rehearsal on regtest proves the refusals and the happy path against real engines and the website's own route code.

**Architecture:** `diary.rs` records `gettxoutsetinfo` and `getchaintxstats` at grid heights into `<datadir>/snapshot-diary.json`, asks the validated gate (`diary::gate_open`, over the core plan's pure `getchainstates` check) right before every entry, and drops entries whose block left the active chain. `statement_check.rs` reads what the node says (`NodeFacts`) and gives the one pure verdict: `Sign`, `Dissent` (a chain field differs from the diary) or `Neither` (the gate is closed, not 144 deep yet, no diary entry on this chain, a foreign chain, replay context or shielded commitment, a held root below the base); it also builds a dissent from a diary entry. `snapshot_site.rs` is the client of the website's routes. `snapshot_serve.rs` exports only on the grid into a waiting set (`waiting.json`), and every keeper tick looks at each waiting pair once (`mature_step`): 144 deep, re-verified, passed by a `BeforeOffer` hook, then offered; `snapshot_producer.rs` is that hook plus the submission. `snapshot_confirmer.rs` reads `pending`, never signs a dissent, and signs a copy or a dissent with the node's own `signutxosnapshotmanifest` under `<datadir>/snapshot-confirmer/`. `src/bin/btx-confirmer.rs` runs the same diary and confirmer beside a plain btxd, under `deploy/esplora/btx-confirmer.service.template`. The Tauri shell drives the diary and the confirmer from the status refresher and the producer from the snapshot keeper, and Copy diagnostics gains a "Snapshots" section. `tests/snapshot_network_regtest.rs` runs five regtest engines, `btx-confirmer` and the website's handlers (`site/scripts/snapshot-rendezvous-local.mjs`) together.

**Tech Stack:** Rust (btx-core, its `btx-confirmer` binary, Tauri 2 shell), k256 0.13.4 (`ecdsa`), reqwest, serde_json, tokio, mockito (tests); btxd v0.34.9 (commit `84b998b4`) on regtest and Node 22 for the rehearsal; systemd for the confirmer's unit.

**This plan builds on the confirmed-snapshots plans** (`2026-09-29-confirmed-snapshots-core.md` and `2026-09-29-confirmed-snapshots-fast-forward.md`, same folder) and uses their interfaces as they define them: `operators::{Chain, OperatorList, MAINNET_GENESIS, REGTEST_GENESIS, for_chain, regtest_env, parse_key, hex, hex_decode}` (the list compiled from `crates/btx-core/snapshot-operators.json`), `confirmed_snapshot::{parse, Manifest, Signed, Statement, Hash32, ChainRules, Refusal, confirming_operators, signature_is_valid, check_with, MAINNET_REPLAY_CONTEXT, REGTEST_REPLAY_CONTEXT}`, `confirmed_load::{Holds, node_view, load, CliRunner, trimmed_manifest_path}`, `attested_snapshot::{prepare_confirmed, ReadyPair}`, `node::{attested_snapshot_record, validating_snapshot_pin_args}`, `node_api::MatmulTrustedStatus::replay_authority_context`. From the amended core plan (read as it stood the night of 29 September) it also uses `confirmed_snapshot::{SNAPSHOT_GRID, SNAPSHOT_DEPTH, check_shape, is_dissent, dissent_statement}`, `ChainRules::shielded`, `node_api::read_chainstates_validated` (the validated gate), `attested_snapshot::{Latest, parse_latest}` and `confirmed_load::load`'s new `datadir` argument; "Names this plan uses from the amended core plan" at the end lists each with its one call site here. The website half is `2026-09-29-snapshot-network-site.md`; its handlers answer this plan's client, and its stand-in serves the rehearsal.

## Global Constraints

- Design: `docs/decisions/2026-09-29-every-node-starts-near-the-tip.md` as amended the night of 29 September (`git -C /Users/m2promende/repos/easynode show claude/cosigned-snapshots:docs/decisions/2026-09-29-every-node-starts-near-the-tip.md`, commit `12e44c3` or later; use `origin/claude/cosigned-snapshots` once it is pushed). Sections implemented here: 2 (grid of 100), 3 (the diary and the validated gate), 4 (producers), 5 (confirmers, sign or dissent), 5a (`btx-confirmer`), and the app's side of 6 and 6a (the website's client: `pending` with its dissent flag, `latest` with its disputed answer, the closed-height refusal, posting a dissent). The routes, the dispute rule and the owner's clear are the website plan.
- Branch: `claude/snapshot-network`, created from `claude/confirmed-snapshots` after both confirmed-snapshots plans are done. The Tools work those plans used to stack on is merged: `origin/main` has it since `aed8755` (`b330e3d` on 29 September), so `claude/confirmed-snapshots` is based on `origin/main`. Worktree: `git -C /Users/m2promende/repos/easynode worktree add ../easynode-snapshot-network -b claude/snapshot-network claude/confirmed-snapshots`. All paths below are relative to that worktree. Where a confirmed-snapshots plan changed a line this plan anchors on, apply this plan's change to that plan's text.
- Grid: **100** on every chain (`confirmed_snapshot::SNAPSHOT_GRID`, and `ChainRules::grid` wherever rules are at hand). 219,000 and 228,000 are on it. A height the tip passed before the app read it is skipped; the next chance is 100 blocks later.
- Depth: **144** (`confirmed_snapshot::SNAPSHOT_DEPTH`), counted as `getblockheader`'s `confirmations` counts it on the node's own active chain (tip minus height plus one). A producer offers and sends, and a confirmer signs or dissents, only at that depth. `snapshot_serve::CONFIRMATIONS_REQUIRED` is this constant, not a second one.
- The validated gate (jpp's point 4): `diary::gate_open(rpc)` is the core plan's `node_api::read_chainstates_validated`, which reads `getchainstates` and applies the pure `chainstates_validated`: open only when the answer parsed and every entry in `chainstates` says `"validated": true`, so no entry has `"validated": false`; a failed or unparsable answer, an empty list or an entry without the field is closed (the core plan's reading, its conflict 2). It runs right before every diary entry (`diary::record_at_tip`), every export (the keeper), every send (`snapshot_producer::check_before_send`) and every signature or dissent (`statement_check::check_against_node`, and at the top of every confirmer round). It replaces every use of `node::attested_snapshot_record` for these purposes; section 8's pin rule still uses that file (core plan). The engine writes `"validated": false` for a chainstate built from a snapshot while the background chainstate exists (`src/rpc/blockchain.cpp:5211-5212` at `84b998b4`), for upstream's plain assumeutxo snapshot and a signed one alike; only the signed load writes `attested_assumeutxo`.
- Diary: `<datadir>/snapshot-diary.json` (`btx-confirmer`: `<state>/snapshot-diary.json`), JSON `{"version":1,"chain_id":"<genesis display hex>","entries":[{"height","block_hash","hash_serialized","coins","chain_tx","recorded_at"}]}`, ascending, newest **100** heights (10,000 blocks, about ten days at 40 an hour), written with `fsx::atomic_write`. Recorded only when `getmatmultrustedstatus.matmul_validation_mode` is `consensus` and the gate is open. The fields come from `gettxoutsetinfo` (`height`, `bestblock`, `txouts`, `hash_serialized_3`), kept only when `getblockhash(height) == bestblock` and `height` is on the grid, and from `getchaintxstats 1 <bestblock>` (`txcount`). Measured on regtest with v0.34.9 on 2026-09-29: at 100 these equal the statement `dumptxoutsetattested` writes (block, UTXO hash, coins 101, transaction count 101). An entry whose block is no longer the block at its height on the active chain is dropped at the next write and never compared (section 3).
- The verdict, `statement_check::verdict` (pure, over the `NodeFacts` that `check_against_node` reads), in this order: gate closed, **Neither** (`Unvalidated`); the statement's shape under its chain's rules (the core plan's `confirmed_snapshot::check_shape`: version, chain id, replay context and shielded commitment as compiled, not a dissent, chunk geometry, file size, grid), **Neither** (`Unreadable`); the node's genesis and replay context are the statement's, else **Neither** (`ChainId`, `ReplayContext`); fewer than 144 deep, **Neither** (`TooShallow`); a known-invalid block or held root on the active chain at or below the height, **Neither** (`HeldRootOnActiveChain`); no diary entry at the height, **Neither** (`NoDiaryEntry`); the entry's block is not the node's block at that height, **Neither** (`DiaryEntryLeftChain`); block hash, UTXO hash, coin count or chain transaction count differ from the entry, **Dissent**; otherwise **Sign**. Read-only.
- Producer: a node with "Serve a chain snapshot" on and a key, validating (as today); exports only when its tip is exactly on the grid, above the offered base, not already waiting, and the gate is open, the diary first; the export goes straight into the waiting set (`<datadir>/snapshots/waiting.json`). Every keeper tick looks at every waiting pair once: at 144 deep it is re-verified on the active chain and passed through `snapshot_producer::check_before_send` (the gate, every known-invalid block and held root refused on this node with `invalidateblock` through `known_invalid`, then the verdict, which must be `Sign`), then offered on P2P. `MATURE_DEADLINE` is **6 hours** (144 blocks take about three and a half at 40 an hour), `KEEP_PAIRS` **4** (the offered one, the one before it and two waiting, about 36 MB). A base that leaves the chain while it waits is dropped; the next export is at the next grid height. Only a producer whose key is on the list (`operators::for_chain(..).operator_of`) sends to easybtx.com; a pair that failed any check is neither offered nor sent. The submission is recorded in `<datadir>/snapshots/submitted.json` (`{"height","statement_hash","at"}`) and retried every 10 minutes until it is; a height the website has closed is recorded the same way and not retried.
- Confirmer: a node that validates, signs (`local_signer`), and whose key is on the list; every **600 s** (200 refresher ticks of 3 s, the first about 2 minutes after the node starts; `btx-confirmer`: the first a minute in) it asks the gate, and while it is closed does nothing more (`Round::Unvalidated`). Otherwise it reads `GET /api/snapshots/pending?chain=main`, skips every entry marked as a dissent (by the website's flag or by its own zero file fields: a dissent is never signed), skips statements its operator already signed, and takes each through the verdict. **Sign**: the engine signs a copy, the app checks the one new signature and posts a manifest carrying the statement and only that signature. **Dissent**: the app builds a dissent from its own diary entry (`statement_check::dissent_from`, the core plan's `dissent_statement`: that entry's height, block hash, UTXO hash, coin count and transaction count, the compiled chain id, replay context and shielded commitment, all four file fields zero), the engine signs it, and it is posted like any manifest, at most once per height (`<state>/snapshot-dissents.json`; a dissent this operator already has in `pending` counts as sent). **Neither**: nothing is signed or sent. It never signs without a diary match. Every mismatch is logged once per statement per run and counted in Copy diagnostics, with dissents sent and gate-closed skips.
- The copy the engine signs: `<datadir>/snapshot-confirmer/<statement hash>.manifest`, handed to `signutxosnapshotmanifest` as the path relative to the node's data folder (`snapshot-confirmer/<statement hash>.manifest`), which the engine resolves against its own network data folder (`AbsPathForConfigVal(args, path)`, `src/rpc/blockchain.cpp:4631-4632`, net-specific by default, `src/common/config.cpp:252-258` at `84b998b4`). The engine refuses without a configured local signer (`:4624-4629`), checks only chain id, replay context and its blocklist before signing (`src/matmul/trusted_exact_replay_attestation.cpp:1387-1403`), and reads a manifest with no signatures, so a dissent can be signed this way. The copy is removed after the call.
- `btx-confirmer` (section 5a): `crates/btx-core/src/bin/btx-confirmer.rs`, beside `btx-witness`, built and installed the same way (`cargo build --release --bin btx-confirmer`, then `/usr/local/bin`). Flags `--datadir <path>` (the node's data folder, holding its `.cookie`), `--state <path>` (its diary and dissents), `--rpc <addr:port>` (default `127.0.0.1:19334`), and `--once` (one diary look and one round, then exit, for a first check and for the rehearsal). At start it refuses to run unless the node is on a chain easyNode knows, reports the replay context compiled for it, checks blocks itself, and has a signing key. It reads the diary every 5 s and runs a round every 600 s, through the same `diary`, `statement_check`, `snapshot_site` and `snapshot_confirmer` code as the app. It learns its node's key from the engine's first signature (no RPC names the local signer's key, and it does not read the key file). It listens on nothing, exports, loads, pins and restarts nothing. Unit: `deploy/esplora/btx-confirmer.service.template`, in the shape of `btx-witness.service.template`, with `ReadWritePaths=` the state folder and `<datadir>/snapshot-confirmer/` only.
- The website (routes, replies and limits exactly as the website plan's Global Constraints): `https://easybtx.com`, overridable with `EASYNODE_SNAPSHOT_SITE` only to another `https://` origin or to `http://127.0.0.1:<port>` / `http://localhost:<port>` (no path, no user info); header `x-ebtx-node: ebtx-snapshot-v1` on every write; every body `content-type: application/octet-stream`; parts of at most 4,194,304 bytes; client timeout 120 s. `GET /api/snapshots/latest` answers the pointer, or HTTP 200 with exactly `{"disputed": [<height>, ...]}` while a dispute stands, or HTTP 404 `{"version":1,"confirmed":null}` with nothing confirmed; `GET /api/snapshots/latest`, `/pending` and `/disputes` all answer `400` for any query other than empty or exactly `?chain=main` / `?chain=regtest`; `GET /api/snapshots/pending` entries carry `"dissent": true|false`; `POST /api/snapshots/statement` at a height the owner cleared is refused with exactly `422 {"error":"closed-height"}`; the file upload's `PUT` (a part) and its `complete` answer `409` exactly as `start` does, when the file is already stored or the upload was let go. The app does not read `GET /api/snapshots/disputes`. Nothing the website says is trusted: loading still goes through `attested_snapshot::prepare_confirmed` and `confirmed_load::load`, which check everything again.
- Operator list: the core plan's `crates/btx-core/snapshot-operators.json`, compiled by `operators.rs` (Mende; Aleksander with three keys; jpp; and the mirrors' pins). It is the only place the list is written; this plan adds no second writer, and the website needs nothing from this repository beyond that file, which it copies byte for byte. Test chains: `EASYNODE_REGTEST_OPERATORS` (`name=key[,key];name=key`), only for regtest statements.
- CI gates, all must pass before each commit that touches them: in `crates/btx-core` and `apps/node/src-tauri`: `cargo fmt --all --check`, `cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious`, `cargo test --locked` (in `crates/btx-core` this also runs `btx-confirmer`'s own tests); in `apps/node`: `npx tsc --noEmit`, `npm test`, `npx vite build`.
- The Tauri crate needs `apps/node/src-tauri/resources/node-pkg/` to hold one file for `cargo check`/`test` (CI writes `CI-PLACEHOLDER`): `mkdir -p apps/node/src-tauri/resources/node-pkg && echo placeholder > apps/node/src-tauri/resources/node-pkg/CI-PLACEHOLDER` if it is empty (the folder is gitignored).
- Known flake, not caused by this plan: `node::tests::launch_watch_detects_an_immediate_child_death` fails now and then on macOS when the suite runs in parallel. Rerun it alone: `cargo test --locked --lib -- --exact node::tests::launch_watch_detects_an_immediate_child_death`.
- Commits end with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (`git commit -m "<subject>" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"`).
- User-facing copy, and every line the node log, Copy diagnostics and `btx-confirmer` print: friendly, simple, no hype, no guarantees, no em-dashes.
- Shared test vectors: the fixtures the core plan adds (`crates/btx-core/tests/fixtures/confirmed_snapshot/`). The regtest statement: height 100, block `bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46`, UTXO hash `e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527`, coins 101, transaction count 101, statement hash `11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194`; the mainnet one: 225,927, block `06780445…b932`, statement hash `d3ee9312…0482`, file hash `f234192d…eb2c`. The mainnet shielded commitment compiled per engine: `94343b766b39c0ea2d92d83323f77b5ccc5e775d99b34b01f5fa6400f2354541` (`src/kernel/chainparams.cpp:1278` at `84b998b4`). The `getchainstates` answers the unit tests use are written in the shape v0.34.9 writes (`src/rpc/blockchain.cpp:5174-5214` at `84b998b4`); the rehearsal reads a real one.
- Test counts marked "derived, not run" were counted from the code in this plan, not from a run. The amended plan has not been dry-run; the original was (its numbers are gone where the code changed).

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `crates/btx-core/src/fake_node.rs` | create | Test-only scripted node (chain, UTXO answers, `getchainstates` in the engine's shape, `invalidateblock`, a real `signutxosnapshotmanifest` with relative paths) |
| `crates/btx-core/src/diary.rs` | create | The diary: record at grid heights behind the validated gate, drop entries off the chain, load, save; `gate_open` |
| `crates/btx-core/src/statement_check.rs` | create | What the node says (`NodeFacts`), the pure verdict Sign / Dissent / Neither, `Mismatch`, `dissent_from` |
| `crates/btx-core/src/snapshot_site.rs` | create | Client of `/api/snapshots/*`: statement, file, `pending` (dissent flag), `latest` (disputed), closed heights |
| `crates/btx-core/src/snapshot_serve.rs` | modify | Export on the grid into a waiting set, `mature_step` per waiting pair, `BeforeOffer` hook, 144 deep, six hours, four pairs |
| `crates/btx-core/src/snapshot_producer.rs` | create | The producer's checks, the submission and its record |
| `crates/btx-core/src/snapshot_confirmer.rs` | create | Read pending, sign or dissent through the verdict, send one signature; `check_node` and `diary_tick` for `btx-confirmer`; tally |
| `crates/btx-core/src/bin/btx-confirmer.rs` | create | The confirmer beside a plain btxd: arguments and the loop |
| `crates/btx-core/src/diagnostics.rs` | modify | A "Snapshots" section in Copy diagnostics |
| `crates/btx-core/src/lib.rs` | modify | Register the modules |
| `apps/node/src-tauri/src/state.rs` | modify | `snapshot_network` report on `AppState`; the settings comment |
| `apps/node/src-tauri/src/commands.rs` | modify | Refresher: diary and confirmer; keeper: grid, gate, waiting pairs, checks, submission; copy |
| `apps/node/src-tauri/src/tools.rs` | modify | Feed the "Snapshots" section |
| `apps/node/index.html`, `apps/node/src/main.ts` | modify | "refreshed every 500 blocks" becomes "taken every 100 blocks" |
| `deploy/esplora/btx-confirmer.service.template` | create | `btx-confirmer` as a systemd unit |
| `deploy/esplora/README.md` | modify | A short section on the confirmer |
| `crates/btx-core/tests/snapshot_network_regtest.rs` | create | The end-to-end rehearsal (opt-in) |
| `apps/node/CHANGELOG.md` | modify | One entry |

## Tasks

### Task 1: The diary and the validated gate (`diary.rs`), and a scripted node for the tests (`fake_node.rs`)

(Was Task 2. The old Task 1, which wrote `operators::published_json()` and a second copy of the list, is folded away: the core plan's `snapshot-operators.json` is the list and the file the website copies.)

**Files:**
- Create: `crates/btx-core/src/fake_node.rs` (test-only)
- Create: `crates/btx-core/src/diary.rs`
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: `confirmed_snapshot::{SNAPSHOT_GRID, parse, Signed}` (core plan, Task 2, as amended), `operators::{Chain, MAINNET_GENESIS, REGTEST_GENESIS, hex}`, `node_api::read_chainstates_validated` (amended core plan, Task 2a: `getchainstates` through the pure `chainstates_validated`), `fsx::atomic_write` (exists), `rpc::Rpc` (exists).
- Produces (Tasks 2, 4, 5, 6, 7, 9):
  - `diary::{DIARY_FILE, DIARY_KEEP, DIARY_VERSION}`, `pub struct DiaryEntry { height: u64, block_hash: String, hash_serialized: String, coins: u64, chain_tx: u64, recorded_at: u64 }`, `pub struct Diary { version: u32, chain_id: String, entries: Vec<DiaryEntry> }` with `new(&str)`, `at(u64) -> Option<&DiaryEntry>`, `newest()`, `record(DiaryEntry)`
  - `diary_path(&Path) -> PathBuf`, `load(&Path, chain_id: &str) -> Diary`, `save(&Path, &Diary) -> io::Result<()>`, `summary(&Path) -> Option<String>`, `grid_for(chain_id: &str) -> Option<u64>`, `may_record(validation_mode: Option<&str>) -> bool`
  - `pub async fn gate_open(&dyn Rpc) -> bool` (the validated gate)
  - `pub enum DiaryOutcome { Recorded(DiaryEntry), AlreadyRecorded(u64), NotOnGrid, TipMoved, NotAllowed, Unvalidated(u64) }`
  - `pub async fn chain_id(&dyn Rpc) -> Option<String>`, `pub async fn read_at_tip(&dyn Rpc, grid: u64) -> Result<Option<DiaryEntry>, String>`, `pub async fn drop_off_chain(&dyn Rpc, &mut Diary) -> Vec<u64>`, `pub async fn record_at_tip(&dyn Rpc, dir: &Path, validation_mode: Option<&str>) -> Result<DiaryOutcome, String>`
  - test-only `crate::fake_node::{FakeNode, NodeState, synthetic_hash, chainstates_plain_assumeutxo, chainstates_signed_snapshot, chainstates_all_validated}`: `FakeNode::new(genesis, tip)`, `with(|&mut NodeState| ..)`, `methods()`, `count(method)`, `params(method)`; `NodeState { chain: BTreeMap<u64, String>, side: HashSet<String>, utxo: Option<Value>, chain_tx: u64, replay_context: Option<String>, mode: String, signer: Option<SigningKey>, invalidate_fails: bool, chainstates: Option<Value>, datadir: Option<PathBuf> }`. It answers `getblockcount`, `getblockhash`, `getblockheader` (confirmations as the engine counts them, -1 off the chain), `gettxoutsetinfo`, `getchaintxstats`, `getchainstates`, `getmatmultrustedstatus`, `invalidateblock` (moves the chain as the engine does) and `signutxosnapshotmanifest` (checks chain id and replay context as the engine does, takes a relative path from `datadir`, appends a real low-S signature to the file), and panics on anything else.

- [ ] **Step 1: Create the branch**

````bash
cd /Users/m2promende/repos/easynode
git fetch origin
git worktree add ../easynode-snapshot-network -b claude/snapshot-network claude/confirmed-snapshots
cd ../easynode-snapshot-network
git log --oneline -1
mkdir -p apps/node/src-tauri/resources/node-pkg && [ -n "$(ls apps/node/src-tauri/resources/node-pkg)" ] || echo placeholder > apps/node/src-tauri/resources/node-pkg/CI-PLACEHOLDER
git grep -n 'pub const SNAPSHOT_GRID\|pub const SNAPSHOT_DEPTH\|pub fn is_dissent\|pub fn dissent_statement\|pub fn check_shape\|pub async fn read_chainstates_validated\|pub fn parse_latest\|pub shielded:' -- crates/btx-core/src
````

Expected: the last commit is the confirmed-snapshots work (its changelog or engine-check commit). If `claude/confirmed-snapshots` exists only on origin, use `origin/claude/confirmed-snapshots`. The `git grep` finds each of the eight names the amended core plan adds; if one is missing or named differently, read "Names this plan uses from the amended core plan" at the end before going on.

- [ ] **Step 2: Write the scripted node and the failing tests**

Create `crates/btx-core/src/fake_node.rs`:

````rust
//! A scripted node for unit tests of the snapshot network (the diary, the
//! verdict, the producer's checks, both confirmers): a chain of block
//! hashes, the answers those modules read, `getchainstates` in the engine's
//! shape, `invalidateblock` that moves the chain the way the engine does,
//! and a signer that appends a real signature to a manifest file the way
//! `signutxosnapshotmanifest` does, a path relative to the node's data
//! folder included. Test-only.

use crate::confirmed_snapshot as cs;
use crate::error::{AppError, AppResult};
use crate::rpc::Rpc;
use async_trait::async_trait;
use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{Signature, SigningKey};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
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
    /// What `getchainstates` answers, `None` for an error. One validated
    /// chainstate unless a test says otherwise.
    pub chainstates: Option<Value>,
    /// The node's data folder: `signutxosnapshotmanifest` takes a relative
    /// path from here, as the engine does (`AbsPathForConfigVal`).
    pub datadir: Option<PathBuf>,
}

pub(crate) struct FakeNode {
    state: Mutex<NodeState>,
    calls: Mutex<Vec<(String, Value)>>,
}

/// A made-up block hash for a height, distinct per height.
pub(crate) fn synthetic_hash(height: u64) -> String {
    format!("{height:064x}")
}

/// One entry of `getchainstates.chainstates` with every field v0.34.9 writes
/// (`src/rpc/blockchain.cpp:5174-5199` at 84b998b4). `snapshot_blockhash`
/// is there only for a chainstate built from a snapshot, and `validated` is
/// false only for that one while the background chainstate still exists
/// (`:5211-5212`).
fn chainstate(blocks: u64, snapshot: Option<&str>, validated: bool, background: bool) -> Value {
    let mut c = json!({
        "blocks": blocks,
        "bestblockhash": synthetic_hash(blocks),
        "bits": "1d00ffff",
        "target": "00000000ffff0000000000000000000000000000000000000000000000000000",
        "difficulty": 1.0,
        "verificationprogress": if background { 0.41 } else { 0.9999 },
        "coins_db_cache_bytes": 8_388_608,
        "coins_tip_cache_bytes": 16_777_216,
        "validated": validated,
        "blocks_in_flight": if background { 16 } else { 0 },
        "admission_queue_depth": 0,
        "background_activation_yields": 0,
    });
    if let Some(base) = snapshot {
        c["snapshot_blockhash"] = json!(base);
    }
    c
}

/// A node on upstream's plain assumeutxo snapshot, taken with `loadtxoutset`
/// at 219,000 (v0.34.9's compiled start point, where Aleksander's nodes
/// started), still checking older history: the background chainstate first,
/// the snapshot one last and not validated. Nothing writes
/// `chainstate_snapshot/attested_assumeutxo` on this path.
pub(crate) fn chainstates_plain_assumeutxo() -> Value {
    json!({
        "headers": 233_812,
        "background_activation_yields": 3,
        "chainstates": [
            chainstate(131_200, None, true, true),
            chainstate(
                233_812,
                Some("dc51220bc7e5db96e29df9d817ae6179245d33eb8adcaaff765cfec83fdb87c3"),
                false,
                false
            ),
        ]
    })
}

/// The same shape on a node that loaded the signed 225,927 pair, a load that
/// also writes `attested_assumeutxo`.
pub(crate) fn chainstates_signed_snapshot() -> Value {
    json!({
        "headers": 233_812,
        "background_activation_yields": 3,
        "chainstates": [
            chainstate(98_400, None, true, true),
            chainstate(
                233_812,
                Some("06780445dae193010e099e6425c5430f121416b067b8d68a8a5c3b52e8a4b932"),
                false,
                false
            ),
        ]
    })
}

/// A node whose chain rests on its own checks: one chainstate, validated.
pub(crate) fn chainstates_all_validated() -> Value {
    json!({
        "headers": 233_812,
        "background_activation_yields": 0,
        "chainstates": [chainstate(233_812, None, true, false)]
    })
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
                chainstates: Some(chainstates_all_validated()),
                datadir: None,
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

    /// The parameters of every call to `method`, in order.
    pub fn params(&self, method: &str) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, p)| p.clone())
            .collect()
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
            "getchainstates" => s
                .chainstates
                .clone()
                .ok_or_else(|| rpc_err(-1, "getchainstates is not available")),
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
                let given = PathBuf::from(params[0].as_str().unwrap_or_default());
                let path = match &s.datadir {
                    Some(d) if given.is_relative() => d.join(&given),
                    _ => given,
                };
                let key = s.signer.clone().ok_or_else(|| {
                    rpc_err(
                        -1,
                        "signutxosnapshotmanifest requires a configured MatMul attestation local signer",
                    )
                })?;
                let bytes = std::fs::read(&path).map_err(|e| {
                    rpc_err(-22, &format!("Couldn't open manifest file for reading: {e}"))
                })?;
                let mut m = cs::parse(&bytes).map_err(|e| rpc_err(-22, &e.to_string()))?;
                // As the engine does: its own chain id and replay context,
                // and nothing else about the statement is looked at.
                let genesis = s.chain.get(&0).cloned().unwrap_or_default();
                let context = m.statement.replay_context().display_hex();
                if m.statement.chain_id().display_hex() != genesis
                    || s.replay_context.as_deref() != Some(context.as_str())
                {
                    return Err(rpc_err(
                        -1,
                        "Failed to sign manifest (check chain id / replay authority / local signer)",
                    ));
                }
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
                    "manifest_path": path.to_string_lossy(),
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
    use crate::fake_node::{
        chainstates_all_validated, chainstates_plain_assumeutxo, chainstates_signed_snapshot,
        FakeNode,
    };
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
        let out = record_at_tip(&node, dir.path(), Some("consensus"))
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
                "getchainstates",
                "gettxoutsetinfo",
                "getblockhash",
                "getchaintxstats"
            ],
            "the gate is read right before the entry"
        );
    }

    #[tokio::test]
    async fn the_same_height_twice_reads_the_utxo_set_once() {
        let dir = tempfile::tempdir().unwrap();
        let node = regtest_at_100();
        record_at_tip(&node, dir.path(), Some("consensus"))
            .await
            .unwrap();
        let again = record_at_tip(&node, dir.path(), Some("consensus"))
            .await
            .unwrap();
        assert_eq!(again, DiaryOutcome::AlreadyRecorded(100));
        assert_eq!(node.count("gettxoutsetinfo"), 1);
    }

    #[tokio::test]
    async fn off_the_grid_nothing_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let node = FakeNode::new(MAINNET_GENESIS, 226_001);
        let out = record_at_tip(&node, dir.path(), Some("consensus"))
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::NotOnGrid);
        assert_eq!(node.count("gettxoutsetinfo"), 0);
        assert_eq!(node.count("getchainstates"), 0);
    }

    #[tokio::test]
    async fn an_answer_for_a_block_that_moved_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        // The tip moved on while the engine read: it answers for 101.
        let node = regtest_at_100();
        node.with(|s| {
            s.utxo.as_mut().unwrap()["height"] = json!(101);
        });
        let out = record_at_tip(&node, dir.path(), Some("consensus"))
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::TipMoved);
        // A reorg while the engine read: the answer names a block that is no
        // longer the one at 100.
        let node = regtest_at_100();
        node.with(|s| {
            s.utxo.as_mut().unwrap()["bestblock"] = json!("ff".repeat(32));
        });
        let out = record_at_tip(&node, dir.path(), Some("consensus"))
            .await
            .unwrap();
        assert_eq!(out, DiaryOutcome::TipMoved);
        assert!(!diary_path(dir.path()).exists());
    }

    #[tokio::test]
    async fn a_mirror_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let node = regtest_at_100();
        for mode in [Some("trusted"), None] {
            let out = record_at_tip(&node, dir.path(), mode).await.unwrap();
            assert_eq!(out, DiaryOutcome::NotAllowed);
        }
        assert!(node.methods().is_empty(), "not even a read");
        assert!(may_record(Some("consensus")));
        assert!(!may_record(Some("trusted")));
        assert!(!may_record(None));
    }

    /// Section 3 (jpp's point 4): nothing is written while `getchainstates`
    /// shows a chainstate the node has not finished checking, for upstream's
    /// plain assumeutxo snapshot (no `attested_assumeutxo` file) and for a
    /// signed one alike; a missing or unreadable answer is closed too. The
    /// same node records again once every chainstate is validated.
    #[tokio::test]
    async fn an_unvalidated_snapshot_chainstate_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let node = regtest_at_100();
        let record = crate::node::attested_snapshot_record(dir.path());
        node.with(|s| s.chainstates = Some(chainstates_plain_assumeutxo()));
        assert!(!record.exists(), "the plain snapshot leaves no such file");
        assert_eq!(
            record_at_tip(&node, dir.path(), Some("consensus"))
                .await
                .unwrap(),
            DiaryOutcome::Unvalidated(100)
        );
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, b"v2").unwrap();
        for answer in [
            Some(chainstates_signed_snapshot()),
            None,
            Some(json!("not an answer")),
        ] {
            node.with(|s| s.chainstates = answer.clone());
            assert_eq!(
                record_at_tip(&node, dir.path(), Some("consensus"))
                    .await
                    .unwrap(),
                DiaryOutcome::Unvalidated(100),
                "{answer:?}"
            );
        }
        assert_eq!(node.count("gettxoutsetinfo"), 0, "the UTXO set is never read");
        assert!(!diary_path(dir.path()).exists());
        // Every chainstate validated: it writes again, and the engine's file
        // is not the test any more.
        node.with(|s| s.chainstates = Some(chainstates_all_validated()));
        let out = record_at_tip(&node, dir.path(), Some("consensus"))
            .await
            .unwrap();
        assert!(matches!(out, DiaryOutcome::Recorded(_)), "{out:?}");
        assert!(gate_open(&node).await);
    }

    /// Section 3: an entry counts only while its block is still the block at
    /// that height on the active chain. A sibling that lost, and an entry
    /// above today's tip after a reorg to a shorter chain, are dropped at the
    /// next write; an entry still on the chain stays.
    #[tokio::test]
    async fn an_entry_whose_block_left_the_chain_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let node = FakeNode::new(REGTEST_GENESIS, 300);
        node.with(|s| {
            s.utxo = Some(json!({
                "height": 300, "bestblock": format!("{:064x}", 300), "txouts": 301,
                "hash_serialized_3": "cd".repeat(32)
            }));
            s.chain_tx = 301;
        });
        let mut d = Diary::new(REGTEST_GENESIS);
        d.record(entry(100));
        d.record(DiaryEntry {
            block_hash: "44".repeat(32),
            ..entry(200)
        });
        d.record(entry(400));
        save(dir.path(), &d).unwrap();
        let out = record_at_tip(&node, dir.path(), Some("consensus"))
            .await
            .unwrap();
        assert!(matches!(out, DiaryOutcome::Recorded(_)), "{out:?}");
        let heights: Vec<u64> = load(dir.path(), REGTEST_GENESIS)
            .entries
            .iter()
            .map(|e| e.height)
            .collect();
        assert_eq!(heights, vec![100, 300]);
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
        assert_eq!(d.entries.len(), DIARY_KEEP);
        assert_eq!(d.entries.first().unwrap().height, 2_100);
        assert_eq!(d.newest().unwrap().height, 12_000);
        assert_eq!(d.at(12_000).unwrap().coins, 7);
        assert!(d.at(2_000).is_none());
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
        assert_eq!(grid_for(MAINNET_GENESIS), Some(100));
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

- [ ] **Step 3: Run them to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- diary::`
Expected: compile errors, among them `` cannot find function `record_at_tip` in this scope `` and `` cannot find struct, variant or union type `DiaryEntry` ``.

- [ ] **Step 4: Write the diary**

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
//! only real check a co-signature carries, and what a dissent is built from.
//!
//! WHEN. `gettxoutsetinfo` answers only at the tip, so the app reads it when
//! the tip is on the grid (every 100 blocks, section 2 of
//! docs/decisions/2026-09-29-every-node-starts-near-the-tip.md) and keeps the
//! answer only if the block it names is still the one at its height. A height
//! the tip passed before the app read it is skipped; the next chance is 100
//! blocks later. An entry counts only while its block is still the block at
//! that height on the node's active chain: one whose block left it is dropped
//! at the next write ([`drop_off_chain`]) and never compared (the verdict
//! checks it again).
//!
//! WHO. Only a node whose chain state rests on its own checks: it validates
//! (a mirror holds a UTXO set it never checked), and `getchainstates` shows
//! no chainstate with `"validated": false` ([`gate_open`], section 3). That
//! covers upstream's plain assumeutxo snapshot and a signed one alike; the
//! engine's `attested_assumeutxo` file exists only for the second, so it is
//! not the test. Otherwise one snapshot could vouch for the next.
//!
//! WHERE. `<datadir>/snapshot-diary.json` (`btx-confirmer` keeps it in its
//! `--state` folder), the newest 100 heights, written atomically, for one
//! chain: a diary from another chain reads as empty.

use crate::confirmed_snapshot as cs;
use crate::error::AppError;
use crate::operators::Chain;
use crate::rpc::Rpc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

pub const DIARY_FILE: &str = "snapshot-diary.json";
/// Heights kept: 100 heights of 100 blocks is 10,000 blocks, about ten days
/// at 40 an hour, longer than the seven days the website's `pending` covers.
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

/// The grid of the chain a genesis hash names: 100 on every chain easyNode
/// knows, `None` for any other.
pub fn grid_for(chain_id: &str) -> Option<u64> {
    Chain::from_genesis_hex(chain_id).map(|_| cs::SNAPSHOT_GRID as u64)
}

/// Whether this node may write in its diary at all: it validates. The
/// validated gate ([`gate_open`]) is asked right before every entry.
pub fn may_record(validation_mode: Option<&str>) -> bool {
    validation_mode.is_some_and(|m| m.trim().eq_ignore_ascii_case("consensus"))
}

/// The validated gate (section 3, jpp's point 4): open only while
/// `getchainstates` answers and no chainstate in it has `"validated": false`.
/// A failed or unreadable answer is closed. The diary, the producer and both
/// confirmers ask it before every entry, export, send and signature.
pub async fn gate_open(rpc: &dyn Rpc) -> bool {
    crate::node_api::read_chainstates_validated(rpc).await
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
    /// A mirror: it does not check blocks itself.
    NotAllowed,
    /// The tip is on the grid at this height, and `getchainstates` shows a
    /// chainstate the node has not finished checking (or did not answer).
    Unvalidated(u64),
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

/// Drop every entry whose block is no longer the block at its height on the
/// node's active chain (section 3), above today's tip included. Returns the
/// heights dropped. An entry the node gave no answer for is kept; the
/// verdict looks at it again before any comparison.
pub async fn drop_off_chain(rpc: &dyn Rpc, diary: &mut Diary) -> Vec<u64> {
    let mut gone = Vec::new();
    for e in &diary.entries {
        let off = match rpc.call("getblockhash", json!([e.height])).await {
            Ok(v) => v.as_str().map(str::to_ascii_lowercase) != Some(e.block_hash.to_ascii_lowercase()),
            // "Block height out of range": the chain is shorter than that now.
            Err(AppError::Rpc { code: -8, .. }) => true,
            Err(_) => false,
        };
        if off {
            gone.push(e.height);
        }
    }
    diary.entries.retain(|e| !gone.contains(&e.height));
    gone
}

/// One diary step, cheap when there is nothing to do: the status refresher
/// calls it on a tick whose tip is on the grid, the producer right before it
/// exports, and `btx-confirmer` every few seconds while the tip is on the
/// grid. `dir` holds the diary (the datadir, or `btx-confirmer`'s state).
pub async fn record_at_tip(
    rpc: &dyn Rpc,
    dir: &Path,
    validation_mode: Option<&str>,
) -> Result<DiaryOutcome, String> {
    if !may_record(validation_mode) {
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
    if !gate_open(rpc).await {
        return Ok(DiaryOutcome::Unvalidated(tip));
    }
    let Some(entry) = read_at_tip(rpc, grid).await? else {
        return Ok(DiaryOutcome::TipMoved);
    };
    drop_off_chain(rpc, &mut diary).await;
    diary.record(entry.clone());
    save(dir, &diary).map_err(|e| format!("writing the diary: {e}"))?;
    Ok(DiaryOutcome::Recorded(entry))
}
````

- [ ] **Step 5: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- diary::`
Expected: `9 passed; 0 failed` (derived, not run). The first test reads the spike's regtest values as the engine reports them (block `bd23c642…2c46`, UTXO hash `e611efee…d527`, 101 coins, 101 transactions) and checks the exact RPC sequence, with the gate right before the UTXO read: `getblockhash`, `getblockcount`, `getchainstates`, `gettxoutsetinfo`, `getblockhash`, `getchaintxstats`.

Sabotage, then undo: in `gate_open`, replace the body with `true`; run the same command; expect `an_unvalidated_snapshot_chainstate_writes_nothing` to FAIL (it records at 100 on the plain snapshot's answer); put the line back.

- [ ] **Step 6: Format, lint, commit**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cd ../..
git add crates/btx-core/src/diary.rs crates/btx-core/src/fake_node.rs crates/btx-core/src/lib.rs
git commit -m "core: the diary, this node's own chain state at every grid height, only on a chain it checked itself" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 2: A statement against this node and its diary, and the one verdict (`statement_check.rs`)

(Was Task 3. Its old Step 3, which split `check_shape` out of `check_with`, is gone: the amended core plan has `confirmed_snapshot::check_shape` (chain fields, not a dissent, file geometry and size, grid).)

**Files:**
- Create: `crates/btx-core/src/statement_check.rs`
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 (`Diary`, `DiaryEntry`, `gate_open`, `FakeNode` and its three `getchainstates` answers), `confirmed_snapshot::{ChainRules, Statement, Hash32, Refusal, parse, check_shape, is_dissent, dissent_statement, SNAPSHOT_DEPTH, REGTEST_REPLAY_CONTEXT}` and `ChainRules::shielded` (core plan, Task 2, as amended), `confirmed_load::Holds` (core plan, Task 4), `known_invalid::HeldBranch`, `node_api::get_matmul_trusted_status` with `replay_authority_context` (core plan, Task 4).
- Produces (Tasks 4, 5, 7, 9):
  - `pub enum statement_check::Mismatch { Unvalidated, Unreadable(String), ChainId { node }, ReplayContext { node: Option<String> }, TooShallow { have: u64, need: u64 }, HeldRootOnActiveChain { height, root }, HeldNotRefused(String), NoDiaryEntry { height }, DiaryEntryLeftChain { height }, BlockHash { diary, statement }, HashSerialized { diary, statement }, Coins { diary, statement }, ChainTx { diary, statement }, NodeUnanswered(String) }` (`Display` one line; `From<Refusal>`)
  - `pub struct NodeFacts { validated: bool, genesis: String, replay_context: Option<String>, tip: u64, block_at_height: Option<String>, held_on_chain: Option<(u64, String)> }` (`Default`)
  - `pub enum Verdict { Sign, Dissent(Mismatch), Neither(Mismatch) }`
  - `pub fn compare_with_diary(&Statement, &DiaryEntry) -> Result<(), Mismatch>`
  - `pub fn verdict(&Statement, &ChainRules, &NodeFacts, &Diary, depth: u64) -> Verdict` (pure, the one both confirmers and the producer use)
  - `pub async fn read_node_facts(&dyn Rpc, height: u64, &Holds<'_>) -> Result<NodeFacts, Mismatch>`
  - `pub async fn check_against_node(&dyn Rpc, &Statement, &ChainRules, &Diary, &Holds<'_>, depth: u64) -> Verdict`
  - `pub fn dissent_from(&DiaryEntry, &ChainRules) -> Result<Statement, Mismatch>`

- [ ] **Step 1: Write the failing tests**

Create `crates/btx-core/src/statement_check.rs` with only the test module. It has one sabotage per chain field (block hash, UTXO hash, coin count, transaction count), each a dissent; a base off this node's chain, a dissent through the node too; no diary entry and an entry whose block left the chain, neither; 143 and 144 deep; a node on another chain, with another or no replay context, a statement with another replay context or shielded commitment, neither; the three `getchainstates` answers; a refused block under the base; an off-grid statement refused before the chain is asked; and the dissent built from a diary entry:

````rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::diary::DiaryEntry;
    use crate::fake_node::{
        chainstates_all_validated, chainstates_plain_assumeutxo, chainstates_signed_snapshot,
        FakeNode,
    };
    use crate::known_invalid::HeldBranch;
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
    const DEPTH: u64 = cs::SNAPSHOT_DEPTH as u64;

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

    /// The node that exported the spike's statement, 144 blocks later.
    fn node() -> FakeNode {
        let n = FakeNode::new(REGTEST_GENESIS, 243);
        n.with(|s| {
            s.chain.insert(100, R_BLOCK.into());
            s.replay_context = Some(cs::REGTEST_REPLAY_CONTEXT.into());
        });
        n
    }

    /// What `read_node_facts` reads from that node about height 100.
    fn facts() -> NodeFacts {
        NodeFacts {
            validated: true,
            genesis: REGTEST_GENESIS.into(),
            replay_context: Some(cs::REGTEST_REPLAY_CONTEXT.into()),
            tip: 243,
            block_at_height: Some(R_BLOCK.into()),
            held_on_chain: None,
        }
    }

    async fn check(n: &FakeNode, d: &Diary, holds: &Holds<'_>) -> Verdict {
        check_against_node(n, &statement(), &rules(), d, holds, DEPTH).await
    }

    #[tokio::test]
    async fn the_spikes_statement_is_signed_by_the_node_that_made_it() {
        assert_eq!(check(&node(), &diary(), &Holds::none()).await, Verdict::Sign);
        assert_eq!(
            read_node_facts(&node(), 100, &Holds::none()).await,
            Ok(facts())
        );
    }

    /// One sabotage per chain field: each is the only thing wrong, the
    /// node's block at 100 is the diary's, and each is a dissent with its
    /// own reason.
    #[test]
    fn a_chain_field_that_differs_from_the_diary_is_a_dissent() {
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
            let f = NodeFacts {
                block_at_height: Some(e.block_hash.clone()),
                ..facts()
            };
            d.record(e);
            assert_eq!(
                verdict(&statement(), &rules(), &f, &d, DEPTH),
                Verdict::Dissent(want),
                "{what}"
            );
        }
    }

    /// A node that took a sibling at 100 and wrote it down: the statement's
    /// block is not its block, and that is a disagreement, not a silence.
    #[tokio::test]
    async fn a_base_off_this_nodes_chain_is_a_dissent_through_the_node_too() {
        let n = node();
        n.with(|s| {
            s.chain.insert(100, "44".repeat(32));
        });
        let mut d = diary();
        let mut e = d.at(100).unwrap().clone();
        e.block_hash = "44".repeat(32);
        d.record(e);
        assert_eq!(
            check(&n, &d, &Holds::none()).await,
            Verdict::Dissent(Mismatch::BlockHash {
                diary: "44".repeat(32),
                statement: R_BLOCK.into()
            })
        );
    }

    #[test]
    fn nothing_is_said_without_a_diary_entry_on_this_chain() {
        assert_eq!(
            verdict(
                &statement(),
                &rules(),
                &facts(),
                &Diary::new(REGTEST_GENESIS),
                DEPTH
            ),
            Verdict::Neither(Mismatch::NoDiaryEntry { height: 100 })
        );
        // The entry agrees with the statement, but its block left this
        // node's chain: it is never compared, so it confirms nothing.
        let f = NodeFacts {
            block_at_height: Some("44".repeat(32)),
            ..facts()
        };
        assert_eq!(
            verdict(&statement(), &rules(), &f, &diary(), DEPTH),
            Verdict::Neither(Mismatch::DiaryEntryLeftChain { height: 100 })
        );
    }

    #[test]
    fn a_block_less_than_144_deep_waits() {
        let f = NodeFacts { tip: 242, ..facts() };
        assert_eq!(
            verdict(&statement(), &rules(), &f, &diary(), DEPTH),
            Verdict::Neither(Mismatch::TooShallow {
                have: 143,
                need: 144
            })
        );
        assert_eq!(
            verdict(&statement(), &rules(), &facts(), &diary(), DEPTH),
            Verdict::Sign,
            "exactly 144 is enough"
        );
        // A node that has not reached 100 at all.
        let f = NodeFacts {
            tip: 60,
            block_at_height: None,
            ..facts()
        };
        assert_eq!(
            verdict(&statement(), &rules(), &f, &diary(), DEPTH),
            Verdict::Neither(Mismatch::TooShallow { have: 0, need: 144 })
        );
    }

    #[test]
    fn a_foreign_chain_replay_context_or_shielded_commitment_is_neither() {
        let v = |f: &NodeFacts| verdict(&statement(), &rules(), f, &diary(), DEPTH);
        assert_eq!(
            v(&NodeFacts {
                genesis: MAINNET_GENESIS.into(),
                ..facts()
            }),
            Verdict::Neither(Mismatch::ChainId {
                node: MAINNET_GENESIS.into()
            })
        );
        assert_eq!(
            v(&NodeFacts {
                replay_context: Some("33".repeat(32)),
                ..facts()
            }),
            Verdict::Neither(Mismatch::ReplayContext {
                node: Some("33".repeat(32))
            })
        );
        assert_eq!(
            v(&NodeFacts {
                replay_context: None,
                ..facts()
            }),
            Verdict::Neither(Mismatch::ReplayContext { node: None })
        );
        // The statement's own replay context (bytes 149..181) or shielded
        // commitment (117..149) is not the one compiled for its chain.
        for (at, what) in [(149usize, "replay context"), (117, "shielded commitment")] {
            let mut raw = *statement().raw();
            raw[at] ^= 1;
            let st = Statement::from_raw(raw);
            assert!(
                matches!(
                    verdict(&st, &rules(), &facts(), &diary(), DEPTH),
                    Verdict::Neither(Mismatch::Unreadable(_))
                ),
                "{what}"
            );
        }
    }

    /// Section 3: while `getchainstates` shows a chainstate the node has not
    /// finished checking, nothing is signed and nothing is disputed, whatever
    /// the diary says, and nothing else is even read.
    #[tokio::test]
    async fn an_unvalidated_snapshot_chainstate_is_neither_whatever_the_diary_says() {
        for answer in [
            Some(chainstates_plain_assumeutxo()),
            Some(chainstates_signed_snapshot()),
            None,
        ] {
            let n = node();
            n.with(|s| s.chainstates = answer.clone());
            assert_eq!(
                check(&n, &diary(), &Holds::none()).await,
                Verdict::Neither(Mismatch::Unvalidated),
                "{answer:?}"
            );
            assert_eq!(n.methods(), vec!["getchainstates"], "nothing else is read");
        }
        let n = node();
        n.with(|s| s.chainstates = Some(chainstates_all_validated()));
        assert_eq!(check(&n, &diary(), &Holds::none()).await, Verdict::Sign);
        assert!(!verdict(
            &statement(),
            &rules(),
            &NodeFacts {
                validated: false,
                ..facts()
            },
            &diary(),
            DEPTH
        )
        .is_sign());
    }

    #[tokio::test]
    async fn a_refused_block_under_the_base_is_neither() {
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
            Verdict::Neither(Mismatch::HeldRootOnActiveChain {
                height: 50,
                root: ROOT.into()
            })
        );
        // The same hold, not on this chain: nothing to say.
        assert_eq!(check(&node(), &diary(), &holds).await, Verdict::Sign);
        assert_eq!(n.count("invalidateblock"), 0, "read-only");
    }

    #[tokio::test]
    async fn an_off_grid_statement_is_neither_before_the_chain_is_asked() {
        let mut raw = *statement().raw();
        raw[65..69].copy_from_slice(&150i32.to_le_bytes());
        let st = Statement::from_raw(raw);
        let n = node();
        let got = check_against_node(&n, &st, &rules(), &diary(), &Holds::none(), DEPTH).await;
        assert_eq!(
            got,
            Verdict::Neither(Mismatch::Unreadable(
                "height 150 is not a multiple of 100".into()
            ))
        );
        assert_eq!(n.methods(), vec!["getchainstates"], "only the gate");
    }

    /// Section 6a: a dissent is this node's diary entry, with the chain id,
    /// replay context and shielded commitment compiled for the chain, and
    /// all four file fields zero. It never passes the shape a load needs.
    #[test]
    fn a_dissent_is_the_diary_entry_with_no_file() {
        let mut e = diary().at(100).unwrap().clone();
        e.coins = 102;
        let st = dissent_from(&e, &rules()).unwrap();
        assert!(cs::is_dissent(&st));
        assert_eq!((st.version(), st.height()), (2, 100));
        assert_eq!(st.block_hash().display_hex(), R_BLOCK);
        assert_eq!(st.hash_serialized().display_hex(), R_UTXO);
        assert_eq!((st.coins(), st.chain_tx()), (102, 101));
        assert_eq!(st.chain_id(), rules().genesis);
        assert_eq!(st.replay_context(), rules().replay_context);
        assert_eq!(st.shielded(), rules().shielded);
        assert_eq!(
            (st.file_size(), st.file_hash(), st.chunk_size(), st.chunk_count()),
            (0, cs::Hash32([0; 32]), 0, 0)
        );
        assert_ne!(st.hash(), statement().hash());
        assert!(cs::check_shape(&st, &rules()).is_err());
        let bad = DiaryEntry {
            block_hash: "not hex".into(),
            ..e
        };
        assert!(matches!(
            dissent_from(&bad, &rules()),
            Err(Mismatch::Unreadable(_))
        ));
    }
}
````

In `crates/btx-core/src/lib.rs`, after `pub mod snapshot_serve;` add `pub mod statement_check;`.

- [ ] **Step 2: Run them to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- statement_check::`
Expected: compile errors, among them `` cannot find function `check_against_node` in this scope ``, `` cannot find type `Verdict` in this scope `` and `` cannot find type `Mismatch` in this scope ``.

- [ ] **Step 3: Write the checks and the verdict**

Insert above `#[cfg(test)]` in `crates/btx-core/src/statement_check.rs`:

````rust
//! Does a snapshot statement say what THIS node saw? The one check behind
//! every co-signature, every dissent and every submission, shared by the
//! app's confirmer, `btx-confirmer` and the producer.
//!
//! `signutxosnapshotmanifest` signs whatever it is handed (a node at height 0
//! co-signed a statement about block 100 in the spike of 2026-09-29), so a
//! signature means something only because the app compares first. The
//! [`verdict`] is pure, over what the node said ([`NodeFacts`]) and its diary
//! (`crate::diary`), in this order (sections 3 to 6a of
//! docs/decisions/2026-09-29-every-node-starts-near-the-tip.md):
//!
//! 1. `getchainstates` shows no chainstate the node has not finished
//!    checking (the validated gate); else **Neither**;
//! 2. the statement's shape under its chain's rules
//!    (`confirmed_snapshot::check_shape`: the compiled chain id, replay
//!    context and shielded commitment among them); else **Neither**;
//! 3. the node is on the statement's chain and reports its replay context;
//!    else **Neither**;
//! 4. the height is at least 144 deep on the node's ACTIVE chain; else
//!    **Neither**, and it waits;
//! 5. no block the app refuses (`known_invalid`) is on that chain at or
//!    below the height; else **Neither**;
//! 6. the node's own diary has that height, for the block the node has there
//!    now; else **Neither**;
//! 7. block hash, UTXO set hash, coin count and transaction count equal the
//!    diary entry: **Sign**; any of them differs: **Dissent**.
//!
//! Read-only: nothing here changes the node.

use crate::confirmed_load::Holds;
use crate::confirmed_snapshot::{self as cs, ChainRules, Hash32, Statement};
use crate::diary::{Diary, DiaryEntry};
use crate::rpc::Rpc;
use serde_json::json;

/// Why a statement is not signed as it is. `Display` is one plain line for
/// the log and for Copy diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    /// `getchainstates` shows a chainstate the node has not finished
    /// checking, or did not answer.
    Unvalidated,
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
    /// Not deep enough on this node's chain yet.
    TooShallow {
        have: u64,
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
    /// The diary has this height, for a block no longer on this chain.
    DiaryEntryLeftChain {
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
            Mismatch::Unvalidated => write!(
                f,
                "this node is still checking the snapshot it started from (getchainstates shows a chainstate not validated yet)"
            ),
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
            Mismatch::TooShallow { have, need } => write!(
                f,
                "the block is {have} blocks deep on this node's chain, {need} are needed"
            ),
            Mismatch::HeldRootOnActiveChain { height, root } => write!(
                f,
                "block {root} at {height}, which the app refuses, is on this node's chain"
            ),
            Mismatch::HeldNotRefused(e) => write!(f, "a refused block could not be refused: {e}"),
            Mismatch::NoDiaryEntry { height } => {
                write!(f, "this node's diary has nothing at {height}")
            }
            Mismatch::DiaryEntryLeftChain { height } => write!(
                f,
                "this node's diary entry at {height} is for a block no longer on its chain"
            ),
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

/// What the running node says about one height, read once per statement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeFacts {
    /// The validated gate is open (`crate::diary::gate_open`).
    pub validated: bool,
    /// `getblockhash 0`, display hex.
    pub genesis: String,
    /// `getmatmultrustedstatus.replay_authority_context`, lowercase.
    pub replay_context: Option<String>,
    pub tip: u64,
    /// The block at the height on the active chain; `None` above the tip.
    pub block_at_height: Option<String>,
    /// The first block the app refuses that sits on the active chain at or
    /// below the height.
    pub held_on_chain: Option<(u64, String)>,
}

/// What to do about a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Every check passed: sign it.
    Sign,
    /// A chain field differs from this node's diary: send a dissent built
    /// from the diary (section 6a), never a signature.
    Dissent(Mismatch),
    /// Nothing to sign and nothing to dispute (yet, or on this node).
    Neither(Mismatch),
}

impl Verdict {
    pub fn is_sign(&self) -> bool {
        matches!(self, Verdict::Sign)
    }
}

/// Field by field against the diary entry at the statement's height.
pub fn compare_with_diary(st: &Statement, e: &DiaryEntry) -> Result<(), Mismatch> {
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

/// The one verdict (module doc, steps 1 to 7). `depth` is
/// `snapshot_serve::CONFIRMATIONS_REQUIRED` (144).
pub fn verdict(
    st: &Statement,
    rules: &ChainRules,
    node: &NodeFacts,
    diary: &Diary,
    depth: u64,
) -> Verdict {
    use Verdict::{Dissent, Neither, Sign};
    if !node.validated {
        return Neither(Mismatch::Unvalidated);
    }
    let height = match cs::check_shape(st, rules) {
        Ok(h) => h,
        Err(r) => return Neither(r.into()),
    };
    if !node
        .genesis
        .eq_ignore_ascii_case(&st.chain_id().display_hex())
    {
        return Neither(Mismatch::ChainId {
            node: node.genesis.clone(),
        });
    }
    if node.replay_context.as_deref().map(str::to_ascii_lowercase)
        != Some(st.replay_context().display_hex())
    {
        return Neither(Mismatch::ReplayContext {
            node: node.replay_context.clone(),
        });
    }
    let have = (node.tip + 1).saturating_sub(height);
    if have < depth {
        return Neither(Mismatch::TooShallow { have, need: depth });
    }
    if let Some((h, root)) = &node.held_on_chain {
        return Neither(Mismatch::HeldRootOnActiveChain {
            height: *h,
            root: root.clone(),
        });
    }
    let Some(entry) = diary.at(height) else {
        return Neither(Mismatch::NoDiaryEntry { height });
    };
    if node.block_at_height.as_deref().map(str::to_ascii_lowercase)
        != Some(entry.block_hash.to_ascii_lowercase())
    {
        return Neither(Mismatch::DiaryEntryLeftChain { height });
    }
    match compare_with_diary(st, entry) {
        Ok(()) => Sign,
        Err(m) => Dissent(m),
    }
}

async fn block_at(rpc: &dyn Rpc, height: u64) -> Option<String> {
    rpc.call("getblockhash", json!([height]))
        .await
        .ok()
        .and_then(|v| v.as_str().map(str::to_ascii_lowercase))
}

/// Everything but the gate, for `height`.
async fn read_chain(rpc: &dyn Rpc, height: u64, holds: &Holds<'_>) -> Result<NodeFacts, Mismatch> {
    let genesis = block_at(rpc, 0)
        .await
        .ok_or_else(|| Mismatch::NodeUnanswered("getblockhash 0".into()))?;
    let replay_context = crate::node_api::get_matmul_trusted_status(rpc)
        .await
        .map_err(|e| Mismatch::NodeUnanswered(format!("getmatmultrustedstatus: {e}")))?
        .replay_authority_context
        .map(|c| c.to_ascii_lowercase());
    let tip = rpc
        .call("getblockcount", json!([]))
        .await
        .ok()
        .and_then(|v| v.as_u64())
        .ok_or_else(|| Mismatch::NodeUnanswered("getblockcount".into()))?;
    let block_at_height = if height <= tip {
        block_at(rpc, height).await
    } else {
        None
    };
    let roots = holds
        .invalid
        .iter()
        .map(|b| (b.height, b.hash))
        .chain(holds.held.iter().map(|h| (h.height, h.root)));
    let mut held_on_chain = None;
    for (h, root) in roots.filter(|(h, _)| *h <= height && *h <= tip) {
        if block_at(rpc, h).await.as_deref() == Some(root) {
            held_on_chain = Some((h, root.to_string()));
            break;
        }
    }
    Ok(NodeFacts {
        validated: true,
        genesis,
        replay_context,
        tip,
        block_at_height,
        held_on_chain,
    })
}

/// What the running node says about `height`, read-only: the gate first,
/// and with it closed nothing else is read.
pub async fn read_node_facts(
    rpc: &dyn Rpc,
    height: u64,
    holds: &Holds<'_>,
) -> Result<NodeFacts, Mismatch> {
    if !crate::diary::gate_open(rpc).await {
        return Ok(NodeFacts::default());
    }
    read_chain(rpc, height, holds).await
}

/// The verdict against the running node: the gate, then the statement's
/// shape before the chain is asked anything, then the chain. A node that
/// does not answer is Neither.
pub async fn check_against_node(
    rpc: &dyn Rpc,
    st: &Statement,
    rules: &ChainRules,
    diary: &Diary,
    holds: &Holds<'_>,
    depth: u64,
) -> Verdict {
    if !crate::diary::gate_open(rpc).await {
        return Verdict::Neither(Mismatch::Unvalidated);
    }
    let height = match cs::check_shape(st, rules) {
        Ok(h) => h,
        Err(r) => return Verdict::Neither(r.into()),
    };
    match read_chain(rpc, height, holds).await {
        Ok(facts) => verdict(st, rules, &facts, diary, depth),
        Err(m) => Verdict::Neither(m),
    }
}

/// A dissent (section 6a): the statement this node would have signed, built
/// from its own diary entry, with the chain id, replay context and shielded
/// commitment compiled for the chain, and all four file fields zero. The
/// engine refuses to load it (a zero file size and a null file hash), the
/// app's parser refuses it, and the website takes no file for it.
pub fn dissent_from(entry: &DiaryEntry, rules: &ChainRules) -> Result<Statement, Mismatch> {
    let hash = |what: &str, hex: &str| {
        Hash32::from_display_hex(hex)
            .ok_or_else(|| Mismatch::Unreadable(format!("the diary's {what} is not a hash: {hex}")))
    };
    let raw = cs::dissent_statement(
        entry.height,
        &hash("block hash", &entry.block_hash)?,
        &hash("UTXO set hash", &entry.hash_serialized)?,
        entry.coins,
        entry.chain_tx,
        &rules.genesis,
        &rules.replay_context,
        &rules.shielded,
    )
    .ok_or_else(|| {
        Mismatch::Unreadable(format!(
            "the diary's height does not fit a statement: {}",
            entry.height
        ))
    })?;
    Ok(Statement::from_raw(raw))
}
````

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- statement_check::`
Expected: `10 passed; 0 failed` (derived, not run).

Sabotage, then undo: in `verdict`, change `Err(m) => Dissent(m)` to `Err(m) => Neither(m)`; run; expect `a_chain_field_that_differs_from_the_diary_is_a_dissent` and `a_base_off_this_nodes_chain_is_a_dissent_through_the_node_too` to FAIL; put it back. Then delete the `DiaryEntryLeftChain` check; expect `nothing_is_said_without_a_diary_entry_on_this_chain` to FAIL with `Sign`; put it back.

- [ ] **Step 5: Format, lint, commit**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cd ../..
git add crates/btx-core/src/statement_check.rs crates/btx-core/src/lib.rs
git commit -m "core: one verdict on a snapshot statement: sign, dissent or neither, against this node and its diary" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 3: The website's client (`snapshot_site.rs`)

(Was Task 4. New here: the `dissent` flag on `pending` entries, `latest` with its disputed answer (read by the core plan's own parser, `attested_snapshot::parse_latest`, so the answer is parsed in one place), and the closed-height refusal.)

**Files:**
- Create: `crates/btx-core/src/snapshot_site.rs`
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: `operators::Chain`; `attested_snapshot::{Latest, parse_latest}` (core plan, Task 3, as amended: `Latest::Confirmed(ConfirmedPointer)` or `Latest::Disputed(Vec<u64>)`) and its fixture `tests/fixtures/confirmed_snapshot/latest.json`; the website plan's routes and replies (its Global Constraints, as amended).
- Produces (Tasks 4, 5, 6, 7, 9): `SITE`, `SITE_ENV` (`EASYNODE_SNAPSHOT_SITE`), `NODE_HEADER`, `NODE_HEADER_VALUE`, `MAX_PART_BYTES`, `TIMEOUT`; `site_base_from(Option<&str>) -> String`, `site_base() -> String`, `client() -> Result<reqwest::Client, String>`, `chain_name(Chain) -> &'static str`; `StatementReply { statement_hash, chain, height, signers, operators, added, file }`, `PendingStatement { statement_hash, height, block_hash, manifest_hex, signers, operators, file, first_seen, confirmed, disputed, dissent }`, `Pending { version, chain, statements }`, `FileStored { stored, file_url, file_sha256, confirmed }`, `pub use attested_snapshot::Latest`; `enum SiteError { Unreachable(String), Refused { status: u16, reason: String }, Unreadable(String) }` with `is_closed_height()`; `post_statement(&Client, base, &[u8]) -> Result<StatementReply, SiteError>`, `get_pending(&Client, base, chain: &str) -> Result<Pending, SiteError>`, `get_latest(&Client, base, chain: &str) -> Result<Option<Latest>, SiteError>` (`None`: nothing confirmed, HTTP 404), `upload_file(&Client, base, statement_hash, &Path) -> Result<Option<FileStored>, SiteError>` (`None`: the website had the file).

- [ ] **Step 1: Write the failing tests**

Create `crates/btx-core/src/snapshot_site.rs` with only the test module (mockito stands in for the website; the parts are checked byte for byte):

````rust
#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Matcher;

    const R_P: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-P.manifest");
    const R_DAT: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/regtest-100.dat");
    const POINTER: &[u8] = include_bytes!("../tests/fixtures/confirmed_snapshot/latest.json");
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
        assert!(!err.is_closed_height());
        let err = post_statement(&client().unwrap(), "http://127.0.0.1:9", R_P)
            .await
            .unwrap_err();
        assert!(matches!(err, SiteError::Unreachable(_)), "{err}");
    }

    /// Section 6a: after the owner's clear, a height is closed and the
    /// website refuses any statement there, with reason `closed-height`.
    #[tokio::test]
    async fn a_closed_height_is_told_apart() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/statement")
            .with_status(422)
            .with_body(r#"{"error":"closed-height"}"#)
            .create_async()
            .await;
        let err = post_statement(&client().unwrap(), &server.url(), R_P)
            .await
            .unwrap_err();
        assert!(err.is_closed_height(), "{err}");
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
    async fn pending_reads_the_websites_list_and_its_dissent_flag() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(format!(
                r#"{{"version":1,"chain":"regtest","statements":[{{"statement_hash":"{H}","height":100,"block_hash":"bd23","manifest_hex":"02","signers":["03"],"operators":["producer"],"file":"stored","first_seen":"2026-10-01T12:00:00.000Z","confirmed":false,"disputed":true,"dissent":false}},{{"statement_hash":"{}","height":100,"block_hash":"44","manifest_hex":"02","signers":["02"],"operators":["confirmer"],"file":"none","first_seen":"2026-10-01T12:10:00.000Z","confirmed":false,"disputed":true,"dissent":true}}]}}"#,
                "22".repeat(32)
            ))
            .create_async()
            .await;
        let p = get_pending(&client().unwrap(), &server.url(), "regtest")
            .await
            .unwrap();
        assert_eq!(p.statements.len(), 2);
        assert_eq!(p.statements[0].operators, vec!["producer".to_string()]);
        assert_eq!(
            p.statements.iter().map(|s| s.dissent).collect::<Vec<_>>(),
            vec![false, true]
        );
    }

    async fn latest_answering(status: usize, body: &[u8]) -> Result<Option<Latest>, SiteError> {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/latest?chain=regtest")
            .with_status(status)
            .with_body(body)
            .create_async()
            .await;
        get_latest(&client().unwrap(), &server.url(), "regtest").await
    }

    /// The three answers `latest` gives (section 6a), read by the core
    /// plan's own parser: the pointer, the disputed heights and nothing
    /// else, or nothing confirmed.
    #[tokio::test]
    async fn latest_reads_a_pointer_a_dispute_and_nothing() {
        let pointer = latest_answering(200, POINTER).await;
        assert!(
            matches!(pointer, Ok(Some(Latest::Confirmed(_)))),
            "{pointer:?}"
        );
        assert_eq!(
            latest_answering(200, br#"{"disputed":[233800,233900]}"#).await,
            Ok(Some(Latest::Disputed(vec![233_800, 233_900])))
        );
        assert_eq!(
            latest_answering(404, br#"{"version":1,"confirmed":null}"#).await,
            Ok(None)
        );
        assert!(
            matches!(
                latest_answering(200, br#"{"disputed":[1],"height":100}"#).await,
                Err(SiteError::Unreadable(_))
            ),
            "a dispute answer says nothing else"
        );
        assert!(matches!(
            latest_answering(503, br#"{"error":"no store"}"#).await,
            Err(SiteError::Refused { status: 503, .. })
        ));
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
//! The app's side of the snapshot meeting point on easybtx.com (sections 6
//! and 6a of docs/decisions/2026-09-29-every-node-starts-near-the-tip.md): a
//! producer sends its statement and file, a confirmer reads what waits and
//! sends back one signature or a dissent. The routes live in the EasyBTX
//! repository (`site/src/lib/snapshotRoutes.mjs`); this is their client,
//! shared by the app and `btx-confirmer`.
//!
//! NOTHING the website answers is trusted. A producer only learns whether its
//! upload was kept; a confirmer re-reads every statement from its bytes and
//! takes it through its own verdict (`crate::statement_check`) before it
//! signs or dissents, and never signs an entry the website or its own file
//! fields mark as a dissent. Loading reads `latest` through
//! `crate::attested_snapshot`, not through this module; [`get_latest`] is for
//! reports and the rehearsal.
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
    /// A dissent (section 6a): never signed. The confirmer also reads it
    /// from the statement's own zero file fields, so a missing flag changes
    /// nothing.
    #[serde(default)]
    pub dissent: bool,
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

/// What `GET /api/snapshots/latest` says with HTTP 200, as the core plan
/// reads it (`attested_snapshot::parse_latest`): the pointer, or the heights
/// the operators disagree about. The app never shows the pointer's names: it
/// names only operators whose signatures it verified itself (core plan).
pub use crate::attested_snapshot::Latest;

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

impl SiteError {
    /// The website refused a statement at a height the owner closed after a
    /// disagreement (section 6a): nothing sent there is ever taken again.
    pub fn is_closed_height(&self) -> bool {
        matches!(self, SiteError::Refused { status: 422, reason } if reason.contains("closed-height"))
    }
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

/// Send a manifest: a new statement, co-signatures for one already there, or
/// a dissent.
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

/// What `latest` answers on `chain` now: `None` when nothing is confirmed
/// (HTTP 404), else the pointer or, while a dispute stands, the disputed
/// heights and nothing else (website plan).
pub async fn get_latest(
    client: &reqwest::Client,
    base: &str,
    chain: &str,
) -> Result<Option<Latest>, SiteError> {
    let resp = client
        .get(format!("{base}/api/snapshots/latest?chain={chain}"))
        .send()
        .await
        .map_err(|e| SiteError::Unreachable(e.to_string()))?;
    let status = resp.status().as_u16();
    let body = resp
        .bytes()
        .await
        .map_err(|e| SiteError::Unreachable(e.to_string()))?;
    match status {
        200 => crate::attested_snapshot::parse_latest(&body)
            .map(Some)
            .map_err(SiteError::Unreadable),
        404 => Ok(None),
        _ => Err(SiteError::Refused {
            status,
            reason: serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["error"].as_str().map(str::to_string))
                .unwrap_or_default(),
        }),
    }
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
        let put: Result<serde_json::Value, SiteError> = answer(
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
        .await;
        match put {
            Ok(_) => {}
            Err(SiteError::Refused { status: 409, .. }) => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    match answer(
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
    {
        Ok(stored) => Ok(Some(stored)),
        Err(SiteError::Refused { status: 409, .. }) => Ok(None),
        Err(e) => Err(e),
    }
}
````

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot_site::`
Expected: `8 passed; 0 failed` (derived, not run).

- [ ] **Step 5: Format, lint, commit**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cd ../..
git add crates/btx-core/src/snapshot_site.rs crates/btx-core/src/lib.rs
git commit -m "core: the client of easybtx.com's snapshot routes, disputes and dissents included" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 4: Producers export on the grid, wait per height until 144 deep, and check before they offer or send (`snapshot_serve.rs`, `snapshot_producer.rs`)

(Was Task 5. The one-cycle-at-a-time `run_cycle` goes: with 144 deep and a grid of 100 the next grid height comes before the last export is deep enough, so exports go into a waiting set and each keeper tick looks at every waiting pair once.)

**Files:**
- Modify: `crates/btx-core/src/snapshot_serve.rs` (`CONFIRMATIONS_REQUIRED` is 144, `EXPORT_GRID`, `KEEP_PAIRS` 4, `MATURE_DEADLINE` 6 h, `MATURE_POLL` and `REFRESH_BLOCKS` removed, `export_due`, `blocks_to_grid`, `Waiting` and `waiting.json`, `export_at_tip`, `BeforeOffer`, `OfferCheck`, `NoChecks`, `MatureOutcome`, `mature_step`; `run_cycle` and `refresh_due` removed; tests)
- Create: `crates/btx-core/src/snapshot_producer.rs`
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: Tasks 1 to 3; `confirmed_snapshot::{SNAPSHOT_GRID, SNAPSHOT_DEPTH}`; `known_invalid::{refuse, refuse_held_in_order, Refusal, HeldBranch}` (exist); `operators::{for_chain, parse_key, Chain}`.
- Produces (Tasks 6, 9):
  - `snapshot_serve::{EXPORT_GRID, CONFIRMATIONS_REQUIRED, KEEP_PAIRS, MATURE_DEADLINE, WAITING_RECORD}`, `export_due(tip, grid, offered: Option<u64>, waiting: &[Waiting]) -> bool`, `blocks_to_grid(tip, grid) -> u64`
  - `pub struct Waiting { height, block_hash, txoutset_hash, exported_at }`, `load_waiting(&Path) -> Vec<Waiting>`, `save_waiting(&Path, &[Waiting])`
  - `pub async fn export_at_tip(rpc, dir, grid) -> Result<Waiting, String>`
  - `#[async_trait] pub trait BeforeOffer: Sync { async fn check(&self, rpc: &dyn Rpc, base_height: u64, manifest: &[u8]) -> Result<(), OfferCheck>; }`, `pub enum OfferCheck { Refused(String), TryLater(String) }`, `pub struct NoChecks`
  - `pub enum MatureOutcome { Waiting { height, confirmations, tip }, Dropped { height, why }, Offered(OfferRecord) }`, `pub async fn mature_step(rpc, dir, &Waiting, now: u64, deadline: Duration, &dyn BeforeOffer) -> Result<MatureOutcome, String>`
  - `snapshot_producer::{SUBMITTED_FILE, RESUBMIT_EVERY}` (600 s), `pub struct Submitted { height, statement_hash, at }`, `load_submitted(&Path)`, `save_submitted(&Path, &Submitted)`, `submission_due(&OfferRecord, Option<&Submitted>, grid) -> bool`, `key_is_listed(pubkey_hex, genesis, regtest_env) -> bool`, `pub async fn check_before_send(&dyn Rpc, manifest: &[u8], diary_dir: &Path, &Holds<'_>, regtest_env: Option<&str>) -> Result<(), Mismatch>`, `pub struct ProducerChecks { diary_dir: PathBuf, holds: Holds<'static>, regtest_env: Option<String> }` (implements `BeforeOffer`), `pub async fn submit(&Client, site: &str, dir: &Path, height: u64) -> Result<Submitted, String>`

- [ ] **Step 1: Change the keeper's tests to the grid and the waiting set, and add the new ones**

In `crates/btx-core/src/snapshot_serve.rs`, in the test module:

*Test: export_due replaces refresh_due, and four pairs are kept.* Replace:

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
    fn waiting(height: u64) -> Waiting {
        Waiting {
            height,
            block_hash: String::new(),
            txoutset_hash: String::new(),
            exported_at: 0,
        }
    }

    #[test]
    fn an_export_is_due_only_on_the_grid_above_the_offered_base_and_once() {
        assert!(export_due(226_200, 100, None, &[]));
        assert!(export_due(226_300, 100, Some(226_200), &[]));
        assert!(
            export_due(226_300, 100, Some(226_100), &[waiting(226_200)]),
            "a second one while the first waits"
        );
        assert!(!export_due(226_201, 100, None, &[]), "one block past the grid");
        assert!(!export_due(226_200, 100, Some(226_200), &[]), "already offered");
        assert!(
            !export_due(226_200, 100, Some(226_300), &[]),
            "below the offered base"
        );
        assert!(
            !export_due(226_200, 100, None, &[waiting(226_200)]),
            "already waiting"
        );
        assert!(!export_due(0, 100, None, &[]));
        assert_eq!(EXPORT_GRID, 100);
        assert_eq!(CONFIRMATIONS_REQUIRED, 144);
        assert_eq!(blocks_to_grid(226_150, 100), 50);
        assert_eq!(blocks_to_grid(226_200, 100), 100);
    }

    #[test]
    fn four_pairs_are_kept_the_offered_one_the_one_before_and_two_waiting() {
        assert_eq!(KEEP_PAIRS, 4);
        assert_eq!(
            pairs_to_prune(
                &[226_000, 226_100, 226_200, 226_300, 226_400],
                KEEP_PAIRS,
                Some(226_200)
            ),
            vec![226_000]
        );
    }

    #[test]
    fn the_waiting_list_round_trips_and_a_broken_file_reads_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_waiting(dir.path()).is_empty());
        save_waiting(dir.path(), &[waiting(226_300), waiting(226_200)]).unwrap();
        assert_eq!(
            load_waiting(dir.path())
                .iter()
                .map(|w| w.height)
                .collect::<Vec<_>>(),
            vec![226_200, 226_300],
            "ascending"
        );
        std::fs::write(dir.path().join(WAITING_RECORD), b"{ not json").unwrap();
        assert!(load_waiting(dir.path()).is_empty());
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

*Test: the scripted node remembers each dump's height.* Replace:

````rust
        dumps: Mutex<u64>,
        offering: Mutex<bool>,
        tip: u64,
    }
````

with:

````rust
        dumps: Mutex<u64>,
        /// The base height of each dump, in order.
        dump_heights: Mutex<Vec<u64>>,
        /// The hash each dump based on, by height: two exports can wait at
        /// once.
        by_height: Mutex<std::collections::HashMap<u64, String>>,
        offering: Mutex<bool>,
        tip: u64,
    }
````

Replace:

````rust
                dumps: Mutex::new(0),
                offering: Mutex::new(false),
````

with:

````rust
                dumps: Mutex::new(0),
                dump_heights: Mutex::new(vec![226_200, 226_300, 226_400]),
                by_height: Mutex::new(Default::default()),
                offering: Mutex::new(false),
````

*Test: the scripted node exports on the grid.* Replace:

````rust
                    *self.canonical.lock().unwrap() = format!("hash-{n}");
                    json!({
                        "base_height": 226_140 + n - 1,
````

with:

````rust
                    *self.canonical.lock().unwrap() = format!("hash-{n}");
                    let height = self.dump_heights.lock().unwrap()[(n - 1) as usize];
                    self.by_height
                        .lock()
                        .unwrap()
                        .insert(height, format!("hash-{n}"));
                    json!({
                        "base_height": height,
````

*Test: the scripted node answers per height.* Replace:

````rust
                "getblockhash" => match self.sibling_at_verify.lock().unwrap().clone() {
                    Some(s) => json!(s),
                    None => json!(self.canonical.lock().unwrap().clone()),
                },
````

with:

````rust
                "getblockhash" => match self.sibling_at_verify.lock().unwrap().clone() {
                    Some(s) => json!(s),
                    None => {
                        let h = params[0].as_u64().unwrap_or(0);
                        let dumped = self.by_height.lock().unwrap().get(&h).cloned();
                        json!(dumped.unwrap_or_else(|| self.canonical.lock().unwrap().clone()))
                    }
                },
````

Replace:

````rust
                    json!({
                        "block_hash": self.canonical.lock().unwrap().clone(),
                        "height": h,
````

with:

````rust
                    let dumped = self.by_height.lock().unwrap().get(&h).cloned();
                    json!({
                        "block_hash": dumped.unwrap_or_else(|| self.canonical.lock().unwrap().clone()),
                        "height": h,
````

*Tests: the cycle's tests become the waiting pair's.* Replace everything from the line `    fn fast() -> (Duration, Duration) {` up to, not including, the line `    fn recorded(dir: &Path, height: u64, hash: &str) -> OfferRecord {` (`fast`, `a_cycle_waits_for_ten_confirmations_then_swaps_and_prunes`, `an_orphaned_base_is_dumped_again_and_the_first_is_never_offered`, `a_base_that_leaves_the_chain_at_the_last_moment_is_not_offered`, `a_stopped_role_aborts_the_wait_and_offers_nothing` and `a_base_that_never_matures_times_out_without_offering`, lines 1229 to 1371 on `origin/main`) with:

````rust
    #[tokio::test]
    async fn an_exported_pair_waits_until_it_is_144_deep_then_is_offered_and_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        // Four old pairs already on disk: once the new one is offered, four
        // are kept and the oldest goes.
        for h in [225_900u64, 226_000, 226_100, 226_150] {
            std::fs::write(d.join(snapshot_file_name(h)), b"old").unwrap();
            std::fs::write(d.join(manifest_file_name(h)), b"old").unwrap();
        }
        let node = ScriptedNode::new(&[3, 143, 144]);
        let w = export_at_tip(&node, d, 100).await.unwrap();
        assert_eq!((w.height, w.block_hash.as_str()), (226_200, "hash-1"));
        assert!(
            d.join(snapshot_file_name(226_200)).is_file(),
            "in place at once, waiting"
        );
        assert!(!d.join(STAGING_DAT).exists());
        assert_eq!(load_waiting(d), vec![w.clone()]);
        for want in [3u64, 143] {
            let out = mature_step(&node, d, &w, w.exported_at, MATURE_DEADLINE, &NoChecks)
                .await
                .unwrap();
            assert!(
                matches!(out, MatureOutcome::Waiting { height: 226_200, confirmations, .. } if confirmations == want),
                "{out:?}"
            );
        }
        assert_eq!(node.count("offerattestedutxosnapshot"), 0, "not before 144");
        let out = mature_step(&node, d, &w, w.exported_at, MATURE_DEADLINE, &NoChecks)
            .await
            .unwrap();
        let MatureOutcome::Offered(record) = out else {
            panic!("{out:?}")
        };
        assert_eq!(record.height, 226_200);
        assert_eq!(record.block_hash, "hash-1");
        assert_eq!(record.sha256, sha256_hex(b"snapshot bytes 1"));
        assert_eq!(
            load_record(d),
            Some(record.clone()),
            "recorded after the offer"
        );
        assert!(load_waiting(d).is_empty());
        assert!(
            !d.join(snapshot_file_name(225_900)).exists(),
            "oldest pair pruned"
        );
        for kept in [226_000u64, 226_100, 226_150, 226_200] {
            assert!(d.join(snapshot_file_name(kept)).is_file(), "{kept} kept");
        }
        // Offered exactly once, after a withdraw, and only at 144.
        assert_eq!(node.count("offerattestedutxosnapshot"), 1);
        assert_eq!(node.count("withdrawattestedutxosnapshot"), 1);
        assert_eq!(node.count("getblockheader"), 3, "one read per look");
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
    }

    /// Section 4: 144 is more than 100, so the next grid height comes while
    /// the last export still waits. Both wait, each is offered in turn.
    #[tokio::test]
    async fn two_exports_wait_at_once_and_each_is_offered_in_turn() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let node = ScriptedNode::new(&[50, 144, 44, 144]);
        let first = export_at_tip(&node, d, 100).await.unwrap();
        let out = mature_step(&node, d, &first, first.exported_at, MATURE_DEADLINE, &NoChecks)
            .await
            .unwrap();
        assert!(
            matches!(out, MatureOutcome::Waiting { confirmations: 50, .. }),
            "{out:?}"
        );
        let second = export_at_tip(&node, d, 100).await.unwrap();
        assert_eq!(
            load_waiting(d).iter().map(|w| w.height).collect::<Vec<_>>(),
            vec![226_200, 226_300]
        );
        let out = mature_step(&node, d, &first, first.exported_at, MATURE_DEADLINE, &NoChecks)
            .await
            .unwrap();
        assert!(
            matches!(&out, MatureOutcome::Offered(r) if r.height == 226_200 && r.block_hash == "hash-1"),
            "{out:?}"
        );
        let out = mature_step(&node, d, &second, second.exported_at, MATURE_DEADLINE, &NoChecks)
            .await
            .unwrap();
        assert!(
            matches!(out, MatureOutcome::Waiting { height: 226_300, confirmations: 44, .. }),
            "{out:?}"
        );
        let out = mature_step(&node, d, &second, second.exported_at, MATURE_DEADLINE, &NoChecks)
            .await
            .unwrap();
        assert!(
            matches!(&out, MatureOutcome::Offered(r) if r.height == 226_300 && r.block_hash == "hash-2"),
            "{out:?}"
        );
        assert_eq!(load_record(d).unwrap().height, 226_300);
        assert!(load_waiting(d).is_empty());
        assert_eq!(node.count("offerattestedutxosnapshot"), 2);
    }

    #[tokio::test]
    async fn a_base_that_leaves_the_chain_while_it_waits_is_dropped() {
        // The 2026-09-20 shape: the base is orphaned within a block. A second
        // dump would base on the tip, off the grid, so none is made.
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let node = ScriptedNode::new(&[2, -1]);
        let w = export_at_tip(&node, d, 100).await.unwrap();
        let out = mature_step(&node, d, &w, w.exported_at, MATURE_DEADLINE, &NoChecks)
            .await
            .unwrap();
        assert!(matches!(out, MatureOutcome::Waiting { .. }), "{out:?}");
        let out = mature_step(&node, d, &w, w.exported_at, MATURE_DEADLINE, &NoChecks)
            .await
            .unwrap();
        assert!(
            matches!(&out, MatureOutcome::Dropped { height: 226_200, why } if why.contains("left the chain")),
            "{out:?}"
        );
        assert!(load_waiting(d).is_empty());
        assert!(!d.join(snapshot_file_name(226_200)).exists(), "its files are gone");
        assert_eq!(node.count("dumptxoutsetattested"), 1, "no second dump");
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
        assert_eq!(load_record(d), None);
    }

    #[tokio::test]
    async fn an_export_that_landed_off_the_grid_is_not_kept() {
        // The tip moved between the keeper's look and the dump.
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let node = ScriptedNode::new(&[144]);
        *node.dump_heights.lock().unwrap() = vec![226_201];
        let err = export_at_tip(&node, d, 100).await.unwrap_err();
        assert!(err.contains("the tip moved to 226201"), "{err}");
        assert!(load_waiting(d).is_empty());
        assert!(!d.join(STAGING_DAT).exists());
        assert!(!d.join(snapshot_file_name(226_201)).exists());
    }

    #[tokio::test]
    async fn a_pair_the_checks_refuse_is_dropped_and_the_old_offer_stays() {
        struct Refuses(Mutex<Vec<(u64, Vec<u8>)>>);
        #[async_trait]
        impl BeforeOffer for Refuses {
            async fn check(
                &self,
                _: &dyn Rpc,
                base: u64,
                manifest: &[u8],
            ) -> Result<(), OfferCheck> {
                self.0.lock().unwrap().push((base, manifest.to_vec()));
                Err(OfferCheck::Refused(
                    "coin count differs from the diary (diary 7, statement 8)".into(),
                ))
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let node = ScriptedNode::new(&[144]);
        let hook = Refuses(Mutex::new(Vec::new()));
        let w = export_at_tip(&node, d, 100).await.unwrap();
        let out = mature_step(&node, d, &w, w.exported_at, MATURE_DEADLINE, &hook)
            .await
            .unwrap();
        assert_eq!(
            out,
            MatureOutcome::Dropped {
                height: 226_200,
                why: "not offered: coin count differs from the diary (diary 7, statement 8)"
                    .into()
            }
        );
        assert_eq!(
            hook.0.lock().unwrap().clone(),
            vec![(226_200, vec![0x02, 0x01, 0x46, 0xfa])],
            "the hook saw the export's own manifest"
        );
        assert_eq!(node.count("withdrawattestedutxosnapshot"), 0);
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
        assert_eq!(load_record(d), None);
        assert!(load_waiting(d).is_empty());
        assert!(!d.join(snapshot_file_name(226_200)).exists());
    }

    #[tokio::test]
    async fn a_check_that_cannot_run_yet_keeps_the_pair_waiting() {
        struct Later;
        #[async_trait]
        impl BeforeOffer for Later {
            async fn check(&self, _: &dyn Rpc, _: u64, _: &[u8]) -> Result<(), OfferCheck> {
                Err(OfferCheck::TryLater(
                    "the node did not answer: getblockhash 0".into(),
                ))
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let node = ScriptedNode::new(&[144]);
        let w = export_at_tip(&node, d, 100).await.unwrap();
        let err = mature_step(&node, d, &w, w.exported_at, MATURE_DEADLINE, &Later)
            .await
            .unwrap_err();
        assert_eq!(
            err,
            "block 226200 waits: the node did not answer: getblockhash 0"
        );
        assert_eq!(load_waiting(d), vec![w]);
        assert!(d.join(snapshot_file_name(226_200)).is_file());
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
    }

    #[tokio::test]
    async fn a_base_that_leaves_the_chain_at_the_last_moment_is_not_offered() {
        // 144 deep reported, but by the final re-verify the active chain has
        // a sibling at that height. Depth and canonicality are read
        // separately, and the last word is the latter.
        let dir = tempfile::tempdir().unwrap();
        let node = ScriptedNode::new(&[144]);
        let w = export_at_tip(&node, dir.path(), 100).await.unwrap();
        *node.sibling_at_verify.lock().unwrap() = Some("some-sibling".into());
        let out = mature_step(&node, dir.path(), &w, w.exported_at, MATURE_DEADLINE, &NoChecks)
            .await
            .unwrap();
        assert!(
            matches!(&out, MatureOutcome::Dropped { why, .. } if why.contains("left the active chain")),
            "{out:?}"
        );
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
        assert_eq!(load_record(dir.path()), None);
    }

    #[tokio::test]
    async fn a_base_not_deep_enough_after_six_hours_is_dropped() {
        assert_eq!(MATURE_DEADLINE, Duration::from_secs(6 * 60 * 60));
        let dir = tempfile::tempdir().unwrap();
        let node = ScriptedNode::new(&[3]);
        let w = export_at_tip(&node, dir.path(), 100).await.unwrap();
        let early = mature_step(
            &node,
            dir.path(),
            &w,
            w.exported_at + 5 * 3600,
            MATURE_DEADLINE,
            &NoChecks,
        )
        .await
        .unwrap();
        assert!(matches!(early, MatureOutcome::Waiting { .. }), "{early:?}");
        let late = mature_step(
            &node,
            dir.path(),
            &w,
            w.exported_at + 6 * 3600 + 1,
            MATURE_DEADLINE,
            &NoChecks,
        )
        .await
        .unwrap();
        assert!(
            matches!(&late, MatureOutcome::Dropped { why, .. } if why.contains("144 blocks deep after 6 hours")),
            "{late:?}"
        );
        assert_eq!(node.count("offerattestedutxosnapshot"), 0);
    }

````

- [ ] **Step 2: Write the producer's failing tests**

Create `crates/btx-core/src/snapshot_producer.rs` with only the test module:

````rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::diary::{Diary, DiaryEntry};
    use crate::fake_node::{chainstates_plain_assumeutxo, FakeNode};
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

    /// The node that exported the spike's statement at 100, 144 blocks later,
    /// with its diary.
    fn producer() -> (FakeNode, tempfile::TempDir) {
        let n = FakeNode::new(REGTEST_GENESIS, 243);
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
    async fn a_producer_sends_nothing_before_144_deep() {
        let (n, dir) = producer();
        n.with(|s| {
            s.chain.remove(&243);
        });
        assert_eq!(
            check_before_send(&n, R_P, dir.path(), &Holds::none(), None).await,
            Err(Mismatch::TooShallow {
                have: 143,
                need: 144
            })
        );
    }

    /// Section 4: a producer on a snapshot chainstate it has not finished
    /// checking sends nothing, and changes nothing on its node either.
    #[tokio::test]
    async fn a_producer_on_an_unvalidated_snapshot_chainstate_sends_nothing() {
        let (n, dir) = producer();
        n.with(|s| {
            s.chainstates = Some(chainstates_plain_assumeutxo());
            s.chain.insert(50, ROOT.into());
        });
        let holds = Holds {
            invalid: &[],
            held: HOLD_AT_50,
        };
        assert_eq!(
            check_before_send(&n, R_P, dir.path(), &holds, None).await,
            Err(Mismatch::Unvalidated)
        );
        assert_eq!(n.count("invalidateblock"), 0);
    }

    #[tokio::test]
    async fn a_held_block_is_refused_first_and_a_base_above_it_is_not_sent() {
        // The held root is at 50 on this node's chain: refusing it takes the
        // chain down to 49, and the base at 100 is not even on it.
        let (n, dir) = producer();
        n.with(|s| {
            s.chain.insert(50, ROOT.into());
        });
        let holds = Holds {
            invalid: &[],
            held: HOLD_AT_50,
        };
        let got = check_before_send(&n, R_P, dir.path(), &holds, None).await;
        assert_eq!(got, Err(Mismatch::TooShallow { have: 0, need: 144 }));
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

    /// The keeper's hook: a mismatch drops the pair, a node that is not deep
    /// enough yet or does not answer keeps it waiting.
    #[tokio::test]
    async fn the_keepers_hook_drops_a_mismatch_and_waits_on_a_slow_node() {
        let (n, dir) = producer();
        let hook = ProducerChecks {
            diary_dir: dir.path().to_path_buf(),
            holds: Holds::none(),
            regtest_env: None,
        };
        assert_eq!(hook.check(&n, 100, R_P).await, Ok(()));
        n.with(|s| {
            s.chain.remove(&243);
        });
        assert!(
            matches!(hook.check(&n, 100, R_P).await, Err(OfferCheck::TryLater(e)) if e.contains("143 blocks deep")),
        );
        let (n, dir) = producer();
        n.with(|s| s.chain.insert(100, "44".repeat(32)));
        let hook = ProducerChecks {
            diary_dir: dir.path().to_path_buf(),
            holds: Holds::none(),
            regtest_env: None,
        };
        assert!(matches!(
            hook.check(&n, 100, R_P).await,
            Err(OfferCheck::Refused(_))
        ));
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
        assert!(submission_due(&record(226_200), None, 100));
        assert!(!submission_due(&record(226_200), Some(&done), 100));
        assert!(submission_due(&record(226_300), Some(&done), 100));
        assert!(
            !submission_due(&record(226_150), None, 100),
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

    /// Section 6a: after the owner's clear, the website refuses the height
    /// for good. The pair is written down as done so it is not sent again.
    #[tokio::test]
    async fn a_closed_height_is_not_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(manifest_file_name(100)), R_P).unwrap();
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/api/snapshots/statement")
            .with_status(422)
            .with_body(r#"{"error":"closed-height"}"#)
            .create_async()
            .await;
        let client = crate::snapshot_site::client().unwrap();
        let err = submit(&client, &server.url(), dir.path(), 100)
            .await
            .unwrap_err();
        assert!(err.contains("closed block 100"), "{err}");
        let record = OfferRecord {
            height: 100,
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
        assert!(!submission_due(
            &record,
            load_submitted(dir.path()).as_ref(),
            100
        ));
    }
}
````

In `crates/btx-core/src/lib.rs`, before `pub mod snapshot_serve;` add `pub mod snapshot_producer;`.

- [ ] **Step 3: Run them to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot_serve:: snapshot_producer::`
Expected: compile errors, among them `` cannot find function `export_due` in this scope ``, `` cannot find function `export_at_tip` in this scope ``, `` cannot find trait `BeforeOffer` in this scope ``, `` cannot find value `NoChecks` in this scope ``, `` cannot find function `check_before_send` in this scope ``.

- [ ] **Step 4: Export on the grid into a waiting set, and let the hook have the last word**

In `crates/btx-core/src/snapshot_serve.rs`, above the test module:

*The module header's second bullet.* Replace:

````rust
//!   within 40 seconds and had been offered. So the cycle is dump, WAIT for
//!   [`CONFIRMATIONS_REQUIRED`] confirmations, re-verify the base is still on
//!   the active chain, THEN offer. Upstream parks reorgs deeper than 6; ten
//!   is that with margin. A base that reaches -1 confirmations is dumped
//!   again. The old offer stays live throughout, so serving never stops.
````

with:

````rust
//!   within 40 seconds and had been offered. So a pair is exported only when
//!   the tip is on the grid ([`EXPORT_GRID`]), WAITS until its base is
//!   [`CONFIRMATIONS_REQUIRED`] (144) deep, is re-verified on the active
//!   chain and checked by the producer ([`BeforeOffer`]), THEN offered.
//!   Since 144 is more than the grid, two exports can wait at once; every
//!   keeper tick looks at each once ([`mature_step`]). A base that reaches
//!   -1 confirmations is dropped, and the next export is at the next grid
//!   height: a second dump would base on today's tip, off the grid. The old
//!   offer stays live throughout, so serving never stops.
````

*The depth.* Replace:

````rust
/// Confirmations a base needs before it is offered. See the module header:
/// the dump bases on the 0-conf tip, siblings arrive every ~25 blocks, and
/// upstream's own reorg park depth is 6.
pub const CONFIRMATIONS_REQUIRED: u64 = 10;
````

with:

````rust
/// How deep a base must be on this node's active chain before it is offered
/// or sent (section 4 of docs/decisions/2026-09-29-every-node-starts-near-
/// the-tip.md): `confirmed_snapshot::SNAPSHOT_DEPTH`, 144, counted as
/// `getblockheader`'s `confirmations`. It keeps the blocks after a snapshot
/// inside the 288 that limited peers serve, and is twice the depth at which
/// the engine's archive profile alarms about a reorg (72).
pub const CONFIRMATIONS_REQUIRED: u64 = crate::confirmed_snapshot::SNAPSHOT_DEPTH as u64;
````

*The refresh constant becomes the grid.* Replace:

````rust
/// Re-export once the tip is this far past the offered base. About eleven
/// hours at the 45 blocks/h measured on 2026-09-21; an importer then catches
/// up at most this much, which a mirror does in minutes.
pub const REFRESH_BLOCKS: u64 = 500;
````

with:

````rust
/// Snapshots are exported where the tip is a multiple of this (section 2):
/// `confirmed_snapshot::SNAPSHOT_GRID`, 100. With 144 deep, the newest
/// confirmed snapshot is then 144 to 243 blocks behind the tip, so the
/// blocks after it stay within the 288 recent blocks almost every peer
/// serves; and 100 divides the engine's compiled heights (219,000, 228,000),
/// so every node's diary has the heights a snapshot can name.
pub const EXPORT_GRID: u64 = crate::confirmed_snapshot::SNAPSHOT_GRID as u64;
````

*Four pairs.* Replace:

````rust
/// Snapshot pairs kept on disk: the offered one and the one before it, so a
/// failed swap always has something the keeper can fall back to.
pub const KEEP_PAIRS: usize = 2;
````

with:

````rust
/// Snapshot pairs kept on disk: the offered one, the one before it (a failed
/// swap always has something to fall back to) and two waiting to be deep
/// enough. About 36 MB at today's 9 MB file.
pub const KEEP_PAIRS: usize = 4;
````

*Six hours, and no poll of its own.* Replace:

````rust
/// How often the maturation wait reads the base's confirmations. One block
/// is ~90 s, so anything faster only burns RPC.
pub const MATURE_POLL: Duration = Duration::from_secs(60);

/// How long a base may take to mature before the cycle gives up. Ninety
/// minutes is ~60 blocks; a base that has not reached ten confirmations by
/// then is on a chain that is not moving, and there is nothing to serve.
pub const MATURE_DEADLINE: Duration = Duration::from_secs(90 * 60);
````

with:

````rust
/// How long an exported pair may wait to be 144 deep before it is dropped.
/// 144 blocks take about three and a half hours at 40 an hour; a base that
/// is not that deep after six is on a chain that is barely moving. The
/// keeper looks at every waiting pair on each of its ticks.
pub const MATURE_DEADLINE: Duration = Duration::from_secs(6 * 60 * 60);
````

*The waiting record's name.* Replace:

````rust
pub const OFFER_RECORD: &str = "current-offer.json";
````

with:

````rust
pub const OFFER_RECORD: &str = "current-offer.json";
/// The pairs exported and not yet deep enough, next to the pairs. JSON, ours.
pub const WAITING_RECORD: &str = "waiting.json";
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
/// Is it time to export? Only when the tip is exactly on the grid, above the
/// base already offered (`None` when nothing ever was), and not already
/// waiting. A tip that moved past a grid height before this was asked waits
/// for the next one.
pub fn export_due(tip: u64, grid: u64, offered: Option<u64>, waiting: &[Waiting]) -> bool {
    tip > 0
        && grid > 0
        && tip.is_multiple_of(grid)
        && offered.is_none_or(|b| tip > b)
        && !waiting.iter().any(|w| w.height == tip)
}

/// Blocks until the tip reaches the next grid height, 1 to `grid`.
pub fn blocks_to_grid(tip: u64, grid: u64) -> u64 {
    grid - tip % grid
}
````

*The waiting record, after the offer record.* Replace:

````rust
pub fn save_record(dir: &Path, r: &OfferRecord) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(r)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    crate::fsx::atomic_write(&dir.join(OFFER_RECORD), json.as_bytes())
}
````

with:

````rust
pub fn save_record(dir: &Path, r: &OfferRecord) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(r)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    crate::fsx::atomic_write(&dir.join(OFFER_RECORD), json.as_bytes())
}

/// A pair exported on the grid and waiting to be deep enough. Written right
/// after the export; removed when the pair is offered or dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiting {
    pub height: u64,
    pub block_hash: String,
    pub txoutset_hash: String,
    /// Unix seconds.
    pub exported_at: u64,
}

/// The waiting pairs, ascending. A missing or broken file reads as none; the
/// files stay on disk and are pruned like any other pair.
pub fn load_waiting(dir: &Path) -> Vec<Waiting> {
    let mut all: Vec<Waiting> = std::fs::read(dir.join(WAITING_RECORD))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default();
    all.sort_by_key(|w| w.height);
    all
}

pub fn save_waiting(dir: &Path, all: &[Waiting]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(all).map_err(std::io::Error::other)?;
    crate::fsx::atomic_write(&dir.join(WAITING_RECORD), &json)
}

fn forget_waiting(dir: &Path, height: u64) {
    let mut all = load_waiting(dir);
    all.retain(|w| w.height != height);
    let _ = save_waiting(dir, &all);
}

/// A waiting pair that will never be offered: its files and its line go.
fn drop_waiting(dir: &Path, height: u64) {
    let _ = std::fs::remove_file(dir.join(snapshot_file_name(height)));
    let _ = std::fs::remove_file(dir.join(manifest_file_name(height)));
    forget_waiting(dir, height);
}
````

*The row's sentence while a pair waits.* Replace:

````rust
                "Snapshot at block {base} exported; waiting for it to mature, {confirmations} of \
                 {CONFIRMATIONS_REQUIRED} confirmations (tip {tip}). The previous one stays on offer."
````

with:

````rust
                "Snapshot at block {base} exported; waiting until it is {CONFIRMATIONS_REQUIRED} \
                 blocks deep, {confirmations} so far (tip {tip}). The previous one stays on offer."
````

*The cycle becomes an export and a look per waiting pair.* Replace everything from the line `/// One full cycle: dump, wait, verify, withdraw the old, offer the new,` through the `    Ok(record)` and `}` that end `run_cycle` (lines 665 to 775 on `origin/main`) with:

````rust
/// The last word before a deep-enough pair is offered or sent anywhere: the
/// producer's checks (`crate::snapshot_producer::ProducerChecks`).
#[async_trait::async_trait]
pub trait BeforeOffer: Sync {
    async fn check(&self, rpc: &dyn Rpc, base_height: u64, manifest: &[u8]) -> Result<(), OfferCheck>;
}

/// Why the hook said no.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferCheck {
    /// The pair failed a check: it is dropped, never offered or sent.
    Refused(String),
    /// The check could not run yet (the node did not answer, or is a block
    /// short): the pair waits for the next tick.
    TryLater(String),
}

/// No check beyond the canonical one.
pub struct NoChecks;

#[async_trait::async_trait]
impl BeforeOffer for NoChecks {
    async fn check(&self, _: &dyn Rpc, _: u64, _: &[u8]) -> Result<(), OfferCheck> {
        Ok(())
    }
}

/// Export the UTXO set at the tip into the waiting set. The dump always bases
/// on the tip, so a tip that moved off the grid before the dump ends here and
/// the next export is at the next grid height. The caller has checked that
/// the tip is on the grid ([`export_due`]) and that the node's chain rests on
/// its own checks (`crate::diary::gate_open`).
pub async fn export_at_tip(rpc: &dyn Rpc, dir: &Path, grid: u64) -> Result<Waiting, String> {
    let base = dump(rpc, dir)
        .await
        .map_err(|e| format!("export failed: {e}"))?;
    if grid == 0 || base.base_height % grid != 0 {
        let _ = std::fs::remove_file(dir.join(STAGING_DAT));
        let _ = std::fs::remove_file(dir.join(STAGING_MANIFEST));
        return Err(format!(
            "the tip moved to {} before the export; the next one is at the next multiple of {grid}",
            base.base_height
        ));
    }
    // Renaming is safe: the manifest embeds no path.
    std::fs::rename(dir.join(STAGING_DAT), dir.join(snapshot_file_name(base.base_height)))
        .map_err(|e| format!("rename: {e}"))?;
    std::fs::rename(
        dir.join(STAGING_MANIFEST),
        dir.join(manifest_file_name(base.base_height)),
    )
    .map_err(|e| format!("rename: {e}"))?;
    let w = Waiting {
        height: base.base_height,
        block_hash: base.base_hash,
        txoutset_hash: base.txoutset_hash,
        exported_at: now_unix(),
    };
    let mut all = load_waiting(dir);
    all.retain(|x| x.height != w.height);
    all.push(w.clone());
    save_waiting(dir, &all).map_err(|e| format!("recording the export: {e}"))?;
    Ok(w)
}

/// What one look at a waiting pair came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatureOutcome {
    /// Not deep enough yet.
    Waiting {
        height: u64,
        confirmations: u64,
        tip: u64,
    },
    /// Never offered, and its files are gone: `why` says what happened.
    Dropped { height: u64, why: String },
    /// Offered, recorded, the mirrors bounced and old pairs pruned.
    Offered(OfferRecord),
}

/// One look at one waiting pair, on every keeper tick: read how deep its base
/// is; once it is [`CONFIRMATIONS_REQUIRED`] deep, re-verify it on the active
/// chain, ask `before_offer`, withdraw the old offer, offer this one, record,
/// bounce, prune. A base that left the chain, a pair still not deep enough
/// once `deadline` has passed since `exported_at` (`now` in Unix seconds), a
/// pair `before_offer` refuses and a pair the engine will not offer are
/// dropped. `Err` is a read that failed, or a check that could not run yet;
/// the pair waits for the next tick.
pub async fn mature_step(
    rpc: &dyn Rpc,
    dir: &Path,
    w: &Waiting,
    now: u64,
    deadline: Duration,
    before_offer: &dyn BeforeOffer,
) -> Result<MatureOutcome, String> {
    let h = w.height;
    let dropped = |why: String| -> Result<MatureOutcome, String> {
        drop_waiting(dir, h);
        Ok(MatureOutcome::Dropped { height: h, why })
    };
    let c = confirmations(rpc, &w.block_hash)
        .await
        .map_err(|e| format!("block {h} waits: {e}"))?;
    if c < 0 {
        return dropped(
            "it left the chain while it matured; the next export is at the next grid height"
                .into(),
        );
    }
    if (c as u64) < CONFIRMATIONS_REQUIRED {
        if now.saturating_sub(w.exported_at) > deadline.as_secs() {
            return dropped(format!(
                "it was not {CONFIRMATIONS_REQUIRED} blocks deep after {} hours",
                deadline.as_secs() / 3600
            ));
        }
        let tip = rpc
            .call("getblockcount", json!([]))
            .await
            .ok()
            .and_then(|v| v.as_u64())
            .unwrap_or(h);
        return Ok(MatureOutcome::Waiting {
            height: h,
            confirmations: c as u64,
            tip,
        });
    }
    // Final re-verify, immediately before anything is offered.
    match base_is_canonical(rpc, h, &w.block_hash).await {
        Ok(true) => {}
        Ok(false) => return dropped("it left the active chain just before the offer".into()),
        Err(e) => return Err(format!("could not re-verify block {h}: {e}")),
    }
    let dat = dir.join(snapshot_file_name(h));
    let man = dir.join(manifest_file_name(h));
    let manifest =
        std::fs::read(&man).map_err(|e| format!("reading the manifest of block {h}: {e}"))?;
    match before_offer.check(rpc, h, &manifest).await {
        Ok(()) => {}
        Err(OfferCheck::Refused(e)) => return dropped(format!("not offered: {e}")),
        Err(OfferCheck::TryLater(e)) => return Err(format!("block {h} waits: {e}")),
    }
    let (file_size, sha256) = sha256_of_file(&dat)
        .await
        .map_err(|e| format!("hashing the snapshot: {e}"))?;
    let (_, manifest_sha256) = sha256_of_file(&man)
        .await
        .map_err(|e| format!("hashing the manifest: {e}"))?;
    // Offering on top of a live offer is untested; withdraw first. The gap is
    // one RPC round trip, and a failure here leaves the previous record for
    // the keeper to re-offer from.
    let _ = withdraw(rpc).await;
    let offered = match offer(rpc, &dat, &man).await {
        Ok(o) => o,
        Err(e) => return dropped(format!("the engine did not offer it: {e}")),
    };
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
    save_record(dir, &record).map_err(|e| format!("recording the offer: {e}"))?;
    forget_waiting(dir, h);
    bounce_mirror_links(rpc).await;
    prune(dir, Some(record.height));
    Ok(MatureOutcome::Offered(record))
}
````

*A re-offer refused waits for the next export.* Replace:

````rust
    /// offered; the next refresh replaces it.
````

with:

````rust
    /// offered; the next export replaces it.
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
//! exports where its tip is a multiple of 100 (`crate::snapshot_serve`),
//! waits until that block is 144 deep on its own chain, and then, before the
//! pair is offered on the network or sent anywhere, checks it once more
//! against its own node:
//!
//! * `getchainstates` shows no chainstate the node has not finished checking
//!   (the validated gate, `crate::diary::gate_open`);
//! * every block the app refuses (`known_invalid`) is refused on this node,
//!   so the base cannot sit on a held branch;
//! * the one verdict (`crate::statement_check`) is `Sign`: the base is 144
//!   deep on the active chain, no refused block is below it, and its own
//!   diary entry for that height is on this chain and matches the statement
//!   field by field.
//!
//! A pair that fails any of it is neither offered nor sent. A producer whose
//! key is on the operator list then sends the statement and the file to
//! easybtx.com (`crate::snapshot_site`), and writes down that it did, so a
//! failed upload is tried again (every [`RESUBMIT_EVERY`]) and a finished one
//! never is. A height the website has closed after a disagreement is written
//! down the same way and not tried again.

use crate::confirmed_load::Holds;
use crate::confirmed_snapshot::{self as cs, ChainRules};
use crate::known_invalid::{self, Refusal};
use crate::operators::{self, Chain};
use crate::rpc::Rpc;
use crate::snapshot_serve::{
    manifest_file_name, snapshot_file_name, BeforeOffer, OfferCheck, OfferRecord,
    CONFIRMATIONS_REQUIRED,
};
use crate::statement_check::{check_against_node, Mismatch, Verdict};
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
/// as the fork check does every 30 seconds anyway), and only once the gate
/// is open; everything else reads.
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
    if !crate::diary::gate_open(rpc).await {
        return Err(Mismatch::Unvalidated);
    }
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
    match check_against_node(
        rpc,
        &m.statement,
        &rules,
        &diary,
        holds,
        CONFIRMATIONS_REQUIRED,
    )
    .await
    {
        Verdict::Sign => Ok(()),
        Verdict::Dissent(why) | Verdict::Neither(why) => Err(why),
    }
}

/// [`check_before_send`] as the keeper's [`BeforeOffer`] hook: a node that
/// did not answer, or a block not quite deep enough, waits; anything else
/// drops the pair.
pub struct ProducerChecks {
    pub diary_dir: PathBuf,
    pub holds: Holds<'static>,
    pub regtest_env: Option<String>,
}

#[async_trait::async_trait]
impl BeforeOffer for ProducerChecks {
    async fn check(&self, rpc: &dyn Rpc, _base_height: u64, manifest: &[u8]) -> Result<(), OfferCheck> {
        match check_before_send(
            rpc,
            manifest,
            &self.diary_dir,
            &self.holds,
            self.regtest_env.as_deref(),
        )
        .await
        {
            Ok(()) => Ok(()),
            Err(m @ (Mismatch::NodeUnanswered(_) | Mismatch::TooShallow { .. })) => {
                Err(OfferCheck::TryLater(m.to_string()))
            }
            Err(m) => Err(OfferCheck::Refused(m.to_string())),
        }
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Send the offered pair in `dir` to the website: the statement, then the
/// file unless the website has it. Writes [`SUBMITTED_FILE`] on success, and
/// also when the website has closed that height, so it is not tried again.
/// The caller has run [`check_before_send`] on this pair.
pub async fn submit(
    client: &reqwest::Client,
    site: &str,
    dir: &Path,
    height: u64,
) -> Result<Submitted, String> {
    let manifest = std::fs::read(dir.join(manifest_file_name(height)))
        .map_err(|e| format!("reading the manifest: {e}"))?;
    let reply = match crate::snapshot_site::post_statement(client, site, &manifest).await {
        Ok(r) => r,
        Err(e) if e.is_closed_height() => {
            let statement_hash = cs::parse(&manifest)
                .map(|m| m.statement.hash().display_hex())
                .unwrap_or_default();
            let _ = save_submitted(
                dir,
                &Submitted {
                    height,
                    statement_hash,
                    at: now_unix(),
                },
            );
            return Err(format!(
                "easybtx.com has closed block {height} after a disagreement, so this pair is not sent ({e})"
            ));
        }
        Err(e) => return Err(e.to_string()),
    };
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
Expected: `40 passed; 0 failed` (30 in `snapshot_serve`, 10 in `snapshot_producer`; derived, not run). Among them: `an_export_is_due_only_on_the_grid_above_the_offered_base_and_once`, `four_pairs_are_kept_the_offered_one_the_one_before_and_two_waiting`, `an_exported_pair_waits_until_it_is_144_deep_then_is_offered_and_pruned`, `two_exports_wait_at_once_and_each_is_offered_in_turn`, `a_base_that_leaves_the_chain_while_it_waits_is_dropped` (one dump, no offer), `an_export_that_landed_off_the_grid_is_not_kept`, `a_pair_the_checks_refuse_is_dropped_and_the_old_offer_stays` (no withdraw, no offer, no record), `a_base_not_deep_enough_after_six_hours_is_dropped`, `a_producer_sends_nothing_before_144_deep`, `a_producer_on_an_unvalidated_snapshot_chainstate_sends_nothing`, `a_producer_whose_diary_disagrees_sends_nothing`, `a_held_block_is_refused_first_and_a_base_above_it_is_not_sent`, `only_a_listed_key_submits_and_only_once_per_pair`, `a_closed_height_is_not_tried_again`. Two of the `snapshot_serve` tests take about 3 s each: the mirror bounce waits for the old sockets to close.

The Tauri crate does not build between this task and Task 6 (the keeper still calls `refresh_due` and `run_cycle`); run only the btx-core gates here.

- [ ] **Step 7: Format, lint, commit**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cargo test --locked --lib && cd ../..
git add crates/btx-core/src/snapshot_serve.rs crates/btx-core/src/snapshot_producer.rs crates/btx-core/src/lib.rs
git commit -m "core: producers export at every 100 blocks, wait until 144 deep, and check against their diary before offering or sending" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 5: Confirmers sign, dissent or wait (`snapshot_confirmer.rs`)

(Was Task 6. New here: the gate at the top of every round, the verdict instead of a refusal, the dissent, pending dissents never signed, the copy under `<datadir>/snapshot-confirmer/` handed over by its relative path, a key learned from the engine's first signature for `btx-confirmer`, and the new counts.)

**Files:**
- Create: `crates/btx-core/src/snapshot_confirmer.rs`
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: Tasks 1 to 4; `confirmed_snapshot::{parse, is_dissent, Manifest, Signed, ChainRules, confirming_operators, signature_is_valid}`, `operators::{hex_decode, parse_key, Chain}`, `node_api::MatmulTrustedStatus`.
- Produces (Tasks 6, 7, 9):
  - `CONFIRM_EVERY_SECS` (600), `WORK_DIR` (`snapshot-confirmer`), `DISSENTS_FILE` (`snapshot-dissents.json`), `UNVALIDATED_LINE`
  - `pub enum Outcome { Signed { height, statement_hash, operators }, Dissented { height, statement_hash, why: Mismatch, operators }, AlreadySigned { height, statement_hash }, AlreadyDissented { height, statement_hash }, Waiting { height, statement_hash, why: Mismatch }, Failed { height, statement_hash, error }, Closed { height, statement_hash } }` with `height()`, `statement_hash()` and `line()`
  - `pub enum Round { Unvalidated, Done(Vec<Outcome>) }`
  - `pub fn why_not(&MatmulTrustedStatus, our_key_hex: Option<&str>, genesis: &str, regtest_env: Option<&str>) -> Option<&'static str>`
  - `load_dissents(&Path) -> BTreeSet<u64>`, `save_dissents(&Path, &BTreeSet<u64>)`
  - `pub struct Confirmer<'a> { rpc: &'a dyn Rpc, client: &'a reqwest::Client, site: &'a str, state_dir: &'a Path, node_dir: &'a Path, our_key: OnceLock<[u8; 33]>, holds: Holds<'a>, regtest_env: Option<&'a str> }` with `async fn round(&self, Chain) -> Result<Round, String>` and `async fn confirm_one(&self, &PendingStatement, all: &[PendingStatement]) -> Option<Outcome>`
  - `pub struct Tally { rounds, signed, dissents, mismatches, gate_closed, failed, last_mismatch, last_error }` with `add(&Round, &mut HashSet<String>) -> Vec<String>` and `report() -> Vec<String>`
  - `pub struct NetworkReport { diary: Option<String>, diary_skipped: BTreeSet<u64>, confirmer: Tally, confirmer_off: Option<String>, seen: HashSet<String>, producer: Option<String>, producer_skipped: BTreeSet<u64> }` (`Default`) with `lines(diary_summary: Option<String>) -> Vec<String>`

- [ ] **Step 1: Write the failing tests**

Create `crates/btx-core/src/snapshot_confirmer.rs` with only the test module. The scripted node signs with key 7 the way the engine does, in its own data folder, and the website is mockito: a statement the diary agrees with gets exactly one signature (the POST body is checked byte for byte, and the engine got the relative path); a diary one coin off gets one signed dissent built from the diary, once; a dissent this operator already has on the website is not sent again; a statement this operator signed is left alone; a pending dissent is never signed, whether the website marks it or only its file fields do; a node on an unvalidated snapshot chainstate asks the website nothing; a listing whose hash lies and a manifest with a broken signature are not signed; an engine that signs with another key sends nothing; a key learned from the first signature, and an unlisted one, for `btx-confirmer`; mismatches, dissents and gate-closed rounds are counted and logged once per run; and only a validating, signing, listed node confirms:

````rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::diary::{Diary, DiaryEntry};
    use crate::fake_node::{chainstates_plain_assumeutxo, FakeNode};
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

    fn entry(coins: u64) -> DiaryEntry {
        DiaryEntry {
            height: 100,
            block_hash: R_BLOCK.into(),
            hash_serialized: R_UTXO.into(),
            coins,
            chain_tx: 101,
            recorded_at: 0,
        }
    }

    /// Our node: validating, 144 blocks past 100, key 7, its data folder and
    /// its diary in `dir`, the diary at 100 with `coins_in_diary` coins.
    fn setup(coins_in_diary: u64) -> (FakeNode, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let n = FakeNode::new(REGTEST_GENESIS, 243);
        n.with(|s| {
            s.chain.insert(100, R_BLOCK.into());
            s.replay_context = Some(cs::REGTEST_REPLAY_CONTEXT.into());
            s.signer = Some(key(7));
            s.datadir = Some(dir.path().to_path_buf());
        });
        let mut d = Diary::new(REGTEST_GENESIS);
        d.record(entry(coins_in_diary));
        crate::diary::save(dir.path(), &d).unwrap();
        let env = format!(
            "producer={P};confirmer={}",
            operators::hex(&pubkey(&key(7)))
        );
        (n, dir, env)
    }

    fn listed(manifest: &[u8], hash: &str, dissent: bool, ops: &[&str]) -> String {
        format!(
            r#"{{"statement_hash":"{hash}","height":100,"block_hash":"{R_BLOCK}","manifest_hex":"{}","signers":[],"operators":{},"file":"stored","first_seen":"2026-10-01T12:00:00.000Z","confirmed":false,"disputed":false,"dissent":{dissent}}}"#,
            operators::hex(manifest),
            serde_json::to_string(ops).unwrap()
        )
    }

    fn pending_json(entries: &[String]) -> String {
        format!(
            r#"{{"version":1,"chain":"regtest","statements":[{}]}}"#,
            entries.join(",")
        )
    }

    fn signed_by(sk: &SigningKey, st: cs::Statement) -> Vec<u8> {
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

    /// The manifest this node should send for the spike's statement: the
    /// statement and its one signature.
    fn ours_only(sk: &SigningKey) -> Vec<u8> {
        signed_by(sk, cs::parse(R_P).unwrap().statement)
    }

    /// The dissent this node should send when its diary says `coins`.
    fn dissent_by(sk: &SigningKey, coins: u64) -> Vec<u8> {
        let rules = ChainRules::for_statement(&cs::parse(R_P).unwrap().statement, None).unwrap();
        signed_by(
            sk,
            crate::statement_check::dissent_from(&entry(coins), &rules).unwrap(),
        )
    }

    async fn round(
        n: &FakeNode,
        dir: &Path,
        env: &str,
        known: Option<[u8; 33]>,
        server: &mockito::ServerGuard,
    ) -> Round {
        confirmer(n, dir, env, known, server)
            .round(Chain::Regtest)
            .await
            .unwrap()
    }

    fn confirmer<'a>(
        n: &'a FakeNode,
        dir: &'a Path,
        env: &'a str,
        known: Option<[u8; 33]>,
        server: &'a mockito::ServerGuard,
    ) -> Confirmer<'a> {
        let client = Box::leak(Box::new(site::client().unwrap()));
        let url = Box::leak(server.url().into_boxed_str());
        Confirmer {
            rpc: n,
            client,
            site: url,
            state_dir: dir,
            node_dir: dir,
            our_key: known.map(OnceLock::from).unwrap_or_default(),
            holds: Holds::none(),
            regtest_env: Some(env),
        }
    }

    fn done(r: Round) -> Vec<Outcome> {
        match r {
            Round::Done(o) => o,
            Round::Unvalidated => panic!("the gate was closed"),
        }
    }

    #[tokio::test]
    async fn a_statement_the_diary_agrees_with_gets_this_nodes_one_signature() {
        let (n, dir, env) = setup(101);
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(&[listed(R_P, H, false, &["producer"])]))
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
        let v = done(round(&n, dir.path(), &env, Some(pubkey(&key(7))), &server).await);
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Outcome::Signed {
                height: 100,
                statement_hash: H.into(),
                operators: vec!["producer".into(), "confirmer".into()]
            }]
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 1);
        assert_eq!(
            n.params("signutxosnapshotmanifest")[0][0],
            json!(format!("snapshot-confirmer/{H}.manifest")),
            "the engine gets the path relative to its data folder"
        );
        assert_eq!(
            std::fs::read_dir(dir.path().join(WORK_DIR)).unwrap().count(),
            0,
            "the copy is gone"
        );
    }

    /// Section 5, check 4, and section 6a: a chain field differs from the
    /// diary. The statement is never signed; a dissent built from the diary
    /// is, and it is sent once per height.
    #[tokio::test]
    async fn a_chain_field_that_differs_is_answered_with_one_signed_dissent() {
        let (n, dir, env) = setup(102);
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(&[listed(R_P, H, false, &["producer"])]))
            .create_async()
            .await;
        let post = server
            .mock("POST", "/api/snapshots/statement")
            .match_body(dissent_by(&key(7), 102))
            .with_body(r#"{"statement_hash":"d1","chain":"regtest","height":100,"signers":[],"operators":["confirmer"],"added":1,"file":"missing"}"#)
            .expect(1)
            .create_async()
            .await;
        let v = done(round(&n, dir.path(), &env, Some(pubkey(&key(7))), &server).await);
        assert_eq!(
            v,
            vec![Outcome::Dissented {
                height: 100,
                statement_hash: H.into(),
                why: Mismatch::Coins {
                    diary: 102,
                    statement: 101
                },
                operators: vec!["confirmer".into()]
            }]
        );
        assert_eq!(load_dissents(dir.path()), BTreeSet::from([100]));
        let again = done(round(&n, dir.path(), &env, Some(pubkey(&key(7))), &server).await);
        assert_eq!(
            again,
            vec![Outcome::AlreadyDissented {
                height: 100,
                statement_hash: H.into()
            }]
        );
        post.assert_async().await;
        assert_eq!(n.count("signutxosnapshotmanifest"), 1, "only the dissent");
    }

    #[tokio::test]
    async fn a_dissent_this_operator_has_on_the_website_counts_as_sent() {
        let (n, dir, env) = setup(102);
        let theirs = dissent_by(&key(7), 102);
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(&[
                listed(R_P, H, false, &["producer"]),
                listed(&theirs, &"d1".repeat(32), true, &["confirmer"]),
            ]))
            .create_async()
            .await;
        let post = server
            .mock("POST", "/api/snapshots/statement")
            .expect(0)
            .create_async()
            .await;
        let v = done(round(&n, dir.path(), &env, Some(pubkey(&key(7))), &server).await);
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Outcome::AlreadyDissented {
                height: 100,
                statement_hash: H.into()
            }],
            "the listed dissent itself is skipped, never signed"
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
            .with_body(pending_json(&[listed(R_PC, H, false, &["producer", "confirmer"])]))
            .create_async()
            .await;
        let v = done(
            round(
                &n,
                dir.path(),
                &env,
                Some(operators::parse_key(C).unwrap()),
                &server,
            )
            .await,
        );
        assert_eq!(
            v,
            vec![Outcome::AlreadySigned {
                height: 100,
                statement_hash: H.into()
            }]
        );
        assert_eq!(n.count("signutxosnapshotmanifest"), 0);
    }

    /// Section 6a: a dissent can never become a snapshot, so nobody signs
    /// one, whether the website marks it or only its zero file fields say so.
    #[tokio::test]
    async fn a_pending_dissent_is_never_signed() {
        let (n, dir, env) = setup(101);
        let theirs = dissent_by(&key(9), 99);
        let theirs_hash = cs::parse(&theirs).unwrap().statement.hash().display_hex();
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(&[
                listed(&theirs, &theirs_hash, true, &[]),
                listed(&theirs, &theirs_hash, false, &[]),
            ]))
            .create_async()
            .await;
        let post = server
            .mock("POST", "/api/snapshots/statement")
            .expect(0)
            .create_async()
            .await;
        let v = done(round(&n, dir.path(), &env, Some(pubkey(&key(7))), &server).await);
        post.assert_async().await;
        assert!(v.is_empty(), "{v:?}");
        assert_eq!(n.count("signutxosnapshotmanifest"), 0);
    }

    /// Section 5: while `getchainstates` shows a chainstate the node has not
    /// finished checking, a round stops before it asks easybtx.com anything.
    #[tokio::test]
    async fn a_node_on_an_unvalidated_snapshot_chainstate_asks_nothing_and_signs_nothing() {
        let (n, dir, env) = setup(101);
        n.with(|s| s.chainstates = Some(chainstates_plain_assumeutxo()));
        let mut server = mockito::Server::new_async().await;
        let asked = server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .expect(0)
            .create_async()
            .await;
        assert_eq!(
            round(&n, dir.path(), &env, Some(pubkey(&key(7))), &server).await,
            Round::Unvalidated
        );
        asked.assert_async().await;
        assert_eq!(n.count("signutxosnapshotmanifest"), 0);
    }

    #[tokio::test]
    async fn a_listing_that_lies_or_a_bad_signature_is_not_signed() {
        let (n, dir, env) = setup(101);
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(&[listed(R_P, &"00".repeat(32), false, &[])]))
            .create_async()
            .await;
        let v = done(round(&n, dir.path(), &env, Some(pubkey(&key(7))), &server).await);
        assert!(
            matches!(
                &v[0],
                Outcome::Waiting {
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
            .with_body(pending_json(&[listed(&bad, H, false, &[])]))
            .create_async()
            .await;
        let v = done(round(&n, dir.path(), &env, Some(pubkey(&key(7))), &server).await);
        assert!(
            matches!(&v[0], Outcome::Waiting { why: Mismatch::Unreadable(e), .. } if e.contains("is not valid")),
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
            .with_body(pending_json(&[listed(R_P, H, false, &["producer"])]))
            .create_async()
            .await;
        let post = server
            .mock("POST", "/api/snapshots/statement")
            .expect(0)
            .create_async()
            .await;
        let v = done(round(&n, dir.path(), &env, Some(pubkey(&key(7))), &server).await);
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Outcome::Failed {
                height: 100,
                statement_hash: H.into(),
                error: "the engine did not sign with this node's key".into()
            }]
        );
    }

    /// Section 5a: `btx-confirmer` does not read the key file, and no RPC
    /// names the local signer's key, so it learns the key from the engine's
    /// first signature; an unlisted key sends nothing.
    #[tokio::test]
    async fn btx_confirmer_learns_its_key_from_the_first_signature() {
        let (n, dir, env) = setup(101);
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(&[listed(R_P, H, false, &["producer"])]))
            .create_async()
            .await;
        server
            .mock("POST", "/api/snapshots/statement")
            .match_body(ours_only(&key(7)))
            .with_body(format!(
                r#"{{"statement_hash":"{H}","chain":"regtest","height":100,"signers":[],"operators":["producer","confirmer"],"added":1,"file":"stored"}}"#
            ))
            .create_async()
            .await;
        let c = confirmer(&n, dir.path(), &env, None, &server);
        let v = done(c.round(Chain::Regtest).await.unwrap());
        assert!(matches!(&v[0], Outcome::Signed { .. }), "{v:?}");
        assert_eq!(c.our_key.get(), Some(&pubkey(&key(7))));
        // A node whose key is on nobody's list.
        let (n, dir, _) = setup(101);
        let env = format!("producer={P}");
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/api/snapshots/pending?chain=regtest")
            .with_body(pending_json(&[listed(R_P, H, false, &["producer"])]))
            .create_async()
            .await;
        let post = server
            .mock("POST", "/api/snapshots/statement")
            .expect(0)
            .create_async()
            .await;
        let v = done(round(&n, dir.path(), &env, None, &server).await);
        post.assert_async().await;
        assert_eq!(
            v,
            vec![Outcome::Failed {
                height: 100,
                statement_hash: H.into(),
                error: "this node's key is not on the operator list".into()
            }]
        );
    }

    #[test]
    fn mismatches_dissents_and_closed_gates_are_counted_and_logged_once_per_run() {
        let waiting = Outcome::Waiting {
            height: 100,
            statement_hash: H.into(),
            why: Mismatch::NoDiaryEntry { height: 100 },
        };
        let young = Outcome::Waiting {
            height: 200,
            statement_hash: "22".repeat(32),
            why: Mismatch::TooShallow {
                have: 12,
                need: 144,
            },
        };
        let already = Outcome::AlreadySigned {
            height: 100,
            statement_hash: H.into(),
        };
        let dissent = Outcome::Dissented {
            height: 300,
            statement_hash: "33".repeat(32),
            why: Mismatch::Coins {
                diary: 7,
                statement: 8,
            },
            operators: vec!["confirmer".into()],
        };
        let mut t = Tally::default();
        let mut seen = HashSet::new();
        assert_eq!(
            t.add(&Round::Done(vec![waiting.clone(), young.clone()]), &mut seen),
            vec!["did not sign the snapshot at 100: this node's diary has nothing at 100".to_string()],
            "a young statement is the normal state and says nothing"
        );
        assert!(t
            .add(&Round::Done(vec![waiting, young]), &mut seen)
            .is_empty());
        assert_eq!(
            t.add(&Round::Done(vec![already, dissent]), &mut seen),
            vec!["sent a dissent about block 300: coin count differs from the diary (diary 7, statement 8)".to_string()]
        );
        assert_eq!(
            t.add(&Round::Unvalidated, &mut seen),
            vec![UNVALIDATED_LINE.to_string()]
        );
        assert!(t.add(&Round::Unvalidated, &mut seen).is_empty());
        assert_eq!(
            (t.rounds, t.signed, t.dissents, t.mismatches, t.gate_closed),
            (5, 0, 1, 2, 2)
        );
        assert_eq!(
            t.report(),
            vec![
                "confirmer: 5 rounds, co-signed 0, dissents sent 1, mismatches 2, failed 0".to_string(),
                "confirmer: 2 rounds skipped while this node was still checking the snapshot it started from".to_string(),
                "confirmer, last mismatch: sent a dissent about block 300: coin count differs from the diary (diary 7, statement 8)".to_string(),
            ]
        );
        assert!(t.report().iter().all(|l| !l.contains('\u{2014}')));
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
        r.diary_skipped = BTreeSet::from([225_900, 226_000]);
        r.producer_skipped = BTreeSet::from([226_000]);
        assert_eq!(
            r.lines(Some(
                "3 heights, newest 226200 (block 0123456789abcdef)".into()
            )),
            vec![
                "diary: 3 heights, newest 226200 (block 0123456789abcdef)".to_string(),
                "diary: 2 grid heights not written down while this node was still checking the snapshot it started from".to_string(),
                "diary, last step: wrote 226200".to_string(),
                "confirmer: off, its key is not on the operator list".to_string(),
                "producer: sent block 226200 to easybtx.com".to_string(),
                "producer: 1 export skipped while this node was still checking the snapshot it started from".to_string(),
            ]
        );
    }

    #[test]
    fn only_a_validating_signing_listed_node_confirms() {
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
        let run = |s, k: Option<&str>| why_not(s, k, REGTEST_GENESIS, Some(&env));
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
    }

    #[test]
    fn the_dissents_record_keeps_the_newest_heights() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_dissents(dir.path()).is_empty());
        let many: BTreeSet<u64> = (1..=150u64).map(|i| i * 100).collect();
        save_dissents(dir.path(), &many).unwrap();
        let kept = load_dissents(dir.path());
        assert_eq!(kept.len(), DISSENTS_KEEP);
        assert_eq!(kept.first(), Some(&5_100));
        assert_eq!(kept.last(), Some(&15_000));
        std::fs::write(dir.path().join(DISSENTS_FILE), b"{ not json").unwrap();
        assert!(load_dissents(dir.path()).is_empty());
    }
}
````

In `crates/btx-core/src/lib.rs`, after `pub mod snapshot;` add `pub mod snapshot_confirmer;`.

- [ ] **Step 2: Run them to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot_confirmer::`
Expected: compile errors, among them `` cannot find struct, variant or union type `Confirmer` in this scope `` and `` cannot find type `Outcome` in this scope ``.

- [ ] **Step 3: Write the confirmer**

Insert above `#[cfg(test)]` in `crates/btx-core/src/snapshot_confirmer.rs`:

````rust
//! The confirmer (section 5 of docs/decisions/2026-09-29-every-node-starts-
//! near-the-tip.md), shared by the app and `btx-confirmer` (section 5a): a
//! node that validates, signs, and whose key is on the operator list reads
//! the statements waiting on easybtx.com about every ten minutes. While
//! `getchainstates` shows a chainstate the node has not finished checking,
//! a round stops right there and asks and signs nothing
//! ([`Round::Unvalidated`]). Otherwise every statement that is not a dissent
//! goes through the one verdict (`crate::statement_check`):
//!
//! * **Sign**: the engine signs a copy (`signutxosnapshotmanifest`), the app
//!   checks that the one new signature is this node's and valid, and sends
//!   back a manifest carrying the statement and that signature alone.
//! * **Dissent** (section 6a): a chain field differs from this node's diary.
//!   The confirmer builds a dissent from its own diary entry
//!   (`statement_check::dissent_from`), has the engine sign it the same way,
//!   and posts it, once per height ([`DISSENTS_FILE`]).
//! * **Neither**: not 144 deep yet, no diary entry on this chain, a foreign
//!   chain, a held root: nothing is signed and nothing is sent.
//!
//! A statement marked as a dissent, by the website or by its own zero file
//! fields, is never signed. The copy the engine signs goes in
//! `<datadir>/snapshot-confirmer/` and the engine is handed its path relative
//! to its own data folder, which is how it reads a relative path
//! (`AbsPathForConfigVal`, src/rpc/blockchain.cpp:4631-4632 at 84b998b4).
//! Every mismatch is logged once per statement per run and counted for Copy
//! diagnostics ([`Tally`]). Nothing the website says is trusted: every
//! statement is read from its own bytes.

use crate::confirmed_load::Holds;
use crate::confirmed_snapshot::{self as cs, ChainRules, Manifest};
use crate::diary::DiaryEntry;
use crate::operators::{self, Chain};
use crate::rpc::Rpc;
use crate::snapshot_serve::CONFIRMATIONS_REQUIRED;
use crate::snapshot_site::{self as site, PendingStatement};
use crate::statement_check::{check_against_node, dissent_from, Mismatch, Verdict};
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::OnceLock;

/// How often a confirmer reads what waits.
pub const CONFIRM_EVERY_SECS: u64 = 600;
/// Under the node's data folder: where a copy is signed and removed again.
pub const WORK_DIR: &str = "snapshot-confirmer";
/// Beside the diary: the heights this operator sent a dissent for.
pub const DISSENTS_FILE: &str = "snapshot-dissents.json";
/// Heights kept in [`DISSENTS_FILE`], as many as the diary keeps.
pub const DISSENTS_KEEP: usize = crate::diary::DIARY_KEEP;
/// What a round says, once per run, while the validated gate is closed.
pub const UNVALIDATED_LINE: &str = "not confirming for now: this node is still checking the snapshot it started from, and it signs nothing until that check is done";

/// What one statement came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Signed {
        height: u64,
        statement_hash: String,
        operators: Vec<String>,
    },
    /// This node's diary disagrees; its signed dissent was posted.
    Dissented {
        height: u64,
        statement_hash: String,
        why: Mismatch,
        operators: Vec<String>,
    },
    /// This node's operator signed it already, with this key or another.
    AlreadySigned { height: u64, statement_hash: String },
    /// This operator's dissent about that height is sent already.
    AlreadyDissented { height: u64, statement_hash: String },
    /// Neither signed nor disputed: not deep enough yet, or this node cannot
    /// say (section 5, checks 1 to 3).
    Waiting {
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
    /// The website has closed this height after a disagreement (section 6a).
    Closed { height: u64, statement_hash: String },
}

impl Outcome {
    pub fn height(&self) -> u64 {
        match self {
            Outcome::Signed { height, .. }
            | Outcome::Dissented { height, .. }
            | Outcome::AlreadySigned { height, .. }
            | Outcome::AlreadyDissented { height, .. }
            | Outcome::Waiting { height, .. }
            | Outcome::Failed { height, .. }
            | Outcome::Closed { height, .. } => *height,
        }
    }

    pub fn statement_hash(&self) -> &str {
        match self {
            Outcome::Signed { statement_hash, .. }
            | Outcome::Dissented { statement_hash, .. }
            | Outcome::AlreadySigned { statement_hash, .. }
            | Outcome::AlreadyDissented { statement_hash, .. }
            | Outcome::Waiting { statement_hash, .. }
            | Outcome::Failed { statement_hash, .. }
            | Outcome::Closed { statement_hash, .. } => statement_hash,
        }
    }

    /// One plain line for the node log.
    pub fn line(&self) -> String {
        match self {
            Outcome::Signed {
                height, operators, ..
            } => format!(
                "co-signed the snapshot at {height}; signed by {} now",
                operators.join(", ")
            ),
            Outcome::Dissented { height, why, .. } => {
                format!("sent a dissent about block {height}: {why}")
            }
            Outcome::AlreadySigned { height, .. } => {
                format!("the snapshot at {height} carries this operator's signature already")
            }
            Outcome::AlreadyDissented { height, .. } => {
                format!("this operator's dissent about block {height} is on easybtx.com already")
            }
            Outcome::Waiting { height, why, .. } => {
                format!("did not sign the snapshot at {height}: {why}")
            }
            Outcome::Failed { height, error, .. } => {
                format!("could not co-sign the snapshot at {height} yet: {error}")
            }
            Outcome::Closed { height, .. } => format!(
                "easybtx.com has closed block {height} after a disagreement; nothing more is sent for it"
            ),
        }
    }
}

/// What one round came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Round {
    /// The validated gate is closed: easybtx.com was not asked anything.
    Unvalidated,
    Done(Vec<Outcome>),
}

/// Why this node does not confirm, or `None` when it does: it validates,
/// signs, and its key is on the list of its chain. Whether its chain rests
/// on its own checks is the gate, asked at the top of every round.
pub fn why_not(
    status: &crate::node_api::MatmulTrustedStatus,
    our_key_hex: Option<&str>,
    genesis: &str,
    regtest_env: Option<&str>,
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
    None
}

/// The heights this operator sent a dissent for. A missing or broken file
/// reads as none; the website's `pending` is looked at too.
pub fn load_dissents(dir: &Path) -> BTreeSet<u64> {
    std::fs::read(dir.join(DISSENTS_FILE))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default()
}

/// Write the newest [`DISSENTS_KEEP`] heights, atomically.
pub fn save_dissents(dir: &Path, heights: &BTreeSet<u64>) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let skip = heights.len().saturating_sub(DISSENTS_KEEP);
    let kept: Vec<u64> = heights.iter().skip(skip).copied().collect();
    let json = serde_json::to_vec(&kept).map_err(std::io::Error::other)?;
    crate::fsx::atomic_write(&dir.join(DISSENTS_FILE), &json)
}

/// One confirmer. The app makes one per round; `btx-confirmer` one per run.
pub struct Confirmer<'a> {
    pub rpc: &'a dyn Rpc,
    pub client: &'a reqwest::Client,
    pub site: &'a str,
    /// Holds the diary and the dissents record: the datadir in the app,
    /// `--state` for `btx-confirmer`.
    pub state_dir: &'a Path,
    /// The node's own data folder, where its `.cookie` is: copies are signed
    /// in its [`WORK_DIR`], which the engine can read and write.
    pub node_dir: &'a Path,
    /// This node's signing key: set by the app from its key file, learned
    /// by `btx-confirmer` from the engine's first signature.
    pub our_key: OnceLock<[u8; 33]>,
    pub holds: Holds<'a>,
    pub regtest_env: Option<&'a str>,
}

impl Confirmer<'_> {
    /// Every statement waiting on `chain`, as the website lists them, unless
    /// the validated gate is closed.
    pub async fn round(&self, chain: Chain) -> Result<Round, String> {
        if !crate::diary::gate_open(self.rpc).await {
            return Ok(Round::Unvalidated);
        }
        let pending = site::get_pending(self.client, self.site, site::chain_name(chain))
            .await
            .map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(pending.statements.len());
        for p in &pending.statements {
            if let Some(o) = self.confirm_one(p, &pending.statements).await {
                out.push(o);
            }
        }
        Ok(Round::Done(out))
    }

    /// `None` for a dissent: it is never signed and says nothing.
    pub async fn confirm_one(&self, p: &PendingStatement, all: &[PendingStatement]) -> Option<Outcome> {
        if p.dissent {
            return None;
        }
        let (height, statement_hash) = (p.height, p.statement_hash.to_ascii_lowercase());
        let waiting = |why: Mismatch| {
            Some(Outcome::Waiting {
                height,
                statement_hash: statement_hash.clone(),
                why,
            })
        };
        let failed = |error: String| {
            Some(Outcome::Failed {
                height,
                statement_hash: statement_hash.clone(),
                error,
            })
        };
        let Some(bytes) = operators::hex_decode(&p.manifest_hex) else {
            return waiting(Mismatch::Unreadable("the manifest is not hex".into()));
        };
        let m = match cs::parse(&bytes) {
            Ok(m) => m,
            Err(e) => return waiting(e.into()),
        };
        if cs::is_dissent(&m.statement) {
            return None;
        }
        if m.statement.hash().display_hex() != statement_hash {
            return waiting(Mismatch::Unreadable(
                "the manifest is not the statement it is listed as".into(),
            ));
        }
        let rules = match ChainRules::for_statement(&m.statement, self.regtest_env) {
            Ok(r) => r,
            Err(e) => return waiting(e.into()),
        };
        let signed_by = match cs::confirming_operators(&m, &rules.operators) {
            Ok(o) => o,
            Err(e) => return waiting(e.into()),
        };
        if let Some(key) = self.our_key.get() {
            let Some(ours) = rules.operators.operator_of(key) else {
                return failed("this node's key is not on the operator list".into());
            };
            if signed_by.iter().any(|o| o == ours) || m.signatures.iter().any(|s| s.key == *key) {
                return Some(Outcome::AlreadySigned {
                    height,
                    statement_hash,
                });
            }
        }
        let diary = crate::diary::load(self.state_dir, &m.statement.chain_id().display_hex());
        match check_against_node(
            self.rpc,
            &m.statement,
            &rules,
            &diary,
            &self.holds,
            CONFIRMATIONS_REQUIRED,
        )
        .await
        {
            Verdict::Neither(why) => waiting(why),
            Verdict::Sign => Some(self.sign(&m, &rules, &signed_by, height, statement_hash).await),
            Verdict::Dissent(why) => {
                let Some(entry) = diary.at(height).cloned() else {
                    return waiting(Mismatch::NoDiaryEntry { height });
                };
                Some(
                    self.dissent(&entry, &rules, why, all, height, statement_hash)
                        .await,
                )
            }
        }
    }

    async fn sign(
        &self,
        m: &Manifest,
        rules: &ChainRules,
        signed_by: &[String],
        height: u64,
        statement_hash: String,
    ) -> Outcome {
        let (key, one) = match self.sign_copy(m, &statement_hash).await {
            Ok(x) => x,
            Err(error) => {
                return Outcome::Failed {
                    height,
                    statement_hash,
                    error,
                }
            }
        };
        let Some(ours) = rules.operators.operator_of(&key) else {
            return Outcome::Failed {
                height,
                statement_hash,
                error: "this node's key is not on the operator list".into(),
            };
        };
        if signed_by.iter().any(|o| o == ours) {
            return Outcome::AlreadySigned {
                height,
                statement_hash,
            };
        }
        match site::post_statement(self.client, self.site, &one.to_bytes()).await {
            Ok(reply) => Outcome::Signed {
                height,
                statement_hash,
                operators: reply.operators,
            },
            Err(e) if e.is_closed_height() => Outcome::Closed {
                height,
                statement_hash,
            },
            Err(e) => Outcome::Failed {
                height,
                statement_hash,
                error: e.to_string(),
            },
        }
    }

    async fn dissent(
        &self,
        entry: &DiaryEntry,
        rules: &ChainRules,
        why: Mismatch,
        all: &[PendingStatement],
        height: u64,
        statement_hash: String,
    ) -> Outcome {
        let mut sent = load_dissents(self.state_dir);
        let ours = self
            .our_key
            .get()
            .and_then(|k| rules.operators.operator_of(k))
            .map(str::to_string);
        let on_site = ours.as_ref().is_some_and(|o| {
            all.iter()
                .any(|q| q.dissent && q.height == height && q.operators.iter().any(|x| x == o))
        });
        if sent.contains(&height) || on_site {
            return Outcome::AlreadyDissented {
                height,
                statement_hash,
            };
        }
        let failed = |error: String| Outcome::Failed {
            height,
            statement_hash: statement_hash.clone(),
            error,
        };
        let st = match dissent_from(entry, rules) {
            Ok(s) => s,
            Err(e) => return failed(e.to_string()),
        };
        let name = st.hash().display_hex();
        let bare = Manifest {
            statement: st,
            signatures: vec![],
        };
        let (key, one) = match self.sign_copy(&bare, &name).await {
            Ok(x) => x,
            Err(e) => return failed(e),
        };
        if rules.operators.operator_of(&key).is_none() {
            return failed("this node's key is not on the operator list".into());
        }
        match site::post_statement(self.client, self.site, &one.to_bytes()).await {
            Ok(reply) => {
                sent.insert(height);
                // The website's pending list covers a record that failed to write.
                let _ = save_dissents(self.state_dir, &sent);
                Outcome::Dissented {
                    height,
                    statement_hash,
                    why,
                    operators: reply.operators,
                }
            }
            Err(e) if e.is_closed_height() => {
                sent.insert(height);
                let _ = save_dissents(self.state_dir, &sent);
                Outcome::Closed {
                    height,
                    statement_hash,
                }
            }
            Err(e) => failed(e.to_string()),
        }
    }

    /// Have the engine sign a copy of `m` and keep only this node's new
    /// signature, checked. Returns the key it signed with. The copy is
    /// `<node_dir>/snapshot-confirmer/<name>.manifest`; the engine is handed
    /// `snapshot-confirmer/<name>.manifest`, relative to its data folder.
    async fn sign_copy(&self, m: &Manifest, name: &str) -> Result<([u8; 33], Manifest), String> {
        let dir = self.node_dir.join(WORK_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| format!("work folder: {e}"))?;
        let file = format!("{name}.manifest");
        let path = dir.join(&file);
        std::fs::write(&path, m.to_bytes()).map_err(|e| format!("writing the copy: {e}"))?;
        let signed = self
            .rpc
            .call("signutxosnapshotmanifest", json!([format!("{WORK_DIR}/{file}")]))
            .await;
        let after = std::fs::read(&path);
        let _ = std::fs::remove_file(&path);
        let answer = signed.map_err(|e| format!("the engine did not sign: {e}"))?;
        let key = answer["signer"]
            .as_str()
            .and_then(operators::parse_key)
            .ok_or("the engine did not say which key signed")?;
        if self.our_key.get().is_some_and(|known| *known != key) {
            return Err("the engine did not sign with this node's key".into());
        }
        let after = cs::parse(&after.map_err(|e| format!("reading the copy: {e}"))?)
            .map_err(|e| format!("the signed copy does not read: {e}"))?;
        let ours = after
            .signatures
            .into_iter()
            .find(|s| s.key == key)
            .ok_or("the engine's signature is not in the copy")?;
        if after.statement != m.statement
            || !cs::signature_is_valid(&m.statement.hash(), &ours.key, &ours.der)
        {
            return Err("the engine's signature does not check out".into());
        }
        let _ = self.our_key.set(key);
        Ok((
            key,
            Manifest {
                statement: m.statement.clone(),
                signatures: vec![ours],
            },
        ))
    }
}

/// What the confirmer did this run, for Copy diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tally {
    pub rounds: u64,
    pub signed: u64,
    /// Dissents sent.
    pub dissents: u64,
    /// Statements this node did not agree with, once each per run: every
    /// dissent, and every "neither" but a statement not deep enough yet.
    pub mismatches: u64,
    /// Rounds the validated gate stopped.
    pub gate_closed: u64,
    pub failed: u64,
    pub last_mismatch: Option<String>,
    /// Why the last round could not run.
    pub last_error: Option<String>,
}

fn kind(o: &Outcome) -> &'static str {
    match o {
        Outcome::Signed { .. } => "signed",
        Outcome::Dissented { .. } => "dissented",
        Outcome::AlreadySigned { .. } => "already",
        Outcome::AlreadyDissented { .. } => "already-dissented",
        Outcome::Waiting { .. } => "waiting",
        Outcome::Failed { .. } => "failed",
        Outcome::Closed { .. } => "closed",
    }
}

impl Tally {
    /// Count a round. Returns the log lines for what this run has not
    /// reported yet (`seen` lives as long as the run), so a mismatch is
    /// logged and counted once, not every ten minutes. A statement already
    /// signed or dissented, and one not 144 deep yet, is the quiet, normal
    /// state and is not logged.
    pub fn add(&mut self, round: &Round, seen: &mut HashSet<String>) -> Vec<String> {
        self.rounds += 1;
        self.last_error = None;
        let outcomes = match round {
            Round::Unvalidated => {
                self.gate_closed += 1;
                return if seen.insert("gate".into()) {
                    vec![UNVALIDATED_LINE.to_string()]
                } else {
                    Vec::new()
                };
            }
            Round::Done(o) => o,
        };
        let mut lines = Vec::new();
        for o in outcomes {
            let first = seen.insert(format!("{}:{}", kind(o), o.statement_hash()));
            let young = matches!(
                o,
                Outcome::Waiting {
                    why: Mismatch::TooShallow { .. },
                    ..
                }
            );
            match o {
                Outcome::Signed { .. } => self.signed += 1,
                Outcome::Dissented { .. } => {
                    self.dissents += 1;
                    self.mismatches += 1;
                    self.last_mismatch = Some(o.line());
                }
                Outcome::Waiting { .. } if !young => {
                    self.last_mismatch = Some(o.line());
                    if first {
                        self.mismatches += 1;
                    }
                }
                Outcome::Failed { .. } if first => self.failed += 1,
                _ => {}
            }
            let quiet = young
                || matches!(
                    o,
                    Outcome::AlreadySigned { .. } | Outcome::AlreadyDissented { .. }
                );
            if first && !quiet {
                lines.push(o.line());
            }
        }
        lines
    }

    /// The lines Copy diagnostics shows.
    pub fn report(&self) -> Vec<String> {
        let mut out = vec![format!(
            "confirmer: {} rounds, co-signed {}, dissents sent {}, mismatches {}, failed {}",
            self.rounds, self.signed, self.dissents, self.mismatches, self.failed
        )];
        if self.gate_closed > 0 {
            out.push(format!(
                "confirmer: {} rounds skipped while this node was still checking the snapshot it started from",
                self.gate_closed
            ));
        }
        if let Some(m) = &self.last_mismatch {
            out.push(format!("confirmer, last mismatch: {m}"));
        }
        if let Some(e) = &self.last_error {
            out.push(format!("confirmer, last round: {e}"));
        }
        out
    }
}

/// What the snapshot network did in this run of the app, for Copy
/// diagnostics. The status refresher writes the diary and confirmer parts,
/// the snapshot keeper the producer part.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkReport {
    /// The last diary step worth saying.
    pub diary: Option<String>,
    /// Grid heights not written down because the validated gate was closed.
    pub diary_skipped: BTreeSet<u64>,
    pub confirmer: Tally,
    /// Why this node does not confirm ([`why_not`]).
    pub confirmer_off: Option<String>,
    /// What the refresher already logged in this run (see [`Tally::add`]).
    pub seen: HashSet<String>,
    /// The producer's last word.
    pub producer: Option<String>,
    /// Grid heights not exported because the validated gate was closed.
    pub producer_skipped: BTreeSet<u64>,
}

fn times(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl NetworkReport {
    /// The "Snapshots" section of Copy diagnostics. `diary_summary` is
    /// `diary::summary` of the datadir.
    pub fn lines(&self, diary_summary: Option<String>) -> Vec<String> {
        const CHECKING: &str = "while this node was still checking the snapshot it started from";
        let mut out = vec![format!(
            "diary: {}",
            diary_summary.unwrap_or_else(|| "empty".into())
        )];
        if !self.diary_skipped.is_empty() {
            out.push(format!(
                "diary: {} not written down {CHECKING}",
                times(self.diary_skipped.len(), "grid height", "grid heights")
            ));
        }
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
        if !self.producer_skipped.is_empty() {
            out.push(format!(
                "producer: {} skipped {CHECKING}",
                times(self.producer_skipped.len(), "export", "exports")
            ));
        }
        out
    }
}
````

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot_confirmer::`
Expected: `13 passed; 0 failed` (derived, not run).

Sabotage, then undo: in `confirm_one`, delete the two `return None;` lines that skip a dissent; run; expect `a_pending_dissent_is_never_signed` to FAIL; put them back. Then in `round`, delete the gate's three lines; expect `a_node_on_an_unvalidated_snapshot_chainstate_asks_nothing_and_signs_nothing` to FAIL (the website is asked); put them back.

- [ ] **Step 5: Format, lint, commit**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cargo test --locked --lib && cd ../..
git add crates/btx-core/src/snapshot_confirmer.rs crates/btx-core/src/lib.rs
git commit -m "core: confirmers co-sign a waiting snapshot only when their own diary agrees, and dissent when it does not" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 6: The app drives it, and Copy diagnostics says what it did

(Was Task 7. Every anchor below was re-read on `origin/main` (`b330e3d`), where the Tools code (`tools.rs`, `diagnostics.rs`) now lives; the confirmed-snapshots plans change other parts of `commands.rs`, not these lines.)

**Files:**
- Modify: `crates/btx-core/src/diagnostics.rs` (`snapshots` on `DiagnosticsInput`, a "Snapshots" section; tests)
- Modify: `apps/node/src-tauri/src/state.rs` (`snapshot_network` on `AppState`; the settings comment)
- Modify: `apps/node/src-tauri/src/commands.rs` (refresher: diary step and confirmer round; keeper: grid, gate, export into the waiting set, a look at every waiting pair, producer checks, submission; `diary_step`, `confirm_round`, `submit_offered_pair`; copy and comments)
- Modify: `apps/node/src-tauri/src/tools.rs` (the section's lines)
- Modify: `apps/node/index.html`, `apps/node/src/main.ts` (one string each)
- Modify: `apps/node/CHANGELOG.md`

**Interfaces:**
- Consumes: Tasks 1 to 5 (`diary::{chain_id, grid_for, gate_open, record_at_tip, summary, DiaryOutcome}`, `snapshot_serve::{export_due, blocks_to_grid, EXPORT_GRID, CONFIRMATIONS_REQUIRED, MATURE_DEADLINE, load_waiting, export_at_tip, mature_step, MatureOutcome, CyclePhase}`, `snapshot_producer::{ProducerChecks, check_before_send, submit, submission_due, load_submitted, key_is_listed, RESUBMIT_EVERY}`, `snapshot_confirmer::{Confirmer, Round, why_not, NetworkReport, CONFIRM_EVERY_SECS}`, `snapshot_site::{client, site_base}`), `confirmed_load::Holds::compiled()`, `operators::{regtest_env, parse_key, Chain}`; the app's existing `AppState` slots `matmul_trusted`, `signer_pubkey`, `node_datadir()`.
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
                "confirmer: 3 rounds, co-signed 1, dissents sent 0, mismatches 0, failed 0".into(),
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
    /// What the snapshot network did in this run: the diary, the confirmer
    /// (mismatches, dissents sent, rounds the validated gate stopped), the
    /// producer (`snapshot_confirmer::NetworkReport::lines`).
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
Expected: `40 passed; 0 failed` (39 on `origin/main` and the new one; derived, not run).

- [ ] **Step 4: The report slot on `AppState`**

In `apps/node/src-tauri/src/state.rs`:

*The settings comment.* Replace:

````rust
    /// with this node's key, wait for it to mature, offer it over P2P, refresh
    /// it every 500 blocks, and re-offer it after every node start. Off by
````

with:

````rust
    /// with this node's key, wait until it is 144 blocks deep, offer it over
    /// P2P, take a fresh one at every multiple of 100 blocks, and re-offer it
    /// after every node start. Off by
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

In `apps/node/src-tauri/src/commands.rs`, in this order (each old text appears exactly once on `origin/main`):

*Start path comment.* Replace:

````rust
    // node is at the tip (the offer never survives a restart), and refreshes
    // it every 500 blocks. Gated against the LIVE node on every tick.
````

with:

````rust
    // node is at the tip (the offer never survives a restart), and takes a
    // fresh one at every multiple of 100 blocks, offered once it is 144 deep.
    // Gated against the LIVE node on every tick.
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
                    // the tip is on the grid and the node's chain rests on its
                    // own checks (btx_core::diary); the confirmer co-signs a
                    // statement waiting on easybtx.com only when that diary
                    // agrees, and sends a dissent when it does not
                    // (btx_core::snapshot_confirmer). Both run off the tick:
                    // gettxoutsetinfo and a round of HTTP can outlast it.
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

*Keeper paragraph.* Replace:

````rust
// btx_core::snapshot_serve is the design. In one paragraph: a node that
// validates and signs exports the UTXO set at the tip, waits ten
// confirmations because the dump bases on the 0-conf tip and siblings arrive
// every ~25 blocks, offers the matured pair over P2P, re-makes the links to
// the mirrors because service bits travel only in the handshake, refreshes
// every 500 blocks, and re-offers after every node start because the offer
// lives in the running process only. The keeper below is that loop. Every
// decision in it is a pure function in the core module with a test; this is
// orchestration and reporting.
````

with:

````rust
// btx_core::snapshot_serve is the design. In one paragraph: a node that
// validates and signs exports the UTXO set when its tip is on the grid of
// 100 and its chain rests on its own checks, keeps each export waiting until
// its block is 144 deep (two can wait at once), checks it against the node's
// own diary (btx_core::snapshot_producer), offers it over P2P, re-makes the
// links to the mirrors because service bits travel only in the handshake,
// sends it to easybtx.com when the node's key is on the operator list, and
// re-offers after every node start because the offer lives in the running
// process only. The keeper below is that loop. Every decision in it is a
// pure function in the core module with a test; this is orchestration and
// reporting.
````

*Keeper: the diary step, the confirmer round and the submission, as plain functions.* Replace:

````rust
/// The keeper: one loop per node start, superseded by the generation counter
/// the moment the node stops or the role is switched off. A cycle in flight
/// asks `keep_going` at every poll and aborts without offering anything.
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
    let note = match record_at_tip(rpc, &dd, validation_mode.as_deref()).await {
        Ok(DiaryOutcome::Recorded(e)) => {
            eprintln!(
                "[snapshot] diary: wrote {} (block {}, {} coins)",
                e.height, e.block_hash, e.coins
            );
            Some(format!(
                "wrote {} (block {})",
                e.height,
                &e.block_hash[..16.min(e.block_hash.len())]
            ))
        }
        Ok(DiaryOutcome::TipMoved) => Some(
            "the tip moved while the chain state was read; the next chance is the next grid \
             height"
                .to_string(),
        ),
        Ok(DiaryOutcome::NotAllowed) => {
            Some("not kept: this node follows signatures instead of checking blocks".to_string())
        }
        Ok(DiaryOutcome::Unvalidated(h)) => {
            let mut r = report.lock().await;
            if r.diary_skipped.insert(h) {
                eprintln!(
                    "[snapshot] diary: not writing down block {h}: this node is still checking \
                     the snapshot it started from"
                );
                r.diary = Some(format!(
                    "block {h} not written down: this node is still checking the snapshot it \
                     started from"
                ));
            }
            None
        }
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
    if let Some(why) = confirmer::why_not(&trusted, key.as_deref(), &genesis, env.as_deref()) {
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
    let c = confirmer::Confirmer {
        rpc,
        client: &client,
        site: &site,
        state_dir: &dd,
        node_dir: &dd,
        our_key: std::sync::OnceLock::from(our_key),
        holds: btx_core::confirmed_load::Holds::compiled(),
        regtest_env: env.as_deref(),
    };
    let round = c.round(chain).await;
    let mut guard = report.lock().await;
    let r = &mut *guard;
    r.confirmer_off = None;
    match round {
        Ok(done) => {
            for line in r.confirmer.add(&done, &mut r.seen) {
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
            "sending block {height} to easybtx.com did not work; trying again in ten minutes: {e}"
        ),
    }
}

/// The keeper: one loop per node start, superseded by the generation counter
/// the moment the node stops or the role is switched off. Each tick looks
/// once at the tip (to export on the grid) and once at every waiting pair;
/// nothing runs between ticks.
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

*Keeper: export on the grid, then a look at every waiting pair.* Replace everything from the line `            if snap::refresh_due(tip.unwrap_or(0), base) {` through the `                continue;` and `            }` that end that `if` (lines 4420 to 4471 on `origin/main`, the export, the `on_phase` closure, `run_cycle` and what a failed cycle says) with:

````rust
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

            // ── Export on the grid, on a chain this node checked itself ──────
            // Nothing is exported while getchainstates shows a chainstate the
            // node has not finished checking (btx_core::diary::gate_open). The
            // diary first, from the same tip, so the check before the offer
            // has this height to compare with.
            let t = tip.unwrap_or(0);
            if snap::export_due(t, grid, base, &snap::load_waiting(&dir)) {
                if !btx_core::diary::gate_open(&rpc).await {
                    if report_slot.lock().await.producer_skipped.insert(t) {
                        eprintln!(
                            "[snapshot] not exporting block {t}: this node is still checking the \
                             snapshot it started from"
                        );
                    }
                } else {
                    match btx_core::diary::record_at_tip(
                        &rpc,
                        &datadir,
                        facts.validation_mode.as_deref(),
                    )
                    .await
                    {
                        Ok(btx_core::diary::DiaryOutcome::Recorded(e)) => {
                            eprintln!("[snapshot] diary: wrote {} (block {})", e.height, e.block_hash)
                        }
                        Ok(_) => {}
                        Err(e) => eprintln!("[snapshot] diary: {e}"),
                    }
                    match snap::export_at_tip(&rpc, &dir, grid).await {
                        Ok(w) => eprintln!(
                            "[snapshot] exported block {}; it is offered once it is {} blocks deep",
                            w.height,
                            snap::CONFIRMATIONS_REQUIRED
                        ),
                        Err(e) => {
                            eprintln!("[snapshot] {e}");
                            report_slot.lock().await.producer = Some(e);
                        }
                    }
                }
            }

            // ── Every waiting pair: deep enough, checked, then offered ────────
            let checks = btx_core::snapshot_producer::ProducerChecks {
                diary_dir: datadir.clone(),
                holds: btx_core::confirmed_load::Holds::compiled(),
                regtest_env: btx_core::operators::regtest_env(),
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let mut waiting_phase: Option<snap::CyclePhase> = None;
            let mut offered_now = false;
            for w in snap::load_waiting(&dir) {
                match snap::mature_step(&rpc, &dir, &w, now, snap::MATURE_DEADLINE, &checks).await {
                    Ok(snap::MatureOutcome::Waiting {
                        height,
                        confirmations,
                        tip,
                    }) => {
                        waiting_phase = Some(snap::CyclePhase::Maturing {
                            base: height,
                            confirmations,
                            tip,
                        });
                    }
                    Ok(snap::MatureOutcome::Offered(r)) => {
                        eprintln!(
                            "[snapshot] offering base {} ({} bytes, sha256 {}, file_hash {}, {} chunks)",
                            r.height, r.file_size, r.sha256, r.file_hash, r.chunk_count
                        );
                        offered_now = true;
                    }
                    Ok(snap::MatureOutcome::Dropped { height, why }) => {
                        let line = format!("block {height} was not offered: {why}");
                        eprintln!("[snapshot] {line}");
                        report_slot.lock().await.producer = Some(line);
                    }
                    Err(e) => eprintln!("[snapshot] {e}"),
                }
            }
            if offered_now {
                // Read the new record, and send it, on the next tick.
                wait = std::time::Duration::from_secs(3);
                continue;
            }
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

*Keeper: the row names a pair that waits.* Replace:

````rust
                    (Some(r), Some(true)) => {
                        snap::ServeStatus::serving_message(r, tip, peers_offering)
                    }
````

with:

````rust
                    (Some(r), Some(true)) => format!(
                        "{}{}",
                        snap::ServeStatus::serving_message(r, tip, peers_offering),
                        waiting_phase
                            .as_ref()
                            .map(|p| format!(" {}", p.message()))
                            .unwrap_or_default()
                    ),
````

Replace:

````rust
                    (None, _) => "Nothing exported yet.".to_string(),
````

with:

````rust
                    (None, _) => waiting_phase
                        .as_ref()
                        .map(|p| p.message())
                        .unwrap_or_else(|| "Nothing exported yet.".to_string()),
````

*The switch's answer.* Replace:

````rust
        "On. The first snapshot is exported at the tip and offered once it has ten \
         confirmations, about fifteen minutes; after that it is refreshed every 500 blocks."
````

with:

````rust
        "On. A snapshot is exported when the tip reaches the next multiple of 100 blocks \
         and offered once it is 144 blocks deep, about three and a half hours later. After \
         that, one every 100 blocks."
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
About 9 MB, taken every 100 blocks
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
blocks itself now writes down what its chain looked like every 100 blocks,
once it has finished checking any snapshot it started from. If you serve a
chain snapshot, a fresh one is taken at every multiple of 100 blocks,
compared with that record, and offered once it is 144 blocks deep; it is
sent to easybtx.com when your key is on the operator list. A node on the list
also co-signs snapshots other operators sent, but only when its own record
matches every detail. When its record disagrees, it sends a signed note
saying so instead, and Fast-forward stays off for everyone until that is
looked into. Copy diagnostics has a new "Snapshots" section that says what
your node did. Operators who run a plain BTX node without this app can do
the same job with `btx-confirmer` (see `deploy/esplora/README.md`).
````

(If the confirmed-snapshots entry reads differently by now, put this paragraph right after it.)

- [ ] **Step 9: Every gate**

````bash
for c in crates/btx-core apps/node/src-tauri; do (cd $c && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cargo test --locked); done
(cd apps/node && npx tsc --noEmit && npm test && npx vite build)
````

Expected: all pass. Known flake: see Global Constraints.

- [ ] **Step 10: Check by hand on the owner's Mac (mainnet, a validating node that signs)**

Start the app, open Tools, press Copy diagnostics. Expected in the "Snapshots" section: `diary: empty` until the tip reaches a multiple of 100, then `diary: 1 height, newest <height> (block <16 hex>)` and `diary, last step: wrote <height> (block …)`; on a node still checking the snapshot it started from, instead `diary: 1 grid height not written down while this node was still checking the snapshot it started from`; `confirmer: no round yet in this run` for the first two minutes, then either `confirmer: off, …` with the reason (on this Mac: `it follows signatures instead of checking blocks` if it runs as a mirror, since its device fails the engine's self-test) or `confirmer: 1 rounds, co-signed 0, dissents sent 0, mismatches 0, failed 0`. The node log shows `[snapshot] diary: wrote …` once per grid height. Nothing is sent to easybtx.com from a node whose key is not on the list. No line in the section has an em-dash.

- [ ] **Step 11: Commit**

````bash
git add crates/btx-core/src/diagnostics.rs apps/node/src-tauri/src/state.rs apps/node/src-tauri/src/commands.rs apps/node/src-tauri/src/tools.rs apps/node/index.html apps/node/src/main.ts apps/node/CHANGELOG.md
git commit -m "node: keep the diary, confirm, dissent and send snapshots, and say so in Copy diagnostics" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 7: `btx-confirmer`, the confirmer beside a plain btxd (`src/bin/btx-confirmer.rs`)

(New. Section 5a: an operator whose node the app does not run still counts. The binary adds the arguments and the loop; the diary, the verdict, the website's client, the dissent and the confirmer are Tasks 1 to 5, the same code the app runs.)

**Files:**
- Modify: `crates/btx-core/src/snapshot_confirmer.rs` (`check_node`, `diary_tick`, `DIARY_EVERY`; tests)
- Create: `crates/btx-core/src/bin/btx-confirmer.rs`

**Interfaces:**
- Consumes: Tasks 1 to 5; `rpc::RpcClient::from_cookie` (exists; it re-reads the cookie after a 401, which is how `btx-witness` survives a btxd restart); `confirmed_snapshot::{MAINNET_REPLAY_CONTEXT, REGTEST_REPLAY_CONTEXT, SNAPSHOT_GRID}`; `confirmed_load::Holds::compiled()`.
- Produces (Tasks 8, 9): `snapshot_confirmer::{DIARY_EVERY, check_node(&dyn Rpc) -> Result<Chain, String>, diary_tick(&dyn Rpc, state_dir: &Path) -> Result<Option<DiaryOutcome>, String>}`; the binary `btx-confirmer --datadir <path> --state <path> [--rpc <addr:port>] [--once]`, exit code 0 after `--once` or `--help`, 1 when the node is unreachable or refused at start, 2 on bad arguments.

- [ ] **Step 1: Write the failing library tests**

In `crates/btx-core/src/snapshot_confirmer.rs`, at the end of the test module (after `the_dissents_record_keeps_the_newest_heights`), add:

````rust
    /// Section 5a: it refuses to run beside a node on another chain or
    /// replay context, one that follows signatures, or one with no key.
    #[tokio::test]
    async fn btx_confirmer_refuses_a_node_it_cannot_confirm_beside() {
        let (n, _dir, _) = setup(101);
        assert_eq!(check_node(&n).await, Ok(Chain::Regtest));
        let (n, _dir, _) = setup(101);
        n.with(|s| s.replay_context = Some("33".repeat(32)));
        let e = check_node(&n).await.unwrap_err();
        assert!(e.contains("replay context"), "{e}");
        let (n, _dir, _) = setup(101);
        n.with(|s| s.replay_context = None);
        assert!(check_node(&n).await.unwrap_err().contains("no replay context"));
        let (n, _dir, _) = setup(101);
        n.with(|s| s.mode = "trusted".into());
        assert!(check_node(&n).await.unwrap_err().contains("trusted mode"));
        let (n, _dir, _) = setup(101);
        n.with(|s| s.signer = None);
        assert!(check_node(&n).await.unwrap_err().contains("no signing key"));
        let (n, _dir, _) = setup(101);
        n.with(|s| {
            s.chain.insert(0, "00".repeat(32));
        });
        assert!(check_node(&n).await.unwrap_err().contains("does not confirm"));
    }

    /// Its diary looks every few seconds: one read off the grid, and on the
    /// grid the app's own step, gate and all, with the mode read fresh.
    #[tokio::test]
    async fn btx_confirmers_diary_look_is_cheap_off_the_grid() {
        let (n, dir, _) = setup(101);
        assert_eq!(diary_tick(&n, dir.path()).await, Ok(None));
        assert_eq!(n.methods(), vec!["getblockcount"]);
        let at_200 = || {
            let n = FakeNode::new(REGTEST_GENESIS, 200);
            n.with(|s| {
                s.utxo = Some(json!({
                    "height": 200, "bestblock": format!("{:064x}", 200), "txouts": 201,
                    "hash_serialized_3": "cd".repeat(32)
                }));
                s.chain_tx = 201;
            });
            n
        };
        let dir = tempfile::tempdir().unwrap();
        let n = at_200();
        assert!(matches!(
            diary_tick(&n, dir.path()).await,
            Ok(Some(DiaryOutcome::Recorded(_)))
        ));
        let dir = tempfile::tempdir().unwrap();
        let n = at_200();
        n.with(|s| s.chainstates = Some(chainstates_plain_assumeutxo()));
        assert_eq!(
            diary_tick(&n, dir.path()).await,
            Ok(Some(DiaryOutcome::Unvalidated(200)))
        );
        let n = at_200();
        n.with(|s| s.mode = "trusted".into());
        assert_eq!(
            diary_tick(&n, dir.path()).await,
            Ok(Some(DiaryOutcome::NotAllowed))
        );
    }
````

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot_confirmer::btx_confirmer`
Expected: compile errors `` cannot find function `check_node` in this scope `` and `` cannot find function `diary_tick` in this scope ``.

- [ ] **Step 2: The start check and the diary look**

In `crates/btx-core/src/snapshot_confirmer.rs`, above `/// What the confirmer did this run, for Copy diagnostics.`, insert:

````rust
/// How often `btx-confirmer` looks at the tip for its diary (section 5a: "the
/// tip every few seconds"; a block comes about every 90).
pub const DIARY_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// Before `btx-confirmer` does anything (section 5a): the node is on a chain
/// easyNode knows, reports the replay context compiled for it (it depends
/// only on the chain, so 0.34.9 and 0.34.12 both qualify), checks blocks
/// itself, and has a signing key, which the engine needs to sign. `Err` is
/// the one line to print before exiting.
pub async fn check_node(rpc: &dyn Rpc) -> Result<Chain, String> {
    let genesis = crate::diary::chain_id(rpc)
        .await
        .ok_or("the node did not answer getblockhash 0")?;
    let chain = Chain::from_genesis_hex(&genesis).ok_or_else(|| {
        format!("this node is on chain {genesis}, which easyNode does not confirm snapshots for")
    })?;
    let compiled = match chain {
        Chain::Main => cs::MAINNET_REPLAY_CONTEXT,
        Chain::Regtest => cs::REGTEST_REPLAY_CONTEXT,
    };
    let status = crate::node_api::get_matmul_trusted_status(rpc)
        .await
        .map_err(|e| format!("the node did not answer getmatmultrustedstatus: {e}"))?;
    match status
        .replay_authority_context
        .as_deref()
        .map(str::to_ascii_lowercase)
    {
        Some(c) if c == compiled => {}
        Some(c) => {
            return Err(format!(
                "this node's replay context is {c}, not {compiled}; refusing to run beside it"
            ))
        }
        None => return Err("this node reports no replay context; refusing to run beside it".into()),
    }
    let mode = status.matmul_validation_mode.trim();
    if !mode.eq_ignore_ascii_case("consensus") {
        return Err(format!(
            "this node runs in {mode} mode; only a node that checks blocks itself can confirm"
        ));
    }
    if !status.local_signer {
        return Err(
            "this node has no signing key configured (-matmulattestationsignerkeyfile), so the \
             engine would refuse to sign"
                .into(),
        );
    }
    Ok(chain)
}

/// One diary look for `btx-confirmer`: one `getblockcount` off the grid; on
/// the grid, the app's own step (`crate::diary::record_at_tip`, the gate
/// included), with the node's mode read fresh so a node restarted as a
/// mirror writes nothing.
pub async fn diary_tick(
    rpc: &dyn Rpc,
    state_dir: &Path,
) -> Result<Option<crate::diary::DiaryOutcome>, String> {
    let tip = rpc
        .call("getblockcount", json!([]))
        .await
        .map_err(|e| format!("getblockcount: {e}"))?
        .as_u64()
        .ok_or("getblockcount answered no number")?;
    if tip == 0 || tip % cs::SNAPSHOT_GRID as u64 != 0 {
        return Ok(None);
    }
    let mode = crate::node_api::get_matmul_trusted_status(rpc)
        .await
        .ok()
        .map(|s| s.matmul_validation_mode);
    crate::diary::record_at_tip(rpc, state_dir, mode.as_deref())
        .await
        .map(Some)
}
````

In the test module's `use` lines of `snapshot_confirmer.rs`, `DiaryOutcome` is needed: change `use crate::diary::{Diary, DiaryEntry};` to `use crate::diary::{Diary, DiaryEntry, DiaryOutcome};`.

Run: `cd crates/btx-core && cargo test --locked --lib -- snapshot_confirmer::`
Expected: `15 passed; 0 failed` (derived, not run).

- [ ] **Step 3: Write the binary, with its own tests first**

Create `crates/btx-core/src/bin/btx-confirmer.rs` (no `[[bin]]` entry is needed: like `btx-witness`, a file in `src/bin/` is a binary):

````rust
//! `btx-confirmer`: confirm easyNode snapshots from a BTX node the easyNode
//! app does not run (section 5a of
//! docs/decisions/2026-09-29-every-node-starts-near-the-tip.md).
//!
//! An operator on easyNode's list whose node is a plain btxd counts through
//! this. It does the confirmer's job and nothing else: it keeps a diary of
//! its node's chain state at every multiple of 100, reading the tip every
//! few seconds, and about every ten minutes it reads the statements waiting
//! on easybtx.com and, for each, co-signs it, sends a dissent, or does
//! neither, by the verdict the app uses (`btx_core::statement_check`). While
//! the node reports a snapshot chainstate it has not finished checking, it
//! writes down and signs nothing. It never exports, loads, pins or restarts
//! anything, and it listens on nothing.
//!
//!     btx-confirmer --datadir /var/lib/btx --state /var/lib/btx-confirmer
//!     btx-confirmer --datadir /var/lib/btx --state /var/lib/btx-confirmer --once
//!
//! It reads the node's `.cookie`, as `btx-witness` does, and needs the node's
//! signing key configured: the engine signs, not this program, and the key
//! never leaves the node. The copy the engine signs is written to
//! `<datadir>/snapshot-confirmer/` and handed to the engine by its path
//! relative to the node's data folder. The diary and the dissents it sent
//! live in `--state`. `EASYNODE_SNAPSHOT_SITE` and
//! `EASYNODE_REGTEST_OPERATORS` work as in the app.
//!
//! The code is the app's (`btx_core::{diary, statement_check, snapshot_site,
//! snapshot_confirmer}`); this file adds the arguments and the loop.

use btx_core::confirmed_load::Holds;
use btx_core::diary::DiaryOutcome;
use btx_core::operators::Chain;
use btx_core::rpc::RpcClient;
use btx_core::snapshot_confirmer::{self as confirmer, Confirmer, Tally};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const USAGE: &str = "\
btx-confirmer: confirm easyNode snapshots from a BTX node the easyNode app does not run

USAGE:
    btx-confirmer --datadir <path> --state <path> [--rpc <addr:port>] [--once]

OPTIONS:
    --datadir <path>     the node's data folder, holding its .cookie (on a test
                         chain, that chain's folder, such as <datadir>/regtest)
    --state <path>       where this keeps its diary and the dissents it sent
    --rpc <addr:port>    the node's JSON-RPC (default 127.0.0.1:19334)
    --once               one diary look and one round, then exit
    -h, --help           this

The node must check blocks itself and have its signing key configured.
It listens on nothing.
";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    datadir: PathBuf,
    state: PathBuf,
    rpc: String,
    once: bool,
}

/// `Ok(None)` for `--help`.
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Option<Args>, String> {
    let mut datadir = None;
    let mut state = None;
    let mut rpc = "127.0.0.1:19334".to_string();
    let mut once = false;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--datadir" => datadir = Some(PathBuf::from(it.next().ok_or("--datadir needs a path")?)),
            "--state" => state = Some(PathBuf::from(it.next().ok_or("--state needs a path")?)),
            "--rpc" => rpc = it.next().ok_or("--rpc needs an address")?,
            "--once" => once = true,
            "-h" | "--help" => return Ok(None),
            other => return Err(format!("unknown option: {other}")),
        }
    }
    Ok(Some(Args {
        datadir: datadir.ok_or("--datadir is required")?,
        state: state.ok_or("--state is required")?,
        rpc,
        once,
    }))
}

/// What the loop keeps quiet about: a grid height already reported as not
/// written down, and the last error, so neither fills the log every 5 s.
#[derive(Default)]
struct Quiet {
    skipped: BTreeSet<u64>,
    last_error: Option<String>,
}

async fn diary_look(rpc: &RpcClient, state: &Path, quiet: &mut Quiet) {
    match confirmer::diary_tick(rpc, state).await {
        Ok(Some(DiaryOutcome::Recorded(e))) => {
            quiet.last_error = None;
            eprintln!(
                "[confirmer] diary: wrote {} (block {}, {} coins)",
                e.height, e.block_hash, e.coins
            );
        }
        Ok(Some(DiaryOutcome::Unvalidated(h))) => {
            if quiet.skipped.insert(h) {
                eprintln!(
                    "[confirmer] diary: not writing down block {h}: the node is still checking \
                     the snapshot it started from"
                );
            }
        }
        Ok(_) => quiet.last_error = None,
        Err(e) => {
            if quiet.last_error.as_deref() != Some(e.as_str()) {
                eprintln!("[confirmer] diary: {e}");
            }
            quiet.last_error = Some(e);
        }
    }
}

async fn round(c: &Confirmer<'_>, chain: Chain, tally: &mut Tally, seen: &mut HashSet<String>) {
    match c.round(chain).await {
        Ok(r) => {
            for line in tally.add(&r, seen) {
                eprintln!("[confirmer] {line}");
            }
        }
        Err(e) => {
            eprintln!("[confirmer] this round did not run: {e}");
            tally.last_error = Some(e);
        }
    }
}

#[tokio::main]
async fn main() {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Some(a)) => a,
        Ok(None) => {
            print!("{USAGE}");
            return;
        }
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(e) = std::fs::create_dir_all(&args.state) {
        eprintln!("cannot make {}: {e}", args.state.display());
        std::process::exit(1);
    }
    // Cookie auth, as btx-witness and the app do: no password on a command
    // line, where every process on the machine could read it.
    let cookie = args.datadir.join(".cookie");
    let rpc = match RpcClient::from_cookie(format!("http://{}", args.rpc), &cookie) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "cannot read {}: {e}\nIs the node running, and is this its data folder?",
                cookie.display()
            );
            std::process::exit(1);
        }
    };
    // Refuse before anything else: another chain, another replay context, a
    // node that follows signatures, a node that cannot sign.
    let chain = match confirmer::check_node(&rpc).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[confirmer] {e}");
            std::process::exit(1);
        }
    };
    let http = match btx_core::snapshot_site::client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[confirmer] {e}");
            std::process::exit(1);
        }
    };
    let site = btx_core::snapshot_site::site_base();
    let env = btx_core::operators::regtest_env();
    eprintln!(
        "[confirmer] the node checks blocks itself and signs; diary in {}; confirming for {site}",
        args.state.display()
    );
    let c = Confirmer {
        rpc: &rpc,
        client: &http,
        site: &site,
        state_dir: &args.state,
        node_dir: &args.datadir,
        our_key: OnceLock::new(),
        holds: Holds::compiled(),
        regtest_env: env.as_deref(),
    };
    let mut tally = Tally::default();
    let mut seen = HashSet::new();
    let mut quiet = Quiet::default();
    if args.once {
        diary_look(&rpc, &args.state, &mut quiet).await;
        round(&c, chain, &mut tally, &mut seen).await;
        for line in tally.report() {
            eprintln!("[confirmer] {line}");
        }
        return;
    }
    // The first round a minute in, then every ten minutes; the diary every
    // few seconds in between. A supervisor that restarts it quickly does not
    // make it ask easybtx.com more often than once a minute.
    let mut next_round = Instant::now() + Duration::from_secs(60);
    loop {
        diary_look(&rpc, &args.state, &mut quiet).await;
        if Instant::now() >= next_round {
            round(&c, chain, &mut tally, &mut seen).await;
            next_round = Instant::now() + Duration::from_secs(confirmer::CONFIRM_EVERY_SECS);
        }
        tokio::time::sleep(confirmer::DIARY_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Result<Option<Args>, String> {
        parse_args(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn the_flags_it_takes() {
        assert_eq!(
            args(&["--datadir", "/var/lib/btx", "--state", "/var/lib/btx-confirmer"]),
            Ok(Some(Args {
                datadir: "/var/lib/btx".into(),
                state: "/var/lib/btx-confirmer".into(),
                rpc: "127.0.0.1:19334".into(),
                once: false
            }))
        );
        assert_eq!(
            args(&["--datadir", "d", "--state", "s", "--rpc", "127.0.0.1:19443", "--once"]),
            Ok(Some(Args {
                datadir: "d".into(),
                state: "s".into(),
                rpc: "127.0.0.1:19443".into(),
                once: true
            }))
        );
        assert_eq!(args(&["--help"]), Ok(None));
    }

    #[test]
    fn what_it_refuses() {
        assert_eq!(args(&["--state", "s"]), Err("--datadir is required".into()));
        assert_eq!(args(&["--datadir", "d"]), Err("--state is required".into()));
        assert_eq!(args(&["--datadir"]), Err("--datadir needs a path".into()));
        assert_eq!(
            args(&["--datadir", "d", "--state", "s", "--listen", "0.0.0.0:1"]),
            Err("unknown option: --listen".into()),
            "it listens on nothing"
        );
    }

    #[test]
    fn its_words_have_no_em_dash() {
        assert!(!USAGE.contains('\u{2014}'));
    }
}
````

- [ ] **Step 4: Run its tests, build it, and try it without a node**

````bash
cd crates/btx-core
cargo test --locked --bin btx-confirmer
cargo build --locked --release --bin btx-confirmer
./target/release/btx-confirmer --help | head -3
./target/release/btx-confirmer --datadir "$(mktemp -d)" --state "$(mktemp -d)"; echo "exit $?"
./target/release/btx-confirmer --datadir x; echo "exit $?"
cd ../..
````

Expected: `3 passed; 0 failed`; the build finishes; the first line of the help is `btx-confirmer: confirm easyNode snapshots from a BTX node the easyNode app does not run`; the run without a node prints `cannot read …/.cookie: …` and `Is the node running, and is this its data folder?` and `exit 1`; the run with `--datadir x` alone prints `--state is required` above the usage and `exit 2` (derived, not run).

- [ ] **Step 5: Format, lint, commit**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cargo test --locked && cd ../..
git add crates/btx-core/src/snapshot_confirmer.rs crates/btx-core/src/bin/btx-confirmer.rs
git commit -m "core: btx-confirmer, the snapshot confirmer beside a plain btxd, on the app's own code" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 8: `btx-confirmer`'s unit and a README section (`deploy/esplora/`)

(New. Section 5a: "the unit is `deploy/esplora/btx-confirmer.service.template`, in the shape of `btx-witness.service.template` … The esplora README gains a short section; `install-systemd.sh` does not change.")

**Files:**
- Create: `deploy/esplora/btx-confirmer.service.template`
- Modify: `deploy/esplora/README.md`

**Interfaces:**
- Consumes: Task 7's binary and flags; `deploy/esplora/btx-witness.service.template` (its `User=USER`, `Restart=always` and hardening lines, on `origin/main`).
- Produces: the unit an operator copies to `/etc/systemd/system/btx-confirmer.service`.

- [ ] **Step 1: Write the unit**

Create `deploy/esplora/btx-confirmer.service.template`:

````ini
[Unit]
Description=easyNode snapshot confirmer (btx-confirmer) beside this BTX node
After=network-online.target btxd.service
Wants=network-online.target

[Service]
# Replace USER, the datadir and the state folder for your host. Run it as the
# node's own user: the node writes the signed copy into
# /var/lib/btx/snapshot-confirmer/, and this program reads it back. Make both
# folders before the first start (deploy/esplora/README.md).
#
# THE POINT OF THIS UNIT. easyNode counts a snapshot as confirmed only when
# two operators on its list signed it, each after comparing it with what their
# own node wrote down at that height. An operator whose node is a plain btxd,
# not the easyNode app, confirms through this. It keeps that record, signs
# through the node's own signutxosnapshotmanifest (the key never leaves the
# node), and talks to the node and to easybtx.com and nothing else. It listens
# on nothing. While the node is still checking a snapshot it started from, it
# writes down and signs nothing.
User=USER
Type=simple
ExecStart=/usr/local/bin/btx-confirmer --datadir /var/lib/btx --rpc 127.0.0.1:19334 --state /var/lib/btx-confirmer
# always, as for the witness: a confirmer that has exited has stopped
# confirming. Thirty seconds between tries keeps a node restart, or a node it
# refuses (another chain, no signing key), from filling the log.
Restart=always
RestartSec=30

# It reads the node's cookie, writes its own state and the copies the node
# signs, and makes HTTPS requests to easybtx.com. It needs nothing else.
ReadWritePaths=/var/lib/btx-confirmer /var/lib/btx/snapshot-confirmer
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=read-only
ProtectKernelTunables=true
ProtectControlGroups=true
# AF_UNIX beside the witness's two: it looks up easybtx.com, and on a host
# with systemd-resolved that lookup goes over a local socket.
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
MemoryDenyWriteExecute=true

[Install]
WantedBy=multi-user.target
````

- [ ] **Step 2: The README section**

In `deploy/esplora/README.md`:

*The file table.* Replace:

````markdown
| `btx-witness.service.template` | the witness server as a unit; runs on any node, pruned or not |
````

with:

````markdown
| `btx-witness.service.template` | the witness server as a unit; runs on any node, pruned or not |
| `btx-confirmer.service.template` | the easyNode snapshot confirmer as a unit, for an operator on easyNode's list whose node the app does not run |
````

*The section, after the witness's.* Replace:

````markdown
**Esplora** serves the whole API, balances included, and needs `prune=0`, the
````

with:

````markdown
A **snapshot confirmer** lets an operator on easyNode's snapshot list confirm
snapshots from a plain btxd. easyNode starts new nodes from a snapshot only
when two operators on its list signed it, each after comparing it with what
their own node wrote down at that height; `btx-confirmer` keeps that record
and signs through the node's own `signutxosnapshotmanifest`, so the key never
leaves the node. It needs a node that checks blocks itself, with its signing
key configured (`-matmulattestationsignerkeyfile`), and your key in easyNode's
list (`crates/btx-core/snapshot-operators.json`). While your node is still
checking a snapshot it started from (`getchainstates` shows
`"validated": false`), it writes down and signs nothing; on mainnet that check
can take days. Replace `USER` with the node's user:

```bash
(cd crates/btx-core && cargo build --release --bin btx-confirmer)
sudo install -m755 crates/btx-core/target/release/btx-confirmer /usr/local/bin/
sudo install -d -o USER /var/lib/btx-confirmer /var/lib/btx/snapshot-confirmer
sudo -u USER btx-confirmer --datadir /var/lib/btx --state /var/lib/btx-confirmer --once
sudo cp deploy/esplora/btx-confirmer.service.template /etc/systemd/system/btx-confirmer.service
sudoedit /etc/systemd/system/btx-confirmer.service   # USER and the paths
sudo systemctl daemon-reload && sudo systemctl enable --now btx-confirmer
```

The `--once` run looks once and says what it would do. `install-systemd.sh`
does not install this unit.

**Esplora** serves the whole API, balances included, and needs `prune=0`, the
````

- [ ] **Step 3: Check the words and the unit's shape**

````bash
LC_ALL=C grep -n $'\xe2\x80\x94' deploy/esplora/btx-confirmer.service.template && echo "EM-DASH FOUND" || echo "no em-dash"
grep -c '^ReadWritePaths=' deploy/esplora/btx-confirmer.service.template
for k in User=USER Restart=always NoNewPrivileges=true PrivateTmp=true ProtectSystem=strict ProtectHome=read-only ProtectKernelTunables=true ProtectControlGroups=true MemoryDenyWriteExecute=true; do grep -q "^$k" deploy/esplora/btx-confirmer.service.template || echo "missing $k"; done
command -v systemd-analyze >/dev/null && systemd-analyze verify deploy/esplora/btx-confirmer.service.template || echo "no systemd here; skipped"
````

Expected: `no em-dash`; `1`; nothing missing; on macOS `no systemd here; skipped` (on a Linux host `systemd-analyze verify` reports only that `USER` is not a user and that `/usr/local/bin/btx-confirmer` is missing if it is not installed). The README section added no em-dash either: `git diff -- deploy/esplora/README.md | LC_ALL=C grep -n $'^+.*\xe2\x80\x94'` prints nothing.

- [ ] **Step 4: Commit**

````bash
git add deploy/esplora/btx-confirmer.service.template deploy/esplora/README.md
git commit -m "deploy: btx-confirmer as a systemd unit, and how to run it beside a plain btxd" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 9: The rehearsal: five engines, `btx-confirmer` and the website's own code on regtest

(Was Task 8. New here: one of the two confirmers is `btx-confirmer` beside a plain btxd; depth 144 and two exports waiting at once on a real engine; the validated gate on a real `getchainstates` answer; a real diary mismatch that sends one dissent, disputes the height and leaves `latest` serving nothing. Kept: the wrong file hash, the one-operator statement, the two co-signatures, the mirror's load through the app's loader, the held branch.)

**Files:**
- Create: `crates/btx-core/tests/snapshot_network_regtest.rs`

**Interfaces:**
- Consumes: everything above through the public API, the `btx-confirmer` binary (`env!("CARGO_BIN_EXE_btx-confirmer")`, which cargo sets for this crate's integration tests), plus `attested_snapshot::{prepare_confirmed, Latest}`, `confirmed_load::{node_view, load, CliRunner, Holds}` (`load` with its trailing `datadir`, which writes the start record) and `node::validating_snapshot_pin_args` (core plan, Tasks 3, 4 and 6, as amended), and the website plan's `site/scripts/snapshot-rendezvous-local.mjs` (its stand-in task), run with `SNAPSHOT_REGTEST_OPERATORS` and the regtest pins for `latest` (`SNAPSHOT_REGTEST_PINS`, the website plan's name).
- Produces: `producers_confirmers_the_website_and_a_mirror_on_regtest`, `#[ignore]`, opt-in by `EASYNODE_TEST_BTXD` and `EASYNODE_TEST_SITE_DIR`.

**Which website:** the local stand-in, `node scripts/snapshot-rendezvous-local.mjs`, not `vercel dev`. It serves the same handler functions the Astro routes wrap (`site/src/lib/snapshotRoutes.mjs`), starts in under a second with nothing but `npm ci`, and needs no Vercel login or Blob token (a local Vercel would still talk to the real Blob store). The stand-in is what the test runs.

**Why the rehearsal never calls `mature_step`:** its offer ends with `bounce_mirror_links`, which dials the mainnet mirrors (`signer::BTX_MIRRORS_FED_BY_SIGNERS`) from a regtest node. The unit tests of Task 4 cover it; here the producer's pairs are exported into the waiting set by `export_at_tip` and checked by `check_before_send`, as `mature_step`'s hook does.

What it proves, in order, each against real v0.34.9 engines on regtest (derived, not run; the original, with one confirmer less and a depth of 10, passed 3 runs in a row on 2026-09-29, about 65 s each; this one mines about 540 blocks, so expect a few minutes):

1. P, C, D and R share a chain to 100; P and C write their diaries there through the app's functions, D through `btx-confirmer --once`; P exports at 100 into its waiting set.
2. At 150 the chain splits: R takes C with it, D stays with P. P and D reach exactly 200 and write it down; P exports at 200 while 100 still waits (two waiting, on a real engine). R reaches 200 on its branch and exports there, as a rogue would.
3. **Refused, less than 144 deep:** at 242 P's own export at 100 is `TooShallow { have: 143, need: 144 }`; at 243 it passes, and its statement matches P's diary on block, coins and transactions. At 343 P's export at 200 passes too.
4. **Refused, a wrong file hash:** the website answers 422 `the file does not match the statement` to a file with one bit flipped, and keeps nothing.
5. **Refused, one operator:** P's real statement and file are stored; `latest` names nothing.
6. **Works:** C (the app's confirmer) co-signs; `latest` names 100 with operators `producer, confirmer1`; `btx-confirmer` beside D co-signs too; `latest` names `producer, confirmer1, confirmer2`.
7. **Works, the load:** mirror M, pinning only P, loads the confirmed pair through `prepare_confirmed` and `confirmed_load::load`, trimmed to P's one signature; its snapshot chainstate's base is P's block 100.
8. **Refused, an unvalidated snapshot chainstate:** M's real `getchainstates` answer has two chainstates, the snapshot one `"validated": false`, the shape a plain assumeutxo load gives (a plain `loadtxoutset` cannot be rehearsed on a fresh regtest chain, see the test's doc). On it the gate is closed, the diary writes nothing at 100, and the app's confirmer asks the website nothing. `btx-confirmer` refuses to run beside M (a mirror). Restarted as V, a validating node on that snapshot with a listed key, `btx-confirmer --once` writes nothing down and signs nothing while V's background check is not done.
9. **Refused, a held branch, and a disagreement disputes it:** R sends its statement at 200. C, on R's branch 144 deep, with a hold on R's first block, neither signs nor dissents. `btx-confirmer` beside D, whose diary has P's block at 200, sends one signed dissent; the height is disputed and `latest` answers `{"disputed": [200]}`, serving no snapshot at any height although 100 is confirmed. A second run sends no second dissent, and nobody signs the dissent. An honest producer on R's branch, knowing the hold, does not send R's statement.

- [ ] **Step 1: Write the test**

Create `crates/btx-core/tests/snapshot_network_regtest.rs`:

````rust
//! The snapshot network end to end, on regtest: five real engines,
//! `btx-confirmer` and the website's own route code, through the app's own
//! functions. Opt-in:
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
//! `EASYNODE_TEST_BTX_CLI`. `btx-confirmer` is this crate's own binary, which
//! cargo builds for the test. Keys are made fresh for each run.
//!
//! The cast: P, the producer; C, a confirmer through the app's functions; D,
//! a plain btxd with `btx-confirmer --once` beside it; M, a mirror that pins
//! only P, as mainnet mirrors pin the 3060, later restarted as V, a
//! validating node on the snapshot it loaded; R, a rogue producer whose key
//! is on the list. The list is `producer=P;confirmer1=C;confirmer2=D;
//! rogue=R;late=V` in `EASYNODE_REGTEST_OPERATORS`'s format, handed to the
//! app's functions, to `btx-confirmer` and to the website, which also pins P
//! for `latest`. The chain: all on one to 150; then R, with C following, and
//! P, with D following, each to 343. P exports at 100 and 200, R at 200.
//!
//! Proven refused: an export less than 144 deep; a file whose hash is not
//! the statement's (the website keeps nothing); a statement one operator
//! signed (`latest` names nothing); an unvalidated snapshot chainstate (on
//! the real `getchainstates` answer after a load, the diary writes nothing
//! and the app's confirmer asks nothing; `btx-confirmer` refuses a mirror and
//! signs nothing beside a validating node still checking its snapshot); a
//! statement on a held branch (the confirmer on it neither signs nor
//! dissents, an honest producer does not send it); a diary mismatch
//! (`btx-confirmer`, whose diary has P's block at 200, sends one signed
//! dissent against R's statement, the height is disputed, `latest` serves
//! nothing at any height, and a second run sends no second dissent). Proven
//! working: two exports waiting at once, each matching its producer's diary;
//! two confirmers co-signing, one of them `btx-confirmer`; `latest` pointing
//! at the statement; the mirror loading it through the app's loader, trimmed
//! to the one key it pins.
//!
//! Regtest facts this test relies on, measured with v0.34.9 on 2026-09-29:
//! blocks above 100 connect with `-allowunverifiablematmulconsensus=1` on a
//! device that fails the engine's self-test; the miner's own block 100
//! connects only after a restart; a validating node that syncs from the
//! producer connects 100 without one; at 100 `gettxoutsetinfo` and
//! `getchaintxstats` give the statement's UTXO hash, coin count (101) and
//! transaction count (101); a mirror connects blocks only to 99 from a
//! producer that serves no attestations. Read in the engine's code at
//! 84b998b4, not run: a plain `loadtxoutset` is refused on a fresh regtest
//! chain (only the compiled heights 110 and 299 are known, by their content
//! hash, and 61,010 by height alone: `src/kernel/chainparams.cpp:2553-2582`,
//! `src/kernel/chainparams.h:156-160`, `src/validation.cpp:17510-17528`), so
//! the plain assumeutxo answer is covered by the unit tests, and the signed
//! load here gives the same `getchainstates` shape
//! (`src/rpc/blockchain.cpp:5211-5212`).

use btx_core::confirmed_load::{self, CliRunner, Holds};
use btx_core::confirmed_snapshot as cs;
use btx_core::diary::{self, DiaryOutcome};
use btx_core::known_invalid::HeldBranch;
use btx_core::operators::Chain;
use btx_core::rpc::{Rpc, RpcClient};
use btx_core::snapshot_confirmer::{Confirmer, Outcome, Round, UNVALIDATED_LINE};
use btx_core::snapshot_serve::{self as serve, manifest_file_name, snapshot_file_name};
use btx_core::snapshot_site::{self as site, Latest, SiteError};
use btx_core::statement_check::Mismatch;
use btx_core::{attested_snapshot, snapshot_producer as producer};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
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

    /// Where the engine keeps regtest's chain and its `.cookie`: also where
    /// the app keeps the diary and the snapshot pairs in this test, and the
    /// `--datadir` `btx-confirmer` gets.
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
    async fn start(site_dir: &Path, store: &Path, operators: &str, pins: &str) -> Self {
        let child = std::process::Command::new("node")
            .current_dir(site_dir)
            .args(["scripts/snapshot-rendezvous-local.mjs", "--port"])
            .arg(SITE_PORT.to_string())
            .arg("--dir")
            .arg(store)
            .env("SNAPSHOT_REGTEST_OPERATORS", operators)
            .env("SNAPSHOT_REGTEST_PINS", pins)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("node scripts/snapshot-rendezvous-local.mjs");
        let s = Site(child);
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if latest().await.is_ok() {
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

/// What `latest` answers now, read by the app's own client: `None` when
/// nothing is confirmed.
async fn latest() -> Result<Option<Latest>, SiteError> {
    site::get_latest(&site::client().unwrap(), &base(), "regtest").await
}

/// `latest`'s pointer: its height, statement hash and the operators the
/// website names.
async fn confirmed_now() -> (u64, String, Vec<String>) {
    match latest().await {
        Ok(Some(Latest::Confirmed(p))) => (
            p.height,
            p.statement_hash.to_ascii_lowercase(),
            p.operators,
        ),
        other => panic!("latest names nothing confirmed: {other:?}"),
    }
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

/// Mine on `rpc` until it has a header at `to`, ten a call, and not one
/// more: several steps need the tip exactly on a grid height. Headers, not
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
    match diary::record_at_tip(rpc, &node.net(), Some("consensus"))
        .await
        .unwrap()
    {
        DiaryOutcome::Recorded(e) => e,
        other => panic!("{other:?}"),
    }
}

fn manifest_at(node: &Node, height: u64) -> Vec<u8> {
    std::fs::read(node.snapshots().join(manifest_file_name(height))).unwrap()
}

/// One round of the app's confirmer on `node`.
async fn confirm(rpc: &RpcClient, node: &Node, key: &str, holds: Holds<'_>, env: &str) -> Round {
    let client = site::client().unwrap();
    let base = base();
    let c = Confirmer {
        rpc,
        client: &client,
        site: &base,
        state_dir: &node.net(),
        node_dir: &node.net(),
        our_key: OnceLock::from(btx_core::operators::parse_key(key).unwrap()),
        holds,
        regtest_env: Some(env),
    };
    c.round(Chain::Regtest).await.unwrap()
}

fn done(r: Round) -> Vec<Outcome> {
    match r {
        Round::Done(o) => o,
        Round::Unvalidated => panic!("the gate was closed"),
    }
}

fn outcome_at(v: &[Outcome], height: u64) -> &Outcome {
    v.iter()
        .find(|o| o.height() == height)
        .unwrap_or_else(|| panic!("no outcome at {height}: {v:?}"))
}

/// `btx-confirmer --once` beside `node`: its exit code and what it printed.
fn confirmer_once(node: &Node, state: &Path, env: &str) -> (i32, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_btx-confirmer"))
        .arg("--datadir")
        .arg(node.net())
        .arg("--rpc")
        .arg(format!("127.0.0.1:{}", node.rpc_port))
        .arg("--state")
        .arg(state)
        .arg("--once")
        .env("EASYNODE_REGTEST_OPERATORS", env)
        .env("EASYNODE_SNAPSHOT_SITE", base())
        .output()
        .expect("run btx-confirmer");
    let said = String::from_utf8_lossy(&out.stderr).into_owned();
    eprintln!("btx-confirmer beside {}:\n{said}", node.dir.display());
    (out.status.code().unwrap_or(-1), said)
}

fn snapshot_base(chainstates: &Value) -> Option<String> {
    chainstates["chainstates"]
        .as_array()?
        .iter()
        .find_map(|c| c["snapshot_blockhash"].as_str().map(str::to_string))
}

/// The chainstate built from a snapshot, in a real `getchainstates` answer.
fn snapshot_chainstate(answer: &Value) -> Option<&Value> {
    answer["chainstates"]
        .as_array()?
        .iter()
        .find(|c| c.get("snapshot_blockhash").is_some())
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
    let (v_wif, v_pub) = regtest_key();
    let env = format!(
        "producer={p_pub};confirmer1={c_pub};confirmer2={d_pub};rogue={r_pub};late={v_pub}"
    );
    let _site = Site::start(&site_dir, &root.path().join("site"), &env, &p_pub).await;
    let client = site::client().unwrap();
    let web = base();
    let d_state = root.path().join("d-state");
    assert_eq!(latest().await, Ok(None));

    // ── One chain to 100, and every validating node's diary there ──────────
    let mut p = Node::new(&btxd, root.path().join("p"), 29621, validating(&p_pub));
    let mut c = Node::new(&btxd, root.path().join("c"), 29622, validating(&c_pub));
    let mut d = Node::new(&btxd, root.path().join("d"), 29623, validating(&d_pub));
    let mut r = Node::new(&btxd, root.path().join("r"), 29625, validating(&r_pub));
    for (n, wif) in [(&p, &p_wif), (&c, &c_wif), (&d, &d_wif), (&r, &r_wif)] {
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
    let rr = r.start().await;
    for n in [&rc, &rd, &rr] {
        connect(n, &p).await;
    }
    for n in [&rc, &rd, &rr] {
        wait_for(n, 100).await;
    }
    let p_100 = record_diary(&rp, &p).await;
    assert_eq!(record_diary(&rc, &c).await.block_hash, p_100.block_hash);
    let (code, said) = confirmer_once(&d, &d_state, &env);
    assert_eq!(code, 0, "{said}");
    assert!(
        said.contains(&format!("diary: wrote 100 (block {}", p_100.block_hash)),
        "{said}"
    );
    let w100 = serve::export_at_tip(&rp, &p.snapshots(), 100).await.unwrap();
    assert_eq!(
        (w100.height, w100.block_hash.as_str()),
        (100, p_100.block_hash.as_str())
    );

    // ── The split at 150: R takes C with it, D stays with P ────────────────
    mine_to(&rp, 150).await;
    for n in [&rc, &rd, &rr] {
        wait_for(n, 150).await;
    }
    let _ = rr.call("disconnectnode", json!([p.addr()])).await;
    let _ = rc.call("disconnectnode", json!([p.addr()])).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    connect(&rc, &r).await;

    // ── P and D at exactly 200: both diaries, and a second export waiting ──
    mine_to(&rp, 200).await;
    wait_for(&rd, 200).await;
    let p_200 = record_diary(&rp, &p).await;
    let (code, said) = confirmer_once(&d, &d_state, &env);
    assert_eq!(code, 0, "{said}");
    assert!(
        said.contains(&format!("diary: wrote 200 (block {}", p_200.block_hash)),
        "{said}"
    );
    let w200 = serve::export_at_tip(&rp, &p.snapshots(), 100).await.unwrap();
    assert_eq!(w200.block_hash, p_200.block_hash);
    assert_eq!(
        serve::load_waiting(&p.snapshots())
            .iter()
            .map(|w| w.height)
            .collect::<Vec<_>>(),
        vec![100, 200],
        "two exports wait at once"
    );

    // ── R and C at exactly 200 on R's branch: the rogue's export ───────────
    mine_to(&rr, 200).await;
    wait_for(&rc, 200).await;
    let rogue = serve::export_at_tip(&rr, &r.snapshots(), 100).await.unwrap();
    assert_ne!(rogue.block_hash, p_200.block_hash, "two branches");
    let held_root = call(&rr, "getblockhash", json!([151])).await;
    let held_root: &'static str =
        Box::leak(held_root.as_str().unwrap().to_string().into_boxed_str());

    // ── Refused: an export less than 144 deep; then it passes ──────────────
    let m100 = manifest_at(&p, 100);
    mine_to(&rp, 242).await;
    assert_eq!(
        producer::check_before_send(&rp, &m100, &p.net(), &Holds::none(), Some(&env)).await,
        Err(Mismatch::TooShallow {
            have: 143,
            need: 144
        })
    );
    mine_to(&rp, 243).await;
    producer::check_before_send(&rp, &m100, &p.net(), &Holds::none(), Some(&env))
        .await
        .expect("144 deep, and the producer's own diary agrees");
    let st = cs::parse(&m100).unwrap().statement;
    let hash = st.hash().display_hex();
    assert_eq!(st.block_hash().display_hex(), p_100.block_hash);
    assert_eq!(
        (st.coins(), st.chain_tx()),
        (p_100.coins, p_100.chain_tx),
        "the diary and the export agree on coins and transactions"
    );
    // Both branches to 343: every statement is then 144 deep where it is
    // asked about.
    mine_to(&rr, 343).await;
    wait_for(&rc, 343).await;
    mine_to(&rp, 343).await;
    wait_for(&rd, 343).await;
    producer::check_before_send(&rp, &manifest_at(&p, 200), &p.net(), &Holds::none(), Some(&env))
        .await
        .expect("the second waiting export checks out too");

    // ── Refused: a file whose hash is not the statement's ──────────────────
    let first = site::post_statement(&client, &web, &m100).await.unwrap();
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
        latest().await,
        Ok(None),
        "one operator, file stored: latest names nothing"
    );

    // ── Two confirmers co-sign, one of them btx-confirmer ──────────────────
    let v = done(confirm(&rc, &c, &c_pub, Holds::none(), &env).await);
    assert!(
        matches!(outcome_at(&v, 100), Outcome::Signed { .. }),
        "{v:?}"
    );
    assert_eq!(
        confirmed_now().await,
        (
            100,
            hash.clone(),
            vec!["producer".to_string(), "confirmer1".to_string()]
        )
    );
    let (code, said) = confirmer_once(&d, &d_state, &env);
    assert_eq!(code, 0, "{said}");
    assert!(said.contains("co-signed the snapshot at 100"), "{said}");
    assert_eq!(
        confirmed_now().await,
        (
            100,
            hash.clone(),
            vec![
                "producer".to_string(),
                "confirmer1".to_string(),
                "confirmer2".to_string()
            ]
        )
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
    let loaded = confirmed_load::load(
        &rm,
        &runner,
        &pair,
        &view,
        &Holds::none(),
        Some(&env),
        &m.net(),
    )
    .await
    .expect("the mirror loads the confirmed snapshot");
    assert_eq!((loaded.height, loaded.signatures), (100, 1));
    let _ = rm.call("disconnectnode", json!([p.addr()])).await;
    let answer = call(&rm, "getchainstates", json!([])).await;
    assert_eq!(snapshot_base(&answer), Some(p_100.block_hash.clone()));

    // ── Refused: an unvalidated snapshot chainstate, on the real answer ────
    // The snapshot chainstate is not validated while the background one
    // exists, the same shape a plain assumeutxo load gives.
    assert_eq!(
        answer["chainstates"].as_array().map(|a| a.len()),
        Some(2),
        "{answer}"
    );
    assert_eq!(
        snapshot_chainstate(&answer).unwrap()["validated"],
        json!(false),
        "{answer}"
    );
    assert!(!diary::gate_open(&rm).await);
    // The mode is the caller's to check; claim consensus here so that the
    // gate is the only thing left to refuse.
    assert_eq!(
        diary::record_at_tip(&rm, &m.net(), Some("consensus"))
            .await
            .unwrap(),
        DiaryOutcome::Unvalidated(100)
    );
    assert!(!diary::diary_path(&m.net()).exists());
    assert_eq!(
        confirm(&rm, &m, &v_pub, Holds::none(), &env).await,
        Round::Unvalidated
    );
    let (code, said) = confirmer_once(&m, &root.path().join("m-state"), &env);
    assert_eq!(code, 1, "btx-confirmer refuses a mirror: {said}");
    assert!(said.contains("trusted mode"), "{said}");
    // ... and beside a validating node still checking that snapshot. Without
    // `-allowunverifiablematmulconsensus=1` and with no peers, a device the
    // engine does not qualify (the owner's Mac) cannot finish the background
    // check; a qualified GPU may finish it at once, and then this part says
    // so and the gate stays proven on M above.
    m.stop(&rm).await;
    std::fs::write(m.net().join("signer.wif"), format!("{v_wif}\n")).unwrap();
    let mut v_args = vec![
        "-matmulvalidation=consensus".to_string(),
        "-connect=0".into(),
        "-matmulattestationsignerkeyfile=signer.wif".into(),
    ];
    v_args.extend(btx_core::node::validating_snapshot_pin_args(
        &m.net(),
        &[p_pub.as_str()],
        &[],
    ));
    m.args = v_args;
    let rv = m.start().await;
    let answer = call(&rv, "getchainstates", json!([])).await;
    if snapshot_chainstate(&answer).is_some_and(|s| s["validated"] == json!(false)) {
        let (code, said) = confirmer_once(&m, &root.path().join("v-state"), &env);
        assert_eq!(code, 0, "{said}");
        assert!(said.contains("not writing down block 100"), "{said}");
        assert!(said.contains(UNVALIDATED_LINE), "{said}");
    } else {
        eprintln!("V finished its background check at once; the gate was shown on M: {answer}");
    }
    let pending = site::get_pending(&client, &web, "regtest").await.unwrap();
    assert!(
        pending
            .statements
            .iter()
            .all(|s| !s.operators.contains(&"late".to_string())),
        "V signed nothing"
    );
    m.stop(&rv).await;

    // ── Refused: a held branch, and a real disagreement disputes it ────────
    producer::submit(&client, &web, &r.snapshots(), 200)
        .await
        .unwrap();
    let rogue_hash = cs::parse(&manifest_at(&r, 200))
        .unwrap()
        .statement
        .hash()
        .display_hex();
    assert_eq!(
        confirmed_now().await.0,
        100,
        "one operator at 200 changes nothing yet"
    );
    let holds_list: &'static [HeldBranch] = Box::leak(Box::new([HeldBranch {
        height: 151,
        root: held_root,
        why: "the rehearsal's held branch",
    }]));
    let holds = Holds {
        invalid: &[],
        held: holds_list,
    };
    // C is on that branch, 144 deep on the rogue's block: with the hold it
    // neither signs nor dissents.
    let v = done(confirm(&rc, &c, &c_pub, holds, &env).await);
    assert_eq!(
        outcome_at(&v, 200),
        &Outcome::Waiting {
            height: 200,
            statement_hash: rogue_hash.clone(),
            why: Mismatch::HeldRootOnActiveChain {
                height: 151,
                root: held_root.into()
            }
        }
    );
    assert!(matches!(outcome_at(&v, 100), Outcome::AlreadySigned { .. }));
    // btx-confirmer beside D wrote down P's block at 200: one signed dissent.
    let (code, said) = confirmer_once(&d, &d_state, &env);
    assert_eq!(code, 0, "{said}");
    assert!(
        said.contains(&format!(
            "sent a dissent about block 200: block hash differs from the diary (diary {}, statement {})",
            p_200.block_hash, rogue.block_hash
        )),
        "{said}"
    );
    assert_eq!(
        latest().await,
        Ok(Some(Latest::Disputed(vec![200]))),
        "no snapshot at any height while the dispute stands"
    );
    let pending = site::get_pending(&client, &web, "regtest").await.unwrap();
    let dissents: Vec<Vec<String>> = pending
        .statements
        .iter()
        .filter(|s| s.dissent)
        .map(|s| s.operators.clone())
        .collect();
    assert_eq!(dissents, vec![vec!["confirmer2".to_string()]]);
    // Once per height, and nobody signs a dissent.
    let (code, said) = confirmer_once(&d, &d_state, &env);
    assert_eq!(code, 0, "{said}");
    assert!(!said.contains("sent a dissent"), "{said}");
    let v = done(confirm(&rc, &c, &c_pub, holds, &env).await);
    assert!(
        !v.iter()
            .any(|o| matches!(o, Outcome::Signed { .. } | Outcome::Dissented { .. })),
        "{v:?}"
    );
    let pending = site::get_pending(&client, &web, "regtest").await.unwrap();
    assert_eq!(pending.statements.iter().filter(|s| s.dissent).count(), 1);
    // An honest producer on that branch, knowing the hold, does not send it.
    let honest =
        producer::check_before_send(&rr, &manifest_at(&r, 200), &r.net(), &holds, Some(&env)).await;
    assert!(honest.is_err(), "{honest:?}");

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

The website plan's tasks up to and including its local stand-in (its Task 6) must be done on `claude/snapshot-rendezvous` in the EasyBTX repository; this test passes the stand-in `SNAPSHOT_REGTEST_PINS=<P's key>`, without which nothing on regtest is ever confirmed. Use its worktree, or make one for this run in your session's scratch folder (`$SCRATCH` below):

````bash
git -C /Users/m2promende/repos/EasyBTX fetch -q origin claude/snapshot-rendezvous
git -C /Users/m2promende/repos/EasyBTX worktree add --detach "$SCRATCH/easybtx-rehearsal" origin/claude/snapshot-rendezvous
(cd "$SCRATCH/easybtx-rehearsal/site" && PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm ci --no-audit --no-fund)
````

Expected: `added … packages`. Never use `/Users/m2promende/repos/EasyBTX` itself (the owner's working tree).

- [ ] **Step 4: Run it against the shipped engine**

`EASYNODE_TEST_BTXD` is a v0.34.9 btxd (commit `84b998b4`), for example the staged package's after `apps/node/scripts/stage-node-pkg.sh` (`apps/node/src-tauri/resources/node-pkg/.../btxd`), with btx-cli beside it. RPC ports 29621 to 29625, P2P ports 29721 to 29725 and port 29650 must be free; Node 22 must be on the PATH.

````bash
cd crates/btx-core
EASYNODE_TEST_BTXD=/path/to/btxd \
EASYNODE_TEST_SITE_DIR="$SCRATCH/easybtx-rehearsal/site" \
  cargo test --locked --test snapshot_network_regtest -- --ignored --nocapture --test-threads=1
````

Expected: `test producers_confirmers_the_website_and_a_mirror_on_regtest ... ok` and `test result: ok. 1 passed` in a few minutes (derived, not run). No btxd, node or btx-confirmer process of the test is left running (`pgrep -fl 'btxd -regtest'`, `pgrep -fl snapshot-rendezvous-local` and `pgrep -fl btx-confirmer` show none from this run). If it stops at "did not reach 100", the miner's block 100 did not connect after the restart: read the tail of `p/regtest/debug.log` in the temp folder the panic names. If it stops at the V part with `btx-confirmer` signing, V's background check finished before the gate could be shown on it; the gate is still proven on M, and the step can be dropped on that machine.

- [ ] **Step 5: Format, lint, commit, and clean up**

````bash
cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cd ../..
git add crates/btx-core/tests/snapshot_network_regtest.rs
git commit -m "core: rehearse the snapshot network on regtest: producer, two confirmers (one btx-confirmer), a mirror, a dispute and the website's own code" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git -C /Users/m2promende/repos/EasyBTX worktree remove --force "$SCRATCH/easybtx-rehearsal"
````

## Names this plan uses from the amended core plan

The core plan was amended in parallel with this one. Every name below was checked against the amended core plan as it stood the night of 29 September (`planb/core.md`, its "Public API other plans use"); each is called from the places listed, so a later rename there changes only those lines.

| Name | What it is | Called here from |
|---|---|---|
| `confirmed_snapshot::SNAPSHOT_GRID: u32` (100) | the grid, every chain (it replaces the core plan's old `MAINNET_GRID` and `REGTEST_GRID`) | `diary::grid_for`, `snapshot_serve::EXPORT_GRID`, `snapshot_confirmer::diary_tick` |
| `confirmed_snapshot::SNAPSHOT_DEPTH: u32` (144) | the depth | `snapshot_serve::CONFIRMATIONS_REQUIRED`; `statement_check`'s tests |
| `confirmed_snapshot::check_shape(&Statement, &ChainRules) -> Result<u64, Refusal>` | chain fields (version, chain id, replay context, shielded commitment), not a dissent, file geometry and size, grid; the height | `statement_check::{verdict, check_against_node}`; `statement_check`'s test of a dissent |
| `confirmed_snapshot::is_dissent(&Statement) -> bool` | all four file fields zero | `snapshot_confirmer::Confirmer::confirm_one`, `statement_check`'s test |
| `confirmed_snapshot::dissent_statement(height: u64, &Hash32 block, &Hash32 hash_serialized, coins: u64, chain_tx: u64, &Hash32 chain_id, &Hash32 replay_context, &Hash32 shielded) -> Option<[u8; STATEMENT_LEN]>` | a dissent's 229 bytes, wrapped with `Statement::from_raw`; `None` when `height` does not fit the statement's 32-bit height field | `statement_check::dissent_from` only |
| `ChainRules::shielded: Hash32` | the shielded commitment compiled for the chain (`MAINNET_SHIELDED_COMMITMENT` `94343b76…4541`, `REGTEST_SHIELDED_COMMITMENT` `e802781d…1d5e`) | `statement_check::dissent_from` and its test |
| `node_api::read_chainstates_validated(&dyn Rpc) -> bool` (over the pure `chainstates_validated(Option<&Value>)`) | the validated gate | `diary::gate_open` only |
| `attested_snapshot::{Latest, parse_latest}` (`Latest::Confirmed(ConfirmedPointer)`, `Latest::Disputed(Vec<u64>)`) | `latest`'s HTTP 200 answer | `snapshot_site::get_latest` (re-exported as `snapshot_site::Latest`); the rehearsal |
| `confirmed_load::load(.., regtest_env, datadir: &Path)` | the one loading path, now writing `snapshot-start.json` | the rehearsal only |
| `operators` compiled from `crates/btx-core/snapshot-operators.json` | the list and the mirrors' pins, and its tests | `operators::for_chain`, `parse_key`, as before (`MAINNET_OPERATORS` is gone and not used here) |

Also used as the core plan already defines them: `node::attested_snapshot_record` (only in a test, to show the file is no longer the test), `node::validating_snapshot_pin_args`, `confirmed_load::{Holds, Holds::none, Holds::compiled, node_view, CliRunner}` (`Holds` is `Copy`), `attested_snapshot::prepare_confirmed`, `confirmed_snapshot::{Hash32, Statement, Manifest, Signed, ChainRules, confirming_operators, signature_is_valid, MAINNET_REPLAY_CONTEXT, REGTEST_REPLAY_CONTEXT, STATEMENT_LEN}`, `node_api::MatmulTrustedStatus::replay_authority_context`.

From the website plan, as amended the same night (checked against `planb/site.md`): `pending` entries carry `dissent`; `latest` answers `{"disputed": [...]}` while a dispute stands and `{"version":1,"confirmed":null}` with 404 when nothing is confirmed; a statement at a closed height is refused with exactly `422 {"error":"closed-height"}`; a dissent's `file` is always `missing`; the stand-in takes the regtest operators from `SNAPSHOT_REGTEST_OPERATORS` and the regtest pins for `latest` from `SNAPSHOT_REGTEST_PINS` (`key,key`).

## Risks and open points (for the owner)

1. **Nothing is confirmed until two listed operators confirm from nodes whose chains rest on their own checks.** Mende, Aleksander and jpp are on the list (the PR that adds it waits for the owner's word on Aleksander, core plan), but each counts only once `getchainstates` shows no unvalidated chainstate on his node: a node that started from any snapshot, plain or signed, writes nothing and signs nothing until its background check finishes, which takes days on mainnet. jpp's box (0.34.12 on upstream's 228,000 snapshot) is held off by exactly this, as he asked. Choice 5 of the design: read each operator's `getchainstates` once before 0.7.0 ships.
2. **A confirmer's diary must have the height.** The refresher reads the tip every 3 seconds and blocks come about every 90, so a grid height is usually caught, but a node that was down or catching up at that moment has no entry and says neither (`NoDiaryEntry`). The next chance is 100 blocks later. The keeper also records right before it exports, so the producer never misses its own height.
3. **The keeper's poll.** An export happens only while the tip is exactly on the grid; the keeper looks every 3 s when the tip is 1 or 2 blocks short and every 30 s otherwise. A block that arrives within those 3 s moves the tip on and that height is skipped.
4. **The producer's checks gate the P2P offer as well**, not only the upload: a keeper whose diary is off (a mirror, or a node on a snapshot still being checked) stops offering new pairs. The 0.6.x mirrors that fetch over P2P lose nothing while the 3060 validates.
5. **An orphaned base is dropped** until the next grid height (about two and a half hours), where the keeper used to dump again at once; a second dump would base on the tip, off the grid.
6. **The rehearsal needs `-allowunverifiablematmulconsensus=1`** for the validating regtest nodes to connect blocks above 100 on a device the engine does not qualify (measured on the owner's M2 Pro). On a qualified card the flag is harmless.
7. **The confirmer trusts the engine's `getmatmultrustedstatus.replay_authority_context`** for the node side; a signing node always reports one (a node that does not is `ReplayContext { node: None }`, neither).
8. **The design's upload path does not fit Vercel as written** (website plan, Risks): parts of at most 4 MiB go up as the website's own private blobs, not as Blob multipart parts, which must be at least 5 MB.
9. **A plain `loadtxoutset` cannot be rehearsed on a fresh regtest chain.** v0.34.9 accepts a plain assumeutxo snapshot on regtest only at its compiled heights 110 and 299, by content hash, and at 61,010 by height alone (`src/kernel/chainparams.cpp:2553-2582`, `src/kernel/chainparams.h:156-160`, `src/validation.cpp:17510-17528` at `84b998b4`). The rehearsal shows the gate on the real answer after a signed load (the same `getchainstates` shape, `src/rpc/blockchain.cpp:5211-5212`) and the unit tests cover the plain shape. The step beside a validating node (V) is shown only where the engine cannot finish V's background check at once (the owner's Mac); on a qualified GPU it says so and the gate stays proven on the mirror.
10. **`btx-confirmer` learns its node's key from the engine's first signature.** No RPC names the local signer's key and it does not read the key file, so after each start the first statement it would sign or dissent on costs one engine signature before it knows whether its operator signed already; it never posts twice (the website merges one signature per key, and dissents are recorded per height). An unlisted key is found out the same way and sends nothing.
11. **`btx-confirmer` refuses more than the design lists at start:** besides another chain id or replay context, a node that follows signatures and a node with no signing key, since it could neither check nor sign. The design says only chain id and replay context; the owner may want the other two as warnings instead.
12. **Two small additions to what the design names for `btx-confirmer`:** a `--once` flag (one look and one round, for the operator's first check and for the rehearsal), and in its unit `AF_UNIX` beside `AF_INET AF_INET6` (it looks up easybtx.com, which on a systemd-resolved host goes over a local socket) and `RestartSec=30` (the witness has 5).
13. **Any listed operator can freeze Fast-forward with a dissent, on purpose or through a bug in its diary** (design, section 6a). The dissent names who sent it, and the owner's clear closes the height. A confirmer that dissented at a height also dissents again after the owner's clear only at a newer height; the closed one is refused by the website (`Closed`, never retried).
14. **A dissent and a signature from the same operator can meet at one height** after a reorg deeper than 144 blocks changes that operator's own diary; the website then records a dispute, which is the right answer for a reorg that deep.
15. **Test counts are derived, not run.** The original plan was dry-run on a copy of `cb0316b`; this amendment was not. Every count says so.
16. **The Tauri crate does not build between Task 4 and Task 6** (the keeper still calls `refresh_due` and `run_cycle` until Task 6 replaces them); run only the btx-core gates in Tasks 4 and 5.
17. **The core plan was amended in parallel.** This plan was aligned with it as it stood the night of 29 September (the table above); a later change to one of those names changes only the call sites listed there.

## Self-review

- Spec coverage: section 2, the grid of 100 (`SNAPSHOT_GRID`, `EXPORT_GRID`, `export_due`, `grid_for`, Tasks 1 and 4); section 3, the diary on every validating node at grid heights only, only when `height` and `bestblock` are that block, newest 100, atomic, nothing while `getchainstates` shows an unvalidated chainstate (plain and signed snapshot alike, the file no longer the test), entries dropped when their block leaves the chain and never compared (Task 1; the verdict's `DiaryEntryLeftChain`, Task 2; the refresher, Task 6); section 4, producers export exactly on the grid and only with the gate open, wait per height until 144 deep with two waiting at once, six-hour deadline, four pairs, re-check canonical, held roots refused, the verdict must be Sign, send only when listed, never send a failed pair, keep offering over P2P (Tasks 4 and 6); section 5, confirmers every ten minutes, the gate first, the four checks at 144 deep, sign on a match, dissent on a chain-field mismatch, neither otherwise, never without a diary match, dissents never signed, every mismatch logged and counted in Copy diagnostics with dissents sent and gate-closed skips (Tasks 2, 5 and 6); section 5a, `btx-confirmer` beside a plain btxd on the same code, refusing a mismatched chain id or replay context, `--datadir`, `--rpc`, `--state`, the copy under `<datadir>/snapshot-confirmer/` handed over by its relative path, its unit with `ReadWritePaths` the two folders only, the README section, `install-systemd.sh` untouched (Tasks 7 and 8); the app's side of 6 and 6a, the client with the dissent flag, the disputed `latest`, the closed height, the dissent built from the diary with the compiled chain id, replay context and shielded commitment and zero file fields, posted once per height (Tasks 2, 3 and 5). The website, the dispute rule, the alert and the owner's clear are the website plan; loading, the start record and the words that name operators are the core and Fast-forward plans.
- Sabotage tests, one per rule: a diary field that differs, a dissent (`a_chain_field_that_differs_from_the_diary_is_a_dissent`, `a_chain_field_that_differs_is_answered_with_one_signed_dissent`, rehearsal step 9); no diary entry, an entry off the chain, fewer than 144 deep, a foreign chain id, replay context or shielded commitment, neither (`nothing_is_said_without_a_diary_entry_on_this_chain`, `a_block_less_than_144_deep_waits`, `a_foreign_chain_replay_context_or_shielded_commitment_is_neither`, rehearsal step 3); the validated gate on the plain assumeutxo answer with no `attested_assumeutxo` file, on the signed one, on a failed answer, and open again when every chainstate is validated (`an_unvalidated_snapshot_chainstate_writes_nothing`, `an_unvalidated_snapshot_chainstate_is_neither_whatever_the_diary_says`, `a_producer_on_an_unvalidated_snapshot_chainstate_sends_nothing`, `a_node_on_an_unvalidated_snapshot_chainstate_asks_nothing_and_signs_nothing`, `btx_confirmers_diary_look_is_cheap_off_the_grid`, rehearsal step 8); an entry whose block left the chain dropped (`an_entry_whose_block_left_the_chain_is_dropped`); off-grid (`an_off_grid_statement_is_neither_before_the_chain_is_asked`, `an_export_that_landed_off_the_grid_is_not_kept`, `an_export_is_due_only_on_the_grid_above_the_offered_base_and_once`); a held branch (`a_refused_block_under_the_base_is_neither`, `a_held_block_is_refused_first_and_a_base_above_it_is_not_sent`, rehearsal step 9); two exports waiting (`two_exports_wait_at_once_and_each_is_offered_in_turn`, rehearsal step 2); the six-hour deadline (`a_base_not_deep_enough_after_six_hours_is_dropped`); a dissent sent once per height and never signed (`a_chain_field_that_differs_is_answered_with_one_signed_dissent`, `a_dissent_this_operator_has_on_the_website_counts_as_sent`, `a_pending_dissent_is_never_signed`, rehearsal step 9); a closed height (`a_closed_height_is_told_apart`, `a_closed_height_is_not_tried_again`); a file whose hash does not match, discarded (rehearsal step 4); `latest` never pointing at a one-operator statement and serving nothing while a dispute stands (rehearsal steps 5 and 9); a producer that fails its checks offers and sends nothing (`a_pair_the_checks_refuse_is_dropped_and_the_old_offer_stays`, `a_producer_whose_diary_disagrees_sends_nothing`); an engine that signs with the wrong key (`an_engine_that_signs_with_another_key_sends_nothing`); `btx-confirmer` beside the wrong node (`btx_confirmer_refuses_a_node_it_cannot_confirm_beside`, rehearsal step 8) and with an unlisted key (`btx_confirmer_learns_its_key_from_the_first_signature`).
- Placeholders: none. Every code step is the code to write; the counts are derived, not run. The values an engineer supplies are the path to btxd and a scratch folder, as in the core plan.
- Consistency: names checked across tasks (`Mismatch`, `Verdict`, `NodeFacts`, `verdict`, `check_against_node`, `dissent_from`, `Outcome`, `Round`, `Tally`, `NetworkReport`, `ProducerChecks`, `BeforeOffer`, `OfferCheck`, `MatureOutcome`, `Waiting`, `EXPORT_GRID`, `CONFIRMATIONS_REQUIRED`, `record_at_tip`, `gate_open`, `check_before_send`, `submit`, `check_node`, `diary_tick`, `WORK_DIR`, `DISSENTS_FILE`, `UNVALIDATED_LINE`), against the amended core plan (the table above) and against the amended website plan (`x-ebtx-node: ebtx-snapshot-v1`, the reply fields, `part_bytes`, `parts`, `manifest_hex`, `dissent`, `disputed`, `closed-height`, `SNAPSHOT_REGTEST_PINS`). Every anchor in `snapshot_serve.rs`, `commands.rs`, `state.rs`, `tools.rs`, `diagnostics.rs`, `lib.rs`, `index.html`, `main.ts` and `deploy/esplora/README.md` was re-read on `origin/main` (`b330e3d`); the engine lines cited were read at `84b998b4`. No em-dash in any string, comment or doc line this plan adds.
