//! Getting a node running again after engine v0.34.12's "Failed to read
//! block" fatal (`crate::node::log_shows_read_block_fatal`; root cause in
//! .superpowers/sdd/progress.md).
//!
//! Once the fatal has happened, every start dies the same way in under a
//! second: the signatures that pointed the background chainstate at a block
//! it never downloaded are stored, and loaded before the first activation.
//! Retrying unchanged cannot help. This is a ladder, one rung per start,
//! at most once per incident:
//!
//! 1. [`Step::Signatures`]: the stored signatures
//!    ([`SIGNATURE_FILES`]) go into a dated folder, and the engine makes a
//!    fresh archive and fetches signatures from its peers again. Only on a
//!    trusted mirror with no signing key of its own ([`signatures_may_go`]):
//!    a node that signs keeps its own signatures in that archive.
//! 2. [`Step::ChainData`]: the chain data (`fast_forward::CHAIN_DATA`) goes
//!    into the same folder (with the signatures, unless the node has its own
//!    key), and the node starts again from the snapshot, as a fresh install.
//! 3. Rolled back ([`roll_back`]): everything goes back as it was, and the
//!    node is not started again. The outcome stays ([`Outcome::RolledBack`])
//!    so the next start does not begin a second ladder ([`next_step`]).
//!
//! The record (`<datadir>/.read-block-recovery.json`) is written before
//! anything moves and says which step ran and which phase it is at, so a
//! move cut off by a crash is carried on at the next start ([`at_start`]):
//! a step that was moving finishes its move, and a roll-back finishes
//! putting back. Every move is a rename (`crate::aside`); nothing the node
//! had is deleted, except the old chain data once the new start has
//! stayed up ([`finish`], Fast-forward's rule for its folders). What the
//! ladder's own starts made (a fresh archive, fresh chain data) is removed
//! on a roll-back, as Fast-forward removes what its attempt made.

use crate::aside::{move_entries, present, remove_any, Pause, Stuck};
use crate::fast_forward::{Before, MoveError, CHAIN_DATA};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// The durable signature archive: `matmul_attestations.dat`, its LevelDB
/// folder, its write-ahead log and an interrupted write's temporary file
/// (node/matmul_trusted_attestations.cpp at v0.34.12).
pub const SIGNATURE_FILES: [&str; 4] = [
    "matmul_attestations.dat",
    "matmul_attestations.dat.db",
    "matmul_attestations.dat.wal",
    "matmul_attestations.dat.tmp",
];

/// How long a start after a step must stay up before the step counts as
/// having worked. The restart fatal fires in under a second of init, and
/// the start's own work that could trip it again (the first activation of
/// both chainstates, the snapshot load, the first blocks from peers) is
/// over within minutes; ten covers it with room. It cannot cover the rarer
/// race while running (about 75 minutes in the field), which needs the
/// engine fix; a later crash of that kind starts a new ladder.
pub const WATCH_SECS: u64 = 10 * 60;

const RECORD_FILE: &str = ".read-block-recovery.json";
const OUTCOME_FILE: &str = ".read-block-recovery-result.json";
const ASIDE_PREFIX: &str = "read-block-recovery-";

/// Which rung ran last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Signatures,
    ChainData,
}

/// Where the step is. Written before the work it names begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The step's entries are moving into the dated folder. The node has
    /// not run since.
    Moving,
    /// Moved; the node runs and is watched for [`WATCH_SECS`].
    Running,
    /// A roll-back has begun: what the ladder's starts made is being
    /// removed, and every original is still in the dated folder (or, cut
    /// off while moving, still in its place).
    Undoing,
    /// The originals are going back.
    Restoring,
    /// The start stayed up: the old chain data is being removed.
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub step: Step,
    pub phase: Phase,
    /// The dated folder, relative to the datadir: always [`aside_name`]'s.
    pub aside: String,
    /// The signature entries that are (or are going) in the dated folder:
    /// the node's own, as they were before the ladder.
    pub signatures: Vec<String>,
    /// The chain data entries likewise.
    pub chain: Vec<String>,
    pub snapshot_loaded_before: bool,
    pub first_load_pending_before: bool,
    pub started_at: u64,
}

impl Record {
    pub fn before(&self) -> Before {
        Before {
            snapshot_loaded: self.snapshot_loaded_before,
            first_load_pending: self.first_load_pending_before,
        }
    }

    fn originals(&self) -> Vec<String> {
        self.signatures
            .iter()
            .chain(self.chain.iter())
            .cloned()
            .collect()
    }
}

/// How the last ladder ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    /// The start after `step` stayed up. `folder` holds the old stored
    /// signatures, when any were set aside.
    Recovered {
        step: Step,
        folder: Option<String>,
        at: u64,
    },
    /// Everything went back and the node was not started again.
    RolledBack { at: u64 },
}

/// What a start that died on the fatal does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    SetAsideSignatures,
    SetAsideChainData {
        signatures: bool,
    },
    RollBack,
    /// A ladder ran for this incident and was rolled back: nothing moves
    /// again until a start gets the node running ([`clear_outcome`]).
    AlreadyTried,
}

/// What the app knows about the launch that died.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Facts {
    /// The launch ran as a trusted mirror ([`launch_is_trusted_mirror`]).
    pub trusted_mirror: bool,
    /// The node has a signing key of its own ([`has_own_signing_key`]).
    pub own_signing_key: bool,
    /// Any of [`SIGNATURE_FILES`] is in the datadir.
    pub signature_files_present: bool,
}

/// Step 1 is allowed: a trusted mirror whose archive holds only other
/// nodes' signatures.
pub fn signatures_may_go(trusted_mirror: bool, own_signing_key: bool) -> bool {
    trusted_mirror && !own_signing_key
}

/// Pure: the ladder's next rung.
pub fn next_step(record: Option<&Record>, last: Option<&Outcome>, facts: Facts) -> Next {
    match record.map(|r| r.step) {
        Some(Step::ChainData) => Next::RollBack,
        Some(Step::Signatures) => Next::SetAsideChainData {
            signatures: !facts.own_signing_key,
        },
        None if matches!(last, Some(Outcome::RolledBack { .. })) => Next::AlreadyTried,
        None if signatures_may_go(facts.trusted_mirror, facts.own_signing_key)
            && facts.signature_files_present =>
        {
            Next::SetAsideSignatures
        }
        None => Next::SetAsideChainData {
            signatures: !facts.own_signing_key,
        },
    }
}

/// The last `-matmulvalidation=` of a launch's arguments says `trusted`.
pub fn launch_is_trusted_mirror(args: &[String]) -> bool {
    args.iter()
        .rev()
        .find_map(|a| a.strip_prefix("-matmulvalidation="))
        == Some("trusted")
}

/// The node has a signing key of its own: the key file the app makes
/// (`crate::signer::SIGNER_KEY_FILE`), or any of the engine's signing-key
/// options in one of `confs`. Either means its archive may hold its own
/// signatures, which nothing here may move.
pub fn has_own_signing_key(datadir: &Path, confs: &[&Path]) -> bool {
    if present(&crate::signer::signer_key_path(datadir)).unwrap_or(true) {
        return true;
    }
    confs.iter().any(|conf| {
        std::fs::read_to_string(conf).is_ok_and(|text| {
            text.lines().map(str::trim).any(|l| {
                l.split_once('=').is_some_and(|(k, _)| {
                    let k = k.trim();
                    k.starts_with("matmulattestationsignerkey")
                        || k == "matmulattestationsignerpqfile"
                })
            })
        })
    })
}

/// Any of [`SIGNATURE_FILES`] is in the datadir.
pub fn signature_files_present(datadir: &Path) -> bool {
    SIGNATURE_FILES
        .iter()
        .any(|n| present(&datadir.join(n)).unwrap_or(true))
}

/// The dated folder's name.
pub fn aside_name(now_unix: u64) -> String {
    format!("{ASIDE_PREFIX}{now_unix}")
}

fn is_aside_name(name: &str) -> bool {
    name.strip_prefix(ASIDE_PREFIX)
        .is_some_and(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
}

/// The dated folder, as a path.
pub fn folder(datadir: &Path, record: &Record) -> PathBuf {
    datadir.join(&record.aside)
}

fn record_path(datadir: &Path) -> PathBuf {
    datadir.join(RECORD_FILE)
}

fn check(record: &Record) -> io::Result<()> {
    let bad = |what: String| io::Error::new(io::ErrorKind::InvalidData, what);
    if !is_aside_name(&record.aside) {
        return Err(bad(format!(
            "the recovery record names {:?} as its folder",
            record.aside
        )));
    }
    if let Some(n) = record
        .signatures
        .iter()
        .find(|n| !SIGNATURE_FILES.contains(&n.as_str()))
        .or_else(|| {
            record
                .chain
                .iter()
                .find(|n| !CHAIN_DATA.contains(&n.as_str()))
        })
    {
        return Err(bad(format!("the recovery record lists {n:?}")));
    }
    Ok(())
}

/// The ladder on disk. `Ok(None)` only when there is none; one that cannot
/// be read, or names anything it may not move, is an error.
pub fn read_record(datadir: &Path) -> io::Result<Option<Record>> {
    let bytes = match std::fs::read(record_path(datadir)) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let record: Record = serde_json::from_slice(&bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    check(&record)?;
    Ok(Some(record))
}

fn write_record(datadir: &Path, record: &Record) -> io::Result<()> {
    let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
    crate::fsx::atomic_write(&record_path(datadir), &bytes)
}

fn clear_record(datadir: &Path) -> io::Result<()> {
    match std::fs::remove_file(record_path(datadir)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

pub fn write_outcome(datadir: &Path, outcome: &Outcome) {
    if let Ok(bytes) = serde_json::to_vec(outcome) {
        let _ = crate::fsx::atomic_write(&datadir.join(OUTCOME_FILE), &bytes);
    }
}

pub fn read_outcome(datadir: &Path) -> Option<Outcome> {
    serde_json::from_slice(&std::fs::read(datadir.join(OUTCOME_FILE)).ok()?).ok()
}

pub fn clear_outcome(datadir: &Path) {
    let _ = std::fs::remove_file(datadir.join(OUTCOME_FILE));
}

fn existing(datadir: &Path, names: &[&str]) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    for n in names {
        if present(&datadir.join(n))? {
            out.push(n.to_string());
        }
    }
    Ok(out)
}

fn stranded(folder: &Path, error: io::Error) -> MoveError {
    MoveError::Stranded {
        folder: folder.to_path_buf(),
        error,
    }
}

fn worded(stuck: Vec<Stuck>, from: &str, to: &str) -> Option<io::Error> {
    let said: Vec<String> = stuck
        .iter()
        .map(|s| match s {
            Stuck::Unreadable { name, error } => format!("{name}: {error}"),
            Stuck::RenameFailed { name, error } => {
                format!("{name} could not be moved to {to}: {error}")
            }
            Stuck::InBoth { name } => format!("{name} is in {from} and in {to} too"),
            Stuck::InNeither { name } => format!("{name} is neither in {from} nor in {to}"),
        })
        .collect();
    (!said.is_empty()).then(|| io::Error::other(said.join("; ")))
}

/// Take a rung: record it, then move. Call with the node stopped (and the
/// engine's lock held). `SetAsideSignatures` and a first
/// `SetAsideChainData` need no record; a `SetAsideChainData` after step 1
/// carries its record on, in the same folder. On a failure the whole ladder
/// is rolled back ([`roll_back`]): [`MoveError::Untouched`] when that
/// worked, [`MoveError::Stranded`] when it did not and the record stays.
pub fn begin(
    datadir: &Path,
    next: Next,
    before: Before,
    now_unix: u64,
) -> Result<Record, MoveError> {
    begin_with(datadir, next, before, now_unix, &mut || Ok(()), true)
}

fn begin_with(
    datadir: &Path,
    next: Next,
    before: Before,
    now_unix: u64,
    pause: Pause,
    clean_up: bool,
) -> Result<Record, MoveError> {
    use MoveError::Untouched;
    let refused = |what: &str| Untouched(io::Error::new(io::ErrorKind::AlreadyExists, what));
    let existing = |names: &[&str]| existing(datadir, names).map_err(Untouched);
    let fresh = |step, signatures, chain| Record {
        step,
        phase: Phase::Moving,
        aside: aside_name(now_unix),
        signatures,
        chain,
        snapshot_loaded_before: before.snapshot_loaded,
        first_load_pending_before: before.first_load_pending,
        started_at: now_unix,
    };
    let current = read_record(datadir).map_err(Untouched)?;
    let new_ladder = current.is_none();
    let mut record = match (current, next) {
        (None, Next::SetAsideSignatures) => {
            let sigs = existing(&SIGNATURE_FILES)?;
            if sigs.is_empty() {
                return Err(Untouched(io::Error::new(
                    io::ErrorKind::NotFound,
                    "no stored signatures to set aside",
                )));
            }
            fresh(Step::Signatures, sigs, Vec::new())
        }
        (None, Next::SetAsideChainData { signatures }) => {
            let sigs = if signatures {
                existing(&SIGNATURE_FILES)?
            } else {
                Vec::new()
            };
            fresh(Step::ChainData, sigs, existing(CHAIN_DATA)?)
        }
        (Some(r), Next::SetAsideChainData { .. })
            if r.step == Step::Signatures && r.phase == Phase::Running =>
        {
            Record {
                step: Step::ChainData,
                phase: Phase::Moving,
                chain: existing(CHAIN_DATA)?,
                snapshot_loaded_before: before.snapshot_loaded,
                first_load_pending_before: before.first_load_pending,
                ..r
            }
        }
        (Some(_), _) => return Err(refused("a recovery step is already recorded")),
        (None, _) => return Err(refused("no recovery step to take")),
    };
    let dir = folder(datadir, &record);
    if new_ladder && present(&dir).map_err(Untouched)? {
        return Err(refused(&format!("{} is in the way", dir.display())));
    }
    pause().map_err(Untouched)?;
    write_record(datadir, &record).map_err(Untouched)?;
    match carry_on_with(datadir, &mut record, pause) {
        Ok(()) => Ok(record),
        // A test's crash: nothing after it runs.
        Err(error) if !clean_up => Err(Untouched(error)),
        Err(error) => match roll_back_with(datadir, &mut || Ok(())) {
            Ok(_) => Err(Untouched(error)),
            Err(MoveError::Stranded { folder, error: e }) => Err(MoveError::Stranded {
                folder,
                error: io::Error::new(
                    error.kind(),
                    format!("{error}, and putting everything back failed: {e}"),
                ),
            }),
            Err(other) => Err(other),
        },
    }
}

/// A recorded move at [`Phase::Moving`], carried to [`Phase::Running`].
/// A signature entry that is both in the folder and in its place is the
/// engine's fresh one from the start after step 1, and goes; so does any
/// other fresh signature entry then.
fn carry_on_with(datadir: &Path, record: &mut Record, pause: Pause) -> io::Result<()> {
    let dir = folder(datadir, record);
    if !present(&dir)? {
        pause()?;
        std::fs::create_dir(&dir)?;
    }
    if !record.signatures.is_empty() {
        for name in SIGNATURE_FILES {
            let place = datadir.join(name);
            let original_aside = present(&dir.join(name))?;
            let listed = record.signatures.iter().any(|s| s == name);
            if present(&place)? && (original_aside || !listed) {
                pause()?;
                remove_any(&place)?;
            }
        }
    }
    let stuck = move_entries(datadir, &dir, &record.originals(), pause)?;
    if let Some(e) = worded(stuck, "its place", &dir.display().to_string()) {
        return Err(e);
    }
    pause()?;
    record.phase = Phase::Running;
    write_record(datadir, record)
}

/// Put everything back as it was before the ladder, from whichever phase
/// it is at, and drop the record; return it (its settings from before are
/// for the caller to put back). `Ok(None)` when no ladder is recorded.
/// Called again after a failure or a crash, it carries on. Call with the
/// node stopped.
pub fn roll_back(datadir: &Path) -> Result<Option<Record>, MoveError> {
    roll_back_with(datadir, &mut || Ok(()))
}

fn roll_back_with(datadir: &Path, pause: Pause) -> Result<Option<Record>, MoveError> {
    let Some(mut record) = read_record(datadir).map_err(MoveError::Untouched)? else {
        return Ok(None);
    };
    if record.phase == Phase::Done {
        return Err(MoveError::Untouched(io::Error::other(
            "the recovery worked; its old chain data is being removed",
        )));
    }
    let dir = folder(datadir, &record);
    let s = |e| stranded(&dir, e);
    if matches!(record.phase, Phase::Moving | Phase::Running) {
        for name in record.originals() {
            let aside = present(&dir.join(&name)).map_err(s)?;
            let placed = present(&datadir.join(&name)).map_err(s)?;
            if !(aside || record.phase == Phase::Moving && placed) {
                return Err(s(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{name} is missing from {}", dir.display()),
                )));
            }
        }
        pause().map_err(s)?;
        record.phase = Phase::Undoing;
        write_record(datadir, &record).map_err(s)?;
    }
    if record.phase == Phase::Undoing {
        let groups: [(&[&str], &Vec<String>); 2] = [
            (&SIGNATURE_FILES, &record.signatures),
            (CHAIN_DATA, &record.chain),
        ];
        for (names, listed) in groups {
            if listed.is_empty() {
                continue;
            }
            for name in names {
                let place = datadir.join(name);
                let original_aside = present(&dir.join(name)).map_err(s)?;
                let is_listed = listed.iter().any(|l| l == name);
                if present(&place).map_err(s)? && (original_aside || !is_listed) {
                    pause().map_err(s)?;
                    remove_any(&place).map_err(s)?;
                }
            }
        }
        pause().map_err(s)?;
        record.phase = Phase::Restoring;
        write_record(datadir, &record).map_err(s)?;
    }
    let stuck = move_entries(&dir, datadir, &record.originals(), pause).map_err(s)?;
    if let Some(e) = worded(stuck, &dir.display().to_string(), "its place") {
        return Err(s(e));
    }
    if present(&dir).map_err(s)? {
        pause().map_err(s)?;
        std::fs::remove_dir_all(&dir).map_err(s)?;
    }
    pause().map_err(s)?;
    clear_record(datadir).map_err(s)?;
    Ok(Some(record))
}

/// The start after the last step stayed up: [`Phase::Done`] first, then the
/// old chain data in the dated folder goes, the folder too when it holds no
/// signatures, then the record; the outcome says what was kept. Carries on
/// after a crash. `Ok(None)` when no ladder is recorded.
pub fn finish(datadir: &Path, now_unix: u64) -> io::Result<Option<Record>> {
    finish_with(datadir, now_unix, &mut || Ok(()))
}

fn finish_with(datadir: &Path, now_unix: u64, pause: Pause) -> io::Result<Option<Record>> {
    let Some(mut record) = read_record(datadir)? else {
        return Ok(None);
    };
    match record.phase {
        Phase::Done => {}
        Phase::Running => {
            pause()?;
            record.phase = Phase::Done;
            write_record(datadir, &record)?;
        }
        _ => return Err(io::Error::other("the recovery is moving or being undone")),
    }
    let dir = folder(datadir, &record);
    for name in &record.chain {
        pause()?;
        remove_any(&dir.join(name))?;
    }
    if record.signatures.is_empty() {
        pause()?;
        remove_any(&dir)?;
    }
    write_outcome(
        datadir,
        &Outcome::Recovered {
            step: record.step,
            folder: (!record.signatures.is_empty()).then(|| record.aside.clone()),
            at: now_unix,
        },
    );
    pause()?;
    clear_record(datadir)?;
    Ok(Some(record))
}

/// What a start found and did before its launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AtStart {
    Nothing,
    /// A step's start is to be watched (a move cut off was carried on).
    Watch(Record),
    /// A roll-back cut off was finished: the caller puts the settings back
    /// and does not start the node.
    RolledBack(Record),
    /// A finish cut off was completed.
    Finished(Record),
}

/// Carry on whatever a previous run left. Call with the node stopped.
pub fn at_start(datadir: &Path, now_unix: u64) -> Result<AtStart, MoveError> {
    let Some(mut record) = read_record(datadir).map_err(MoveError::Untouched)? else {
        return Ok(AtStart::Nothing);
    };
    match record.phase {
        Phase::Running => Ok(AtStart::Watch(record)),
        Phase::Moving => {
            let dir = folder(datadir, &record);
            carry_on_with(datadir, &mut record, &mut || Ok(())).map_err(|e| stranded(&dir, e))?;
            Ok(AtStart::Watch(record))
        }
        Phase::Undoing | Phase::Restoring => {
            let r = roll_back(datadir)?.unwrap_or(record);
            write_outcome(datadir, &Outcome::RolledBack { at: now_unix });
            Ok(AtStart::RolledBack(r))
        }
        Phase::Done => finish(datadir, now_unix)
            .map_err(MoveError::Untouched)?
            .map(AtStart::Finished)
            .map_or(Ok(AtStart::Nothing), Ok),
    }
}

/// The sentences the window and Copy diagnostics show.
pub mod copy {
    use super::{Outcome, Record, Step};

    pub fn taking(step: Step) -> &'static str {
        match step {
            Step::Signatures => {
                "The node stopped on a known engine error. Setting aside its stored \
                 signatures and starting again."
            }
            Step::ChainData => {
                "The node stopped on a known engine error. Setting aside its chain data and \
                 starting again from the snapshot."
            }
        }
    }

    /// While a step's start is watched.
    pub fn watching(record: &Record) -> String {
        let what = match record.step {
            Step::Signatures => "setting aside its stored signatures",
            Step::ChainData => "starting it again from the snapshot",
        };
        format!(
            "The node stopped on a known engine error, and easyNode got it going again by \
             {what}. It is checking that the node stays up. What was set aside is in {}.",
            record.aside
        )
    }

    pub fn rolled_back(datadir: &str) -> String {
        format!(
            "The node stopped on a known engine error, and starting it again from the snapshot \
             did not help either, so easyNode put its chain data and stored signatures back as \
             they were and did not start it again. Nothing was deleted; everything is in the \
             node folder, {datadir}. Copy diagnostics in Tools gathers what helps."
        )
    }

    pub fn already_tried(datadir: &str) -> String {
        format!(
            "The node stopped on a known engine error again. easyNode already tried setting \
             aside its stored signatures and chain data for it and put everything back, so it \
             did not try again. The node folder is {datadir}. Copy diagnostics in Tools \
             gathers what helps."
        )
    }

    pub fn outcome(o: &Outcome) -> String {
        match o {
            Outcome::Recovered { step, folder, .. } => {
                let how = match step {
                    Step::Signatures => "by setting aside its stored signatures",
                    Step::ChainData => "by starting it again from the snapshot",
                };
                match folder {
                    Some(f) => format!(
                        "The node stopped on a known engine error, and easyNode got it running \
                         again {how}. The old stored signatures are kept in {f} in the node \
                         folder."
                    ),
                    None => format!(
                        "The node stopped on a known engine error, and easyNode got it running \
                         again {how}."
                    ),
                }
            }
            Outcome::RolledBack { .. } => {
                "The node stopped on a known engine error, and easyNode could not get it running \
                 again, so it put everything back as it was."
                    .into()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAIN_DIRS: [&str; 5] = [
        "blocks",
        "chainstate",
        "chainstate_snapshot",
        "indexes",
        "shielded_state",
    ];

    /// A mirror's datadir: chain data, an archive (file, LevelDB folder,
    /// WAL), and things nothing here may touch.
    fn datadir() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        for n in CHAIN_DIRS {
            std::fs::create_dir_all(d.join(n)).unwrap();
            std::fs::write(d.join(n).join("old"), n).unwrap();
        }
        std::fs::write(d.join("snapshot-start.json"), "old start").unwrap();
        std::fs::write(d.join("matmul_attestations.dat"), "old dat").unwrap();
        std::fs::create_dir(d.join("matmul_attestations.dat.db")).unwrap();
        std::fs::write(d.join("matmul_attestations.dat.db/CURRENT"), "old db").unwrap();
        std::fs::write(d.join("matmul_attestations.dat.wal"), "old wal").unwrap();
        std::fs::write(d.join("peers.dat"), "peers").unwrap();
        std::fs::create_dir_all(d.join("wallets/main")).unwrap();
        std::fs::write(d.join("wallets/main/wallet.dat"), "keys").unwrap();
        t
    }

    /// What a start after a step makes: a fresh archive, and with chain
    /// data aside, fresh chain data.
    fn fresh_start(d: &Path, chain: bool) {
        std::fs::write(d.join("matmul_attestations.dat"), "fresh dat").unwrap();
        std::fs::write(d.join("matmul_attestations.dat.wal"), "fresh wal").unwrap();
        std::fs::write(d.join("matmul_attestations.dat.tmp"), "fresh tmp").unwrap();
        if chain {
            for n in CHAIN_DIRS {
                std::fs::create_dir_all(d.join(n)).unwrap();
                std::fs::write(d.join(n).join("new"), n).unwrap();
            }
        }
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    fn untouched_kept(d: &Path) {
        assert_eq!(read(&d.join("peers.dat")), "peers");
        assert_eq!(read(&d.join("wallets/main/wallet.dat")), "keys");
    }

    fn as_before(d: &Path) {
        for n in CHAIN_DIRS {
            assert_eq!(read(&d.join(n).join("old")), n);
            assert!(!d.join(n).join("new").exists(), "{n}");
        }
        assert_eq!(read(&d.join("snapshot-start.json")), "old start");
        assert_eq!(read(&d.join("matmul_attestations.dat")), "old dat");
        assert_eq!(
            read(&d.join("matmul_attestations.dat.db/CURRENT")),
            "old db"
        );
        assert_eq!(read(&d.join("matmul_attestations.dat.wal")), "old wal");
        assert!(!d.join("matmul_attestations.dat.tmp").exists());
        assert!(read_record(d).unwrap().is_none());
        assert!(!d.join(aside_name(100)).exists());
        untouched_kept(d);
    }

    const MIRROR: Facts = Facts {
        trusted_mirror: true,
        own_signing_key: false,
        signature_files_present: true,
    };

    fn before() -> Before {
        Before {
            snapshot_loaded: true,
            first_load_pending: false,
        }
    }

    // ── The ladder's choice ────────────────────────────────────────────────

    #[test]
    fn a_keyless_mirror_sets_its_signatures_aside_first() {
        assert_eq!(next_step(None, None, MIRROR), Next::SetAsideSignatures);
    }

    #[test]
    fn a_node_with_its_own_signing_key_never_loses_its_signatures() {
        let signer = Facts {
            own_signing_key: true,
            ..MIRROR
        };
        assert_eq!(
            next_step(None, None, signer),
            Next::SetAsideChainData { signatures: false }
        );
        assert!(!signatures_may_go(true, true));
    }

    #[test]
    fn a_validating_node_or_one_with_no_archive_goes_straight_to_the_snapshot() {
        let validating = Facts {
            trusted_mirror: false,
            ..MIRROR
        };
        assert_eq!(
            next_step(None, None, validating),
            Next::SetAsideChainData { signatures: true }
        );
        let empty = Facts {
            signature_files_present: false,
            ..MIRROR
        };
        assert_eq!(
            next_step(None, None, empty),
            Next::SetAsideChainData { signatures: true }
        );
    }

    #[test]
    fn the_same_fatal_again_climbs_one_rung_then_rolls_back() {
        let d = datadir();
        let r1 = begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
        assert_eq!(
            next_step(Some(&r1), None, MIRROR),
            Next::SetAsideChainData { signatures: true }
        );
        let r2 = begin(
            d.path(),
            Next::SetAsideChainData { signatures: true },
            before(),
            200,
        )
        .unwrap();
        assert_eq!(next_step(Some(&r2), None, MIRROR), Next::RollBack);
    }

    #[test]
    fn no_second_ladder_after_a_roll_back() {
        let rolled = Outcome::RolledBack { at: 1 };
        assert_eq!(next_step(None, Some(&rolled), MIRROR), Next::AlreadyTried);
        let recovered = Outcome::Recovered {
            step: Step::Signatures,
            folder: None,
            at: 1,
        };
        assert_eq!(
            next_step(None, Some(&recovered), MIRROR),
            Next::SetAsideSignatures,
            "a ladder that worked does not stop a later incident's"
        );
    }

    #[test]
    fn the_mirror_and_the_key_are_read_from_what_the_launch_had() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(launch_is_trusted_mirror(&args(&[
            "-matmulvalidation=trusted"
        ])));
        assert!(!launch_is_trusted_mirror(&args(&[
            "-matmulvalidation=trusted",
            "-matmulvalidation=consensus"
        ])));
        assert!(!launch_is_trusted_mirror(&args(&[])));

        let d = tempfile::tempdir().unwrap();
        let conf = d.path().join("faststart.conf");
        std::fs::write(&conf, "server=1\n").unwrap();
        assert!(!has_own_signing_key(d.path(), &[&conf]));
        std::fs::write(&conf, "server=1\nmatmulattestationsignerkeyfile=k\n").unwrap();
        assert!(has_own_signing_key(d.path(), &[&conf]));
        std::fs::write(&conf, "server=1\n").unwrap();
        std::fs::write(crate::signer::signer_key_path(d.path()), "wif").unwrap();
        assert!(has_own_signing_key(d.path(), &[&conf]));
    }

    // ── Step 1 ─────────────────────────────────────────────────────────────

    #[test]
    fn step_one_moves_only_the_stored_signatures() {
        let d = datadir();
        let r = begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
        assert_eq!((r.step, r.phase), (Step::Signatures, Phase::Running));
        let f = d.path().join(aside_name(100));
        assert_eq!(read(&f.join("matmul_attestations.dat")), "old dat");
        assert_eq!(
            read(&f.join("matmul_attestations.dat.db/CURRENT")),
            "old db"
        );
        assert_eq!(read(&f.join("matmul_attestations.dat.wal")), "old wal");
        for n in SIGNATURE_FILES {
            assert!(!d.path().join(n).exists(), "{n}");
        }
        for n in CHAIN_DIRS {
            assert!(d.path().join(n).join("old").exists(), "{n}");
        }
        untouched_kept(d.path());
        assert_eq!(read_record(d.path()).unwrap(), Some(r));
    }

    #[test]
    fn a_second_step_one_is_refused() {
        let d = datadir();
        begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
        assert!(matches!(
            begin(d.path(), Next::SetAsideSignatures, before(), 101),
            Err(MoveError::Untouched(_))
        ));
    }

    #[test]
    fn step_one_that_stays_up_keeps_the_signatures_folder() {
        let d = datadir();
        begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
        fresh_start(d.path(), false);
        let r = finish(d.path(), 700).unwrap().unwrap();
        assert_eq!(r.step, Step::Signatures);
        assert!(read_record(d.path()).unwrap().is_none());
        let f = d.path().join(aside_name(100));
        assert_eq!(read(&f.join("matmul_attestations.dat")), "old dat");
        assert_eq!(read(&d.path().join("matmul_attestations.dat")), "fresh dat");
        assert_eq!(
            read_outcome(d.path()),
            Some(Outcome::Recovered {
                step: Step::Signatures,
                folder: Some(aside_name(100)),
                at: 700
            })
        );
    }

    // ── Step 2 ─────────────────────────────────────────────────────────────

    #[test]
    fn step_two_after_step_one_drops_the_fresh_archive_and_moves_the_chain() {
        let d = datadir();
        begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
        fresh_start(d.path(), false);
        let r = begin(
            d.path(),
            Next::SetAsideChainData { signatures: true },
            before(),
            200,
        )
        .unwrap();
        assert_eq!((r.step, r.phase), (Step::ChainData, Phase::Running));
        assert_eq!(r.aside, aside_name(100), "one folder per incident");
        let f = d.path().join(aside_name(100));
        for n in CHAIN_DIRS {
            assert!(!d.path().join(n).exists(), "{n}");
            assert_eq!(read(&f.join(n).join("old")), n);
        }
        assert_eq!(read(&f.join("snapshot-start.json")), "old start");
        for n in SIGNATURE_FILES {
            assert!(!d.path().join(n).exists(), "{n}: the fresh archive goes");
        }
        assert_eq!(read(&f.join("matmul_attestations.dat")), "old dat");
        untouched_kept(d.path());
    }

    #[test]
    fn step_two_on_a_signer_leaves_its_archive_alone() {
        let d = datadir();
        let r = begin(
            d.path(),
            Next::SetAsideChainData { signatures: false },
            before(),
            100,
        )
        .unwrap();
        assert!(r.signatures.is_empty());
        assert_eq!(read(&d.path().join("matmul_attestations.dat")), "old dat");
        assert_eq!(
            read(&d.path().join("matmul_attestations.dat.wal")),
            "old wal"
        );
        fresh_start(d.path(), true);
        roll_back(d.path()).unwrap().unwrap();
        // The archive the node wrote meanwhile is its own: never removed.
        assert_eq!(read(&d.path().join("matmul_attestations.dat")), "fresh dat");
        for n in CHAIN_DIRS {
            assert_eq!(read(&d.path().join(n).join("old")), n);
        }
    }

    #[test]
    fn step_two_that_stays_up_removes_the_old_chain_and_keeps_the_signatures() {
        let d = datadir();
        begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
        fresh_start(d.path(), false);
        begin(
            d.path(),
            Next::SetAsideChainData { signatures: true },
            before(),
            200,
        )
        .unwrap();
        fresh_start(d.path(), true);
        finish(d.path(), 900).unwrap().unwrap();
        let f = d.path().join(aside_name(100));
        for n in CHAIN_DIRS {
            assert!(!f.join(n).exists(), "{n}");
            assert!(d.path().join(n).join("new").exists(), "{n}");
        }
        assert_eq!(read(&f.join("matmul_attestations.dat")), "old dat");
        assert!(read_record(d.path()).unwrap().is_none());
    }

    #[test]
    fn a_signers_step_two_that_stays_up_leaves_no_folder() {
        let d = datadir();
        begin(
            d.path(),
            Next::SetAsideChainData { signatures: false },
            before(),
            100,
        )
        .unwrap();
        fresh_start(d.path(), true);
        finish(d.path(), 900).unwrap();
        assert!(!d.path().join(aside_name(100)).exists());
        assert_eq!(
            read_outcome(d.path()),
            Some(Outcome::Recovered {
                step: Step::ChainData,
                folder: None,
                at: 900
            })
        );
    }

    // ── Step 3 ─────────────────────────────────────────────────────────────

    #[test]
    fn step_three_puts_everything_back_as_it_was() {
        let d = datadir();
        begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
        fresh_start(d.path(), false);
        begin(
            d.path(),
            Next::SetAsideChainData { signatures: true },
            Before::default(),
            200,
        )
        .unwrap();
        fresh_start(d.path(), true);
        let r = roll_back(d.path()).unwrap().unwrap();
        assert_eq!(r.before(), Before::default());
        as_before(d.path());
    }

    #[test]
    fn a_roll_back_after_step_one_alone_puts_the_signatures_back() {
        let d = datadir();
        begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
        fresh_start(d.path(), false);
        roll_back(d.path()).unwrap().unwrap();
        as_before(d.path());
        assert!(roll_back(d.path()).unwrap().is_none());
    }

    // ── Cut off mid-way ────────────────────────────────────────────────────

    /// Every point a step can be cut off at: the next start carries the move
    /// on, and a roll-back from there still gets everything back.
    #[test]
    fn a_step_cut_off_anywhere_is_carried_on_at_the_next_start() {
        for stop_at in 1..40 {
            let d = datadir();
            begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
            fresh_start(d.path(), false);
            let mut calls = 0;
            let cut = begin_with(
                d.path(),
                Next::SetAsideChainData { signatures: true },
                before(),
                200,
                &mut || {
                    calls += 1;
                    if calls == stop_at {
                        Err(io::Error::other("crash"))
                    } else {
                        Ok(())
                    }
                },
                false,
            );
            if cut.is_ok() {
                break;
            }
            // A crash leaves no time for a clean-up (`clean_up` false):
            // the next start finds the record as the crash left it.
            match at_start(d.path(), 300).unwrap() {
                AtStart::Watch(r) => {
                    assert_eq!(r.phase, Phase::Running, "stop {stop_at}");
                    for n in CHAIN_DIRS {
                        // Cut off before the step was recorded, step 1
                        // stands; after, step 2's move is carried through.
                        let moved = !d.path().join(n).exists();
                        assert_eq!(moved, r.step == Step::ChainData, "stop {stop_at}: {n}");
                    }
                    roll_back(d.path()).unwrap();
                    as_before(d.path());
                }
                other => panic!("stop {stop_at}: {other:?}"),
            }
        }
    }

    /// A step whose move fails (not a crash) puts the whole ladder back at
    /// once, step 1 included, and says nothing moved.
    #[test]
    fn a_step_that_cannot_move_puts_everything_back() {
        let d = datadir();
        begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
        fresh_start(d.path(), false);
        let mut calls = 0;
        let failed = begin_with(
            d.path(),
            Next::SetAsideChainData { signatures: true },
            before(),
            200,
            &mut || {
                calls += 1;
                if calls == 6 {
                    Err(io::Error::other("disk"))
                } else {
                    Ok(())
                }
            },
            true,
        );
        assert!(matches!(failed, Err(MoveError::Untouched(_))));
        as_before(d.path());
    }

    #[test]
    fn a_roll_back_cut_off_anywhere_is_finished_at_the_next_start() {
        for stop_at in 1..40 {
            let d = datadir();
            begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
            fresh_start(d.path(), false);
            begin(
                d.path(),
                Next::SetAsideChainData { signatures: true },
                before(),
                200,
            )
            .unwrap();
            fresh_start(d.path(), true);
            let mut calls = 0;
            let cut = roll_back_with(d.path(), &mut || {
                calls += 1;
                if calls == stop_at {
                    Err(io::Error::other("crash"))
                } else {
                    Ok(())
                }
            });
            if cut.is_ok() {
                as_before(d.path());
                break;
            }
            match at_start(d.path(), 300).unwrap() {
                AtStart::RolledBack(_) => {}
                AtStart::Watch(_) => {
                    // Cut off before the undo was marked: nothing moved yet.
                    roll_back(d.path()).unwrap();
                }
                other => panic!("stop {stop_at}: {other:?}"),
            }
            as_before(d.path());
        }
    }

    #[test]
    fn a_finish_cut_off_is_completed_at_the_next_start() {
        for stop_at in 1..10 {
            let d = datadir();
            begin(
                d.path(),
                Next::SetAsideChainData { signatures: true },
                before(),
                100,
            )
            .unwrap();
            fresh_start(d.path(), true);
            let mut calls = 0;
            let cut = finish_with(d.path(), 500, &mut || {
                calls += 1;
                if calls == stop_at {
                    Err(io::Error::other("crash"))
                } else {
                    Ok(())
                }
            });
            if cut.is_ok() {
                break;
            }
            match at_start(d.path(), 600).unwrap() {
                AtStart::Finished(_) | AtStart::Watch(_) | AtStart::Nothing => {}
                other => panic!("stop {stop_at}: {other:?}"),
            }
            let _ = finish(d.path(), 600).unwrap();
            assert!(read_record(d.path()).unwrap().is_none());
            let f = d.path().join(aside_name(100));
            for n in CHAIN_DIRS {
                assert!(!f.join(n).exists(), "stop {stop_at}: {n}");
                assert!(d.path().join(n).join("new").exists());
            }
            assert_eq!(read(&f.join("matmul_attestations.dat")), "old dat");
        }
    }

    #[test]
    fn a_record_naming_anything_else_is_refused() {
        let d = datadir();
        let r = begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
        for bad in [
            Record {
                aside: "..".into(),
                ..r.clone()
            },
            Record {
                signatures: vec!["wallets".into()],
                ..r.clone()
            },
            Record {
                chain: vec!["peers.dat".into()],
                ..r.clone()
            },
        ] {
            write_record(d.path(), &bad).unwrap();
            assert!(read_record(d.path()).is_err());
            assert!(roll_back(d.path()).is_err());
        }
        untouched_kept(d.path());
    }

    #[test]
    fn the_sentences_follow_the_copy_rules() {
        let d = datadir();
        let r = begin(d.path(), Next::SetAsideSignatures, before(), 100).unwrap();
        let all = [
            copy::taking(Step::Signatures).to_string(),
            copy::taking(Step::ChainData).to_string(),
            copy::watching(&r),
            copy::rolled_back("/home/me/.easybtx"),
            copy::already_tried("/home/me/.easybtx"),
            copy::outcome(&Outcome::RolledBack { at: 1 }),
            copy::outcome(&Outcome::Recovered {
                step: Step::Signatures,
                folder: Some(r.aside.clone()),
                at: 1,
            }),
        ];
        for s in &all {
            assert!(!s.contains('\u{2014}') && !s.contains('\u{2013}'), "{s}");
            for word in ["guarantee", "always works", "operator"] {
                assert!(!s.to_lowercase().contains(word), "{s}");
            }
        }
        assert_eq!(
            copy::taking(Step::Signatures),
            "The node stopped on a known engine error. Setting aside its stored signatures \
             and starting again."
        );
    }
}
