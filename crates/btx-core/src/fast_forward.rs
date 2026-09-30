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
        assert_eq!(
            offer(true, Some(232_000), 231_001, false),
            None,
            "999 behind"
        );
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
        assert_eq!(
            offer(true, Some(232_000), 232_000, true),
            None,
            "not above the tip"
        );
        assert_eq!(
            offer(true, Some(232_000), 232_500, true),
            None,
            "below the tip"
        );
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
        let three = vec![
            "Mende".to_string(),
            "Aleksander".to_string(),
            "jpp".to_string(),
        ];
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
