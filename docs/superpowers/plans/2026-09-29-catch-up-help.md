# Catch-up Help Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Amended 2026-09-29 (night).**
> 1. **Rebased on `origin/main` at `b330e3d`.** Tools is squash-merged into main as `aed8755` (#154), and #160 added a `signed_frontier` slot to `AppState` that the refresher fills on every node every tick, and moved the tip-age block above the role in `get_node_status`. Every file:line and find-this-text anchor below was re-checked against `b330e3d`. The Tools quick action "Fetch a stuck block" (the changelog's "ask good peers for a block") and the watchdog's `BlockFetchGated` sentence are on main: this plan changes both in place and adds no second of either.
> 2. **The engine's disconnect rule, read from source** (84b998b4 `src/net_processing.cpp:9384-9391`, v0.34.12 `:9922-9929`, identical): a serving peer disconnects a requester it does not grant `noban` when the block asked for is more than 290 below its tip, but only if the serving peer advertises NETWORK_LIMITED and not NODE_NETWORK. A NODE_NETWORK peer serves old blocks to anyone, unless it has reached its outbound target. Peer choice now puts NODE_NETWORK peers first for old blocks and a limited peer last; a peer that drops us twice this run while asked for old blocks is not asked for them again this run, and the log and Copy diagnostics say so (Tasks 1, 4, 5, 9; Task 6 proves it against the engine). Two strikes, and the mark for any peer, full-history or limited, are the controller's decisions of the same night; the Tools button stays as it is.
> 3. **Section 11's thresholds are unchanged.** A new test pins that a node 200 behind, whose next block the engine is fetching, gets no help: the case after a load from a confirmed snapshot (Task 4).
> 4. **The target reads the frontier from the `signed_frontier` slot.** The help sends no `getmatmulattestedtip` of its own (Tasks 5, 6, 8).
> 5. **Copy:** friendly, simple, no hype, no guarantees, no em-dashes in any new or touched user-facing string.
> 6. **When no archive peer serves old blocks, both** (the owner's decision 1): the help pauses, logs one plain line, and says so in Copy diagnostics and on the status card, and it exposes the conclusion (`CatchUp::no_archive_serves_old_blocks()`, and `no_archive_serves_old_blocks` in the `catch_up_help` slot) for the Fast-forward plan to offer Fast-forward below its 1,000-block line (Tasks 4, 5, 7, 8; Decision 19).
>
> Code in the amended steps is **derived, not run**: nothing was compiled or tested for this amendment (the rule for tonight: no cargo, no npm, no node contacted). Counts that could only come from a run say so. The owner's and the controller's answers to this amendment's questions are at the end, under "For the owner"; only the second-IP test is still pending.

> **Amended 2026-09-30, after the final review and the first mainnet run** (below, "Mainnet acceptance, 30 September").
> 1. **"The engine is fetching on its own" means its blocks are arriving, not that requests exist.** On mainnet the help never started: the engine's next three blocks sat in flight for good at the app's three discovery relays. The quiet wait now restarts only when the tip moves, and the engine counts as fetching only while one of the next 100 is in flight outside the help's batch and it connected a block of its own within the last 30 seconds (Decisions 2 and 3, the constraints, Risk 3). The no-archive conclusion also ends when the engine really fetches (Decision 19).
> 2. **A node that does not answer is not a peer refusing** (Decision 20), **each tick asks the node for about 1.5 seconds** (Decision 21), and **a drop that looks like this Mac losing its connections is not counted** (Decision 22).
> 3. **The tip's hash is read with `getblockhash` at the tick's height**, so the help no longer sends `getbestblockhash`; **the status card leaves Fast-forward out** until the Fast-forward branch adds it with the button (Decision 19).
>
> The code in the Tasks below is the plan as it was first built. Where it differs from the branch, the branch's code and tests are the reference.

**Goal:** While a node is 20 or more blocks behind and its newest block has not changed for 30 seconds, the app asks its own archive peers for the next 100 blocks by name (`getblockfrompeer`), full-history peers first, waits for them to connect, and repeats, until the engine fetches on its own again or the node is within 20 blocks.

**Architecture:** Two new pure-ish modules in btx-core. `header_path.rs` walks the header chain from a target down to the tip with `getblockheader`, keeps it between calls and extends it instead of re-walking; the Tools button "Fetch a stuck block" uses the same walk. `catchup_assist.rs` holds a pure state machine (`Helper::decide`, tested with synthetic clocks and recorded `getpeerinfo` shapes) and one async `tick` that reads what the decision needs, sends the requests and reports back. The Tauri status refresher calls `tick` once per 3-second tick with the reads it already makes, the signed frontier included, and keeps the help's own summary in a state slot that Copy diagnostics prints; the watchdog's `BlockFetchGated` sentence names the help.

**Tech Stack:** Rust 2021 (stable toolchain, rustc 1.95 on the owner's Mac), tokio, serde_json, async-trait; Tauri 2 shell; engine btxd v0.34.9 = commit 84b998b4 (RPC `getblockfrompeer`, `getblockheader`, `getblockhash`; the refresher's own `getmatmulattestedtip`, `getpeerinfo`, `getchaintips`).

**Design:** `docs/decisions/2026-09-29-every-node-starts-near-the-tip.md` section 11 and its Context note "Limited peers serve 288 blocks", as amended the night of 2026-09-29 (base: `origin/claude/cosigned-snapshots`). This plan covers section 11, its line in "Rollback" (the help sits behind one constant), its line in "Tests" ("Catch-up help: target and peer choice over recorded RPC answers"), and the watchdog wording change section 11 names. Nothing else from that document.

## Global Constraints

- Help only when the followed chain is **at least 20 blocks** above the tip (`MIN_BEHIND = 20`).
- Start only after **the newest block has not changed for 30 seconds** (`QUIET = 30 s`), whatever requests the engine has out: a request that is never answered does not hold the help off (Decision 3).
- Ask for **the next 100 blocks** by name, from one peer (`BATCH = 100`), then **wait for them to connect** before asking for more.
- **Rotate to the next archive peer if a batch has not connected within three minutes** (`ROTATE_AFTER = 180 s`).
- Follow the chain toward **the signed frontier the node reads** (`signed_frontier.hash`, from the refresher's `signed_frontier` slot, which #160 fills from `getmatmulattestedtip` on every node every tick; the help sends no `getmatmulattestedtip` of its own); a node that reads none follows **its best header** (`stuck_blocks::target_tip`).
- Ask only **the archive peers the app dials**: `connection_type == "manual"` in `getpeerinfo` and the address in `btx_core::node::block_source_peers()`.
- **Old blocks from full-history peers first.** A block is old for a peer when it is more than **288** below the last header that peer announced (`synced_headers`; `LIMITED_SERVES = 288`, the engine's `NODE_NETWORK_LIMITED_MIN_BLOCKS`). For old blocks, peers advertising NODE_NETWORK (service bit 0) come first; a peer advertising NETWORK_LIMITED without NODE_NETWORK is asked only after every full-history peer has had its turn. Two exceptions (Decision 13): a limited peer whose last batch of old blocks connected goes ahead of a full-history peer not asked yet, and a full-history peer silent on old blocks for three minutes ranks below the limited ones until it delivers.
- **A peer that drops us twice this run while asked for old blocks is not asked for them again this run** (`DROPS_TO_MARK = 2`). "Drops us": its connection id is gone from `getpeerinfo` (gone, or back under a new id) before any block of a batch of old blocks it was asked for connected. After one drop the peer is rotated away from, so the other archive peers are asked first, and it may be asked again. The mark applies to any peer, full-history (NODE_NETWORK) or limited. A marked peer is still asked for blocks within 288 of its height. A drop that looks like this Mac losing its connections is not counted (Decision 22). The log says each drop once, and Copy diagnostics says so for the rest of the run.
- **Never add, ban or disconnect a peer, and never reconnect one.** The only state-changing command the help sends is `getblockfrompeer`. A test fails on any other.
- **Never fetch a refused block**: no request toward a chain that passes an entry of `known_invalid::KNOWN_INVALID_BLOCKS` or `known_invalid::HELD_BRANCHES`, and none while the node's own chain contains one.
- **Stop as soon as the engine fetches on its own again**: its blocks keep arriving, that is, one of the next 100 heights is in flight at some peer outside the batch this help has out, and the engine connected a block of its own within the last 30 seconds (Decision 2).
- The whole help sits behind **one constant**, `catchup_assist::ENABLED`.
- The walk is cheap when repeated: headers read once are kept; at most **1,000 headers read per tick** (`WALK_PER_TICK`), and about **1.5 seconds of asking the node per tick** (`TICK_BUDGET`, Decision 21).
- User-facing copy: friendly, simple, no hype, no guarantees, **no em-dashes**.
- CI enforces, in both `crates/btx-core` and `apps/node/src-tauri`: `cargo fmt --all --check` and `cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious`, plus `cargo test --locked`; and `npx tsc --noEmit` and `npm test` in `apps/node`.
- Every commit message ends with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## The evidence this plan relies on

- **Mainnet, 2026-09-29, owner's Mac, engine v0.34.9.** A mirror started from the signed snapshot at 225,927 got 225,928 after 2.5 minutes and 225,929 after 2.3 more. Asking 109.199.124.187 by name, 100 at a time and waiting for each batch to connect, moved it 7,490 blocks in 8.0 minutes (about 940 a minute) to the tip, after which it followed new blocks unassisted. Script: `scratchpad/mainnet/assist.py`; log: `scratchpad/mainnet/evidence/assist.log` (session scratchpad of 2026-09-29).
- **Engine source, v0.34.9 = commit `84b998b4`** (`~/repos/btx`, `git show 84b998b4:<path>`), each rule compared with v0.34.12 (`5f32c4c4`) and unchanged there:
  - `src/net_processing.cpp:6989-6994` (v0.34.12 `:7414-7419`): the block scheduler skips a limited peer for any block 286 or more below that peer's best known block. "Limited" is `IsLimitedPeer` (`:3189-3193`): NETWORK_LIMITED **and not** NODE_NETWORK. This is why nothing is requested while every peer is limited, and why the engine fetches old blocks from a NODE_NETWORK peer on its own.
  - `src/net_processing.cpp:290` and `:3643`: the engine's own rescue re-requests the tip's next block only after it has been the stuck root for 120 seconds.
  - `src/net_processing.cpp:8458` (`FetchBlock`): `getblockfrompeer` marks the block in flight at the peer asked, so the help's own requests show up in `getpeerinfo.inflight`. Confirmed on regtest the same day. A peer that is gone answers "Peer does not exist" (`:8468`).
  - `src/rpc/blockchain.cpp:742`: a block the node already has answers `RPC_MISC_ERROR` "Block already downloaded". The help counts that as taken.
  - `src/net_processing.cpp:9384-9391` (v0.34.12 `:9922-9929`, identical): a serving peer disconnects a requester **without** the NoBan permission when the requested block is more than `NODE_NETWORK_LIMITED_MIN_BLOCKS + 2` (290; the constant is 288 at `:512`) below the server's tip, **but only if the server's own services are NETWORK_LIMITED and not NODE_NETWORK**. A NODE_NETWORK peer serves old blocks to anyone.
  - `src/net_processing.cpp:9373-9381` (v0.34.12 `:9911-9919`): the one rule that stops a NODE_NETWORK peer too. A server that has reached its outbound target (`OutboundTargetReached`, `-maxuploadtarget`) disconnects a requester without the Download permission for a block more than a week older than its best header (`HISTORICAL_BLOCK_AGE`, `:124`).
- **109.199.124.187 on 29 September.** It advertises only NETWORK_LIMITED (`crates/btx-core/src/node.rs:171`, prune=5000) and still served 7,490 blocks deeper than 290 to the owner's Mac, so it grants NoBan to at least that IP. Whether it does for everyone is unknown; a second-IP test is pending with the owner (Task 11 Step 4).
- **Regtest dry run, 2026-09-29, this plan's unamended code** (two v0.34.9 nodes, Task 6's first test): with the help ticking, node B went from 1 to 99 in under a minute; without it, B stayed at 1. In both runs the peer connection dropped after 11 and 16 of the first 100 requests, although A granted `noban`; the help paused, then asked again and finished. Nothing tonight explains that drop; it is why a peer is marked only on its second drop (Decision 14).
- **Measured cost:** `getblockheader` over local RPC took 0.155 ms per header on regtest (421 headers in 65 ms), so a 1,000-header tick costs well under a second.
- **Mainnet acceptance, 30 September** (owner's Mac, this branch at `9fbac9c`, a fresh keeper mirror from the pinned pair at 225,927, headers at 234,039, signed frontier 234,038, 8,107 behind). From 01:52 to 02:07 UTC the help wrote no line and the node gained one block about every 2 minutes. Every `getpeerinfo` sample showed the engine's next three blocks in flight at the app's three discovery relays (manual, `node::BTX_DISCOVERY_PEERS`: WITNESS, SHIELDED, P2P_V2, MATMUL_DISCOVERY, no NETWORK bit), which never deliver them; the engine never disconnects a manual peer for slow delivery. 109.199.124.187:19335 was connected (manual, NETWORK_LIMITED, `synced_headers` 234,039, nothing in flight):

  ```
  02:05:30 tip=225932 node.btx.dev:[225933, 225934, 225935] node.btxchain.org:[225933, 225934, 225935, 1] node.btx.tools:[225935, 225933, 225934]
  02:06:30 tip=225933 node.btx.dev:[225934, 225935, 225936] node.btxchain.org:[225934, 225935, 225936, 1] node.btx.tools:[225934, 225935, 225936]
  ```

  So the old reading of Decision 2 said "the engine is fetching" on every tick, and the quiet wait never completed. At 02:11:24 the controller removed the three relays from that node by hand (`addnode <relay> remove` and `disconnectnode`); the help started on its own at 02:12:02 ("asked 109.199.124.187:19335 for blocks 225936 to 226035 (100 sent, 0 already here)"), then a new batch about every 7 seconds, and the tip went from 225,934 to 226,412 in about a minute. The amended reading starts the help without anyone removing the relays; `the_mainnet_stall_of_30_september_asks_the_archive_peer_past_the_relays_hung_requests` replays that shape.

## Before you start

Everything this plan builds on is on `origin/main`: `stuck_blocks.rs`, `PeerInfo::synced_headers/synced_blocks`, `diagnostics.rs` and `tools.rs` came with Tools (#154, squash-merged as `aed8755`), and the refresher's `signed_frontier` slot with #160 (`b330e3d`, the head this plan was amended against). Branch from it. The plan's own branch, `claude/catch-up-help`, sits on the older base `b008f98`, so the code goes on a new one:

```bash
cd /Users/m2promende/repos/easynode
git fetch origin
git log --oneline -1 origin/main
git worktree add -b claude/catch-up-help-code .claude/worktrees/catch-up-help origin/main
cd .claude/worktrees/catch-up-help
mkdir -p apps/node/src-tauri/resources/node-pkg
echo "CI placeholder. Not a node package." > apps/node/src-tauri/resources/node-pkg/CI-PLACEHOLDER
git status --short
```

Expected: the log line prints main's head (`b330e3d` or later); `git status --short` prints nothing (the `node-pkg` folder is gitignored; the placeholder only satisfies `tauri.conf.json`'s bundle glob so the shell builds, exactly as `.github/workflows/ci.yml:134-136` does). All paths below are relative to this worktree. Line numbers were verified at `b330e3d`; if main moved, find each spot by the quoted text instead.

## File structure

| File | Change | Responsibility |
|---|---|---|
| `crates/btx-core/src/node_api.rs` | modify | `AttestedTip` also carries `signed_frontier.hash`; `serves_full_history(&PeerInfo)` reads NODE_NETWORK (bit 0) |
| `crates/btx-core/src/role.rs` | modify | its three test literals of `AttestedTip` get `hash: None` |
| `crates/btx-core/src/known_invalid.rs` | modify | `refused_blocks()`: every refused (height, hash) in one list |
| `crates/btx-core/src/header_path.rs` | create | the cached walk from a target header down to the tip; shared by the help and Tools |
| `crates/btx-core/src/catchup_assist.rs` | create | the decision (`Helper`) and peer choice, the target choice, the log and diagnostics lines, and the async `tick` |
| `crates/btx-core/src/lib.rs` | modify | `pub mod catchup_assist;`, `pub mod header_path;` |
| `crates/btx-core/tests/catchup_regtest.rs` | create | two opt-in tests against real engines on regtest |
| `crates/btx-core/src/watchdog.rs` | modify | `BlockFetchGated` names the help and the Tools button |
| `crates/btx-core/src/diagnostics.rs` | modify | the report has a "Catch-up help" section |
| `apps/node/src-tauri/src/tools.rs` | modify | "Fetch a stuck block" walks with `HeaderPath`; Copy diagnostics fills the new section |
| `apps/node/src-tauri/src/state.rs` | modify | `catch_up_help` slot: the help's diagnostics lines for this run |
| `apps/node/src-tauri/src/commands.rs` | modify | the status refresher ticks the help with the `signed_frontier` slot and fills `catch_up_help`; start and stop clear it |
| `apps/node/CHANGELOG.md` | modify | one entry under Unreleased |

No UI file changes: the design names no screen for the help beyond the watchdog sentence, which the validation card already shows (`apps/node/src/validation.ts:85-90`, `status.stall.summary`), and Copy diagnostics, whose text the backend builds.

---

### Task 1: What the help reads: the frontier's hash, the refused blocks, and which peers keep every block

**Files:**
- Modify: `crates/btx-core/src/node_api.rs:531-561` (`AttestedTip`, `get_attested_tip`), `:466` (before `summarize_archive_peers`'s doc, next to `advertises_archive` at `:456`), tests at `:1119`
- Modify: `crates/btx-core/src/role.rs:1472`, `:1559`, `:1628` (the `AttestedTip` literals in its tests, added by #160)
- Modify: `crates/btx-core/src/known_invalid.rs:123` (after `LIFTED_BRANCHES`), tests at `:299`

**Interfaces:**
- Produces: `node_api::AttestedTip { .., pub hash: Option<String> }`, filled from `signed_frontier.hash`. Since #160 the refresher keeps the whole `AttestedTip` in `AppState::signed_frontier`, so the slot carries the hash too.
- Produces: `node_api::NODE_NETWORK_BIT` and `node_api::serves_full_history(&PeerInfo) -> bool`: NODE_NETWORK (service bit 0) from the `services` hex, the exact name `NETWORK` only when the hex is absent, the same rule `advertises_archive` follows for bit 31.
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

    /// Service bits as this crate's tests record them: 207.56.229.99 from the
    /// live getpeerinfo of 2026-08-17 above (NETWORK among its bits), and
    /// btxscan's mirror 20.86.181.203 as the snapshot_serve tests record it
    /// (NETWORK_LIMITED, bit 10, without NETWORK).
    #[test]
    fn full_history_is_read_from_bit_0_with_the_name_as_fallback() {
        let peers: Vec<PeerInfo> = serde_json::from_value(json!([
            {"id": 1, "addr": "207.56.229.99:19335", "services": "0000000080000c09"},
            {"id": 2, "addr": "20.86.181.203:19338", "services": "0000000088000d08"},
            {"id": 3, "servicesnames": ["NETWORK", "WITNESS"]},
            {"id": 4, "servicesnames": ["NETWORK_LIMITED"]}
        ]))
        .unwrap();
        let full: Vec<bool> = peers.iter().map(serves_full_history).collect();
        assert_eq!(full, [true, false, true, false]);
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
Expected: the crate does not compile, with `error[E0609]: no field `hash` on type `AttestedTip``, `error[E0425]: cannot find function `refused_blocks` in this scope` and the same for `serves_full_history`.

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

Then run `grep -rn "AttestedTip {" crates apps/node/src-tauri/src | grep -v "pub struct"`. At `b330e3d` it prints four: the literal in `get_attested_tip` (`node_api.rs:553`) and three in `role.rs`'s tests that #160 added (`:1472`, `:1559`, `:1628`), none with `..Default::default()`. In `crates/btx-core/src/role.rs`, replace:

```rust
            on_active_chain,
        }
    }
```

with:

```rust
            on_active_chain,
            hash: None,
        }
    }
```

then replace:

```rust
                    on_active_chain: None,
                },
```

with:

```rust
                    on_active_chain: None,
                    hash: None,
                },
```

then replace:

```rust
                                on_active_chain: on,
                            });
```

with:

```rust
                                on_active_chain: on,
                                hash: None,
                            });
```

Each of the three is unique in the file at `b330e3d`. Any other literal the grep prints without `..Default::default()` gets `hash: None,` the same way.

In `crates/btx-core/src/node_api.rs`, replace:

```rust
/// The gate rule, verbatim from the incident diagnosis: archive service bit
```

with:

```rust
/// NODE_NETWORK, service bit 0: the peer keeps and serves every block.
pub const NODE_NETWORK_BIT: u64 = 1;

/// Does this peer say it serves every block, not only its last 288? Decided
/// from the service bits as [`advertises_archive`] is, with the exact name
/// only for peer objects without them. The difference matters for old
/// blocks: a peer with NETWORK_LIMITED and without this bit drops a node it
/// does not grant `noban` when asked for a block more than 290 below its tip
/// (engine `src/net_processing.cpp:9384-9391` at 84b998b4); one with it does
/// not. The catch-up help (`crate::catchup_assist`) chooses by it.
pub fn serves_full_history(p: &PeerInfo) -> bool {
    let hex = p.services.trim_start_matches("0x");
    if let Ok(bits) = u64::from_str_radix(hex, 16) {
        return bits & NODE_NETWORK_BIT != 0;
    }
    p.servicesnames.iter().any(|n| n == "NETWORK")
}

/// The gate rule, verbatim from the incident diagnosis: archive service bit
```

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

Run: `cd crates/btx-core && cargo test --locked --lib attested_tip_reads && cargo test --locked --lib refused_blocks_names && cargo test --locked --lib full_history_is_read && cargo test --locked --lib role:: 2>&1 | grep "test result"`
Expected: each of the first three prints `test result: ok. 1 passed`; the role tests still pass with 0 failed (their count is derived, not run: note yours).

- [ ] **Step 5: Format and commit**

```bash
cd crates/btx-core && cargo fmt --all && cd ../..
git add crates/btx-core/src/node_api.rs crates/btx-core/src/role.rs crates/btx-core/src/known_invalid.rs
git commit -F - <<'EOF'
core: the signed frontier's hash, every refused block in one list, and which peers keep every block

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

How it works: `hashes` maps height to hash on the followed chain. `down` is the lowest known block whose header is not read yet; reading it yields its parent one height lower, until the tip's height is reached. A new target that is not on the walked chain starts `up`, a second cursor that reads the new branch downward until it meets a hash already walked, so a higher target costs only the new headers and a new branch costs only the headers above the fork. The tip moving up costs nothing; a tip that falls below everything walked (a rollback) resumes the walk.

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
- Modify: `apps/node/src-tauri/src/tools.rs:15` (imports) and `:364-400` (`missing_blocks`)

**Interfaces:**
- Consumes: `HeaderPath`, `PathStatus` from Task 2.
- Produces: `missing_blocks` keeps its signature and its three sentences; `tools_fetch_stuck_blocks` is unchanged, and so is its peer choice (`stuck_blocks::plan`).

This is the Tools quick action the changelog calls "ask good peers for a block your node is stuck on" (`index.html:961`, button `tools-fetch`, "Fetch a stuck block"). It is on main since `aed8755`; the help shares its walk and its target choice (`stuck_blocks::target_tip`) and adds no second fetch path.

No new test: the walk's behaviour is covered by Task 2's tests, and the button's sentences are unchanged. The shell's suite must stay green.

- [ ] **Step 1: Run the shell's suite first, as the baseline**

Run: `cd apps/node/src-tauri && cargo test --locked 2>&1 | grep "test result" | head -1`
Expected: `test result: ok. N passed; 0 failed; M ignored`. The count at `b330e3d` is derived, not run (the plan's first draft saw 96 passed and 1 ignored at the Tools branch's `62a03f1`, before #160); note yours.

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
- Consumes: `stuck_blocks::target_tip` (existing), `known_invalid::refused_blocks` and `node_api::serves_full_history` (Task 1), `HeaderPath::hash_at` (Task 2), `node_api::PeerInfo` (`id`, `addr`, `connection_type`, `services`, `synced_headers`, `inflight`).
- Produces: constants `ENABLED`, `MIN_BEHIND`, `QUIET`, `BATCH`, `ROTATE_AFTER`, `WALK_PER_TICK`, `LIMITED_SERVES`, `DROPS_TO_MARK`; `enum Why { Off, NotBehind, EngineFetching, Quiet, NoPath, NoPeer, NoOldBlocks, Refused, BatchOut }`; `enum Decision { Wait(Why), Ask { peer_id: i64, addr: String, deep: bool, blocks: Vec<(u64, String)> } }`; `struct Seen<'a> { now, tip, target: Option<u64>, next: &'a [(u64, String)], peers: &'a [PeerInfo], refused: Option<&'static str> }`; `Helper::new(Vec<String>)`, `Helper::helping()`, `Helper::decide(&mut self, &Seen) -> Decision`, `Helper::sent(&mut self, &Decision, Instant, accepted: usize, already_have: usize)`, `Helper::refuses_old() -> &[String]`, `Helper::dropped(&str) -> u32`, `Helper::take_news() -> Vec<String>`, `Helper::no_archive_serves_old_blocks() -> bool`; `old_for(&PeerInfo, u64) -> bool`; `choose_target(Option<(u64, String)>, &[ChainTip], u64) -> Option<(u64, String)>`; `refused_on_path(&HeaderPath) -> Option<&'static str>`; `note(bool, &Decision) -> Option<String>`; `dropped_once_line(&str) -> String`; `refuses_old_line(&str) -> String`; constants `DROPS_TO_MARK`, `NO_ARCHIVE_SERVES_OLD_BLOCKS`.

The rules, in the order `decide` applies them:
1. Switched off: nothing.
2. The followed chain is fewer than 20 above the tip: nothing, and forget any batch and quiet clock.
3. A refused block on the way: nothing, likewise.
4. Some peer has one of the next 100 heights in flight that is not in this help's batch: the engine is fetching; nothing, likewise.
5. A batch is out: if the tip reached its top, ask for the next batch at once from the same peer; if its peer is gone or three minutes passed, ask the next archive peer; else wait. When the peer is gone (its connection id is no longer in `getpeerinfo`, which is also what a reconnect under a new id looks like), the batch was of old blocks for it (`deep`), and none of the batch connected, the peer dropped us over old blocks. The help counts that per peer for this run (`dropped`). The first time, the peer becomes the one to rotate away from and the log says so; the second time, it goes on the `refuses_old` list for this run and the log says that. Any peer counts, full-history or limited.
6. No batch: the quiet clock runs from the moment the current tip was first seen with nothing in flight; after 30 seconds, ask.

Asking picks among manual peers whose address is in the app's list and which announced a header above the tip. A block is old for a peer (`old_for`) when it is more than 288 below the last header that peer announced (`synced_headers`, the height the engine's own scheduler measures from). A peer on `refuses_old` is left out while the next block is old for it, and kept for blocks within 288 of its height. The rest are ranked: when the next block is old for a peer, one advertising NODE_NETWORK before one without it; then the peer whose last batch connected; then list order. The pick starts after the peer to rotate away from, so a limited peer's turn for old blocks comes only after every full-history peer has had one. It asks for up to 100 blocks from `tip + 1`, never above the height the peer announced. A peer that takes none of them is rotated away from, after another 30-second wait. A peer that dropped us once is rotated away from the same way: the others are asked first, and it is asked again when its turn comes or when it is the only one left. When the only peers left out are ones on `refuses_old`, the help waits with `Why::NoOldBlocks` and the node goes on with the engine's own 120-second rescue. That is the help's conclusion that no archive peer will serve old blocks to this node (the owner's decision 1, "Both"): `Helper::no_archive_serves_old_blocks()` turns true, the log says `NO_ARCHIVE_SERVES_OLD_BLOCKS` once, and it stays true while the engine's rescue has the next block in flight (which would otherwise flip it every two minutes). It turns false the tick a peer the help may ask is connected (a new archive peer, or a marked one the next block is no longer old for), when a batch connects, and when the help stops (off, within 20 blocks, or a refused chain). Tasks 7 to 9 show it on the status card and in Copy diagnostics, and the Fast-forward plan reads it to offer Fast-forward below its 1,000-block line. A marked full-history peer counts as not serving: it dropped us twice like any other.

In practice the help meets a full-history peer rarely: the engine's scheduler skips only limited peers (`IsLimitedPeer`, engine `net_processing.cpp:3189-3193`), so while a NODE_NETWORK peer with the headers is connected the engine normally fetches from it itself and the help stays idle (rule 4). The preference matters when the engine is not asking that peer, for example while it has paused requests to it after a download timeout (`m_block_download_paused_until`, `:4939`).

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
//! A limited peer (NETWORK_LIMITED without NODE_NETWORK) drops a node it does
//! not grant `noban` when asked for a block more than 290 below its tip
//! (engine `src/net_processing.cpp:9384-9391` at 84b998b4, the same in
//! v0.34.12). So old blocks go to full-history peers first, and a peer that
//! drops us over them twice is not asked for them again this run.
//!
//! [`Helper`] decides from what one refresher tick saw, and is pure. [`tick`]
//! reads what the decision needs from the node, sends the requests and reports
//! back. It never adds, bans, disconnects or reconnects a peer:
//! `getblockfrompeer` is the only command it sends that changes anything.

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
    /// Service bits as this crate's tests record them. LIMITED is btxscan's
    /// mirror (B) in the snapshot_serve tests: NETWORK_LIMITED without
    /// NETWORK, which is also what 109.199.124.187 (A) advertises
    /// (node.rs:171). FULL is role.rs's CONSENSUS_ARCHIVE_BITS, NETWORK among
    /// them, the names node.rs records for 37.230.134.222 (C).
    const LIMITED: &str = "0000000088000d08";
    const FULL: &str = "0000000088000c09";
    /// A manual peer as `getpeerinfo` answers for one, decoded the way the
    /// refresher decodes it.
    fn recorded(id: i64, addr: &str, services: &str, synced_headers: i64) -> PeerInfo {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "addr": addr,
            "connection_type": "manual",
            "services": services,
            "synced_headers": synced_headers,
            "synced_blocks": 225_927,
            "inflight": []
        }))
        .unwrap()
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

    #[test]
    fn a_full_history_peer_is_asked_for_old_blocks_before_a_limited_one() {
        let t0 = Instant::now();
        let next = next_from(225_927);
        // A comes first in the list, but keeps only its last 288 blocks.
        let both = [
            recorded(4, A, LIMITED, 233_481),
            recorded(6, C, FULL, 233_481),
        ];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &both));
        let d = h.decide(&seen(t0 + QUIET, 225_927, &next, &both));
        assert_eq!(asked_of(&d), (6, 225_928, 226_027));
        assert!(matches!(d, Decision::Ask { deep: true, .. }));
        // With no full-history peer connected, the limited one is the last
        // resort, and it is asked.
        let only_a = [recorded(4, A, LIMITED, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &only_a));
        let d = h.decide(&seen(t0 + QUIET, 225_927, &next, &only_a));
        assert_eq!(asked_of(&d).0, 4);
    }

    #[test]
    fn one_drop_then_a_batch_that_connects_leaves_the_peer_unmarked() {
        let t0 = Instant::now();
        let next = next_from(225_927);
        let both = [
            recorded(4, A, LIMITED, 233_481),
            recorded(9, B, LIMITED, 233_481),
        ];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &both));
        let d = step(&mut h, &seen(t0 + QUIET, 225_927, &next, &both));
        assert_eq!(asked_of(&d).0, 4, "list order: A first");
        // Three seconds on, A is back under a new connection id and no block
        // of the batch has connected: it dropped us once. The others are
        // asked first.
        let back = [
            recorded(5, A, LIMITED, 233_481),
            recorded(9, B, LIMITED, 233_481),
        ];
        let d = step(&mut h, &seen(t0 + secs(33), 225_927, &next, &back));
        assert_eq!(asked_of(&d).0, 9);
        assert_eq!(h.dropped(A), 1);
        assert!(h.refuses_old().is_empty());
        assert_eq!(h.take_news(), [dropped_once_line(A)]);
        assert!(h.take_news().is_empty(), "said once");
        // B's batch connects and B goes; A may be asked again, and is.
        let only_a = [recorded(5, A, LIMITED, 233_481)];
        let d = h.decide(&seen(t0 + secs(40), 226_027, &next_from(226_027), &only_a));
        assert_eq!(asked_of(&d), (5, 226_028, 226_127));
        assert!(h.refuses_old().is_empty());
    }

    #[test]
    fn a_limited_peer_that_dropped_us_twice_is_not_asked_for_old_blocks_again() {
        let t0 = Instant::now();
        let next = next_from(225_927);
        let a4 = [recorded(4, A, LIMITED, 233_481)];
        let a5 = [recorded(5, A, LIMITED, 233_481)];
        let a6 = [recorded(6, A, LIMITED, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &a4));
        step(&mut h, &seen(t0 + QUIET, 225_927, &next, &a4));
        // The first drop. A is the only archive peer, so it is asked again.
        let d = step(&mut h, &seen(t0 + secs(33), 225_927, &next, &a5));
        assert_eq!(asked_of(&d).0, 5);
        assert_eq!(h.dropped(A), 1);
        assert!(h.refuses_old().is_empty());
        assert_eq!(h.take_news(), [dropped_once_line(A)]);
        // The second drop marks it.
        assert_eq!(
            h.decide(&seen(t0 + secs(36), 225_927, &next, &a6)),
            Decision::Wait(Why::NoOldBlocks)
        );
        assert_eq!(h.dropped(A), 2);
        assert_eq!(h.refuses_old(), [A.to_string()]);
        assert_eq!(
            h.take_news(),
            [
                refuses_old_line(A),
                NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string()
            ]
        );
        assert!(h.take_news().is_empty(), "said once");
        // For the rest of the run it is not asked for them, even when it is
        // the only archive peer left.
        for n in 1..=20 {
            assert_eq!(
                h.decide(&seen(t0 + secs(36 + 30 * n), 225_927, &next, &a6)),
                Decision::Wait(Why::NoOldBlocks)
            );
        }
        // Another archive peer is asked instead.
        let with_b = [
            recorded(6, A, LIMITED, 233_481),
            recorded(9, B, LIMITED, 233_481),
        ];
        let d = h.decide(&seen(t0 + secs(700), 225_927, &next, &with_b));
        assert_eq!(asked_of(&d).0, 9);
    }

    /// A alone, dropping us twice over old blocks: the conclusion that no
    /// archive peer serves them.
    fn concluded(t0: Instant) -> Helper {
        let next = next_from(225_927);
        let a4 = [recorded(4, A, LIMITED, 233_481)];
        let a5 = [recorded(5, A, LIMITED, 233_481)];
        let a6 = [recorded(6, A, LIMITED, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &a4));
        step(&mut h, &seen(t0 + QUIET, 225_927, &next, &a4));
        step(&mut h, &seen(t0 + secs(33), 225_927, &next, &a5));
        assert!(!h.no_archive_serves_old_blocks(), "one drop is no conclusion");
        assert_eq!(h.take_news(), [dropped_once_line(A)]);
        h.decide(&seen(t0 + secs(36), 225_927, &next, &a6));
        h
    }

    #[test]
    fn no_archive_serves_old_blocks_until_a_new_archive_peer_appears() {
        let t0 = Instant::now();
        let next = next_from(225_927);
        let mut h = concluded(t0);
        assert!(h.no_archive_serves_old_blocks());
        assert_eq!(
            h.take_news(),
            [
                refuses_old_line(A),
                NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string()
            ]
        );
        // It holds while the engine's own rescue has the next block in flight,
        // which holds the help off, and it is said once.
        let rescue = [with_inflight(recorded(6, A, LIMITED, 233_481), &[225_928])];
        assert_eq!(
            h.decide(&seen(t0 + secs(200), 225_927, &next, &rescue)),
            Decision::Wait(Why::EngineFetching)
        );
        assert!(h.no_archive_serves_old_blocks());
        let a6 = [recorded(6, A, LIMITED, 233_481)];
        for n in 1..=10 {
            h.decide(&seen(t0 + secs(200 + 30 * n), 225_927, &next, &a6));
        }
        assert!(h.no_archive_serves_old_blocks());
        assert!(h.take_news().is_empty(), "said once");
        // A new archive peer appears: false at once, and it is asked after the
        // quiet wait the rescue restarted.
        let with_b = [
            recorded(6, A, LIMITED, 233_481),
            recorded(9, B, LIMITED, 233_481),
        ];
        h.decide(&seen(t0 + secs(600), 225_927, &next, &rescue));
        let d = h.decide(&seen(t0 + secs(603), 225_927, &next, &with_b));
        assert_eq!(d, Decision::Wait(Why::Quiet));
        assert!(!h.no_archive_serves_old_blocks());
        let d = h.decide(&seen(t0 + secs(633), 225_927, &next, &with_b));
        assert_eq!(asked_of(&d).0, 9);
    }

    #[test]
    fn no_archive_serves_old_blocks_ends_when_a_batch_connects_again() {
        let t0 = Instant::now();
        let mut h = concluded(t0);
        assert!(h.no_archive_serves_old_blocks());
        // The engine's rescue brings the tip within 288 of A's newest header.
        // A serves those, so the conclusion ends and A is asked.
        let a6 = [recorded(6, A, LIMITED, 233_481)];
        let near = next_from(233_231);
        let mut s = seen(t0 + secs(60), 233_231, &near, &a6);
        s.target = Some(233_481);
        assert_eq!(h.decide(&s), Decision::Wait(Why::Quiet));
        assert!(!h.no_archive_serves_old_blocks());
        s.now = t0 + secs(90);
        assert_eq!(asked_of(&step(&mut h, &s)), (6, 233_232, 233_331));
        // The batch connects: still false, and the next batch goes out at once.
        let after = next_from(233_331);
        let mut s = seen(t0 + secs(95), 233_331, &after, &a6);
        s.target = Some(233_481);
        assert_eq!(asked_of(&h.decide(&s)), (6, 233_332, 233_431));
        assert!(!h.no_archive_serves_old_blocks());
        // And near the chain it follows, the help stops and the conclusion
        // with it, whatever came before.
        let mut h = concluded(t0);
        let mut s = seen(t0 + secs(60), 225_927, &next_from(225_927), &a6);
        s.target = Some(225_927 + 10);
        assert_eq!(h.decide(&s), Decision::Wait(Why::NotBehind));
        assert!(!h.no_archive_serves_old_blocks());
    }

    #[test]
    fn a_full_history_peer_that_drops_us_twice_is_marked_the_same_way() {
        // At its outbound target a NODE_NETWORK peer drops a requester over
        // old blocks too (engine net_processing.cpp:9373-9381).
        let t0 = Instant::now();
        let next = next_from(225_927);
        let c6 = [recorded(6, C, FULL, 233_481)];
        let c7 = [recorded(7, C, FULL, 233_481)];
        let c8 = [recorded(8, C, FULL, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next, &c6));
        step(&mut h, &seen(t0 + QUIET, 225_927, &next, &c6));
        let d = step(&mut h, &seen(t0 + secs(33), 225_927, &next, &c7));
        assert_eq!(asked_of(&d).0, 7, "one drop: asked again");
        assert!(h.refuses_old().is_empty());
        assert_eq!(
            h.decide(&seen(t0 + secs(36), 225_927, &next, &c8)),
            Decision::Wait(Why::NoOldBlocks)
        );
        assert_eq!(h.refuses_old(), [C.to_string()]);
    }

    #[test]
    fn a_limited_peer_is_still_asked_for_blocks_within_288_of_its_height() {
        let a = recorded(4, A, LIMITED, 233_481);
        assert!(!old_for(&a, 233_193), "288 below: a limited peer serves it");
        assert!(old_for(&a, 233_192), "289 below: old");
        let t0 = Instant::now();
        // 250 behind A's newest header, as after a snapshot load.
        let near = next_from(233_231);
        let mut h = helper();
        let mut s = seen(t0, 233_231, &near, std::slice::from_ref(&a));
        s.target = Some(233_481);
        h.decide(&s);
        s.now = t0 + QUIET;
        let d = h.decide(&s);
        assert_eq!(asked_of(&d), (4, 233_232, 233_331));
        assert!(matches!(d, Decision::Ask { deep: false, .. }));
        // The same holds for a peer marked earlier in the run, after two
        // drops over old blocks.
        let far = next_from(225_927);
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &far, std::slice::from_ref(&a)));
        step(
            &mut h,
            &seen(t0 + QUIET, 225_927, &far, std::slice::from_ref(&a)),
        );
        let a5 = [recorded(5, A, LIMITED, 233_481)];
        step(&mut h, &seen(t0 + secs(33), 225_927, &far, &a5));
        let back = [recorded(6, A, LIMITED, 233_481)];
        h.decide(&seen(t0 + secs(36), 225_927, &far, &back));
        assert_eq!(h.refuses_old(), [A.to_string()]);
        let mut s = seen(t0 + secs(60), 233_231, &near, &back);
        s.target = Some(233_481);
        h.decide(&s);
        s.now = t0 + secs(90);
        assert_eq!(asked_of(&h.decide(&s)), (6, 233_232, 233_331));
    }

    #[test]
    fn a_drop_after_part_of_the_batch_connected_only_rotates() {
        let t0 = Instant::now();
        let a = [recorded(4, A, LIMITED, 233_481)];
        let mut h = helper();
        h.decide(&seen(t0, 225_927, &next_from(225_927), &a));
        step(&mut h, &seen(t0 + QUIET, 225_927, &next_from(225_927), &a));
        // 60 of the 100 connected before the connection went: A served old
        // blocks, so this is not the limited peers' rule, and not a drop.
        let back = [recorded(5, A, LIMITED, 233_481)];
        let d = h.decide(&seen(t0 + secs(40), 225_987, &next_from(225_987), &back));
        assert_eq!(asked_of(&d), (5, 225_988, 226_087));
        assert_eq!(h.dropped(A), 0);
        assert!(h.refuses_old().is_empty());
    }

    #[test]
    fn two_hundred_behind_after_a_snapshot_load_the_help_stays_idle() {
        // Section 11: a load from a confirmed snapshot (grid 100, 144 deep)
        // leaves the node 144 to about 250 behind, inside what limited peers
        // serve, so the engine asks for the next blocks itself.
        let t0 = Instant::now();
        let tip = 233_281;
        let next = next_from(tip);
        let mut engine_peer = recorded(12, "1.2.3.4:19335", LIMITED, 233_481);
        engine_peer.connection_type = "outbound-full-relay".into();
        engine_peer.inflight = (tip as i64 + 1..=tip as i64 + 16).collect();
        let peers = [recorded(4, A, LIMITED, 233_481), engine_peer];
        let mut h = helper();
        for n in 0..40 {
            let mut s = seen(t0 + secs(3 * n), tip, &next, &peers);
            s.target = Some(tip + 200);
            assert_eq!(h.decide(&s), Decision::Wait(Why::EngineFetching));
        }
        assert!(!h.helping());
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
        assert_eq!(note(true, &Decision::Wait(Why::NoOldBlocks)), None);
        assert!(!NO_ARCHIVE_SERVES_OLD_BLOCKS.contains('\u{2014}'));
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
        let line = refuses_old_line(A);
        assert!(line.starts_with("109.199.124.187:19335 does not serve old blocks to us"));
        assert!(!line.contains('\u{2014}'));
        let once = dropped_once_line(A);
        assert!(once.starts_with("109.199.124.187:19335 dropped the connection once"));
        assert!(!once.contains('\u{2014}'));
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
use crate::node_api::{serves_full_history, PeerInfo};


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
/// A NETWORK_LIMITED peer keeps its last 288 blocks
/// (`NODE_NETWORK_LIMITED_MIN_BLOCKS`, engine `src/net_processing.cpp:512` at
/// 84b998b4). A block further below the height a peer announced is old for it
/// ([`old_for`]).
pub const LIMITED_SERVES: u64 = 288;
/// Drops over old blocks, this run, before a peer is not asked for them
/// again (the controller's decision of 2026-09-29, night: two strikes, so one
/// unexplained drop like the regtest dry run's does not lose a peer).
pub const DROPS_TO_MARK: u32 = 2;
/// The one plain line for the log, and for Copy diagnostics, while
/// [`Helper::no_archive_serves_old_blocks`] holds (the owner's decision 1,
/// the night of 2026-09-29).
pub const NO_ARCHIVE_SERVES_OLD_BLOCKS: &str =
    "stopped asking for old blocks: none of the archive peers connected now serves them to \
     this node";

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
    /// The archive peers connected with the next block have each dropped us
    /// over old blocks [`DROPS_TO_MARK`] times this run
    /// ([`Helper::refuses_old`]), and the next block is old for each of them.
    NoOldBlocks,
    /// The followed chain passes a block this app refuses.
    Refused,
    /// A batch is out and has not connected yet.
    BatchOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Wait(Why),
    /// Ask this peer for these blocks, lowest first. `deep`: the first of
    /// them is old for this peer ([`old_for`]).
    Ask {
        peer_id: i64,
        addr: String,
        deep: bool,
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
    /// The batch's first block was old for its peer.
    deep: bool,
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
    /// How often each peer dropped us while asked for old blocks this run, in
    /// the order they first did. Any peer counts, full-history or limited.
    drops: Vec<(String, u32)>,
    /// The peers among them that reached [`DROPS_TO_MARK`]: not asked for
    /// old blocks again this run.
    refuses_old: Vec<String>,
    /// Log lines not yet said: a drop, and the conclusion below.
    news: Vec<String>,
    /// No archive peer will serve old blocks: every archive peer connected with
    /// the next block is on `refuses_old` and the next block is old for each.
    /// Set when the help pauses with [`Why::NoOldBlocks`]; kept while the
    /// engine's own rescue has the next block in flight (that holds the help
    /// off, not the conclusion); cleared when a peer the help may ask is
    /// connected again, a batch connects, or the help stops.
    no_old_blocks: bool,
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
            drops: Vec::new(),
            refuses_old: Vec::new(),
            news: Vec::new(),
            no_old_blocks: false,
        }
    }

    /// A batch is out.
    pub fn helping(&self) -> bool {
        self.batch.is_some()
    }

    /// The peers that dropped us over old blocks [`DROPS_TO_MARK`] times this
    /// run, in the order they got there.
    pub fn refuses_old(&self) -> &[String] {
        &self.refuses_old
    }

    /// How often `addr` dropped us over old blocks this run.
    pub fn dropped(&self, addr: &str) -> u32 {
        self.drops
            .iter()
            .find(|(a, _)| a == addr)
            .map_or(0, |(_, n)| *n)
    }

    /// The log lines since the last call ([`dropped_once_line`],
    /// [`refuses_old_line`], [`NO_ARCHIVE_SERVES_OLD_BLOCKS`]), so each is
    /// said once.
    pub fn take_news(&mut self) -> Vec<String> {
        std::mem::take(&mut self.news)
    }

    /// The help's conclusion that no archive peer will serve old blocks to
    /// this node: every archive peer connected with the next block dropped us
    /// over old blocks [`DROPS_TO_MARK`] times this run, full-history ones
    /// included, and the next block is old for each. What the status card,
    /// Copy diagnostics and the Fast-forward offer read (through
    /// [`CatchUp::no_archive_serves_old_blocks`]).
    pub fn no_archive_serves_old_blocks(&self) -> bool {
        self.no_old_blocks
    }

    pub fn decide(&mut self, s: &Seen) -> Decision {
        if !self.enabled {
            return self.stop(Why::Off);
        }
        if s.target.map_or(0, |t| t.saturating_sub(s.tip)) < MIN_BEHIND {
            return self.stop(Why::NotBehind);
        }
        if s.refused.is_some() {
            return self.stop(Why::Refused);
        }
        if self.no_old_blocks && self.pick(s.peers, s.tip).is_ok() {
            // A peer the help may ask is connected: a new archive peer, or a
            // marked one the next block is no longer old for. Checked before
            // the engine's rescue can hold the help off, so the conclusion
            // ends the tick it stops being true.
            self.no_old_blocks = false;
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
                self.no_old_blocks = false;
            } else if peer_here && s.now.duration_since(b.asked_at) < ROTATE_AFTER {
                self.batch = Some(b);
                return Decision::Wait(Why::BatchOut);
            } else {
                if !peer_here && b.deep && s.tip < b.from {
                    // Gone, or back under a new connection id, before any of
                    // a batch of old blocks connected: what a limited peer
                    // does to a node it does not grant `noban` (engine
                    // net_processing.cpp:9384-9391), and a full-history one
                    // at its outbound target (:9373-9381). It applies the
                    // rule to the first old block asked for, so a batch that
                    // partly connected is not this, and only rotates. The
                    // second time, the peer is marked.
                    self.count_drop(&b.addr);
                }
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
            deep,
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
            deep: *deep,
            asked_at: now,
        });
    }

    fn idle(&mut self, why: Why) -> Decision {
        self.batch = None;
        self.quiet_since = None;
        Decision::Wait(why)
    }

    /// [`idle`](Self::idle), and the conclusion ends: the help is off, the
    /// node is near the chain it follows, or that chain is refused.
    fn stop(&mut self, why: Why) -> Decision {
        self.no_old_blocks = false;
        self.idle(why)
    }

    fn count_drop(&mut self, addr: &str) {
        let n = match self.drops.iter_mut().find(|(a, _)| a == addr) {
            Some((_, n)) => {
                *n += 1;
                *n
            }
            None => {
                self.drops.push((addr.to_string(), 1));
                1
            }
        };
        if n < DROPS_TO_MARK {
            self.news.push(dropped_once_line(addr));
        } else if !self.refuses_old.iter().any(|a| a == addr) {
            self.refuses_old.push(addr.to_string());
            self.news.push(refuses_old_line(addr));
        }
    }

    fn ask(&mut self, s: &Seen) -> Decision {
        if s.next.first().map(|(h, _)| *h) != Some(s.tip + 1) {
            return Decision::Wait(Why::NoPath);
        }
        let peer = match self.pick(s.peers, s.tip) {
            Ok(p) => {
                self.no_old_blocks = false;
                p
            }
            Err(Why::NoOldBlocks) => {
                if !self.no_old_blocks {
                    self.no_old_blocks = true;
                    self.news.push(NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string());
                }
                return Decision::Wait(Why::NoOldBlocks);
            }
            Err(why) => return Decision::Wait(why),
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
            deep: old_for(peer, s.tip + 1),
            blocks,
        }
    }

    /// The app's own archive peers that announced the next block, best
    /// first: when the next block is old for a peer, one that keeps every
    /// block before one that does not; then the one that delivered last;
    /// then list order; starting after the slow one, which is also where a
    /// peer that just dropped us once goes. A peer that dropped us over old
    /// blocks [`DROPS_TO_MARK`] times is left out while the next block is old
    /// for it. `Err` says why nobody is left.
    fn pick<'p>(&self, peers: &'p [PeerInfo], tip: u64) -> Result<&'p PeerInfo, Why> {
        let first = tip + 1;
        let ours: Vec<&PeerInfo> = peers
            .iter()
            .filter(|p| {
                p.connection_type == "manual"
                    && self.archive.contains(&p.addr)
                    && p.synced_headers > tip as i64
            })
            .collect();
        let mut ranked: Vec<&PeerInfo> = ours
            .iter()
            .copied()
            .filter(|p| !(old_for(p, first) && self.refuses_old.contains(&p.addr)))
            .collect();
        if ranked.is_empty() {
            return Err(if ours.is_empty() {
                Why::NoPeer
            } else {
                Why::NoOldBlocks
            });
        }
        ranked.sort_by_key(|p| {
            (
                old_for(p, first) && !serves_full_history(p),
                self.last_good.as_ref() != Some(&p.addr),
                self.archive.iter().position(|a| *a == p.addr),
            )
        });
        let start = self
            .slow
            .as_ref()
            .and_then(|slow| ranked.iter().position(|p| p.addr == *slow))
            .map_or(0, |i| (i + 1) % ranked.len());
        Ok(ranked[start])
    }
}

/// Whether `height` is old for this peer: more than [`LIMITED_SERVES`] below
/// the last header it announced (`synced_headers`, which is the peer's best
/// known block, the height the engine's own scheduler measures from at
/// net_processing.cpp:6989-6994).
pub fn old_for(p: &PeerInfo, height: u64) -> bool {
    p.synced_headers.saturating_sub(height as i64) > LIMITED_SERVES as i64
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
        // NoOldBlocks is said once by NO_ARCHIVE_SERVES_OLD_BLOCKS, when the
        // conclusion is reached, not on every pause.
        Why::Quiet | Why::BatchOut | Why::NoOldBlocks => return None,
    };
    Some(line)
}

/// The line for the log, and for Copy diagnostics, about a peer that dropped
/// this node once when asked for old blocks.
pub fn dropped_once_line(addr: &str) -> String {
    format!(
        "{addr} dropped the connection once when asked for old blocks, so the other archive \
         peers are asked first; after a second time it is asked only for newer blocks until \
         the node restarts"
    )
}

/// The line for the log, and for Copy diagnostics, about a peer that dropped
/// this node [`DROPS_TO_MARK`] times when asked for old blocks.
pub fn refuses_old_line(addr: &str) -> String {
    format!(
        "{addr} does not serve old blocks to us: it dropped the connection twice when asked \
         for them, so until the node restarts it is asked only for blocks within \
         {LIMITED_SERVES} of its newest"
    )
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib catchup_assist`
Expected: `test result: ok. 25 passed; 0 failed` (the first draft's 16, seven for peer choice and drops, and two for the conclusion that no archive peer serves old blocks; derived, not run).

- [ ] **Step 5: Format, lint and commit**

```bash
cd crates/btx-core && cargo fmt --all && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cd ../..
git add crates/btx-core/src/catchup_assist.rs crates/btx-core/src/lib.rs
git commit -F - <<'EOF'
core: when to ask an archive peer for the next blocks, and whom, full-history peers first for old ones

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 5: One tick against the node

**Files:**
- Modify: `crates/btx-core/src/catchup_assist.rs` (imports, the driver after `refuses_old_line`, tests appended inside `mod tests`)

**Interfaces:**
- Consumes: everything from Task 4; `HeaderPath` and `PathStatus` (Task 2); `node_api::AttestedTip` with `hash` (Task 1), as the refresher's `signed_frontier` slot holds it (#160); `node::block_source_peers()` (existing, `crates/btx-core/src/node.rs:318`).
- Produces: `struct Tick<'a> { blocks: u64, headers: u64, tips: &'a [ChainTip], peers: &'a [PeerInfo], frontier: Option<&'a AttestedTip> }`; `CatchUp::new(Vec<String>)`, `CatchUp::for_this_app()`, `CatchUp::diagnostics() -> Vec<String>`, `CatchUp::no_archive_serves_old_blocks() -> bool`, `CatchUp::report() -> CatchUpReport`; `struct CatchUpReport { pub lines: Vec<String>, pub no_archive_serves_old_blocks: bool }` (`Default`, `Serialize`); `async fn tick(rpc: &dyn Rpc, cu: &mut CatchUp, t: &Tick<'_>, now: Instant) -> Vec<String>`.

What `tick` does: nothing at all on the node while `headers < blocks + 20` or `blocks == 0` (the decision still runs so a stop is logged). Otherwise it reads the tip's hash, takes the signed frontier the refresher already read this tick (`Tick::frontier`, followed only when its header is known), chooses the target, walks at most 1,000 headers, checks the walked chain and the node's own chain for refused blocks, decides, and on `Ask` sends one `getblockfrompeer` per block, counting "Block already downloaded" as taken. It returns the lines for the log: a drop over old blocks (the first, or the second, which marks the peer), then the batch asked for, or the stop or pause. It sends no `getmatmulattestedtip`: since #160 the refresher reads it on every node every tick and keeps the answer in `AppState::signed_frontier`, and the fake node below panics on it.

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
        /// What the refresher's `signed_frontier` slot holds for this node.
        fn attested(&self) -> Option<AttestedTip> {
            self.frontier.map(|f| AttestedTip {
                height: Some(f),
                blocks_behind: Some((f - self.tip) as i64),
                on_active_chain: Some(true),
                hash: Some(self.hash(f)),
            })
        }
    }

    #[async_trait]
    impl Rpc for FakeNode {
        async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), params.clone()));
            // No getmatmulattestedtip: the frontier comes in through the
            // Tick, from the refresher's slot. Asking again panics below.
            match method {
                "getbestblockhash" => Ok(json!(self.hash(self.tip))),
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
            frontier: None,
        }
    }

    #[tokio::test]
    async fn a_tick_asks_the_archive_peer_for_the_next_hundred_by_name() {
        let node = FakeNode::new(300, 100, None);
        let (tips, peers) = ([tip(300, "headers-only")], [peer(7, A, 300)]);
        let t = tick_of(100, 300, &tips, &peers);
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        assert!(tick(&node, &mut cu, &t, t0).await.is_empty());
        assert!(node.asked().is_empty());
        assert_eq!(
            tick(&node, &mut cu, &t, t0 + QUIET).await,
            ["asked 109.199.124.187:19335 for blocks 101 to 200 (100 sent, 0 already here)"]
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
        let slot = node.attested();
        let t = Tick {
            frontier: slot.as_ref(),
            ..tick_of(100, 300, &tips, &peers)
        };
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

    #[tokio::test]
    async fn a_tick_says_each_drop_once_and_marks_a_peer_on_the_second() {
        let node = FakeNode::new(600, 100, None);
        let tips = [tip(600, "headers-only")];
        let first = [recorded(7, A, LIMITED, 600)];
        let mut cu = CatchUp::for_this_app();
        let t0 = Instant::now();
        let asked = "asked 109.199.124.187:19335 for blocks 101 to 200 (100 sent, 0 already here)";
        tick(&node, &mut cu, &tick_of(100, 600, &tips, &first), t0).await;
        assert_eq!(
            tick(&node, &mut cu, &tick_of(100, 600, &tips, &first), t0 + QUIET).await,
            [asked]
        );
        // Back under a new connection id, nothing connected: the first drop.
        // A is the only archive peer, so it is asked again at once.
        let back = [recorded(8, A, LIMITED, 600)];
        let t1 = t0 + QUIET + secs(3);
        assert_eq!(
            tick(&node, &mut cu, &tick_of(100, 600, &tips, &back), t1).await,
            [dropped_once_line(A), asked.to_string()]
        );
        // The second drop: marked.
        let again = [recorded(9, A, LIMITED, 600)];
        let t2 = t1 + secs(3);
        let lines = tick(&node, &mut cu, &tick_of(100, 600, &tips, &again), t2).await;
        assert_eq!(
            lines,
            [
                refuses_old_line(A),
                NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string()
            ]
        );
        let later = tick(&node, &mut cu, &tick_of(100, 600, &tips, &again), t2 + QUIET).await;
        assert!(later.is_empty(), "said once: {later:?}");
        assert_eq!(node.asked().len(), 200, "never asked for old blocks again");
        // What the refresher hands the shell: the lines and the conclusion.
        assert!(cu.no_archive_serves_old_blocks());
        assert_eq!(
            cu.report(),
            CatchUpReport {
                lines: vec![
                    "not asking any peer for blocks right now".to_string(),
                    NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string(),
                    refuses_old_line(A),
                ],
                no_archive_serves_old_blocks: true,
            }
        );
        // A new archive peer: the conclusion ends on the next tick.
        let b = [
            recorded(9, A, LIMITED, 600),
            recorded(11, "20.86.181.203:19338", LIMITED, 600),
        ];
        tick(&node, &mut cu, &tick_of(100, 600, &tips, &b), t2 + QUIET * 2).await;
        assert!(!cu.report().no_archive_serves_old_blocks);
    }

    #[test]
    fn copy_diagnostics_says_what_the_help_is_doing() {
        let mut cu = CatchUp::new(vec![A.into()]);
        assert_eq!(
            cu.diagnostics(),
            ["not asking any peer for blocks right now"]
        );
        let t0 = Instant::now();
        let next = next_from(225_927);
        let a = [recorded(4, A, LIMITED, 233_481)];
        cu.helper.decide(&seen(t0, 225_927, &next, &a));
        step(&mut cu.helper, &seen(t0 + QUIET, 225_927, &next, &a));
        let asking = "asking 109.199.124.187:19335 for blocks 225928 to 226027";
        assert_eq!(cu.diagnostics(), [asking]);
        // One drop, and nobody left for now.
        cu.helper.decide(&seen(t0 + secs(33), 225_927, &next, &[]));
        assert_eq!(
            cu.diagnostics(),
            [
                "not asking any peer for blocks right now".to_string(),
                dropped_once_line(A)
            ]
        );
        // A is back and asked again; then it drops us a second time.
        let a5 = [recorded(5, A, LIMITED, 233_481)];
        step(&mut cu.helper, &seen(t0 + secs(36), 225_927, &next, &a5));
        assert_eq!(
            cu.diagnostics(),
            [asking.to_string(), dropped_once_line(A)]
        );
        cu.helper.decide(&seen(t0 + secs(39), 225_927, &next, &[]));
        let lines = cu.diagnostics();
        assert_eq!(
            lines,
            [
                "not asking any peer for blocks right now".to_string(),
                refuses_old_line(A)
            ]
        );
        assert!(lines.iter().all(|l| !l.contains('\u{2014}')));
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
use crate::node_api::{serves_full_history, PeerInfo};
```

with:

```rust
use std::time::{Duration, Instant};

use serde_json::json;

use crate::error::AppError;
use crate::fork::ChainTip;
use crate::header_path::{HeaderPath, PathStatus};
use crate::node_api::{serves_full_history, AttestedTip, PeerInfo};
use crate::rpc::Rpc;
```

and insert this after `pub fn refuses_old_line(..)` (the last item before `#[cfg(test)]`):

```rust


/// What the refresher already read this tick.
#[derive(Debug, Clone, Copy)]
pub struct Tick<'a> {
    pub blocks: u64,
    pub headers: u64,
    pub tips: &'a [ChainTip],
    pub peers: &'a [PeerInfo],
    /// The refresher's `signed_frontier` slot: its last good
    /// `getmatmulattestedtip` answer this run, read on every node every tick
    /// since 0.7.0 (#160). The help follows it and asks the node for it no
    /// second time.
    pub frontier: Option<&'a AttestedTip>,
}

/// The help's memory for one node run.
#[derive(Debug, Clone)]
pub struct CatchUp {
    helper: Helper,
    path: HeaderPath,
}

/// What the shell keeps of the help between ticks (`AppState::catch_up_help`,
/// written by the refresher every tick, reset on every start and stop): the
/// Copy diagnostics lines, and the conclusion the status card and the
/// Fast-forward offer read.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct CatchUpReport {
    /// [`CatchUp::diagnostics`].
    pub lines: Vec<String>,
    /// [`CatchUp::no_archive_serves_old_blocks`].
    pub no_archive_serves_old_blocks: bool,
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

    /// No archive peer will serve old blocks to this node
    /// ([`Helper::no_archive_serves_old_blocks`]): the owner's decision 1
    /// pauses the help, says so, and lets Fast-forward be offered below its
    /// 1,000-block line.
    pub fn no_archive_serves_old_blocks(&self) -> bool {
        self.helper.no_archive_serves_old_blocks()
    }

    /// What the refresher puts in `AppState::catch_up_help` each tick.
    pub fn report(&self) -> CatchUpReport {
        CatchUpReport {
            lines: self.diagnostics(),
            no_archive_serves_old_blocks: self.no_archive_serves_old_blocks(),
        }
    }

    /// What Copy diagnostics says about the help this run, one line each:
    /// the batch out, if any, the conclusion that no archive peer serves old
    /// blocks while it holds, then every peer that dropped this node when
    /// asked for old blocks, once or [`DROPS_TO_MARK`] times.
    pub fn diagnostics(&self) -> Vec<String> {
        let mut out = vec![match &self.helper.batch {
            Some(b) => format!("asking {} for blocks {} to {}", b.addr, b.from, b.to),
            None => "not asking any peer for blocks right now".to_string(),
        }];
        if self.no_archive_serves_old_blocks() {
            out.push(NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string());
        }
        out.extend(self.helper.drops.iter().map(|(a, n)| {
            if *n >= DROPS_TO_MARK {
                refuses_old_line(a)
            } else {
                dropped_once_line(a)
            }
        }));
        out
    }
}

/// One refresher tick. Returns the lines for the log: a drop over old blocks
/// ([`dropped_once_line`] or [`refuses_old_line`]), then the batch it asked
/// for, or the stop or pause. Reads nothing from the node while no header it
/// knows is [`MIN_BEHIND`] above the tip.
pub async fn tick(rpc: &dyn Rpc, cu: &mut CatchUp, t: &Tick<'_>, now: Instant) -> Vec<String> {
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
        return said(cu, was_helping, &d);
    }
    let Ok(tip_hash) = rpc.call("getbestblockhash", json!([])).await else {
        return Vec::new();
    };
    let Some(tip_hash) = tip_hash.as_str().map(str::to_string) else {
        return Vec::new();
    };
    let frontier = known_frontier(rpc, t.frontier, &cu.path).await;
    let target = choose_target(frontier, t.tips, t.blocks);
    let mut next = Vec::new();
    if let Some((height, hash)) = &target {
        cu.path.retarget(*height, hash);
        if cu.path.walk(rpc, t.blocks, WALK_PER_TICK).await.is_err() {
            return Vec::new();
        }
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
        ..
    } = &d
    else {
        return said(cu, was_helping, &d);
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
    let mut lines = said(cu, false, &d);
    if let (Some(first), Some(last)) = (blocks.first(), blocks.last()) {
        let (first, last) = (first.0, last.0);
        lines.push(if accepted + have == 0 {
            format!(
                "{addr} took none of blocks {first} to {last}; the next archive peer is asked \
                 later"
            )
        } else {
            format!(
                "asked {addr} for blocks {first} to {last} ({accepted} sent, {have} already here)"
            )
        });
    }
    lines
}

/// The log lines one decision earns: a drop over old blocks, once or the
/// one that marks the peer, the conclusion that no archive peer serves them,
/// then the stop or pause [`note`] names.
fn said(cu: &mut CatchUp, was_helping: bool, d: &Decision) -> Vec<String> {
    let mut lines = cu.helper.take_news();
    lines.extend(note(was_helping, d));
    lines
}

/// The signed frontier the refresher read, when it names a hash and the node
/// knows that header.
async fn known_frontier(
    rpc: &dyn Rpc,
    slot: Option<&AttestedTip>,
    path: &HeaderPath,
) -> Option<(u64, String)> {
    let t = slot?;
    let (height, hash) = (t.height?, t.hash.clone()?);
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
Expected: `test result: ok. 32 passed; 0 failed` (Task 4's 25, the first draft's five tick tests, and the two for drops, the report and Copy diagnostics; derived, not run).

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
- Consumes: `catchup_assist::{CatchUp, Tick, tick, dropped_once_line, refuses_old_line}` (Tasks 4 and 5), `node_api::{get_attested_tip, get_blockchain_info, get_chain_tips, get_peer_info}`, `rpc::{Rpc, RpcClient}` (existing; `RpcClient::from_cookie` as in `tests/console_regtest.rs`).

Two tests of the whole path through real engines, one per case the engine's rule makes. Each builds the same stall: A is a limited archive peer with 400 blocks, B has every header and block 1 and only A as a peer, so B's engine asks A for nothing. In the first, A grants loopback `noban`: the help moves B, as in the dry run of 2026-09-29. In the second, A grants nothing: asked for block 2, which is 398 below its tip, A drops B (engine `net_processing.cpp:9384-9391`) every time. The help must say the first drop, ask A again (it is the only archive peer), and on the second drop say that A does not serve old blocks to us, never ask it for them again, and conclude (the owner's decision 1) that no archive peer serves old blocks to B. Both check that the help sends nothing but the allowed commands. They need a v0.34.9 `btxd`; without `EASYNODE_TEST_BTXD` they return early, and `#[ignore]` keeps them out of CI. Each takes about 100 seconds (80 of them mining A's 400 blocks); the first uses RPC ports 29451/29452 and P2P port 29461, the second 29453/29454 and 29463. The harness reads `getmatmulattestedtip` on the help's behalf, the way the refresher fills its `signed_frontier` slot, so it is not on the help's allowed list.

The code below is derived, not run: the first test is the one that passed on 2026-09-29 with the ticks moved into `tick_once` and the pair into `stalled_pair`; the second is new. Two strikes (Task 4) keep the first test's expectations as they were: the dry run's single drop is logged and A is asked again.

- [ ] **Step 1: Write the tests**

Create `crates/btx-core/tests/catchup_regtest.rs`:

```rust
//! The catch-up help against real v0.34.9 engines on regtest. Opt-in:
//!
//! ```text
//! EASYNODE_TEST_BTXD=/path/to/btxd cargo test --test catchup_regtest -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The mainnet stall of 2026-09-29 in miniature. Node A plays the archive
//! peer: pruned (`-prune=1`, so it advertises NETWORK_LIMITED without
//! NODE_NETWORK, like 109.199.124.187) with 400 blocks. Node B holds all 400
//! headers and block 1, and its only peer is A. Blocks 2 to 101 are more than
//! 288 below A's tip, so B's engine asks A for none of them: its tip stays at
//! 1. The help is ticked the way the refresher ticks it.
//!
//! - When A grants the test's loopback address `noban`, which is what lets a
//!   limited node serve a block deeper than 290 when asked for it by name,
//!   the help must ask A for blocks 2 to 101 by name and move B's tip.
//! - When A grants nothing, A drops B on the first of them (engine
//!   `src/net_processing.cpp:9384-9391` at 84b998b4), each time it is asked.
//!   The help must say the first drop and ask A again, then on the second
//!   drop say once that A does not serve old blocks to us, never ask it for
//!   them again, and conclude that no archive peer serves old blocks to B.
//!
//! Either way it must send no command but the ones it is allowed.
//!
//! Heights up to 99 connect without ExactReplay on regtest, so the first test
//! asks for 99, which takes seconds on any machine. On a Mac whose device
//! fails the engine's self-test, A's own block 100 connects only after a
//! restart; the setup does that when it has to (measured 2026-09-29 on the
//! owner's M2 Pro).

use std::ffi::OsStr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use btx_core::catchup_assist::{self, CatchUp, Tick};
use btx_core::error::AppResult;
use btx_core::node_api::{get_attested_tip, get_blockchain_info, get_chain_tips, get_peer_info};
use btx_core::rpc::{Rpc, RpcClient};
use serde_json::{json, Value};

/// Every command the help may send. Anything else fails the test. The signed
/// frontier reaches the help through the Tick, as the refresher's slot, so
/// `getmatmulattestedtip` is not among them.
const ALLOWED: &[&str] = &[
    "getbestblockhash",
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

async fn start(bin: &OsStr, dir: &std::path::Path, rpc_port: u16, args: &[String]) -> Btxd {
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

impl Watched<'_> {
    fn count(&self, method: &str) -> usize {
        self.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == method)
            .count()
    }
}

#[async_trait]
impl Rpc for Watched<'_> {
    async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
        self.methods.lock().unwrap().push(method.to_string());
        self.inner.call(method, params).await
    }
}

#[derive(Clone, Copy)]
struct Ports {
    a_rpc: u16,
    a_p2p: u16,
    b_rpc: u16,
}

/// The two nodes and their folders. Fields drop in order, so the nodes are
/// killed before their folders go.
struct Pair {
    a: Btxd,
    b: Btxd,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

/// The stall: A with 400 blocks, pruned, granting loopback `noban` only when
/// `noban`; B with every header and block 1, whose only peer is A, and whose
/// engine is shown to ask A for nothing.
async fn stalled_pair(bin: &OsStr, ports: Ports, noban: bool) -> Pair {
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut a_args: Vec<String> = [
        "-listen=1",
        "-bind=127.0.0.1",
        "-listenonion=0",
        "-connect=0",
        "-prune=1",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([format!("-port={}", ports.a_p2p)])
    .collect();
    if noban {
        a_args.push("-whitelist=noban@127.0.0.1".into());
    }

    // A: 400 blocks.
    let mut a = start(bin, dir_a.path(), ports.a_rpc, &a_args).await;
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
        a = start(bin, dir_a.path(), ports.a_rpc, &a_args).await;
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
        bin,
        dir_b.path(),
        ports.b_rpc,
        &[
            "-listen=0".to_string(),
            format!("-connect=127.0.0.1:{}", ports.a_p2p),
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
    Pair {
        a,
        b,
        _dirs: (dir_a, dir_b),
    }
}

/// One tick the way the refresher runs it: its own reads, the signed
/// frontier among them as its `signed_frontier` slot would hold it, then the
/// help on the watched client.
async fn tick_once(
    node: &RpcClient,
    watched: &Watched<'_>,
    cu: &mut CatchUp,
    now: Instant,
) -> Vec<String> {
    let info = get_blockchain_info(node).await.unwrap();
    let tips = get_chain_tips(node).await.unwrap();
    let peers = get_peer_info(node).await.unwrap();
    let slot = get_attested_tip(node).await.ok();
    let t = Tick {
        blocks: info.blocks,
        headers: info.headers,
        tips: &tips,
        peers: &peers,
        frontier: slot.as_ref(),
    };
    let lines = catchup_assist::tick(watched, cu, &t, now).await;
    for line in &lines {
        eprintln!("catch-up help: {line}");
    }
    lines
}

#[tokio::test]
#[ignore]
async fn the_help_moves_a_node_its_engine_leaves_standing() {
    let Some(bin) = std::env::var_os("EASYNODE_TEST_BTXD") else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let ports = Ports {
        a_rpc: 29451,
        a_p2p: 29461,
        b_rpc: 29452,
    };
    let pair = stalled_pair(&bin, ports, true).await;

    // The help, ticked like the refresher: 3 s of its clock per tick.
    let watched = Watched {
        inner: &pair.b.rpc,
        methods: Mutex::new(Vec::new()),
    };
    let archive = format!("127.0.0.1:{}", ports.a_p2p);
    let mut cu = CatchUp::new(vec![archive.clone()]);
    let clock = Instant::now();
    let mut lines = Vec::new();
    for i in 0..200u32 {
        if blocks(&pair.b.rpc).await >= 99 {
            break;
        }
        let now = clock + Duration::from_secs(3 * u64::from(i));
        lines.extend(tick_once(&pair.b.rpc, &watched, &mut cu, now).await);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let reached = blocks(&pair.b.rpc).await;
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

    stop(pair.b).await;
    stop(pair.a).await;
}

#[tokio::test]
#[ignore]
async fn a_peer_that_drops_us_twice_over_old_blocks_is_not_asked_for_them_again() {
    let Some(bin) = std::env::var_os("EASYNODE_TEST_BTXD") else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let ports = Ports {
        a_rpc: 29453,
        a_p2p: 29463,
        b_rpc: 29454,
    };
    let pair = stalled_pair(&bin, ports, false).await;

    let watched = Watched {
        inner: &pair.b.rpc,
        methods: Mutex::new(Vec::new()),
    };
    let archive = format!("127.0.0.1:{}", ports.a_p2p);
    let once = catchup_assist::dropped_once_line(&archive);
    let mark = catchup_assist::refuses_old_line(&archive);
    let mut cu = CatchUp::new(vec![archive.clone()]);
    let clock = Instant::now();
    let mut lines = Vec::new();
    // (tick of the mark, requests sent by then)
    let mut marked: Option<(u32, usize)> = None;
    for i in 0..120u32 {
        let now = clock + Duration::from_secs(3 * u64::from(i));
        lines.extend(tick_once(&pair.b.rpc, &watched, &mut cu, now).await);
        match marked {
            None if lines.contains(&mark) => {
                marked = Some((i, watched.count("getblockfrompeer")));
            }
            // Twenty ticks, a minute of the help's clock, after the mark.
            Some((at, _)) if i >= at + 20 => break,
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let Some((_, sent_by_the_mark)) = marked else {
        panic!("the help never said A dropped us; it said {lines:?}");
    };
    assert!(
        lines[0].starts_with(&format!("asked {archive} for blocks 2 to 101 (")),
        "{lines:?}"
    );
    // Asked twice: once at first, once more after the first drop.
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("asked ")).count(),
        2,
        "asked A for old blocks a third time: {lines:?}"
    );
    assert_eq!(lines.iter().filter(|l| **l == once).count(), 1, "{lines:?}");
    assert_eq!(
        watched.count("getblockfrompeer"),
        sent_by_the_mark,
        "requests after the mark"
    );
    assert_eq!(lines.iter().filter(|l| **l == mark).count(), 1, "said once");
    // With A back and marked, the help concludes that no archive peer serves
    // old blocks to B, and says so once.
    let concluded = catchup_assist::NO_ARCHIVE_SERVES_OLD_BLOCKS.to_string();
    assert_eq!(lines.iter().filter(|l| **l == concluded).count(), 1, "{lines:?}");
    assert!(cu.no_archive_serves_old_blocks());
    assert_eq!(blocks(&pair.b.rpc).await, 1, "A served B an old block");
    for m in watched.methods.lock().unwrap().iter() {
        assert!(ALLOWED.contains(&m.as_str()), "the help sent {m}");
    }

    stop(pair.b).await;
    stop(pair.a).await;
}
```

- [ ] **Step 2: Run it without an engine (CI's view)**

Run: `cd crates/btx-core && cargo test --locked --test catchup_regtest 2>&1 | grep "test result"`
Expected: `test result: ok. 0 passed; 0 failed; 2 ignored`.

- [ ] **Step 3: Run it against the shipped engine**

Run with any v0.34.9 `btxd`: on the owner's Mac, `apps/node/src-tauri/resources/node-pkg/bin/btxd` once `apps/node/scripts/stage-node-pkg.sh` has staged the package (Task 11 Step 1), or the copy the 2026-09-29 spike used:

```bash
cd crates/btx-core
EASYNODE_TEST_BTXD=/path/to/btxd cargo test --locked --test catchup_regtest -- --ignored --nocapture --test-threads=1 2>&1 | grep -E "catch-up help|test result"
```

Expected (derived, not run: the first test's lines are the dry run of 2026-09-29 with the drop line two strikes add, the rest what the second test's asserts require; the counts in brackets vary with the connection, and a "paused: none of this app's archive peers is connected" line can sit after either drop while A reconnects):

```text
catch-up help: asked 127.0.0.1:29461 for blocks 2 to 101 (16 sent, 0 already here)
catch-up help: 127.0.0.1:29461 dropped the connection once when asked for old blocks, so the other archive peers are asked first; after a second time it is asked only for newer blocks until the node restarts
catch-up help: paused: none of this app's archive peers is connected with the next block
catch-up help: asked 127.0.0.1:29461 for blocks 2 to 101 (100 sent, 0 already here)
catch-up help: asked 127.0.0.1:29463 for blocks 2 to 101 (N sent, 0 already here)
catch-up help: 127.0.0.1:29463 dropped the connection once when asked for old blocks, so the other archive peers are asked first; after a second time it is asked only for newer blocks until the node restarts
catch-up help: asked 127.0.0.1:29463 for blocks 2 to 101 (N sent, 0 already here)
catch-up help: 127.0.0.1:29463 does not serve old blocks to us: it dropped the connection twice when asked for them, so until the node restarts it is asked only for blocks within 288 of its newest
catch-up help: stopped asking for old blocks: none of the archive peers connected now serves them to this node
test result: ok. 2 passed; 0 failed; 0 ignored
```

The dry run's first line was a drop although A grants `noban`, and nothing in the rule explains it. With two strikes that drop is logged and A is asked again, as in the dry run. Only a second such drop marks A; the first test then fails with `tip 1; the help said [.., "127.0.0.1:29461 does not serve old blocks to us: ..", ..]`. That failure is a finding to report (Risk 2), not a bug to fix by loosening the rule here. If the second test panics with "the help never said A dropped us" and its lines end with the first drop and no second "asked", A came back without announcing a header above B's tip (`synced_headers` stays -1 until it does), so the help had no one to ask: the dry run saw A picked again after its drop, but that is the one step here nobody has watched against a peer without `noban`. The same holds after the second drop: the conclusion line and `no_archive_serves_old_blocks()` need A back with a header above B's tip; without one the help says `paused: none of this app's archive peers is connected with the next block` and draws no conclusion, by design (Decision 19). If a test fails at "the engine fetched on its own", check that block 1 went in after the headers (the setup's comment says why). If A never passes 99, the engine qualified the device differently: read `$TMPDIR/.../regtest/debug.log` of A for "ExactReplay required before activation". If the second test panics with "the help never said A dropped us", read A's `debug.log` for "Ignore block request below NODE_NETWORK_LIMITED threshold" (logged under `-debug=net` only; add it to `a_args` to see it).

- [ ] **Step 4: Lint and commit**

```bash
cargo fmt --all && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cd ../..
git add crates/btx-core/tests/catchup_regtest.rs
git commit -F - <<'EOF'
core: the catch-up help moves a stalled regtest node, and leaves a peer that drops it twice over old blocks alone, proven against the engine (opt-in)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

Clippy's `zombie_processes` lint (in `suspicious`) is why `start` kills and waits for the child before it panics.

---

### Task 7: The watchdog names the help, and says when no archive peer serves old blocks

**Files:**
- Modify: `crates/btx-core/src/watchdog.rs:240-248` (the `BlockFetchGated` verdict) and `:545-571` (its test); `:141` (`StallClass`), `:258` (before the class-C verdict), tests before `:573`

**Interfaces:**
- Consumes: `catchup_assist::MIN_BEHIND` (Task 4), in the test only.
- Produces: `StallClass::OldBlocksRefused` (serialised `old_blocks_refused`) and `watchdog::old_blocks_refused_verdict() -> StallVerdict`, the status card's sentence while `CatchUp::no_archive_serves_old_blocks()` holds. Task 8 puts it in `NodeStatusInfo::stall`.

The sentence is shown on the validation card (`apps/node/src/validation.ts:85-90`) when a mirror has been frozen for 15 minutes with nothing in flight. It is on main unchanged since before Tools, so it still says "nothing in this app does yet" although the Tools button "Fetch a stuck block" has asked peers by name since `aed8755`. This task changes that one sentence in place and names both: the help for a node 20 or more behind, the button for a stuck tip closer than that. The one new verdict is the help's conclusion that no archive peer serves old blocks (the owner's decision 1): `discriminate` does not issue it, because the engine's rescue moves the tip every few minutes in that state and a 15-minute freeze never comes; the shell shows it on any node while the conclusion holds (Task 8).

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
        // Closer than that, the Tools button on main does it once; and when
        // the tip still does not move, the report says what was tried.
        assert!(v.summary.contains("Fetch a stuck block"));
        assert!(v.summary.contains("Copy diagnostics"));
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
                      archive peers for the next blocks by name; closer than that, Tools > \
                      Fetch a stuck block asks for them once. If the tip still does not move, \
                      those peers may not be connected, or may not serve old blocks to this \
                      node. Copy diagnostics in Tools shows what the app tried",
```

- [ ] **Step 4: The sentence for the help's conclusion, test first**

In `crates/btx-core/src/watchdog.rs`, inside `mod tests`, replace:

```rust
    #[test]
    fn outstanding_requests_stay_body_missing_so_redial_is_still_offered() {
```

with:

```rust
    #[test]
    fn the_catch_up_help_s_conclusion_has_its_own_sentence() {
        let v = old_blocks_refused_verdict();
        assert_eq!(v.class, StallClass::OldBlocksRefused);
        assert_eq!(
            serde_json::to_value(v.class).unwrap(),
            "old_blocks_refused"
        );
        for part in [
            "none of the archive peers",
            "twice",
            "Fast-forward",
            "Copy diagnostics",
        ] {
            assert!(v.summary.contains(part), "{part}");
        }
        assert!(!v.summary.contains('\u{2014}'), "no em-dashes in copy");
        assert!(
            !v.summary.contains("automatically") && !v.summary.contains("will fix"),
            "say what the app does, never promise the outcome"
        );
    }

    #[test]
    fn outstanding_requests_stay_body_missing_so_redial_is_still_offered() {
```

Run: `cd crates/btx-core && cargo test --locked --lib the_catch_up_help_s_conclusion 2>&1 | grep -E "^error" | head -2`
Expected: `cannot find function `old_blocks_refused_verdict`` and `no variant ... named `OldBlocksRefused``.

Then replace:

```rust
    BlockFetchGated,
    AttestationMissing,
```

with:

```rust
    BlockFetchGated,
    /// The catch-up help concluded that no archive peer connected serves old
    /// blocks to this node (`crate::catchup_assist::CatchUp::
    /// no_archive_serves_old_blocks`). Not issued by [`discriminate`]: the
    /// shell shows [`old_blocks_refused_verdict`] while that holds.
    OldBlocksRefused,
    AttestationMissing,
```

and insert, directly above the doc comment of `fn no_qualifying_peer_verdict()` (`:258`, the line that starts `/// The class-C verdict`, unchanged):

```rust
/// The status card's sentence while the catch-up help concludes that no
/// archive peer serves old blocks to this node (the owner's decision 1, the
/// night of 2026-09-29). On any node, not only mirrors, and without waiting
/// for a freeze: the engine's own rescue still connects a block every few
/// minutes in that state.
pub fn old_blocks_refused_verdict() -> StallVerdict {
    StallVerdict {
        class: StallClass::OldBlocksRefused,
        summary: "this node is far behind and none of the archive peers connected now serves \
                  old blocks to it: each dropped the connection twice when asked for them. \
                  This app has stopped asking them, and the node keeps asking on its own, \
                  which is slow. Fast-forward can take it closer to the tip; Copy \
                  diagnostics in Tools names the peers",
    }
}

```

- [ ] **Step 5: Run the watchdog tests**

Run: `cd crates/btx-core && cargo test --locked --lib watchdog 2>&1 | grep "test result" | head -1`
Expected: `test result: ok. 17 passed; 0 failed` (the 16 `#[test]` in `watchdog.rs` at `b330e3d`, and the new one; derived, not run).

- [ ] **Step 6: Commit**

```bash
cd crates/btx-core && cargo fmt --all && cd ../..
git add crates/btx-core/src/watchdog.rs
git commit -F - <<'EOF'
core: the stalled-scheduler verdict names the catch-up help and the Tools button, and a sentence for when no archive peer serves old blocks

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 8: The status refresher runs the help

**Files:**
- Modify: `apps/node/src-tauri/src/state.rs:652` (after the `signed_frontier` slot) and `:788` (its initialiser)
- Modify: `apps/node/src-tauri/src/commands.rs:1530` (the refresher's slot clones), `:1595-1597` (before the refresher's `loop {`), `:2077-2080` (between the fork block and the watchdog tick), the two stop/start clears at `:1035` and `:2543`, and `:3318` (`stall` in `get_node_status`)

**Interfaces:**
- Consumes: `catchup_assist::{CatchUp, CatchUpReport, Tick, tick}` and `CatchUp::report` (Task 5); `watchdog::old_blocks_refused_verdict` (Task 7). From the refresher's own tick: `chain` (`getblockchaininfo`, read every tick), `peer_infos: Option<Vec<PeerInfo>>` (`:1699`, one `getpeerinfo` per tick), `fork_tips: Vec<ChainTip>` (`getchaintips`, refreshed every `FORK_CHECK_EVERY` = 10 ticks, `:1574` and `:1578`), `signed_frontier_slot` (`:1530`, filled at `:1817-1824` from this tick's `getmatmulattestedtip` on every node, #160), `rpc: RpcClient`, `bootstrap_launch`, and `setup_log` (`:3367`).
- Produces: `AppState::catch_up_help: Arc<Mutex<btx_core::catchup_assist::CatchUpReport>>` (tokio `Mutex`, like the other slots): `.lines`, the help's diagnostics lines, and `.no_archive_serves_old_blocks`, the conclusion, both as of the refresher's last tick; reset to `CatchUpReport::default()` (no lines, `false`) on every stop and start. Task 9 prints `.lines`; `get_node_status` turns the conclusion into the status card's sentence; the Fast-forward plan reads the flag.

The refresher (`spawn_status_refresher`, `:1519`) is spawned per node run, so the help's memory, and with it the list of peers that dropped us over old blocks, starts fresh on every start and restart. A tip that is 30 seconds old at most is fine for the target: the walk joins the tip from whatever header the tips named. The frontier comes from the slot, not from a second `getmatmulattestedtip`: the block at `:1814-1824` has already written this tick's answer there when the answer came, and a failed read leaves the last good one, which is what the help should follow. Lines go to stderr and to `<datadir>/setup.log`, where the header bootstrap already writes its end message.

- [ ] **Step 1: The slot for Copy diagnostics, the status card and Fast-forward**

In `apps/node/src-tauri/src/state.rs`, replace:

```rust
    pub signed_frontier: Arc<Mutex<Option<btx_core::node_api::AttestedTip>>>,
```

with:

```rust
    pub signed_frontier: Arc<Mutex<Option<btx_core::node_api::AttestedTip>>>,
    /// What the catch-up help (`btx_core::catchup_assist`) is doing this run,
    /// as the refresher's last tick left it (`CatchUp::report`): the Copy
    /// diagnostics lines, and whether it concluded that no archive peer serves
    /// old blocks to this node. The status card and the Fast-forward offer
    /// read that flag. Reset on every stop/start like the others.
    pub catch_up_help: Arc<Mutex<btx_core::catchup_assist::CatchUpReport>>,
```

and replace:

```rust
            signed_frontier: Arc::new(Mutex::new(None)),
```

with:

```rust
            signed_frontier: Arc::new(Mutex::new(None)),
            catch_up_help: Arc::new(Mutex::new(Default::default())),
```

In `apps/node/src-tauri/src/commands.rs`, the start clears (`:1035`); replace:

```rust
    *state.signed_frontier.lock().await = None;
    *state.recent_signers.lock().await = None;
```

with:

```rust
    *state.signed_frontier.lock().await = None;
    *state.catch_up_help.lock().await = Default::default();
    *state.recent_signers.lock().await = None;
```

and the stop clears (`:2543`); replace:

```rust
    *state.signed_frontier.lock().await = None;
    *state.fork.lock().await = None;
```

with:

```rust
    *state.signed_frontier.lock().await = None;
    *state.catch_up_help.lock().await = Default::default();
    *state.fork.lock().await = None;
```

Each of the two is unique in the file at `b330e3d`.

- [ ] **Step 2: Add the help's state**

In `spawn_status_refresher`, replace:

```rust
    let signed_frontier_slot = state.signed_frontier.clone();
```

with:

```rust
    let signed_frontier_slot = state.signed_frontier.clone();
    let catch_up_slot = state.catch_up_help.clone();
```

and replace:

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
        // (the walked headers, the batch out, the peers that dropped us over
        // old blocks) lasts for this run only.
        let mut catch_up = btx_core::catchup_assist::CatchUp::for_this_app();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
```

- [ ] **Step 3: Tick it**

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
                    // Every node, every tick, from the reads above, the signed
                    // frontier included: the slot this tick already filled
                    // (#160), so no second getmatmulattestedtip. It reads more
                    // from the node only while a header it knows is 20 or more
                    // blocks above the tip. Not during a header bootstrap,
                    // which ends in a restart.
                    if !bootstrap_launch {
                        if let Some(peers) = peer_infos.as_deref() {
                            let frontier = signed_frontier_slot.lock().await.clone();
                            let tick = btx_core::catchup_assist::Tick {
                                blocks: chain.blocks,
                                headers: chain.headers,
                                tips: &fork_tips,
                                peers,
                                frontier: frontier.as_ref(),
                            };
                            for line in btx_core::catchup_assist::tick(
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
                            *catch_up_slot.lock().await = catch_up.report();
                        }
                    }

                    // ── Trusted-mirror stall watchdog tick ──────────────────
```

- [ ] **Step 4: The status card says when no archive peer serves old blocks**

The owner's decision 1: while the help concludes that no archive peer serves old blocks, the card says so, on any node. The watchdog's own verdict needs a 15-minute freeze on a mirror, which the engine's rescue prevents in that state, so the conclusion is read here and outranks it. The watchdog slot itself is left alone, so its progress rule and its redial are unchanged. In `get_node_status`, replace:

```rust
        stall: state.stall_verdict.lock().await.clone(),
```

with:

```rust
        // The catch-up help's conclusion (btx_core::catchup_assist) outranks
        // the watchdog's verdict: it holds on any node while the tip trickles
        // in from the engine's rescue, and it says what to do.
        stall: if state.catch_up_help.lock().await.no_archive_serves_old_blocks {
            Some(btx_core::watchdog::old_blocks_refused_verdict())
        } else {
            state.stall_verdict.lock().await.clone()
        },
```

It is unique in the file at `b330e3d` (`:3318`). The front end needs no change: `validation.ts:85-90` shows any `status.stall` as "Needs attention" with its summary, and the class arrives as the string `old_blocks_refused`.

- [ ] **Step 5: Run the shell's checks**

Run: `cd apps/node/src-tauri && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cargo test --locked 2>&1 | grep "test result" | head -1`
Expected: no fmt output, `Finished ...`, and the count from Task 3 Step 1 with 0 failed. (The refresher and `get_node_status` have no unit test here; Task 6 drives `tick` exactly as this block does, the slot's value included, Task 5 tests `report()`, Task 7 the sentence, and Task 11 runs it for real.)

- [ ] **Step 6: Commit**

```bash
git add apps/node/src-tauri/src/state.rs apps/node/src-tauri/src/commands.rs
git commit -F - <<'EOF'
node: the status refresher asks archive peers for the next blocks while the engine asks nobody, and the card says when none serves old blocks

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 9: Copy diagnostics says what the help did

**Files:**
- Modify: `crates/btx-core/src/diagnostics.rs:70` (`DiagnosticsInput`), `:216-219` (after the Watchdog line in `render`), tests at `:1102` and `:1247`
- Modify: `apps/node/src-tauri/src/tools.rs:491-496` (the `stall` field of the input in `tools_diagnostics`)

**Interfaces:**
- Consumes: `AppState::catch_up_help` and its `.lines` (Task 8), `catchup_assist::refuses_old_line` (Task 4), in the tests only.
- Produces: `DiagnosticsInput::catch_up: Vec<String>`, printed as a "Catch-up help" section under "Watchdog".

The lines are built in `catchup_assist` (`CatchUp::diagnostics`, tested in Task 5), so this task only carries them. The addresses in them are this app's own archive peers, which `published_peer_hosts()` lists, so redaction keeps them; the first test below proves that on the real pipeline.

- [ ] **Step 1: Change the tests first**

In `crates/btx-core/src/diagnostics.rs`, in `fn report_hides_every_private_value_end_to_end`, replace:

```rust
            attested_tip: None,
            stall: None,
            log_warnings: vec![format!(
```

with:

```rust
            attested_tip: None,
            stall: None,
            catch_up: vec![crate::catchup_assist::refuses_old_line(
                "109.199.124.187:19335",
            )],
            log_warnings: vec![format!(
```

and, in the same test, replace:

```rust
        for gone in [wif.as_str(), "84.32.49.226"] {
            assert!(!out.contains(gone), "{gone} survived end to end:\n{out}");
        }
```

with:

```rust
        for gone in [wif.as_str(), "84.32.49.226"] {
            assert!(!out.contains(gone), "{gone} survived end to end:\n{out}");
        }
        assert!(
            out.contains("  109.199.124.187:19335 does not serve old blocks to us"),
            "the app's own archive peer is named:\n{out}"
        );
```

In `fn the_report_has_every_section_and_no_em_dash`, replace:

```rust
            attested_tip: None,
            stall: None,
            log_warnings: vec!["[warning] something".into()],
```

with:

```rust
            attested_tip: None,
            stall: None,
            catch_up: vec!["not asking any peer for blocks right now".into()],
            log_warnings: vec!["[warning] something".into()],
```

and, in its list of parts, replace:

```rust
            "not shown on the home screen",
            "Last warning lines of debug.log (1)",
```

with:

```rust
            "not shown on the home screen",
            "Catch-up help\n  not asking any peer for blocks right now",
            "Last warning lines of debug.log (1)",
```

- [ ] **Step 2: Run them to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib diagnostics 2>&1 | grep -E "^error" | head -3`
Expected: `error[E0560]: struct `DiagnosticsInput` has no field named `catch_up``.

- [ ] **Step 3: Implement**

In `crates/btx-core/src/diagnostics.rs`, replace:

```rust
    pub stall: Option<String>,
    pub log_warnings: Vec<String>,
}
```

with:

```rust
    pub stall: Option<String>,
    /// What the catch-up help is doing this run (`crate::catchup_assist`,
    /// `CatchUp::diagnostics`), one line each. Empty when no refresher ran.
    pub catch_up: Vec<String>,
    pub log_warnings: Vec<String>,
}
```

and replace:

```rust
    o.push(format!(
        "Watchdog: {}",
        i.stall.as_deref().unwrap_or("nothing to report")
    ));
```

with:

```rust
    o.push(format!(
        "Watchdog: {}",
        i.stall.as_deref().unwrap_or("nothing to report")
    ));
    o.push("Catch-up help".into());
    if i.catch_up.is_empty() {
        o.push("  nothing to report".into());
    }
    for l in &i.catch_up {
        o.push(format!("  {l}"));
    }
```

In `apps/node/src-tauri/src/tools.rs`, in `tools_diagnostics`, replace:

```rust
        stall: state
            .stall_verdict
            .lock()
            .await
            .as_ref()
            .map(|v| v.summary.to_string()),
```

with:

```rust
        stall: state
            .stall_verdict
            .lock()
            .await
            .as_ref()
            .map(|v| v.summary.to_string()),
        catch_up: state.catch_up_help.lock().await.lines.clone(),
```

The other literals of `DiagnosticsInput` (`diagnostics.rs:706`, `:1211`, `tools.rs:473`, `:863`) end in `..Default::default()` and need nothing.

- [ ] **Step 4: Run the checks**

Run: `cd crates/btx-core && cargo test --locked --lib diagnostics 2>&1 | grep "test result" | head -1 && cd ../../apps/node/src-tauri && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cargo test --locked 2>&1 | grep "test result" | head -1`
Expected: both `test result: ok.` with 0 failed (counts derived, not run: note yours), no fmt output, `Finished ...`.

- [ ] **Step 5: Format and commit**

```bash
cd ../../../crates/btx-core && cargo fmt --all && cd ../..
git add crates/btx-core/src/diagnostics.rs apps/node/src-tauri/src/tools.rs
git commit -F - <<'EOF'
node: Copy diagnostics says what the catch-up help asked, and which peer does not serve old blocks to us

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 10: Changelog, and every check CI runs

**Files:**
- Modify: `apps/node/CHANGELOG.md` (under `## [Unreleased]`, after the role-card paragraph from #160)

- [ ] **Step 1: Add the entry**

In `apps/node/CHANGELOG.md`, after the paragraph that starts `**The role card no longer says "At the tip" on a node that is not there.**` (the second paragraph under `## [Unreleased]` at `b330e3d`, after the Tools one) and before `## [0.6.32] - 2026-09-28`, insert:

```markdown
**A node that is far behind now asks for its next blocks by name.** Most
nodes on the network keep only recent blocks, and the node engine asks them
for an older block only after two minutes of waiting, so a node that started
days behind gained about one block every two minutes. When your node is 20 or
more blocks behind and nobody has been asked for its next block for 30
seconds, the app now asks its own archive peers for the next 100 blocks by
name, waits for them to arrive, and repeats. For older blocks it asks peers
that keep the whole chain first. A peer that drops the connection twice when
asked for older blocks is not asked for them again until the node restarts,
and Copy diagnostics in Tools names it. If none of the archive peers
connected serves older blocks to your node, the app stops asking them, the
status card says so, and Fast-forward is offered. The app stops asking as
soon as the engine is fetching blocks on its own again. On 29 September this
moved a test node 7,490 blocks in 8 minutes. It never adds, bans or
disconnects a peer, and never asks for a branch the app refuses.

```

- [ ] **Step 2: Run every check CI runs**

```bash
(cd crates/btx-core && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cargo test --locked 2>&1 | grep -E "test result|FAILED")
(cd apps/node/src-tauri && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious 2>&1 | tail -1 && cargo test --locked 2>&1 | grep -E "test result|FAILED")
(cd apps/node && npm ci && npx tsc --noEmit && npm test 2>&1 | tail -3)
git diff origin/main -- crates apps | grep '^+' | grep -c '—'
```

Expected: every `test result: ok.` with 0 failed and no `FAILED`; both clippy runs end `Finished ...`; `tsc` prints nothing; vitest reports all passed; the em-dash count is `0` (none of this plan's added or touched lines has one; the `──` in the refresher's section rule is a box-drawing character, not a dash).

- [ ] **Step 3: Commit**

```bash
git add apps/node/CHANGELOG.md
git commit -F - <<'EOF'
changelog: catch-up help

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 11: Mainnet acceptance on the owner's Mac (by hand, no commit)

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
1. Within about 2 minutes of the snapshot load, `setup.log` shows `catch-up help: asked 109.199.124.187:19335 for blocks 225,928 to 226,027 (...)` (or another listed peer; a listed peer that advertises NODE_NETWORK and is connected with the headers comes first).
2. The block count rises by at least 5,000 within the first 10 minutes of the first "asked" line (the measurement did 7,490 in 8; this is half that speed).
3. Near the tip, `setup.log` shows `catch-up help: stopped: the node is within 20 blocks of the chain it follows`, and for the next 5 minutes the count follows new blocks with no further "asked" lines.
4. The node stayed off the held branch: `$CLI -datadir="$D" getblockhash 228146` does not start with `8240c62e`.
5. Tools > Copy diagnostics, while the help is asking, has a "Catch-up help" section with `asking <peer> for blocks <from> to <to>`, and after the stop, `not asking any peer for blocks right now`.

If `setup.log` shows `<peer> dropped the connection once when asked for old blocks` or `<peer> does not serve old blocks to this node`, record which peer, which of the two, whether another was asked next, and the block count 10 minutes later. If it also shows `stopped asking for old blocks: none of the archive peers connected now serves them to this node`, check that the status card shows the sentence of Task 7 and that Copy diagnostics has the same line: that is Decision 19 on mainnet.

Record the numbers (first "asked" time, count at +5 and +10 minutes, time of "stopped", any "does not serve old blocks" line) in the pull request description.

- [ ] **Step 4: The second-IP test: pending, run on other operators' machines**

Pending, and not run from this Mac: the owner has it run from other easyNode boxes (Aleksander's zbtx1 to zbtx3 and others), each on its own public IP (see Risk 1), with Steps 2 and 3 as the procedure. A pass there is the evidence that 109.199.124.187 serves old blocks to nodes other than this Mac; a `109.199.124.187:19335 does not serve old blocks to us` line is the evidence that it does not, and shows the rule of Task 4 working on mainnet. If every archive peer is marked on such a box, its setup.log shows `stopped asking for old blocks: none of the archive peers connected now serves them to this node` and its status card shows the sentence of Task 7.

- [ ] **Step 5: Clean up**

Stop the node in the window, quit the app, and delete `~/easynode-catchup-accept` yourself once the numbers are recorded.

---

## Decisions this plan made where the design is silent

1. **Which peers are "the archive peers it dials"**: manual connections whose address is in `node::block_source_peers()`. `BTX_ARCHIVE_PEERS` alone would leave out 109.199.124.187, the peer the measurement used.
2. **"The engine is fetching on its own" means its blocks are arriving** (amended 30 September): one of the next 100 heights is in flight at some peer, minus this help's own batch, **and** the engine made progress within the last 30 seconds. Its progress is the tip rising while the help has no batch out, or the tip going past the batch's last block (blocks beyond the batch connected). `getblockfrompeer` puts the help's own requests in `inflight` (engine `FetchBlock`, confirmed on regtest), so they must not count. A request alone is no sign: on mainnet the engine's next three sat in flight at the discovery relays for good (Mainnet acceptance, 30 September). So a healthy engine keeps the tip moving and the help never asks; a hung request no longer holds the help off; and after a batch of the help's connects and the engine carries on past it, the help goes idle.
3. **"The newest block has not changed for 30 seconds"** is measured per tip: the clock restarts only when the tip moves (and after a peer took none of a batch, Decision 5), never because a request is in flight.
4. **After a batch connects the next goes out at once**, without another 30-second wait, as in the measured run. The 30-second wait applies only to starting, and after a peer took nothing.
5. **Rotation happens at three minutes, and at once when the batch's peer disconnects.** A peer that takes none of a batch is rotated away from after another 30-second wait, so a refusing peer is not hammered.
6. **"Block already downloaded" counts as taken**, so a node that has the bodies and is still checking them is not moved from peer to peer.
7. **The signed frontier wins when the node reads one and knows its header.** A node at or above its frontier gets no help even if its best header is higher, because a mirror cannot connect unsigned blocks. The frontier is the refresher's `signed_frontier` slot (#160): the last good answer this run, so a failed read this tick keeps the last one rather than dropping to the best header.
8. **Refused blocks are never fetched, even with `EASYBTX_NODE_REFUSE_KNOWN_INVALID=0`.** The help only helps; the engine can still fetch whatever the operator lets it.
9. **No help when `blocks == 0` or during a header bootstrap launch** (the snapshot has not loaded; asking for blocks from genesis would be waste).
10. **1,000 headers per tick** while walking; no cap on the walk's length, so a node 30,000 behind is helped too (about 5 MB of hashes).
11. **Where it speaks**: stderr and `<datadir>/setup.log`, one line per batch, stop, pause or newly refusing peer; and Copy diagnostics, from the `catch_up_help` slot. No new screen element; the design names none.
12. **"Old" is measured against the peer's `synced_headers`** with the engine's own 288 (`LIMITED_SERVES`), the number and the field its scheduler uses. The serving side's disconnect counts 290 from its own tip; 288 from what it announced is the conservative side of that.
13. **Full-history first, limited last, for old blocks only**, with two exceptions. A limited peer whose last batch of old blocks connected goes ahead of a full-history peer not asked yet: it has shown it serves old blocks to this node. A full-history peer whose batch of old blocks went three minutes with none of it connected while it stayed connected ranks below the limited ones until it delivers. For blocks within 288 of a peer's height the order is the first draft's (the peer that delivered last, then list order): every peer serves those.
14. **"Dropped us" is: the batch's connection id is gone from `getpeerinfo`, the batch was of old blocks, and none of it connected. Two drops this run mark the peer** (`DROPS_TO_MARK = 2`; the controller's decision, the night of 2026-09-29). A reconnect shows as a new id, so it counts. The serving peer applies its rule to the first old block it is asked for and the rest of a batch is newer, so a drop after part of a batch connected is something else (a slow-delivery disconnect, a network glitch), is not counted, and only rotates. After the first drop the peer is the one rotated away from, so the other archive peers are asked first; it is asked again when its turn comes, or at once when it is the only one. The count is for the run: a batch that connects in between does not reset it, so "twice in this run" means twice, not twice in a row. A drop that looks like this Mac losing its connections is not counted (Decision 22).
15. **The mark applies to any peer, full-history (NODE_NETWORK) or limited** (the controller's decision). A NODE_NETWORK peer drops a requester over old blocks at its outbound target (`net_processing.cpp:9373-9381`), which is also "does not serve old blocks to us", and it is counted and marked the same way (`a_full_history_peer_that_drops_us_twice_is_marked_the_same_way`).
16. **The mark lasts for this node run.** It lives in the refresher's `CatchUp`, which starts fresh on every start and restart, so a peer that changes its mind is tried again after a restart. The help never reconnects, bans or disconnects the peer; the engine redials its manual peers on its own as it always has.
17. **Each drop is said once**: one log line for the first drop, one for the mark, and a standing line per peer in Copy diagnostics for the rest of the run ("dropped the connection once" or "does not serve old blocks to this node").
18. **Tools' "Fetch a stuck block" stays as it is** (the controller's decision): it is a manual action, keeps its own peer choice (`stuck_blocks::plan`), and does not read the help's drops or marks.
19. **When no archive peer serves old blocks, both** (the owner's decision 1, the night of 2026-09-29): the help (a) pauses, logs one plain line (`NO_ARCHIVE_SERVES_OLD_BLOCKS`), and says so in Copy diagnostics and on the status card (`StallClass::OldBlocksRefused`), and (b) lets the Fast-forward plan offer Fast-forward below its 1,000-block line, through `CatchUp::no_archive_serves_old_blocks()` and the `catch_up_help` slot's `no_archive_serves_old_blocks`. The conclusion is: the help pauses with `Why::NoOldBlocks`, that is, at least one archive peer is connected with the next block and every such peer dropped us twice over old blocks, the next block being old for each. "None advertises full history" is read as "no full-history peer is left to ask": a NODE_NETWORK peer that dropped us twice counts as not serving (Decision 15), and an unmarked one connected keeps the conclusion false. With no archive peer connected at all the help says `NoPeer` and draws no conclusion: that is a connection problem, not a refusal. The conclusion is sticky against the engine's rescue, which would otherwise end it every two minutes, and ends when a peer the help may ask is connected, when a batch connects, when the engine really fetches (a second block of its own within 30 seconds, which its 120-second rescue never brings, amended 30 September), when the help stops (off, within 20 blocks, a refused chain), and on every start and stop. On this branch the status card does not name Fast-forward, which this build does not have; the Fast-forward branch adds that clause with the button, and reads the slot's flag.
20. **A node that does not answer is not a peer refusing** (amended 30 September). A `getblockfrompeer` that fails without an RPC answer (a timeout, a lost connection) stops the requests for this tick. If none went out, nothing is recorded (no rotation, no new quiet wait) and one plain line says the node did not answer; the same peer is asked on the next tick. If some went out, the batch runs to the last block actually taken.
21. **Each tick asks the node for about 1.5 seconds** (`TICK_BUDGET`, amended 30 September). Every `getblockheader` takes the engine's `cs_main`, and the refresher waits for the tick. The deadline is checked between two calls: the walk stops between two headers (`HeaderPath::walk_until`, which reads one at least), the requests between two, and what went out is the batch. Not an outer timeout, which could cancel the walk or the requests midway and lose what the help knows. While the engine fetches on its own and no batch is out, the tick reads no header and nothing of the node's own chain. A tick slower than a second says so in the log, once until one is quick again. The refresher checks its generation after the tick and goes no further when a stop or restart overtook it.
22. **A drop that looks like this Mac losing its connections is not counted** (amended 30 September): no other peer of the previous tick is still connected, or the wall clock moved more than 30 seconds between two ticks (the refresher ticks every 3; `Instant` stands still while a Mac sleeps). A Wi-Fi change, a VPN toggle or a sleep would otherwise mark the one peer that serves the node. It still rotates.

## Risks

1. **The archive peer may not serve every node.** Engine `src/net_processing.cpp:9384-9391` at 84b998b4 (v0.34.12 `:9922-9929`, identical): a peer that advertises NETWORK_LIMITED and not NODE_NETWORK disconnects a requester without NoBan that asks for a block more than 290 below its tip. 109.199.124.187 and btxscan's mirror advertise NETWORK_LIMITED only, yet 109.199.124.187 served 7,490 old blocks to the owner's Mac, so it grants that IP NoBan. Whether it grants everyone is unknown. On regtest the same requests failed until the serving node granted `noban` to the requester. If other nodes are refused, each refusing peer costs two dropped connections per run, is named in the log and in Copy diagnostics, and is not asked for old blocks again after the second; the help will not speed those nodes up. Task 11 Step 4 is the check; asking the operator of 109.199.124.187 is the other.
2. **Drops that are not refusals can still mark a peer that would have served.** The regtest dry run saw a limited peer that grants `noban` drop the connection after 16 of the first 100 requests with nothing connected, and serve all 100 when asked again. Two strikes absorb one such drop: it is logged and the peer is asked again. Two in one run still mark it for the run, full-history peers included. Nothing read tonight explains the drop; `getblockfrompeer`'s own help says "When a peer does not respond with a block, we will disconnect", and the engine's slow-delivery disconnect (`CatchUpMayDisconnectOnSlowDelivery`, `:4943`) is gentler with manual and `noban` peers but not absent. Task 6's first test fails with the mark line if it happens twice.
3. **The engine's own rescue brings a block every two minutes or so** after the tip has been stuck for 120 seconds. Each such block is the engine's own progress, so for 30 seconds after it, while the next block is in flight, the help waits; then it asks. Before the 30 September amendment any request in flight held the quiet clock off, which on mainnet meant for good (Mainnet acceptance, 30 September). Once helping, the help ignores the rescue's request because it is inside the help's own batch. The rescue can also ask a limited peer for an old block and be dropped the same way; that is the engine's behaviour and not counted as the help's mark.
4. **The watchdog runs on mirrors only, after 15 minutes frozen**, so its sentence about the scheduler gate reaches mirrors only. The help's conclusion that no archive peer serves old blocks is shown on every node's status card (Task 8 Step 4), and Copy diagnostics shows the help on every node.
5. **Other parts of the app still redial and ask on their own terms.** The watchdog's remediation redials every block source with `addnode ... onetry` on a frozen mirror (`commands.rs:2181-2190`), which can reconnect a peer the help marked; it asks that peer for no block, so the mark stands. Tools' "Fetch a stuck block" ranks peers its own way (`stuck_blocks::plan`: manual, then outbound, then inbound) for at most 16 blocks and up to 5,000 behind (`MAX_WALK`), so it can still ask a limited peer for an old block and be dropped. It does not read the help's marks. Left as it is by the controller's decision (Decision 18): a manual action.

## Self-review

- **Spec coverage.** Section 11: trigger (20 behind, 30 s): Task 4 rules 2 and 6, tests `does_nothing_within_twenty_blocks`, `waits_thirty_quiet_seconds_then_asks_for_the_next_hundred`, `a_request_in_flight_while_the_tip_stands_still_does_not_restart_the_quiet_wait`, `a_new_tip_restarts_the_quiet_wait`, `while_the_engine_keeps_connecting_blocks_the_help_never_asks`, and the mainnet shape, `the_mainnet_stall_of_30_september_asks_the_archive_peer_past_the_relays_hung_requests`. Archive peers it dials: Task 4 `pick`, test `asks_only_the_app_s_own_archive_peers`. Next 100 by name: Task 5, tests `a_tick_asks_the_archive_peer_for_the_next_hundred_by_name`, `never_asks_past_what_the_peer_announced`, Task 6. Wait to connect: `waits_for_a_batch_to_connect_then_asks_for_the_next_at_once`. Rotation after three minutes: `rotates_to_the_next_archive_peer_after_three_minutes`, `a_peer_that_disconnects_is_replaced_at_once`. Signed frontier else best header: `choose_target` and `follows_the_signed_frontier_*` (both), the frontier from the slot (Task 5's `FakeNode` panics on `getmatmulattestedtip`; Task 6's `ALLOWED` leaves it out; Task 8 passes the slot). Never add, ban, disconnect: `FakeNode` panics on any other method; Task 6's `ALLOWED`. Stop when the engine fetches: `stops_when_the_engine_connects_blocks_past_the_batch_and_asks_again_when_they_stop`. Idle after a near-tip load, 200 behind with the engine fetching: `two_hundred_behind_after_a_snapshot_load_the_help_stays_idle_while_the_engine_connects_blocks`. Watchdog wording: Task 7. Rollback "one constant": `ENABLED`, test `a_refused_block_or_the_switch_stops_it`. Tests section "target and peer choice over recorded RPC answers": Task 5's `FakeNode` tests and Task 4, the peer-choice tests over `getpeerinfo` shapes with recorded service bits. Refused held branches: `a_chain_through_a_refused_block_is_never_asked_for`. Cheap repeated walk: `the_headers_are_read_once_and_kept`, and Task 2's tests. Tools shares the walk: Task 3. Real engine: Task 6, both cases of the disconnect rule. Mainnet: Task 11.
- **The disconnect rule (tonight's item 2).** NODE_NETWORK first for old blocks, limited as last resort: `a_full_history_peer_is_asked_for_old_blocks_before_a_limited_one`. Two strikes: one drop then a batch that connects leaves the peer unmarked, with the others asked first (`one_drop_then_a_batch_that_connects_leaves_the_peer_unmarked`); two drops mark a limited peer for the rest of the run (`a_limited_peer_that_dropped_us_twice_is_not_asked_for_old_blocks_again`, `a_tick_says_each_drop_once_and_marks_a_peer_on_the_second`, Task 6's second test) and a full-history one the same way (`a_full_history_peer_that_drops_us_twice_is_marked_the_same_way`). A limited peer still used within 288: `a_limited_peer_is_still_asked_for_blocks_within_288_of_its_height`. A partial batch only rotates: `a_drop_after_part_of_the_batch_connected_only_rotates`. No archive peer serves old blocks (the owner's decision 1): true and said once, held through the engine's rescue, false again when a new archive peer appears (`no_archive_serves_old_blocks_until_a_new_archive_peer_appears`), when a batch connects or near the chain (`no_archive_serves_old_blocks_ends_when_a_batch_connects_again`), in the shell's report (`a_tick_says_each_drop_once_and_marks_a_peer_on_the_second`), on the card (`the_catch_up_help_s_conclusion_has_its_own_sentence`), and against the engine (Task 6's second test). Copy diagnostics: `copy_diagnostics_says_what_the_help_is_doing`, Task 9's two report tests. Never reconnect or ban: the same `FakeNode` and `ALLOWED` checks.
- **Placeholders.** None. Every code step carries the code. The first draft's code passed `cargo fmt --check`, both clippy runs and both test suites in a throwaway worktree on 2026-09-29 (btx-core: all tests green, including its 32 new ones; shell: 96 passed), and Task 6's first test passed against v0.34.9. The amended code (Task 1's `serves_full_history` and `role.rs` literals, Task 4's peer choice and two-strike mark, Task 5's `Tick::frontier`, `Vec<String>` lines and `diagnostics`, Task 6's second test and the setup moved into `stalled_pair`, Task 8's slot, Task 9) is derived, not run. btx-core gains 45 tests by count (Task 1: 3, Task 2: 9, Task 4: 25, Task 5: 7, Task 7: 1), plus the two regtest tests; the suite totals are derived, not run.
- **Type consistency.** `HeaderPath`/`PathStatus` (Task 2) are used with the same names in Tasks 3 and 5. `Helper`, `Seen`, `Decision` (with `deep`), `Why` (with `NoOldBlocks`), `old_for`, `DROPS_TO_MARK`, `Helper::dropped`, `Helper::take_news`, `dropped_once_line`, `refuses_old_line` (Task 4) and `Tick` (with `frontier`), `CatchUp`, `CatchUp::diagnostics`, `tick` returning `Vec<String>` (Task 5) match their uses in Tasks 6, 8 and 9. `AttestedTip::hash` (Task 1) is read in `known_frontier` (Task 5) and set in `FakeNode::attested`. `serves_full_history` (Task 1) is used by `pick` (Task 4). `refused_blocks` (Task 1) is used by `refused_on_path` (Task 4) and `refused_on_own_chain` (Task 5). `MIN_BEHIND` (Task 4) is read by Task 7's test. `AppState::catch_up_help` holds a `CatchUpReport` (Task 5) from Task 8 on; Task 9 reads `.lines`, `get_node_status` and the Fast-forward plan read `.no_archive_serves_old_blocks`. `StallClass::OldBlocksRefused` and `old_blocks_refused_verdict` (Task 7) are used in Task 8 Step 4.

## For the owner

All five questions of the first amendment are answered:

1. **Every archive peer refuses old blocks: "Both"** (the owner). The help pauses and says so, and Fast-forward is offered below its 1,000-block line (Decision 19; Tasks 4, 5, 7, 8, 9; the Fast-forward plan reads the flag).
2. to 4. **Two strikes, the mark for any peer, the Tools button left as it is** (the controller; Decisions 14, 15, 18).
5. **The second-IP test** is pending, to be run on other operators' easyNode boxes (Aleksander's zbtx1 to zbtx3 and others), not this Mac (Task 11 Step 4). It decides whether 109.199.124.187 is a help to anyone but the owner's Mac.

Still worth the owner's eye, not blocking: shipping a full-history (prune=0) archive peer in `BTX_ARCHIVE_PEERS` (node.rs:178 records that 109.199.124.187's operator was evaluating one), or asking that operator to grant `noban` more widely, would make the conclusion of Decision 19 rarer.
