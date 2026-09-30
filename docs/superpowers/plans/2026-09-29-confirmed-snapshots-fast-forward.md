# Confirmed Snapshots, Part 2: Fast-forward (Implementation Plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Amended 2026-09-29 (night).** Rebased on `origin/main` and brought in line with the amended decision and with plan 1 as amended the same night. What changed, and why:
> - **Base.** The Tools overlay is on main (#154, `aed8755`) and the role-card fix too (#160, `b330e3d`). Every anchor in `tools.ts`, `tools-history.ts`, `tools-history.test.ts`, `index.html`, `tools.rs`, `lib.rs` and `commands.rs` was re-read on `origin/main`. Two were wrong for main: both `tools-history` imports also bring `ReportCopy`, which the first version's replacement lines dropped. #160 touched none of these anchors.
> - **The prompt names who confirmed it.** The first click shows the decision's words, "Fast-forward to block 233,800, confirmed by Mende and jpp? Your node stops for a few minutes to load it, then carries on from there.", with the operators whose signatures this app verified (never the website's list), then what stays and the wallet sentence. The Tools decision's "in these words or close to them" allows that.
> - **A dispute turns it off, and says so.** While `latest` answers `{"disputed": [...]}`, the check reports the newest disputed height and the section shows "Fast-forward is off while the snapshot operators disagree about block 233,800." with no button, on any node the app owns. `peek_confirmed` returns `Peek::Confirmed { height, operators }` or `Peek::Disputed { newest }`.
> - **A run that ends below the snapshot rolls back.** If `latest` turns disputed between the check and the load, the start path takes the pinned pair or the compiled snapshot. `judge` now rolls that back at once instead of waiting three hours.
> - **The start record moves with the chain.** `snapshot-start.json` (plan 1, Task 2b) is set aside with the chain data and comes back on a roll-back; the "Done" message names the operators from it.
> - **Sooner when no archive peer serves old blocks (the owner's decision, added later the same night).** When the catch-up help concludes that no archive peer will serve this node old blocks (`CatchUpReport::no_archive_serves_old_blocks`, the catch-up plan, which lands first), Fast-forward is offered below the 1,000-block line, as long as a confirmed snapshot sits above the node's tip. Never during a dispute. The rule is in the pure `offer` and `fast_forward_check`, with tests; the section's note says why it is offered early and promises nothing new.
> - **Kept:** the 1,000-block threshold otherwise, two clicks within ten seconds, the three-hour limit, the records and the roll-back.
> - **Copy:** no new or touched string has an em-dash.

**Goal:** A node the app owns that is more than 1,000 blocks behind a confirmed snapshot, or any distance behind one while no archive peer will serve it old blocks, gets a Fast-forward button in Tools; the first click names the block and the operators who confirmed it, and a second within ten seconds sets the chain data aside, loads the confirmed snapshot through the one loading path, and keeps the new chain, or puts the old one back and says what failed. While the snapshot operators disagree about a snapshot, the section says Fast-forward is off and why.

**Architecture:** `btx-core/src/fast_forward.rs` holds everything that can be decided without the app: which data moves, setting it aside and putting it back, the on-disk record of a run, a pure verdict (`judge`) on what the node shows, and the copy. `attested_snapshot::peek_confirmed` answers "is there a confirmed snapshot, at what height and confirmed by whom, or do the operators disagree" without downloading the file. The app's `fast_forward.rs` drives a run: download while the node runs, stop, set aside, start; the start path of plan 1 does the loading (header bootstrap, a validating node's mirror launch, the load, the start record, the restart); the driver watches for the verdict and rolls back on failure. The Tools overlay shows the section per the Tools decision, section 3, with one pure function deciding what it shows.

**Tech Stack:** Rust (btx-core, Tauri 2), TypeScript (Vite, vitest), btxd v0.34.9 for the opt-in rehearsal.

**This is plan 2 of 2.** It needs plan 1 (`2026-09-29-confirmed-snapshots-core.md`, same folder, as amended the night of 2026-09-29) done first, on the same branch.

## Global Constraints

- Everything in plan 1's Global Constraints applies (branch `claude/confirmed-snapshots` from `origin/main`, CI gates, commit trailer, copy rules, lock-file rule, the known flake, "derived, not run").
- Design: sections 6a and 10 of `docs/decisions/2026-09-29-every-node-starts-near-the-tip.md` (as amended, commit `12e44c3` on `claude/cosigned-snapshots`) and its "Who confirmed the start point, on screen"; the button: section 3 of `docs/decisions/2026-09-29-tools-and-command-window.md` (on main).
- The button shows only when all hold: the app owns the node (`destructive_allowed(node_ownership(..))` is `Ok`); a confirmed snapshot is available, checked as the loader checks it (the `latest` answer and the manifest, every rule of plan 1's `confirmed_snapshot::check`); it is above the node's tip; and either it is more than **1,000** blocks above the tip, or the catch-up help has concluded that no archive peer serves this node old blocks (`state.catch_up_help.lock().await.no_archive_serves_old_blocks`, the catch-up plan's `CatchUpReport`, written every refresher tick and reset to `false` on every start and stop). Same for a node that checks blocks and one that follows signatures. The front end's `status.stall?.class === "old_blocks_refused"` is the same fact; the offer is decided in the shell, so it reads the slot.
- The section's note says why the button is there: "A confirmed snapshot is far ahead of your node." when it is more than 1,000 blocks ahead; otherwise, when it is offered early, "Your node's peers are not sending the older blocks it needs, so Fast-forward is offered sooner than usual. It works the same way as always.
- While `latest` answers a dispute, the section shows, on any node the app owns, exactly "Fast-forward is off while the snapshot operators disagree about block 233,800." (the newest disputed height, with thousands commas) and no button.
- Button text: "Fast-forward to block 233,800" (height with thousands commas). The first click shows, in these words: "Fast-forward to block 233,800, confirmed by Mende and jpp? Your node stops for a few minutes to load it, then carries on from there. Your wallets and keys stay as they are, and afterwards the node checks the older history in the background. A wallet you last used before block 233,800 opens once that check gets there. Click again to start." The names are the operators whose signatures the app verified in the manifest, in the list's order, joined as "Mende and jpp" or "Mende, Aleksander and jpp" (plan 1's `snapshot_start::join_names`). The second click must come within **10 seconds**.
- Moved into `<datadir>/fast-forward-<unix seconds>/`: `blocks`, `chainstate`, `chainstate_snapshot`, `indexes`, `shielded_state` (chain data the design's list leaves out; left in place it would describe the old chain) and `snapshot-start.json` (plan 1's start record, which describes the chain it sits beside). Wallets, keys, the conf, settings, peers, bans and the diary stay.
- On success delete the dated folder. On any failure: stop, remove what the attempt made, move the old data back, restore the "snapshot loaded" setting, clear the mirror-load and header-bootstrap markers, start as before, and say what failed in one sentence. A run not done in **3 hours** is rolled back, and so is a run whose node comes up on a snapshot below the one offered.
- Records: `<datadir>/.fast-forward.json` (a run in progress; survives a quit, and the next start resumes watching it) and `<datadir>/.fast-forward-result.json` (how the last run ended, for the overlay; a finished run keeps the operators the start record names).
- Copy: friendly, simple, no hype, no guarantees, no em-dashes.

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `crates/btx-core/src/fast_forward.rs` | create | What moves, set aside, restore, discard, the run record and outcome, `judge`, `offer`, the copy |
| `crates/btx-core/src/attested_snapshot.rs` | modify | `Peek`, `peek_confirmed` (the `latest` answer and the manifest checked, file not fetched) |
| `crates/btx-core/src/lib.rs` | modify | Register `fast_forward` |
| `apps/node/src-tauri/src/fast_forward.rs` | create | The driver: run, watch, roll back, resume |
| `apps/node/src-tauri/src/commands.rs` | modify | Signed-only loads during a run; failures go to the driver; resume at start |
| `apps/node/src-tauri/src/tools.rs`, `apps/node/src-tauri/src/lib.rs` | modify | `tools_fast_forward_check` (offer, off, or nothing), `_run`, `_status`; registration; a test of the check's shape |
| `apps/node/index.html`, `apps/node/src/tools.ts`, `apps/node/src/tools-history.ts`, `apps/node/src/tools-history.test.ts` | modify | The Fast-forward section of the Tools overlay, and the pure `fastForwardView` that decides what it shows |
| `crates/btx-core/tests/confirmed_snapshot_regtest.rs` | modify | Roll-back rehearsal on a real engine folder |
| `apps/node/CHANGELOG.md` | modify | One entry |
## Tasks

### Task 1: Set aside, put back, and judge a run (`btx-core/src/fast_forward.rs`)

**Files:**
- Create: `crates/btx-core/src/fast_forward.rs`
- Modify: `crates/btx-core/src/lib.rs`

**Interfaces:**
- Consumes: plan 1's `snapshot_start::{block_number, join_names, START_RECORD_FILE}` (Task 2b).
- Produces (Tasks 3, 4, 6):
  - consts `MIN_LEAD` (1,000), `MAX_RUN_SECS` (10,800), `CHAIN_DATA` (now with `snapshot-start.json`)
  - `pub struct Record { height: u64, aside: String, moved: Vec<String>, snapshot_loaded_before: bool, started_at: u64 }` (serde)
  - `pub enum Outcome { Done { height: u64, operators: Vec<String> }, RolledBack { reason: String } }` (serde, tagged `state`; `operators` defaults to empty, so a result file from before reads)
  - `pub enum OfferReason { FarBehind, OldBlocksRefused }`, `pub fn offer(owned: bool, confirmed_height: Option<u64>, tip: u64, old_blocks_refused: bool) -> Option<(u64, OfferReason)>`, `pub fn aside_name(u64) -> String`
  - `pub fn set_aside(datadir: &Path, height: u64, snapshot_loaded_before: bool, now_unix: u64) -> std::io::Result<Record>`, `pub fn restore(&Path, &Record) -> std::io::Result<()>`, `pub fn discard(&Path, &Record) -> std::io::Result<()>`
  - `write_record`, `read_record`, `clear_record`, `write_outcome`, `read_outcome`, `clear_outcome`
  - `pub struct Look { snapshot_base_height: Option<u64>, mirror_load_pending: bool, header_bootstrap_pending: bool, running: bool, load_failed: Option<String> }`, `pub enum Verdict { Continue, Done, RollBack(String) }`, `pub fn judge(&Record, &Look, now_unix: u64) -> Verdict`
  - `pub mod copy { height, button, confirm(h, operators), note(OfferReason), off(h), running, done(h, operators), rolled_back }`

"Done" accepts a snapshot base at or above the run's height, because the start path re-reads `latest` before it loads and may find a newer confirmed snapshot. A base below it, on the node's ordinary launch, is a roll-back: the start path took a fallback (a dispute that began after the check, or a refusal), and waiting three hours would not change that.

- [ ] **Step 1: Write the failing tests**

Create `crates/btx-core/src/fast_forward.rs` with only the test module:

````rust
#[cfg(test)]
mod tests {
    use super::*;

    fn datadir_with_chain() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        for name in [
            "blocks",
            "chainstate",
            "chainstate_snapshot",
            "shielded_state",
        ] {
            std::fs::create_dir_all(d.join(name)).unwrap();
            std::fs::write(d.join(name).join("old"), name).unwrap();
        }
        std::fs::write(d.join("snapshot-start.json"), b"old start").unwrap();
        std::fs::create_dir_all(d.join("wallets/main")).unwrap();
        std::fs::write(d.join("wallets/main/wallet.dat"), b"keys").unwrap();
        std::fs::write(d.join("attestation-signer.key"), b"wif").unwrap();
        std::fs::write(d.join("peers.dat"), b"peers").unwrap();
        std::fs::write(d.join("banlist.json"), b"[]").unwrap();
        std::fs::write(d.join("snapshot-diary.json"), b"[]").unwrap();
        tmp
    }

    #[test]
    fn the_button_shows_only_for_an_owned_node_far_behind() {
        let far = Some((232_000, OfferReason::FarBehind));
        assert_eq!(offer(true, Some(232_000), 230_999, false), far);
        assert_eq!(
            offer(true, Some(232_000), 231_000, false),
            None,
            "exactly 1,000 is not more"
        );
        assert_eq!(offer(true, Some(232_000), 231_001, false), None, "999 behind");
        assert_eq!(offer(false, Some(232_000), 100, false), None, "not ours");
        assert_eq!(offer(true, None, 100, false), None, "nothing confirmed");
    }

    /// The owner's decision: while no archive peer will serve this node old
    /// blocks, the button shows below the 1,000-block line, as long as the
    /// confirmed snapshot is above the tip at all.
    #[test]
    fn the_button_shows_sooner_when_no_archive_peer_serves_old_blocks() {
        assert_eq!(
            offer(true, Some(232_000), 231_700, true),
            Some((232_000, OfferReason::OldBlocksRefused)),
            "300 behind"
        );
        assert_eq!(
            offer(true, Some(232_000), 231_999, true),
            Some((232_000, OfferReason::OldBlocksRefused)),
            "one block behind"
        );
        assert_eq!(
            offer(true, Some(232_000), 230_000, true),
            Some((232_000, OfferReason::FarBehind)),
            "far behind says so, whatever the flag"
        );
        assert_eq!(offer(true, Some(232_000), 232_000, true), None, "not above the tip");
        assert_eq!(offer(true, Some(232_000), 232_500, true), None, "below the tip");
        assert_eq!(offer(true, None, 100, true), None, "nothing confirmed");
        assert_eq!(offer(false, Some(232_000), 231_700, true), None, "not ours");
    }

    #[test]
    fn only_chain_data_moves_and_it_all_comes_back() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let r = set_aside(d, 232_000, true, 1_790_000_000).unwrap();
        assert_eq!(r.aside, "fast-forward-1790000000");
        assert_eq!(
            r.moved,
            vec![
                "blocks",
                "chainstate",
                "chainstate_snapshot",
                "shielded_state",
                "snapshot-start.json"
            ]
        );
        for name in CHAIN_DATA {
            assert!(!d.join(name).exists(), "{name} still in place");
        }
        for kept in [
            "wallets/main/wallet.dat",
            "attestation-signer.key",
            "peers.dat",
            "banlist.json",
            "snapshot-diary.json",
        ] {
            assert!(d.join(kept).exists(), "{kept} moved");
        }
        // The failed attempt made new chain data and a new start record;
        // restoring replaces them.
        std::fs::create_dir_all(d.join("blocks")).unwrap();
        std::fs::write(d.join("blocks/new"), b"new").unwrap();
        std::fs::create_dir_all(d.join("indexes")).unwrap();
        std::fs::write(d.join("snapshot-start.json"), b"new start").unwrap();
        restore(d, &r).unwrap();
        for name in [
            "blocks",
            "chainstate",
            "chainstate_snapshot",
            "shielded_state",
        ] {
            assert_eq!(
                std::fs::read_to_string(d.join(name).join("old")).unwrap(),
                name
            );
        }
        assert!(!d.join("blocks/new").exists());
        assert!(
            !d.join("indexes").exists(),
            "made by the attempt, gone with it"
        );
        assert_eq!(
            std::fs::read_to_string(d.join("snapshot-start.json")).unwrap(),
            "old start",
            "the start record comes back with the chain it describes"
        );
        assert!(!d.join(&r.aside).exists());
    }

    #[test]
    fn restore_removes_nothing_when_an_original_is_missing() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let r = set_aside(d, 232_000, false, 7).unwrap();
        std::fs::create_dir_all(d.join("blocks")).unwrap();
        std::fs::write(d.join("blocks/new"), b"new").unwrap();
        std::fs::remove_dir_all(d.join(&r.aside).join("chainstate")).unwrap();
        assert!(restore(d, &r).is_err());
        assert!(d.join("blocks/new").exists(), "nothing removed");
    }

    #[test]
    fn a_failed_move_puts_back_what_moved() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        // A second entry whose move fails (its parent does not exist in the
        // dated folder), after `blocks` has already moved.
        std::fs::create_dir_all(d.join("nested/data")).unwrap();
        assert!(set_aside_names(d, &["blocks", "nested/data"], 232_000, false, 9).is_err());
        assert!(d.join("blocks/old").exists(), "blocks came back");
        assert!(d.join("nested/data").exists());
        assert!(!d.join(aside_name(9)).exists(), "the dated folder is gone");
        // And a dated folder in the way stops it before anything moves.
        std::fs::write(d.join(aside_name(9)), b"in the way").unwrap();
        assert!(set_aside(d, 232_000, false, 9).is_err());
        for name in [
            "blocks",
            "chainstate",
            "chainstate_snapshot",
            "shielded_state",
        ] {
            assert!(d.join(name).join("old").exists(), "{name}");
        }
    }

    #[test]
    fn discard_removes_only_the_dated_folder() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let r = set_aside(d, 232_000, false, 11).unwrap();
        std::fs::create_dir_all(d.join("blocks")).unwrap();
        discard(d, &r).unwrap();
        assert!(!d.join(&r.aside).exists());
        assert!(d.join("blocks").exists());
        assert!(d.join("wallets/main/wallet.dat").exists());
    }

    #[test]
    fn the_record_and_the_outcome_survive_a_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        assert_eq!(read_record(d), None);
        let r = Record {
            height: 232_000,
            aside: aside_name(1),
            moved: vec!["blocks".into()],
            snapshot_loaded_before: true,
            started_at: 1,
        };
        write_record(d, &r).unwrap();
        assert_eq!(read_record(d), Some(r));
        clear_record(d);
        assert_eq!(read_record(d), None);
        let done = Outcome::Done {
            height: 232_000,
            operators: vec!["Mende".into(), "jpp".into()],
        };
        write_outcome(d, &done);
        assert_eq!(read_outcome(d), Some(done));
        clear_outcome(d);
        assert_eq!(read_outcome(d), None);
        // A result written before the names were kept still reads.
        std::fs::write(
            d.join(".fast-forward-result.json"),
            r#"{"state":"done","height":232000}"#,
        )
        .unwrap();
        assert_eq!(
            read_outcome(d),
            Some(Outcome::Done {
                height: 232_000,
                operators: vec![]
            })
        );
    }

    #[test]
    fn a_run_is_done_only_when_the_node_runs_ordinarily_on_the_snapshot() {
        let r = Record {
            height: 232_000,
            aside: aside_name(1_000),
            moved: vec![],
            snapshot_loaded_before: false,
            started_at: 1_000,
        };
        let on_it = Look {
            snapshot_base_height: Some(232_000),
            running: true,
            ..Look::default()
        };
        assert_eq!(judge(&r, &on_it, 2_000), Verdict::Done);
        let mirror_launch = Look {
            mirror_load_pending: true,
            ..on_it.clone()
        };
        assert_eq!(judge(&r, &mirror_launch, 2_000), Verdict::Continue);
        let bootstrap = Look {
            header_bootstrap_pending: true,
            snapshot_base_height: None,
            ..on_it.clone()
        };
        assert_eq!(judge(&r, &bootstrap, 2_000), Verdict::Continue);
        // The node came up on something lower than the snapshot offered: the
        // start path took a fallback. Rolled back at once, not after 3 hours.
        let compiled = Look {
            snapshot_base_height: Some(219_000),
            ..on_it.clone()
        };
        assert_eq!(
            judge(&r, &compiled, 2_000),
            Verdict::RollBack(
                "the node started from block 219,000 instead of the confirmed snapshot at block 232,000"
                    .into()
            )
        );
        // Judged only on the ordinary launch: a mirror launch holding the
        // pinned pair is still on its way.
        let pinned_in_mirror_launch = Look {
            snapshot_base_height: Some(225_927),
            mirror_load_pending: true,
            ..on_it.clone()
        };
        assert_eq!(
            judge(&r, &pinned_in_mirror_launch, 2_000),
            Verdict::Continue
        );
        let newer = Look {
            snapshot_base_height: Some(232_200),
            ..on_it.clone()
        };
        assert_eq!(judge(&r, &newer, 2_000), Verdict::Done);
        let failed = Look {
            load_failed: Some("the engine did not load it".into()),
            ..Look::default()
        };
        assert_eq!(
            judge(&r, &failed, 2_000),
            Verdict::RollBack("the engine did not load it".into())
        );
        assert!(matches!(
            judge(&r, &Look::default(), 1_000 + MAX_RUN_SECS),
            Verdict::RollBack(_)
        ));
    }

    #[test]
    fn the_copy_is_the_decisions_and_has_no_em_dash() {
        assert_eq!(copy::height(233_800), "233,800");
        assert_eq!(copy::height(999), "999");
        assert_eq!(copy::height(1_234_567), "1,234,567");
        assert_eq!(copy::button(233_800), "Fast-forward to block 233,800");
        let names = vec!["Mende".to_string(), "jpp".to_string()];
        let c = copy::confirm(233_800, &names);
        assert!(
            c.starts_with(
                "Fast-forward to block 233,800, confirmed by Mende and jpp? Your node stops for a few minutes to load it, then carries on from there."
            ),
            "{c}"
        );
        assert!(c.contains("Your wallets and keys stay as they are"));
        assert!(c.contains(
            "A wallet you last used before block 233,800 opens once that check gets there."
        ));
        assert!(c.ends_with("Click again to start."));
        let three = vec!["Mende".to_string(), "Aleksander".to_string(), "jpp".to_string()];
        assert!(copy::confirm(233_800, &three).contains("confirmed by Mende, Aleksander and jpp?"));
        assert_eq!(
            copy::off(233_800),
            "Fast-forward is off while the snapshot operators disagree about block 233,800."
        );
        assert_eq!(
            copy::note(OfferReason::FarBehind),
            "A confirmed snapshot is far ahead of your node."
        );
        assert_eq!(
            copy::note(OfferReason::OldBlocksRefused),
            "Your node's peers are not sending the older blocks it needs, so Fast-forward is offered sooner than usual. It works the same way as always."
        );
        assert_eq!(
            copy::done(233_800, &names),
            "Done. Your node now starts from block 233,800, confirmed by Mende and jpp."
        );
        assert_eq!(
            copy::done(233_800, &[]),
            "Done. Your node now starts from block 233,800."
        );
        for s in [
            c,
            copy::off(1),
            copy::note(OfferReason::FarBehind),
            copy::note(OfferReason::OldBlocksRefused),
            copy::running(),
            copy::done(1, &names),
            copy::rolled_back("x"),
        ] {
            assert!(!s.contains('\u{2014}'), "{s}");
            assert!(!s.to_lowercase().contains("guarantee"), "{s}");
        }
    }
}
````

In `crates/btx-core/src/lib.rs`, after `pub mod esplora_sidecar;`:

````rust
pub mod esplora_sidecar;
pub mod fast_forward;
````

- [ ] **Step 2: Run the tests to see them fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- fast_forward`
Expected: compile errors, among them `cannot find function `set_aside`` and `cannot find type `Record``.

- [ ] **Step 3: Write the implementation**

Insert above `#[cfg(test)]` in `crates/btx-core/src/fast_forward.rs`:

````rust
//! Fast-forward: move a node that has fallen far behind a confirmed snapshot
//! onto it, and put everything back if anything fails
//! (docs/decisions/2026-09-29-every-node-starts-near-the-tip.md, section 10;
//! the button is docs/decisions/2026-09-29-tools-and-command-window.md,
//! section 3).
//!
//! The loading itself is the one path every node uses
//! (`crate::confirmed_load`, driven by the app's start path). What is here is
//! the part only Fast-forward has: setting the chain data aside in a dated
//! folder beside it, remembering that in a record so an interrupted run can
//! be finished or undone, and deciding from what the node shows whether the
//! run is done, still going, or has to be rolled back.
//!
//! Wallets, keys, the conf, settings, peers, bans and the diary stay where
//! they are: only [`CHAIN_DATA`] moves.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A confirmed snapshot must be more than this far ahead of the node's tip
/// before the button shows (the owner's choice 4).
pub const MIN_LEAD: u64 = 1_000;

/// A run that has not finished in this long is rolled back. A load takes
/// minutes; the header sync before it, on a slow link, can take an hour.
pub const MAX_RUN_SECS: u64 = 3 * 60 * 60;

/// Everything that describes the chain. `shielded_state` is not in the
/// decision's list but is chain data as much as `chainstate` is: left in
/// place it would describe the old chain to the new one. The start record
/// (`crate::snapshot_start`) says where the chain beside it started, so it
/// goes and comes back with that chain; the load writes the new one.
pub const CHAIN_DATA: &[&str] = &[
    "blocks",
    "chainstate",
    "chainstate_snapshot",
    "indexes",
    "shielded_state",
    crate::snapshot_start::START_RECORD_FILE,
];

fn record_path(datadir: &Path) -> PathBuf {
    datadir.join(".fast-forward.json")
}

fn result_path(datadir: &Path) -> PathBuf {
    datadir.join(".fast-forward-result.json")
}

/// A run in progress, on disk, so a quit in the middle can be finished or
/// undone at the next start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The confirmed snapshot's base.
    pub height: u64,
    /// The dated folder, relative to the datadir.
    pub aside: String,
    /// What was moved into it, so exactly that comes back.
    pub moved: Vec<String>,
    /// The app's "a snapshot was loaded" setting before the run.
    pub snapshot_loaded_before: bool,
    pub started_at: u64,
}

/// How the last run ended, for the Tools overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    Done {
        height: u64,
        /// Who confirmed it, from the start record the load wrote: only
        /// operators whose signatures the app verified.
        #[serde(default)]
        operators: Vec<String>,
    },
    RolledBack {
        reason: String,
    },
}

/// Why the button shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferReason {
    /// A confirmed snapshot is more than [`MIN_LEAD`] blocks above the tip
    /// (section 10).
    FarBehind,
    /// Closer than that, but the catch-up help has concluded that no archive
    /// peer will serve this node old blocks (the owner's decision; the
    /// catch-up plan's `CatchUpReport::no_archive_serves_old_blocks`).
    OldBlocksRefused,
}

/// The button's height and why it shows: the app owns the node, a confirmed
/// snapshot is above the tip, and it is more than [`MIN_LEAD`] blocks above
/// it or `old_blocks_refused` holds. Never without a confirmed snapshot above
/// the tip; a dispute is decided before this is asked.
pub fn offer(
    owned: bool,
    confirmed_height: Option<u64>,
    tip: u64,
    old_blocks_refused: bool,
) -> Option<(u64, OfferReason)> {
    let h = confirmed_height?;
    if !owned || h <= tip {
        return None;
    }
    if h > tip.saturating_add(MIN_LEAD) {
        Some((h, OfferReason::FarBehind))
    } else if old_blocks_refused {
        Some((h, OfferReason::OldBlocksRefused))
    } else {
        None
    }
}

/// The dated folder's name.
pub fn aside_name(now_unix: u64) -> String {
    format!("fast-forward-{now_unix}")
}

/// Move every [`CHAIN_DATA`] entry that exists into a new dated folder and
/// return the record. On any failure, what was moved goes back and the
/// error is returned: the datadir is as it was. Call only with the node
/// stopped.
pub fn set_aside(
    datadir: &Path,
    height: u64,
    snapshot_loaded_before: bool,
    now_unix: u64,
) -> std::io::Result<Record> {
    set_aside_names(
        datadir,
        CHAIN_DATA,
        height,
        snapshot_loaded_before,
        now_unix,
    )
}

fn set_aside_names(
    datadir: &Path,
    names: &[&str],
    height: u64,
    snapshot_loaded_before: bool,
    now_unix: u64,
) -> std::io::Result<Record> {
    let aside = aside_name(now_unix);
    let dir = datadir.join(&aside);
    std::fs::create_dir(&dir)?;
    let mut moved: Vec<String> = Vec::new();
    for name in names {
        let from = datadir.join(name);
        if !from.exists() {
            continue;
        }
        if let Err(e) = std::fs::rename(&from, dir.join(name)) {
            for back in moved.iter().rev() {
                let _ = std::fs::rename(dir.join(back), datadir.join(back));
            }
            let _ = std::fs::remove_dir(&dir);
            return Err(e);
        }
        moved.push(name.to_string());
    }
    Ok(Record {
        height,
        aside,
        moved,
        snapshot_loaded_before,
        started_at: now_unix,
    })
}

/// Undo a run: the chain data the failed attempt made is removed, and what
/// was set aside goes back. The aside folder is checked first, so nothing is
/// removed unless its original is there to replace it. Call only with the
/// node stopped.
pub fn restore(datadir: &Path, record: &Record) -> std::io::Result<()> {
    let dir = datadir.join(&record.aside);
    for name in &record.moved {
        if !dir.join(name).exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} is missing from {}", name, dir.display()),
            ));
        }
    }
    for name in CHAIN_DATA {
        let fresh = datadir.join(name);
        if fresh.is_dir() {
            std::fs::remove_dir_all(&fresh)?;
        } else if fresh.exists() {
            std::fs::remove_file(&fresh)?;
        }
    }
    for name in &record.moved {
        std::fs::rename(dir.join(name), datadir.join(name))?;
    }
    std::fs::remove_dir(&dir)
}

/// A finished run: the set-aside chain data is no longer needed.
pub fn discard(datadir: &Path, record: &Record) -> std::io::Result<()> {
    let dir = datadir.join(&record.aside);
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

pub fn write_record(datadir: &Path, record: &Record) -> std::io::Result<()> {
    let path = record_path(datadir);
    let tmp = path.with_extension("partial");
    std::fs::write(
        &tmp,
        serde_json::to_vec(record).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(tmp, path)
}

pub fn read_record(datadir: &Path) -> Option<Record> {
    serde_json::from_slice(&std::fs::read(record_path(datadir)).ok()?).ok()
}

pub fn clear_record(datadir: &Path) {
    let _ = std::fs::remove_file(record_path(datadir));
}

pub fn write_outcome(datadir: &Path, outcome: &Outcome) {
    if let Ok(bytes) = serde_json::to_vec(outcome) {
        let _ = std::fs::write(result_path(datadir), bytes);
    }
}

pub fn read_outcome(datadir: &Path) -> Option<Outcome> {
    serde_json::from_slice(&std::fs::read(result_path(datadir)).ok()?).ok()
}

pub fn clear_outcome(datadir: &Path) {
    let _ = std::fs::remove_file(result_path(datadir));
}

/// What the node shows, one look at a time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Look {
    /// The height of the snapshot chainstate's base, if the node has one.
    pub snapshot_base_height: Option<u64>,
    /// This launch is a validating node's one mirror launch.
    pub mirror_load_pending: bool,
    /// This launch is a header bootstrap.
    pub header_bootstrap_pending: bool,
    /// The node answers RPC.
    pub running: bool,
    /// The load path reported a failure for this run.
    pub load_failed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Continue,
    Done,
    RollBack(String),
}

/// Pure: is the run done, still going, or to be rolled back? Done means a
/// snapshot at or above the run's is loaded (the start path may have found a
/// newer confirmed one) and the node runs its ordinary launch again: no
/// mirror launch and no header bootstrap pending. A snapshot below the run's
/// on that ordinary launch means the start path took a fallback (the
/// operators began to disagree after the check, or the pair was refused):
/// rolled back at once.
pub fn judge(record: &Record, look: &Look, now_unix: u64) -> Verdict {
    if let Some(why) = &look.load_failed {
        return Verdict::RollBack(why.clone());
    }
    let ordinary = look.running && !look.mirror_load_pending && !look.header_bootstrap_pending;
    match look.snapshot_base_height {
        Some(h) if ordinary && h >= record.height => return Verdict::Done,
        Some(h) if ordinary => {
            return Verdict::RollBack(format!(
                "the node started from block {} instead of the confirmed snapshot at block {}",
                copy::height(h),
                copy::height(record.height)
            ))
        }
        _ => {}
    }
    if now_unix.saturating_sub(record.started_at) >= MAX_RUN_SECS {
        return Verdict::RollBack(format!(
            "it did not finish within {} hours",
            MAX_RUN_SECS / 3600
        ));
    }
    Verdict::Continue
}

/// The sentences the Tools overlay shows. One place, so a test can hold them
/// to the copy rules.
pub mod copy {
    use crate::snapshot_start::{block_number, join_names};

    /// "233,800".
    pub fn height(h: u64) -> String {
        block_number(h)
    }

    pub fn button(h: u64) -> String {
        format!("Fast-forward to block {}", height(h))
    }

    /// What the first click shows: the decision's prompt (section 7, with
    /// the operators whose signatures this app verified, in the list's
    /// order), then what stays and what a wallet used below the snapshot
    /// does (section 10). The Tools decision, section 3, fixes where it
    /// sits and that a second click within ten seconds runs it.
    pub fn confirm(h: u64, operators: &[String]) -> String {
        let at = height(h);
        format!(
            "Fast-forward to block {at}, confirmed by {}? Your node stops for a few minutes to \
             load it, then carries on from there. Your wallets and keys stay as they are, and \
             afterwards the node checks the older history in the background. A wallet you last \
             used before block {at} opens once that check gets there. Click again to start.",
            join_names(operators)
        )
    }

    /// The line above the button: why it is there. Early, it says why and
    /// promises nothing a normal run does not.
    pub fn note(why: super::OfferReason) -> String {
        match why {
            super::OfferReason::FarBehind => {
                "A confirmed snapshot is far ahead of your node.".into()
            }
            super::OfferReason::OldBlocksRefused => {
                "Your node's peers are not sending the older blocks it needs, so Fast-forward \
                 is offered sooner than usual. It works the same way as always."
                    .into()
            }
        }
    }

    /// While `latest` answers a dispute (section 6a): the one sentence,
    /// naming the newest disputed height.
    pub fn off(newest_disputed: u64) -> String {
        format!(
            "Fast-forward is off while the snapshot operators disagree about block {}.",
            height(newest_disputed)
        )
    }

    pub fn running() -> String {
        "Fast-forward is running. Your node restarts a few times on the way.".into()
    }

    pub fn done(h: u64, operators: &[String]) -> String {
        if operators.is_empty() {
            format!("Done. Your node now starts from block {}.", height(h))
        } else {
            format!(
                "Done. Your node now starts from block {}, confirmed by {}.",
                height(h),
                join_names(operators)
            )
        }
    }

    pub fn rolled_back(reason: &str) -> String {
        format!("Fast-forward stopped and your node is back as it was. What happened: {reason}.")
    }
}
````

- [ ] **Step 4: Run the tests to see them pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- fast_forward`
Expected: `test result: ok. 9 passed; 0 failed` (the first version's eight with the cases above, and `the_button_shows_sooner_when_no_archive_peer_serves_old_blocks`; derived, not run).

- [ ] **Step 5: Format, lint, commit**

````bash
cd crates/btx-core
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
cd ../..
````
````bash
git add crates/btx-core/src/fast_forward.rs crates/btx-core/src/lib.rs
git commit -m "core: Fast-forward sets the chain data aside and can always put it back" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 2: Is there a confirmed snapshot, how high, and who confirmed it? (`peek_confirmed`)

**Files:**
- Modify: `crates/btx-core/src/attested_snapshot.rs`

**Interfaces:**
- Consumes: plan 1's `parse_latest`, `Latest`, `check_pointer`, `pointer_matches`, `cs::{parse, check, MAX_MANIFEST_BYTES}`, `MAX_POINTER_BYTES`; test helpers `regtest_pointer`, `view`, `any_url`, `R_PC`, `R_DAT`, `P`, `C` (plan 1, Task 3).
- Produces (Task 4):
  - `pub enum Peek { Confirmed { height: u64, operators: Vec<String> }, Disputed { newest: u64 } }` (`operators`: the ones whose signatures this app verified in the manifest, in the list's order)
  - `pub async fn peek_confirmed(&reqwest::Client, pointer_url: &str, &NodeView, regtest_env: Option<&str>, url_ok: fn(&str) -> bool) -> Result<Peek, String>`

- [ ] **Step 1: Write the failing test**

In `crates/btx-core/src/attested_snapshot.rs`, inside `mod tests`, directly before `#[tokio::test] async fn no_confirmed_snapshot_is_a_plain_404()`, add:

````rust
    #[tokio::test]
    async fn peeking_checks_the_manifest_and_never_fetches_the_file() {
        let mut server = mockito::Server::new_async().await;
        let p = regtest_pointer(&server.url(), R_PC, R_DAT);
        server
            .mock("GET", "/latest")
            .with_body(serde_json::to_vec(&p).unwrap())
            .create_async()
            .await;
        server
            .mock("GET", "/m")
            .with_body(R_PC)
            .create_async()
            .await;
        let file = server.mock("GET", "/f").expect(0).create_async().await;
        let env = format!("producer={P};confirmer={C}");
        let url = format!("{}/latest", server.url());
        let client = reqwest::Client::new();
        assert_eq!(
            peek_confirmed(&client, &url, &view(), Some(&env), any_url).await,
            Ok(Peek::Confirmed {
                height: 100,
                operators: vec!["producer".into(), "confirmer".into()],
            }),
            "the names are the verified signers, not the pointer's"
        );
        let one = format!("producer={P}");
        assert!(peek_confirmed(&client, &url, &view(), Some(&one), any_url)
            .await
            .unwrap_err()
            .contains("two are needed"));
        file.assert_async().await;
    }

    /// Section 6a: a dispute is an answer, not an error. Fast-forward says
    /// it is off and names the newest disputed height; nothing is fetched.
    #[tokio::test]
    async fn peeking_a_dispute_names_its_newest_height_and_fetches_nothing() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/latest")
            .with_body(r#"{"disputed":[233700,233800]}"#)
            .create_async()
            .await;
        let anything_else = server
            .mock("GET", mockito::Matcher::Regex("^/(m|f)".into()))
            .expect(0)
            .create_async()
            .await;
        let url = format!("{}/latest", server.url());
        assert_eq!(
            peek_confirmed(&reqwest::Client::new(), &url, &view(), None, any_url).await,
            Ok(Peek::Disputed { newest: 233_800 })
        );
        anything_else.assert_async().await;
    }
````

- [ ] **Step 2: Run it to see it fail**

Run: `cd crates/btx-core && cargo test --locked --lib -- attested_snapshot::tests::peeking`
Expected: compile error `cannot find function `peek_confirmed``.

- [ ] **Step 3: Write the implementation**

In `crates/btx-core/src/attested_snapshot.rs`, directly before `/// The pair this node should start from, verified on disk, or `None` for the`, add:

````rust
async fn get_capped(client: &reqwest::Client, url: &str, cap: usize) -> Result<Vec<u8>, String> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("unreachable: {e}"))?;
    if resp.status().as_u16() != 200 {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("read: {e}"))? {
        body.extend_from_slice(&chunk);
        if body.len() > cap {
            return Err(format!("larger than {cap} bytes"));
        }
    }
    Ok(body)
}

/// What `latest` says, checked as [`prepare_confirmed`] checks it, without
/// the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peek {
    /// A confirmed snapshot at `height`, and the operators whose signatures
    /// this app verified, in the list's order. The pointer's own list is
    /// never used.
    Confirmed { height: u64, operators: Vec<String> },
    /// The operators disagree (section 6a); the newest disputed height.
    Disputed { newest: u64 },
}

/// The confirmed snapshot `latest` names, after the same checks
/// [`prepare_confirmed`] makes on the answer and the manifest, and without
/// downloading the file; or the dispute it answers instead. For deciding
/// what the Fast-forward section shows.
pub async fn peek_confirmed(
    client: &reqwest::Client,
    pointer_url: &str,
    view: &NodeView,
    regtest_env: Option<&str>,
    url_ok: fn(&str) -> bool,
) -> Result<Peek, String> {
    let body = get_capped(client, pointer_url, MAX_POINTER_BYTES)
        .await
        .map_err(|e| format!("no confirmed snapshot ({e})"))?;
    let p = match parse_latest(&body)? {
        Latest::Confirmed(p) => p,
        disputed @ Latest::Disputed(_) => {
            return Ok(Peek::Disputed {
                newest: disputed.newest_disputed().unwrap_or(0),
            })
        }
    };
    check_pointer(&p, url_ok)?;
    let bytes = get_capped(client, &p.manifest_url, cs::MAX_MANIFEST_BYTES)
        .await
        .map_err(|e| format!("manifest: {e}"))?;
    let sha = {
        use sha2::{Digest, Sha256};
        crate::operators::hex(&Sha256::digest(&bytes))
    };
    if bytes.len() as u64 != p.manifest_size || !sha.eq_ignore_ascii_case(&p.manifest_sha256) {
        return Err("the manifest is not the one the pointer names".into());
    }
    let confirmed = cs::parse(&bytes)
        .and_then(|m| cs::check(&m, view, regtest_env))
        .map_err(|e| format!("not confirmed: {e}"))?;
    pointer_matches(&p, &confirmed)?;
    Ok(Peek::Confirmed {
        height: confirmed.height,
        operators: confirmed.operators,
    })
}
````

- [ ] **Step 4: Run it to see it pass**

Run: `cd crates/btx-core && cargo test --locked --lib -- attested_snapshot`
Expected: `17 passed; 0 failed; 1 ignored` (plan 1's 15 and these two; derived, not run).

- [ ] **Step 5: Format, lint, commit**

````bash
cd crates/btx-core
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
cd ../..
````
````bash
git add crates/btx-core/src/attested_snapshot.rs
git commit -m "core: peek at the confirmed snapshot, or the dispute, without downloading anything" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 3: The driver, and the start path during a run (`apps/node/src-tauri/src/fast_forward.rs`, `commands.rs`)

**Files:**
- Create: `apps/node/src-tauri/src/fast_forward.rs`
- Modify: `apps/node/src-tauri/src/commands.rs` (`set_phase` and `SIGNED_LOAD_FAILED` widened to `pub(crate)`; `signed_load_for` gains `fast_forward`; `spawn_load_watch` and `after_snapshot_load` carry the `SignedLoad`; failures during a run go to the driver; the start path resumes a run)
- Modify: `apps/node/src-tauri/src/lib.rs` (module)

**Interfaces:**
- Consumes: Task 1 (`fast_forward::{set_aside, restore, discard, write_record, read_record, clear_record, write_outcome, clear_outcome, judge, Look, Verdict, Outcome, Record}`), plan 1 (`attested_snapshot::{http_client, prepare_confirmed, prune_others, fallback_start, confirmed_url_allowed, CONFIRMED_POINTER_URL}`, `snapshot_start::{read, StartSource}`, `confirmed_load::node_view`, `node::{mirror_load_marker_exists, header_bootstrap_pending, end_mirror_load, end_header_bootstrap, BTX_TRUSTED_ATTESTATION_PUBKEYS}`, `snapshot::{clear_snapshot_marker, mark_snapshot_marker, SignedLoad, SnapshotOutcome}`, `node_api::get_chainstates`, `operators::regtest_env`, the app's `stop_node_inner`, `start_node_projected`, `snapshot_spec`, `NodeAppSettings`, `NodePhase`).
- Produces (Task 4): `crate::fast_forward::{spawn_run(AppHandle) -> Result<(), String>, resume_if_needed(&AppHandle), active() -> bool, report_failure(String)}`; `commands::{set_phase, SIGNED_LOAD_FAILED}` now `pub(crate)`; `signed_load_for(mirror_load_launch, follows_signatures, failed_this_run, fast_forward: bool)`.

How a run goes (section 10): (1) the confirmed pair is checked and downloaded while the node runs (a disputed `latest` stops it here, and nothing moves); (2) the node stops, the chain data and the start record move aside, the run's record is written, the "snapshot loaded" setting is reset so the loaders load again; (3) the node starts, and plan 1's start path does the rest: an empty folder gets its header bootstrap, a validating node its one mirror launch; during a run a node that follows signatures loads signed-only (no compiled fallback), and a failed signed load is reported to the driver instead of restarting; the load writes the new start record; (4) the driver looks every 5 s: done (a snapshot at or above the run's height, ordinary launch) discards the dated folder and keeps who confirmed it for the overlay; a reported failure, a snapshot below the run's height on the ordinary launch, or 3 hours rolls back. A quit leaves the record; the next start resumes the watch.

- [ ] **Step 1: Write the failing test**

In `apps/node/src-tauri/src/commands.rs`, in `mod signed_start_tests`, replace the test `each_launch_makes_the_signed_load_its_node_can_make` (from its `#[test]` line up to the `#[test]` line of `the_end_of_a_mirror_launch_says_what_happens_next`) with:

````rust
    #[test]
    fn each_launch_makes_the_signed_load_its_node_can_make() {
        assert_eq!(
            signed_load_for(true, false, false, false),
            SignedLoad::SignedOnly
        );
        assert_eq!(
            signed_load_for(true, true, true, false),
            SignedLoad::SignedOnly
        );
        assert_eq!(
            signed_load_for(false, true, false, false),
            SignedLoad::Mirror
        );
        assert_eq!(
            signed_load_for(false, true, true, false),
            SignedLoad::None,
            "a mirror whose signed load failed this run takes the compiled one"
        );
        assert_eq!(
            signed_load_for(false, false, false, false),
            SignedLoad::None
        );
        // Fast-forward: a mirror loads the signed pair or nothing, and the
        // driver rolls back on nothing; a validating node's ordinary launch
        // loads nothing signed (its mirror launch does).
        assert_eq!(
            signed_load_for(false, true, true, true),
            SignedLoad::SignedOnly
        );
        assert_eq!(signed_load_for(false, false, false, true), SignedLoad::None);
    }
````

- [ ] **Step 2: Run it to see it fail**

Run: `cd apps/node/src-tauri && cargo test --locked -- signed_start_tests`
Expected: compile error `this function takes 3 arguments but 4 arguments were supplied`.

- [ ] **Step 3: Create the driver**

Create `apps/node/src-tauri/src/fast_forward.rs`:

````rust
//! Fast-forward, driven: check and download the confirmed snapshot while the
//! node runs, stop it, set the chain data aside, start it again and let the
//! start path load the snapshot (the one loading path every node uses), then
//! keep the new chain or put the old one back.
//! docs/decisions/2026-09-29-every-node-starts-near-the-tip.md, section 10.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use btx_core::fast_forward::{self as ff, Look, Outcome, Record, Verdict};
use tauri::{AppHandle, Manager, State};

use crate::commands::{
    set_phase, snapshot_spec, start_node_projected, stop_node_inner, SIGNED_LOAD_FAILED,
};
use crate::state::{node_datadir, AppState, NodeAppSettings, NodePhase};

/// One driver at a time.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Set by the start path when a load failed during a run; the driver takes it
/// and rolls back.
pub(crate) static FAILURE: Mutex<Option<String>> = Mutex::new(None);

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A run is under way on this datadir (its record is on disk).
pub(crate) fn active() -> bool {
    ff::read_record(&node_datadir()).is_some()
}

/// Tell the driver the load failed. Called by the start path instead of its
/// own restart while a run is active.
pub(crate) fn report_failure(why: String) {
    *FAILURE.lock().unwrap_or_else(|e| e.into_inner()) = Some(why);
}

/// Start a run. Returns at once; the Tools overlay asks for the status.
pub(crate) fn spawn_run(app: AppHandle) -> Result<(), String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Fast-forward is already running.".into());
    }
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        run(&app, &state).await;
        RUNNING.store(false, Ordering::SeqCst);
    });
    Ok(())
}

/// A record with no driver (the app quit during a run): watch it to its end.
pub(crate) fn resume_if_needed(app: &AppHandle) {
    let Some(record) = ff::read_record(&node_datadir()) else {
        return;
    };
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        watch(&app, &state, record).await;
        RUNNING.store(false, Ordering::SeqCst);
    });
}

async fn run(app: &AppHandle, state: &State<'_, AppState>) {
    let datadir = node_datadir();
    ff::clear_outcome(&datadir);
    *FAILURE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // A person asked for this: a signed load that failed earlier in this run
    // of the app does not stand in its way.
    SIGNED_LOAD_FAILED.store(false, Ordering::SeqCst);

    // Section 10, step 1: check and download while the node keeps running.
    let Some(rpc) = state.rpc.lock().await.clone() else {
        ff::write_outcome(
            &datadir,
            &Outcome::RolledBack {
                reason: "the node was not running".into(),
            },
        );
        return;
    };
    let anchor = snapshot_spec().anchor_height;
    let view = btx_core::confirmed_load::node_view(
        &rpc,
        &btx_core::node::BTX_TRUSTED_ATTESTATION_PUBKEYS,
        btx_core::attested_snapshot::fallback_start(anchor),
    )
    .await;
    let prepared = match btx_core::attested_snapshot::http_client() {
        Ok(client) => {
            btx_core::attested_snapshot::prepare_confirmed(
                &client,
                btx_core::attested_snapshot::CONFIRMED_POINTER_URL,
                &datadir,
                &view,
                btx_core::operators::regtest_env().as_deref(),
                btx_core::attested_snapshot::confirmed_url_allowed,
            )
            .await
        }
        Err(e) => Err(e),
    };
    let pair = match prepared {
        Ok(p) => p,
        Err(e) => {
            ff::write_outcome(
                &datadir,
                &Outcome::RolledBack {
                    reason: format!("the confirmed snapshot did not check out ({e})"),
                },
            );
            return;
        }
    };
    btx_core::attested_snapshot::prune_others(&datadir, pair.height);

    // Step 2: stop, and set the chain data aside.
    stop_node_inner(state).await;
    set_phase(app, state, NodePhase::Stopped).await;
    let loaded_before = NodeAppSettings::load(&datadir).snapshot_loaded;
    let record = match ff::set_aside(&datadir, pair.height, loaded_before, now()) {
        Ok(r) => r,
        Err(e) => {
            ff::write_outcome(
                &datadir,
                &Outcome::RolledBack {
                    reason: format!("the chain data could not be set aside ({e})"),
                },
            );
            let _ = start_node_projected(app, state).await;
            return;
        }
    };
    if let Err(e) = ff::write_record(&datadir, &record) {
        let _ = ff::restore(&datadir, &record);
        ff::write_outcome(
            &datadir,
            &Outcome::RolledBack {
                reason: format!("the run could not be recorded ({e})"),
            },
        );
        let _ = start_node_projected(app, state).await;
        return;
    }
    NodeAppSettings::update(&datadir, |s| s.snapshot_loaded = false);
    btx_core::snapshot::clear_snapshot_marker(&datadir);

    // Step 3: start. The start path does the rest: the header bootstrap of
    // an empty datadir, the one mirror launch of a validating node, the load
    // with every check, and the restart as a validating node.
    if let Err(e) = start_node_projected(app, state).await {
        roll_back(app, state, &record, format!("the node did not start ({e})")).await;
        return;
    }
    watch(app, state, record).await;
}

async fn look(state: &State<'_, AppState>) -> Look {
    let datadir = node_datadir();
    let rpc = state.rpc.lock().await.clone();
    let mut snapshot_base_height = None;
    if let Some(rpc) = &rpc {
        if let Ok(cs) = btx_core::node_api::get_chainstates(rpc).await {
            if let Some(hash) = cs.snapshot().and_then(|c| c.snapshot_blockhash.clone()) {
                use btx_core::rpc::Rpc;
                snapshot_base_height = rpc
                    .call("getblockheader", serde_json::json!([hash, true]))
                    .await
                    .ok()
                    .and_then(|h| h["height"].as_u64());
            }
        }
    }
    Look {
        snapshot_base_height,
        mirror_load_pending: btx_core::node::mirror_load_marker_exists(&datadir),
        header_bootstrap_pending: btx_core::node::header_bootstrap_pending(&datadir),
        running: rpc.is_some(),
        load_failed: FAILURE.lock().unwrap_or_else(|e| e.into_inner()).take(),
    }
}

/// Step 4: poll until the run is done or has to be undone.
async fn watch(app: &AppHandle, state: &State<'_, AppState>, record: Record) {
    let datadir = node_datadir();
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        if state.quitting.load(Ordering::SeqCst) {
            return; // the record stays; the next start resumes the watch
        }
        match ff::judge(&record, &look(state).await, now()) {
            Verdict::Continue => {}
            Verdict::Done => {
                if let Err(e) = ff::discard(&datadir, &record) {
                    eprintln!(
                        "[fast-forward] could not remove {}: {e}",
                        datadir.join(&record.aside).display()
                    );
                }
                ff::clear_record(&datadir);
                // Who confirmed it: the start record the load wrote (plan 1,
                // Task 2b), which holds only operators whose signatures this
                // app verified. The load may have found a newer snapshot.
                let start = btx_core::snapshot_start::read(&datadir).filter(|s| {
                    s.source == btx_core::snapshot_start::StartSource::Confirmed
                        && s.height >= record.height
                });
                ff::write_outcome(
                    &datadir,
                    &Outcome::Done {
                        height: start.as_ref().map_or(record.height, |s| s.height),
                        operators: start.map(|s| s.operators).unwrap_or_default(),
                    },
                );
                return;
            }
            Verdict::RollBack(why) => {
                roll_back(app, state, &record, why).await;
                return;
            }
        }
    }
}

/// Stop, put the old chain data back, start as before, and say why.
async fn roll_back(app: &AppHandle, state: &State<'_, AppState>, record: &Record, why: String) {
    let datadir = node_datadir();
    eprintln!("[fast-forward] rolling back: {why}");
    stop_node_inner(state).await;
    set_phase(app, state, NodePhase::Stopped).await;
    btx_core::node::end_mirror_load(&datadir);
    btx_core::node::end_header_bootstrap(&datadir);
    let reason = match ff::restore(&datadir, record) {
        Ok(()) => {
            NodeAppSettings::update(&datadir, |s| {
                s.snapshot_loaded = record.snapshot_loaded_before
            });
            if record.snapshot_loaded_before {
                btx_core::snapshot::mark_snapshot_marker(&datadir);
            }
            why
        }
        Err(e) => format!(
            "{why}; the old chain data could not be put back ({e}) and is in {}",
            datadir.join(&record.aside).display()
        ),
    };
    ff::clear_record(&datadir);
    ff::write_outcome(&datadir, &Outcome::RolledBack { reason });
    let _ = start_node_projected(app, state).await;
}
````

In `apps/node/src-tauri/src/lib.rs`, after `mod commands;`:

````rust
mod commands;
mod fast_forward;
````

- [ ] **Step 4: Open two items of `commands.rs` to the driver**

Replace `async fn set_phase(app: &AppHandle, state: &AppState, phase: NodePhase) {` (`commands.rs:462` on `origin/main`) with:

````rust
pub(crate) async fn set_phase(app: &AppHandle, state: &AppState, phase: NodePhase) {
````

Replace:

````rust
static SIGNED_LOAD_FAILED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
````

with:

````rust
pub(crate) static SIGNED_LOAD_FAILED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
````

- [ ] **Step 5: Signed-only during a run**

Replace the whole `fn signed_load_for` (plan 1's):

````rust
fn signed_load_for(
    mirror_load_launch: bool,
    follows_signatures: bool,
    failed_this_run: bool,
) -> btx_core::snapshot::SignedLoad {
    use btx_core::snapshot::SignedLoad;
    if mirror_load_launch {
        SignedLoad::SignedOnly
    } else if follows_signatures && !failed_this_run {
        SignedLoad::Mirror
    } else {
        SignedLoad::None
    }
}
````

with:

````rust
fn signed_load_for(
    mirror_load_launch: bool,
    follows_signatures: bool,
    failed_this_run: bool,
    fast_forward: bool,
) -> btx_core::snapshot::SignedLoad {
    use btx_core::snapshot::SignedLoad;
    if mirror_load_launch || (fast_forward && follows_signatures) {
        SignedLoad::SignedOnly
    } else if follows_signatures && !failed_this_run {
        SignedLoad::Mirror
    } else {
        SignedLoad::None
    }
}
````

- [ ] **Step 6: Failures during a run go to the driver**

Replace the whole `fn spawn_load_watch`:

````rust
fn spawn_load_watch(
    app: AppHandle,
    handle: tokio::task::JoinHandle<btx_core::snapshot::SnapshotOutcome>,
    gen: u64,
    mirror_load_launch: bool,
) {
    tauri::async_runtime::spawn(async move {
        let outcome = handle.await.unwrap_or_else(|e| {
            btx_core::snapshot::SnapshotOutcome::NotLoaded(format!("the load task ended: {e}"))
        });
        let state = app.state::<AppState>();
        if let Err(e) = after_snapshot_load(&app, &state, gen, mirror_load_launch, outcome).await {
            eprintln!("[node-app] the restart after a snapshot load failed: {e}");
        }
    });
}
````

with:

````rust
fn spawn_load_watch(
    app: AppHandle,
    handle: tokio::task::JoinHandle<btx_core::snapshot::SnapshotOutcome>,
    gen: u64,
    signed: btx_core::snapshot::SignedLoad,
    mirror_load_launch: bool,
) {
    tauri::async_runtime::spawn(async move {
        let outcome = handle.await.unwrap_or_else(|e| {
            btx_core::snapshot::SnapshotOutcome::NotLoaded(format!("the load task ended: {e}"))
        });
        let state = app.state::<AppState>();
        if let Err(e) =
            after_snapshot_load(&app, &state, gen, signed, mirror_load_launch, outcome).await
        {
            eprintln!("[node-app] the restart after a snapshot load failed: {e}");
        }
    });
}
````

Then replace the head of `after_snapshot_load` (up to, not including, `    if !mirror_load_launch && !held {`):

````rust
async fn after_snapshot_load(
    app: &AppHandle,
    state: &State<'_, AppState>,
    gen: u64,
    mirror_load_launch: bool,
    outcome: btx_core::snapshot::SnapshotOutcome,
) -> Result<(), String> {
    use btx_core::snapshot::SnapshotOutcome as O;
    let held = matches!(outcome, O::HeldRootOnChain(_));
````

with:

````rust
async fn after_snapshot_load(
    app: &AppHandle,
    state: &State<'_, AppState>,
    gen: u64,
    signed: btx_core::snapshot::SignedLoad,
    mirror_load_launch: bool,
    outcome: btx_core::snapshot::SnapshotOutcome,
) -> Result<(), String> {
    use btx_core::snapshot::SnapshotOutcome as O;
    let held = matches!(outcome, O::HeldRootOnChain(_));
    // During Fast-forward a signed-only load that failed is the driver's to
    // undo (`crate::fast_forward`), not a reason for the restart below.
    if crate::fast_forward::active()
        && signed == btx_core::snapshot::SignedLoad::SignedOnly
        && matches!(outcome, O::NotLoaded(_) | O::HeldRootOnChain(_))
    {
        crate::fast_forward::report_failure(mirror_load_end_message(&outcome));
        return Ok(());
    }
````

- [ ] **Step 7: The start path passes the run on, and resumes a run**

In `start_node_inner`, replace (plan 1's version):

````rust
    let mut load_watch = None;
    if bootstrap_launch {
        eprintln!(
            "[snapshot] header bootstrap launch: the snapshot loads after the restart that ends it"
        );
    } else {
        // Where this node starts (the confirmed-snapshot decision, sections 7
        // and 9): a validating node's one mirror launch loads a confirmed or
        // the pinned pair and is then restarted as a validating node; a node
        // that follows signatures loads one in place, else the compiled one;
        // an ordinary validating launch loads the compiled one.
        let mut mirror_load_launch = btx_core::node::mirror_load_pending(&datadir).is_some();
        if mirror_load_launch && !attached_node_is_ours_to_stop(*state.attached_to.lock().await) {
            // Another app's node never read the marker, and is not ours to restart.
            btx_core::node::end_mirror_load(&datadir);
            mirror_load_launch = false;
        }
        let signed = signed_load_for(
            mirror_load_launch,
            btx_core::node::host_follows_signatures(&paths.btxd, &datadir, node_backend()),
            SIGNED_LOAD_FAILED.load(Ordering::SeqCst),
        );
        let handle = btx_core::snapshot::ensure_snapshot_loaded_with(
            rpc.clone(),
            paths.btx_cli.clone(),
            datadir.clone(),
            spec.anchor_height,
            Arc::new(NodeAppSnapshotFlags {
                datadir: datadir.clone(),
            }),
            signed,
        );
        load_watch = Some((handle, mirror_load_launch));
    }

    set_phase(app, state, NodePhase::LoadingSnapshot).await;
    spawn_status_refresher(app.clone(), state, bootstrap_launch);
    if let Some((handle, mirror_load_launch)) = load_watch {
        // This run's generation: a stop or restart moves it, and then the
        // outcome is no longer this run's to act on.
        let gen = state.refresher_gen.load(Ordering::SeqCst);
        spawn_load_watch(app.clone(), handle, gen, mirror_load_launch);
    }
````

with:

````rust
    let mut load_watch = None;
    if bootstrap_launch {
        eprintln!(
            "[snapshot] header bootstrap launch: the snapshot loads after the restart that ends it"
        );
    } else {
        // Where this node starts (the confirmed-snapshot decision, sections 7
        // and 9): a validating node's one mirror launch loads a confirmed or
        // the pinned pair and is then restarted as a validating node; a node
        // that follows signatures loads one in place, else the compiled one;
        // an ordinary validating launch loads the compiled one.
        let mut mirror_load_launch = btx_core::node::mirror_load_pending(&datadir).is_some();
        if mirror_load_launch && !attached_node_is_ours_to_stop(*state.attached_to.lock().await) {
            // Another app's node never read the marker, and is not ours to restart.
            btx_core::node::end_mirror_load(&datadir);
            mirror_load_launch = false;
        }
        let signed = signed_load_for(
            mirror_load_launch,
            btx_core::node::host_follows_signatures(&paths.btxd, &datadir, node_backend()),
            SIGNED_LOAD_FAILED.load(Ordering::SeqCst),
            crate::fast_forward::active(),
        );
        let handle = btx_core::snapshot::ensure_snapshot_loaded_with(
            rpc.clone(),
            paths.btx_cli.clone(),
            datadir.clone(),
            spec.anchor_height,
            Arc::new(NodeAppSnapshotFlags {
                datadir: datadir.clone(),
            }),
            signed,
        );
        load_watch = Some((handle, signed, mirror_load_launch));
    }

    set_phase(app, state, NodePhase::LoadingSnapshot).await;
    spawn_status_refresher(app.clone(), state, bootstrap_launch);
    if let Some((handle, signed, mirror_load_launch)) = load_watch {
        // This run's generation: a stop or restart moves it, and then the
        // outcome is no longer this run's to act on.
        let gen = state.refresher_gen.load(Ordering::SeqCst);
        spawn_load_watch(app.clone(), handle, gen, signed, mirror_load_launch);
    }
    // A Fast-forward the app quit in the middle of is watched to its end.
    crate::fast_forward::resume_if_needed(app);
````

- [ ] **Step 8: Run the tests to see them pass**

Run: `cd apps/node/src-tauri && cargo test --locked`
Expected: all pass, `signed_start_tests` included. (`crate::fast_forward::spawn_run` is unused until Task 4; a `dead_code` warning for it is expected here and gone after Task 4.)

- [ ] **Step 9: Format, lint, commit**

````bash
cd apps/node/src-tauri
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
cd ../../..
````
````bash
git add apps/node/src-tauri/src/fast_forward.rs apps/node/src-tauri/src/commands.rs apps/node/src-tauri/src/lib.rs
git commit -m "node: the Fast-forward driver, and the start path hands it every failed load during a run" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 4: The Tools commands (`tools.rs`)

**Files:**
- Modify: `apps/node/src-tauri/src/tools.rs` (three commands, a pure helper and its tests at the end), `apps/node/src-tauri/src/lib.rs` (registration)

**Interfaces:**
- Consumes: Task 1 (`fast_forward::{offer, OfferReason, copy, read_outcome, Outcome}`), Task 2 (`peek_confirmed`, `Peek`), Task 3 (`crate::fast_forward::{spawn_run, active}`), plan 1 (`confirmed_load::node_view`, `attested_snapshot::{http_client, fallback_start, CONFIRMED_POINTER_URL, confirmed_url_allowed}`), the catch-up plan (`AppState::catch_up_help: Arc<tokio::sync::Mutex<btx_core::catchup_assist::CatchUpReport>>`, right after `signed_frontier` in `state.rs`, and its `no_archive_serves_old_blocks: bool`; that plan lands before this one), existing (on `origin/main`) `destructive_allowed` and `node_ownership` (`commands.rs:4621` and `:4645`, imported at `tools.rs:21`), `rpc_handle` (`tools.rs:227`), `api::get_blockchain_info`, `crate::commands::snapshot_spec`.
- Produces (Task 5, the window):
  - `tools_fast_forward_check() -> FastForwardCheck`, serialized as `{ kind: "none" } | { kind: "offer", height: number, button: string, confirm: string, note: string } | { kind: "off", height: number, sentence: string }`
  - `tools_fast_forward_run() -> string` (the "running" sentence, or an error string), `tools_fast_forward_status() -> { running: boolean, message: string | null }`
  - `fn fast_forward_check(owned: bool, peek: Option<Peek>, tip: u64, old_blocks_refused: bool) -> FastForwardCheck` (pure, tested here)

The commands are glue over tested pure parts (`offer`, `copy`, `judge`, `peek_confirmed`, `fast_forward_check`); the rest is checked by compile, by the window's tests in Task 5, and by hand in Task 6.

- [ ] **Step 1: Add the commands**

At the end of `apps/node/src-tauri/src/tools.rs` (after the `#[cfg(test)] mod tests { ... }` block, which starts at line 606 on `origin/main`, is fine; items after a test module compile the same), add:

````rust
// ── Fast-forward (docs/decisions/2026-09-29-every-node-starts-near-the-tip.md,
// sections 6a and 10; the button: the Tools decision, section 3) ────────────

/// What the Fast-forward section shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FastForwardCheck {
    /// Nothing to show.
    None,
    /// The button, the words its first click shows, and the line above it.
    Offer {
        height: u64,
        button: String,
        confirm: String,
        note: String,
    },
    /// The snapshot operators disagree: the section says Fast-forward is off.
    Off { height: u64, sentence: String },
}

/// Pure: what the section shows, from what `latest` said, the node's tip and
/// whether the catch-up help has concluded that no archive peer serves this
/// node old blocks. A dispute shows on any node the app owns, whatever the
/// flag; the button only on one more than 1,000 blocks behind a confirmed
/// snapshot, or any distance behind one while the flag holds.
fn fast_forward_check(
    owned: bool,
    peek: Option<btx_core::attested_snapshot::Peek>,
    tip: u64,
    old_blocks_refused: bool,
) -> FastForwardCheck {
    use btx_core::attested_snapshot::Peek;
    use btx_core::fast_forward::{copy, offer};
    match peek {
        Some(Peek::Disputed { newest }) if owned => FastForwardCheck::Off {
            height: newest,
            sentence: copy::off(newest),
        },
        Some(Peek::Confirmed { height, operators }) => {
            match offer(owned, Some(height), tip, old_blocks_refused) {
                Some((h, why)) => FastForwardCheck::Offer {
                    height: h,
                    button: copy::button(h),
                    confirm: copy::confirm(h, &operators),
                    note: copy::note(why),
                },
                None => FastForwardCheck::None,
            }
        }
        _ => FastForwardCheck::None,
    }
}

/// The section's state: the button when the app owns the node, no run is
/// under way, and a confirmed snapshot (checked as the loader checks it, but
/// without its file) is more than 1,000 blocks above the tip, or above it at
/// all while no archive peer serves this node old blocks; the dispute
/// sentence while the operators disagree; else nothing.
#[tauri::command]
pub async fn tools_fast_forward_check(
    state: State<'_, AppState>,
) -> Result<FastForwardCheck, String> {
    let owned = destructive_allowed(node_ownership(&state, &node_datadir()).await).is_ok();
    if !owned || crate::fast_forward::active() {
        return Ok(FastForwardCheck::None);
    }
    let Some(rpc) = rpc_handle(&state).await else {
        return Ok(FastForwardCheck::None);
    };
    let tip = match api::get_blockchain_info(&rpc).await {
        Ok(info) => info.blocks,
        Err(_) => return Ok(FastForwardCheck::None),
    };
    let anchor = crate::commands::snapshot_spec().anchor_height;
    let view = btx_core::confirmed_load::node_view(
        &rpc,
        &btx_core::node::BTX_TRUSTED_ATTESTATION_PUBKEYS,
        btx_core::attested_snapshot::fallback_start(anchor),
    )
    .await;
    let client = btx_core::attested_snapshot::http_client()?;
    let peek = btx_core::attested_snapshot::peek_confirmed(
        &client,
        btx_core::attested_snapshot::CONFIRMED_POINTER_URL,
        &view,
        btx_core::operators::regtest_env().as_deref(),
        btx_core::attested_snapshot::confirmed_url_allowed,
    )
    .await
    .ok();
    // The catch-up help's conclusion as of the refresher's last tick (the
    // catch-up plan): `false` after every start and stop until it concludes.
    let old_blocks_refused = state.catch_up_help.lock().await.no_archive_serves_old_blocks;
    Ok(fast_forward_check(owned, peek, tip, old_blocks_refused))
}

/// The second click. Starts the run and returns; the overlay asks for the
/// status until it ends.
#[tauri::command]
pub async fn tools_fast_forward_run(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    destructive_allowed(node_ownership(&state, &node_datadir()).await)?;
    crate::fast_forward::spawn_run(app)?;
    Ok(btx_core::fast_forward::copy::running())
}

#[derive(Debug, Clone, Serialize)]
pub struct FastForwardStatus {
    pub running: bool,
    pub message: Option<String>,
}

#[tauri::command]
pub async fn tools_fast_forward_status() -> Result<FastForwardStatus, String> {
    use btx_core::fast_forward::{copy, read_outcome, Outcome};
    let datadir = node_datadir();
    let running = crate::fast_forward::active();
    let message = if running {
        Some(copy::running())
    } else {
        read_outcome(&datadir).map(|o| match o {
            Outcome::Done { height, operators } => copy::done(height, &operators),
            Outcome::RolledBack { reason } => copy::rolled_back(&reason),
        })
    };
    Ok(FastForwardStatus { running, message })
}

#[cfg(test)]
mod fast_forward_check_tests {
    use super::{fast_forward_check, FastForwardCheck};
    use btx_core::attested_snapshot::Peek;

    fn confirmed(height: u64) -> Option<Peek> {
        Some(Peek::Confirmed {
            height,
            operators: vec!["Mende".into(), "jpp".into()],
        })
    }

    #[test]
    fn the_button_names_the_block_and_who_confirmed_it() {
        match fast_forward_check(true, confirmed(233_800), 232_000, false) {
            FastForwardCheck::Offer {
                height,
                button,
                confirm,
                note,
            } => {
                assert_eq!(height, 233_800);
                assert_eq!(button, "Fast-forward to block 233,800");
                assert!(
                    confirm.starts_with("Fast-forward to block 233,800, confirmed by Mende and jpp?"),
                    "{confirm}"
                );
                assert_eq!(note, "A confirmed snapshot is far ahead of your node.");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            fast_forward_check(true, confirmed(233_800), 232_800, false),
            FastForwardCheck::None,
            "exactly 1,000 behind is not more"
        );
        assert_eq!(
            fast_forward_check(false, confirmed(233_800), 1, false),
            FastForwardCheck::None,
            "not ours"
        );
        assert_eq!(fast_forward_check(true, None, 1, false), FastForwardCheck::None);
    }

    /// The owner's decision: while no archive peer serves this node old
    /// blocks, the button shows below the 1,000-block line. Never during a
    /// dispute, never without a confirmed snapshot above the node.
    #[test]
    fn no_archive_serving_old_blocks_offers_it_sooner_and_nothing_else() {
        assert_eq!(
            fast_forward_check(true, confirmed(233_800), 232_801, false),
            FastForwardCheck::None,
            "flag false, 999 behind"
        );
        match fast_forward_check(true, confirmed(233_800), 233_500, true) {
            FastForwardCheck::Offer { height, note, .. } => {
                assert_eq!(height, 233_800, "flag true, 300 behind");
                assert!(note.contains("offered sooner than usual"), "{note}");
                assert!(note.contains("It works the same way as always."), "{note}");
            }
            other => panic!("{other:?}"),
        }
        assert!(
            matches!(
                fast_forward_check(true, Some(Peek::Disputed { newest: 233_800 }), 233_500, true),
                FastForwardCheck::Off { .. }
            ),
            "flag true, dispute"
        );
        assert_eq!(
            fast_forward_check(true, confirmed(233_800), 233_800, true),
            FastForwardCheck::None,
            "flag true, the confirmed snapshot is not above the node"
        );
        assert_eq!(
            fast_forward_check(true, None, 233_500, true),
            FastForwardCheck::None,
            "flag true, no confirmed snapshot"
        );
    }

    #[test]
    fn a_dispute_turns_it_off_and_says_why() {
        let off = fast_forward_check(true, Some(Peek::Disputed { newest: 233_800 }), 233_900, false);
        assert_eq!(
            off,
            FastForwardCheck::Off {
                height: 233_800,
                sentence:
                    "Fast-forward is off while the snapshot operators disagree about block 233,800."
                        .into()
            }
        );
        assert_eq!(
            fast_forward_check(false, Some(Peek::Disputed { newest: 233_800 }), 1, true),
            FastForwardCheck::None
        );
    }

    /// The window reads this shape (Task 5's `FastForwardCheck`).
    #[test]
    fn the_check_has_the_shape_the_window_reads() {
        assert_eq!(
            serde_json::to_value(FastForwardCheck::Off {
                height: 233_800,
                sentence: "s".into()
            })
            .unwrap(),
            serde_json::json!({"kind": "off", "height": 233800, "sentence": "s"})
        );
        assert_eq!(
            serde_json::to_value(FastForwardCheck::None).unwrap(),
            serde_json::json!({"kind": "none"})
        );
    }
}
````

In `apps/node/src-tauri/src/lib.rs`, in the `invoke_handler` list, after `tools::tools_restart_node,` (line 54 on `origin/main`):

````rust
            tools::tools_restart_node,
            tools::tools_fast_forward_check,
            tools::tools_fast_forward_run,
            tools::tools_fast_forward_status,
````

- [ ] **Step 2: Build and test**

Run: `cd apps/node/src-tauri && cargo test --locked`
Expected: all pass, `fast_forward_check_tests` (4, derived, not run) included, and the Task 3 `dead_code` warning for `spawn_run` is gone.

- [ ] **Step 3: Format, lint, commit**

````bash
cd apps/node/src-tauri
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious
cd ../../..
````
````bash
git add apps/node/src-tauri/src/tools.rs apps/node/src-tauri/src/lib.rs
git commit -m "node: Tools can check, run and report a Fast-forward, and says when it is off" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````
### Task 5: The Fast-forward section of the Tools overlay

**Files:**
- Modify: `apps/node/index.html` (inside `#tools-overlay`), `apps/node/src/tools-history.ts`, `apps/node/src/tools-history.test.ts`, `apps/node/src/tools.ts`

**Interfaces:**
- Consumes: Task 4's three commands; `RestartArm` (the two-click arm, exists in `tools-history.ts:50`).
- Produces: in `tools-history.ts`, `FAST_FORWARD_ARM_MS = 10_000`, `FAST_FORWARD_POLL_MS = 3_000`, `FF_NOTE`, the types `FastForwardCheck` and `FastForwardStatus`, and the pure `fastForwardView(check, status) -> { section, button, note }`; DOM ids `tools-ff`, `tools-ff-note`, `tools-ff-btn`, `tools-ff-result`.

The section sits between Quick actions and Diagnostics, as the Tools decision lists it. It shows when a run is going, a last run left a message, the check offers the button (with the note Rust sends: far ahead, or offered early because no archive peer serves old blocks), or the operators disagree (then with the dispute sentence and no button). The first click puts the confirm sentence (the prompt with the operators' names) in the note and turns the button into "Click again to fast-forward"; after ten seconds, or when the overlay closes, it goes back. The second click starts the run and the overlay asks for the status every three seconds while it is open.

Anchors on `origin/main` (`b330e3d`, where the Tools overlay landed as `aed8755`): `tools.ts` imports `{ History, ReportCopy, RestartArm, capForDisplay }` at line 6 and `tools-history.test.ts` imports `{ History, ReportCopy, RestartArm, capForDisplay, DISPLAY_LIMIT }` at line 2, so both replacement lines below keep `ReportCopy`; the first version of this plan dropped it. `index.html:966` is the `tools-notices` div and `:968` the Diagnostics heading.

- [ ] **Step 1: Write the failing tests**

In `apps/node/src/tools-history.test.ts`, change the import line (line 2) to:

````ts
import {
  History,
  ReportCopy,
  RestartArm,
  capForDisplay,
  DISPLAY_LIMIT,
  FAST_FORWARD_ARM_MS,
  FF_NOTE,
  fastForwardView,
} from "./tools-history";
````

and append at the end of the file:

````ts
describe("Fast-forward arm", () => {
  it("gives ten seconds for the second click", () => {
    expect(FAST_FORWARD_ARM_MS).toBe(10_000);
  });
  it("runs only on the second click, and a disarm starts over", () => {
    const arm = new RestartArm();
    expect(arm.click()).toBe(false); // shows what will happen
    arm.disarm(); // ten seconds passed
    expect(arm.click()).toBe(false);
    expect(arm.click()).toBe(true); // runs
  });
});

describe("Fast-forward section", () => {
  const offer = {
    kind: "offer" as const,
    height: 233800,
    button: "Fast-forward to block 233,800",
    confirm: "Fast-forward to block 233,800, confirmed by Mende and jpp? ...",
    note: FF_NOTE,
  };
  it("offers the button with the note Rust sent", () => {
    expect(fastForwardView(offer, { running: false, message: null })).toEqual({
      section: true,
      button: "Fast-forward to block 233,800",
      note: FF_NOTE,
    });
    const early =
      "Your node's peers are not sending the older blocks it needs, so Fast-forward is offered sooner than usual. It works the same way as always.";
    expect(fastForwardView({ ...offer, note: early }, null).note).toBe(early);
  });
  it("says it is off, with no button, while the operators disagree", () => {
    const sentence = "Fast-forward is off while the snapshot operators disagree about block 233,800.";
    expect(fastForwardView({ kind: "off", height: 233800, sentence }, null)).toEqual({
      section: true,
      button: null,
      note: sentence,
    });
  });
  it("shows no button during a run, and nothing when there is nothing to say", () => {
    expect(fastForwardView(offer, { running: true, message: "Fast-forward is running." }).button).toBeNull();
    expect(fastForwardView({ kind: "none" }, { running: false, message: null }).section).toBe(false);
    expect(fastForwardView(null, { running: false, message: "Done." }).section).toBe(true);
  });
});
````

- [ ] **Step 2: Run them to see them fail**

Run: `cd apps/node && npx vitest run src/tools-history.test.ts`
Expected: FAIL, `FAST_FORWARD_ARM_MS` and `fastForwardView` are not exported (`expected undefined to be 10000` and `fastForwardView is not a function`).

- [ ] **Step 3: The constants, the types and the view**

In `apps/node/src/tools-history.ts`, directly before `// The note counts against DISPLAY_LIMIT itself,` (line 108 on `origin/main`), add:

````ts
/** Fast-forward's second click must come within this long of the first
 * (the Tools decision, section 3). The arm itself is a RestartArm. */
export const FAST_FORWARD_ARM_MS = 10_000;

/** How often the overlay asks how a Fast-forward is going. */
export const FAST_FORWARD_POLL_MS = 3_000;

/** The section's line when nothing else needs saying. */
export const FF_NOTE = "A confirmed snapshot is far ahead of your node.";

/** What `tools_fast_forward_check` answers. */
export type FastForwardCheck =
  | { kind: "none" }
  | { kind: "offer"; height: number; button: string; confirm: string; note: string }
  | { kind: "off"; height: number; sentence: string };

/** What `tools_fast_forward_status` answers. */
export interface FastForwardStatus {
  running: boolean;
  message: string | null;
}

/** What the Fast-forward section shows. */
export interface FastForwardView {
  /** The section is visible. */
  section: boolean;
  /** The button's label, or null for no button. */
  button: string | null;
  /** The line above the button. */
  note: string;
}

/** Decided in one place, so it has a test: a run hides the button, a
 * dispute shows its sentence and no button (the confirmed-snapshot
 * decision, section 6a), an offer shows the button with the note Rust sent
 * (far ahead, or offered early because no archive peer serves old blocks),
 * and a last run's message keeps the section open on its own. */
export function fastForwardView(
  check: FastForwardCheck | null,
  status: FastForwardStatus | null,
): FastForwardView {
  if (status?.running) return { section: true, button: null, note: FF_NOTE };
  if (check?.kind === "off") return { section: true, button: null, note: check.sentence };
  if (check?.kind === "offer") return { section: true, button: check.button, note: check.note };
  return { section: Boolean(status?.message), button: null, note: FF_NOTE };
}
````

- [ ] **Step 4: The markup**

In `apps/node/index.html`, inside `#tools-overlay`, after `<div class="tools-notices" id="tools-notices" hidden></div>` (line 966 on `origin/main`) and before `<h3 class="tools-h">Diagnostics</h3>` (line 968), add:

````html
          <div id="tools-ff" hidden>
            <h3 class="tools-h">Fast-forward</h3>
            <p class="tools-note" id="tools-ff-note">A confirmed snapshot is far ahead of your node.</p>
            <button id="tools-ff-btn" class="btn-secondary" type="button" hidden></button>
            <p class="setting-result" id="tools-ff-result" hidden></p>
          </div>
````

- [ ] **Step 5: The behaviour**

In `apps/node/src/tools.ts`:

1. Change the import (line 6) to:

````ts
import {
  FAST_FORWARD_ARM_MS,
  FAST_FORWARD_POLL_MS,
  FF_NOTE,
  History,
  ReportCopy,
  RestartArm,
  capForDisplay,
  fastForwardView,
  type FastForwardCheck,
  type FastForwardStatus,
} from "./tools-history";
````

2. In `initTools`, directly after `let restartArmTimer: ReturnType<typeof setTimeout> | undefined;` (line 82 on `origin/main`), add:

````ts
  const ffArm = new RestartArm();
  let ffArmTimer: ReturnType<typeof setTimeout> | undefined;
  let ffPoll: ReturnType<typeof setInterval> | undefined;
  let ffOffer: Extract<FastForwardCheck, { kind: "offer" }> | null = null;
  let ffNote = FF_NOTE;

  /** Back to one click away, as the section says. */
  const resetFfArm = () => {
    clearTimeout(ffArmTimer);
    ffArmTimer = undefined;
    ffArm.disarm();
    $("tools-ff-note").textContent = ffNote;
    if (ffOffer) $("tools-ff-btn").textContent = ffOffer.button;
  };
  const stopFfPoll = () => {
    clearInterval(ffPoll);
    ffPoll = undefined;
  };
  const showFfStatus = (status: FastForwardStatus | null) => {
    const result = $("tools-ff-result");
    if (status?.message) {
      result.textContent = status.message;
      result.hidden = false;
    }
    if (status?.running) $("tools-ff").hidden = false;
  };
  /** While a run is going: ask every few seconds, stop when it ends. */
  const pollFf = () => {
    stopFfPoll();
    ffPoll = setInterval(async () => {
      const status = await invoke<FastForwardStatus>("tools_fast_forward_status").catch(() => null);
      showFfStatus(status);
      if (status && !status.running) stopFfPoll();
    }, FAST_FORWARD_POLL_MS);
  };
  const refreshFastForward = async () => {
    const btn = $<HTMLButtonElement>("tools-ff-btn");
    const status = await invoke<FastForwardStatus>("tools_fast_forward_status").catch(() => null);
    showFfStatus(status);
    if (status?.running) {
      btn.hidden = true;
      pollFf();
      return;
    }
    const check = await invoke<FastForwardCheck>("tools_fast_forward_check").catch(() => null);
    ffOffer = check?.kind === "offer" ? check : null;
    const view = fastForwardView(check, status);
    ffNote = view.note;
    btn.hidden = view.button === null;
    btn.disabled = false;
    resetFfArm();
    $("tools-ff").hidden = !view.section;
  };
````

3. In `closeTools`, after `resetRestartArm();` (line 113), add:

````ts
    resetFfArm();
    stopFfPoll();
````

4. In `open`, after `if (why !== null) say(why);` (line 127), add:

````ts
    void refreshFastForward();
````

5. Directly before `$("tools-fetch").addEventListener("click", async () => {` (line 162), add:

````ts
  // Fast-forward: the first click says what will happen and who confirmed
  // the snapshot, a second within ten seconds runs it (the Tools decision,
  // section 3).
  $("tools-ff-btn").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("tools-ff-btn");
    if (!ffOffer) return;
    if (!ffArm.click()) {
      $("tools-ff-note").textContent = ffOffer.confirm;
      btn.textContent = "Click again to fast-forward";
      clearTimeout(ffArmTimer);
      ffArmTimer = setTimeout(resetFfArm, FAST_FORWARD_ARM_MS);
      return;
    }
    clearTimeout(ffArmTimer);
    ffArmTimer = undefined;
    btn.disabled = true;
    btn.hidden = true;
    $("tools-ff-note").textContent = ffNote;
    const msg = await invoke<string>("tools_fast_forward_run").catch((e) => String(e));
    const result = $("tools-ff-result");
    result.textContent = msg;
    result.hidden = false;
    pollFf();
  });
````

- [ ] **Step 6: Run the web checks**

Run: `cd apps/node && npx tsc --noEmit && npm test && npx vite build`
Expected: no type errors; vitest `133 passed` (128 on `origin/main` `b330e3d`, counted from its `it(` calls, plus these 5; derived, not run; if another plan landed tests first, 5 more than before); the bundle builds.

- [ ] **Step 7: Commit**

````bash
git add apps/node/index.html apps/node/src/tools.ts apps/node/src/tools-history.ts apps/node/src/tools-history.test.ts
git commit -m "node: Fast-forward in Tools, two clicks within ten seconds, off while the operators disagree" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````
### Task 6: Rehearse the roll-back on a real engine, check by hand, changelog

**Files:**
- Modify: `crates/btx-core/tests/confirmed_snapshot_regtest.rs` (one more opt-in test)
- Modify: `apps/node/CHANGELOG.md`

**Interfaces:**
- Consumes: Task 1 (`set_aside`, `restore`), plan 1's regtest helpers (`Node`, `args`, `call`, `mine_to`).
- Produces: `fast_forward_puts_a_real_chain_back` (run by plan 1's engine check, since `check-engine-tag.sh` runs every test in that file).

- [ ] **Step 1: Write the test**

Append to `crates/btx-core/tests/confirmed_snapshot_regtest.rs`:

````rust
/// Fast-forward's roll-back on a real engine's folder: set the chain aside,
/// start on nothing, put it back, and the node is where it was.
#[tokio::test]
#[ignore]
async fn fast_forward_puts_a_real_chain_back() {
    let Some(btxd) = std::env::var_os("EASYNODE_TEST_BTXD").map(PathBuf::from) else {
        eprintln!("EASYNODE_TEST_BTXD unset; nothing to test against");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let mut n = Node::new(&btxd, root.path().join("f"), 29475);
    let validating = args(&["-matmulvalidation=consensus", "-connect=0"]);
    let r = n.start(&validating).await.unwrap();
    mine_to(&r, 50).await;
    let best = call(&r, "getbestblockhash", json!([])).await;
    n.stop(&r).await;

    let record = btx_core::fast_forward::set_aside(&n.net(), 100, true, 1).unwrap();
    assert!(record.moved.contains(&"blocks".to_string()), "{record:?}");
    let r = n.start(&validating).await.unwrap();
    assert_eq!(
        call(&r, "getblockcount", json!([])).await,
        json!(0),
        "a fresh chain"
    );
    n.stop(&r).await;

    btx_core::fast_forward::restore(&n.net(), &record).unwrap();
    let r = n.start(&validating).await.unwrap();
    assert_eq!(call(&r, "getblockcount", json!([])).await, json!(50));
    assert_eq!(call(&r, "getbestblockhash", json!([])).await, best);
    n.stop(&r).await;
}
````

- [ ] **Step 2: Run it against the engine**

Run: `cd crates/btx-core && EASYNODE_TEST_BTXD=/path/to/btxd cargo test --locked --test confirmed_snapshot_regtest -- --ignored --test-threads=1`
Expected: `test result: ok. 3 passed` (it passed against v0.34.9 while this plan was written: 50 blocks set aside, a fresh start at 0, then the same 50 and the same best block after the restore).

- [ ] **Step 3: By hand, on a Mac (validating) and on a node that follows signatures**

With a confirmed snapshot published (or a test pointer: point `CONFIRMED_POINTER_URL` at a local server only in a scratch build, never committed), open Tools on a node more than 1,000 blocks behind and check: the button reads "Fast-forward to block N"; the first click shows "Fast-forward to block N, confirmed by A and B? Your node stops for a few minutes to load it, then carries on from there." and the rest of the confirm sentence, with the operators whose signatures are in the manifest (never names the pointer adds), and the button reads "Click again to fast-forward"; after ten seconds it goes back; the second click shows "Fast-forward is running. Your node restarts a few times on the way."; the node stops, `~/.easybtx/fast-forward-<time>/` appears holding `blocks`, `chainstate`, `chainstate_snapshot`, `shielded_state` and, if the node had one, `snapshot-start.json`; the wallet folder, `attestation-signer.key`, the conf, `peers.dat` and `snapshot-diary.json` stay; the run ends with "Done. Your node now starts from block N, confirmed by A and B." and the dated folder is gone. Then force a failure (disconnect the network after the download, so headers stall) and check the roll-back message and that the node is back at its old height with the old data and its old `snapshot-start.json`. Then, on a node less than 1,000 blocks behind a confirmed snapshot, wait until the status card shows the catch-up help's "no archive peer serves old blocks" sentence (the catch-up plan) and open Tools: the button shows, with the note "Your node's peers are not sending the older blocks it needs, so Fast-forward is offered sooner than usual. It works the same way as always."; restart the node and the note and button go until the help concludes again. Then make the test pointer answer `{"disputed": [N]}` and reopen Tools: the section says "Fast-forward is off while the snapshot operators disagree about block N." and shows no button, on a node at the tip as much as on one far behind. Until Mende and one other listed operator have both signed a snapshot on mainnet (the 3060 is the only listed key every node pins), the button never shows there; that is expected.

- [ ] **Step 4: The changelog**

In `apps/node/CHANGELOG.md`, under `## [Unreleased]`, after plan 1's entry, add:

````markdown
**Fast-forward, for a node that has fallen far behind.** When a confirmed
snapshot is more than 1,000 blocks ahead of your node, or your node's peers
are not sending the older blocks it needs, Tools offers to fast-forward to it
and says who confirmed it. Your node stops for a few
minutes to load it, then carries on from there; your wallets and keys stay
where they are. If anything goes wrong on the way, the old chain data goes
back and Tools says what happened. While the snapshot operators disagree
about a snapshot, Fast-forward is off, and Tools says so.
````

- [ ] **Step 5: Final gates and commit**

````bash
for c in crates/btx-core apps/node/src-tauri; do (cd $c && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D clippy::correctness -D clippy::suspicious && cargo test --locked); done
(cd apps/node && npx tsc --noEmit && npm test && npx vite build)
````
````bash
git add crates/btx-core/tests/confirmed_snapshot_regtest.rs apps/node/CHANGELOG.md
git commit -m "core: rehearse the Fast-forward roll-back on a real engine; changelog" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

## Risks and open points (for the owner)

1. **Nothing to fast-forward to yet.** A snapshot counts only when Mende's 3060 (the only listed key every node pins) and one of Aleksander or jpp have signed it, and the list merges only once the owner confirms Aleksander agreed. Until then the button never shows on mainnet.
2. **A run can take a while.** An empty chain folder gets the header bootstrap first (about a minute of headers to 219,000 from the curated sources), then headers to the snapshot, then on a validating node a mirror launch and a validating launch: up to three MatMul canaries on a Mac. The prompt says "a few minutes", as the decision words it; on a slow link it may be longer, and the run is rolled back after 3 hours.
3. **`shielded_state` and `snapshot-start.json` move too**, beyond the design's list: the first is derived from the chain, the second says where that chain started.
4. **A newer snapshot may be loaded than the one offered**, because the start path re-reads `latest`; the driver accepts any base at or above the offered one, and "Done" names the operators of the one actually loaded.
5. **A dispute that begins mid-run.** The check found a confirmed snapshot, then `latest` turned disputed before the start path read it again. The start path then takes the pinned pair or the compiled snapshot, and `judge` rolls the run back as soon as the node comes up on it, instead of after three hours. The node pays a download and a load it did not need; making a run's load confirmed-only would avoid that and is a change to plan 1's `SignedLoad`, not made here.
6. **Wallets below the snapshot height** open only when the background check gets there; the confirm copy says so.
7. **The prompt joins the decision's sentence and the Tools decision's.** The first click shows the prompt of the confirmed-snapshot decision (section 7) followed by what stays and the wallet sentence, which the Tools decision allows ("in these words or close to them"). If the owner wants the prompt alone, drop the last three sentences of `copy::confirm`.
8. **Offered early, below 1,000 blocks.** The owner's decision: while the catch-up help concludes that no archive peer serves old blocks, the button shows at any lead above the tip, even a few blocks. The note says why and promises nothing more than a normal run. The catch-up plan's status sentence and changelog already name Fast-forward and assume this ships in the same release. The catch-up plan words the usual line as "1,000 or more behind"; this plan keeps the decision's "more than 1,000" (section 10), which the tests pin (exactly 1,000 behind is not offered without the flag).

## Self-review

- Spec coverage: section 10 steps 1 to 4 (Tasks 1, 3), the 1,000-block threshold and the owner's early offer when no archive peer serves old blocks (Task 1's `offer`, Task 4's `fast_forward_check`, both tested), the button and its copy (Tasks 4, 5, as the Tools decision's section 3 fixes them), the prompt naming the operators the app verified (section 7; Tasks 1, 2, 4), Fast-forward off during a dispute with its one sentence (section 6a; Tasks 2, 4, 5), "for any node the app owns" (Task 4's ownership check, Task 3's signed-only load for mirrors), "on any failure, stop, move it back, start as before, and say what failed in one sentence" (Tasks 1, 3; rehearsed on a real engine in Task 6), the wallet sentence (Task 1's copy), the start record kept with its chain (Task 1's `CHAIN_DATA`, Task 3's "Done").
- Placeholders: none. Names checked against plan 1 as amended (`SignedLoad::SignedOnly`, `prepare_confirmed`, `parse_latest`, `Latest`, `node_view`, `snapshot_start::{read, StartSource, START_RECORD_FILE, block_number, join_names}`, `mirror_load_marker_exists`, `end_mirror_load`, `header_bootstrap_pending`, `end_header_bootstrap`, `SIGNED_LOAD_FAILED`, `after_snapshot_load`, `spawn_load_watch`), and every Tools anchor against `origin/main` `b330e3d`.
- Replayed while written: on top of plan 1 applied to a clean `72304d9` worktree, every gate passed (fmt, clippy correctness and suspicious, btx-core and app tests, `tsc`, vitest, vite build), and the three regtest tests passed against v0.34.9. This amendment was not replayed (no cargo or npm ran); the counts it changed are marked "derived, not run".
