# Catch-up Help Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** While a node is 20 or more blocks behind and its engine has asked nobody for the next block for 30 seconds, the app asks its own archive peers for the next 100 blocks by name (`getblockfrompeer`), waits for them to connect, and repeats, until the engine fetches on its own again or the node is within 20 blocks.

**Architecture:** Two new pure-ish modules in btx-core. `header_path.rs` walks the header chain from a target down to the tip with `getblockheader`, keeps it between calls and extends it instead of re-walking; the Tools button's "Fetch a stuck block" uses the same walk. `catchup_assist.rs` holds a pure state machine (`Helper::decide`, tested with synthetic clocks) and one async `tick` that reads what the decision needs, sends the requests and reports back. The Tauri status refresher calls `tick` once per 3-second tick with the reads it already makes; the watchdog's `BlockFetchGated` sentence names the help.

**Tech Stack:** Rust 2021 (stable toolchain, rustc 1.95 on the owner's Mac), tokio, serde_json, async-trait; Tauri 2 shell; engine btxd v0.34.9 (RPC `getblockfrompeer`, `getblockheader`, `getmatmulattestedtip`, `getpeerinfo`, `getchaintips`).

**Design:** `docs/decisions/2026-09-29-every-node-starts-near-the-tip.md` section 11, approved 2026-09-29, on branch `origin/claude/cosigned-snapshots`. This plan covers section 11, its line in "Rollback" (the help sits behind one constant), its line in "Tests" ("Catch-up help: target and peer choice over recorded RPC answers"), and the watchdog wording change section 11 names. Nothing else from that document.

## Global Constraints

- Help only when the followed chain is **at least 20 blocks** above the tip (`MIN_BEHIND = 20`).
- Start only after **the next block has not been requested from anyone for 30 seconds** (`QUIET = 30 s`).
- Ask for **the next 100 blocks** by name, from one peer (`BATCH = 100`), then **wait for them to connect** before asking for more.
- **Rotate to the next archive peer if a batch has not connected within three minutes** (`ROTATE_AFTER = 180 s`).
- Follow the chain toward **the signed frontier the node reads** (`getmatmulattestedtip`, `signed_frontier.hash`); a node that reads none follows **its best header** (`stuck_blocks::target_tip`).
- Ask only **the archive peers the app dials**: `connection_type == "manual"` in `getpeerinfo` and the address in `btx_core::node::block_source_peers()`.
- **Never add, ban or disconnect a peer.** The only state-changing command the help sends is `getblockfrompeer`. A test fails on any other.
- **Never fetch a refused block**: no request toward a chain that passes an entry of `known_invalid::KNOWN_INVALID_BLOCKS` or `known_invalid::HELD_BRANCHES`, and none while the node's own chain contains one.
- **Stop as soon as the engine fetches on its own again**: any block among the next 100 heights in flight at any peer, other than the batch this help has out.
- The whole help sits behind **one constant**, `catchup_assist::ENABLED`.
- The walk is cheap when repeated: headers read once are kept; at most **1,000 headers read per tick** (`WALK_PER_TICK`).
- User-facing copy: friendly, simple, no hype, no guarantees, **no em-dashes**.
- CI enforces, in both `crates/btx-core` and `apps/node/src-tauri`: `cargo fmt --all --check` and `cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious`, plus `cargo test --locked`; and `npx tsc --noEmit` and `npm test` in `apps/node`.
- Every commit message ends with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## The evidence this plan relies on

- **Mainnet, 2026-09-29, owner's Mac, engine v0.34.9.** A mirror started from the signed snapshot at 225,927 got 225,928 after 2.5 minutes and 225,929 after 2.3 more. Asking 109.199.124.187 by name, 100 at a time and waiting for each batch to connect, moved it 7,490 blocks in 8.0 minutes (about 940 a minute) to the tip, after which it followed new blocks unassisted. Script: `scratchpad/mainnet/assist.py`; log: `scratchpad/mainnet/evidence/assist.log` (session scratchpad of 2026-09-29).
- **Engine source, tag v0.34.9** (`~/repos/btx`, `git show v0.34.9:<path>`):
  - `src/net_processing.cpp:6992`: the block scheduler skips a NETWORK_LIMITED peer for any block 286 or more below that peer's best block. This is why nothing is requested.
  - `src/net_processing.cpp:290` and `:3643`: the engine's own rescue re-requests the tip's next block only after it has been the stuck root for 120 seconds.
  - `src/net_processing.cpp:8458` (`FetchBlock`): `getblockfrompeer` marks the block in flight at the peer asked, so the help's own requests show up in `getpeerinfo.inflight`. Confirmed on regtest the same day.
  - `src/rpc/blockchain.cpp:742`: a block the node already has answers `RPC_MISC_ERROR` "Block already downloaded". The help counts that as taken.
  - `src/net_processing.cpp:9385`: a NETWORK_LIMITED node disconnects a requester that asks for a block more than 290 below its tip **unless it grants that requester `noban`**. See "Risks" at the end.
- **Regtest dry run, 2026-09-29, this plan's code** (two v0.34.9 nodes, Task 6's test): with the help ticking, node B went from 1 to 99 in under a minute; without it, B stayed at 1. In both runs the peer connection dropped after 11 and 16 of the first 100 requests; the help paused, then asked again and finished.
- **Measured cost:** `getblockheader` over local RPC took 0.155 ms per header on regtest (421 headers in 65 ms), so a 1,000-header tick costs well under a second.

## Before you start

The code this plan builds on (`stuck_blocks.rs`, `PeerInfo::synced_headers/synced_blocks`, `tools.rs`) is on the **local** branch `claude/tools-command-window` in `/Users/m2promende/repos/easynode` (at `62a03f1` when this plan was written, and still moving). `origin/claude/tools-command-window` has only the Tools plan and decision (`bb92b4e`). Branch from the local branch:

```bash
cd /Users/m2promende/repos/easynode
git log --oneline -1 claude/tools-command-window
git worktree add -b claude/catch-up-help .claude/worktrees/catch-up-help claude/tools-command-window
cd .claude/worktrees/catch-up-help
mkdir -p apps/node/src-tauri/resources/node-pkg
echo "CI placeholder. Not a node package." > apps/node/src-tauri/resources/node-pkg/CI-PLACEHOLDER
git status --short
```

Expected: the log line prints the branch head; `git status --short` prints nothing (the `node-pkg` folder is gitignored; the placeholder only satisfies `tauri.conf.json`'s bundle glob so the shell builds, exactly as `.github/workflows/ci.yml` does). All paths below are relative to this worktree. Line numbers were verified at `62a03f1`; if the branch moved, find each spot by the quoted text instead.

## File structure

| File | Change | Responsibility |
|---|---|---|
| `crates/btx-core/src/node_api.rs` | modify | `AttestedTip` also carries `signed_frontier.hash` |
| `crates/btx-core/src/known_invalid.rs` | modify | `refused_blocks()`: every refused (height, hash) in one list |
| `crates/btx-core/src/header_path.rs` | create | the cached walk from a target header down to the tip; shared by the help and Tools |
| `crates/btx-core/src/catchup_assist.rs` | create | the decision (`Helper`), the target choice, the log lines, and the async `tick` |
| `crates/btx-core/src/lib.rs` | modify | `pub mod catchup_assist;`, `pub mod header_path;` |
| `crates/btx-core/tests/catchup_regtest.rs` | create | opt-in test against two real engines on regtest |
| `crates/btx-core/src/watchdog.rs` | modify | `BlockFetchGated` names the help |
| `apps/node/src-tauri/src/tools.rs` | modify | "Fetch a stuck block" walks with `HeaderPath` |
| `apps/node/src-tauri/src/commands.rs` | modify | the status refresher ticks the help |
| `apps/node/CHANGELOG.md` | modify | one entry under Unreleased |

No UI file changes: the design names no screen for the help beyond the watchdog sentence, which the validation card already shows (`apps/node/src/validation.ts`, `status.stall.summary`).

---

### Task 1: What the help reads: the frontier's hash and the refused blocks

**Files:**
- Modify: `crates/btx-core/src/node_api.rs:531-561` (`AttestedTip`, `get_attested_tip`), tests at `:1119`
- Modify: `crates/btx-core/src/known_invalid.rs:123` (after `LIFTED_BRANCHES`), tests at `:299`

**Interfaces:**
- Produces: `node_api::AttestedTip { .., pub hash: Option<String> }`, filled from `signed_frontier.hash`.
- Produces: `known_invalid::refused_blocks() -> impl Iterator<Item = (u64, &'static str)>`, the known-invalid block and every held root.

- [ ] **Step 1: Write the failing tests**

In `crates/btx-core/src/node_api.rs`, inside `mod tests`, directly above `async fn parses_mining_info_with_chain_guard()` (its `#[tokio::test]` line), insert:

```rust
    #[tokio::test]
    async fn attested_tip_reads_the_signed_frontier_hash() {
        let hash = "ab".repeat(32);
        let rpc = FakeRpc::new(&[(
            "getmatmulattestedtip",
            json!({"configured": true, "signed_frontier": {
                "height": 233475, "hash": hash, "on_active_chain": true,
                "on_chain_attested_height": 225927, "blocks_behind": 7548}}),
        )]);
        let t = get_attested_tip(&rpc).await.unwrap();
        assert_eq!(t.height, Some(233_475));
        assert_eq!(t.hash.as_deref(), Some(hash.as_str()));
        let unpinned = FakeRpc::new(&[("getmatmulattestedtip", json!({"configured": false}))]);
        assert_eq!(
            get_attested_tip(&unpinned).await.unwrap(),
            AttestedTip::default()
        );
    }

```

In `crates/btx-core/src/known_invalid.rs`, inside `mod tests`, directly above `fn refusal_is_on_unless_the_operator_says_otherwise()` (its `#[test]` line), insert:

```rust
    #[test]
    fn refused_blocks_names_the_invalid_block_and_every_held_root() {
        let all: Vec<(u64, &str)> = refused_blocks().collect();
        assert_eq!(all.len(), KNOWN_INVALID_BLOCKS.len() + HELD_BRANCHES.len());
        assert!(all.contains(&(227_313, KNOWN_INVALID_BLOCKS[0].hash)));
        for b in HELD_BRANCHES {
            assert!(all.contains(&(b.height, b.root)), "{b:?}");
        }
    }

```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib attested_tip_reads 2>&1 | grep -E "^error"`
Expected: the crate does not compile, with `error[E0609]: no field `hash` on type `AttestedTip`` and `error[E0425]: cannot find function `refused_blocks` in this scope`.

- [ ] **Step 3: Implement**

In `crates/btx-core/src/node_api.rs`, replace the end of `pub struct AttestedTip`:

```rust
    /// Whether the signed frontier is on our active chain.
    #[serde(default)]
    pub on_active_chain: Option<bool>,
}
```

with:

```rust
    /// Whether the signed frontier is on our active chain.
    #[serde(default)]
    pub on_active_chain: Option<bool>,
    /// The hash the node recorded for the frontier's height
    /// (`signed_frontier.hash`), when it knows one. The catch-up help
    /// (`crate::catchup_assist`) follows this header.
    #[serde(default)]
    pub hash: Option<String>,
}
```

and in `get_attested_tip`, replace:

```rust
            .or_else(|| v.get("on_active_chain").and_then(|x| x.as_bool())),
    })
}
```

with:

```rust
            .or_else(|| v.get("on_active_chain").and_then(|x| x.as_bool())),
        hash: sf.get("hash").and_then(|x| x.as_str()).map(str::to_string),
    })
}
```

Then run `grep -rn "AttestedTip {" crates apps/node/src-tauri/src | grep -v "pub struct"`. At `62a03f1` it prints only the literal in `get_attested_tip`. Any other literal without `..Default::default()` gets `hash: None,`.

In `crates/btx-core/src/known_invalid.rs`, replace:

```rust
pub const LIFTED_BRANCHES: &[&str] = &[];
```

with:

```rust
pub const LIFTED_BRANCHES: &[&str] = &[];

/// Every block this app refuses, the known-invalid ones and the held roots, as
/// (height, hash). The catch-up help (`crate::catchup_assist`) never asks a
/// peer for a chain that passes one of them.
pub fn refused_blocks() -> impl Iterator<Item = (u64, &'static str)> {
    KNOWN_INVALID_BLOCKS
        .iter()
        .map(|b| (b.height, b.hash))
        .chain(HELD_BRANCHES.iter().map(|b| (b.height, b.root)))
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib attested_tip_reads && cargo test --locked --lib refused_blocks_names`
Expected: each prints `test result: ok. 1 passed`.

- [ ] **Step 5: Format and commit**

```bash
cd crates/btx-core && cargo fmt --all && cd ../..
git add crates/btx-core/src/node_api.rs crates/btx-core/src/known_invalid.rs
git commit -F - <<'EOF'
core: the signed frontier's hash, and every refused block in one list

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 2: The header walk, kept between calls

**Files:**
- Create: `crates/btx-core/src/header_path.rs`
- Modify: `crates/btx-core/src/lib.rs:33` (after `pub mod fsx;`)

**Interfaces:**
- Produces: `header_path::HeaderPath` with `new()`, `target() -> Option<(u64, &str)>`, `hash_at(u64) -> Option<&str>`, `blocks() -> impl Iterator<Item = (u64, &str)>`, `retarget(&mut self, height: u64, hash: &str)`, `async walk(&mut self, rpc: &dyn Rpc, tip: u64, budget: usize) -> AppResult<usize>`, `status(&self, tip: u64, tip_hash: &str) -> PathStatus`, `next(&self, tip: u64, n: usize) -> Vec<(u64, String)>`.
- Produces: `header_path::PathStatus { Nothing, Walking, Ready, OtherBranch }`.

How it works: `hashes` maps height to hash on the followed chain. `down` is the lowest known block whose header is not read yet; reading it yields its parent one height lower, until the tip's height is reached. A new target that is not on the walked chain starts `up`, a second cursor that reads the new branch downward until it meets a hash already walked, so a higher target costs only the new headers and a new branch costs only the headers above the fork. The tip moving up costs nothing; a tip that falls below everything walked (a rollback) starts the walk again.

- [ ] **Step 1: Write the failing tests**

Create `crates/btx-core/src/header_path.rs` with only the module doc and the tests for now:

```rust
//! The headers between a node's tip and a header it follows, walked down with
//! `getblockheader` and kept between calls.
//!
//! A node can name the blocks above its tip only by walking back from a header
//! it knows: `getblockhash` answers for the active chain alone. The mirror
//! started from the signed snapshot on 2026-09-29 sat 7,490 headers below the
//! tip, so the walk is kept: a higher target on the same chain costs only the
//! headers above the old one, a tip that moves costs nothing, and a long walk
//! can be spread over several calls with a budget.
//!
//! Shared by the catch-up help (`crate::catchup_assist`, every refresher tick)
//! and the Tools button "Fetch a stuck block" (one walk per click).
#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A node that knows a set of headers and counts `getblockheader` calls.
    #[derive(Default)]
    struct Headers {
        by_hash: HashMap<String, (u64, String)>,
        reads: Mutex<usize>,
        fail_on: Mutex<Option<String>>,
    }

    fn main_hash(h: u64) -> String {
        format!("a{h:063x}")
    }
    fn side_hash(h: u64) -> String {
        format!("b{h:063x}")
    }

    impl Headers {
        /// The main chain from 0 to `top`, and a side branch from `fork + 1`
        /// to `side_top` when `side` is given.
        fn new(top: u64, side: Option<(u64, u64)>) -> Self {
            let mut by_hash = HashMap::new();
            for h in 1..=top {
                by_hash.insert(main_hash(h), (h, main_hash(h - 1)));
            }
            if let Some((fork, side_top)) = side {
                for h in fork + 1..=side_top {
                    let prev = if h == fork + 1 {
                        main_hash(fork)
                    } else {
                        side_hash(h - 1)
                    };
                    by_hash.insert(side_hash(h), (h, prev));
                }
            }
            Self {
                by_hash,
                ..Default::default()
            }
        }
        fn reads(&self) -> usize {
            *self.reads.lock().unwrap()
        }
    }

    #[async_trait]
    impl Rpc for Headers {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            assert_eq!(method, "getblockheader");
            let hash = params[0].as_str().unwrap().to_string();
            if self.fail_on.lock().unwrap().as_deref() == Some(hash.as_str()) {
                return Err(AppError::Http("connection reset".into()));
            }
            *self.reads.lock().unwrap() += 1;
            let (height, prev) = self.by_hash.get(&hash).cloned().ok_or(AppError::Rpc {
                code: -5,
                message: "Block not found".into(),
            })?;
            Ok(json!({"hash": hash, "height": height, "previousblockhash": prev}))
        }
    }

    #[tokio::test]
    async fn walks_from_the_target_down_to_the_tip() {
        let node = Headers::new(500, None);
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Walking);
        assert_eq!(p.walk(&node, 100, usize::MAX).await.unwrap(), 400);
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Ready);
        let next = p.next(100, 100);
        assert_eq!(next.len(), 100);
        assert_eq!(next[0], (101, main_hash(101)));
        assert_eq!(next[99], (200, main_hash(200)));
    }

    #[tokio::test]
    async fn a_budget_spreads_the_walk_over_calls() {
        let node = Headers::new(500, None);
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        assert_eq!(p.walk(&node, 100, 150).await.unwrap(), 150);
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Walking);
        assert_eq!(p.walk(&node, 100, 150).await.unwrap(), 150);
        assert_eq!(p.walk(&node, 100, 150).await.unwrap(), 100);
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Ready);
        assert_eq!(node.reads(), 400);
    }

    #[tokio::test]
    async fn a_higher_target_on_the_same_chain_reads_only_the_new_headers() {
        let node = Headers::new(510, None);
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        p.walk(&node, 100, usize::MAX).await.unwrap();
        p.retarget(510, &main_hash(510));
        assert_eq!(p.walk(&node, 100, usize::MAX).await.unwrap(), 10);
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Ready);
        assert_eq!(p.next(505, 10).last().unwrap(), &(510, main_hash(510)));
    }

    #[tokio::test]
    async fn a_higher_target_found_during_a_long_walk_keeps_the_walk() {
        let node = Headers::new(510, None);
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        p.walk(&node, 100, 50).await.unwrap(); // 500 down to 451
        p.retarget(510, &main_hash(510));
        p.walk(&node, 100, usize::MAX).await.unwrap();
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Ready);
        assert_eq!(node.reads(), 410, "every header read once");
    }

    #[tokio::test]
    async fn a_moving_tip_costs_nothing() {
        let node = Headers::new(500, None);
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        p.walk(&node, 100, usize::MAX).await.unwrap();
        assert_eq!(p.walk(&node, 300, usize::MAX).await.unwrap(), 0);
        assert_eq!(p.status(300, &main_hash(300)), PathStatus::Ready);
        assert_eq!(p.next(300, 1), vec![(301, main_hash(301))]);
        assert_eq!(p.status(500, &main_hash(500)), PathStatus::Nothing);
    }

    #[tokio::test]
    async fn a_target_on_another_branch_is_named_as_such() {
        let node = Headers::new(500, Some((300, 520)));
        let mut p = HeaderPath::new();
        p.retarget(520, &side_hash(520));
        p.walk(&node, 310, usize::MAX).await.unwrap();
        assert_eq!(p.status(310, &main_hash(310)), PathStatus::OtherBranch);
    }

    #[tokio::test]
    async fn a_new_branch_is_read_until_it_meets_the_old_one() {
        let node = Headers::new(500, Some((300, 520)));
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        p.walk(&node, 100, usize::MAX).await.unwrap();
        p.retarget(520, &side_hash(520));
        assert_eq!(p.walk(&node, 100, usize::MAX).await.unwrap(), 220);
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Ready);
        assert_eq!(p.hash_at(301), Some(side_hash(301).as_str()));
        assert_eq!(p.hash_at(300), Some(main_hash(300).as_str()));
    }

    #[tokio::test]
    async fn a_rollback_below_the_walk_starts_it_again() {
        let node = Headers::new(500, None);
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        p.walk(&node, 300, usize::MAX).await.unwrap();
        p.walk(&node, 50, usize::MAX).await.unwrap();
        assert_eq!(p.status(50, &main_hash(50)), PathStatus::Ready);
        assert_eq!(p.next(50, 1), vec![(51, main_hash(51))]);
    }

    #[tokio::test]
    async fn an_error_leaves_the_walk_where_it_was() {
        let node = Headers::new(500, None);
        *node.fail_on.lock().unwrap() = Some(main_hash(400));
        let mut p = HeaderPath::new();
        p.retarget(500, &main_hash(500));
        assert!(p.walk(&node, 100, usize::MAX).await.is_err());
        *node.fail_on.lock().unwrap() = None;
        p.walk(&node, 100, usize::MAX).await.unwrap();
        assert_eq!(p.status(100, &main_hash(100)), PathStatus::Ready);
        assert_eq!(node.reads(), 400);
    }
}
```

In `crates/btx-core/src/lib.rs`, replace `pub mod fsx;` with:

```rust
pub mod fsx;
pub mod header_path;
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib header_path 2>&1 | grep -E "^error" | head -3`
Expected: the crate does not compile, with errors naming `HeaderPath`, `PathStatus` and `AppResult` as not found.

- [ ] **Step 3: Implement**

Insert this between the module doc and `#[cfg(test)]` in `crates/btx-core/src/header_path.rs`:

```rust
use std::collections::BTreeMap;

use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::rpc::Rpc;

/// Where a [`HeaderPath`] stands against the node's tip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathStatus {
    /// No target, or the target is not above the tip.
    Nothing,
    /// The walk has not reached the tip yet; call [`HeaderPath::walk`] again.
    Walking,
    /// The walked chain sits on the node's tip.
    Ready,
    /// The walked chain has another block at the tip's height: the target is
    /// on another branch, which the node decides on its own.
    OtherBranch,
}

#[derive(Debug, Clone, Default)]
pub struct HeaderPath {
    /// Height to hash on the followed chain.
    hashes: BTreeMap<u64, String>,
    /// The header the walk follows.
    top: Option<(u64, String)>,
    /// The lowest known block of the walk whose header is not read yet.
    down: Option<(u64, String)>,
    /// A new branch being read down until it meets the headers read before.
    up: Option<(u64, String)>,
}

impl HeaderPath {
    pub fn new() -> Self {
        Self::default()
    }

    /// The header being followed, if any.
    pub fn target(&self) -> Option<(u64, &str)> {
        self.top.as_ref().map(|(h, s)| (*h, s.as_str()))
    }

    /// The hash this path holds at `height`, if it has walked there.
    pub fn hash_at(&self, height: u64) -> Option<&str> {
        self.hashes.get(&height).map(String::as_str)
    }

    /// Every (height, hash) the path holds, lowest first.
    pub fn blocks(&self) -> impl Iterator<Item = (u64, &str)> {
        self.hashes.iter().map(|(h, s)| (*h, s.as_str()))
    }

    /// Follow `hash` at `height` from now on. Headers already read on the same
    /// chain are kept.
    pub fn retarget(&mut self, height: u64, hash: &str) {
        if self.target() == Some((height, hash)) {
            return;
        }
        let known = self.hash_at(height) == Some(hash);
        self.hashes.retain(|h, _| *h <= height);
        self.top = Some((height, hash.to_string()));
        if known {
            self.up = None;
        } else if self.hashes.is_empty() {
            self.hashes.insert(height, hash.to_string());
            self.down = Some((height, hash.to_string()));
            self.up = None;
        } else {
            self.up = Some((height, hash.to_string()));
        }
    }

    /// Read at most `budget` headers toward `tip`. Returns how many were read.
    /// On an error the walk stays where it was, so the next call resumes.
    pub async fn walk(&mut self, rpc: &dyn Rpc, tip: u64, budget: usize) -> AppResult<usize> {
        self.forget_below(tip);
        let mut read = 0;
        while read < budget {
            if let Some((height, hash)) = self.up.take() {
                if self.hash_at(height) == Some(hash.as_str()) {
                    continue; // met the chain read before
                }
                if self.hashes.range(..height).next().is_none() {
                    // Below everything read before: this branch is the walk now.
                    self.hashes.insert(height, hash.clone());
                    self.down = Some((height, hash));
                    continue;
                }
                let prev = match parent(rpc, &hash, height).await {
                    Ok(p) => p,
                    Err(e) => {
                        self.up = Some((height, hash));
                        return Err(e);
                    }
                };
                read += 1;
                self.hashes.insert(height, hash);
                self.up = Some((height - 1, prev));
                continue;
            }
            let Some((height, hash)) = self.down.take() else {
                break;
            };
            if height <= tip {
                break; // reached the tip; the walk is done
            }
            let prev = match parent(rpc, &hash, height).await {
                Ok(p) => p,
                Err(e) => {
                    self.down = Some((height, hash));
                    return Err(e);
                }
            };
            read += 1;
            self.hashes.insert(height - 1, prev.clone());
            self.down = Some((height - 1, prev));
        }
        Ok(read)
    }

    /// Where the path stands against the node's tip.
    pub fn status(&self, tip: u64, tip_hash: &str) -> PathStatus {
        match &self.top {
            None => return PathStatus::Nothing,
            Some((h, _)) if *h <= tip => return PathStatus::Nothing,
            Some(_) => {}
        }
        if self.up.is_some() || self.down.as_ref().is_some_and(|(h, _)| *h > tip) {
            return PathStatus::Walking;
        }
        match self.hash_at(tip) {
            Some(h) if h == tip_hash => PathStatus::Ready,
            Some(_) => PathStatus::OtherBranch,
            None => PathStatus::Walking,
        }
    }

    /// Up to `n` blocks above `tip`, lowest first. Meaningful once
    /// [`status`](Self::status) is [`PathStatus::Ready`].
    pub fn next(&self, tip: u64, n: usize) -> Vec<(u64, String)> {
        self.hashes
            .range(tip.saturating_add(1)..)
            .take(n)
            .map(|(h, s)| (*h, s.clone()))
            .collect()
    }

    /// Drop what lies below the tip. A tip that fell below everything read (a
    /// rollback) starts the walk again from the target.
    fn forget_below(&mut self, tip: u64) {
        let Some(&low) = self.hashes.keys().next() else {
            return;
        };
        if tip < low && self.up.is_none() && self.down.is_none() {
            let top = self.top.take();
            *self = Self::default();
            if let Some((h, s)) = top {
                self.retarget(h, &s);
            }
            return;
        }
        self.hashes.retain(|h, _| *h >= tip);
        if self.down.as_ref().is_some_and(|(h, _)| *h < tip) {
            self.down = None;
        }
    }
}

/// The parent of the header `hash`, checking it sits at `height`.
async fn parent(rpc: &dyn Rpc, hash: &str, height: u64) -> AppResult<String> {
    let h = rpc.call("getblockheader", json!([hash, true])).await?;
    if h["height"].as_u64() != Some(height) {
        return Err(AppError::Decode(format!(
            "header {hash} is not at height {height}"
        )));
    }
    h["previousblockhash"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| AppError::Decode(format!("header {hash} has no parent")))
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib header_path`
Expected: `test result: ok. 9 passed; 0 failed`.

- [ ] **Step 5: Format, lint and commit**

```bash
cd crates/btx-core && cargo fmt --all && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cd ../..
git add crates/btx-core/src/header_path.rs crates/btx-core/src/lib.rs
git commit -F - <<'EOF'
core: walk the headers above the tip once and keep them

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

Expected from clippy: `Finished ...` with no `error`.

---

### Task 3: "Fetch a stuck block" uses the shared walk

**Files:**
- Modify: `apps/node/src-tauri/src/tools.rs:15` (imports) and `:289-326` (`missing_blocks`)

**Interfaces:**
- Consumes: `HeaderPath`, `PathStatus` from Task 2.
- Produces: `missing_blocks` keeps its signature and its three sentences; `tools_fetch_stuck_blocks` is unchanged.

No new test: the walk's behaviour is covered by Task 2's tests, and the button's sentences are unchanged. The shell's suite must stay green.

- [ ] **Step 1: Run the shell's suite first, as the baseline**

Run: `cd apps/node/src-tauri && cargo test --locked 2>&1 | grep "test result" | head -1`
Expected: `test result: ok. 96 passed; 0 failed; 1 ignored` (the count at `62a03f1`; note yours).

- [ ] **Step 2: Replace the hand-written walk**

In `apps/node/src-tauri/src/tools.rs`, after `use btx_core::error::AppError;` add:

```rust
use btx_core::header_path::{HeaderPath, PathStatus};
```

Replace the whole of `missing_blocks` (from its doc comment `/// Walk back from `target` to the block above `tip_height`, and check it` to the closing brace before `#[tauri::command]` / `pub async fn tools_fetch_stuck_blocks`) with:

```rust
/// Walk back from `target` to the block above `tip_height`, and check it
/// sits on the node's own tip. The walk is the one the catch-up help keeps
/// between ticks (`btx_core::header_path`); here it runs once per click.
async fn missing_blocks(
    rpc: &RpcClient,
    target: &btx_core::fork::ChainTip,
    tip_height: u64,
    tip_hash: &str,
) -> Result<Vec<(u64, String)>, String> {
    if target.height.saturating_sub(tip_height) > stuck_blocks::MAX_WALK {
        return Err(
            "Your node is far behind. That is catching up, not a stuck block; leave it running."
                .into(),
        );
    }
    let mut path = HeaderPath::new();
    path.retarget(target.height, &target.hash);
    path.walk(rpc, tip_height, usize::MAX)
        .await
        .map_err(fetch_error)?;
    match path.status(tip_height, tip_hash) {
        PathStatus::Ready => Ok(path.next(tip_height, usize::MAX)),
        PathStatus::OtherBranch => Err("The newest headers are on another branch than your node's tip. The node decides that on its own.".into()),
        PathStatus::Nothing | PathStatus::Walking => Ok(Vec::new()),
    }
}
```

`json!` stays imported: the file uses it elsewhere.

- [ ] **Step 3: Run the checks**

Run: `cd apps/node/src-tauri && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cargo test --locked 2>&1 | grep "test result" | head -1`
Expected: no fmt output, `Finished ...`, and the same count as Step 1.

- [ ] **Step 4: Commit**

```bash
git add apps/node/src-tauri/src/tools.rs
git commit -F - <<'EOF'
node: Fetch a stuck block walks the headers with the shared walk

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 4: The decision

**Files:**
- Create: `crates/btx-core/src/catchup_assist.rs`
- Modify: `crates/btx-core/src/lib.rs:20` (after `pub mod backend;`)

**Interfaces:**
- Consumes: `stuck_blocks::target_tip` (existing), `known_invalid::refused_blocks` (Task 1), `HeaderPath::hash_at` (Task 2), `node_api::PeerInfo` (`id`, `addr`, `connection_type`, `synced_headers`, `inflight`).
- Produces: constants `ENABLED`, `MIN_BEHIND`, `QUIET`, `BATCH`, `ROTATE_AFTER`, `WALK_PER_TICK`; `enum Why { Off, NotBehind, EngineFetching, Quiet, NoPath, NoPeer, Refused, BatchOut }`; `enum Decision { Wait(Why), Ask { peer_id: i64, addr: String, blocks: Vec<(u64, String)> } }`; `struct Seen<'a> { now, tip, target: Option<u64>, next: &'a [(u64, String)], peers: &'a [PeerInfo], refused: Option<&'static str> }`; `Helper::new(Vec<String>)`, `Helper::helping()`, `Helper::decide(&mut self, &Seen) -> Decision`, `Helper::sent(&mut self, &Decision, Instant, accepted: usize, already_have: usize)`; `choose_target(Option<(u64, String)>, &[ChainTip], u64) -> Option<(u64, String)>`; `refused_on_path(&HeaderPath) -> Option<&'static str>`; `note(bool, &Decision) -> Option<String>`.

The rules, in the order `decide` applies them:
1. Switched off: nothing.
2. The followed chain is fewer than 20 above the tip: nothing, and forget any batch and quiet clock.
3. A refused block on the way: nothing, likewise.
4. Some peer has one of the next 100 heights in flight that is not in this help's batch: the engine is fetching; nothing, likewise.
5. A batch is out: if the tip reached its top, ask for the next batch at once from the same peer; if its peer is gone or three minutes passed, ask the next archive peer; else wait.
6. No batch: the quiet clock runs from the moment the current tip was first seen with nothing in flight; after 30 seconds, ask.

Asking picks, among manual peers whose address is in the app's list and which announced a header above the tip: the peer whose last batch connected, then list order, starting after the peer to rotate away from. It asks for up to 100 blocks from `tip + 1`, never above the height the peer announced. A peer that takes none of them is rotated away from, after another 30-second wait.

- [ ] **Step 1: Write the failing tests**

Create `crates/btx-core/src/catchup_assist.rs` with only the module doc and the tests for now:

```rust
//! Catch-up help: while the node is behind and the engine asks nobody for the
//! next blocks, ask the app's own archive peers for them by name.
//!
//! Decision: docs/decisions/2026-09-29-every-node-starts-near-the-tip.md,
//! section 11. Measured on mainnet on 2026-09-29 with engine v0.34.9: a mirror
//! started from the signed snapshot 7,500 blocks behind got one block about
//! every 2.3 minutes, because the engine asks a NETWORK_LIMITED peer for an
//! older block only through its 120-second "served-body-tip wedge" rescue.
//! Asking 109.199.124.187 for the next blocks with `getblockfrompeer`, 100 at
//! a time and waiting for each batch to connect, moved it 7,490 blocks in 8
//! minutes, after which it kept up on its own.
//!
//! [`Helper`] decides from what one refresher tick saw, and is pure. [`tick`]
//! reads what the decision needs from the node, sends the requests and reports
//! back. It never adds, bans or disconnects a peer: `getblockfrompeer` is the
//! only command it sends that changes anything.

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "109.199.124.187:19335";
    const B: &str = "20.86.181.203:19338";
    const C: &str = "37.230.134.222:19335";

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }
    fn hash(h: u64) -> String {
        format!("{h:064x}")
    }
    fn next_from(tip: u64) -> Vec<(u64, String)> {
        (tip + 1..=tip + BATCH).map(|h| (h, hash(h))).collect()
    }
    fn peer(id: i64, addr: &str, synced_headers: i64) -> PeerInfo {
        PeerInfo {
            id,
            addr: addr.into(),
            connection_type: "manual".into(),
            synced_headers,
            ..Default::default()
        }
    }
    fn with_inflight(mut p: PeerInfo, heights: &[i64]) -> PeerInfo {
        p.inflight = heights.to_vec();
        p
    }
    fn helper() -> Helper {
        Helper::new(vec![A.into(), B.into(), C.into()])
    }
    fn seen<'a>(
        now: Instant,
        tip: u64,
        next: &'a [(u64, String)],
        peers: &'a [PeerInfo],
    ) -> Seen<'a> {
        Seen {
            now,
            tip,
            target: Some(tip + 7_000),
            next,
            peers,
            refused: None,
        }
    }
    /// Decide, and when it asks, report every request taken.
    fn step(h: &mut Helper, s: &Seen) -> Decision {
        let d = h.decide(s);
        if let Decision::Ask { blocks, .. } = &d {
            h.sent(&d, s.now, blocks.len(), 0);
        }
        d
    }
    fn asked_of(d: &Decision) -> (i64, u64, u64) {
        match d {
            Decision::Ask {
                peer_id, blocks, ..
            } => (*peer_id, blocks[0].0, blocks.last().unwrap().0),
            other => panic!("expected Ask, got {other:?}"),
        }
    }

    #[test]
    fn waits_thirty_quiet_seconds_then_asks_for_the_next_hundred() {
        let t0 = Instant::now();
        let (next, peers) = (next_from(1_000), [peer(7, A, 8_000)]);
        let mut h = helper();
        assert_eq!(
            h.decide(&seen(t0, 1_000, &next, &peers)),
            Decision::Wait(Why::Quiet)
        );
        assert_eq!(
            h.decide(&seen(t0 + secs(29), 1_000, &next, &peers)),
            Decision::Wait(Why::Quiet)
        );
        let d = h.decide(&seen(t0 + QUIET, 1_000, &next, &peers));
        assert_eq!(asked_of(&d), (7, 1_001, 1_100));
    }

    #[test]
    fn does_nothing_within_twenty_blocks() {
        let t0 = Instant::now();
        let (next, peers) = (next_from(1_000), [peer(7, A, 8_000)]);
        let mut h = helper();
        let mut s = seen(t0, 1_000, &next, &peers);
        s.target = Some(1_019);
        assert_eq!(h.decide(&s), Decision::Wait(Why::NotBehind));
        s.target = Some(1_020);
        assert_eq!(h.decide(&s), Decision::Wait(Why::Quiet));
        s.target = None;
        assert_eq!(h.decide(&s), Decision::Wait(Why::NotBehind));
    }

    #[test]
    fn the_next_block_in_flight_restarts_the_quiet_wait() {
        let t0 = Instant::now();
        let next = next_from(1_000);
        let idle = [peer(7, A, 8_000)];
        let busy = [with_inflight(peer(7, A, 8_000), &[1_001])];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &idle));
        assert_eq!(
            h.decide(&seen(t0 + secs(20), 1_000, &next, &busy)),
            Decision::Wait(Why::EngineFetching)
        );
        assert_eq!(
            h.decide(&seen(t0 + secs(45), 1_000, &next, &idle)),
            Decision::Wait(Why::Quiet)
        );
        assert!(matches!(
            h.decide(&seen(t0 + secs(75), 1_000, &next, &idle)),
            Decision::Ask { .. }
        ));
    }

    #[test]
    fn a_new_tip_restarts_the_quiet_wait() {
        let t0 = Instant::now();
        let peers = [peer(7, A, 8_000)];
        let (n1, n2) = (next_from(1_000), next_from(1_001));
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &n1, &peers));
        assert_eq!(
            h.decide(&seen(t0 + secs(25), 1_001, &n2, &peers)),
            Decision::Wait(Why::Quiet)
        );
        assert_eq!(
            h.decide(&seen(t0 + secs(35), 1_001, &n2, &peers)),
            Decision::Wait(Why::Quiet)
        );
        assert!(matches!(
            h.decide(&seen(t0 + secs(55), 1_001, &n2, &peers)),
            Decision::Ask { .. }
        ));
    }

    #[test]
    fn waits_for_a_batch_to_connect_then_asks_for_the_next_at_once() {
        let t0 = Instant::now();
        let peers = [peer(7, A, 8_000)];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next_from(1_000), &peers));
        step(&mut h, &seen(t0 + QUIET, 1_000, &next_from(1_000), &peers));
        assert!(h.helping());
        let mid = next_from(1_050);
        assert_eq!(
            h.decide(&seen(t0 + secs(36), 1_050, &mid, &peers)),
            Decision::Wait(Why::BatchOut)
        );
        let done = next_from(1_100);
        let d = h.decide(&seen(t0 + secs(37), 1_100, &done, &peers));
        assert_eq!(asked_of(&d), (7, 1_101, 1_200));
    }

    #[test]
    fn rotates_to_the_next_archive_peer_after_three_minutes() {
        let t0 = Instant::now();
        let next = next_from(1_000);
        let peers = [peer(9, B, 8_000), peer(7, A, 8_000)];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &peers));
        let d = step(&mut h, &seen(t0 + QUIET, 1_000, &next, &peers));
        assert_eq!(asked_of(&d).0, 7, "list order: 109.199.124.187 first");
        let t1 = t0 + QUIET;
        assert_eq!(
            h.decide(&seen(t1 + secs(179), 1_000, &next, &peers)),
            Decision::Wait(Why::BatchOut)
        );
        let d = step(&mut h, &seen(t1 + ROTATE_AFTER, 1_000, &next, &peers));
        assert_eq!(asked_of(&d).0, 9);
        let d = step(&mut h, &seen(t1 + ROTATE_AFTER * 2, 1_000, &next, &peers));
        assert_eq!(asked_of(&d).0, 7, "wraps around");
    }

    #[test]
    fn a_peer_that_disconnects_is_replaced_at_once() {
        let t0 = Instant::now();
        let next = next_from(1_000);
        let both = [peer(7, A, 8_000), peer(9, B, 8_000)];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &both));
        step(&mut h, &seen(t0 + QUIET, 1_000, &next, &both));
        let left = [peer(9, B, 8_000)];
        let d = h.decide(&seen(t0 + secs(33), 1_000, &next, &left));
        assert_eq!(asked_of(&d).0, 9);
    }

    #[test]
    fn stops_when_the_engine_asks_for_the_next_blocks_itself() {
        let t0 = Instant::now();
        let mut h = helper();
        let quiet = [peer(7, A, 8_000)];
        h.decide(&seen(t0, 1_000, &next_from(1_000), &quiet));
        step(&mut h, &seen(t0 + QUIET, 1_000, &next_from(1_000), &quiet));
        // Our own requests show up in flight and do not count.
        let ours = [with_inflight(peer(7, A, 8_000), &[1_051, 1_052])];
        assert_eq!(
            h.decide(&seen(t0 + secs(35), 1_050, &next_from(1_050), &ours)),
            Decision::Wait(Why::BatchOut)
        );
        let engine = [
            with_inflight(peer(7, A, 8_000), &[1_051]),
            with_inflight(peer(12, "1.2.3.4:19335", 8_000), &[1_120]),
        ];
        assert_eq!(
            h.decide(&seen(t0 + secs(38), 1_050, &next_from(1_050), &engine)),
            Decision::Wait(Why::EngineFetching)
        );
        assert!(!h.helping());
    }

    #[test]
    fn asks_only_the_app_s_own_archive_peers() {
        let t0 = Instant::now();
        let next = next_from(1_000);
        let mut inbound = peer(1, "5.6.7.8:40000", 8_000);
        inbound.connection_type = "inbound".into();
        inbound.inbound = true;
        let mut outbound = peer(2, C, 8_000);
        outbound.connection_type = "outbound-full-relay".into();
        let strangers = [
            inbound,
            outbound,
            peer(3, "9.9.9.9:19335", 8_000), // someone's own addnode
            peer(4, A, 1_000),               // ours, without the next block
        ];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &strangers));
        assert_eq!(
            h.decide(&seen(t0 + QUIET, 1_000, &next, &strangers)),
            Decision::Wait(Why::NoPeer)
        );
    }

    #[test]
    fn never_asks_past_what_the_peer_announced() {
        let t0 = Instant::now();
        let (next, peers) = (next_from(1_000), [peer(7, A, 1_050)]);
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &peers));
        let d = h.decide(&seen(t0 + QUIET, 1_000, &next, &peers));
        assert_eq!(asked_of(&d), (7, 1_001, 1_050));
    }

    #[test]
    fn a_peer_that_takes_nothing_is_skipped_after_another_quiet_wait() {
        let t0 = Instant::now();
        let next = next_from(1_000);
        let peers = [peer(7, A, 8_000), peer(9, B, 8_000)];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &peers));
        let d = h.decide(&seen(t0 + QUIET, 1_000, &next, &peers));
        h.sent(&d, t0 + QUIET, 0, 0);
        assert!(!h.helping());
        assert_eq!(
            h.decide(&seen(t0 + secs(33), 1_000, &next, &peers)),
            Decision::Wait(Why::Quiet)
        );
        let d = h.decide(&seen(t0 + QUIET * 2, 1_000, &next, &peers));
        assert_eq!(asked_of(&d).0, 9);
    }

    #[test]
    fn blocks_already_downloaded_count_as_taken() {
        let t0 = Instant::now();
        let (next, peers) = (next_from(1_000), [peer(7, A, 8_000)]);
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &next, &peers));
        let d = h.decide(&seen(t0 + QUIET, 1_000, &next, &peers));
        h.sent(&d, t0 + QUIET, 0, 100);
        assert_eq!(
            h.decide(&seen(t0 + secs(40), 1_000, &next, &peers)),
            Decision::Wait(Why::BatchOut)
        );
    }

    #[test]
    fn a_refused_block_or_the_switch_stops_it() {
        let t0 = Instant::now();
        let (next, peers) = (next_from(1_000), [peer(7, A, 8_000)]);
        let mut h = helper();
        let mut s = seen(t0, 1_000, &next, &peers);
        s.refused = Some("8240c62e");
        assert_eq!(h.decide(&s), Decision::Wait(Why::Refused));
        let mut off = Helper::with_switch(vec![A.into()], false);
        assert_eq!(
            off.decide(&seen(t0, 1_000, &next, &peers)),
            Decision::Wait(Why::Off)
        );
    }

    #[test]
    fn waits_for_the_headers_to_be_read() {
        let t0 = Instant::now();
        let peers = [peer(7, A, 8_000)];
        let mut h = helper();
        h.decide(&seen(t0, 1_000, &[], &peers));
        assert_eq!(
            h.decide(&seen(t0 + QUIET, 1_000, &[], &peers)),
            Decision::Wait(Why::NoPath)
        );
    }

    fn tip(height: u64, status: &str) -> ChainTip {
        ChainTip {
            height,
            hash: hash(height),
            branchlen: 1,
            status: status.into(),
        }
    }

    #[test]
    fn follows_the_signed_frontier_when_there_is_one_else_the_best_header() {
        let tips = [tip(8_000, "headers-only"), tip(9_000, "invalid")];
        assert_eq!(
            choose_target(Some((7_500, hash(7_500))), &tips, 1_000),
            Some((7_500, hash(7_500)))
        );
        assert_eq!(
            choose_target(Some((1_000, hash(1_000))), &tips, 1_000),
            None,
            "at the signed frontier there is nothing to fetch"
        );
        assert_eq!(
            choose_target(None, &tips, 1_000),
            Some((8_000, hash(8_000)))
        );
    }

    #[test]
    fn the_log_names_a_stop_once() {
        assert_eq!(note(false, &Decision::Wait(Why::NotBehind)), None);
        assert!(note(true, &Decision::Wait(Why::NotBehind))
            .unwrap()
            .contains("within 20 blocks"));
        assert!(note(true, &Decision::Wait(Why::EngineFetching))
            .unwrap()
            .contains("on its own"));
        assert_eq!(note(true, &Decision::Wait(Why::BatchOut)), None);
        for why in [
            Why::NotBehind,
            Why::EngineFetching,
            Why::Refused,
            Why::NoPeer,
            Why::NoPath,
            Why::Off,
        ] {
            assert!(!note(true, &Decision::Wait(why))
                .unwrap()
                .contains('\u{2014}'));
        }
    }
}
```

In `crates/btx-core/src/lib.rs`, replace `pub mod backend;` with:

```rust
pub mod backend;
pub mod catchup_assist;
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib catchup_assist 2>&1 | grep -E "^error" | head -3`
Expected: errors such as `error[E0412]: cannot find type `Helper` in this scope` and `cannot find value `QUIET``.

- [ ] **Step 3: Implement**

Insert this between the module doc and `#[cfg(test)]` in `crates/btx-core/src/catchup_assist.rs`:

```rust
use std::time::{Duration, Instant};

use crate::fork::ChainTip;
use crate::header_path::HeaderPath;
use crate::node_api::PeerInfo;


/// The one switch. Off, the help sends nothing at all.
pub const ENABLED: bool = true;
/// Help only when the followed chain is at least this many blocks above the tip.
pub const MIN_BEHIND: u64 = 20;
/// ...and nobody has been asked for the next block for this long.
pub const QUIET: Duration = Duration::from_secs(30);
/// Blocks asked for at once, from one peer.
pub const BATCH: u64 = 100;
/// A batch that has not connected by then goes to the next archive peer.
pub const ROTATE_AFTER: Duration = Duration::from_secs(180);
/// Headers read per tick while walking down to the tip (`crate::header_path`).
pub const WALK_PER_TICK: usize = 1_000;

/// Why a tick asked for nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// [`ENABLED`] is off.
    Off,
    /// The followed chain is fewer than [`MIN_BEHIND`] blocks above the tip.
    NotBehind,
    /// Some peer is being asked for one of the next blocks, and not by us.
    EngineFetching,
    /// Waiting for [`QUIET`] to pass.
    Quiet,
    /// The headers to follow are not read yet, or not on the node's tip.
    NoPath,
    /// None of the app's archive peers is connected and has the next block.
    NoPeer,
    /// The followed chain passes a block this app refuses.
    Refused,
    /// A batch is out and has not connected yet.
    BatchOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Wait(Why),
    /// Ask this peer for these blocks, lowest first.
    Ask {
        peer_id: i64,
        addr: String,
        blocks: Vec<(u64, String)>,
    },
}

/// What one tick saw.
#[derive(Debug, Clone)]
pub struct Seen<'a> {
    pub now: Instant,
    /// The node's tip height.
    pub tip: u64,
    /// The followed chain's height, `None` when there is nothing to follow.
    pub target: Option<u64>,
    /// The next blocks on the followed chain from `tip + 1`, lowest first.
    /// Empty while the headers are not read or not on the node's tip.
    pub next: &'a [(u64, String)],
    pub peers: &'a [PeerInfo],
    /// A refused block on the followed chain or on the node's own chain.
    pub refused: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Batch {
    peer_id: i64,
    addr: String,
    from: u64,
    to: u64,
    asked_at: Instant,
}

/// The decision state for one node run.
#[derive(Debug, Clone)]
pub struct Helper {
    enabled: bool,
    /// The addresses of the peers the app dials that can serve a block, in
    /// the order it prefers them.
    archive: Vec<String>,
    /// Since when the next block has gone unrequested, at which tip.
    quiet_since: Option<(u64, Instant)>,
    batch: Option<Batch>,
    /// The peer whose last batch connected: asked first.
    last_good: Option<String>,
    /// The peer to rotate away from on the next request.
    slow: Option<String>,
}

impl Helper {
    pub fn new(archive: Vec<String>) -> Self {
        Self::with_switch(archive, ENABLED)
    }

    fn with_switch(archive: Vec<String>, enabled: bool) -> Self {
        Self {
            enabled,
            archive,
            quiet_since: None,
            batch: None,
            last_good: None,
            slow: None,
        }
    }

    /// A batch is out.
    pub fn helping(&self) -> bool {
        self.batch.is_some()
    }

    pub fn decide(&mut self, s: &Seen) -> Decision {
        if !self.enabled {
            return self.idle(Why::Off);
        }
        if s.target.map_or(0, |t| t.saturating_sub(s.tip)) < MIN_BEHIND {
            return self.idle(Why::NotBehind);
        }
        if s.refused.is_some() {
            return self.idle(Why::Refused);
        }
        let ours = self.batch.as_ref().map(|b| (b.from, b.to));
        if engine_fetching(s.peers, s.tip, ours) {
            return self.idle(Why::EngineFetching);
        }
        if let Some(b) = self.batch.take() {
            let peer_here = s.peers.iter().any(|p| p.id == b.peer_id);
            if s.tip >= b.to {
                self.last_good = Some(b.addr);
                self.slow = None;
            } else if peer_here && s.now.duration_since(b.asked_at) < ROTATE_AFTER {
                self.batch = Some(b);
                return Decision::Wait(Why::BatchOut);
            } else {
                self.slow = Some(b.addr);
            }
            return self.ask(s);
        }
        let since = match self.quiet_since {
            Some((tip, at)) if tip == s.tip => at,
            _ => {
                self.quiet_since = Some((s.tip, s.now));
                s.now
            }
        };
        if s.now.duration_since(since) < QUIET {
            return Decision::Wait(Why::Quiet);
        }
        self.ask(s)
    }

    /// What came of a [`Decision::Ask`]: how many requests the node took, and
    /// how many named a block it had already downloaded.
    pub fn sent(&mut self, d: &Decision, now: Instant, accepted: usize, already_have: usize) {
        let Decision::Ask {
            peer_id,
            addr,
            blocks,
        } = d
        else {
            return;
        };
        let (Some(first), Some(last)) = (blocks.first(), blocks.last()) else {
            return;
        };
        if accepted + already_have == 0 {
            // The peer took none: the next one, after another quiet wait.
            self.slow = Some(addr.clone());
            self.quiet_since = Some((first.0.saturating_sub(1), now));
            return;
        }
        self.batch = Some(Batch {
            peer_id: *peer_id,
            addr: addr.clone(),
            from: first.0,
            to: last.0,
            asked_at: now,
        });
    }

    fn idle(&mut self, why: Why) -> Decision {
        self.batch = None;
        self.quiet_since = None;
        Decision::Wait(why)
    }

    fn ask(&mut self, s: &Seen) -> Decision {
        if s.next.first().map(|(h, _)| *h) != Some(s.tip + 1) {
            return Decision::Wait(Why::NoPath);
        }
        let Some(peer) = self.pick(s.peers, s.tip) else {
            return Decision::Wait(Why::NoPeer);
        };
        let blocks = s
            .next
            .iter()
            .take(BATCH as usize)
            .filter(|(h, _)| *h as i64 <= peer.synced_headers)
            .cloned()
            .collect();
        Decision::Ask {
            peer_id: peer.id,
            addr: peer.addr.clone(),
            blocks,
        }
    }

    /// The app's own archive peers that announced the next block: the one
    /// that delivered last first, then list order, starting after the slow one.
    fn pick<'p>(&self, peers: &'p [PeerInfo], tip: u64) -> Option<&'p PeerInfo> {
        let mut ranked: Vec<&PeerInfo> = peers
            .iter()
            .filter(|p| {
                p.connection_type == "manual"
                    && self.archive.contains(&p.addr)
                    && p.synced_headers > tip as i64
            })
            .collect();
        ranked.sort_by_key(|p| {
            (
                self.last_good.as_ref() != Some(&p.addr),
                self.archive.iter().position(|a| *a == p.addr),
            )
        });
        let start = self
            .slow
            .as_ref()
            .and_then(|slow| ranked.iter().position(|p| p.addr == *slow))
            .map_or(0, |i| (i + 1) % ranked.len());
        ranked.get(start).copied()
    }
}

/// Whether some peer is being asked for one of the next [`BATCH`] blocks by
/// the engine. `getblockfrompeer` marks a block in flight at the peer asked,
/// so the batch this help has out (`ours`) does not count.
fn engine_fetching(peers: &[PeerInfo], tip: u64, ours: Option<(u64, u64)>) -> bool {
    let window = (tip as i64 + 1)..=((tip + BATCH) as i64);
    peers.iter().flat_map(|p| p.inflight.iter()).any(|&h| {
        window.contains(&h)
            && !ours.is_some_and(|(from, to)| (from as i64..=to as i64).contains(&h))
    })
}

/// The chain to follow: toward the signed frontier when the node reads one,
/// else its best header that is not refused (as the Tools button does).
pub fn choose_target(
    frontier: Option<(u64, String)>,
    tips: &[ChainTip],
    tip: u64,
) -> Option<(u64, String)> {
    match frontier {
        Some((height, hash)) => (height > tip).then_some((height, hash)),
        None => crate::stuck_blocks::target_tip(tips, tip).map(|t| (t.height, t.hash.clone())),
    }
}

/// The first refused block on the walked headers, if any.
pub fn refused_on_path(path: &HeaderPath) -> Option<&'static str> {
    crate::known_invalid::refused_blocks()
        .find(|(height, hash)| path.hash_at(*height) == Some(*hash))
        .map(|(_, hash)| hash)
}

/// One line for the log when the help stops or pauses.
pub fn note(was_helping: bool, d: &Decision) -> Option<String> {
    let Decision::Wait(why) = d else {
        return None;
    };
    if !was_helping {
        return None;
    }
    let line = match why {
        Why::NotBehind => {
            format!("stopped: the node is within {MIN_BEHIND} blocks of the chain it follows")
        }
        Why::EngineFetching => {
            "stopped: the node is fetching the next blocks on its own again".into()
        }
        Why::Refused => "stopped: the chain it follows passes a block this app refuses".into(),
        Why::NoPeer => {
            "paused: none of this app's archive peers is connected with the next block".into()
        }
        Why::NoPath => "paused: the headers to follow are not on this node's tip".into(),
        Why::Off => "stopped: switched off".into(),
        Why::Quiet | Why::BatchOut => return None,
    };
    Some(line)
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib catchup_assist`
Expected: `test result: ok. 16 passed; 0 failed`.

- [ ] **Step 5: Format, lint and commit**

```bash
cd crates/btx-core && cargo fmt --all && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cd ../..
git add crates/btx-core/src/catchup_assist.rs crates/btx-core/src/lib.rs
git commit -F - <<'EOF'
core: when to ask an archive peer for the next blocks, and whom

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 5: One tick against the node

**Files:**
- Modify: `crates/btx-core/src/catchup_assist.rs` (imports, the driver after `note`, tests appended inside `mod tests`)

**Interfaces:**
- Consumes: everything from Task 4; `HeaderPath` and `PathStatus` (Task 2); `node_api::get_attested_tip` with `hash` (Task 1); `node::block_source_peers()` (existing, `crates/btx-core/src/node.rs:318`).
- Produces: `struct Tick<'a> { blocks: u64, headers: u64, tips: &'a [ChainTip], peers: &'a [PeerInfo] }`; `CatchUp::new(Vec<String>)`, `CatchUp::for_this_app()`; `async fn tick(rpc: &dyn Rpc, cu: &mut CatchUp, t: &Tick<'_>, now: Instant) -> Option<String>`.

What `tick` does: nothing at all on the node while `headers < blocks + 20` or `blocks == 0` (the decision still runs so a stop is logged). Otherwise it reads the tip's hash, the signed frontier (followed only when its header is known), chooses the target, walks at most 1,000 headers, checks the walked chain and the node's own chain for refused blocks, decides, and on `Ask` sends one `getblockfrompeer` per block, counting "Block already downloaded" as taken. It returns a line for the log when it asked, stopped or paused.

Why the archive list is `block_source_peers()` and not `BTX_ARCHIVE_PEERS`: the peer that served the measured catch-up, 109.199.124.187, is in `BTX_BOOTSTRAP_PEERS` (`crates/btx-core/src/node.rs:183`), not in `BTX_ARCHIVE_PEERS` (`:207`). `block_source_peers()` is both lists without the discovery relays, which serve no blocks, and every entry is dialled as a manual peer (`manual_peers()`, `:342`).

- [ ] **Step 1: Write the failing tests**

In `crates/btx-core/src/catchup_assist.rs`, replace the first lines of the test module:

```rust
mod tests {
    use super::*;

```

with:

```rust
mod tests {
    use super::*;
    use crate::error::AppResult;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::Mutex;

```

and insert this after the last test in `mod tests`, before the module's closing brace:

```rust


    /// A node with one chain up to `top`, at `tip`, that refuses to run any
    /// command the help must never send.
    struct FakeNode {
        top: u64,
        tip: u64,
        frontier: Option<u64>,
        special: Vec<(u64, &'static str)>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl FakeNode {
        fn new(top: u64, tip: u64, frontier: Option<u64>) -> Self {
            Self {
                top,
                tip,
                frontier,
                special: Vec::new(),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn hash(&self, h: u64) -> String {
            self.special
                .iter()
                .find(|(x, _)| *x == h)
                .map_or_else(|| hash(h), |(_, s)| s.to_string())
        }
        fn height_of(&self, hash: &str) -> Option<u64> {
            if let Some((h, _)) = self.special.iter().find(|(_, s)| *s == hash) {
                return Some(*h);
            }
            let h = u64::from_str_radix(hash, 16).ok()?;
            (h <= self.top && self.hash(h) == hash).then_some(h)
        }
        fn count(&self, method: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == method)
                .count()
        }
        fn asked(&self) -> Vec<(String, i64)> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == "getblockfrompeer")
                .map(|(_, p)| (p[0].as_str().unwrap().to_string(), p[1].as_i64().unwrap()))
                .collect()
        }
    }

    #[async_trait]
    impl Rpc for FakeNode {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), params.clone()));
            match method {
                "getbestblockhash" => Ok(json!(self.hash(self.tip))),
                "getmatmulattestedtip" => Ok(match self.frontier {
                    Some(f) => json!({"configured": true, "signed_frontier": {
                        "height": f, "hash": self.hash(f), "on_active_chain": true,
                        "blocks_behind": f - self.tip}}),
                    None => json!({"configured": false}),
                }),
                "getblockheader" => {
                    let h = self
                        .height_of(params[0].as_str().unwrap())
                        .ok_or(AppError::Rpc {
                            code: -5,
                            message: "Block not found".into(),
                        })?;
                    Ok(json!({"height": h, "previousblockhash": self.hash(h - 1)}))
                }
                "getblockhash" => Ok(json!(self.hash(params[0].as_u64().unwrap()))),
                "getblockfrompeer" => Ok(json!({})),
                other => panic!("the catch-up help must never call {other}"),
            }
        }
    }

    fn tick_of<'a>(
        blocks: u64,
        headers: u64,
        tips: &'a [ChainTip],
        peers: &'a [PeerInfo],
    ) -> Tick<'a> {
        Tick {
            blocks,
            headers,
            tips,
            peers,
        }
    }

    #[tokio::test]
    async fn a_tick_asks_the_archive_peer_for_the_next_hundred_by_name() {
        let node = FakeNode::new(300, 100, None);
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let t = tick_of(100, 300, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        assert_eq!(tick(&node, &mut cu, &t, t0).await, None);
        assert!(node.asked().is_empty());
        let line = tick(&node, &mut cu, &t, t0 + QUIET).await.unwrap();
        assert_eq!(
            line,
            "asked 109.199.124.187:19335 for blocks 101 to 200 (100 sent, 0 already here)"
        );
        let asked = node.asked();
        assert_eq!(asked.len(), 100);
        assert_eq!(asked[0], (hash(101), 7));
        assert_eq!(asked[99], (hash(200), 7));
    }

    #[tokio::test]
    async fn the_headers_are_read_once_and_kept() {
        let node = FakeNode::new(300, 100, None);
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &tick_of(100, 300, &tips, &peers), t0).await;
        tick(
            &node,
            &mut cu,
            &tick_of(100, 300, &tips, &peers),
            t0 + QUIET,
        )
        .await;
        assert_eq!(node.count("getblockheader"), 200);
        let moved = FakeNode::new(300, 150, None);
        let t = tick_of(150, 300, &tips, &peers);
        tick(&moved, &mut cu, &t, t0 + QUIET + secs(3)).await;
        assert_eq!(moved.count("getblockheader"), 0);
    }

    #[tokio::test]
    async fn nothing_is_read_while_no_header_is_twenty_ahead() {
        let node = FakeNode::new(300, 281, None);
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        for i in 0..20 {
            tick(
                &node,
                &mut cu,
                &tick_of(281, 300, &tips, &peers),
                t0 + secs(3 * i),
            )
            .await;
        }
        assert!(node.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn follows_the_signed_frontier_the_node_reads() {
        let node = FakeNode::new(300, 100, Some(160));
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let t = tick_of(100, 300, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &t, t0).await;
        tick(&node, &mut cu, &t, t0 + QUIET).await;
        let asked = node.asked();
        assert_eq!(asked.len(), 60);
        assert_eq!(asked.last().unwrap().0, hash(160));
    }

    #[tokio::test]
    async fn a_chain_through_a_refused_block_is_never_asked_for() {
        let root = crate::known_invalid::HELD_BRANCHES[0];
        let mut node = FakeNode::new(root.height + 150, root.height - 50, None);
        node.special.push((root.height, root.root));
        let top = root.height + 150;
        let tips = [ChainTip {
            height: top,
            hash: hash(top),
            branchlen: 1,
            status: "headers-only".into(),
        }];
        let peers = [peer(7, A, top as i64)];
        let t = tick_of(root.height - 50, top, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        tick(&node, &mut cu, &t, t0).await;
        tick(&node, &mut cu, &t, t0 + QUIET).await;
        tick(&node, &mut cu, &t, t0 + QUIET * 2).await;
        assert!(node.asked().is_empty());
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib catchup_assist 2>&1 | grep -E "^error" | head -3`
Expected: errors such as `error[E0422]: cannot find struct, variant or union type `Tick`` and `cannot find function `tick``.

- [ ] **Step 3: Implement**

Replace the imports at the top of `crates/btx-core/src/catchup_assist.rs`:

```rust
use std::time::{Duration, Instant};

use crate::fork::ChainTip;
use crate::header_path::HeaderPath;
use crate::node_api::PeerInfo;
```

with:

```rust
use std::time::{Duration, Instant};

use serde_json::json;

use crate::error::AppError;
use crate::fork::ChainTip;
use crate::header_path::{HeaderPath, PathStatus};
use crate::node_api::PeerInfo;
use crate::rpc::Rpc;
```

and insert this after `pub fn note(..)` (the last item before `#[cfg(test)]`):

```rust


/// What the refresher already read this tick.
#[derive(Debug, Clone, Copy)]
pub struct Tick<'a> {
    pub blocks: u64,
    pub headers: u64,
    pub tips: &'a [ChainTip],
    pub peers: &'a [PeerInfo],
}

/// The help's memory for one node run.
#[derive(Debug, Clone)]
pub struct CatchUp {
    helper: Helper,
    path: HeaderPath,
}

impl CatchUp {
    pub fn new(archive: Vec<String>) -> Self {
        Self {
            helper: Helper::new(archive),
            path: HeaderPath::new(),
        }
    }

    /// For this app's nodes: the peers it dials that can serve a block
    /// (`crate::node::block_source_peers`), in that order.
    pub fn for_this_app() -> Self {
        Self::new(
            crate::node::block_source_peers()
                .into_iter()
                .map(str::to_string)
                .collect(),
        )
    }
}

/// One refresher tick. Returns a line for the log when it asked for blocks,
/// stopped or paused. Reads nothing from the node while no header it knows is
/// [`MIN_BEHIND`] above the tip.
pub async fn tick(rpc: &dyn Rpc, cu: &mut CatchUp, t: &Tick<'_>, now: Instant) -> Option<String> {
    let was_helping = cu.helper.helping();
    let mut seen = Seen {
        now,
        tip: t.blocks,
        target: None,
        next: &[],
        peers: t.peers,
        refused: None,
    };
    if !cu.helper.enabled || t.blocks == 0 || t.headers < t.blocks.saturating_add(MIN_BEHIND) {
        let d = cu.helper.decide(&seen);
        return note(was_helping, &d);
    }
    let tip_hash = rpc.call("getbestblockhash", json!([])).await.ok()?;
    let tip_hash = tip_hash.as_str()?.to_string();
    let frontier = signed_frontier(rpc, &cu.path).await;
    let target = choose_target(frontier, t.tips, t.blocks);
    let mut next = Vec::new();
    if let Some((height, hash)) = &target {
        cu.path.retarget(*height, hash);
        cu.path.walk(rpc, t.blocks, WALK_PER_TICK).await.ok()?;
        if cu.path.status(t.blocks, &tip_hash) == PathStatus::Ready {
            next = cu.path.next(t.blocks, BATCH as usize);
            seen.refused = match refused_on_path(&cu.path) {
                Some(r) => Some(r),
                None => refused_on_own_chain(rpc, t.blocks).await,
            };
        }
    }
    seen.target = target.as_ref().map(|(h, _)| *h);
    seen.next = &next;
    let d = cu.helper.decide(&seen);
    let Decision::Ask {
        peer_id,
        addr,
        blocks,
    } = &d
    else {
        return note(was_helping, &d);
    };
    let (mut accepted, mut have) = (0, 0);
    for (_, hash) in blocks {
        match rpc.call("getblockfrompeer", json!([hash, peer_id])).await {
            Ok(_) => accepted += 1,
            Err(AppError::Rpc { message, .. }) if message.contains("already downloaded") => {
                have += 1
            }
            Err(_) => {}
        }
    }
    cu.helper.sent(&d, now, accepted, have);
    let (first, last) = (blocks.first()?.0, blocks.last()?.0);
    Some(if accepted + have == 0 {
        format!(
            "{addr} took none of blocks {first} to {last}; the next archive peer is asked later"
        )
    } else {
        format!("asked {addr} for blocks {first} to {last} ({accepted} sent, {have} already here)")
    })
}

/// The signed frontier, when the node reads one and knows its header.
async fn signed_frontier(rpc: &dyn Rpc, path: &HeaderPath) -> Option<(u64, String)> {
    let t = crate::node_api::get_attested_tip(rpc).await.ok()?;
    let (height, hash) = (t.height?, t.hash?);
    let known = path.target() == Some((height, hash.as_str()))
        || rpc
            .call("getblockheader", json!([hash, true]))
            .await
            .is_ok();
    known.then_some((height, hash))
}

/// A refused block the node's own chain already contains, at or below `tip`.
async fn refused_on_own_chain(rpc: &dyn Rpc, tip: u64) -> Option<&'static str> {
    for (height, hash) in crate::known_invalid::refused_blocks() {
        if height > tip {
            continue;
        }
        if let Ok(v) = rpc.call("getblockhash", json!([height])).await {
            if v.as_str() == Some(hash) {
                return Some(hash);
            }
        }
    }
    None
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib catchup_assist`
Expected: `test result: ok. 21 passed; 0 failed`.

- [ ] **Step 5: Format, lint, full suite, commit**

```bash
cd crates/btx-core && cargo fmt --all && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cargo test --locked 2>&1 | grep "test result" | head -1 && cd ../..
git add crates/btx-core/src/catchup_assist.rs
git commit -F - <<'EOF'
core: one catch-up tick: read, walk, decide, ask by name

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

Expected: `Finished ...`, then `test result: ok.` with 0 failed.

---

### Task 6: The help against real engines on regtest (opt-in)

**Files:**
- Create: `crates/btx-core/tests/catchup_regtest.rs`

**Interfaces:**
- Consumes: `catchup_assist::{CatchUp, Tick, tick}` (Task 5), `node_api::{get_blockchain_info, get_chain_tips, get_peer_info}`, `rpc::{Rpc, RpcClient}` (existing; `RpcClient::from_cookie` as in `tests/console_regtest.rs`).

A test of the whole path through a real engine: the stall appears (B's engine asks for nothing), the help moves it, and it sends nothing but the allowed commands. It needs a v0.34.9 `btxd`; without `EASYNODE_TEST_BTXD` it returns early, and `#[ignore]` keeps it out of CI. It takes about 100 seconds (80 of them mining A's 400 blocks) and uses RPC ports 29451/29452 and P2P port 29461.

- [ ] **Step 1: Write the test**

Create `crates/btx-core/tests/catchup_regtest.rs`:

```rust
//! The catch-up help against two real v0.34.9 engines on regtest. Opt-in:
//!
//! ```text
//! EASYNODE_TEST_BTXD=/path/to/btxd cargo test --test catchup_regtest -- --ignored --nocapture
//! ```
//!
//! The mainnet stall of 2026-09-29 in miniature. Node A plays the archive
//! peer: pruned (`-prune=1`, so it advertises NETWORK_LIMITED like
//! 109.199.124.187) with 400 blocks, and it grants the test's loopback
//! address `noban`, which is what lets a limited node serve a block deeper
//! than 288 when asked for it by name. Node B holds all 400 headers and block
//! 1, and its only peer is A. Blocks 2 to 101 are more than 288 below A's
//! tip, so B's engine asks A for none of them: its tip stays at 1. The help,
//! ticked the way the refresher ticks it, must ask A for blocks 2 to 101 by
//! name and move B's tip, and must send no command but the ones it is allowed.
//!
//! Heights up to 99 connect without ExactReplay on regtest, so the test asks
//! for 99, which takes seconds on any machine. On a Mac whose device fails the
//! engine's self-test, A's own block 100 connects only after a restart; the
//! test does that when it has to (measured 2026-09-29 on the owner's M2 Pro).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use btx_core::catchup_assist::{self, CatchUp, Tick};
use btx_core::error::AppResult;
use btx_core::node_api::{get_blockchain_info, get_chain_tips, get_peer_info};
use btx_core::rpc::{Rpc, RpcClient};
use serde_json::{json, Value};

const A_RPC: u16 = 29451;
const A_P2P: u16 = 29461;
const B_RPC: u16 = 29452;

/// Every command the help may send. Anything else fails the test.
const ALLOWED: &[&str] = &[
    "getbestblockhash",
    "getmatmulattestedtip",
    "getblockheader",
    "getblockhash",
    "getblockfrompeer",
];

struct Btxd {
    child: std::process::Child,
    rpc: RpcClient,
}

impl Drop for Btxd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn start(
    bin: &std::ffi::OsStr,
    dir: &std::path::Path,
    rpc_port: u16,
    args: &[String],
) -> Btxd {
    let mut child = std::process::Command::new(bin)
        .arg(format!("-datadir={}", dir.display()))
        .arg(format!("-rpcport={rpc_port}"))
        .args([
            "-regtest",
            "-server=1",
            "-printtoconsole=0",
            "-daemon=0",
            "-dnsseed=0",
            "-fixedseeds=0",
            "-discover=0",
            "-allowunverifiablematmulconsensus=1",
        ])
        .args(args)
        .spawn()
        .unwrap();
    let cookie = dir.join("regtest").join(".cookie");
    let url = format!("http://127.0.0.1:{rpc_port}");
    for _ in 0..120 {
        if let Ok(c) = RpcClient::from_cookie(&url, &cookie) {
            if c.call("getblockcount", json!([])).await.is_ok() {
                return Btxd { child, rpc: c };
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("btxd on {rpc_port} did not answer within 60 s");
}

async fn stop(mut node: Btxd) {
    let _ = node.rpc.call("stop", json!([])).await;
    for _ in 0..120 {
        if node.child.try_wait().unwrap().is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn blocks(rpc: &RpcClient) -> u64 {
    get_blockchain_info(rpc).await.unwrap().blocks
}

/// The node under test, recording every command sent to it.
struct Watched<'a> {
    inner: &'a RpcClient,
    methods: Mutex<Vec<String>>,
}

#[async_trait]
impl Rpc for Watched<'_> {
    async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
        self.methods.lock().unwrap().push(method.to_string());
        self.inner.call(method, params).await
    }
}

#[tokio::test]
#[ignore]
async fn the_help_moves_a_node_its_engine_leaves_standing() {
    let Some(bin) = std::env::var_os("EASYNODE_TEST_BTXD") else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let a_args: Vec<String> = [
        "-listen=1",
        "-bind=127.0.0.1",
        "-listenonion=0",
        "-connect=0",
        "-prune=1",
        "-whitelist=noban@127.0.0.1",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([format!("-port={A_P2P}")])
    .collect();

    // A: 400 blocks.
    let mut a = start(&bin, dir_a.path(), A_RPC, &a_args).await;
    a.rpc.call("createwallet", json!(["w"])).await.unwrap();
    let addr = a.rpc.call("getnewaddress", json!([])).await.unwrap();
    a.rpc
        .call("generatetoaddress", json!([99, addr]))
        .await
        .unwrap();
    let _ = tokio::time::timeout(
        Duration::from_secs(20),
        a.rpc.call("generatetoaddress", json!([1, addr])),
    )
    .await;
    if blocks(&a.rpc).await < 100 {
        stop(a).await;
        a = start(&bin, dir_a.path(), A_RPC, &a_args).await;
        let _ = a.rpc.call("loadwallet", json!(["w"])).await;
        for _ in 0..120 {
            if blocks(&a.rpc).await >= 100 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    while blocks(&a.rpc).await < 400 {
        let n = (400 - blocks(&a.rpc).await).min(50);
        a.rpc
            .call("generatetoaddress", json!([n, addr]))
            .await
            .unwrap();
    }

    // B: every header, then block 1, with A as its only peer. Block 1 goes in
    // after the headers: a node whose tip is recent fetches the blocks of the
    // next headers it hears straight away (up to 16), which on mainnet a node
    // days behind never does.
    let b = start(
        &bin,
        dir_b.path(),
        B_RPC,
        &[
            "-listen=0".to_string(),
            format!("-connect=127.0.0.1:{A_P2P}"),
        ],
    )
    .await;
    for _ in 0..120 {
        if get_blockchain_info(&b.rpc).await.unwrap().headers >= 400 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let one = a.rpc.call("getblockhash", json!([1])).await.unwrap();
    let raw = a.rpc.call("getblock", json!([one, 0])).await.unwrap();
    b.rpc.call("submitblock", json!([raw])).await.unwrap();
    let info = get_blockchain_info(&b.rpc).await.unwrap();
    assert_eq!((info.blocks, info.headers), (1, 400));

    // Left alone, B's engine asks A for none of blocks 2 to 101.
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(blocks(&b.rpc).await, 1, "the engine fetched on its own");

    // The help, ticked like the refresher: 3 s of its clock per tick.
    let watched = Watched {
        inner: &b.rpc,
        methods: Mutex::new(Vec::new()),
    };
    let archive = format!("127.0.0.1:{A_P2P}");
    let mut cu = CatchUp::new(vec![archive.clone()]);
    let clock = Instant::now();
    let mut lines = Vec::new();
    for i in 0..200u32 {
        let info = get_blockchain_info(&b.rpc).await.unwrap();
        if info.blocks >= 99 {
            break;
        }
        let tips = get_chain_tips(&b.rpc).await.unwrap();
        let peers = get_peer_info(&b.rpc).await.unwrap();
        let t = Tick {
            blocks: info.blocks,
            headers: info.headers,
            tips: &tips,
            peers: &peers,
        };
        let now = clock + Duration::from_secs(3 * u64::from(i));
        if let Some(line) = catchup_assist::tick(&watched, &mut cu, &t, now).await {
            eprintln!("catch-up help: {line}");
            lines.push(line);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let reached = blocks(&b.rpc).await;
    assert!(reached >= 99, "tip {reached}; the help said {lines:?}");
    // The first request names exactly the next hundred. How many the node
    // took depends on the connection: in the dry run of 2026-09-29 the peer
    // dropped after 11 of the first 100, the help paused, and asked again.
    assert!(
        lines[0].starts_with(&format!("asked {archive} for blocks 2 to 101 (")),
        "{lines:?}"
    );
    for m in watched.methods.lock().unwrap().iter() {
        assert!(ALLOWED.contains(&m.as_str()), "the help sent {m}");
    }

    stop(b).await;
    stop(a).await;
}
```

- [ ] **Step 2: Run it without an engine (CI's view)**

Run: `cd crates/btx-core && cargo test --locked --test catchup_regtest 2>&1 | grep "test result"`
Expected: `test result: ok. 0 passed; 0 failed; 1 ignored`.

- [ ] **Step 3: Run it against the shipped engine**

Run with any v0.34.9 `btxd`: on the owner's Mac, `apps/node/src-tauri/resources/node-pkg/bin/btxd` once `apps/node/scripts/stage-node-pkg.sh` has staged the package (Task 10 Step 1), or the copy the 2026-09-29 spike used:

```bash
cd crates/btx-core
EASYNODE_TEST_BTXD=/path/to/btxd cargo test --locked --test catchup_regtest -- --ignored --nocapture 2>&1 | grep -E "catch-up help|test result"
```

Expected (the middle lines vary with the connection; this is the dry run of 2026-09-29):

```text
catch-up help: asked 127.0.0.1:29461 for blocks 2 to 101 (16 sent, 0 already here)
catch-up help: paused: none of this app's archive peers is connected with the next block
catch-up help: asked 127.0.0.1:29461 for blocks 2 to 101 (100 sent, 0 already here)
test result: ok. 1 passed; 0 failed; 0 ignored
```

If it fails at "the engine fetched on its own", check that block 1 went in after the headers (the test's comment says why). If A never passes 99, the engine qualified the device differently: read `$TMPDIR/.../regtest/debug.log` of A for "ExactReplay required before activation".

- [ ] **Step 4: Lint and commit**

```bash
cargo fmt --all && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cd ../..
git add crates/btx-core/tests/catchup_regtest.rs
git commit -F - <<'EOF'
core: the catch-up help moves a stalled regtest node, proven against the engine (opt-in)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

Clippy's `zombie_processes` lint (in `suspicious`) is why `start` kills and waits for the child before it panics.

---

### Task 7: The watchdog names the help

**Files:**
- Modify: `crates/btx-core/src/watchdog.rs:240-247` (the `BlockFetchGated` verdict) and `:546-572` (its test)

**Interfaces:**
- Consumes: `catchup_assist::MIN_BEHIND` (Task 4), in the test only.

The sentence is shown on the validation card (`apps/node/src/validation.ts:85-90`) when a mirror has been frozen for 15 minutes with nothing in flight.

- [ ] **Step 1: Change the test first**

In `crates/btx-core/src/watchdog.rs`, in `fn zero_in_flight_with_headers_ahead_is_the_scheduler_gate_not_the_peer_set`, replace:

```rust
        // And it must not promise a repair nothing performs. The copy used to
        // end "which the guardian does automatically"; there is no
        // getblockfrompeer call anywhere in this workspace, so the sentence
        // told a stuck operator to wait for a fix that was never coming.
        assert!(
            !v.summary.contains("automatically"),
            "do not claim an automatic fix unless something in the tree performs it"
        );
    }
```

with:

```rust
        // It names the help that now exists (crate::catchup_assist), and only
        // what that help does: it asks, and peers may not answer. The copy once
        // ended "which the guardian does automatically" while nothing did.
        assert!(v.summary.contains("asks its own archive peers"));
        assert!(!v.summary.contains("nothing in this app"));
        assert!(
            !v.summary.contains("automatically") && !v.summary.contains("will fix"),
            "say what the app does, never promise the outcome"
        );
        assert!(!v.summary.contains('\u{2014}'), "no em-dashes in copy");
        assert!(v
            .summary
            .contains(&crate::catchup_assist::MIN_BEHIND.to_string()));
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cd crates/btx-core && cargo test --locked --lib zero_in_flight_with_headers_ahead 2>&1 | grep -E "panicked|test result"`
Expected: `panicked at src/watchdog.rs:...: assertion failed: v.summary.contains("asks its own archive peers")`.

- [ ] **Step 3: Change the sentence**

Replace:

```rust
            summary: "this node can see the next blocks and is not asking any peer for them \
                      (a known upstream scheduler bug); adding or redialling peers will NOT \
                      help — the fix is to request the next blocks by name with \
                      getblockfrompeer, which nothing in this app does yet",
```

with:

```rust
            summary: "this node can see the next blocks and is not asking any peer for them \
                      (a known upstream scheduler bug), so adding or redialling peers will NOT \
                      help. While it is 20 or more blocks behind, this app asks its own \
                      archive peers for the next blocks by name. If the tip still does not \
                      move, none of them may be connected right now",
```

- [ ] **Step 4: Run the watchdog tests**

Run: `cd crates/btx-core && cargo test --locked --lib watchdog 2>&1 | grep "test result" | head -1`
Expected: `test result: ok. 16 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
cd crates/btx-core && cargo fmt --all && cd ../..
git add crates/btx-core/src/watchdog.rs
git commit -F - <<'EOF'
core: the stalled-scheduler verdict names the catch-up help

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 8: The status refresher runs the help

**Files:**
- Modify: `apps/node/src-tauri/src/commands.rs:1593-1594` (before the refresher's `loop {`) and `:2071-2074` (between the fork block and the watchdog tick)

**Interfaces:**
- Consumes: `catchup_assist::{CatchUp, Tick, tick}` (Task 5). From the refresher's own tick: `chain` (`getblockchaininfo`, read every tick), `peer_infos: Option<Vec<PeerInfo>>` (`:1697`, one `getpeerinfo` per tick), `fork_tips: Vec<ChainTip>` (`getchaintips`, refreshed every `FORK_CHECK_EVERY` = 10 ticks, `:1572`), `rpc: RpcClient`, `bootstrap_launch`, and `setup_log` (`:3350`).

The refresher (`spawn_status_refresher`, `:1518`) is spawned per node run, so the help's memory starts fresh on every start and restart. A tip that is 30 seconds old at most is fine for the target: the walk joins the tip from whatever header the tips named. Lines go to stderr and to `<datadir>/setup.log`, where the header bootstrap already writes its end message.

- [ ] **Step 1: Add the help's state**

In `apps/node/src-tauri/src/commands.rs`, replace:

```rust
        let mut bootstrap_moved_at = std::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
```

with:

```rust
        let mut bootstrap_moved_at = std::time::Instant::now();
        // Catch-up help (btx_core::catchup_assist, decision 2026-09-29 §11):
        // while the node is behind and the engine asks nobody for the next
        // blocks, ask this app's archive peers for them by name. Its memory
        // (the walked headers, the batch out) lasts for this run only.
        let mut catch_up = btx_core::catchup_assist::CatchUp::for_this_app();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
```

- [ ] **Step 2: Tick it**

Replace:

```rust
                        *fork_slot.lock().await = verdict;
                    }

                    // ── Trusted-mirror stall watchdog tick ──────────────────
```

with:

```rust
                        *fork_slot.lock().await = verdict;
                    }

                    // ── Catch-up help ───────────────────────────────────────
                    // Every node, every tick, from the reads above; it reads
                    // more from the node only while a header it knows is 20
                    // or more blocks above the tip. Not during a header
                    // bootstrap, which ends in a restart.
                    if !bootstrap_launch {
                        if let Some(peers) = peer_infos.as_deref() {
                            let tick = btx_core::catchup_assist::Tick {
                                blocks: chain.blocks,
                                headers: chain.headers,
                                tips: &fork_tips,
                                peers,
                            };
                            if let Some(line) = btx_core::catchup_assist::tick(
                                &rpc,
                                &mut catch_up,
                                &tick,
                                std::time::Instant::now(),
                            )
                            .await
                            {
                                let msg = format!("catch-up help: {line}");
                                eprintln!("[node-app] {msg}");
                                setup_log(&node_datadir(), &msg);
                            }
                        }
                    }

                    // ── Trusted-mirror stall watchdog tick ──────────────────
```

- [ ] **Step 3: Run the shell's checks**

Run: `cd apps/node/src-tauri && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cargo test --locked 2>&1 | grep "test result" | head -1`
Expected: no fmt output, `Finished ...`, and the count from Task 3 Step 1 with 0 failed. (The refresher has no unit test; Task 6 drives `tick` exactly as this block does, and Task 10 runs it for real.)

- [ ] **Step 4: Commit**

```bash
git add apps/node/src-tauri/src/commands.rs
git commit -F - <<'EOF'
node: the status refresher asks archive peers for the next blocks while the engine asks nobody

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 9: Changelog, and every check CI runs

**Files:**
- Modify: `apps/node/CHANGELOG.md` (under `## [Unreleased]`, after the Tools paragraph)

- [ ] **Step 1: Add the entry**

In `apps/node/CHANGELOG.md`, after the paragraph that starts `**Tools: the things support used to need a terminal for, behind one button.**` and before `## [0.6.32] - 2026-09-28`, insert:

```markdown
**Catching up no longer waits minutes for every block.** Most nodes on the
network keep only recent blocks, and the node engine asks them for an older
block only after two minutes of waiting, so a node that starts days behind
gained about one block every two minutes. When your node is 20 or more blocks
behind and nobody has been asked for its next block for 30 seconds, the app
now asks its own archive peers for the next 100 blocks by name, waits for them
to arrive, and repeats. It stops as soon as the engine is fetching blocks on
its own again. On 29 September this moved a test node 7,490 blocks in 8
minutes. It never adds, bans or disconnects a peer, and never asks for a
branch the app refuses.

```

- [ ] **Step 2: Run every check CI runs**

```bash
(cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cargo test --locked 2>&1 | grep -E "test result|FAILED")
(cd apps/node/src-tauri && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cargo test --locked 2>&1 | grep -E "test result|FAILED")
(cd apps/node && npm ci && npx tsc --noEmit && npm test 2>&1 | tail -3)
git diff claude/tools-command-window -- crates apps | grep '^+' | grep -c '—'
```

Expected: every `test result: ok.` with 0 failed and no `FAILED`; both clippy runs end `Finished ...`; `tsc` prints nothing; vitest reports all passed; the em-dash count is `0` (none of this plan's added lines has one).

- [ ] **Step 3: Commit**

```bash
git add apps/node/CHANGELOG.md
git commit -F - <<'EOF'
changelog: catch-up help

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 10: Mainnet acceptance on the owner's Mac (by hand, no commit)

Mirrors the measurement of 2026-09-29: a mirror that starts from the signed snapshot at 225,927, now about 8,000 blocks behind. A fresh data folder on this Mac starts exactly there (the M2 Pro fails the engine's device self-test, so the app runs it as a mirror, and every mirror since 0.6.31 starts from the pinned pair at 225,927).

- [ ] **Step 1: Stage the engine and free the ports**

```bash
cd /Users/m2promende/repos/easynode/.claude/worktrees/catch-up-help
apps/node/scripts/stage-node-pkg.sh
pgrep -fl btxd || echo "no btxd running"
```

The script reads the v0.34.9 package from `$EASYBTX_NODE_PKG_SRC` or `~/btx-node-research/btx-0.34.9` (see its header), and replaces the placeholder from "Before you start". Quit the installed easyNode first if it is running (its node uses the same ports). Expected: `no btxd running`.

- [ ] **Step 2: Run the app on a throwaway data folder**

```bash
cd apps/node
export EASYBTX_NODE_DATADIR="$HOME/easynode-catchup-accept"
npx tauri dev
```

Go through setup in the window. In a second terminal, once the snapshot has loaded (the node shows about 225,927):

```bash
D="$HOME/easynode-catchup-accept"
CLI=/Users/m2promende/repos/easynode/.claude/worktrees/catch-up-help/apps/node/src-tauri/resources/node-pkg/bin/btx-cli
tail -f "$D/setup.log" | grep --line-buffered "catch-up help" &
while sleep 60; do date -u +%T; $CLI -datadir="$D" getblockcount; done
```

- [ ] **Step 3: Check what happens**

Pass, all of:
1. Within about 2 minutes of the snapshot load, `setup.log` shows `catch-up help: asked 109.199.124.187:19335 for blocks 225928 to 226027 (...)` (or another listed peer).
2. The block count rises by at least 5,000 within the first 10 minutes of the first "asked" line (the measurement did 7,490 in 8; this is half that speed).
3. Near the tip, `setup.log` shows `catch-up help: stopped: the node is within 20 blocks of the chain it follows`, and for the next 5 minutes the count follows new blocks with no further "asked" lines.
4. The node stayed off the held branch: `$CLI -datadir="$D" getblockhash 228146` does not start with `8240c62e`.

Record the numbers (first "asked" time, count at +5 and +10 minutes, time of "stopped") in the pull request description.

- [ ] **Step 4: A second machine, if one is at hand**

Repeat Steps 1 to 3 on a machine with a different public IP (see Risk 1). A pass there is the evidence that the archive peers serve old blocks to nodes other than this Mac.

- [ ] **Step 5: Clean up**

Stop the node in the window, quit the app, and delete `~/easynode-catchup-accept` yourself once the numbers are recorded.

---

## Decisions this plan made where the design is silent

1. **Which peers are "the archive peers it dials"**: manual connections whose address is in `node::block_source_peers()`. `BTX_ARCHIVE_PEERS` alone would leave out 109.199.124.187, the peer the measurement used.
2. **"The engine is fetching on its own"**: any of the next 100 heights in flight at any peer, minus this help's own batch. `getblockfrompeer` puts the help's own requests in `inflight` (engine `FetchBlock`, confirmed on regtest), so they must not count.
3. **"The next block has not been requested for 30 seconds"** is measured per tip: the clock restarts when the tip moves or when any of the next 100 is in flight.
4. **After a batch connects the next goes out at once**, without another 30-second wait, as in the measured run. The 30-second wait applies only to starting, and after a peer took nothing.
5. **Rotation happens at three minutes, and at once when the batch's peer disconnects.** A peer that takes none of a batch is rotated away from after another 30-second wait, so a refusing peer is not hammered.
6. **"Block already downloaded" counts as taken**, so a node that has the bodies and is still checking them is not moved from peer to peer.
7. **The signed frontier wins when the node reads one and knows its header.** A node at or above its frontier gets no help even if its best header is higher, because a mirror cannot connect unsigned blocks.
8. **Refused blocks are never fetched, even with `EASYBTX_NODE_REFUSE_KNOWN_INVALID=0`.** The help only helps; the engine can still fetch whatever the operator lets it.
9. **No help when `blocks == 0` or during a header bootstrap launch** (the snapshot has not loaded; asking for blocks from genesis would be waste).
10. **1,000 headers per tick** while walking; no cap on the walk's length, so a node 30,000 behind is helped too (about 5 MB of hashes).
11. **Where it speaks**: stderr and `<datadir>/setup.log`, one line per batch, stop or pause. No new screen element; the design names none.

## Risks

1. **The archive peer may not serve every node.** Engine v0.34.9 `src/net_processing.cpp:9385`: a NETWORK_LIMITED node disconnects a requester that asks for a block more than 290 below its tip unless it grants that requester `noban`. 109.199.124.187 and btxscan's mirror advertise NETWORK_LIMITED, yet 109.199.124.187 served 7,490 old blocks to the owner's Mac. Either it grants `noban` widely, or to that IP. On regtest the same requests failed until the serving node granted `noban` to the requester. If other nodes are refused, the help rotates and backs off harmlessly, but it will not speed them up. Task 10 Step 4 is the check; asking the operator of 109.199.124.187 is the other.
2. **The engine may drop a slow peer.** `getblockfrompeer`'s own help says "When a peer does not respond with a block, we will disconnect." The engine is gentler with manual and `noban` peers (`CatchUpMayDisconnectOnSlowDelivery` takes `IsManualConn() || m_noban`), and the help asks only manual ones. On regtest the connection dropped mid-batch twice (after 11 and 16 of 100); the help paused and resumed.
3. **The engine's own rescue keeps the next block in flight** after it has been stuck for 120 seconds, which holds off the 30-second quiet clock. The help starts inside the first 120 seconds after each block, which is where the measured stall spent most of its time, and once helping it ignores the rescue's request because it is inside the help's own batch.
4. **The watchdog runs on mirrors only, after 15 minutes frozen**, so its sentence reaches mirrors only. A validating node gets the help but no sentence about it.
5. **The branch this plan builds on is not pushed.** `claude/tools-command-window` exists with this code only locally (`62a03f1` and moving). Merge order: Tools first, then this.

## Self-review

- **Spec coverage.** Section 11: trigger (20 behind, 30 s): Task 4 rules 2 and 6, tests `does_nothing_within_twenty_blocks`, `waits_thirty_quiet_seconds_then_asks_for_the_next_hundred`, `the_next_block_in_flight_restarts_the_quiet_wait`, `a_new_tip_restarts_the_quiet_wait`. Archive peers it dials: Task 4 `pick`, test `asks_only_the_app_s_own_archive_peers`. Next 100 by name: Task 5, tests `a_tick_asks_the_archive_peer_for_the_next_hundred_by_name`, `never_asks_past_what_the_peer_announced`, Task 6. Wait to connect: `waits_for_a_batch_to_connect_then_asks_for_the_next_at_once`. Rotation after three minutes: `rotates_to_the_next_archive_peer_after_three_minutes`, `a_peer_that_disconnects_is_replaced_at_once`. Signed frontier else best header: `choose_target` and `follows_the_signed_frontier_*` (both). Never add, ban, disconnect: `FakeNode` panics on any other method; Task 6's `ALLOWED`. Stop when the engine fetches: `stops_when_the_engine_asks_for_the_next_blocks_itself`. Watchdog wording: Task 7. Rollback "one constant": `ENABLED`, test `a_refused_block_or_the_switch_stops_it`. Tests section "target and peer choice over recorded RPC answers": Task 5's `FakeNode` tests and Task 4. Refused held branches: `a_chain_through_a_refused_block_is_never_asked_for`. Cheap repeated walk: `the_headers_are_read_once_and_kept`, and Task 2's tests. Tools shares the walk: Task 3. Real engine: Task 6. Mainnet: Task 10.
- **Placeholders.** None. Every code step carries the code; it is the code that passed `cargo fmt --check`, both clippy runs and both test suites in a throwaway worktree on 2026-09-29 (btx-core: all tests green, including the 32 new ones; shell: 96 passed), and Task 6 passed against v0.34.9.
- **Type consistency.** `HeaderPath`/`PathStatus` (Task 2) are used with the same names in Tasks 3 and 5. `Helper`, `Seen`, `Decision`, `Why` (Task 4) and `Tick`, `CatchUp`, `tick` (Task 5) match their uses in Tasks 6 and 8. `AttestedTip::hash` (Task 1) is read in `signed_frontier` (Task 5). `refused_blocks` (Task 1) is used by `refused_on_path` (Task 4) and `refused_on_own_chain` (Task 5). `MIN_BEHIND` (Task 4) is read by Task 7's test.
