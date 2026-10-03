//! The read-block recovery, driven (the ladder on disk is
//! `btx_core::read_block_recovery`).
//!
//! * [`after_fatal`]: the launch loop saw this attempt's btxd die on the
//!   "Failed to read block" fatal. It takes the next rung and answers
//!   [`STEP_TAKEN`], and `commands::start_node_held` starts again from the
//!   top, so a step's start is an ordinary start in every way (a fresh
//!   chain gets the header bootstrap and the first load a new install
//!   gets). On the last rung it rolls everything back and answers the
//!   sentence that stops the start.
//! * [`after_failed_launch`]: a launch after step 2 that failed some other
//!   way (kept exiting, never opened RPC) is step 2 not working either: the
//!   same roll-back.
//! * [`before_start`]: a move, a roll-back or a finish a previous run was
//!   cut off in is carried on before anything launches.
//! * [`after_start`]: a start that reached RPC with a step recorded is
//!   watched for `WATCH_SECS`, then the step is finished; and any start
//!   that reached RPC ends a rolled-back incident's "already tried".
//!
//! A btxd that dies while running is not restarted by this app: the status
//! refresher says "The node stopped responding" after about a minute and
//! the next Start (the button, the tray, or the app's own start when it
//! opens) goes through this same path, where the restart fatal shows within
//! a second.
//!
//! Every move holds `crate::fast_forward::with_disk` (the one serialisation
//! point for moves in the data folder) and the engine's own lock on the
//! folder, so no btxd, whichever app starts it, runs on a half-moved one.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use btx_core::fast_forward::{Before, MoveError};
use btx_core::read_block_recovery::{
    self as rbr, copy, AtStart, Facts, Next, Outcome, Phase, Record, Step,
};
use tauri::{AppHandle, Manager};

use crate::commands::{set_phase, setup_log, snapshot_spec};
use crate::fast_forward::{put_settings, run_settings, with_disk};
use crate::state::{node_datadir, AppState, NodePhase};

/// What [`after_fatal`] answers when it took a step: the start is to begin
/// again from the top. Never shown; `commands::start_node_held` consumes it.
pub(crate) const STEP_TAKEN: &str = "read-block recovery: step taken, starting again";

/// The fatal while a Fast-forward run is recorded: that run owns the chain
/// data until it ends, so nothing here moves.
const FAST_FORWARD_FIRST: &str = "The node stopped on a known engine error while a \
     Fast-forward has not finished, so easyNode did not set anything aside. Copy diagnostics \
     in Tools gathers what helps.";

/// How long the success note stays on the status screen after a ladder
/// worked.
const NOTE_SECS: u64 = 24 * 60 * 60;

/// One watch at a time.
static WATCHING: AtomicBool = AtomicBool::new(false);

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn log(datadir: &Path, msg: &str) {
    eprintln!("[read-block] {msg}");
    setup_log(datadir, &format!("read-block recovery: {msg}"));
}

/// Pure: the facts of the launch that died. `args` are the arguments it
/// was started with, so the mirror question is answered by what ran.
fn facts(datadir: &Path, args: &[String], confs: &[&Path]) -> Facts {
    Facts {
        trusted_mirror: rbr::launch_is_trusted_mirror(args),
        own_signing_key: rbr::has_own_signing_key(datadir, confs),
        signature_files_present: rbr::signature_files_present(datadir),
    }
}

/// Why [`on_disk_locked`] did not run its work.
#[derive(Debug)]
enum DiskErr {
    /// Another process holds the folder's lock: a node runs on it.
    Held,
    Other(String),
}

impl std::fmt::Display for DiskErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiskErr::Held => write!(f, "a node holds the data folder's lock"),
            DiskErr::Other(why) => write!(f, "{why}"),
        }
    }
}

/// Run `f` on a blocking thread under `with_disk`, holding the engine's
/// lock on the folder.
async fn on_disk_locked<T: Send + 'static>(
    datadir: &Path,
    f: impl FnOnce(&Path) -> T + Send + 'static,
) -> Result<T, DiskErr> {
    use btx_core::fsx::{EngineLock, EngineLockError};
    let dd = datadir.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        with_disk(|| {
            let _engine = EngineLock::take(&dd).map_err(|e| match e {
                EngineLockError::Held => DiskErr::Held,
                EngineLockError::Io(e) => DiskErr::Other(format!("the folder's lock: {e}")),
            })?;
            Ok(f(&dd))
        })
    })
    .await
    .map_err(|e| DiskErr::Other(format!("the task stopped: {e}")))?
}

/// Another node holds the folder while something here must still be
/// carried on: nothing moved, and the node is not started.
const LOCK_HELD: &str = "Another node is using the node folder, so easyNode could not finish \
     putting things back after a known engine error and did not start the node. Stop the \
     other node (the easyBTX miner's, a second copy of this app, or a btxd started by hand) \
     and press Start.";

/// Pure: the sentence when carrying on could not begin.
fn not_carried_on(why: &DiskErr, shown: &str) -> String {
    match why {
        DiskErr::Held => LOCK_HELD.to_string(),
        DiskErr::Other(_) => stranded_sentence(shown),
    }
}

/// The sentence on screen while the app starts a node that died on the
/// fatal again by itself.
pub(crate) const AUTO_START: &str = "The node stopped on a known engine error, so easyNode is \
     starting it again on its own.";

/// When this app run last started the node by itself after the fatal; 0
/// for never. In memory: the bound is per incident, and an incident does
/// not outlive the app run that saw it (the next opening starts the node
/// anyway).
static LAST_AUTO_START: AtomicU64 = AtomicU64::new(0);

/// What the status refresher knows when the node stopped answering.
#[derive(Debug, Clone, Copy)]
struct AutoStart {
    /// The child this app spawned has exited.
    child_gone: bool,
    /// This app spawned it (not attached to another app's node).
    ours: bool,
    /// Its own log (rotated per run) shows the fatal.
    fatal_in_log: bool,
    /// The person pressed Stop (the run's generation moved) or the app quits.
    stopped_or_quitting: bool,
    fast_forward_recorded: bool,
    step_recorded: bool,
    /// The last ladder rolled back: the next Start says so, no automatic one.
    rolled_back: bool,
    last_auto_start: u64,
    now: u64,
}

/// Pure: start the node again by itself? Only for the fatal, on our own
/// node, with nothing recorded that owns the chain data, and once per
/// incident: a death within `WATCH_SECS` of the last automatic start is the
/// same incident.
fn auto_start_wanted(a: &AutoStart) -> bool {
    a.child_gone
        && a.ours
        && a.fatal_in_log
        && !a.stopped_or_quitting
        && !a.fast_forward_recorded
        && !a.step_recorded
        && !a.rolled_back
        && (a.last_auto_start == 0 || a.now.saturating_sub(a.last_auto_start) >= rbr::WATCH_SECS)
}

/// The status refresher found the node silent: should it start it again
/// by itself? Records the start when it says yes.
pub(crate) fn take_auto_start(
    datadir: &Path,
    child_gone: bool,
    ours: bool,
    stopped_or_quitting: bool,
) -> bool {
    let now = now();
    let facts = AutoStart {
        child_gone,
        ours,
        fatal_in_log: child_gone
            && btx_core::node::log_shows_read_block_fatal(&btx_core::node::node_log_tail(
                datadir,
                64 * 1024,
            )),
        stopped_or_quitting,
        fast_forward_recorded: !matches!(btx_core::fast_forward::read_record(datadir), Ok(None)),
        step_recorded: !matches!(rbr::read_record(datadir), Ok(None)),
        rolled_back: matches!(rbr::read_outcome(datadir), Some(Outcome::RolledBack { .. })),
        last_auto_start: LAST_AUTO_START.load(Ordering::SeqCst),
        now,
    };
    let wanted = auto_start_wanted(&facts);
    if wanted {
        LAST_AUTO_START.store(now, Ordering::SeqCst);
        log(
            datadir,
            "the node died on the fatal while running; starting it again on its own",
        );
    } else if facts.fatal_in_log {
        log(
            datadir,
            &format!("the node died on the fatal; not starting it by itself: {facts:?}"),
        );
    }
    wanted
}

/// Pure: the note after an automatic start, for a day.
fn auto_note(last_auto_start: u64, now_unix: u64) -> Option<String> {
    (last_auto_start != 0 && now_unix.saturating_sub(last_auto_start) < NOTE_SECS).then(|| {
        "The node stopped on a known engine error and easyNode started it again on its own."
            .to_string()
    })
}

/// The roll-back on disk, settings and the attempt's launch markers with
/// it. `Ok` when everything is back; `Err(folder)` when not all of it is.
fn roll_back_on_disk(datadir: &Path) -> Result<(), String> {
    match rbr::roll_back(datadir) {
        Ok(record) => {
            if let Some(r) = record {
                undo_settings(datadir, &r);
            }
            rbr::write_outcome(datadir, &Outcome::RolledBack { at: now() });
            Ok(())
        }
        Err(MoveError::Stranded { folder, error }) => {
            log(
                datadir,
                &format!("putting everything back stopped: {error}"),
            );
            Err(folder.display().to_string())
        }
        Err(MoveError::Untouched(error)) => {
            log(
                datadir,
                &format!("putting everything back did not begin: {error}"),
            );
            Err(datadir.display().to_string())
        }
    }
}

/// After a roll-back of a ladder whose step 2 ran: the settings from before
/// it, and no marker of the fresh chain's launches (a header bootstrap or a
/// mirror launch on the old chain would be wrong).
fn undo_settings(datadir: &Path, record: &Record) {
    if record.step == Step::ChainData {
        put_settings(datadir, record.before());
        btx_core::node::end_mirror_load(datadir);
        btx_core::node::end_header_bootstrap(datadir);
    }
}

/// The sentence for a roll-back that could not put everything back.
fn stranded_sentence(folder: &str) -> String {
    format!(
        "The node stopped on a known engine error, and easyNode could not get it running \
         again or put everything back as it was, so it did not start it. What is not back in \
         place is in {folder}. Copy diagnostics in Tools gathers what helps."
    )
}

async fn roll_back_and_say(datadir: &Path) -> String {
    let shown = datadir.display().to_string();
    match on_disk_locked(datadir, roll_back_on_disk).await {
        Ok(Ok(())) => {
            log(
                datadir,
                "step 3: everything is back as it was; not starting",
            );
            copy::rolled_back(&shown)
        }
        Ok(Err(folder)) => stranded_sentence(&folder),
        Err(why) => {
            log(datadir, &format!("could not put everything back: {why}"));
            not_carried_on(&why, &shown)
        }
    }
}

/// The launch loop saw the fatal. `args` are the dead launch's, `confs`
/// the files a signing key could be named in. Answers [`STEP_TAKEN`] after
/// a step, or the sentence that ends the start.
pub(crate) async fn after_fatal(
    app: &AppHandle,
    state: &AppState,
    datadir: &Path,
    args: &[String],
    confs: &[&Path],
) -> String {
    if !matches!(btx_core::fast_forward::read_record(datadir), Ok(None)) {
        // Fast-forward owns the chain data until its run ends.
        log(datadir, "a Fast-forward run is recorded; not taking a step");
        return FAST_FORWARD_FIRST.to_string();
    }
    let record = match rbr::read_record(datadir) {
        Ok(r) => r,
        Err(e) => {
            log(
                datadir,
                &format!("the record cannot be read ({e}); not taking a step"),
            );
            return stranded_sentence(&datadir.display().to_string());
        }
    };
    let facts = facts(datadir, args, confs);
    let next = rbr::next_step(record.as_ref(), rbr::read_outcome(datadir).as_ref(), facts);
    log(
        datadir,
        &format!("the engine stopped on the fatal; {facts:?}; next: {next:?}"),
    );
    let step = match next {
        Next::AlreadyTried => return copy::already_tried(&datadir.display().to_string()),
        Next::RollBack => return roll_back_and_say(datadir).await,
        Next::SetAsideSignatures => Step::Signatures,
        Next::SetAsideChainData { .. } => Step::ChainData,
    };
    set_phase(
        app,
        state,
        NodePhase::Warming {
            message: copy::taking(step).to_string(),
        },
    )
    .await;
    if step == Step::ChainData {
        // As a fresh install: the pinned snapshot in place before the
        // start. Already there and correct, nothing is fetched. A failure
        // is not the step's: the start path then syncs without it, or a
        // mirror loads a signed one.
        if let Err(e) =
            btx_core::snapshot::download_snapshot(&snapshot_spec(), datadir, &|_| {}).await
        {
            log(
                datadir,
                &format!("the pinned snapshot could not be fetched ({e}); going on"),
            );
        }
    }
    let taken = on_disk_locked(datadir, move |d| take_step(d, next)).await;
    match taken {
        Ok(Ok(record)) => {
            log(
                datadir,
                &format!(
                    "step {:?} taken; set aside in {} (signatures {:?}, chain {:?})",
                    record.step, record.aside, record.signatures, record.chain
                ),
            );
            STEP_TAKEN.to_string()
        }
        Ok(Err(MoveError::Stranded { folder, error })) => {
            log(datadir, &format!("the step stopped: {error}"));
            stranded_sentence(&folder.display().to_string())
        }
        Ok(Err(MoveError::Untouched(error))) => {
            log(
                datadir,
                &format!("the step did not happen, nothing moved: {error}"),
            );
            rbr::write_outcome(datadir, &Outcome::RolledBack { at: now() });
            copy::rolled_back(&datadir.display().to_string())
        }
        Err(DiskErr::Held) => {
            log(
                datadir,
                "the step did not happen: another node holds the folder",
            );
            "Another node is using the node folder, so easyNode did not set anything aside \
             after a known engine error. Stop the other node and press Start."
                .to_string()
        }
        Err(why) => {
            log(datadir, &format!("the step did not happen: {why}"));
            copy::rolled_back(&datadir.display().to_string())
        }
    }
}

/// A rung on disk. For step 2 the app's "snapshot loaded" and "first load
/// to come" are reset before anything moves, as Fast-forward does, so the
/// next start loads the snapshot; a step that did not happen puts them
/// back (the record keeps the old values for a roll-back).
fn take_step(datadir: &Path, next: Next) -> Result<Record, MoveError> {
    let before = run_settings(datadir);
    let chain = matches!(next, Next::SetAsideChainData { .. });
    if chain {
        put_settings(datadir, Before::default());
    }
    let taken = rbr::begin(datadir, next, before, now());
    if chain && matches!(taken, Err(MoveError::Untouched(_))) {
        put_settings(datadir, before);
    }
    taken
}

/// A launch that failed other than on the fatal. After step 2 that is the
/// fresh start not working: roll back, and say so with `error`. Otherwise
/// `error` as it was.
pub(crate) async fn after_failed_launch(datadir: &Path, error: String) -> String {
    match rbr::read_record(datadir) {
        Ok(Some(r)) if r.step == Step::ChainData && r.phase == Phase::Running => {
            log(
                datadir,
                &format!("the start from the snapshot failed: {error}"),
            );
            format!("{} It said: {error}", roll_back_and_say(datadir).await)
        }
        _ => error,
    }
}

/// Before anything launches: carry on what a previous run was cut off in.
/// `Err` is a sentence, and the node is not started.
pub(crate) async fn before_start(datadir: &Path) -> Result<(), String> {
    match rbr::read_record(datadir) {
        Ok(None) => return Ok(()),
        Ok(Some(r)) if r.phase == Phase::Running => return Ok(()),
        Ok(Some(_)) => {}
        Err(e) => {
            log(datadir, &format!("the record cannot be read ({e})"));
            return Err(stranded_sentence(&datadir.display().to_string()));
        }
    }
    let shown = datadir.display().to_string();
    let found = on_disk_locked(datadir, |d| {
        let found = rbr::at_start(d, now());
        if let Ok(AtStart::RolledBack(r)) = &found {
            undo_settings(d, r);
        }
        found
    })
    .await;
    match found {
        Ok(Ok(AtStart::RolledBack(_))) => {
            log(
                datadir,
                "finished putting everything back after a stop; not starting",
            );
            Err(copy::rolled_back(&shown))
        }
        Ok(Ok(found)) => {
            log(datadir, &format!("carried on after a stop: {found:?}"));
            Ok(())
        }
        Ok(Err(MoveError::Stranded { folder, error })) => {
            log(datadir, &format!("carrying on stopped: {error}"));
            Err(stranded_sentence(&folder.display().to_string()))
        }
        Ok(Err(MoveError::Untouched(error))) => {
            log(datadir, &format!("carrying on did not begin: {error}"));
            Err(stranded_sentence(&shown))
        }
        Err(why) => {
            log(datadir, &format!("carrying on did not begin: {why}"));
            Err(not_carried_on(&why, &shown))
        }
    }
}

/// A start reached RPC. A rolled-back incident is over; a recorded step's
/// start is watched, and finished once it stayed up.
pub(crate) fn after_start(app: &AppHandle, state: &AppState) {
    let datadir = node_datadir();
    if matches!(
        rbr::read_outcome(&datadir),
        Some(Outcome::RolledBack { .. })
    ) {
        rbr::clear_outcome(&datadir);
    }
    if !matches!(rbr::read_record(&datadir), Ok(Some(r)) if r.phase == Phase::Running) {
        return;
    }
    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let gen = state.refresher_gen.load(Ordering::SeqCst);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                WATCHING.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done;
        let state = app.state::<AppState>();
        let mut waited = 0;
        while waited < rbr::WATCH_SECS {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            waited += 30;
            if state.refresher_gen.load(Ordering::SeqCst) != gen {
                // Stopped or restarted: the next start watches again.
                return;
            }
        }
        let rpc = state.rpc.lock().await.clone();
        let up = match rpc {
            Some(rpc) => btx_core::node_api::get_blockchain_info(&rpc).await.is_ok(),
            None => false,
        };
        if !up || state.refresher_gen.load(Ordering::SeqCst) != gen {
            return;
        }
        let datadir = node_datadir();
        let dd = datadir.clone();
        let finished =
            tauri::async_runtime::spawn_blocking(move || with_disk(|| rbr::finish(&dd, now())))
                .await;
        match finished {
            Ok(Ok(Some(r))) => log(
                &datadir,
                &format!("the node stayed up after step {:?}; done", r.step),
            ),
            Ok(Ok(None)) => {}
            Ok(Err(e)) => log(
                &datadir,
                &format!("finishing stopped (the next start carries on): {e}"),
            ),
            Err(e) => log(&datadir, &format!("finishing stopped: {e}")),
        }
    });
}

/// Pure: the status screen's note, if any: a step being watched, or a
/// ladder that worked in the last day.
fn note_for(record: Option<&Record>, outcome: Option<&Outcome>, now_unix: u64) -> Option<String> {
    if let Some(r) = record.filter(|r| r.phase == Phase::Running) {
        return Some(copy::watching(r));
    }
    match outcome {
        Some(o @ Outcome::Recovered { at, .. }) if now_unix.saturating_sub(*at) < NOTE_SECS => {
            Some(copy::outcome(o))
        }
        _ => None,
    }
}

/// [`note_for`], read from the data folder.
pub(crate) fn status_note(datadir: &Path) -> Option<String> {
    note_for(
        rbr::read_record(datadir).ok().flatten().as_ref(),
        rbr::read_outcome(datadir).as_ref(),
        now(),
    )
    .or_else(|| auto_note(LAST_AUTO_START.load(Ordering::SeqCst), now()))
}

/// Pure: Copy diagnostics' lines.
fn lines_for(
    record: Option<&Record>,
    outcome: Option<&Outcome>,
    last_auto_start: u64,
) -> Vec<String> {
    let mut out = Vec::new();
    if last_auto_start != 0 {
        out.push(format!(
            "started again on its own at {} after the engine's \"Failed to read block\" fatal",
            crate::update_log::rfc3339_utc(last_auto_start)
        ));
    }
    if let Some(r) = record {
        out.push(format!(
            "step {:?}, phase {:?}, folder {}, set aside: signatures {:?}, chain data {:?}",
            r.step, r.phase, r.aside, r.signatures, r.chain
        ));
    }
    if let Some(o) = outcome {
        out.push(format!("last: {}", copy::outcome(o)));
    }
    out
}

/// [`lines_for`], read from the data folder.
pub(crate) fn diagnostics_lines(datadir: &Path) -> Vec<String> {
    let record = match rbr::read_record(datadir) {
        Ok(r) => r,
        Err(e) => return vec![format!("the record cannot be read: {e}")],
    };
    lines_for(
        record.as_ref(),
        rbr::read_outcome(datadir).as_ref(),
        LAST_AUTO_START.load(Ordering::SeqCst),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(step: Step, phase: Phase) -> Record {
        Record {
            step,
            phase,
            aside: rbr::aside_name(100),
            signatures: vec!["matmul_attestations.dat".into()],
            chain: Vec::new(),
            snapshot_loaded_before: true,
            first_load_pending_before: false,
            started_at: 100,
        }
    }

    #[test]
    fn the_facts_come_from_the_launch_and_the_folder() {
        let d = tempfile::tempdir().unwrap();
        let conf = d.path().join("faststart.conf");
        std::fs::write(&conf, "server=1\n").unwrap();
        let mirror = vec!["-matmulvalidation=trusted".to_string()];
        let f = facts(d.path(), &mirror, &[&conf]);
        assert!(f.trusted_mirror && !f.own_signing_key && !f.signature_files_present);
        std::fs::write(d.path().join("matmul_attestations.dat"), "x").unwrap();
        std::fs::write(
            &conf,
            "matmulattestationsignerkeyfile=attestation-signer.key\n",
        )
        .unwrap();
        let f = facts(d.path(), &[], &[&conf]);
        assert!(!f.trusted_mirror && f.own_signing_key && f.signature_files_present);
    }

    #[test]
    fn a_step_on_disk_resets_the_load_settings_only_for_the_chain_data() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("matmul_attestations.dat"), "old").unwrap();
        std::fs::create_dir(d.path().join("blocks")).unwrap();
        put_settings(
            d.path(),
            Before {
                snapshot_loaded: true,
                first_load_pending: false,
            },
        );
        take_step(d.path(), Next::SetAsideSignatures).unwrap();
        assert!(run_settings(d.path()).snapshot_loaded);
        let r = take_step(d.path(), Next::SetAsideChainData { signatures: true }).unwrap();
        assert!(r.snapshot_loaded_before);
        assert_eq!(run_settings(d.path()), Before::default());
        assert!(!d.path().join("blocks").exists());
        roll_back_on_disk(d.path()).unwrap();
        assert!(
            run_settings(d.path()).snapshot_loaded,
            "the roll-back puts it back"
        );
        assert!(d.path().join("blocks").exists());
        assert_eq!(
            std::fs::read_to_string(d.path().join("matmul_attestations.dat")).unwrap(),
            "old"
        );
        assert!(matches!(
            rbr::read_outcome(d.path()),
            Some(Outcome::RolledBack { .. })
        ));
    }

    #[test]
    fn the_note_shows_a_watched_step_and_a_recent_success_only() {
        let watched = record(Step::Signatures, Phase::Running);
        assert!(note_for(Some(&watched), None, 200)
            .unwrap()
            .contains("checking that the node stays up"));
        assert!(note_for(Some(&record(Step::Signatures, Phase::Moving)), None, 200).is_none());
        let ok = Outcome::Recovered {
            step: Step::ChainData,
            folder: Some(rbr::aside_name(100)),
            at: 1_000,
        };
        assert!(note_for(None, Some(&ok), 1_000 + 60)
            .unwrap()
            .contains(&rbr::aside_name(100)));
        assert!(note_for(None, Some(&ok), 1_000 + NOTE_SECS).is_none());
        assert!(note_for(None, Some(&Outcome::RolledBack { at: 1 }), 2).is_none());
    }

    #[test]
    fn diagnostics_name_the_step_the_folder_and_the_last_outcome() {
        let r = record(Step::ChainData, Phase::Running);
        let lines = lines_for(Some(&r), Some(&Outcome::RolledBack { at: 1 }), 0);
        assert!(
            lines[0].contains("ChainData") && lines[0].contains(&r.aside),
            "{lines:?}"
        );
        assert!(lines[1].starts_with("last: "), "{lines:?}");
        assert!(lines_for(None, None, 0).is_empty());
    }

    /// Final review I2: the common case, the refresher's own start and a
    /// node that stayed up, leaves no record and no outcome; the report
    /// still says when it happened and why.
    #[test]
    fn diagnostics_name_an_automatic_start() {
        let lines = lines_for(None, None, 1_791_000_000);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("Failed to read block"), "{lines:?}");
        assert!(lines[0].contains("on its own"), "{lines:?}");
        assert!(lines[0].contains("2026-"), "{lines:?}");
    }

    fn due() -> AutoStart {
        AutoStart {
            child_gone: true,
            ours: true,
            fatal_in_log: true,
            stopped_or_quitting: false,
            fast_forward_recorded: false,
            step_recorded: false,
            rolled_back: false,
            last_auto_start: 0,
            now: 10_000,
        }
    }

    #[test]
    fn a_node_that_died_on_the_fatal_is_started_again_by_itself() {
        assert!(auto_start_wanted(&due()));
    }

    #[test]
    fn any_other_death_or_state_keeps_todays_behaviour() {
        for (what, a) in [
            (
                "alive",
                AutoStart {
                    child_gone: false,
                    ..due()
                },
            ),
            (
                "another app's node",
                AutoStart {
                    ours: false,
                    ..due()
                },
            ),
            (
                "another cause",
                AutoStart {
                    fatal_in_log: false,
                    ..due()
                },
            ),
            (
                "Stop or Quit",
                AutoStart {
                    stopped_or_quitting: true,
                    ..due()
                },
            ),
            (
                "Fast-forward",
                AutoStart {
                    fast_forward_recorded: true,
                    ..due()
                },
            ),
            (
                "a step recorded",
                AutoStart {
                    step_recorded: true,
                    ..due()
                },
            ),
            (
                "rolled back",
                AutoStart {
                    rolled_back: true,
                    ..due()
                },
            ),
        ] {
            assert!(!auto_start_wanted(&a), "{what}");
        }
    }

    /// One automatic start per incident: a death within the watch of the
    /// last one is the same incident; one after it is a new one.
    #[test]
    fn one_automatic_start_per_incident() {
        let just = AutoStart {
            last_auto_start: 10_000 - 60,
            ..due()
        };
        assert!(!auto_start_wanted(&just));
        let long_ago = AutoStart {
            last_auto_start: 10_000 - rbr::WATCH_SECS,
            ..due()
        };
        assert!(auto_start_wanted(&long_ago));
    }

    #[test]
    fn the_note_says_it_started_again_on_its_own() {
        assert!(auto_note(5_000, 5_100).unwrap().contains("on its own"));
        assert!(auto_note(0, 5_100).is_none());
        assert!(auto_note(5_000, 5_000 + NOTE_SECS).is_none());
    }

    /// Q3: an interrupted roll-back that finds another node holding the
    /// folder's lock says so, not "could not put everything back".
    #[test]
    fn a_held_lock_says_another_node_is_using_the_folder() {
        assert_eq!(not_carried_on(&DiskErr::Held, "/d"), LOCK_HELD);
        assert!(not_carried_on(&DiskErr::Other("x".into()), "/d").contains("/d"));
    }

    #[test]
    fn the_sentences_here_follow_the_copy_rules() {
        for s in [
            stranded_sentence("/x/read-block-recovery-1"),
            STEP_TAKEN.to_string(),
            FAST_FORWARD_FIRST.to_string(),
            AUTO_START.to_string(),
            auto_note(1, 2).unwrap(),
            LOCK_HELD.to_string(),
        ] {
            assert!(!s.contains('\u{2014}') && !s.contains('\u{2013}'), "{s}");
            assert!(!s.to_lowercase().contains("guarantee"), "{s}");
        }
    }
}
