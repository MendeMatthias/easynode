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
//!
//! The old chain can always be put back, whenever the app stops. The record
//! (`<datadir>/.fast-forward.json`) is written before anything moves and
//! says which step the run is at ([`Phase`]), so every step after it can be
//! cut off by a crash and carried on from the record at the next start:
//!
//! * [`set_aside`]: the record ([`Phase::SettingAside`], listing what is to
//!   move), the dated folder, the moves, a copy of the record marked
//!   [`Phase::Undoing`] for later, then [`Phase::Running`]. Only then may
//!   the node start on new chain data.
//! * [`restore`]: from `Running`, the undo is marked first (`Undoing`, by
//!   renaming that copy into place, which needs no free space on a disk a
//!   failed load may have filled), so the next start never watches the run
//!   again; then the chain data the attempt made is removed, then
//!   [`Phase::Restoring`], then the old data goes back, then the dated
//!   folder, and the record last. From `SettingAside` or `Restoring` nothing
//!   is removed: what is still in the dated folder goes back.
//! * [`finish`]: [`Phase::Done`] first, so nothing puts old data back once
//!   any of it may be gone; then the dated folder takes a name [`restore`]
//!   never accepts, the record goes, and the folder last.
//! * [`sweep`]: at the next start, and in Remove node data, a dated folder
//!   that no recorded run needs is removed.
//! * [`resume`]: a run the app was closed on is watched again from the start
//!   of the app that resumes it.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// A confirmed snapshot must be more than this far ahead of the node's tip
/// before the button shows (the owner's choice 4).
pub const MIN_LEAD: u64 = 1_000;

/// A run that has not finished in this long is rolled back. A load takes
/// minutes; the header sync before it, on a slow link, can take an hour.
pub const MAX_RUN_SECS: u64 = 3 * 60 * 60;

/// However often a run is resumed with a fresh watch window, it is rolled
/// back this long after it began ([`Record::started_at`]), so a crash loop,
/// or an app never kept open for [`MAX_RUN_SECS`] in a row, does not keep it
/// (and a node with no usable chain) going for ever.
pub const MAX_TOTAL_SECS: u64 = 24 * 60 * 60;

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

/// The run's record marked [`Phase::Undoing`], written by [`set_aside`]
/// while there is room, so [`restore`] can mark the undo with a rename.
fn undo_path(datadir: &Path) -> PathBuf {
    datadir.join(".fast-forward-undo.json")
}

/// This module's files, each written through `fsx::atomic_write`.
const OWN_FILES: [&str; 3] = [
    ".fast-forward.json",
    ".fast-forward-result.json",
    ".fast-forward-undo.json",
];

/// Which step a run is at. Written before the step it names begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The chain data is moving into the dated folder; `moved` is what is to
    /// move. The node has not run since.
    SettingAside,
    /// Everything in `moved` is in the dated folder and the node runs on new
    /// chain data: the run is watched ([`judge`]).
    Running,
    /// A roll-back has begun: the chain data the attempt made is being
    /// removed, and everything in `moved` is still in the dated folder. The
    /// run is never watched again ([`judge`] rolls it back).
    Undoing,
    /// The chain data the attempt made is gone and the old data is going
    /// back: some of `moved` may be back in place already.
    Restoring,
    /// The run is done and its dated folder is being removed: nothing may
    /// put it back ([`finish`]).
    Done,
}

/// A run in progress, on disk, so a quit in the middle can be finished or
/// undone at the next start. Only this module writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The confirmed snapshot's base.
    pub height: u64,
    /// The dated folder, relative to the datadir: always [`aside_name`]'s.
    pub aside: String,
    /// What moves (while [`Phase::SettingAside`]) or moved into it, so
    /// exactly that comes back. Only [`CHAIN_DATA`] names.
    pub moved: Vec<String>,
    pub phase: Phase,
    /// The app's "a snapshot was loaded" setting before the run.
    pub snapshot_loaded_before: bool,
    /// The app's "first load still to come" setting before the run. With
    /// `blocks` set aside the start path sets it, so a roll-back puts this
    /// value back.
    #[serde(default)]
    pub first_load_pending_before: bool,
    /// When the run began. [`MAX_TOTAL_SECS`] counts from here, however
    /// often the run is resumed.
    pub started_at: u64,
    /// When the current watch began: the run's start, or the start of the
    /// app that resumed it ([`resume`]). [`MAX_RUN_SECS`] counts from here.
    pub watch_started_at: u64,
}

/// The app's settings before a run, kept in its record for a roll-back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Before {
    pub snapshot_loaded: bool,
    pub first_load_pending: bool,
}

/// Why [`set_aside`] or [`restore`] stopped.
#[derive(Debug)]
pub enum MoveError {
    /// This call changed nothing in the datadir.
    Untouched(io::Error),
    /// Not all of the old chain data is back in place; what is not is in
    /// `folder`. The record stays, so [`restore`] carries on from where this
    /// stopped (at the next start at the latest).
    Stranded { folder: PathBuf, error: io::Error },
}

impl std::fmt::Display for MoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MoveError::Untouched(e) => write!(f, "{e}"),
            MoveError::Stranded { folder, error } => write!(
                f,
                "{error}; the old chain data that is not back in place is in {}",
                folder.display()
            ),
        }
    }
}

impl std::error::Error for MoveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MoveError::Untouched(e) | MoveError::Stranded { error: e, .. } => Some(e),
        }
    }
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
    format!("{ASIDE_PREFIX}{now_unix}")
}

const ASIDE_PREFIX: &str = "fast-forward-";

/// `fast-forward-<digits>` and nothing else: one plain name in the datadir.
fn is_aside_name(name: &str) -> bool {
    name.strip_prefix(ASIDE_PREFIX)
        .is_some_and(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
}

/// A record comes from a file, and its names become paths that are moved
/// and removed: the dated folder must be one [`aside_name`] makes, and every
/// entry chain data.
fn check(record: &Record) -> io::Result<()> {
    let bad = |what: String| io::Error::new(io::ErrorKind::InvalidData, what);
    if !is_aside_name(&record.aside) {
        return Err(bad(format!(
            "the Fast-forward record names {:?} as its folder",
            record.aside
        )));
    }
    if let Some(name) = record
        .moved
        .iter()
        .find(|m| !CHAIN_DATA.contains(&m.as_str()))
    {
        return Err(bad(format!(
            "the Fast-forward record lists {name:?}, which is not chain data"
        )));
    }
    Ok(())
}

/// Whether anything is at `path`, a link included (not followed). Only "not
/// found" is no; any other error is passed on.
fn present(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Remove what is at `path`: a folder with everything in it, a file, or a
/// link (never what it points at). Nothing there is fine.
fn remove_any(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Between two steps that change the disk. The app always goes on; a test
/// stops a call here, as a crash would: nothing after it runs, not even a
/// clean-up.
type Pause<'a> = &'a mut dyn FnMut() -> io::Result<()>;

/// Move every [`CHAIN_DATA`] entry that exists into a new dated folder, and
/// keep the record of it on disk. Refused while a run is recorded. On a
/// failure, what moved goes back ([`MoveError::Untouched`]); if that fails
/// too, the record stays and the error says where the old data is
/// ([`MoveError::Stranded`]). Call only with the node stopped.
pub fn set_aside(
    datadir: &Path,
    height: u64,
    before: Before,
    now_unix: u64,
) -> Result<Record, MoveError> {
    set_aside_with(
        datadir,
        CHAIN_DATA,
        height,
        before,
        now_unix,
        &mut || Ok(()),
    )
}

fn set_aside_with(
    datadir: &Path,
    names: &[&str],
    height: u64,
    before: Before,
    now_unix: u64,
    pause: Pause,
) -> Result<Record, MoveError> {
    use MoveError::Untouched;
    if read_record(datadir).map_err(Untouched)?.is_some() {
        return Err(Untouched(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "a Fast-forward run is already recorded",
        )));
    }
    let mut moved = Vec::new();
    for name in names {
        if present(&datadir.join(name)).map_err(Untouched)? {
            moved.push(name.to_string());
        }
    }
    let aside = aside_name(now_unix);
    let dir = datadir.join(&aside);
    if present(&dir).map_err(Untouched)? {
        return Err(Untouched(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} is in the way", dir.display()),
        )));
    }
    let mut record = Record {
        height,
        aside,
        moved,
        phase: Phase::SettingAside,
        snapshot_loaded_before: before.snapshot_loaded,
        first_load_pending_before: before.first_load_pending,
        started_at: now_unix,
        watch_started_at: now_unix,
    };
    pause().map_err(Untouched)?;
    write_record(datadir, &record).map_err(Untouched)?;
    pause().map_err(Untouched)?;
    if let Err(e) = std::fs::create_dir(&dir) {
        return Err(undo_set_aside(datadir, &record, e));
    }
    for name in &record.moved {
        pause().map_err(Untouched)?;
        if let Err(e) = std::fs::rename(datadir.join(name), dir.join(name)) {
            return Err(undo_set_aside(datadir, &record, e));
        }
    }
    pause().map_err(Untouched)?;
    let undoing = Record {
        phase: Phase::Undoing,
        ..record.clone()
    };
    if let Err(e) = write_to(&undo_path(datadir), &undoing) {
        return Err(undo_set_aside(datadir, &record, e));
    }
    pause().map_err(Untouched)?;
    record.phase = Phase::Running;
    if let Err(e) = write_record(datadir, &record) {
        record.phase = Phase::SettingAside;
        return Err(undo_set_aside(datadir, &record, e));
    }
    Ok(record)
}

/// Setting aside failed with `error`: put back what moved, and drop the
/// record.
fn undo_set_aside(datadir: &Path, record: &Record, error: io::Error) -> MoveError {
    match put_back(datadir, record, &mut || Ok(())) {
        Ok(()) => MoveError::Untouched(error),
        Err(MoveError::Stranded { folder, error: e }) => MoveError::Stranded {
            folder,
            error: io::Error::new(
                error.kind(),
                format!("{error}, and putting back what had moved failed: {e}"),
            ),
        },
        Err(untouched) => untouched,
    }
}

/// Undo the run on disk, from whichever step it is at, and return its
/// record (its settings from before the run are for the caller to put
/// back); `Ok(None)` when no run is recorded. From [`Phase::Running`], the
/// dated folder is checked first, so nothing is removed unless its original
/// is there to replace it; then the undo is marked ([`Phase::Undoing`]), so
/// the next start rolls the run back rather than watching it again; then the
/// chain data the attempt made goes, then the old data comes back. Called
/// again after any failure or crash, it carries on. Call only with the node
/// stopped, and start the node only after it returns `Ok`.
pub fn restore(datadir: &Path) -> Result<Option<Record>, MoveError> {
    restore_with(datadir, &mut || Ok(()))
}

fn restore_with(datadir: &Path, pause: Pause) -> Result<Option<Record>, MoveError> {
    let Some(mut record) = read_record(datadir).map_err(MoveError::Untouched)? else {
        return Ok(None);
    };
    if record.phase == Phase::Done {
        return Err(MoveError::Untouched(io::Error::other(format!(
            "the run to block {} is done; its old chain data is being removed",
            copy::height(record.height)
        ))));
    }
    let folder = datadir.join(&record.aside);
    let stranded = |error| MoveError::Stranded {
        folder: folder.clone(),
        error,
    };
    if matches!(record.phase, Phase::Running | Phase::Undoing) {
        for name in &record.moved {
            if !present(&folder.join(name)).map_err(stranded)? {
                return Err(stranded(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{name} is missing from {}", folder.display()),
                )));
            }
        }
        if record.phase == Phase::Running {
            mark_undoing(datadir, &mut record).map_err(stranded)?;
        }
        for name in CHAIN_DATA {
            let fresh = datadir.join(name);
            if present(&fresh).map_err(stranded)? {
                pause().map_err(stranded)?;
                remove_any(&fresh).map_err(stranded)?;
            }
        }
        pause().map_err(stranded)?;
        record.phase = Phase::Restoring;
        write_record(datadir, &record).map_err(stranded)?;
    }
    put_back(datadir, &record, pause)?;
    Ok(Some(record))
}

/// Mark the undo of a running run as begun, before anything is removed.
/// The copy [`set_aside`] wrote is renamed into place, which needs no free
/// space; without it, or with one that is not this run's, the mark is
/// written.
fn mark_undoing(datadir: &Path, record: &mut Record) -> io::Result<()> {
    record.phase = Phase::Undoing;
    let copy = std::fs::read(undo_path(datadir))
        .ok()
        .and_then(|b| serde_json::from_slice::<Record>(&b).ok());
    // The watch window is the one field a resume changes after the copy.
    let this_runs = copy.is_some_and(|c| {
        c == Record {
            watch_started_at: c.watch_started_at,
            ..record.clone()
        }
    });
    if this_runs {
        std::fs::rename(undo_path(datadir), record_path(datadir))
    } else {
        write_record(datadir, record)
    }
}

/// Move what of `record.moved` is still in the dated folder back to its
/// place, then remove the folder and, last, the record. Nothing is removed
/// and nothing overwritten on the way: an entry both in the folder and in
/// its place, or in neither, cannot go back. Everything that can goes back;
/// then what could not is named, and the folder and the record stay. Chain
/// data left in the folder stops the folder's removal; anything else there
/// (a `.DS_Store`) goes with it.
fn put_back(datadir: &Path, record: &Record, pause: Pause) -> Result<(), MoveError> {
    let folder = datadir.join(&record.aside);
    let stranded = |error| MoveError::Stranded {
        folder: folder.clone(),
        error,
    };
    let mut failed = Vec::new();
    for name in &record.moved {
        let old = folder.join(name);
        let place = datadir.join(name);
        let (waiting, placed) = match (present(&old), present(&place)) {
            (Ok(w), Ok(p)) => (w, p),
            (Err(e), _) | (_, Err(e)) => {
                failed.push(io::Error::new(e.kind(), format!("{name}: {e}")));
                continue;
            }
        };
        match (waiting, placed) {
            (true, false) => {
                pause().map_err(stranded)?;
                if let Err(e) = std::fs::rename(&old, &place) {
                    failed.push(io::Error::new(
                        e.kind(),
                        format!("{name} could not be moved back: {e}"),
                    ));
                }
            }
            (false, true) => {} // back already
            (true, true) => failed.push(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{name} is in {} and in its place too", folder.display()),
            )),
            (false, false) => failed.push(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{name} is neither in its place nor in {}", folder.display()),
            )),
        }
    }
    if let Some(first) = failed.first() {
        let said: Vec<String> = failed.iter().map(|e| e.to_string()).collect();
        return Err(stranded(io::Error::new(first.kind(), said.join("; "))));
    }
    if present(&folder).map_err(stranded)? {
        let mut left = Vec::new();
        for name in CHAIN_DATA {
            if present(&folder.join(name)).map_err(stranded)? {
                left.push(*name);
            }
        }
        if !left.is_empty() {
            return Err(stranded(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} still in {}", left.join(", "), folder.display()),
            )));
        }
        pause().map_err(stranded)?;
        std::fs::remove_dir_all(&folder).map_err(stranded)?;
    }
    pause().map_err(stranded)?;
    clear_record(datadir).map_err(stranded)
}

/// A run judged done: the set-aside chain data is no longer needed. Phase
/// done is written first; then the dated folder is renamed to a name
/// [`restore`] never accepts, the record goes, and the folder is removed
/// last. An `Err` leaves the record, and a later call (or the next start)
/// carries on; if only the removal fails, the run is finished and [`sweep`]
/// removes the folder at the next start. Refused while a run is being set
/// aside or undone. `Ok` when no run is recorded.
pub fn finish(datadir: &Path) -> io::Result<()> {
    finish_with(datadir, &mut || Ok(()))
}

fn finish_with(datadir: &Path, pause: Pause) -> io::Result<()> {
    let Some(mut record) = read_record(datadir)? else {
        return Ok(());
    };
    match record.phase {
        Phase::Done => {}
        Phase::Running => {
            pause()?;
            record.phase = Phase::Done;
            write_record(datadir, &record)?;
        }
        Phase::SettingAside | Phase::Restoring | Phase::Undoing => {
            return Err(io::Error::other(
                "the Fast-forward run is being undone, not finished",
            ))
        }
    }
    let folder = datadir.join(&record.aside);
    let doomed = datadir.join(discard_name(&record.aside));
    if present(&folder)? {
        pause()?;
        // A leftover under the same name: nothing ever reads one.
        remove_any(&doomed)?;
        std::fs::rename(&folder, &doomed)?;
    }
    pause()?;
    clear_record(datadir)?;
    pause()?;
    if let Err(e) = remove_any(&doomed) {
        eprintln!(
            "[fast-forward] could not remove {} (the next start sweeps it): {e}",
            doomed.display()
        );
    }
    Ok(())
}

const DISCARD_SUFFIX: &str = ".discard";

/// The name a done run's folder takes before it is removed.
fn discard_name(aside: &str) -> String {
    format!("{aside}{DISCARD_SUFFIX}")
}

/// A temporary file `fsx::atomic_write` left for one of [`OWN_FILES`] when
/// it was cut off: `.<file>.<pid>.<n>.tmp`, exactly.
fn is_own_leftover(name: &str) -> bool {
    OWN_FILES.iter().any(|file| {
        name.strip_prefix('.')
            .and_then(|n| n.strip_prefix(file))
            .and_then(|n| n.strip_prefix('.'))
            .and_then(|n| n.strip_suffix(".tmp"))
            .and_then(|n| n.split_once('.'))
            .is_some_and(|(pid, n)| {
                [pid, n]
                    .iter()
                    .all(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
            })
    })
}

/// Tidy what no run needs. A temporary file one of this module's writes
/// left behind goes whatever the record says. Then, only when no run is
/// recorded (and not when a record cannot be read): every dated folder
/// (`fast-forward-<digits>` and its `.discard` name) and the undo copy.
/// Never a link, and nothing else. Call with the node stopped: at start,
/// before [`resume`], and in Remove node data. What it removed.
pub fn sweep(datadir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(datadir) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    let mut folders = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        // `file_type` does not follow a link.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() && is_own_leftover(name) {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => removed.push(entry.path()),
                Err(e) => eprintln!(
                    "[fast-forward] could not remove {} (non-fatal): {e}",
                    entry.path().display()
                ),
            }
        } else if kind.is_dir()
            && (is_aside_name(name) || name.strip_suffix(DISCARD_SUFFIX).is_some_and(is_aside_name))
        {
            folders.push(entry.path());
        }
    }
    match read_record(datadir) {
        Ok(None) => {}
        Ok(Some(_)) => return removed,
        Err(e) => {
            eprintln!("[fast-forward] not sweeping: the run's record cannot be read ({e})");
            return removed;
        }
    }
    let copy = undo_path(datadir);
    if std::fs::symlink_metadata(&copy).is_ok_and(|m| m.is_file()) {
        match std::fs::remove_file(&copy) {
            Ok(()) => removed.push(copy),
            Err(e) => eprintln!(
                "[fast-forward] could not remove {} (non-fatal): {e}",
                copy.display()
            ),
        }
    }
    for folder in folders {
        match std::fs::remove_dir_all(&folder) {
            Ok(()) => removed.push(folder),
            Err(e) => eprintln!(
                "[fast-forward] could not remove {} (non-fatal): {e}",
                folder.display()
            ),
        }
    }
    removed
}

/// Give up a run whose old chain data cannot come back (an original is
/// missing from its dated folder, or cannot be checked, so [`restore`]
/// cannot begin), for Remove node data, which removes the new chain with
/// the rest: the record and the undo copy go, and [`sweep`] then removes the
/// dated folder. Only a run at [`Phase::Running`], the one phase such a run
/// is left at, and only when its old chain is not all there; any other is
/// refused and nothing changes.
pub fn abandon(datadir: &Path) -> io::Result<()> {
    match read_record(datadir)? {
        None => Ok(()),
        Some(r) if r.phase == Phase::Running && !all_waiting(datadir, &r) => clear_record(datadir),
        Some(r) => Err(io::Error::other(format!(
            "the run to block {} is not one to give up now",
            copy::height(r.height)
        ))),
    }
}

/// Every entry of `record.moved` is in the dated folder, as [`restore`]
/// needs before it removes anything from a running run.
fn all_waiting(datadir: &Path, record: &Record) -> bool {
    let folder = datadir.join(&record.aside);
    record
        .moved
        .iter()
        .all(|name| matches!(present(&folder.join(name)), Ok(true)))
}

/// The run a previous start left, to be watched again: one at
/// [`Phase::Running`] gets a fresh watch window from `now_unix` (time the
/// app was closed does not count against it), kept on disk. When that
/// window cannot be written (a disk a failed load filled), the run keeps
/// the window it had and is watched with it, so the start is not held up
/// and a roll-back, whose mark needs no space, still comes. A record at any
/// other phase comes back as it is; [`judge`] says what to do with it. An
/// error only when the record cannot be read.
pub fn resume(datadir: &Path, now_unix: u64) -> io::Result<Option<Record>> {
    let Some(record) = read_record(datadir)? else {
        return Ok(None);
    };
    if record.phase != Phase::Running {
        return Ok(Some(record));
    }
    let fresh = Record {
        watch_started_at: now_unix,
        ..record.clone()
    };
    match write_record(datadir, &fresh) {
        Ok(()) => Ok(Some(fresh)),
        Err(e) => {
            eprintln!(
                "[fast-forward] the run's fresh watch window could not be written, so it keeps \
                 the one it had: {e}"
            );
            Ok(Some(record))
        }
    }
}

/// Only this module writes the record, so its phases follow the steps.
fn write_record(datadir: &Path, record: &Record) -> std::io::Result<()> {
    write_to(&record_path(datadir), record)
}

fn write_to(path: &Path, record: &Record) -> io::Result<()> {
    let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
    crate::fsx::atomic_write(path, &bytes)
}

/// The run on disk. `Ok(None)` only when there is no record; one that cannot
/// be read, or names anything but a dated folder and chain data, is an
/// error, so a run is never forgotten or misread.
pub fn read_record(datadir: &Path) -> io::Result<Option<Record>> {
    let bytes = match std::fs::read(record_path(datadir)) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let record = serde_json::from_slice(&bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    check(&record)?;
    Ok(Some(record))
}

/// The undo copy goes first, so no copy outlives its record. Every phase
/// that clears the record is past needing the copy.
fn clear_record(datadir: &Path) -> io::Result<()> {
    for path in [undo_path(datadir), record_path(datadir)] {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

pub fn write_outcome(datadir: &Path, outcome: &Outcome) {
    if let Ok(bytes) = serde_json::to_vec(outcome) {
        let _ = crate::fsx::atomic_write(&result_path(datadir), &bytes);
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
    /// The loader's own after-load check passed: the app's "a snapshot was
    /// loaded" setting, which the run resets before anything moves and only
    /// the loader sets again, once its check after the load has passed. The
    /// engine shows the snapshot chainstate before that check ends.
    pub loaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Continue,
    Done,
    RollBack(String),
}

/// Pure: is the run done, still going, or to be rolled back? A record at
/// [`Phase::Done`] is done; one left between two steps of a move
/// ([`Phase::SettingAside`], [`Phase::Restoring`]) is rolled back, whatever
/// the node shows. The limit counts from `watch_started_at`, and the total
/// one ([`MAX_TOTAL_SECS`]) from `started_at`: a clock that jumps forward
/// only rolls a run back early, and one set back only delays both limits
/// until it catches up. Done means a snapshot at or above the run's is
/// loaded (the start path may have found a newer confirmed one), the
/// loader's own check after the load has passed ([`Look::loaded`]), and the
/// node runs its ordinary launch again: no mirror launch and no header
/// bootstrap pending. A snapshot below the run's on that ordinary launch
/// means the start path took a fallback (the operators began to disagree
/// after the check, or the pair was refused): rolled back at once.
///
/// Follow-up, not done here: done compares heights only. The base block's
/// hash (the confirmed snapshot's, or the start record's for a newer one)
/// is for the driver to check as well before it calls [`finish`].
pub fn judge(record: &Record, look: &Look, now_unix: u64) -> Verdict {
    match record.phase {
        Phase::Done => return Verdict::Done,
        Phase::Running => {}
        Phase::SettingAside => {
            return Verdict::RollBack(
                "the app stopped while it was setting the chain data aside".into(),
            )
        }
        Phase::Undoing => {
            return Verdict::RollBack("the app stopped while it was undoing Fast-forward".into())
        }
        Phase::Restoring => {
            return Verdict::RollBack(
                "the app stopped while it was putting the old chain data back".into(),
            )
        }
    }
    if let Some(why) = &look.load_failed {
        return Verdict::RollBack(why.clone());
    }
    let ordinary = look.running && !look.mirror_load_pending && !look.header_bootstrap_pending;
    match look.snapshot_base_height {
        // Only once the loader's check after the load has passed: until
        // then a refusal can still come, and the old chain must be there.
        Some(h) if ordinary && h >= record.height && look.loaded => return Verdict::Done,
        Some(h) if ordinary && h >= record.height => {}
        Some(h) if ordinary => {
            return Verdict::RollBack(format!(
                "the node started from block {} instead of the confirmed snapshot at block {}",
                copy::height(h),
                copy::height(record.height)
            ))
        }
        _ => {}
    }
    if now_unix.saturating_sub(record.watch_started_at) >= MAX_RUN_SECS {
        return Verdict::RollBack(format!(
            "it did not finish within {} hours",
            MAX_RUN_SECS / 3600
        ));
    }
    if now_unix.saturating_sub(record.started_at) >= MAX_TOTAL_SECS {
        return Verdict::RollBack(format!(
            "it had not finished {} hours after it started",
            MAX_TOTAL_SECS / 3600
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

    /// The chain folders every datadir in these tests holds, each with a
    /// file `old` naming it.
    const CHAIN_DIRS: [&str; 5] = [
        "blocks",
        "chainstate",
        "chainstate_snapshot",
        "indexes",
        "shielded_state",
    ];

    const KEPT: [&str; 5] = [
        "wallets/main/wallet.dat",
        "attestation-signer.key",
        "peers.dat",
        "banlist.json",
        "snapshot-diary.json",
    ];

    fn datadir_with_chain() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fill_datadir(tmp.path());
        tmp
    }

    fn fill_datadir(d: &Path) {
        for name in CHAIN_DIRS {
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
    }

    /// New chain data, as an attempt that went wrong leaves it.
    fn attempt_made_chain(d: &Path) {
        for name in ["blocks", "chainstate", "indexes"] {
            std::fs::create_dir_all(d.join(name)).unwrap();
            std::fs::write(d.join(name).join("new"), b"new").unwrap();
        }
        std::fs::write(d.join("snapshot-start.json"), b"new start").unwrap();
    }

    /// The datadir exactly as [`fill_datadir`] made it, and no run left.
    fn assert_as_before(d: &Path, when: &str) {
        for name in CHAIN_DIRS {
            let entries: Vec<String> = std::fs::read_dir(d.join(name))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            assert_eq!(entries, ["old"], "{name}, {when}");
            assert_eq!(
                std::fs::read_to_string(d.join(name).join("old")).unwrap(),
                name,
                "{when}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(d.join("snapshot-start.json")).unwrap(),
            "old start",
            "the start record comes back with the chain it describes, {when}"
        );
        for kept in KEPT {
            assert!(d.join(kept).exists(), "{kept}, {when}");
        }
        let left: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("fast-forward-") || n.starts_with(".fast-forward"))
            .collect();
        assert!(left.is_empty(), "{left:?} left, {when}");
    }

    /// A pause that stops the call at its `k`th stop, as a crash would.
    fn crash_at(k: usize) -> impl FnMut() -> io::Result<()> {
        let mut n = 0;
        move || {
            n += 1;
            if n == k {
                Err(io::Error::other("crash"))
            } else {
                Ok(())
            }
        }
    }

    /// What the app does with a datadir when it starts: the sweep, then the
    /// run it finds, watched again.
    fn next_start(d: &Path, now_unix: u64) -> Option<Record> {
        sweep(d);
        resume(d, now_unix).unwrap()
    }

    /// The node looks fine on the snapshot: only the phase can keep a run
    /// from being judged done, or watched again.
    fn on_the_snapshot() -> Look {
        Look {
            snapshot_base_height: Some(232_000),
            running: true,
            loaded: true,
            ..Look::default()
        }
    }

    fn running_record(started_at: u64) -> Record {
        Record {
            height: 232_000,
            aside: aside_name(started_at),
            moved: vec![],
            phase: Phase::Running,
            snapshot_loaded_before: false,
            first_load_pending_before: false,
            started_at,
            watch_started_at: started_at,
        }
    }

    /// The new chain an attempt that worked leaves, and nothing of the run
    /// beside it.
    fn assert_new_chain_kept(d: &Path, when: &str) {
        for name in ["blocks", "chainstate", "indexes"] {
            assert!(d.join(name).join("new").exists(), "{name}, {when}");
            assert!(!d.join(name).join("old").exists(), "{name}, {when}");
        }
        assert_eq!(
            std::fs::read_to_string(d.join("snapshot-start.json")).unwrap(),
            "new start",
            "{when}"
        );
        for kept in KEPT {
            assert!(d.join(kept).exists(), "{kept}, {when}");
        }
        let left: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("fast-forward-") || n.starts_with(".fast-forward"))
            .collect();
        assert!(left.is_empty(), "{left:?} left, {when}");
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
        let before = Before {
            snapshot_loaded: true,
            first_load_pending: true,
        };
        let r = set_aside(d, 232_000, before, 1_790_000_000).unwrap();
        assert_eq!(r.aside, "fast-forward-1790000000");
        assert_eq!(
            r.moved,
            vec![
                "blocks",
                "chainstate",
                "chainstate_snapshot",
                "indexes",
                "shielded_state",
                "snapshot-start.json"
            ]
        );
        assert!(r.snapshot_loaded_before && r.first_load_pending_before);
        assert_eq!(r.phase, Phase::Running);
        assert_eq!(
            read_record(d).unwrap(),
            Some(r.clone()),
            "set_aside keeps its own record"
        );
        for name in CHAIN_DATA {
            assert!(!present(&d.join(name)).unwrap(), "{name} still in place");
        }
        for kept in KEPT {
            assert!(d.join(kept).exists(), "{kept} moved");
        }
        // The failed attempt made new chain data and a new start record;
        // restoring replaces them.
        attempt_made_chain(d);
        let undone = restore(d).unwrap().expect("a run to undo");
        assert_eq!(undone.height, 232_000);
        assert!(undone.snapshot_loaded_before && undone.first_load_pending_before);
        assert_as_before(d, "restored");
    }

    /// Chain data the node had none of before the run, made by the attempt,
    /// goes with it.
    #[test]
    fn what_the_attempt_made_goes_with_it() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        std::fs::remove_dir_all(d.join("indexes")).unwrap();
        let r = set_aside(d, 232_000, Before::default(), 3).unwrap();
        assert!(!r.moved.contains(&"indexes".to_string()));
        std::fs::create_dir_all(d.join("indexes")).unwrap();
        std::fs::write(d.join("indexes/new"), b"new").unwrap();
        restore(d).unwrap();
        assert!(
            !d.join("indexes").exists(),
            "made by the attempt, gone with it"
        );
        assert!(d.join("blocks/old").exists());
    }

    /// A chain folder that is a link (say to another disk) moves as the
    /// link, even when what it points at is not there right now: a check
    /// that follows the link would leave it in place.
    #[cfg(unix)]
    #[test]
    fn a_linked_chain_folder_moves_as_the_link() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        std::fs::remove_dir_all(d.join("blocks")).unwrap();
        std::os::unix::fs::symlink(d.join("unmounted-disk/blocks"), d.join("blocks")).unwrap();
        let r = set_aside(d, 232_000, Before::default(), 4).unwrap();
        assert!(r.moved.contains(&"blocks".to_string()));
        assert!(std::fs::symlink_metadata(d.join("blocks")).is_err());
        // The attempt made a real `blocks`; the link comes back in its place.
        std::fs::create_dir_all(d.join("blocks")).unwrap();
        restore(d).unwrap();
        let back = std::fs::symlink_metadata(d.join("blocks")).unwrap();
        assert!(back.file_type().is_symlink());
    }

    #[test]
    fn restore_removes_nothing_when_an_original_is_missing() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let r = set_aside(d, 232_000, Before::default(), 7).unwrap();
        attempt_made_chain(d);
        std::fs::remove_dir_all(d.join(&r.aside).join("chainstate")).unwrap();
        match restore(d) {
            Err(MoveError::Stranded { folder, .. }) => assert_eq!(folder, d.join(&r.aside)),
            other => panic!("{other:?}"),
        }
        assert!(d.join("blocks/new").exists(), "nothing removed");
        assert_eq!(read_record(d).unwrap(), Some(r), "the run stays recorded");
    }

    #[test]
    fn a_failed_move_puts_back_what_moved() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        // A second entry whose move fails (its parent does not exist in the
        // dated folder), after `blocks` has already moved.
        std::fs::create_dir_all(d.join("nested/data")).unwrap();
        let got = set_aside_with(
            d,
            &["blocks", "nested/data"],
            232_000,
            Before::default(),
            9,
            &mut || Ok(()),
        );
        assert!(matches!(got, Err(MoveError::Untouched(_))), "{got:?}");
        assert!(d.join("blocks/old").exists(), "blocks came back");
        assert!(d.join("nested/data").exists());
        assert!(!d.join(aside_name(9)).exists(), "the dated folder is gone");
        assert_eq!(read_record(d).unwrap(), None, "and so is its record");
        // And a dated folder in the way stops it before anything moves.
        std::fs::write(d.join(aside_name(9)), b"in the way").unwrap();
        let got = set_aside(d, 232_000, Before::default(), 9);
        assert!(matches!(got, Err(MoveError::Untouched(_))), "{got:?}");
        std::fs::remove_file(d.join(aside_name(9))).unwrap();
        assert_as_before(d, "nothing moved");
    }

    /// When a move fails and putting back what had moved fails too, the
    /// error says where the old chain data is, never that all is as it was;
    /// the record stays, so a later restore finishes the job.
    #[test]
    fn a_failed_put_back_says_where_the_old_chain_is() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let folder = d.join(aside_name(9));
        // Stops 1 and 2 are the record and the dated folder, 3 is before
        // `blocks` moves and 4 before `chainstate` does. At 4, something
        // takes `chainstate`'s place in the folder, so its move fails, and
        // something takes `blocks`' place in the datadir, so putting
        // `blocks` back fails too.
        let mut stop = 0;
        let mut pause = || {
            stop += 1;
            if stop == 4 {
                std::fs::create_dir_all(folder.join("chainstate/x")).unwrap();
                std::fs::create_dir_all(d.join("blocks/x")).unwrap();
            }
            Ok(())
        };
        let got = set_aside_with(d, CHAIN_DATA, 232_000, Before::default(), 9, &mut pause);
        match &got {
            Err(MoveError::Stranded { folder: f, .. }) => assert_eq!(f, &folder),
            other => panic!("{other:?}"),
        }
        let said = got.unwrap_err().to_string();
        assert!(said.contains(&folder.display().to_string()), "{said}");
        assert!(!said.contains("as it was"), "{said}");
        assert!(folder.join("blocks/old").exists(), "the old blocks wait");
        assert!(d.join("blocks/x").exists(), "nothing was overwritten");
        let left = read_record(d).unwrap().expect("the run stays recorded");
        assert_eq!(left.phase, Phase::SettingAside);
        // Once the way is clear, the next start puts it all back.
        std::fs::remove_dir_all(d.join("blocks")).unwrap();
        std::fs::remove_dir_all(folder.join("chainstate")).unwrap();
        restore(d).unwrap();
        assert_as_before(d, "after the way was cleared");
    }

    #[test]
    fn a_run_is_never_set_aside_over_another() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let r = set_aside(d, 232_000, Before::default(), 20).unwrap();
        attempt_made_chain(d);
        let again = set_aside(d, 232_000, Before::default(), 21);
        assert!(
            matches!(&again, Err(MoveError::Untouched(e)) if e.kind() == io::ErrorKind::AlreadyExists),
            "{again:?}"
        );
        assert!(d.join("blocks/new").exists());
        assert!(!d.join(aside_name(21)).exists());
        assert_eq!(read_record(d).unwrap(), Some(r));
        restore(d).unwrap();
        assert_as_before(d, "the first run undone");
    }

    /// A crash between any two steps of setting aside: the next start finds
    /// the run half set aside and undoes it. Nothing is removed (the node
    /// never ran on new data) and what moved comes back.
    #[test]
    fn a_crash_while_setting_aside_is_undone_at_the_next_start() {
        let mut crashed = 0;
        for k in 1.. {
            let tmp = datadir_with_chain();
            let d = tmp.path();
            let got = set_aside_with(
                d,
                CHAIN_DATA,
                232_000,
                Before::default(),
                9,
                &mut crash_at(k),
            );
            if got.is_ok() {
                break;
            }
            crashed += 1;
            if let Some(r) = next_start(d, 50_000) {
                assert_eq!(r.phase, Phase::SettingAside, "a crash at stop {k}");
                assert_eq!(r.watch_started_at, 9, "not watched again, stop {k}");
                assert!(
                    matches!(judge(&r, &on_the_snapshot(), 50_000), Verdict::RollBack(_)),
                    "a crash at stop {k}"
                );
            }
            restore(d).unwrap();
            assert_as_before(d, &format!("a crash at stop {k}"));
        }
        // The record, the dated folder, six moves, the undo copy, the record
        // again.
        assert_eq!(crashed, 10);
    }

    /// A crash between any two steps of a restore, then the next start as
    /// the app runs it (the sweep, resume, judge): the run is never watched
    /// again or judged done, however the node looks, so the node is never
    /// launched on the half-removed chain; it is rolled back, and the
    /// datadir ends as it was before the run. A second restore after that
    /// finds nothing to do.
    #[test]
    fn a_crash_while_restoring_is_finished_at_the_next_start() {
        let mut crashed = 0;
        for k in 1.. {
            let tmp = datadir_with_chain();
            let d = tmp.path();
            set_aside(d, 232_000, Before::default(), 9).unwrap();
            attempt_made_chain(d);
            if restore_with(d, &mut crash_at(k)).is_ok() {
                assert_as_before(d, "no crash");
                break;
            }
            crashed += 1;
            // The undo was marked before anything went: the copy set_aside
            // wrote was renamed into place.
            assert!(!d.join(".fast-forward-undo.json").exists(), "stop {k}");
            let left = next_start(d, 50_000).expect("the run stays recorded");
            let phase = if k <= 5 {
                Phase::Undoing
            } else {
                Phase::Restoring
            };
            assert_eq!(left.phase, phase, "stop {k}");
            assert_eq!(left.watch_started_at, 9, "not watched again, stop {k}");
            assert!(
                matches!(
                    judge(&left, &on_the_snapshot(), 50_000),
                    Verdict::RollBack(_)
                ),
                "rolled back, not watched, stop {k}"
            );
            assert!(finish(d).is_err(), "never finished, stop {k}");
            let undone = restore(d).unwrap();
            assert_eq!(undone.map(|u| u.height), Some(232_000), "stop {k}");
            assert_as_before(d, &format!("a crash at stop {k}"));
            assert!(restore(d).unwrap().is_none(), "called twice, stop {k}");
            assert_as_before(d, &format!("restored twice, stop {k}"));
        }
        // Four removals (blocks, chainstate, indexes, the start record), the
        // record, six moves back, the folder, the record.
        assert_eq!(crashed, 13);
    }

    /// The undo is marked by renaming the copy set_aside wrote, which needs
    /// no free space. Without that copy, or with one that is not this
    /// run's, the mark is written instead.
    #[test]
    fn the_undo_is_marked_even_without_its_copy() {
        for copy in ["missing", "another run's"] {
            let tmp = datadir_with_chain();
            let d = tmp.path();
            let r = set_aside(d, 232_000, Before::default(), 9).unwrap();
            assert!(
                d.join(".fast-forward-undo.json").is_file(),
                "set_aside wrote the copy"
            );
            attempt_made_chain(d);
            if copy == "missing" {
                std::fs::remove_file(d.join(".fast-forward-undo.json")).unwrap();
            } else {
                let other = Record {
                    aside: aside_name(8),
                    phase: Phase::Undoing,
                    ..r.clone()
                };
                std::fs::write(
                    d.join(".fast-forward-undo.json"),
                    serde_json::to_vec(&other).unwrap(),
                )
                .unwrap();
            }
            assert!(restore_with(d, &mut crash_at(1)).is_err());
            let left = read_record(d).unwrap().unwrap();
            assert_eq!(left.phase, Phase::Undoing, "{copy}");
            assert_eq!(left.aside, r.aside, "{copy}");
            restore(d).unwrap();
            assert_as_before(d, copy);
        }
    }

    /// Stopped with the attempt's data gone, the record saying so, and two
    /// of six entries back: the next restore moves the other four.
    #[test]
    fn restore_carries_on_when_some_of_the_old_data_is_back() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let r = set_aside(d, 232_000, Before::default(), 9).unwrap();
        attempt_made_chain(d);
        assert!(restore_with(d, &mut crash_at(8)).is_err());
        let left = read_record(d).unwrap().unwrap();
        assert_eq!(left.phase, Phase::Restoring);
        let folder = d.join(&r.aside);
        let waiting = r
            .moved
            .iter()
            .filter(|n| present(&folder.join(n)).unwrap())
            .count();
        assert_eq!(waiting, 4);
        assert!(d.join("blocks/old").exists() && d.join("chainstate/old").exists());
        assert!(!d.join("indexes").exists(), "the attempt's is gone");
        restore(d).unwrap();
        assert_as_before(d, "carried on");
    }

    /// Two entries cannot go back (something is in their places): the other
    /// four go back anyway, and the error names both.
    #[test]
    fn a_restore_brings_back_all_it_can_and_names_the_rest() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let r = set_aside(d, 232_000, Before::default(), 9).unwrap();
        attempt_made_chain(d);
        // Stopped with phase restoring written, before anything went back.
        assert!(restore_with(d, &mut crash_at(6)).is_err());
        for name in ["blocks", "chainstate"] {
            std::fs::create_dir_all(d.join(name).join("x")).unwrap();
        }
        let got = restore(d);
        let said = match &got {
            Err(e @ MoveError::Stranded { .. }) => e.to_string(),
            other => panic!("{other:?}"),
        };
        assert!(
            said.contains("blocks") && said.contains("chainstate"),
            "{said}"
        );
        for name in ["chainstate_snapshot", "indexes", "shielded_state"] {
            assert!(d.join(name).join("old").exists(), "{name} is back");
        }
        assert_eq!(
            std::fs::read_to_string(d.join("snapshot-start.json")).unwrap(),
            "old start"
        );
        let folder = d.join(&r.aside);
        assert!(folder.join("blocks/old").exists() && folder.join("chainstate/old").exists());
        assert_eq!(read_record(d).unwrap().unwrap().phase, Phase::Restoring);
        for name in ["blocks", "chainstate"] {
            std::fs::remove_dir_all(d.join(name)).unwrap();
        }
        restore(d).unwrap();
        assert_as_before(d, "the rest after the way was cleared");
    }

    #[test]
    fn a_stray_file_in_the_dated_folder_does_not_stop_the_restore() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let r = set_aside(d, 232_000, Before::default(), 9).unwrap();
        attempt_made_chain(d);
        std::fs::write(d.join(&r.aside).join(".DS_Store"), b"finder").unwrap();
        restore(d).unwrap();
        assert_as_before(d, "a .DS_Store in the folder");
    }

    /// Chain data in the dated folder that the record does not list is not
    /// the run's to delete: the folder stays and the restore says where.
    #[test]
    fn chain_data_the_record_does_not_list_is_never_deleted() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        std::fs::remove_dir_all(d.join("indexes")).unwrap();
        let r = set_aside(d, 232_000, Before::default(), 9).unwrap();
        let folder = d.join(&r.aside);
        std::fs::create_dir_all(folder.join("indexes")).unwrap();
        std::fs::write(folder.join("indexes/old"), b"indexes").unwrap();
        let got = restore(d);
        assert!(matches!(got, Err(MoveError::Stranded { .. })), "{got:?}");
        assert!(folder.join("indexes/old").exists());
        assert!(d.join("blocks/old").exists(), "the listed data is back");
    }

    #[test]
    fn finish_removes_only_the_dated_folder() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        set_aside(d, 232_000, Before::default(), 11).unwrap();
        attempt_made_chain(d);
        finish(d).unwrap();
        assert_new_chain_kept(d, "finished");
        assert!(restore(d).unwrap().is_none(), "nothing left to put back");
        assert_new_chain_kept(d, "a restore after the finish");
        finish(d).unwrap();
    }

    /// Once a run may have lost any of its old chain data, nothing puts
    /// the rest back: phase done is written before anything is deleted, the
    /// folder then takes a name restore never accepts, and the sweep at the
    /// next start removes whatever a crash left.
    #[test]
    fn a_crash_while_finishing_never_brings_the_old_chain_back() {
        let on_it = Look {
            snapshot_base_height: Some(232_000),
            running: true,
            loaded: true,
            ..Look::default()
        };
        let mut crashed = 0;
        for k in 1.. {
            let tmp = datadir_with_chain();
            let d = tmp.path();
            let r = set_aside(d, 232_000, Before::default(), 9).unwrap();
            attempt_made_chain(d);
            if finish_with(d, &mut crash_at(k)).is_ok() {
                assert_new_chain_kept(d, "no crash");
                break;
            }
            crashed += 1;
            match read_record(d).unwrap() {
                Some(left) if left.phase == Phase::Running => {
                    for name in &r.moved {
                        assert!(d.join(&r.aside).join(name).exists(), "{name}, stop {k}");
                    }
                }
                Some(left) => {
                    assert_eq!(left.phase, Phase::Done, "stop {k}");
                    assert!(
                        matches!(restore(d), Err(MoveError::Untouched(_))),
                        "a finished run is not undone, stop {k}"
                    );
                }
                None => assert!(restore(d).unwrap().is_none(), "stop {k}"),
            }
            // The next start: the sweep, then the run is watched again and
            // judged done.
            sweep(d);
            if let Some(left) = resume(d, 20).unwrap() {
                assert_eq!(judge(&left, &on_it, 20), Verdict::Done, "stop {k}");
                finish(d).unwrap();
            }
            assert_new_chain_kept(d, &format!("a crash at stop {k}"));
        }
        // Phase done, the rename, the record, the folder.
        assert_eq!(crashed, 4);
    }

    #[test]
    fn a_run_being_undone_is_never_finished() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        // Stopped after the record, the folder and two moves.
        assert!(set_aside_with(
            d,
            CHAIN_DATA,
            232_000,
            Before::default(),
            9,
            &mut crash_at(5)
        )
        .is_err());
        assert!(finish(d).is_err());
        assert!(d.join(aside_name(9)).join("blocks/old").exists());
        restore(d).unwrap();
        assert_as_before(d, "setting aside undone");
        // Stopped with phase restoring written, before anything went back.
        set_aside(d, 232_000, Before::default(), 10).unwrap();
        attempt_made_chain(d);
        assert!(restore_with(d, &mut crash_at(6)).is_err());
        assert_eq!(read_record(d).unwrap().unwrap().phase, Phase::Restoring);
        assert!(finish(d).is_err());
        assert!(d.join(aside_name(10)).join("blocks/old").exists());
        restore(d).unwrap();
        assert_as_before(d, "restore carried on");
        // Stopped with the undo marked, before anything was removed.
        set_aside(d, 232_000, Before::default(), 12).unwrap();
        attempt_made_chain(d);
        assert!(restore_with(d, &mut crash_at(1)).is_err());
        assert_eq!(read_record(d).unwrap().unwrap().phase, Phase::Undoing);
        assert!(finish(d).is_err());
        assert!(d.join(aside_name(12)).join("blocks/old").exists());
        restore(d).unwrap();
        assert_as_before(d, "undo carried on");
    }

    #[test]
    fn the_sweep_removes_only_folders_no_run_needs() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        for dir in [
            "fast-forward-5/blocks",
            "fast-forward-6.discard/chainstate",
            "fast-forward-x",
            "fast-forward-8.old",
            "chainstate_snapshot.refused-9",
        ] {
            std::fs::create_dir_all(d.join(dir)).unwrap();
        }
        std::fs::write(d.join("fast-forward-7"), b"a file, not ours").unwrap();
        // What a crash inside fsx::atomic_write leaves: `.<name>.<pid>.<n>.tmp`.
        let leftovers = [
            "..fast-forward.json.4242.0.tmp",
            "..fast-forward-result.json.4242.1.tmp",
            "..fast-forward-undo.json.4242.2.tmp",
        ];
        for name in leftovers {
            std::fs::write(d.join(name), b"half").unwrap();
        }
        let others = [
            "fast-forward-x",
            "fast-forward-8.old",
            "chainstate_snapshot.refused-9",
            "fast-forward-7",
            "..fast-forward.json.tmp",
            "..fast-forward.json.4242.x.tmp",
            "..settings.json.4242.0.tmp",
            ".fast-forward.json.4242.0.tmp.keep",
        ];
        for name in &others[4..] {
            std::fs::write(d.join(name), b"not ours").unwrap();
        }
        // While a run is recorded, or a record cannot be read, no folder
        // and no undo copy goes; a half-written temporary file always does.
        set_aside(d, 232_000, Before::default(), 11).unwrap();
        let mut swept = sweep(d);
        swept.sort();
        let mut half: Vec<PathBuf> = leftovers.iter().map(|n| d.join(n)).collect();
        half.sort();
        assert_eq!(swept, half);
        assert!(d.join(aside_name(11)).join("blocks/old").exists());
        assert!(d.join(".fast-forward-undo.json").exists());
        restore(d).unwrap();
        std::fs::write(d.join(".fast-forward.json"), b"not json").unwrap();
        std::fs::write(d.join(".fast-forward-undo.json"), b"{}").unwrap();
        assert!(sweep(d).is_empty());
        assert!(d.join("fast-forward-5/blocks").exists());
        assert!(d.join(".fast-forward-undo.json").exists());
        std::fs::remove_file(d.join(".fast-forward.json")).unwrap();
        // With no run, the dated folders and an undo copy no run needs go,
        // and nothing else.
        let mut swept = sweep(d);
        swept.sort();
        assert_eq!(
            swept,
            vec![
                d.join(".fast-forward-undo.json"),
                d.join("fast-forward-5"),
                d.join("fast-forward-6.discard")
            ]
        );
        for other in others {
            assert!(d.join(other).exists(), "{other}");
            remove_any(&d.join(other)).unwrap();
        }
        assert_as_before(d, "swept");
    }

    /// A link named like a dated folder is not ours to follow: the sweep
    /// leaves it, and what it points at.
    #[cfg(unix)]
    #[test]
    fn the_sweep_leaves_a_link_named_like_a_dated_folder() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(elsewhere.join("blocks")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, d.join("fast-forward-12")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, d.join("fast-forward-13.discard")).unwrap();
        assert!(sweep(d).is_empty());
        assert!(std::fs::symlink_metadata(d.join("fast-forward-12")).is_ok());
        assert!(std::fs::symlink_metadata(d.join("fast-forward-13.discard")).is_ok());
        assert!(elsewhere.join("blocks").exists());
    }

    /// Review M6: a run whose old chain cannot come back is given up with
    /// the rest of the node's data: its record and undo copy go, and the
    /// sweep takes its dated folder. A running run whose old chain is all
    /// there can still be rolled back, and a run at any other phase is on
    /// its way somewhere: both are refused, and nothing changes.
    #[test]
    fn a_run_whose_old_chain_cannot_come_back_can_be_given_up() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let r = set_aside(d, 232_000, Before::default(), 9).unwrap();
        attempt_made_chain(d);
        assert!(abandon(d).is_err(), "the old chain can still come back");
        assert_eq!(read_record(d).unwrap(), Some(r.clone()));
        assert!(d.join(".fast-forward-undo.json").exists());
        std::fs::remove_dir_all(d.join(&r.aside).join("chainstate")).unwrap();
        assert!(matches!(restore(d), Err(MoveError::Stranded { .. })));
        abandon(d).unwrap();
        assert_eq!(read_record(d).unwrap(), None);
        assert!(!d.join(".fast-forward-undo.json").exists());
        let mut swept = sweep(d);
        swept.sort();
        assert_eq!(swept, vec![d.join(&r.aside)]);
        assert!(
            d.join("blocks/new").exists(),
            "the rest is Remove node data's"
        );
        abandon(d).unwrap();

        for phase in [
            Phase::SettingAside,
            Phase::Undoing,
            Phase::Restoring,
            Phase::Done,
        ] {
            let tmp = datadir_with_chain();
            let d = tmp.path();
            let r = set_aside(d, 232_000, Before::default(), 9).unwrap();
            let at = Record { phase, ..r };
            write_record(d, &at).unwrap();
            assert!(abandon(d).is_err(), "{phase:?}");
            assert_eq!(read_record(d).unwrap(), Some(at), "{phase:?}");
        }
    }

    /// The owner's decision: time the app was closed does not count against
    /// a run. Resumed, it is watched for the full limit again.
    #[test]
    fn a_resumed_run_gets_a_fresh_watch_window() {
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let r = set_aside(d, 232_000, Before::default(), 1_000).unwrap();
        assert_eq!(r.watch_started_at, 1_000);
        let four_hours_later = 1_000 + 4 * 60 * 60;
        assert!(matches!(
            judge(&r, &Look::default(), four_hours_later),
            Verdict::RollBack(_)
        ));
        let resumed = resume(d, four_hours_later).unwrap().unwrap();
        assert_eq!(resumed.watch_started_at, four_hours_later);
        assert_eq!(resumed.started_at, 1_000);
        assert_eq!(read_record(d).unwrap(), Some(resumed.clone()), "kept");
        assert_eq!(
            judge(&resumed, &Look::default(), four_hours_later),
            Verdict::Continue,
            "the first look after a resume"
        );
        assert_eq!(
            judge(
                &resumed,
                &Look::default(),
                four_hours_later + MAX_RUN_SECS - 1
            ),
            Verdict::Continue
        );
        assert!(matches!(
            judge(&resumed, &Look::default(), four_hours_later + MAX_RUN_SECS),
            Verdict::RollBack(_)
        ));
        restore(d).unwrap();
        assert_eq!(resume(d, 5).unwrap(), None, "no run, nothing to resume");
        // A run stopped mid-move is left as it is, for judge to roll back.
        assert!(set_aside_with(
            d,
            CHAIN_DATA,
            232_000,
            Before::default(),
            9,
            &mut crash_at(4)
        )
        .is_err());
        let left = read_record(d).unwrap();
        assert_eq!(resume(d, 99_999).unwrap(), left);
        restore(d).unwrap();
        // And so is a run whose undo has begun.
        set_aside(d, 232_000, Before::default(), 30).unwrap();
        attempt_made_chain(d);
        assert!(restore_with(d, &mut crash_at(1)).is_err());
        let left = read_record(d).unwrap();
        assert_eq!(left.as_ref().map(|r| r.phase), Some(Phase::Undoing));
        assert_eq!(resume(d, 99_999).unwrap(), left);
        assert_eq!(read_record(d).unwrap(), left, "nothing written");
    }

    /// Review M7: the fresh watch window at every start has a total bound,
    /// counted from the run's own start. A crash loop, or an app never kept
    /// open three hours in a row, does not keep a run (and a node with no
    /// usable chain) going for ever: a day after it began, it is rolled
    /// back, however recently it was resumed. On the snapshot it is still
    /// done, however long it took. A clock that jumps forward only rolls
    /// back early; one set back before the start is no reason either way.
    #[test]
    fn a_run_is_rolled_back_a_day_after_it_began_however_often_it_resumed() {
        let started = 1_000;
        let resumed = Record {
            watch_started_at: started + MAX_TOTAL_SECS - 60 * 60,
            ..running_record(started)
        };
        assert_eq!(
            judge(&resumed, &Look::default(), started + MAX_TOTAL_SECS - 1),
            Verdict::Continue,
            "a second short of a day"
        );
        assert_eq!(
            judge(&resumed, &Look::default(), started + MAX_TOTAL_SECS),
            Verdict::RollBack("it had not finished 24 hours after it started".into()),
            "an hour into a fresh window"
        );
        assert_eq!(
            judge(&resumed, &on_the_snapshot(), started + 2 * MAX_TOTAL_SECS),
            Verdict::Done
        );
        let fresh = running_record(started);
        assert!(matches!(
            judge(&fresh, &Look::default(), started + 10 * MAX_TOTAL_SECS),
            Verdict::RollBack(_)
        ));
        assert_eq!(
            judge(&fresh, &Look::default(), started - 500),
            Verdict::Continue,
            "a clock set back"
        );
        assert!(MAX_TOTAL_SECS > MAX_RUN_SECS);
    }

    /// Review M1: a resume that cannot write the fresh watch window (on a
    /// disk a failed load filled, say) keeps the window it had and carries
    /// on: the run is still watched, and rolled back once that window ends
    /// (the undo's mark needs no space). Only a record that cannot be read
    /// is an error.
    #[cfg(unix)]
    #[test]
    fn a_resume_that_cannot_write_keeps_the_window_it_had() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = datadir_with_chain();
        let d = tmp.path();
        let r = set_aside(d, 232_000, Before::default(), 1_000).unwrap();
        attempt_made_chain(d);
        let mode = |m| std::fs::set_permissions(d, std::fs::Permissions::from_mode(m)).unwrap();
        mode(0o555);
        if std::fs::write(d.join("probe"), b"").is_ok() {
            mode(0o755);
            eprintln!("skipped: this user can write a read-only folder");
            return;
        }
        let got = resume(d, 50_000);
        let kept = read_record(d);
        mode(0o755);
        assert_eq!(got.unwrap(), Some(r.clone()), "the window it had");
        assert_eq!(kept.unwrap(), Some(r.clone()));
        assert!(matches!(
            judge(&r, &Look::default(), 50_000),
            Verdict::RollBack(_)
        ));
        restore(d).unwrap();
        assert_as_before(d, "rolled back with the window it had");
    }

    #[test]
    fn the_record_and_the_outcome_survive_a_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        assert_eq!(read_record(d).unwrap(), None);
        let r = Record {
            moved: vec!["blocks".into()],
            snapshot_loaded_before: true,
            first_load_pending_before: true,
            ..running_record(1)
        };
        write_record(d, &r).unwrap();
        assert_eq!(read_record(d).unwrap(), Some(r.clone()));
        clear_record(d).unwrap();
        assert_eq!(read_record(d).unwrap(), None);
        clear_record(d).unwrap();
        // A record that cannot be read is an error, never "no run": the
        // driver refuses to start a run over it.
        std::fs::write(d.join(".fast-forward.json"), b"{\"height\":").unwrap();
        let e = read_record(d).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
        // One written before the first-load setting was kept reads as false.
        std::fs::write(
            d.join(".fast-forward.json"),
            r#"{"height":232000,"aside":"fast-forward-1","moved":["blocks"],"phase":"running","snapshot_loaded_before":true,"started_at":1,"watch_started_at":1}"#,
        )
        .unwrap();
        assert!(!read_record(d).unwrap().unwrap().first_load_pending_before);
        clear_record(d).unwrap();
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
        let r = running_record(1_000);
        let on_it = Look {
            snapshot_base_height: Some(232_000),
            running: true,
            loaded: true,
            ..Look::default()
        };
        assert_eq!(judge(&r, &on_it, 2_000), Verdict::Done);
        // Review I2: the engine shows the snapshot chainstate before the
        // loader's own check after the load has passed, and a refusal can
        // still come then. Not done until the loader says it loaded, on a
        // newer snapshot too; the limit still applies meanwhile.
        for height in [232_000, 232_200] {
            let checking = Look {
                snapshot_base_height: Some(height),
                loaded: false,
                ..on_it.clone()
            };
            assert_eq!(judge(&r, &checking, 2_000), Verdict::Continue, "{height}");
            assert!(matches!(
                judge(&r, &checking, 1_000 + MAX_RUN_SECS),
                Verdict::RollBack(_)
            ));
        }
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
        assert_eq!(
            judge(&r, &Look::default(), 1_000 + MAX_RUN_SECS - 1),
            Verdict::Continue,
            "a second short of the limit"
        );
        assert!(matches!(
            judge(&r, &Look::default(), 1_000 + MAX_RUN_SECS),
            Verdict::RollBack(_)
        ));
        assert_eq!(
            judge(&r, &on_it, 1_000 + MAX_RUN_SECS + 1),
            Verdict::Done,
            "on the snapshot is done, however long it took"
        );
        // A run recorded done is done, whatever the node shows now.
        let finished = Record {
            phase: Phase::Done,
            ..running_record(1_000)
        };
        let gone_wrong = Look {
            load_failed: Some("late".into()),
            ..Look::default()
        };
        assert_eq!(
            judge(&finished, &gone_wrong, 1_000 + MAX_RUN_SECS),
            Verdict::Done
        );
        // A record left between two steps of a move is undone, whatever the
        // node shows, and the reason names the step (review M9).
        for (phase, step) in [
            (Phase::SettingAside, "setting the chain data aside"),
            (Phase::Undoing, "undoing Fast-forward"),
            (Phase::Restoring, "putting the old chain data back"),
        ] {
            let r = Record {
                phase,
                ..running_record(1_000)
            };
            assert_eq!(
                judge(&r, &on_it, 2_000),
                Verdict::RollBack(format!("the app stopped while it was {step}")),
                "{phase:?}"
            );
        }
    }

    /// Paths in the record come from a file: the dated folder must be one
    /// this module names and every entry chain data, or nothing is touched.
    #[test]
    fn a_record_that_names_anything_but_chain_data_is_refused() {
        // Two levels down, so a check that let ".." through would still only
        // reach inside this test's own folder.
        let tmp = tempfile::tempdir().unwrap();
        let d = &tmp.path().join("root/node");
        fill_datadir(d);
        let good = Record {
            moved: vec!["blocks".into()],
            ..running_record(5)
        };
        let outside = tmp.path().join("root/outside");
        std::fs::create_dir_all(outside.join("blocks")).unwrap();
        let bad_asides = [
            String::new(),
            "..".into(),
            outside.display().to_string(),
            "wallets".into(),
            "fast-forward-".into(),
            "fast-forward-5/../wallets".into(),
            "fast-forward-5.discard".into(),
        ];
        let bad_moved = ["wallets", "..", "", "blocks/../wallets"];
        let mut bad = Vec::new();
        for aside in bad_asides {
            bad.push(Record {
                aside,
                ..good.clone()
            });
        }
        for name in bad_moved {
            bad.push(Record {
                moved: vec!["blocks".into(), name.into()],
                ..good.clone()
            });
        }
        for r in bad {
            write_record(d, &r).unwrap();
            let e = read_record(d).unwrap_err();
            assert_eq!(e.kind(), std::io::ErrorKind::InvalidData, "{r:?}");
            assert!(matches!(restore(d), Err(MoveError::Untouched(_))), "{r:?}");
            assert!(finish(d).is_err(), "{r:?}");
            assert!(sweep(d).is_empty(), "{r:?}");
            assert!(d.join("wallets/main/wallet.dat").exists(), "{r:?}");
            assert!(d.join("blocks/old").exists(), "{r:?}");
            assert!(outside.join("blocks").exists(), "{r:?}");
        }
        write_record(d, &good).unwrap();
        assert_eq!(read_record(d).unwrap(), Some(good));
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
        assert!(c.contains(
            " Your wallets and keys stay as they are, and afterwards the node checks the older history in the background. "
        ));
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
