# Quick start or Full check, the history check's progress, and a calmer stale card: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Amended 2026-09-29 (night).** The plan was written at `origin/main` `b008f98`. Three things changed:
>
> 1. **Owner decision A (29 September): "preselect Quick start on Macs, Full check on NVIDIA machines".** Engine 0.34.9 does not move a refused Mac to Quick start by itself, and a `local_accelerator_failure` is not recorded as a refusal. So `Backend::may_check_blocks` (Metal and Cuda) still decides whether Full check *can* be picked, and a new `Backend::full_check_first` (Cuda only), shipped as the status field `full_check_first`, decides which choice is selected first. Cpu: Quick start, Full check greyed out with the reason, as before. Changed: Global Constraints, Decisions 5 and 6 and a new Decision 10, Tasks 2, 5, 8, 9, 11 and 12 (and one type anchor in Task 10), the coverage table, and the third bullet of "Where the design does not match the code", which this resolves. Checked: a Mac that keeps the preselected Quick start gets the `.follow-signatures` marker (`may_check_blocks` is true on Metal), so it follows signatures from its first start instead of trying the chip; Task 2's test now shows the Mac case by name.
> 2. **Owner decision B (29 September): "Wizard copy says 'from a recent signed snapshot' until a second operator is on the list."** The line under the choices is now "Both start from a recent signed snapshot and check the older history in the background. Full check also checks every new block on this computer's graphics card. You can switch later in Settings." Changed: Global Constraints, Task 9's markup, Task 12's screen check, and the second bullet of "Where the design does not match the code". The changelog and the tests never quoted the line.
> 3. **Rebased onto `origin/main` `aed8755` (Tools, #154).** Every line citation and find-this-text anchor was re-checked there. Tools moved `main.ts` down by one line after its imports, `index.html` by six after the header, `lib.rs` by eight, the `node.rs` tests by 29 and the `node_api.rs` tests by 22; `commands.rs`, `state.rs`, `role.rs`, `backend.rs`, `catchup-trend.ts` and `styles.css` anchors did not move. The expected test counts now include the tests Tools added. Tasks 3 and 4 say which anchor to use if #160 (`claude/role-card-not-at-tip`, not merged yet) lands first. Task 10 notes that Tools reads the chain card.
>
> The amended code blocks and counts were not run; see "Dry run" at the end.

> **Amended 2026-09-30.** The owner reworded the line under the choices once more before it shipped (`docs/decisions/2026-09-29-quick-start-full-check-and-progress.md:51`); this plan's quotes are updated to that shipped wording, word for word from `apps/node/index.html`: "Both start from a recent signed snapshot and check the older history in the background. Full check also checks every new block on this computer's graphics card. You can switch later in Settings."

**Goal:** Build spec C of easyNode 0.7.0 as approved on 2026-09-29: the setup screen asks Quick start or Full check and hands the answer to `begin_setup`; the status screen shows the background check of the snapshot's older history as one line and a thin bar; a validating node on a signed snapshot says its older history is still being checked; the stale-tip card stays calm while the gap closes; and a Mac whose chip the engine refused is told so once.

**Architecture:** Every rule is a small pure function with its own tests: the history check and the base-height cache in `btx_core::node_api`, the start choice in `btx_core::node` and `btx_core::backend`, the role sentence in `btx_core::role`, the stale card in `catchup-trend.ts`, and the history line and setup-screen rules in two new TypeScript modules. The Tauri shell only wires them: the refresher projects `getchainstates` into one new `AppState` slot, `get_node_status` ships four new fields, and `begin_setup` takes an optional choice. `main.ts` lays the answers out in the existing wizard and status screen.

**Tech Stack:** Rust (`crates/btx-core`, Tauri 2 shell in `apps/node/src-tauri`), TypeScript with Vite and Vitest (`apps/node`), plain HTML and CSS.

**Source of the requirements:** `docs/decisions/2026-09-29-quick-start-full-check-and-progress.md` on this branch (all three "Choices for the owner" approved as proposed, then choice 1 and the wizard line changed by the owner's decisions A and B of 29 September, see the amendment note above), and sections 7 and 8 of `docs/decisions/2026-09-29-every-node-starts-near-the-tip.md` on `origin/claude/cosigned-snapshots`.

## Global Constraints

- Branch: `claude/ui-start-choice-and-progress`. It holds only the decision document and this plan on top of `origin/main` at `b008f98`. Before Task 1, rebase it onto `origin/main` at `aed8755` (Tools, #154): its two commits touch only these two documents, which Tools does not, so the rebase is clean. Every path below is relative to the repository root; every cited line number is on `origin/main` at `aed8755`.
- If #160 (`claude/role-card-not-at-tip`, "At the tip only when the clock and the signed frontier agree") merges before this work starts, rebase onto it too. It moves two anchors in Task 3 (`role.rs`) and one in Task 4 (`commands.rs`); each task says what to find instead. It also adds three `btx-core` library tests (all in `role.rs`), so add 3 to every `btx-core` library count below. Line numbers in `commands.rs` after line 1032 and in `role.rs` after line 45 move with it; the text anchors do not, except the three named in Tasks 3 and 4.
- Commits: one per task, message ends with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Prefixes follow the log: `core:` for `crates/btx-core`, `node:` for `apps/node`, `changelog:` for the changelog.
- CI gates, run before each commit for the part the task touched:
  - `crates/btx-core`: `cargo fmt --all --check`, `cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious`, `cargo test --locked`.
  - `apps/node/src-tauri`: the same three commands. They need the ignored placeholder `apps/node/src-tauri/resources/node-pkg/CI-PLACEHOLDER` (see Task 4, Step 1).
  - `apps/node`: `npx tsc --noEmit`, `npm test`, `npx vite build`.
- The window stays 560x780. No new top-level screens and no new overlays (the only new overlay in 0.7.0 is Tools, decided separately). The status screen stays home; every new line sits in the existing scrolling `.screen`.
- User-facing copy: friendly, simple, no hype, no guarantees, **no em-dashes**. Existing strings that already contain one (`catchupLine`'s "still catching up — ") are not touched.
- Copy, verbatim from the design, except the line under the choices, which is the owner's (decision B):
  - Choice 1: "Quick start", with "Follows signatures, ready in minutes."
  - Choice 2: "Full check", with "Validates every block, takes longer the first time."
  - The line under them: "Both start from a recent signed snapshot and check the older history in the background. Full check also checks every new block on this computer's graphics card. You can switch later in Settings." (The design's "confirmed by two node operators" comes back once a second operator is on the list.)
  - Full check greyed out: "This computer has no graphics card the BTX engine can check blocks with."
  - Refused Mac, once: "This Mac's graphics chip didn't pass the engine's check, so your node follows signatures. Nothing for you to do."
  - History line: "Checking older history: 131,200 of 225,927 (58%)" (numbers from the node). No time estimate.
  - Role line on a signed snapshot: "Checks every new block itself. Its older history is still being checked."
  - Stale card, trend not measured: "Checking whether your node is catching up..." (three full stops, as the design writes it), neutral, not amber.
  - Stale card, gap not closing: today's sentence followed by "It adds about A blocks an hour while the network adds about 40."
  - Stale card, no newer block known: today's sentence unchanged (`apps/node/src-tauri/src/commands.rs:3145-3157`; `3162-3173` once #160 has merged).
- Names on the wire: `begin_setup` takes an optional argument `choice`, `"quick_start"` or `"full_check"`; no choice is setup as it is today. New status fields: `history_check` (`{ checked, base }` or null), `full_check_possible`, `full_check_first`, `chip_refused`.
- Which choice the setup screen offers and selects first (the owner's decision A of 29 September):
  - Cuda (an NVIDIA card the bundled engine can use): Full check selected first.
  - Metal (an Apple Silicon Mac): Quick start selected first; Full check can still be picked.
  - Cpu: Quick start; Full check greyed out, with the reason sentence.
  - `Backend::may_check_blocks` (Metal and Cuda) decides whether Full check can be picked; `Backend::full_check_first` (Cuda only) decides whether it is selected first. The window reads both from the status (`full_check_possible`, `full_check_first`) and holds no hardware rule of its own.
- The history check's base is the snapshot block's own height, from `getblockheader`, never the compiled 219,000.
- A signed snapshot is detected by the file `<datadir>/chainstate_snapshot/attested_assumeutxo`.
- Out of scope: finding, checking, downloading or loading any snapshot (spec B), and the Tools overlay.

## Decisions this plan makes (the owner may overrule any)

1. **The neutral line yields to the fork and "behind the signers" messages.** The amber stale rows keep outranking both, as today. The neutral "Checking whether..." is not a verdict, so a fork or signer message, which is, shows instead of it.
2. **"Is behind" means the node knows of 2 or more blocks beyond its tip** (`blocks_behind >= 2` on `ready`, `headers - height >= 2` on `syncing`), as on the role card (`role.rs` `HEADERS_AHEAD_IS_BEHIND`), where one header ahead is a block in flight. Anything else is row 4.
3. **The role sentence replaces the validation note rather than being appended to it.** The current note says the node "takes nobody's word for the chain", which is not yet true on a signed snapshot. The value ("Checking every block itself") and the green verdict stay.
4. **The role sentence shows while the history check runs and the `attested_assumeutxo` file exists.** So it goes the moment the engine reports the check done, not only at the next restart, when the engine deletes the file.
5. **Quick start on a machine that cannot check blocks writes no marker.** That machine follows signatures anyway, and a marker would make the Settings switch appear and do nothing. A Mac is not such a machine: `may_check_blocks` is true on Metal, so a Mac that keeps the preselected Quick start (decision A) gets the marker, follows signatures from its first start, and has the Settings switch to move to Full check. Without the marker, `launches_as_mirror` would try the chip at the first start, which is exactly what Quick start on a Mac is meant to avoid.
6. **The host's backend is asked once per app run** (`OnceLock`), because `node_host_backend` reads the loader cache and logs on every call. Both setup answers, whether Full check is possible and whether it is selected first, come from that one reading. The `EASYBTX_NODE_TRUSTED_MIRROR` override is not consulted for the setup screen.
7. **The history line stays hidden until the snapshot's own height has been read**, never shown against a guessed base, and its percentage is rounded down so it never reads 100% before the engine says done. A failed `getchainstates` keeps the last answer.
8. **The refused-Mac notice** shows when the app's refusal marker exists and the engine reports the node follows signatures (`rc_trusted_mirror`), with an OK button. OK is remembered per engine tag in `localStorage`, so a newer engine that tries the chip again and is refused again says so once more.
9. **The follow-signatures marker file's text is reworded** to cover Quick start. No code reads the text.
10. **What is selected first is decided in Rust, next to what can be picked** (`Backend::full_check_first` beside `Backend::may_check_blocks`, shipped as `full_check_first` beside `full_check_possible`), so the owner's rule (decision A: Full check first on NVIDIA, Quick start first on a Mac) lives in one tested place and the window only lays it out.

## Where the design does not match the code

These are reported, not fixed, by this plan:

- **A refused Mac is not moved to Quick start on the engine that ships.** The design says a Mac whose chip the engine refuses "moves to Quick start, as it does today". Since 0.34.5 the engine does not refuse at start; it logs a degraded start and keeps running (`crates/btx-core/src/node.rs:1365-1397`, `node_allows_degraded_matmul_start`; the app pins `v0.34.9`, `commands.rs:311`). The app records a refusal only for an init refusal with `canary=missing_golden` (`node.rs:1665`, `commands.rs:1410-1432`), and a `local_accelerator_failure` like the owner's M2 Pro on 29 September is deliberately excluded (`node.rs:4439-4468`). Such a Mac shows "NOT FOLLOWING" and gets the two-click "Follow signatures instead" offer (`main.ts:784-812`, `validation.ts:222-230`). So the "Nothing for you to do" notice, built here exactly as approved, will only appear on the older path. Making the degraded start move the node on its own would reverse the owner's 2026-09-26 rule "offered, never done for them" and `node.rs`'s rule that a device fault must not fall back silently. That is a new decision, not part of this plan. The owner weighed this on 29 September and chose decision A instead (Quick start first on a Mac, see the third bullet); the notice stays built as approved.
- **The wizard's first sentence (resolved by the owner's decision B, 29 September).** The design's "Both start from a snapshot confirmed by two node operators" is true only once spec B loads confirmed snapshots and a second operator is on the list (spec B, section 1: "While the list holds a single operator, nothing counts as confirmed and every node uses the fallbacks"). The owner chose the wording for 0.7.0: "Wizard copy says 'from a recent signed snapshot' until a second operator is on the list." This plan uses "Both start from a recent signed snapshot." One condition is left: the sentence is true for Full check only where a validating node really starts from a signed snapshot. On `aed8755` a validating node starts at the compiled 219,000, which carries no signature, and a mirror at the single-signed 225,927; with spec B (section 9) every node tries a confirmed snapshot, then the pinned 225,927 pair, and only then the compiled 219,000. So Task 9 still ships with spec B's loading path, not before it. "Recent" also ages: the pinned pair is from 21 September and falls about 960 blocks further behind the tip each day, so the word stays true only while newer snapshots keep being published. When a second operator is on the list, the design's sentence comes back.
- **Full check first on every Apple Silicon Mac (resolved by the owner's decision A, 29 September).** Approved choice 1 would have put, on 0.34.9, every Mac whose chip fails the engine's check into the degraded state above; the owner's M2 Pro is one. The owner chose Quick start first on a Mac and Full check first on an NVIDIA machine, and this plan builds that (`Backend::full_check_first`, Tasks 2, 5, 8 and 9). A Mac owner who picks Full check by hand can still land in the degraded state, and then gets the existing two-click offer, not the chip notice. Worth knowing before release: this changes today's default on a Mac. A fresh Mac setup has so far tried to check blocks; one that keeps Quick start follows signatures, cannot sign, and keeps the `.follow-signatures` marker across engine upgrades (node upgrades clear only the refusal marker), so a Mac whose chip would pass stays a mirror until its owner switches in Settings.

## File structure

| File | Change | Responsibility |
|---|---|---|
| `crates/btx-core/src/node_api.rs` | modify | `ChainStates::unchecked_snapshot_base`, `HistoryCheck`, `history_check`, `get_block_height`, `refresh_history_check`, and their tests |
| `crates/btx-core/src/backend.rs` | modify | `Backend::may_check_blocks`, `Backend::full_check_first` and their tests |
| `crates/btx-core/src/node.rs` | modify | `StartChoice`, `apply_start_choice`, `on_signed_snapshot`, the marker text, and their tests |
| `crates/btx-core/src/role.rs` | modify | `NodeRole::with_signed_snapshot` and the validation sentence on a signed snapshot, with its test |
| `apps/node/src-tauri/src/state.rs` | modify | the `history_check` slot |
| `apps/node/src-tauri/src/commands.rs` | modify | refresher projection, slot clearing, `NodeStatusInfo` fields, role wiring, `begin_setup(choice)`, `setup_backend`, `full_check_possible`, `full_check_first` |
| `apps/node/src-tauri/src/lib.rs` | modify | the E2E seam passes no choice |
| `apps/node/src/catchup-trend.ts` | modify | `paceSentence` (shared), `staleCard`, `CHECKING_CATCHUP` |
| `apps/node/src/catchup-trend.test.ts` | modify | one test per table row, the crossing, the restart, and `paceSentence` |
| `apps/node/src/history-check.ts` | create | `historyCheckView`: the line and the bar width |
| `apps/node/src/history-check.test.ts` | create | its tests |
| `apps/node/src/start-choice.ts` | create | `startChoiceView`, `setupArgs`, `NO_GPU_REASON`, `chipNoticeVisible` |
| `apps/node/src/start-choice.test.ts` | create | its tests |
| `apps/node/index.html` | modify | the two choices in the wizard card; the history line under the status line; the chip notice card |
| `apps/node/src/styles.css` | modify | styles for the three |
| `apps/node/src/main.ts` | modify | types, `reflectStartChoice`, `beginSetup`, `reflectFork`, `reflectHistoryCheck`, `reflectChipNotice` |
| `apps/node/CHANGELOG.md` | modify | an entry under `[Unreleased]` |

Task order: 1, 2, 3 (core) then 4, 5 (shell) then 6, 7, 8 (pure TypeScript) then 9, 10 (window) then 11 (changelog) then 12 (checks, no commit). Task 4 needs 1 and 3; Task 5 needs 2; Task 9 needs 5 and 8; Task 10 needs 4 to 8.

---

### Task 1: The history check, pure (`btx-core`)

**Files:**
- Modify: `crates/btx-core/src/node_api.rs:220-225` (inside `impl ChainStates`), `:227` (before `get_blockchain_info`), tests after `:1479-1501`

**Interfaces:**
- Consumes: `ChainStates`, `ChainstateEntry`, `Rpc`, `AppResult`, `json!` (all already in `node_api.rs`).
- Produces:
  - `impl ChainStates { pub fn unchecked_snapshot_base(&self) -> Option<&str> }`
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] pub struct HistoryCheck { pub checked: u64, pub base: u64 }`
  - `pub fn history_check(chainstates: &ChainStates, base_height: Option<u64>) -> Option<HistoryCheck>`
  - `pub async fn get_block_height(rpc: &dyn Rpc, hash: &str) -> AppResult<u64>`
  - `pub async fn refresh_history_check(rpc: &dyn Rpc, chainstates: &ChainStates, base: &mut Option<(String, u64)>) -> Option<HistoryCheck>`

- [ ] **Step 1: Write the failing tests**

In `crates/btx-core/src/node_api.rs`, in `mod tests`, find the end of `fully_validated_single_chainstate_has_no_snapshot` (lines 1500-1501; Tools added `PeerInfo.synced_headers`/`synced_blocks` and a test above it, and the test module now imports `serde_json::{json, Value}` itself, which the code below uses as is):

```rust
        assert!(cs.active().unwrap().validated);
    }
```

and insert directly after it:

```rust

    // ── the history check (docs/decisions/2026-09-29-quick-start-full-check-and-progress.md) ──

    /// A node on a snapshot whose base is 225,927, the pinned pair's height,
    /// with the background check at 131,200: the numbers in the decision.
    fn signed_snapshot_being_checked() -> ChainStates {
        serde_json::from_value(json!({
            "headers": 233453,
            "chainstates": [
                { "blocks": 131200, "bestblockhash": "bg", "verificationprogress": 0.41, "validated": true },
                {
                    "blocks": 233400,
                    "bestblockhash": "tip",
                    "verificationprogress": 0.9999,
                    "snapshot_blockhash": "b225927",
                    "validated": false
                }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn history_check_is_nothing_without_a_snapshot() {
        let only_background: ChainStates = serde_json::from_value(json!({
            "headers": 0,
            "chainstates": [
                { "blocks": 0, "bestblockhash": "75a9", "verificationprogress": 7.5e-6, "validated": true }
            ]
        }))
        .unwrap();
        assert_eq!(only_background.unchecked_snapshot_base(), None);
        assert_eq!(history_check(&only_background, Some(225_927)), None);
        assert_eq!(history_check(&ChainStates::default(), Some(225_927)), None);
    }

    #[tokio::test]
    async fn history_check_reports_a_snapshot_being_checked() {
        let rpc = FakeRpc::new(&[("getchainstates", snapshot_chainstates_json())]);
        let cs = get_chainstates(&rpc).await.unwrap();
        assert_eq!(
            cs.unchecked_snapshot_base(),
            Some("88a7b534ff66a863d45813668d9e53010a257af18b2d73154ec31a873bd36534")
        );
        assert_eq!(
            history_check(&cs, Some(106_875)),
            Some(HistoryCheck {
                checked: 1_200,
                base: 106_875
            })
        );
        // No base read yet: say nothing rather than guess one.
        assert_eq!(history_check(&cs, None), None);
        assert_eq!(history_check(&cs, Some(0)), None);
    }

    #[test]
    fn history_check_is_nothing_once_the_check_is_done() {
        // In the same run: the engine drops the background chainstate, and
        // the snapshot chainstate, alone, reads validated.
        let same_run: ChainStates = serde_json::from_value(json!({
            "headers": 233500,
            "chainstates": [
                {
                    "blocks": 233500,
                    "bestblockhash": "tip",
                    "verificationprogress": 1.0,
                    "snapshot_blockhash": "b225927",
                    "validated": true
                }
            ]
        }))
        .unwrap();
        assert_eq!(same_run.unchecked_snapshot_base(), None);
        assert_eq!(history_check(&same_run, Some(225_927)), None);
        // After a restart: one ordinary chainstate, no snapshot hash.
        let restarted: ChainStates = serde_json::from_value(json!({
            "headers": 233500,
            "chainstates": [
                { "blocks": 233500, "bestblockhash": "tip", "verificationprogress": 1.0, "validated": true }
            ]
        }))
        .unwrap();
        assert_eq!(history_check(&restarted, Some(225_927)), None);
    }

    #[test]
    fn history_check_counts_to_the_snapshots_own_base() {
        let cs = signed_snapshot_being_checked();
        assert_eq!(cs.unchecked_snapshot_base(), Some("b225927"));
        let h = history_check(&cs, Some(225_927)).expect("a check is running");
        assert_eq!(
            h,
            HistoryCheck {
                checked: 131_200,
                base: 225_927
            }
        );
        // Not the compiled start point, which would put the bar at 60% of the
        // wrong total.
        assert_ne!(h.base, 219_000);
        // The count never passes the base, whatever the engine reports.
        let mut past = cs.clone();
        past.chainstates[0].blocks = 230_000;
        assert_eq!(
            history_check(&past, Some(225_927)).unwrap().checked,
            225_927
        );
        // The window reads these names.
        assert_eq!(
            serde_json::to_value(h).unwrap(),
            json!({ "checked": 131200, "base": 225927 })
        );
    }

    #[tokio::test]
    async fn block_height_is_read_from_the_header() {
        let rpc = FakeRpc::new(&[(
            "getblockheader",
            json!({ "hash": "b225927", "height": 225927, "confirmations": 7500 }),
        )]);
        assert_eq!(get_block_height(&rpc, "b225927").await.unwrap(), 225_927);
        let no_height = FakeRpc::new(&[("getblockheader", json!({ "hash": "b225927" }))]);
        assert!(get_block_height(&no_height, "b225927").await.is_err());
    }

    #[tokio::test]
    async fn the_snapshots_height_is_read_once_per_snapshot() {
        let cs = signed_snapshot_being_checked();
        let rpc = FakeRpc::new(&[("getblockheader", json!({ "height": 225927 }))]);
        let mut base = None;
        assert_eq!(
            refresh_history_check(&rpc, &cs, &mut base).await,
            Some(HistoryCheck {
                checked: 131_200,
                base: 225_927
            })
        );
        assert_eq!(base, Some(("b225927".to_string(), 225_927)));

        // The next tick does not ask again: with the header gone, the
        // remembered height still answers.
        rpc.responses.lock().unwrap().remove("getblockheader");
        assert_eq!(
            refresh_history_check(&rpc, &cs, &mut base)
                .await
                .map(|h| h.base),
            Some(225_927)
        );

        // A different snapshot is read afresh, never given the old height.
        let mut other = cs.clone();
        other.chainstates[1].snapshot_blockhash = Some("b226000".to_string());
        assert_eq!(refresh_history_check(&rpc, &other, &mut base).await, None);
        assert_eq!(base, None);

        // A finished check shows nothing and asks nothing.
        let mut done = cs.clone();
        done.chainstates.remove(0);
        done.chainstates[0].validated = true;
        assert_eq!(refresh_history_check(&rpc, &done, &mut base).await, None);
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run (in `crates/btx-core`): `cargo test --locked --lib node_api::tests::history`
Expected: FAIL to compile, including `cannot find struct, variant or union type `HistoryCheck` in this scope` and `no method named `unchecked_snapshot_base` found for struct `node_api::ChainStates``.

- [ ] **Step 3: Write the implementation**

In `crates/btx-core/src/node_api.rs`, find the end of `snapshot_ready` and of `impl ChainStates` (lines 220-225):

```rust
    pub fn snapshot_ready(&self, min_snapshot_height: u64) -> bool {
        self.snapshot()
            .map(|c| c.blocks >= min_snapshot_height)
            .unwrap_or(false)
    }
}
```

Replace it with:

```rust
    pub fn snapshot_ready(&self, min_snapshot_height: u64) -> bool {
        self.snapshot()
            .map(|c| c.blocks >= min_snapshot_height)
            .unwrap_or(false)
    }

    /// The snapshot block's hash while the history below it is still being
    /// checked, or `None` once the check is done or on a node that never
    /// loaded a snapshot. The engine keeps `validated` false on the snapshot
    /// chainstate until its background chainstate reaches the base; then it
    /// drops the background one, and after a restart the snapshot chainstate
    /// is an ordinary one with no snapshot hash at all.
    pub fn unchecked_snapshot_base(&self) -> Option<&str> {
        self.snapshot()
            .filter(|s| !s.validated)
            .and_then(|s| s.snapshot_blockhash.as_deref())
    }
}

/// How far a node that started from a snapshot has got checking the history
/// below it. The engine re-checks every block from 0 up to the snapshot's own
/// height in a background chainstate; until it gets there, that older history
/// rests on the snapshot. The status screen shows this as one line and a thin
/// bar (`NodeStatusInfo.history_check`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct HistoryCheck {
    /// The highest block the background check has reached.
    pub checked: u64,
    /// The snapshot's own height, where the check ends. Read from the
    /// snapshot block's header ([`get_block_height`]), never the compiled
    /// start point: a node on the 225,927 snapshot checks up to 225,927, not
    /// 219,000.
    pub base: u64,
}

/// The history check from one `getchainstates` answer and the snapshot's own
/// height (`base_height`, read once per snapshot with [`get_block_height`]).
///
/// `None` when there is nothing to show: no snapshot, a finished check, no
/// background chainstate in the answer, or a base not read yet. The line then
/// stays hidden rather than guessing a base, because a wrong base is how a bar
/// reaches 100% on a check that is not done.
pub fn history_check(chainstates: &ChainStates, base_height: Option<u64>) -> Option<HistoryCheck> {
    chainstates.unchecked_snapshot_base()?;
    let background = chainstates.chainstates.iter().find(|c| !c.is_snapshot())?;
    let base = base_height.filter(|b| *b > 0)?;
    Some(HistoryCheck {
        checked: background.blocks.min(base),
        base,
    })
}

/// A block's height, from its header. The history check reads the snapshot
/// block's height this way, once per snapshot.
pub async fn get_block_height(rpc: &dyn Rpc, hash: &str) -> AppResult<u64> {
    let v = rpc.call("getblockheader", json!([hash, true])).await?;
    v.get("height")
        .and_then(|h| h.as_u64())
        .ok_or_else(|| crate::error::AppError::Decode(format!("getblockheader: no height in {v}")))
}

/// One refresher tick of the history check. Reads the snapshot block's height
/// from its header the first time a snapshot is seen and keeps it in `base`,
/// so every later tick costs no extra call. A different snapshot is read
/// afresh; a failed read leaves `base` empty and the line hidden until the
/// next tick reads it.
pub async fn refresh_history_check(
    rpc: &dyn Rpc,
    chainstates: &ChainStates,
    base: &mut Option<(String, u64)>,
) -> Option<HistoryCheck> {
    let hash = chainstates.unchecked_snapshot_base()?;
    if base.as_ref().map(|(h, _)| h.as_str()) != Some(hash) {
        *base = get_block_height(rpc, hash)
            .await
            .ok()
            .map(|height| (hash.to_string(), height));
    }
    history_check(chainstates, base.as_ref().map(|(_, height)| *height))
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run (in `crates/btx-core`): `cargo test --locked --lib node_api::`
Expected: PASS, `test result: ok. 43 passed` (37 on `aed8755`, Tools' `peer_sync_heights_decode_and_default_to_minus_one` among them, plus these 6), including `history_check_is_nothing_without_a_snapshot`, `history_check_reports_a_snapshot_being_checked`, `history_check_is_nothing_once_the_check_is_done`, `history_check_counts_to_the_snapshots_own_base`, `block_height_is_read_from_the_header`, `the_snapshots_height_is_read_once_per_snapshot`.

- [ ] **Step 5: Format and lint**

Run (in `crates/btx-core`): `cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious`
Expected: no diff from fmt; clippy ends with `Finished` and no `error`. (The advisory warnings it prints, such as the existing `non_snake_case` test name in `node_api.rs`, are not yours.)

- [ ] **Step 6: Commit**

```bash
git add crates/btx-core/src/node_api.rs
git commit -m "core: the history check below a snapshot, read from getchainstates and the snapshot's own header

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The start choice (`btx-core`)

**Files:**
- Modify: `crates/btx-core/src/backend.rs:25-31` (inside `impl Backend`), tests after `:659-666`
- Modify: `crates/btx-core/src/node.rs:1796-1801` (doc), `:1818-1833` (marker text, then new items after it), tests after `:4580-4608`

**Interfaces:**
- Consumes: `set_follows_signatures_by_choice`, `follows_signatures_by_choice`, `launches_as_mirror`, `Backend` (existing).
- Produces:
  - `impl Backend { pub fn may_check_blocks(&self) -> bool }` (Metal and Cuda true, Cpu false): whether Full check can be picked
  - `impl Backend { pub fn full_check_first(&self) -> bool }` (Cuda true, Metal and Cpu false): whether Full check is selected first (the owner's decision A)
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)] #[serde(rename_all = "snake_case")] pub enum StartChoice { QuickStart, FullCheck }` in `btx_core::node`
  - `pub fn apply_start_choice(datadir: &Path, choice: StartChoice, may_check_blocks: bool) -> std::io::Result<()>` in `btx_core::node`

- [ ] **Step 1: Write the failing tests**

In `crates/btx-core/src/backend.rs`, at the end of `mod host_backend_tests`, find (lines 664-667):

```rust
        assert_eq!(pc_host_backend(false, true), Backend::Cpu);
        assert_eq!(pc_host_backend(false, false), Backend::Cpu);
    }
}
```

Replace with:

```rust
        assert_eq!(pc_host_backend(false, true), Backend::Cpu);
        assert_eq!(pc_host_backend(false, false), Backend::Cpu);
    }

    /// The setup screen offers Full check on a machine with a graphics chip
    /// the engine could use, and greys it out on one without.
    #[test]
    fn only_a_graphics_backend_may_check_blocks() {
        assert!(Backend::Metal.may_check_blocks());
        assert!(Backend::Cuda.may_check_blocks());
        assert!(!Backend::Cpu.may_check_blocks());
    }

    /// Full check is selected first only on an NVIDIA card. A Mac may still
    /// pick it, but starts on Quick start, because engine 0.34.9 leaves a Mac
    /// whose chip fails its check degraded rather than moving it (the owner's
    /// decision of 2026-09-29).
    #[test]
    fn only_an_nvidia_card_has_full_check_selected_first() {
        assert!(Backend::Cuda.full_check_first());
        assert!(!Backend::Metal.full_check_first());
        assert!(Backend::Metal.may_check_blocks(), "a Mac can still pick it");
        assert!(!Backend::Cpu.full_check_first());
    }
}
```

In `crates/btx-core/src/node.rs`, find the end of `the_owners_choice_to_follow_signatures_decides_the_launch` (lines 4606-4608):

```rust
        // Withdrawing twice is not an error.
        set_follows_signatures_by_choice(dir, false).unwrap();
    }
```

and insert directly after it:

```rust

    /// The setup screen's choice decides the first start: Quick start writes
    /// the marker, Full check removes it, and a machine that cannot check
    /// blocks gets no marker at all, since it follows signatures anyway and a
    /// marker would put a Settings switch on screen that changes nothing.
    #[test]
    fn the_setup_choice_writes_or_removes_the_marker() {
        let tmp = tempfile::tempdir().expect("temp datadir");
        let dir = tmp.path();
        let btxd = Path::new("/x/btx/v0.34.9/mac/btxd");

        // A Mac that keeps the preselected Quick start: Metal may check
        // blocks, so the marker is written and the node follows signatures
        // from its first start instead of trying the chip. An NVIDIA machine
        // that picks Quick start is the same.
        let mac = Backend::Metal.may_check_blocks();
        apply_start_choice(dir, StartChoice::QuickStart, mac).unwrap();
        assert!(follows_signatures_by_choice(dir));
        assert!(launches_as_mirror(btxd, dir, Backend::Metal));
        assert!(launches_as_mirror(btxd, dir, Backend::Cuda));

        // A Mac that picks Full check: no marker, the chip is tried.
        apply_start_choice(dir, StartChoice::FullCheck, mac).unwrap();
        assert!(!follows_signatures_by_choice(dir));
        assert!(!launches_as_mirror(btxd, dir, Backend::Metal));

        // No usable GPU: Quick start writes nothing, and the node follows
        // signatures anyway.
        let no_gpu = Backend::Cpu.may_check_blocks();
        apply_start_choice(dir, StartChoice::QuickStart, no_gpu).unwrap();
        assert!(!follows_signatures_by_choice(dir));
        assert!(launches_as_mirror(btxd, dir, Backend::Cpu));

        // Full check on a fresh folder is not an error.
        let fresh = tempfile::tempdir().expect("temp datadir");
        apply_start_choice(fresh.path(), StartChoice::FullCheck, true).unwrap();
        assert!(!follows_signatures_by_choice(fresh.path()));
    }

    /// The window sends the choice by name, and an older window sends none,
    /// which is setup as it was.
    #[test]
    fn the_window_names_the_choice() {
        let quick: StartChoice = serde_json::from_value(serde_json::json!("quick_start")).unwrap();
        let full: StartChoice = serde_json::from_value(serde_json::json!("full_check")).unwrap();
        assert_eq!(quick, StartChoice::QuickStart);
        assert_eq!(full, StartChoice::FullCheck);
        assert!(serde_json::from_value::<StartChoice>(serde_json::json!("fast")).is_err());
        let none: Option<StartChoice> = serde_json::from_value(serde_json::Value::Null).unwrap();
        assert_eq!(none, None);
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run (in `crates/btx-core`): `cargo test --locked --lib -- the_setup_choice the_window_names only_a_graphics only_an_nvidia`
Expected: FAIL to compile with `cannot find type `StartChoice` in this scope`, `no method named `may_check_blocks`` and `no method named `full_check_first``.

- [ ] **Step 3: Write the implementation**

In `crates/btx-core/src/backend.rs`, find (lines 25-31):

```rust
    fn from_name(name: &str) -> Backend {
        match name {
            "metal" | "mlx" => Backend::Metal,
            "cuda" => Backend::Cuda,
            _ => Backend::Cpu,
        }
    }
```

Replace with:

```rust
    fn from_name(name: &str) -> Backend {
        match name {
            "metal" | "mlx" => Backend::Metal,
            "cuda" => Backend::Cuda,
            _ => Backend::Cpu,
        }
    }

    /// Whether a node on this backend may check blocks itself, as far as the
    /// app can tell before the engine's first start: an Apple Silicon Mac, or
    /// an NVIDIA card the bundled engine can use. `Cpu` cannot. Whether the
    /// chip then passes is the engine's verdict after start
    /// (`node::node_rc_status`); this is only what the setup screen can offer.
    pub fn may_check_blocks(&self) -> bool {
        !matches!(self, Backend::Cpu)
    }

    /// Whether the setup screen selects Full check first on this backend: only
    /// on an NVIDIA card. A Mac may check blocks and can still pick Full
    /// check, but engine 0.34.9 does not move a Mac whose chip fails its check
    /// to Quick start by itself (a `local_accelerator_failure` is not recorded
    /// as a refusal), so a Mac starts on Quick start (the owner's decision of
    /// 2026-09-29).
    pub fn full_check_first(&self) -> bool {
        matches!(self, Backend::Cuda)
    }
```

In `crates/btx-core/src/node.rs`, find (lines 1800-1801):

```rust
/// does not move at all (validation.ts `stalledFollowOffer`). It sets it only
/// on the owner's click.
```

Replace with:

```rust
/// does not move at all (validation.ts `stalledFollowOffer`). It sets it only
/// on the owner's click, or when the owner picks Quick start on the setup
/// screen ([`apply_start_choice`]).
```

Then find the body of `set_follows_signatures_by_choice` (lines 1818-1833):

```rust
pub fn set_follows_signatures_by_choice(datadir: &Path, on: bool) -> std::io::Result<()> {
    let path = follow_signatures_path(datadir);
    if on {
        std::fs::write(
            &path,
            "The owner chose, in the app, to follow signatures instead of checking\n\
             blocks on this machine, which was adding fewer blocks an hour than\n\
             the chain makes. Settings switches it back.\n",
        )
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}
```

Replace with:

```rust
pub fn set_follows_signatures_by_choice(datadir: &Path, on: bool) -> std::io::Result<()> {
    let path = follow_signatures_path(datadir);
    if on {
        std::fs::write(
            &path,
            "The owner chose, in the app, to follow signatures instead of checking\n\
             blocks on this machine: Quick start at setup, or the offer on the\n\
             status screen. Settings switches it back.\n",
        )
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// What the owner picked on the setup screen
/// (docs/decisions/2026-09-29-quick-start-full-check-and-progress.md). The
/// window sends `"quick_start"` or `"full_check"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartChoice {
    /// Follows signatures, ready in minutes.
    QuickStart,
    /// Validates every block, takes longer the first time.
    FullCheck,
}

/// Record the setup choice before the node's first start, where
/// [`launches_as_mirror`] reads it.
///
/// Quick start writes the follow-signatures marker on a machine that may check
/// blocks (`Backend::may_check_blocks`), so Settings can take it back later.
/// That includes a Mac, where Quick start is selected first: without the
/// marker, [`launches_as_mirror`] would try the chip at the first start.
/// Full check removes it. A machine that cannot check blocks gets no marker
/// either way: it follows signatures anyway, and a marker there would show a
/// Settings switch that changes nothing.
pub fn apply_start_choice(
    datadir: &Path,
    choice: StartChoice,
    may_check_blocks: bool,
) -> std::io::Result<()> {
    set_follows_signatures_by_choice(
        datadir,
        choice == StartChoice::QuickStart && may_check_blocks,
    )
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run (in `crates/btx-core`): `cargo test --locked --lib -- the_setup_choice the_window_names only_a_graphics only_an_nvidia the_owners_choice`
Expected: PASS, `test result: ok. 5 passed`.

- [ ] **Step 5: Format, lint, full suite**

Run (in `crates/btx-core`): `cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cargo test --locked`
Expected: no fmt diff; clippy `Finished`; `test result: ok.` with 0 failed. `node::tests::launch_watch_detects_an_immediate_child_death` is timing-based (a shell child must exit inside a 3-second window) and failed twice in the dry run, each time straight after a clippy build, then passed on every quiet re-run; if it fails, re-run it alone with `cargo test --locked --lib launch_watch_detects_an_immediate_child_death` before looking further.

- [ ] **Step 6: Commit**

```bash
git add crates/btx-core/src/backend.rs crates/btx-core/src/node.rs
git commit -m "core: the setup choice writes or removes the follow-signatures marker, and Full check comes first only on an NVIDIA card

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The role line on a signed snapshot (`btx-core`)

**Files:**
- Modify: `crates/btx-core/src/node.rs` (a new function after `apply_start_choice` from Task 2; a test after `the_window_names_the_choice`)
- Modify: `crates/btx-core/src/role.rs:174-175` (field), `:289-292` (constructor), `:309-312` (builder), `:353-356` (validation line), test after `:829-845` (all unchanged by Tools)

If #160 has merged first, two of these anchors read differently; Step 3 gives both. #160 leaves `with_distinct_signers`, `validation_line`, the test anchor and the wire-shape test as they are.

**Interfaces:**
- Consumes: `StartChoice` tests from Task 2 (only as an anchor).
- Produces:
  - `pub fn on_signed_snapshot(datadir: &Path) -> bool` in `btx_core::node`
  - `impl NodeRole { pub fn with_signed_snapshot(self, on_signed_snapshot: bool) -> Self }` in `btx_core::role`

- [ ] **Step 1: Write the failing tests**

In `crates/btx-core/src/node.rs`, find the end of `the_window_names_the_choice` (from Task 2):

```rust
        let none: Option<StartChoice> = serde_json::from_value(serde_json::Value::Null).unwrap();
        assert_eq!(none, None);
    }
```

and insert directly after it:

```rust

    /// The engine keeps a signed snapshot's manifest beside the snapshot's
    /// chain state until the background check retires it.
    #[test]
    fn a_signed_snapshot_is_read_from_the_engines_own_file() {
        let tmp = tempfile::tempdir().expect("temp datadir");
        let dir = tmp.path();
        assert!(!on_signed_snapshot(dir));
        std::fs::create_dir_all(dir.join("chainstate_snapshot")).unwrap();
        assert!(!on_signed_snapshot(dir), "an unsigned snapshot is not one");
        std::fs::write(
            dir.join("chainstate_snapshot").join("attested_assumeutxo"),
            b"x",
        )
        .unwrap();
        assert!(on_signed_snapshot(dir));
    }
```

In `crates/btx-core/src/role.rs`, find the end of `a_validating_node_is_untouched_by_the_signer_count` (lines 842-845):

```rust
        let v = line(&lines, "Validation");
        assert_eq!(v.value, "Checking every block itself");
        assert_eq!(v.helps, Some(true));
    }
```

and insert directly after it:

```rust

    /// A node that checks blocks and started from a signed snapshot says so:
    /// every new block is its own check, the history below the snapshot is not
    /// yet. Once that check is done the usual sentence is back. A mirror and a
    /// degraded start are never told they check blocks.
    #[test]
    fn a_validating_node_on_a_signed_snapshot_says_its_history_is_still_being_checked() {
        let validating = node_role(
            Some(&status("consensus", true)),
            CONSENSUS_BITS,
            &[],
            1,
            600,
            Some(0),
            None,
        );
        let on_snapshot = validating.clone().with_signed_snapshot(true).lines();
        let v = line(&on_snapshot, "Validation");
        assert_eq!(v.value, "Checking every block itself");
        assert_eq!(v.helps, Some(true));
        assert_eq!(
            v.note,
            "Checks every new block itself. Its older history is still being checked."
        );

        let done = validating.with_signed_snapshot(false).lines();
        assert!(line(&done, "Validation")
            .note
            .starts_with("Validates the proof of work on its own hardware"));

        let mirror = node_role(
            Some(&status("trusted", false)),
            ARCHIVE_ONLY_BITS,
            &[],
            0,
            600,
            Some(0),
            None,
        )
        .with_signed_snapshot(true)
        .lines();
        assert!(!line(&mirror, "Validation")
            .note
            .contains("Checks every new block"));

        let degraded = node_role(
            Some(&status("consensus", false)),
            PLAIN_BITS,
            &[],
            0,
            600,
            Some(0),
            None,
        )
        .with_signed_snapshot(true)
        .lines();
        assert_eq!(line(&degraded, "Validation").value, "Started degraded");
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run (in `crates/btx-core`): `cargo test --locked --lib signed_snapshot`
Expected: FAIL to compile with `cannot find function `on_signed_snapshot` in this scope` and `no method named `with_signed_snapshot` found for struct `role::NodeRole``.

- [ ] **Step 3: Write the implementation**

In `crates/btx-core/src/node.rs`, find the end of `apply_start_choice` (from Task 2):

```rust
    set_follows_signatures_by_choice(
        datadir,
        choice == StartChoice::QuickStart && may_check_blocks,
    )
}
```

and insert directly after it:

```rust

/// Does this node run on a signed snapshot? The engine stores the snapshot's
/// signed manifest as `chainstate_snapshot/attested_assumeutxo` and removes it
/// when the background check of the older history retires the snapshot
/// (docs/decisions/2026-09-29-every-node-starts-near-the-tip.md, section 8).
pub fn on_signed_snapshot(datadir: &Path) -> bool {
    datadir
        .join("chainstate_snapshot")
        .join("attested_assumeutxo")
        .exists()
}
```

In `crates/btx-core/src/role.rs`, find the last field of `NodeRole` (lines 174-175):

```rust
    pub distinct_signers: Option<u64>,
}
```

Replace with (the field is `#[serde(skip)]` because `the_wire_shape_is_what_the_ui_declares`, `role.rs:1453`, pins the serialised object whole):

```rust
    pub distinct_signers: Option<u64>,
    /// The node runs on a signed snapshot whose older history is still being
    /// checked. Changes what the validation line says, not its verdict: every
    /// new block is still this node's own check. Not serialised, like
    /// `archive`: `role_lines` carries the words, and the wire shape test
    /// below pins the object.
    #[serde(skip)]
    on_signed_snapshot: bool,
}
```

If #160 has merged first, the last field is `signed_frontier` instead; find

```rust
    #[serde(skip)]
    signed_frontier: Option<AttestedTip>,
}
```

and put the same doc comment, `#[serde(skip)]` and `on_signed_snapshot: bool,` between `signed_frontier: Option<AttestedTip>,` and the closing `}`.

Find the end of the `NodeRole` literal in `node_role` (lines 289-292):

```rust
        signed_recent: None,
        distinct_signers: None,
    }
}
```

Replace with:

```rust
        signed_recent: None,
        distinct_signers: None,
        on_signed_snapshot: false,
    }
}
```

If #160 has merged first, the literal ends `tip_age_secs: None,` then `signed_frontier: None,` after `distinct_signers: None,`; add `on_signed_snapshot: false,` after `signed_frontier: None,` instead.

Find `with_distinct_signers` (lines 309-312):

```rust
    pub fn with_distinct_signers(mut self, distinct_signers: Option<u64>) -> Self {
        self.distinct_signers = distinct_signers;
        self
    }
```

Replace with:

```rust
    pub fn with_distinct_signers(mut self, distinct_signers: Option<u64>) -> Self {
        self.distinct_signers = distinct_signers;
        self
    }

    /// Attach whether the node started from a signed snapshot whose older
    /// history is still being checked (`node::on_signed_snapshot` while the
    /// status carries a history check).
    pub fn with_signed_snapshot(mut self, on_signed_snapshot: bool) -> Self {
        self.on_signed_snapshot = on_signed_snapshot;
        self
    }
```

Find the start of `validation_line` (lines 353-355):

```rust
    fn validation_line(&self) -> RoleLine {
        let (value, helps, note) = match self.validation_mode {
            ValidationMode::Consensus if self.advertises_consensus => (
```

Replace with:

```rust
    fn validation_line(&self) -> RoleLine {
        let (value, helps, note) = match self.validation_mode {
            // A signed snapshot: every new block is checked here, the history
            // below the snapshot is still being checked in the background, so
            // "takes nobody's word for the chain" is not true yet.
            ValidationMode::Consensus if self.advertises_consensus && self.on_signed_snapshot => (
                "Checking every block itself",
                Some(true),
                "Checks every new block itself. Its older history is still being checked."
                    .to_string(),
            ),
            ValidationMode::Consensus if self.advertises_consensus => (
```

- [ ] **Step 4: Run the tests to see them pass**

Run (in `crates/btx-core`): `cargo test --locked --lib signed_snapshot`
Expected: PASS, `a_signed_snapshot_is_read_from_the_engines_own_file ... ok` and `a_validating_node_on_a_signed_snapshot_says_its_history_is_still_being_checked ... ok`.

- [ ] **Step 5: Format, lint, full suite**

Run (in `crates/btx-core`): `cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cargo test --locked`
Expected: no fmt diff; clippy `Finished`; `test result: ok. 624 passed; 0 failed; 2 ignored` for the library (612 before Task 1: the dry run's 548 on `b008f98` plus the 64 tests Tools added; the 12 from Tasks 1 to 3 and the wire-shape test included). If #160 has merged first: 627 and 615. These counts are derived by counting test attributes on `origin/main` (calibrated against the dry run on `b008f98`, where the same count gave 548 and 2 ignored on a Mac), not run; if yours differ only by tests you did not write, trust the run.

- [ ] **Step 6: Commit**

```bash
git add crates/btx-core/src/node.rs crates/btx-core/src/role.rs
git commit -m "core: a validating node on a signed snapshot says its older history is still being checked

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The history check and the role line reach the window (Tauri shell)

**Files:**
- Modify: `apps/node/src-tauri/src/state.rs:699` (slot), `:788` (constructor)
- Modify: `apps/node/src-tauri/src/commands.rs:1037-1038` (clear on start), `:1534` and `:1548` (refresher locals), `:1696-1697` (projection), `:2538-2539` (clear on stop), `:2745` (struct field), `:3115-3127` (role), `:3272` (struct literal)

**Interfaces:**
- Consumes: `btx_core::node_api::{HistoryCheck, refresh_history_check}` (Task 1), `btx_core::node::on_signed_snapshot` and `NodeRole::with_signed_snapshot` (Task 3).
- Produces: `AppState.history_check: Arc<Mutex<Option<btx_core::node_api::HistoryCheck>>>`; `NodeStatusInfo.history_check: Option<btx_core::node_api::HistoryCheck>` serialised as `history_check: { checked: number, base: number } | null`.

This task is wiring. The rules it wires are tested in Tasks 1 and 3; the shell has no harness for the refresher loop or `get_node_status`, so the check here is that it compiles, keeps every existing test green, and passes the lints. Task 12 checks the result on screen.

- [ ] **Step 1: Make sure the shell builds locally**

Run: `mkdir -p apps/node/src-tauri/resources/node-pkg && test -n "$(ls -A apps/node/src-tauri/resources/node-pkg)" || echo "CI placeholder. Not a node package." > apps/node/src-tauri/resources/node-pkg/CI-PLACEHOLDER`
Then (in `apps/node/src-tauri`): `cargo test --locked`
Expected: `test result: ok. 108 passed; 0 failed; 1 ignored` (85 on `b008f98` plus the 23 tests in Tools' `tools.rs`; counted, not run). The placeholder path is in `.gitignore:18` and is never committed.

- [ ] **Step 2: Add the slot**

In `apps/node/src-tauri/src/state.rs`, find (line 699):

```rust
    pub engine_warnings: Arc<Mutex<Vec<btx_core::engine_warnings::EngineWarning>>>,
```

Replace with:

```rust
    pub engine_warnings: Arc<Mutex<Vec<btx_core::engine_warnings::EngineWarning>>>,
    /// How far the background check of a snapshot's older history has got
    /// (`btx_core::node_api::refresh_history_check`), from the refresher's
    /// `getchainstates`. `None` when no check is running or its base height
    /// has not been read yet. Kept through a failed read, like
    /// `engine_warnings`; cleared on every stop/start like the others.
    pub history_check: Arc<Mutex<Option<btx_core::node_api::HistoryCheck>>>,
```

Find (line 788):

```rust
            engine_warnings: Arc::new(Mutex::new(Vec::new())),
```

Replace with:

```rust
            engine_warnings: Arc::new(Mutex::new(Vec::new())),
            history_check: Arc::new(Mutex::new(None)),
```

- [ ] **Step 3: Clear it on every start and stop**

In `apps/node/src-tauri/src/commands.rs` the block below appears twice, in `start_node_inner` (lines 1037-1038) and in `stop_node_inner` (lines 2538-2539). (#160 adds a `signed_frontier` line above each, which leaves these two lines as they are.)

```rust
    *state.tip_median_time.lock().await = None;
    state.engine_warnings.lock().await.clear();
```

Replace **both** occurrences with:

```rust
    *state.tip_median_time.lock().await = None;
    state.engine_warnings.lock().await.clear();
    *state.history_check.lock().await = None;
```

- [ ] **Step 4: Project `getchainstates` in the refresher**

In `spawn_status_refresher`, find (line 1534):

```rust
    let engine_warnings_slot = state.engine_warnings.clone();
```

Replace with:

```rust
    let engine_warnings_slot = state.engine_warnings.clone();
    let history_slot = state.history_check.clone();
```

Find (line 1548):

```rust
        let mut snapshot_swept = false;
```

Replace with:

```rust
        let mut snapshot_swept = false;
        // The snapshot block's own height, read from its header once per
        // snapshot (btx_core::node_api::refresh_history_check).
        let mut history_base: Option<(String, u64)> = None;
```

Find (lines 1696-1697):

```rust
                    let chainstates = chainstates.unwrap_or_default();
                    let peer_infos = peer_infos.ok();
```

Replace with:

```rust
                    let chainstates_read = chainstates.is_ok();
                    let chainstates = chainstates.unwrap_or_default();
                    let peer_infos = peer_infos.ok();
                    // How far the background check of the snapshot's older
                    // history has got, for the status screen's line and bar.
                    // A failed getchainstates keeps the last answer: one lost
                    // call is not the check finishing.
                    if chainstates_read {
                        *history_slot.lock().await = btx_core::node_api::refresh_history_check(
                            &rpc,
                            &chainstates,
                            &mut history_base,
                        )
                        .await;
                    }
```

- [ ] **Step 5: Ship the field and the role sentence**

In `pub struct NodeStatusInfo`, find (lines 2743-2745):

```rust
    /// Everything else btxd is warning about, one sentence each, those that
    /// ask for attention first. Empty when stopped or when there is nothing.
    pub engine_notes: Vec<btx_core::engine_warnings::EngineNote>,
```

Replace with:

```rust
    /// Everything else btxd is warning about, one sentence each, those that
    /// ask for attention first. Empty when stopped or when there is nothing.
    pub engine_notes: Vec<btx_core::engine_warnings::EngineNote>,
    /// How far the background check of the snapshot's older history has got
    /// (`btx_core::node_api::HistoryCheck`), for the status screen's
    /// "Checking older history" line and bar. `None` when the node is stopped,
    /// never loaded a snapshot, or the engine reports the check done.
    pub history_check: Option<btx_core::node_api::HistoryCheck>,
```

In `get_node_status`, find (lines 3115-3116):

```rust
    let role = net.as_ref().filter(|_| running).map(|n| {
        btx_core::role::node_role(
```

Replace with:

```rust
    let history_check = if running {
        *state.history_check.lock().await
    } else {
        None
    };
    // A validating node on a signed snapshot says its older history is still
    // being checked, for as long as the check runs (btx_core::role).
    let on_signed_snapshot =
        history_check.is_some() && btx_core::node::on_signed_snapshot(&datadir);
    let role = net.as_ref().filter(|_| running).map(|n| {
        btx_core::role::node_role(
```

Find (lines 3125-3127):

```rust
        .with_signed_recent(signed_recent)
        .with_distinct_signers(distinct_signers)
    });
```

Replace with:

```rust
        .with_signed_recent(signed_recent)
        .with_distinct_signers(distinct_signers)
        .with_signed_snapshot(on_signed_snapshot)
    });
```

If #160 has merged first: the `let role` anchor above is unchanged (it sits at about line 3141, under #160's new tip-age and signed-frontier block, and the history check goes between that block and `let role`), but the chain now ends

```rust
        .with_tip_age(tip_age_secs)
        .with_signed_frontier(signed_frontier.as_ref())
    });
```

so find those three lines and add `.with_signed_snapshot(on_signed_snapshot)` after `.with_signed_frontier(signed_frontier.as_ref())`.

In the `Ok(NodeStatusInfo { ... })` literal, find (lines 3271-3273):

```rust
        behind_signers_message,
        engine_notes,
        fork,
```

Replace with:

```rust
        behind_signers_message,
        engine_notes,
        history_check,
        fork,
```

- [ ] **Step 6: Build, test, format, lint**

Run (in `apps/node/src-tauri`): `cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cargo test --locked`
Expected: no fmt diff; clippy `Finished` without `error`; `test result: ok. 108 passed; 0 failed; 1 ignored`.

- [ ] **Step 7: Commit**

```bash
git add apps/node/src-tauri/src/state.rs apps/node/src-tauri/src/commands.rs
git commit -m "node: the refresher projects the history check, and the role line says when a signed snapshot's history is unchecked

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `begin_setup` takes the choice; the setup screen learns what the machine can do (Tauri shell)

**Files:**
- Modify: `apps/node/src-tauri/src/commands.rs:350-352` (helper after `node_backend`), `:2802` (fields), `:3299` (literal), `:3340-3343` (`begin_setup`), `:3365-3376` (`guarded_setup`), `:3392-3399` (`run_setup_pipeline`)
- Modify: `apps/node/src-tauri/src/lib.rs:120` (was 112; Tools registered its commands above it)

**Interfaces:**
- Consumes: `btx_core::node::{StartChoice, apply_start_choice, matmul_consensus_was_refused}`, `Backend::may_check_blocks` and `Backend::full_check_first` (Task 2).
- Produces: `begin_setup(choice: Option<StartChoice>)` (JS: `invoke("begin_setup", { choice: "quick_start" | "full_check" })`); `guarded_setup(app, state, choice: Option<StartChoice>)`; `NodeStatusInfo.full_check_possible: bool`, `NodeStatusInfo.full_check_first: bool`, `NodeStatusInfo.chip_refused: bool`; private `fn setup_backend() -> Backend`, `fn full_check_possible() -> bool`, `fn full_check_first() -> bool`.

No other caller of `guarded_setup` or `begin_setup` exists on `aed8755` (Tools adds none), so the two call sites below are all of them.

Wiring again: `apply_start_choice` and the serde names are tested in Task 2; Task 12 checks that the window's click reaches `begin_setup` with the choice.

- [ ] **Step 1: Ask the host once**

In `apps/node/src-tauri/src/commands.rs`, find (lines 350-352):

```rust
fn node_backend() -> Backend {
    btx_core::backend::node_host_backend()
}
```

Replace with:

```rust
fn node_backend() -> Backend {
    btx_core::backend::node_host_backend()
}

/// This machine's backend as the setup screen sees it, asked of the host once
/// per app run. `node_host_backend` reads the loader cache on a PC and logs its
/// answer, which is too much for a status poll every 1.5 s, and the hardware
/// does not change while the app is open.
fn setup_backend() -> Backend {
    static BACKEND: std::sync::OnceLock<Backend> = std::sync::OnceLock::new();
    *BACKEND.get_or_init(node_backend)
}

/// Whether this machine may check blocks itself (`Backend::may_check_blocks`):
/// whether the setup screen lets the owner pick Full check.
fn full_check_possible() -> bool {
    setup_backend().may_check_blocks()
}

/// Whether the setup screen selects Full check first
/// (`Backend::full_check_first`): on an NVIDIA machine, not on a Mac, where
/// Quick start comes first (the owner's decision of 2026-09-29).
fn full_check_first() -> bool {
    setup_backend().full_check_first()
}
```

- [ ] **Step 2: Three status fields**

In `pub struct NodeStatusInfo`, find (lines 2799-2802):

```rust
    /// The owner chose to follow signatures on a machine that could check
    /// blocks itself (`btx_core::node::follows_signatures_by_choice`). Drives
    /// the Settings switch that takes it back.
    pub follow_signatures: bool,
```

Replace with:

```rust
    /// The owner chose to follow signatures on a machine that could check
    /// blocks itself (`btx_core::node::follows_signatures_by_choice`). Drives
    /// the Settings switch that takes it back.
    pub follow_signatures: bool,
    /// Whether this machine may check blocks itself, as far as the app can
    /// know before the engine's first start (`full_check_possible`). The setup
    /// screen greys out Full check when it is false.
    pub full_check_possible: bool,
    /// Whether the setup screen selects Full check first (`full_check_first`):
    /// true on an NVIDIA machine, false on a Mac, where Quick start comes
    /// first and Full check can still be picked. Read only while
    /// `full_check_possible` is true.
    pub full_check_first: bool,
    /// The engine refused this Mac's graphics chip at a start, and the app
    /// moved the node to following signatures
    /// (`btx_core::node::matmul_consensus_was_refused`). The status screen
    /// says so once.
    pub chip_refused: bool,
```

In the `Ok(NodeStatusInfo { ... })` literal, find (line 3299):

```rust
        follow_signatures: btx_core::node::follows_signatures_by_choice(&datadir),
```

Replace with:

```rust
        follow_signatures: btx_core::node::follows_signatures_by_choice(&datadir),
        full_check_possible: full_check_possible(),
        full_check_first: full_check_first(),
        chip_refused: btx_core::node::matmul_consensus_was_refused(&datadir),
```

- [ ] **Step 3: The choice through setup**

Find (lines 3340-3343):

```rust
#[tauri::command]
pub async fn begin_setup(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    guarded_setup(&app, &state).await
}
```

Replace with:

```rust
/// `choice` is the setup screen's Quick start or Full check. `None` (an older
/// window, the E2E seam) is setup as it was before the choice existed.
#[tauri::command]
pub async fn begin_setup(
    app: AppHandle,
    state: State<'_, AppState>,
    choice: Option<btx_core::node::StartChoice>,
) -> Result<(), String> {
    guarded_setup(&app, &state, choice).await
}
```

Find (lines 3365-3368):

```rust
pub(crate) async fn guarded_setup(
    app: &AppHandle,
    state: &State<'_, AppState>,
) -> Result<(), String> {
```

Replace with:

```rust
pub(crate) async fn guarded_setup(
    app: &AppHandle,
    state: &State<'_, AppState>,
    choice: Option<btx_core::node::StartChoice>,
) -> Result<(), String> {
```

Find (line 3376):

```rust
    let result = run_setup_pipeline(app, state).await;
```

Replace with:

```rust
    let result = run_setup_pipeline(app, state, choice).await;
```

Find (lines 3392-3399):

```rust
async fn run_setup_pipeline(app: &AppHandle, state: &State<'_, AppState>) -> Result<(), String> {
    let datadir = node_datadir();
    std::fs::create_dir_all(&datadir)
        .map_err(|e| format!("couldn't create {}: {e}", datadir.display()))?;
    setup_log(
        &datadir,
        &format!("setup started (app v{})", env!("CARGO_PKG_VERSION")),
    );
```

Replace with:

```rust
async fn run_setup_pipeline(
    app: &AppHandle,
    state: &State<'_, AppState>,
    choice: Option<btx_core::node::StartChoice>,
) -> Result<(), String> {
    let datadir = node_datadir();
    std::fs::create_dir_all(&datadir)
        .map_err(|e| format!("couldn't create {}: {e}", datadir.display()))?;
    setup_log(
        &datadir,
        &format!("setup started (app v{})", env!("CARGO_PKG_VERSION")),
    );

    // 0. The owner's pick on the setup screen, recorded before the first start
    //    reads it (`launches_as_mirror`). No pick leaves the marker as it is.
    if let Some(choice) = choice {
        btx_core::node::apply_start_choice(&datadir, choice, full_check_possible()).map_err(
            |e| {
                format!(
                    "couldn't record your start choice in {}: {e}",
                    datadir.display()
                )
            },
        )?;
        setup_log(&datadir, &format!("start choice: {choice:?}"));
    }
```

In `apps/node/src-tauri/src/lib.rs`, find (line 120):

```rust
                    if let Err(message) = commands::guarded_setup(&handle, &state).await {
```

Replace with:

```rust
                    if let Err(message) = commands::guarded_setup(&handle, &state, None).await {
```

- [ ] **Step 4: Build, test, format, lint**

Run (in `apps/node/src-tauri`): `cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cargo test --locked`
Expected: no fmt diff; clippy `Finished` without `error`; `test result: ok. 108 passed; 0 failed; 1 ignored`.

- [ ] **Step 5: Commit**

```bash
git add apps/node/src-tauri/src/commands.rs apps/node/src-tauri/src/lib.rs
git commit -m "node: begin_setup records Quick start or Full check before the first start, and the status says which one this machine selects first

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: The stale card, pure (`catchup-trend.ts`)

**Files:**
- Modify: `apps/node/src/catchup-trend.ts:166-187` (`catchupLine`), before `:189` (new exports)
- Test: `apps/node/src/catchup-trend.test.ts:1-19` (imports), after `:350` (new tests)

**Interfaces:**
- Consumes: `judge`, `CatchupPace`, `CatchupSample`, `CHAIN_BLOCKS_PER_HOUR` (existing, same file).
- Produces:
  - `export const paceSentence = (pace: CatchupPace | null): string | null`
  - `export const CHECKING_CATCHUP = "Checking whether your node is catching up..."`
  - `export type StaleCard = { message: string; tone: "amber" | "neutral" }`
  - `export const staleCard = (tipStaleMessage: string | null, behind: number | null, samples: CatchupSample[], now: number): StaleCard | null`

- [ ] **Step 1: Write the failing tests**

In `apps/node/src/catchup-trend.test.ts`, find the import (lines 12-19):

```ts
  TOO_SLOW_FOR_THE_CHAIN_PER_HOUR,
  cannotCatchUp,
  catchupLine,
  catchupPace,
  catchupTrend,
  pushSample,
  timeToGo,
} from "./catchup-trend";
```

Replace with:

```ts
  TOO_SLOW_FOR_THE_CHAIN_PER_HOUR,
  CHECKING_CATCHUP,
  cannotCatchUp,
  catchupLine,
  catchupPace,
  catchupTrend,
  paceSentence,
  pushSample,
  staleCard,
  timeToGo,
} from "./catchup-trend";
```

Append to the end of the file (after line 350):

```ts

// The chain card's stale sentence (docs/decisions/2026-09-29-quick-start-full-
// check-and-progress.md, section 3). btx2 and btx3 on 28 September read "The
// node is not following the chain" under "about 5 days to go".
const STALE =
  "The newest block this node has is 200 hours old. That is measured against the clock, not " +
  "against what its peers report, so it holds even when every peer agrees with it. The node is " +
  "not following the chain.";

// A node from 219,000 closing its gap at about 94 blocks an hour.
const closingNode = (): CatchupSample[] => [
  { at: T0, behind: 10_800, height: 219_000 },
  { at: T0 + min(10), behind: 10_769, height: 219_038 },
  { at: T0 + min(20), behind: 10_737, height: 219_076 },
];

describe("staleCard", () => {
  it("says nothing when the tip is not stale", () => {
    expect(staleCard(null, 10_812, slowNode(), T0 + min(30))).toBeNull();
  });

  it("row 1: behind and closing, no stale sentence at all", () => {
    expect(staleCard(STALE, 10_737, closingNode(), T0 + min(20))).toBeNull();
  });

  it("row 2: behind before the trend is measured, neutral and not amber", () => {
    expect(staleCard(STALE, 7_500, [], T0)).toEqual({ message: CHECKING_CATCHUP, tone: "neutral" });
    const firstMinutes = [
      { at: T0, behind: 7_500, height: 225_927 },
      { at: T0 + min(3), behind: 7_420, height: 226_010 },
    ];
    expect(staleCard(STALE, 7_420, firstMinutes, T0 + min(3))).toEqual({
      message: "Checking whether your node is catching up...",
      tone: "neutral",
    });
  });

  it("row 3: behind and not closing, amber with the pace", () => {
    expect(staleCard(STALE, 10_812, slowNode(), T0 + min(30))).toEqual({
      message: `${STALE} It adds about 8 blocks an hour while the network adds about ${CHAIN_BLOCKS_PER_HOUR}.`,
      tone: "amber",
    });
  });

  it("row 4: no newer block known and the newest is old, today's sentence unchanged", () => {
    expect(staleCard(STALE, 0, [], T0)).toEqual({ message: STALE, tone: "amber" });
    // A closing history does not soften it: with nothing newer known, an old
    // tip is the real "not following the chain".
    expect(staleCard(STALE, 0, closingNode(), T0 + min(20))).toEqual({ message: STALE, tone: "amber" });
    expect(staleCard(STALE, null, [], T0)).toEqual({ message: STALE, tone: "amber" });
  });

  it("turns amber when a closing gap stops closing", () => {
    const s = closingNode();
    expect(staleCard(STALE, 10_737, s, T0 + min(20))).toBeNull();
    s.push({ at: T0 + min(26), behind: 10_738, height: 219_081 });
    s.push({ at: T0 + min(31), behind: 10_739, height: 219_086 });
    // The last ten minutes closed nothing. Over the hour it still added more
    // than the chain, so the pace is left out rather than contradict the card.
    expect(staleCard(STALE, 10_739, s, T0 + min(31))).toEqual({ message: STALE, tone: "amber" });
  });

  it("starts over after a restart", () => {
    expect(staleCard(STALE, 10_812, slowNode(), T0 + min(30))?.tone).toBe("amber");
    // The window clears its samples on a stop or a start; the first reading
    // after it cannot judge anything yet.
    const after = pushSample([], { at: T0 + min(33), behind: 10_815, height: 219_005 }, T0 + min(33));
    expect(staleCard(STALE, 10_815, after, T0 + min(33))).toEqual({
      message: CHECKING_CATCHUP,
      tone: "neutral",
    });
  });
});

describe("paceSentence", () => {
  it("is what the catch-up line and the stale card both say", () => {
    const pace = catchupPace(slowNode(), T0 + min(30));
    expect(paceSentence(pace)).toBe(
      `It adds about 8 blocks an hour while the network adds about ${CHAIN_BLOCKS_PER_HOUR}.`,
    );
    expect(catchupLine(10_812, slowNode(), T0 + min(30))).toBe(
      `Your node is live but not catching up: 10,812 blocks behind. ${paceSentence(pace)}`,
    );
    expect(paceSentence(null)).toBeNull();
    expect(paceSentence({ addedPerHour: 50, closingPerHour: 0, spanMs: min(60) })).toBeNull();
    expect(paceSentence({ addedPerHour: 0, closingPerHour: -40, spanMs: min(20) })).toBe(
      "It has added no blocks in the last 20 minutes.",
    );
  });
});
```

- [ ] **Step 2: Run the tests to see them fail**

Run (in `apps/node`): `npx vitest run src/catchup-trend.test.ts`
Expected: FAIL, `8 failed | 30 passed (38)`, with `TypeError: (0 , staleCard) is not a function`.

- [ ] **Step 3: Write the implementation**

In `apps/node/src/catchup-trend.ts`, find the start of `catchupLine` (lines 166-182):

```ts
export const catchupLine = (behind: number, samples: CatchupSample[], now: number): string => {
  const n = behind.toLocaleString("en-US");
  const { trend, pace } = judge(samples, now);
  if (trend === "stalled") {
    const plain = `Your node is live but not catching up. It is ${n} blocks behind and the gap is not closing.`;
    if (!pace) return plain;
    const added = Math.round(pace.addedPerHour);
    if (added > CHAIN_BLOCKS_PER_HOUR) return plain;
    const head = `Your node is live but not catching up: ${n} blocks behind.`;
    if (added < 1) {
      return `${head} It has added no blocks in the last ${Math.round(pace.spanMs / 60_000)} minutes.`;
    }
    return (
      `${head} It adds about ${added} block${added === 1 ? "" : "s"} an hour ` +
      `while the network adds about ${CHAIN_BLOCKS_PER_HOUR}.`
    );
  }
```

Replace with (the catch-up line's words do not change; its existing tests guard that):

```ts
/** The pace of a node that is not catching up, in one sentence: what it adds
 *  an hour against what the chain adds, or that it added nothing. Null before
 *  the pace is measured, and when the hour's pace is faster than the chain's,
 *  which under "not catching up" would read as a bug. The catch-up line and
 *  the stale card both say it, so they can never disagree. */
export const paceSentence = (pace: CatchupPace | null): string | null => {
  if (!pace) return null;
  const added = Math.round(pace.addedPerHour);
  if (added > CHAIN_BLOCKS_PER_HOUR) return null;
  if (added < 1) return `It has added no blocks in the last ${Math.round(pace.spanMs / 60_000)} minutes.`;
  return (
    `It adds about ${added} block${added === 1 ? "" : "s"} an hour ` +
    `while the network adds about ${CHAIN_BLOCKS_PER_HOUR}.`
  );
};

export const catchupLine = (behind: number, samples: CatchupSample[], now: number): string => {
  const n = behind.toLocaleString("en-US");
  const { trend, pace } = judge(samples, now);
  if (trend === "stalled") {
    const said = paceSentence(pace);
    return said
      ? `Your node is live but not catching up: ${n} blocks behind. ${said}`
      : `Your node is live but not catching up. It is ${n} blocks behind and the gap is not closing.`;
  }
```

Then find (line 189):

```ts
/** Keep the sample list bounded and in order. The caller holds it across
```

Replace with:

```ts
/** The neutral line for a node that is behind before its trend is measured. */
export const CHECKING_CATCHUP = "Checking whether your node is catching up...";

/** What the chain card says about an old tip. `amber` is a warning; `neutral`
 *  is said in the card's quiet colours. */
export type StaleCard = { message: string; tone: "amber" | "neutral" };

/** The chain card's stale sentence, from the trend and from what the node
 *  knows (docs/decisions/2026-09-29-quick-start-full-check-and-progress.md,
 *  section 3).
 *
 *  `tipStaleMessage` is Rust's sentence, present only when the newest block is
 *  more than 2 hours old by the clock. `behind` is how many blocks the node
 *  knows of beyond its tip (headers minus blocks), or null when the phase
 *  carries no height. On 28 September btx2 and btx3 read "The node is not
 *  following the chain" under "about 5 days to go": a node closing a gap of
 *  days has an old tip by construction, and that sentence is for a node that
 *  knows of nothing newer.
 *
 *  - behind, gap closing: no stale sentence; the catch-up line says it all.
 *  - behind, trend not measured yet: neutral, "Checking whether your node is
 *    catching up...".
 *  - behind, gap not closing: amber, Rust's sentence and the pace.
 *  - no newer block known: amber, Rust's sentence unchanged. */
export const staleCard = (
  tipStaleMessage: string | null,
  behind: number | null,
  samples: CatchupSample[],
  now: number,
): StaleCard | null => {
  if (!tipStaleMessage) return null;
  if (behind === null || !(behind > 0)) return { message: tipStaleMessage, tone: "amber" };
  const { trend, pace } = judge(samples, now);
  if (trend === "converging") return null;
  if (trend === "unknown") return { message: CHECKING_CATCHUP, tone: "neutral" };
  const said = paceSentence(pace);
  return { message: said ? `${tipStaleMessage} ${said}` : tipStaleMessage, tone: "amber" };
};

/** Keep the sample list bounded and in order. The caller holds it across
```

- [ ] **Step 4: Run the tests to see them pass**

Run (in `apps/node`): `npx vitest run src/catchup-trend.test.ts && npx tsc --noEmit`
Expected: `Tests  38 passed (38)`; tsc prints nothing.

- [ ] **Step 5: Commit**

```bash
git add apps/node/src/catchup-trend.ts apps/node/src/catchup-trend.test.ts
git commit -m "node: the stale card stays calm while the gap closes, and says the pace when it does not

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: The history line, pure (`history-check.ts`)

**Files:**
- Create: `apps/node/src/history-check.ts`
- Test: `apps/node/src/history-check.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `export type HistoryCheck = { checked: number; base: number }`; `export type HistoryCheckView = { line: string; pct: number }`; `export function historyCheckView(h: HistoryCheck | null): HistoryCheckView | null`.

- [ ] **Step 1: Write the failing test**

Create `apps/node/src/history-check.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { historyCheckView } from "./history-check";

describe("historyCheckView", () => {
  it("reads as the decision wrote it", () => {
    expect(historyCheckView({ checked: 131_200, base: 225_927 })).toEqual({
      line: "Checking older history: 131,200 of 225,927 (58%)",
      pct: 58,
    });
  });

  it("rounds down, so the bar never reads 100% before the engine says done", () => {
    expect(historyCheckView({ checked: 225_926, base: 225_927 })?.pct).toBe(99);
    expect(historyCheckView({ checked: 0, base: 225_927 })?.line).toBe(
      "Checking older history: 0 of 225,927 (0%)",
    );
  });

  it("never counts past the base", () => {
    expect(historyCheckView({ checked: 230_000, base: 225_927 })).toEqual({
      line: "Checking older history: 225,927 of 225,927 (100%)",
      pct: 100,
    });
  });

  it("shows nothing without a check", () => {
    expect(historyCheckView(null)).toBeNull();
    expect(historyCheckView({ checked: 5, base: 0 })).toBeNull();
    expect(historyCheckView({ checked: Number.NaN, base: 225_927 })).toBeNull();
  });
});
```

- [ ] **Step 2: Run it to see it fail**

Run (in `apps/node`): `npx vitest run src/history-check.test.ts`
Expected: FAIL, `Error: Cannot find module './history-check'`.

- [ ] **Step 3: Write the implementation**

Create `apps/node/src/history-check.ts`:

```ts
/**
 * The background check of a snapshot's older history, as one line and a thin
 * bar on the status screen (docs/decisions/2026-09-29-quick-start-full-check-
 * and-progress.md, section 2).
 *
 * A node that starts from a snapshot checks every new block from there on,
 * and re-checks the history below the snapshot in the background, from block
 * 0 up to the snapshot's own height. Until this line, only a wallet caveat
 * said so. No time estimate on purpose: the engine paces the check, and its
 * speed says nothing reliable yet.
 */

/** `NodeStatusInfo.history_check` (btx_core::node_api::HistoryCheck). */
export type HistoryCheck = { checked: number; base: number };

export type HistoryCheckView = { line: string; pct: number };

/** "Checking older history: 131,200 of 225,927 (58%)" and the bar's width.
 *  Rounded down, so the bar never reads 100% before the engine reports the
 *  check done, which is when the field goes away. */
export function historyCheckView(h: HistoryCheck | null): HistoryCheckView | null {
  if (!h || !(h.base > 0) || !Number.isFinite(h.checked)) return null;
  const checked = Math.min(Math.max(0, h.checked), h.base);
  const pct = Math.floor((checked / h.base) * 100);
  const fmt = (n: number) => n.toLocaleString("en-US");
  return { line: `Checking older history: ${fmt(checked)} of ${fmt(h.base)} (${pct}%)`, pct };
}
```

- [ ] **Step 4: Run it to see it pass**

Run (in `apps/node`): `npx vitest run src/history-check.test.ts && npx tsc --noEmit`
Expected: `Tests  4 passed (4)`; tsc prints nothing.

- [ ] **Step 5: Commit**

```bash
git add apps/node/src/history-check.ts apps/node/src/history-check.test.ts
git commit -m "node: the words and the bar for the history check

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: The setup screen's rules and the chip notice rule, pure (`start-choice.ts`)

**Files:**
- Create: `apps/node/src/start-choice.ts`
- Test: `apps/node/src/start-choice.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `export type StartChoice = "quick_start" | "full_check"`; `export const NO_GPU_REASON: string`; `export type StartChoiceView = { selected: StartChoice; fullCheckDisabled: boolean }`; `export function startChoiceView(fullCheckPossible: boolean, fullCheckFirst: boolean, picked: StartChoice | null): StartChoiceView` (the two booleans are `NodeStatusInfo.full_check_possible` and `.full_check_first`, Task 5); `export function setupArgs(choice: StartChoice): { choice: StartChoice }`; `export function chipNoticeVisible(chipRefused: boolean, followsSignatures: boolean, seenForTag: string | null, tag: string): boolean`.

- [ ] **Step 1: Write the failing test**

Create `apps/node/src/start-choice.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { NO_GPU_REASON, chipNoticeVisible, setupArgs, startChoiceView } from "./start-choice";

describe("startChoiceView", () => {
  it("selects Full check first on an NVIDIA machine", () => {
    expect(startChoiceView(true, true, null)).toEqual({ selected: "full_check", fullCheckDisabled: false });
  });

  it("selects Quick start first on a Mac, and Full check can still be picked", () => {
    expect(startChoiceView(true, false, null)).toEqual({ selected: "quick_start", fullCheckDisabled: false });
    expect(startChoiceView(true, false, "full_check")).toEqual({
      selected: "full_check",
      fullCheckDisabled: false,
    });
  });

  it("keeps the owner's pick across polls", () => {
    expect(startChoiceView(true, true, "quick_start")).toEqual({
      selected: "quick_start",
      fullCheckDisabled: false,
    });
    expect(startChoiceView(true, true, "full_check").selected).toBe("full_check");
  });

  it("does not let a machine with no usable GPU pick Full check", () => {
    expect(startChoiceView(false, false, null)).toEqual({ selected: "quick_start", fullCheckDisabled: true });
    expect(startChoiceView(false, false, "full_check")).toEqual({
      selected: "quick_start",
      fullCheckDisabled: true,
    });
    // Rust never says "first" without "possible", but if it did, greyed out wins.
    expect(startChoiceView(false, true, null)).toEqual({ selected: "quick_start", fullCheckDisabled: true });
    expect(NO_GPU_REASON).toBe(
      "This computer has no graphics card the BTX engine can check blocks with.",
    );
  });
});

describe("setupArgs", () => {
  it("hands begin_setup the choice under the names the Rust side reads", () => {
    expect(setupArgs("quick_start")).toEqual({ choice: "quick_start" });
    expect(setupArgs("full_check")).toEqual({ choice: "full_check" });
  });
});

describe("chipNoticeVisible", () => {
  it("shows once per engine after the chip is refused and the node follows signatures", () => {
    expect(chipNoticeVisible(true, true, null, "v0.34.9")).toBe(true);
    expect(chipNoticeVisible(true, true, "v0.34.9", "v0.34.9")).toBe(false);
    // A newer engine tries the chip again; refused again, it says so again.
    expect(chipNoticeVisible(true, true, "v0.34.9", "v0.34.12")).toBe(true);
  });

  it("says nothing it cannot stand behind", () => {
    expect(chipNoticeVisible(false, true, null, "v0.34.9")).toBe(false);
    // Refused, but not (yet) following signatures: "Nothing for you to do"
    // would not be true.
    expect(chipNoticeVisible(true, false, null, "v0.34.9")).toBe(false);
  });
});
```

- [ ] **Step 2: Run it to see it fail**

Run (in `apps/node`): `npx vitest run src/start-choice.test.ts`
Expected: FAIL, `Error: Cannot find module './start-choice'`.

- [ ] **Step 3: Write the implementation**

Create `apps/node/src/start-choice.ts`:

```ts
/**
 * The setup screen's one question, and the notice when the engine turns a
 * Mac's chip down (docs/decisions/2026-09-29-quick-start-full-check-and-
 * progress.md, section 1). Pure, so the rules are tested without a window.
 */

/** The names `begin_setup` reads (btx_core::node::StartChoice). */
export type StartChoice = "quick_start" | "full_check";

/** Shown under a greyed-out Full check. */
export const NO_GPU_REASON = "This computer has no graphics card the BTX engine can check blocks with.";

export type StartChoiceView = { selected: StartChoice; fullCheckDisabled: boolean };

/** Which choice is selected, and whether Full check can be picked. Both
 *  answers come from Rust (`full_check_possible`, `full_check_first`); the
 *  owner's rule of 2026-09-29 behind them:
 *  - an NVIDIA machine: Full check selected first;
 *  - a Mac: Quick start selected first, Full check still there to pick;
 *  - a machine that cannot check blocks: Quick start, Full check greyed out.
 *  The owner's own pick, once made, holds across polls. */
export function startChoiceView(
  fullCheckPossible: boolean,
  fullCheckFirst: boolean,
  picked: StartChoice | null,
): StartChoiceView {
  if (!fullCheckPossible) return { selected: "quick_start", fullCheckDisabled: true };
  const first: StartChoice = fullCheckFirst ? "full_check" : "quick_start";
  return { selected: picked ?? first, fullCheckDisabled: false };
}

/** The arguments `begin_setup` takes. */
export function setupArgs(choice: StartChoice): { choice: StartChoice } {
  return { choice };
}

/** Whether the status screen shows "This Mac's graphics chip didn't pass the
 *  engine's check, so your node follows signatures. Nothing for you to do."
 *
 *  Only while it is true: the chip was refused and the node follows
 *  signatures. Once per engine: `seenForTag` is the engine tag the owner
 *  dismissed it for, and a newer engine clears the refusal and tries the chip
 *  again, so a second refusal is news. */
export function chipNoticeVisible(
  chipRefused: boolean,
  followsSignatures: boolean,
  seenForTag: string | null,
  tag: string,
): boolean {
  return chipRefused && followsSignatures && seenForTag !== tag;
}
```

- [ ] **Step 4: Run it to see it pass**

Run (in `apps/node`): `npx vitest run src/start-choice.test.ts && npx tsc --noEmit`
Expected: `Tests  7 passed (7)`; tsc prints nothing.

- [ ] **Step 5: Commit**

```bash
git add apps/node/src/start-choice.ts apps/node/src/start-choice.test.ts
git commit -m "node: the rules for Quick start, Full check and the refused-chip notice

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: The wizard asks one question (window)

**Files:**
- Modify: `apps/node/index.html:89-100` (inside the wizard card, before the setup button; six lines lower than on `b008f98`, below Tools' header button)
- Modify: `apps/node/src/styles.css:211-215` (after `.wizard-note`)
- Modify: `apps/node/src/main.ts:14-20` (imports), `:210` (type), before `:518` (`renderWizard`), `:529-530`, `:1150-1151`, `:1162` (each one line lower than on `b008f98`, below Tools' `import { initTools }`)

**Interfaces:**
- Consumes: `startChoiceView`, `setupArgs`, `NO_GPU_REASON`, `StartChoice` (Task 8); `NodeStatusInfo.full_check_possible` and `.full_check_first` (Task 5); `begin_setup(choice)` (Task 5).
- Produces: DOM ids `start-choice`, `choice-quick`, `choice-quick-label`, `choice-full`, `choice-full-label`, `choice-full-reason`; `main.ts` functions `reflectStartChoice(status, inProgress)` and `currentChoice()`.

The window has no DOM test harness (`vitest.config.ts`: Node environment, pure logic only). The rules are tested in Task 8; this task is checked by the typecheck, the bundle, and Task 12.

- [ ] **Step 1: The markup**

In `apps/node/index.html`, inside `<div class="card wizard-card">`, find (lines 99-100):

```html
          </div>
          <button id="setup-btn" class="btn-primary" type="button">
```

Replace with:

```html
          </div>
          <!-- The one question (docs/decisions/2026-09-29-quick-start-full-
               check-and-progress.md). main.ts selects Full check first on an
               NVIDIA machine and Quick start first on a Mac (the owner's
               decision of 2026-09-29), and greys Full check out where the
               machine cannot check blocks (start-choice.ts); begin_setup
               records the pick before the first start. -->
          <fieldset class="start-choice" id="start-choice" aria-label="How your node starts">
            <label class="choice" id="choice-quick-label">
              <input type="radio" name="start-choice" value="quick_start" id="choice-quick" />
              <span class="choice-text">
                <span class="choice-name">Quick start</span>
                <span class="choice-desc">Follows signatures, ready in minutes.</span>
              </span>
            </label>
            <label class="choice" id="choice-full-label">
              <input type="radio" name="start-choice" value="full_check" id="choice-full" />
              <span class="choice-text">
                <span class="choice-name">Full check</span>
                <span class="choice-desc">Validates every block, takes longer the first time.</span>
                <span class="choice-reason" id="choice-full-reason" hidden></span>
              </span>
            </label>
            <!-- The owner's wording until a second operator is on the list
                 (2026-09-29); then "confirmed by two node operators". -->
            <p class="wizard-note start-choice-note">
              Both start from a recent signed snapshot and check the older
              history in the background. Full check also checks every new
              block on this computer's graphics card. You can switch later in
              Settings.
            </p>
          </fieldset>
          <button id="setup-btn" class="btn-primary" type="button">
```

- [ ] **Step 2: The styles**

In `apps/node/src/styles.css`, find (lines 211-215):

```css
.wizard-note {
  font-size: 12px;
  color: var(--color-muted);
  text-align: center;
}
```

Replace with:

```css
.wizard-note {
  font-size: 12px;
  color: var(--color-muted);
  text-align: center;
}
/* The setup screen's one question: two choices, one line under them. */
.start-choice {
  border: none;
  margin: 0;
  padding: 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
}
.choice {
  display: flex;
  align-items: flex-start;
  gap: var(--space-s);
  padding: var(--space-s);
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  cursor: pointer;
}
.choice.is-selected { border-color: var(--color-accent); }
.choice.is-disabled { cursor: default; opacity: 0.55; }
.choice input { margin-top: 4px; accent-color: var(--color-accent); }
.choice-text { display: flex; flex-direction: column; gap: 2px; }
.choice-name { font-size: 14px; font-weight: 700; }
.choice-desc,
.choice-reason { font-size: 12.5px; color: var(--color-muted); }
.start-choice-note { text-align: left; margin-top: var(--space-xs); }
```

(The selected border is a class set by `main.ts`, not `:has()`, which older WebKitGTK builds on Linux lack.)

- [ ] **Step 3: The imports and the type**

In `apps/node/src/main.ts`, find (line 20):

```ts
} from "./catchup-trend";
```

Replace with:

```ts
} from "./catchup-trend";
import { NO_GPU_REASON, type StartChoice, setupArgs, startChoiceView } from "./start-choice";
```

Find (lines 209-210):

```ts
  /** The owner chose to follow signatures on a machine that could validate. */
  follow_signatures: boolean;
```

Replace with:

```ts
  /** The owner chose to follow signatures on a machine that could validate. */
  follow_signatures: boolean;
  /** This machine may check blocks itself, as far as the app can know before
   *  the engine's first start. The setup screen greys out Full check when not. */
  full_check_possible: boolean;
  /** The setup screen selects Full check first: an NVIDIA machine. False on a
   *  Mac, where Quick start comes first and Full check can still be picked. */
  full_check_first: boolean;
```

- [ ] **Step 4: Reflect the choice**

Find (line 518):

```ts
function renderWizard(status: NodeStatusInfo) {
```

Replace with:

```ts
/** The owner's pick on the setup screen, null until they touch it, so the
 *  default can follow what the machine can do (start-choice.ts). */
let pickedChoice: StartChoice | null = null;

function reflectStartChoice(status: NodeStatusInfo, inProgress: boolean): void {
  const view = startChoiceView(status.full_check_possible, status.full_check_first, pickedChoice);
  const quick = $<HTMLInputElement>("choice-quick");
  const full = $<HTMLInputElement>("choice-full");
  quick.checked = view.selected === "quick_start";
  full.checked = view.selected === "full_check";
  // No changing course once setup has started; the choice is already recorded.
  quick.disabled = inProgress;
  full.disabled = inProgress || view.fullCheckDisabled;
  $("choice-quick-label").classList.toggle("is-selected", quick.checked);
  $("choice-full-label").classList.toggle("is-selected", full.checked);
  $("choice-full-label").classList.toggle("is-disabled", view.fullCheckDisabled);
  const reason = $("choice-full-reason");
  reason.hidden = !view.fullCheckDisabled;
  reason.textContent = view.fullCheckDisabled ? NO_GPU_REASON : "";
}

/** What the setup button sends: the owner's pick, or the default for this
 *  machine. */
function currentChoice(): StartChoice {
  return startChoiceView(
    lastStatus?.full_check_possible ?? false,
    lastStatus?.full_check_first ?? false,
    pickedChoice,
  ).selected;
}

for (const id of ["choice-quick", "choice-full"]) {
  $<HTMLInputElement>(id).addEventListener("change", (e) => {
    const box = e.target as HTMLInputElement;
    if (box.checked) pickedChoice = box.value as StartChoice;
    if (lastStatus) reflectStartChoice(lastStatus, setupInFlight);
  });
}

function renderWizard(status: NodeStatusInfo) {
```

Find (lines 529-530, in `renderWizard`):

```ts
  // The button IS the live readout while setting up; idle otherwise.
  if (inProgress) setSetupButton(true, setupPhaseLabel(p));
```

Replace with:

```ts
  reflectStartChoice(status, inProgress);

  // The button IS the live readout while setting up; idle otherwise.
  if (inProgress) setSetupButton(true, setupPhaseLabel(p));
```

- [ ] **Step 5: Send it**

In `beginSetup`, find (lines 1150-1151):

```ts
  setSetupButton(true, "Setting up your node…");
  $<HTMLButtonElement>("retry-btn").disabled = true;
```

Replace with:

```ts
  setSetupButton(true, "Setting up your node…");
  if (lastStatus) reflectStartChoice(lastStatus, true);
  $<HTMLButtonElement>("retry-btn").disabled = true;
```

Find (line 1162):

```ts
    await invoke("begin_setup");
```

Replace with:

```ts
    await invoke("begin_setup", setupArgs(currentChoice()));
```

- [ ] **Step 6: Typecheck, test, build**

Run (in `apps/node`): `npx tsc --noEmit && npm test && npx vite build`
Expected: tsc prints nothing; `Test Files  10 passed (10)`, `Tests  147 passed (147)` (128 in 8 files on `aed8755`, Tools' `tools-history.test.ts` among them, plus 8, 4 and 7 from Tasks 6 to 8; counted, not run); vite ends with `✓ built in`.

- [ ] **Step 7: Commit**

```bash
git add apps/node/index.html apps/node/src/styles.css apps/node/src/main.ts
git commit -m "node: the setup screen asks Quick start or Full check

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: The status screen: the history line, the calm stale card, the chip notice (window)

**Files:**
- Modify: `apps/node/index.html:151` (after the status line), `:186-188` (after the engine card); six lines lower than on `b008f98`
- Modify: `apps/node/src/styles.css:393-398` (after `.status-sub`), `:540` (after `.follow-card button`)
- Modify: `apps/node/src/main.ts` (imports from Task 9, the type from Task 9, `renderStatus` at `:926-928` and `:967`, `reflectFork` at `:1671-1690`; each one line lower than on `b008f98`)

**Interfaces:**
- Consumes: `staleCard` (Task 6), `historyCheckView`, `HistoryCheck` (Task 7), `chipNoticeVisible` (Task 8), `NodeStatusInfo.history_check` (Task 4) and `.chip_refused` (Task 5), existing `catchupSamples`, `lastStatus`, `$`.
- Produces: DOM ids `history-check`, `history-line`, `history-fill`, `chip-card`, `chip-ok`; `main.ts` functions `reflectHistoryCheck(status)`, `reflectChipNotice(status)`; `localStorage` key `ebtx-node.chip-notice-seen` (value: the engine tag).

Tools reads the chain card as shown: `windowLines` in `apps/node/src/tools.ts:39-45` puts `#fork-msg` into Copy diagnostics only while `#fork-card` is not hidden, and `statusLine` (`:32-37`, shown at the top of Tools and in the report) reads `#status-badge` and `#status-sub`. Both keep working unchanged. After this task, Copy diagnostics carries the calm "Checking whether your node is catching up..." line in the first minutes and no stale sentence while the gap closes, which is what the window says; the history line sits beside `#status-sub`, not inside it, so it stays out of Tools' status line. Nothing in `tools.ts` needs changing.

- [ ] **Step 1: The markup**

In `apps/node/index.html`, find (lines 151-152):

```html
          <div class="status-sub" id="status-sub"></div>
        </div>
```

Replace with:

```html
          <div class="status-sub" id="status-sub"></div>
          <!-- The background check of the snapshot's older history
               (NodeStatusInfo.history_check, history-check.ts). Gone once the
               engine reports it done. No time estimate on purpose. -->
          <div class="history-check" id="history-check" hidden>
            <div class="history-line" id="history-line"></div>
            <div class="progress-track history-track"><div class="progress-fill" id="history-fill"></div></div>
          </div>
        </div>
```

Find (lines 186-188):

```html
        <div class="card fork-card engine-card" id="engine-card" hidden>
          <div id="engine-notes"></div>
        </div>
```

Replace with:

```html
        <div class="card fork-card engine-card" id="engine-card" hidden>
          <div id="engine-notes"></div>
        </div>

        <!-- Once per engine, after the engine refused this Mac's graphics
             chip and the app moved the node to following signatures
             (start-choice.ts chipNoticeVisible). OK remembers it. -->
        <div class="card chip-card" id="chip-card" hidden>
          <p>This Mac's graphics chip didn't pass the engine's check, so your node follows signatures. Nothing for you to do.</p>
          <button id="chip-ok" class="btn-secondary" type="button">OK</button>
        </div>
```

- [ ] **Step 2: The styles**

In `apps/node/src/styles.css`, find (lines 393-398):

```css
.status-sub {
  font-size: 12.5px;
  color: var(--color-muted);
  min-height: 20px;
  text-align: center;
}
```

Replace with:

```css
.status-sub {
  font-size: 12.5px;
  color: var(--color-muted);
  min-height: 20px;
  text-align: center;
}
/* The history check under the status line: one line and a thin bar. */
.history-check {
  width: 100%;
  max-width: 340px;
  display: flex;
  flex-direction: column;
  align-items: center;
}
.history-line { font-size: 12px; color: var(--color-muted); text-align: center; }
.history-track { width: 100%; height: 4px; margin-top: var(--space-xs); }
```

Find (line 540):

```css
.follow-card button { margin-top: var(--space-s); }
```

Replace with:

```css
.follow-card button { margin-top: var(--space-s); }
/* The stale card before the catch-up trend is measured: "Checking whether
   your node is catching up...", said quietly, never as an alarm. */
#fork-card.is-calm { border-color: var(--color-border); }
#fork-card.is-calm p { color: var(--color-muted); }
/* The one-time note after the engine turned this Mac's chip down. */
.chip-card { font-size: 13px; }
.chip-card p { margin: 0; line-height: 1.45; }
.chip-card button { margin-top: var(--space-s); }
```

- [ ] **Step 3: The imports and the types**

In `apps/node/src/main.ts`, find (after Task 9):

```ts
  pushSample,
} from "./catchup-trend";
import { NO_GPU_REASON, type StartChoice, setupArgs, startChoiceView } from "./start-choice";
```

Replace with:

```ts
  pushSample,
  staleCard,
} from "./catchup-trend";
import { type HistoryCheck, historyCheckView } from "./history-check";
import {
  NO_GPU_REASON,
  type StartChoice,
  chipNoticeVisible,
  setupArgs,
  startChoiceView,
} from "./start-choice";
```

Find (after Task 9):

```ts
  full_check_first: boolean;
```

Replace with:

```ts
  full_check_first: boolean;
  /** The engine refused this Mac's graphics chip, so the node follows
   *  signatures. The status screen says so once per engine. */
  chip_refused: boolean;
  /** How far the background check of the snapshot's older history has got;
   *  null when there is none running. */
  history_check: HistoryCheck | null;
```

- [ ] **Step 4: Call the new reflectors, and judge the stale card after the sample**

In `renderStatus`, find (lines 926-928):

```ts
  reflectPeerNames(status);
  reflectFork(status);
  reflectEngineNotes(status);
```

Replace with:

```ts
  reflectPeerNames(status);
  reflectEngineNotes(status);
  reflectChipNotice(status);
  reflectHistoryCheck(status);
```

Find (lines 967-969):

```ts
  reflectFollowOffer(status);

  let height = 0;
```

Replace with:

```ts
  // After the sample above, so the stale card judges the trend as it is now.
  reflectFork(status);
  reflectFollowOffer(status);

  let height = 0;
```

- [ ] **Step 5: The stale card, the history line, the chip notice**

Find the whole of `reflectFork` (lines 1671-1690):

```ts
function reflectFork(status: NodeStatusInfo): void {
  const card = $("fork-card");
  const running = status.phase.phase === "ready" || status.phase.phase === "syncing";
  // A stale tip outranks a fork verdict. A fork says "there is a better chain
  // we cannot reach"; a stale tip says "the newest block we have is hours old
  // however healthy everything else reads", which is the condition every
  // peer-derived signal in this app is blind to by construction.
  //
  // Behind the signers comes last because it is the earliest and the least
  // specific: it fires minutes into a split the node cannot see, before the
  // tip is old enough to be stale, and says less than either once they do.
  const message =
    status.tip_stale_message ?? status.fork_message ?? status.behind_signers_message;
  if (!running || !message) {
    card.hidden = true;
    return;
  }
  card.hidden = false;
  $("fork-msg").textContent = message;
}
```

Replace with:

```ts
function reflectFork(status: NodeStatusInfo): void {
  const card = $("fork-card");
  const p = status.phase;
  const running = p.phase === "ready" || p.phase === "syncing";
  // How many blocks the node knows of beyond its tip: the stale card's
  // "is behind".
  const behind =
    p.phase === "ready"
      ? p.blocks_behind
      : p.phase === "syncing"
        ? Math.max(0, p.headers - p.height)
        : null;
  // The stale sentence, judged with the catch-up trend (catchup-trend.ts
  // `staleCard`): nothing while the gap closes, a neutral line before the
  // trend is measured, amber with the pace when the gap is not closing, and
  // Rust's sentence unchanged when the node knows of no newer block.
  const stale = staleCard(status.tip_stale_message, behind, catchupSamples, Date.now());
  const amber = stale?.tone === "amber" ? stale.message : null;
  const neutral = stale?.tone === "neutral" ? stale.message : null;
  // An amber stale tip outranks a fork verdict. A fork says "there is a better
  // chain we cannot reach"; a stale tip says "the newest block we have is
  // hours old however healthy everything else reads", which is the condition
  // every peer-derived signal in this app is blind to by construction.
  //
  // Behind the signers comes after both because it is the earliest and the
  // least specific: it fires minutes into a split the node cannot see, before
  // the tip is old enough to be stale, and says less than either once they do.
  //
  // The neutral line comes last: it is not a verdict, and both of those are.
  const message = amber ?? status.fork_message ?? status.behind_signers_message ?? neutral;
  if (!running || !message) {
    card.hidden = true;
    return;
  }
  card.hidden = false;
  card.classList.toggle("is-calm", message === neutral);
  $("fork-msg").textContent = message;
}

/** The background check of the snapshot's older history: one line and a thin
 *  bar under the status line while it runs (history-check.ts). Hidden on a
 *  node that is not running, and the moment the engine reports it done. */
function reflectHistoryCheck(status: NodeStatusInfo): void {
  const wrap = $("history-check");
  const running = status.phase.phase === "ready" || status.phase.phase === "syncing";
  const view = running ? historyCheckView(status.history_check) : null;
  if (!view) {
    wrap.hidden = true;
    return;
  }
  wrap.hidden = false;
  $("history-line").textContent = view.line;
  $("history-fill").style.width = `${view.pct}%`;
}

/** The note after the engine turned this Mac's chip down, once per engine
 *  (start-choice.ts `chipNoticeVisible`). Remembered in localStorage by engine
 *  tag; where storage is unavailable it still closes for this run. */
const CHIP_NOTICE_KEY = "ebtx-node.chip-notice-seen";
let chipNoticeClosed = false;

function reflectChipNotice(status: NodeStatusInfo): void {
  let seen: string | null = null;
  try {
    seen = localStorage.getItem(CHIP_NOTICE_KEY);
  } catch {
    // No storage: show it until OK is pressed in this run.
  }
  $("chip-card").hidden =
    chipNoticeClosed ||
    !chipNoticeVisible(status.chip_refused, status.rc_trusted_mirror, seen, status.node_tag);
}

$("chip-ok").addEventListener("click", () => {
  chipNoticeClosed = true;
  try {
    if (lastStatus) localStorage.setItem(CHIP_NOTICE_KEY, lastStatus.node_tag);
  } catch {
    // Closed for this run; it may show again on the next.
  }
  $("chip-card").hidden = true;
});
```

(`CHIP_NOTICE_KEY` and `chipNoticeClosed` are declared after `renderStatus` in the file, which is safe: the first render happens after the first `await invoke("get_node_status")`, when the whole module has run.)

- [ ] **Step 6: Typecheck, test, build**

Run (in `apps/node`): `npx tsc --noEmit && npm test && npx vite build`
Expected: tsc prints nothing; `Test Files  10 passed (10)`, `Tests  147 passed (147)` (128 in 8 files on `aed8755`, Tools' `tools-history.test.ts` among them, plus 8, 4 and 7 from Tasks 6 to 8; counted, not run); vite ends with `✓ built in`.

- [ ] **Step 7: Commit**

```bash
git add apps/node/index.html apps/node/src/styles.css apps/node/src/main.ts
git commit -m "node: the status screen shows the history check, keeps the stale card calm while catching up, and tells a refused Mac once

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: The changelog

**Files:**
- Modify: `apps/node/CHANGELOG.md` (under `## [Unreleased]`, after the entries already there: on `aed8755` that is the Tools entry, "**Tools: the things support used to need a terminal for, behind one button.**", and #160 adds its role-card entry after it if merged first)

- [ ] **Step 1: Add the entry**

Add under `## [Unreleased]`, after the last paragraph already in that section:

```markdown
**Setup asks one question: Quick start or Full check.** Quick start follows
signatures and is ready in minutes. Full check checks every block on this
computer's graphics card and takes longer the first time. On a computer with
an NVIDIA graphics card, Full check is selected first. On a Mac, Quick start
is selected first, because some Mac graphics chips don't pass the engine's
check; you can still pick Full check. A computer without a graphics card
the engine can use gets Quick start, and the screen says why. You can switch
later in Settings. If the engine turns down a Mac's graphics chip and the app
moves the node to following signatures, the status screen says so once.

**The status screen shows the check of older history.** A node that starts
from a snapshot checks every new block from there, and checks the history
below the snapshot in the background. One line and a thin bar under the
status now show how far that has got, for example "Checking older history:
131,200 of 225,927 (58%)", and go away when it is done. There is no time
estimate, because the engine sets the pace. A node that checks blocks on a
signed snapshot says so on its role card.

**A node that is catching up is no longer told it is not following the
chain.** That sentence is kept for a node whose newest block is over two hours
old and that knows of nothing newer. While the gap is closing, the card stays
away. In the first minutes it says "Checking whether your node is catching
up...". If the gap is not closing, it stays amber and says how many blocks an
hour your node adds against the network's 40.
```

- [ ] **Step 2: Check the copy rules**

Run: `git diff apps/node/CHANGELOG.md | grep '^+' | grep -c '—'`
Expected: `0`.

- [ ] **Step 3: Commit**

```bash
git add apps/node/CHANGELOG.md
git commit -m "changelog: Quick start or Full check, the history check, a calmer stale card

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 12: Check it on screen (no commit)

**Files:**
- Create, temporarily and never committed: `apps/node/public/mock.js`, one `<script>` line in `apps/node/index.html`

This is the dry run's own check, repeatable: a fake of Tauri's IPC so the window renders in a plain browser at 560x780.

- [ ] **Step 1: The fake IPC**

Create `apps/node/public/mock.js`:

```js
// CHECK ONLY, never commit: fake Tauri IPC so the window renders in a browser.
(function () {
  const scen = location.hash.slice(1) || "wizard-nvidia";
  const STALE = "The newest block this node has is 200 hours old. That is measured against the clock, not against what its peers report, so it holds even when every peer agrees with it. The node is not following the chain.";
  const base = {
    running: true, uptime_secs: 600, disk_free_mb: 300000, disk_warn_mb: 20000, disk_critical_mb: 5000,
    disk_required_mb: 143360, datadir_size_mb: 12000, datadir: "/Users/x/.easybtx", node_tag: "v0.34.9",
    installed: true, setup_complete: true, welcome_shown: true, keep_awake: true, keep_awake_supported: true,
    tray_term: "menu bar", txindex_enabled: false, attestation_serve_enabled: true, signer_enabled: false,
    signer_applies_here: false, signer_pubkey: null, signing_live: false, signer_publish_enabled: false, signer_offer: null,
    archive_service: null, archive_service_message: null, archive_service_needs_attention: false,
    role: { validation_mode: "consensus", holds_signing_key: false, advertises_consensus: true, advertises_archive: false, reachable_inbound: false, inbound: 0, uptime_secs: 600, blocks_behind: 7400, signed_recent: null },
    role_lines: [{ label: "Validation", value: "Checking every block itself", helps: true, note: "Checks every new block itself. Its older history is still being checked." }],
    fork: null, fork_message: null, tip_stale: true, tip_age_secs: 720000, tip_stale_message: STALE,
    behind_signers_message: null, engine_notes: [], node_nickname: "", broadcast_nickname: null, subversion: null,
    peer_nicknames: [], service_report_enabled: false, wallet_enabled: false, on_close: "ask",
    rc_mode: "strict-device", rc_validates_independently: false, rc_may_fall_behind: false, rc_reason: null,
    rc_stalled: false, rc_unverifiable_message: null, rc_trusted_mirror: true, follow_signatures: false,
    full_check_possible: true, full_check_first: false, chip_refused: true, history_check: { checked: 131200, base: 225927 },
    bytes_sent: 1000000, inbound_peers: 0, archive_peers: null, stall: null, node_profile: "full",
    datadir_pruned: false, keeper_engine_ready: false, esplora_enabled: false, esplora_listen: "127.0.0.1:3002",
    esplora_serving_on: null, esplora_running: false, esplora_indexing: false, esplora_freshness: null, esplora_message: null,
    witness_enabled: false, witness_listen: "127.0.0.1:3003", witness_serving_on: null, witness_running: false,
    witness_public: false, witness_message: null, snapshot_serve_enabled: false, snapshot_serve: null,
    last_update_check_at: null, last_update_check_outcome: null, last_update_check_detail: "",
    phase: { phase: "ready", height: 226000, peers: 8, blocks_behind: 7400 },
  };
  let status = base;
  if (scen.startsWith("wizard")) {
    status = { ...base, running: false, setup_complete: false, phase: { phase: "welcome" }, full_check_possible: scen !== "wizard-cpu", full_check_first: scen === "wizard-nvidia", uptime_secs: 0 };
  }
  let cb = 0;
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    transformCallback: () => ++cb,
    unregisterCallback: () => {},
    convertFileSrc: (p) => p,
    invoke: async (cmd, args) => {
      window.__calls = window.__calls || [];
      window.__calls.push([cmd, args]);
      if (cmd === "get_node_status") return status;
      if (cmd === "plugin:app|version") return "0.7.0";
      if (cmd === "plugin:autostart|is_enabled") return false;
      if (cmd === "plugin:event|listen") return 1;
      return null;
    },
  };
})();
```

In `apps/node/index.html`, find (line 8):

```html
    <script type="module" src="/src/main.ts" defer></script>
```

and temporarily put this line directly above it:

```html
    <script src="/mock.js"></script>
```

- [ ] **Step 2: Look at it**

Run (in `apps/node`): `npx vite --port 1530 --strictPort`, open it in a browser sized to 560x780, and check:

- `http://localhost:1530/#wizard-nvidia`: Full check selected; the setup button is on screen without scrolling (the dry run measured its bottom at 649 px of 780 on `b008f98`; Tools' button sits in the existing header row, so expect the same); the line under the choices reads "Both start from a recent signed snapshot and check the older history in the background. Full check also checks every new block on this computer's graphics card. You can switch later in Settings." Click Quick start, wait for a poll (1.5 s): Quick start stays selected. Click Set up my node; in the console, `window.__calls.filter(c => c[0] === "begin_setup")` is `[["begin_setup", {"choice": "quick_start"}]]`.
- `http://localhost:1530/#wizard-mac` (reload after changing the hash): Quick start selected, Full check not greyed out and no reason line under it. Click Full check, wait for a poll: Full check stays selected. Click Set up my node: the call is `[["begin_setup", {"choice": "full_check"}]]`.
- `http://localhost:1530/#wizard-cpu` (reload after changing the hash): Quick start selected, Full check greyed out and not clickable, with "This computer has no graphics card the BTX engine can check blocks with." under it.
- `http://localhost:1530/#status`: "Checking older history: 131,200 of 225,927 (58%)" with a bar at 58% under the status line; the chain card reads "Checking whether your node is catching up..." in the quiet colours; the role card's note reads "Checks every new block itself. Its older history is still being checked."; the chip notice shows, OK hides it and `localStorage.getItem("ebtx-node.chip-notice-seen")` is `"v0.34.9"`.

- [ ] **Step 3: Remove the fake and prove it is gone**

Run: `rm apps/node/public/mock.js`, delete the `<script src="/mock.js"></script>` line, then `git status --short`
Expected: nothing listed.

- [ ] **Step 4: By hand on real machines (the design's own list)**

On a Mac, a Windows PC and a Linux machine, with a fresh data folder each time:

- What is selected first: Quick start on the Mac, with Full check still clickable; Full check on the Linux machine with an NVIDIA card; Quick start on Windows, with Full check greyed out and the reason under it (its engine has no GPU code).
- Quick start: the node starts as a mirror; `.follow-signatures` is in the data folder on the Mac (kept as preselected) and on the Linux machine with an NVIDIA card (picked by hand), and absent on Windows. `setup.log` has `start choice: QuickStart`. Settings shows the follow-signatures switch on the Mac and Linux.
- Full check (picked by hand on the Mac, the default on Linux with NVIDIA): the node starts validating where the card qualifies; `setup.log` has `start choice: FullCheck`; there is no `.follow-signatures`. A Mac whose chip fails the engine's check shows "NOT FOLLOWING" and the existing "Follow signatures instead" offer, not the chip notice (see "Where the design does not match the code").
- Then switch in Settings and watch the node restart the other way.
- On a node that started from a snapshot, the history line appears and its numbers move; on a node started from the compiled 219,000 snapshot, the base reads 219,000; on a signed snapshot (spec B), it reads that snapshot's own height.

---

## Self-review

**Spec coverage.**

| Design requirement | Task |
|---|---|
| Two choices above "Set up my node", with the line under them (in the owner's wording of 29 September, decision B) | 9, 12 |
| Which choice is selected first, as the owner decided on 29 September (decision A): Full check on an NVIDIA machine; Quick start on a Mac, with Full check still selectable; Quick start where the machine cannot check blocks | 2 (`may_check_blocks`, `full_check_first`), 5 (`full_check_possible`, `full_check_first`), 8, 9, 12 |
| Full check greyed out with the reason on a machine without a usable GPU | 8, 9 |
| Refused Mac told once: "...Nothing for you to do." | 5 (`chip_refused`), 8, 10 (see "Where the design does not match the code") |
| `begin_setup` takes the choice and writes or removes `.follow-signatures` before the first start, a Mac that keeps Quick start included | 2, 5 |
| History line and thin bar under the status line, base from the snapshot's own header, gone when done, no estimate | 1, 4, 7, 10 |
| One new field `history_check`, from the `getchainstates` the refresher already reads | 1, 4 |
| Role line on a validating node on a signed snapshot, detected by `attested_assumeutxo`, gone when the check is done | 3, 4 |
| Pure card function in `catchup-trend.ts`, four rows | 6 |
| Fork and "behind the signers" keep their place after the stale sentence | 10 |
| Tests: one per row, a crossing, a restart | 6 |
| Tests: `history_check` with no snapshot, a snapshot being checked, a finished check, a signed snapshot's own base | 1 |
| Tests: the choice reaches `begin_setup`; no-GPU cannot pick Full check; a Mac starts on Quick start, an NVIDIA machine on Full check | 2 (serde names, marker, `full_check_first`), 8 (`setupArgs`, `startChoiceView`), 12 (the clicks, in a browser) |
| By hand on Mac, Windows, Linux | 12 |
| 560x780, no new screens | Global Constraints; 12 measures the wizard |
| Rollback: `begin_setup` without a choice behaves as today; the card and the line can be switched off in one place | 5 (`Option`), 6 (`staleCard`), 10 (`reflectHistoryCheck`) |

The `getchainstates` fixtures are the recorded shape already in `node_api.rs` (`snapshot_chainstates_json`, a real node's answer) plus two shapes derived from the engine's `getchainstates` code (the in-run finish, and a signed snapshot at 225,927); none was captured from a node that finished a check, because the mainnet test of 29 September ran 23 minutes with the background check at 0.

**Placeholder scan.** No "TBD", "TODO", "similar to", or step without its code. Every edit names the text to find and the text that replaces it.

**Type consistency.** `HistoryCheck { checked, base }` (Rust) and `{ checked: number; base: number }` (TypeScript); `StartChoice::{QuickStart, FullCheck}` and `"quick_start" | "full_check"`; `refresh_history_check(&dyn Rpc, &ChainStates, &mut Option<(String, u64)>)` in Tasks 1 and 4; `with_signed_snapshot(bool)` in Tasks 3 and 4; `apply_start_choice(&Path, StartChoice, bool)` in Tasks 2 and 5; `full_check_first` (Rust `Backend` method, shell helper, status field) and `fullCheckFirst` (TypeScript) in Tasks 2, 5, 8, 9 and 12; `startChoiceView(boolean, boolean, StartChoice | null)` in Tasks 8 and 9; `staleCard(string | null, number | null, CatchupSample[], number)` in Tasks 6 and 10; `chipNoticeVisible(boolean, boolean, string | null, string)` in Tasks 8 and 10; DOM ids match between Tasks 9, 10 and 12.

**Dry run.** Every code block above was applied to a throwaway worktree of `origin/claude/ui-start-choice-and-progress` and run: `crates/btx-core` fmt, clippy (`-D clippy::correctness -D clippy::suspicious`) and `cargo test --locked` (559 passed, 2 ignored; the timing test noted in Task 2 failed twice straight after a clippy build and passed on every quiet re-run); `apps/node/src-tauri` fmt, clippy and `cargo test --locked` (85 passed, 1 ignored) after Task 4 and again after Task 5; `apps/node` `npx tsc --noEmit`, `npm test` (137 passed) and `npx vite build`, with Task 9 alone typechecked separately and Tasks 9 and 10 together reproducing the checked `main.ts` byte for byte; and the browser check of Task 12 at 560x780. Each failing-test step was run first and failed as stated. Then the worktree was reset and this document's own code blocks were applied mechanically, in order, by a script following each find, replace, insert, append and create step (Tasks 1 to 11); the result matched the dry run's code exactly, the changelog aside, and every gate passed again.

**Not run since the amendment of 29 September (night).** The dry run above was on `b008f98`. The amendments were not run: decision A's code (`Backend::full_check_first` and its test, the Mac lines of `the_setup_choice_writes_or_removes_the_marker`, `setup_backend`/`full_check_first` and the status field in the shell, the three-argument `startChoiceView` and its new test, the two `main.ts` call sites, the mock's `wizard-mac` scenario), decision B's copy, and the rebase onto `aed8755`. The anchors were checked by reading `origin/main` at `aed8755`, not by applying them. The test counts are derived: test attributes were counted on `origin/main` and the same count on `b008f98` reproduced the dry run's numbers (548 and 2 ignored for `btx-core` on a Mac, 85 and 1 ignored for the shell, 119 for `apps/node` before Tasks 6 to 8). Treat each written count as a check, and if a run differs only by tests this plan did not write, trust the run.
