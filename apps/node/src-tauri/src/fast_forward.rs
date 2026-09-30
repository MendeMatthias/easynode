//! Fast-forward, driven: check and download the confirmed snapshot while the
//! node runs, stop it, set the chain data aside, start it again and let the
//! start path load the snapshot (the one loading path every node uses), then
//! keep the new chain or put the old one back.
//! docs/decisions/2026-09-29-every-node-starts-near-the-tip.md, section 10.
//!
//! What is on disk is `btx_core::fast_forward`'s: its record says which step
//! a run is at, and every step can be cut off by a crash and carried on from
//! it. This module decides when each step runs:
//!
//! * [`spawn_run`]: section 10's steps 1 to 3, then the watch.
//! * the watch: every five seconds, `ff::judge`. Done keeps the new chain
//!   once its base block is checked ([`confirmed_start`]), then `ff::finish`;
//!   anything else rolls it back ([`undo`]).
//! * [`before_start`]: at every start, the gate ([`start_gate`]); with no
//!   driver at work, the sweep, then the run a previous start left, by its
//!   phase. The node is launched only once any undo it needs returned `Ok`,
//!   and never while this app's driver moves chain data.
//! * [`resume_if_needed`]: a run the app was closed on is watched again.
//!
//! A roll-back is decided for good when its reason is written: a run clears
//! the outcome before it sets anything aside, so a Running record beside a
//! roll-back's outcome is always one to roll back ([`verdict`]).
//!
//! One serialisation point, [`with_disk`], for every step that moves or
//! removes chain data or writes the run's files: set aside, restore, finish,
//! the outcome, the sweep, and Remove node data. It is only ever taken inside
//! `spawn_blocking`, never across an await. And while chain data moves the
//! driver holds the start path's own guard (`AppState::start_in_flight`), so
//! no start launches btxd on a half-moved datadir.
//!
//! Every sentence here can reach the window, so none carries an error's own
//! text: that goes to the log.
//!
//! Another app sharing the datadir (the easyBTX miner) takes neither the lock
//! nor the start guard, so every move of chain data (setting it aside, and a
//! restore's removals and put-back) also holds the engine's own lock on the
//! folder, `<datadir>/.lock` ([`engine_lock`]): a btxd any app launches
//! meanwhile refuses to start, and a lock that cannot be had means a node
//! holds the folder, so nothing moves. Residual, left as it is: between the
//! set-aside and this app's own start, and between the put-back and the
//! start after it, no lock is held (the start's btxd must take it).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use btx_core::fast_forward::{
    self as ff, Before, Look, MoveError, Outcome, Phase, Record, Verdict,
};
use btx_core::snapshot_start::{StartRecord, StartSource};
use tauri::{AppHandle, Manager, State};

use crate::commands::{
    destructive_allowed, node_ownership, nominal_btxd_path, rpc_already_answering, set_phase,
    setup_log, snapshot_spec, start_node_inner, stop_node_inner, ALREADY_STARTING,
    SET_ASIDE_PENDING_FILE, SIGNED_LOAD_FAILED,
};
use crate::state::{node_datadir, AppState, NodeAppSettings, NodePhase};

/// How often the watch looks at the node.
const LOOK_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// The longest the check and download of step 1 may take. The node runs
/// meanwhile; this only keeps a server that drips from holding the driver.
const PREPARE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// How long a roll-back waits, a second at a time, for a start already
/// under way to end before it stops the node itself. Every start ends by
/// its own limits, and the stop before the wait ends a start's wait for RPC.
const HOLD_STARTS_TRIES: u32 = 600;

// ── What the driver says ────────────────────────────────────────────────────
//
// Sentences for the window, and reasons for `copy::rolled_back` ("... What
// happened: {reason}."), which is why a reason starts in lower case and has
// no full stop.

pub(crate) const ALREADY_RUNNING: &str = "Fast-forward is already running.";
pub(crate) const NOT_FINISHED: &str = "A Fast-forward has not finished yet. The next time \
     easyNode starts the node, it first carries that run on or puts the old chain data back.";
pub(crate) const NOTE_WAITS: &str = "easyNode still has to move a snapshot it did not accept out \
     of the way, which it does the next time it starts the node. Restart the node, then try \
     Fast-forward again.";
/// Remove node data, while a run is under way or recorded.
pub(crate) const REMOVE_WAITS: &str = "Fast-forward is running or has not finished, so easyNode \
     is not removing the node's data now. Try again once Fast-forward has finished.";
const NODE_IN_THE_WAY: &str = "easyNode has to finish putting the old chain data back after \
     Fast-forward, but a node it did not start is using the data folder. Stop that node, then \
     start the node again.";
const NOT_STOPPED: &str = "Fast-forward could not stop the node to put the old chain data back, \
     so it has not changed anything yet. If another app uses the node's data folder, stop it. \
     Then start the node again, and easyNode puts the old chain data back.";
const UNDO_FAILED: &str = "Fast-forward could not put the old chain data back, so easyNode has \
     not started the node. Start the node again to try once more.";
const MOVE_CUT_OFF: &str = "Fast-forward stopped unexpectedly while it was moving the chain \
     data. When you start the node again, easyNode first reads what the run had got to, and \
     either carries it on or puts the old chain data back.";
/// A start while this app's driver moves chain data, or is about to move it
/// back.
const MOVING: &str = "Fast-forward is moving the chain data, so easyNode is not starting the \
     node now. It starts the node itself once it has finished, or shows what went wrong.";

const WHY_NOT_OURS: &str = "another app is running the node in this data folder";
const WHY_NOT_RUNNING: &str = "the node was not running";
const WHY_NOT_CHECKED: &str = "the confirmed snapshot could not be downloaded and checked";
const WHY_STARTING: &str = "the node was starting or restarting";
const WHY_NOT_STOPPED: &str = "the node did not stop";
const WHY_NOT_SET_ASIDE: &str = "the chain data could not be set aside";
const WHY_NOTE_APPEARED: &str =
    "a snapshot the node did not accept was waiting to be moved out of the way";
const WHY_NOT_CLEARED: &str = "the result of the last Fast-forward could not be cleared";
const WHY_NOT_STARTED: &str = "the node did not start on the new chain data";
const WHY_CAUGHT_UP: &str =
    "the node had caught up with the confirmed snapshot by the time it was ready";
const WHY_NO_TIP: &str = "the node did not say which block it had reached";
/// A load during a run that loaded nothing (`commands::run_failure_reason`).
pub(crate) const WHY_NOT_LOADED: &str = "the node could not load the confirmed snapshot";
/// A load during a run that the app refuses.
pub(crate) const WHY_REFUSED: &str = "the snapshot the node loaded did not pass easyNode's checks";

/// At a start, when the run's record cannot be read: one sentence, and
/// nothing is touched.
fn unreadable_sentence(datadir: &Path) -> String {
    format!(
        "easyNode cannot read the Fast-forward record .fast-forward.json in {}, so it is not \
         starting the node and has changed nothing.",
        datadir.display()
    )
}

/// Where the old chain data is, when a roll-back could not put it all back.
/// `nothing_removed`: the restore could not begin (an original is missing
/// from the dated folder, or could not be checked), so the run stands as it
/// was, and the node is not started again in this run of the app: one plain
/// sentence (controller note 2b).
fn stranded_sentence(folder: &Path, nothing_removed: bool) -> String {
    if nothing_removed {
        format!(
            "Fast-forward could not put the old chain data back, because part of it is missing \
             from {} or could not be checked, so easyNode leaves the node stopped until you quit \
             and reopen it, then starts the node on the new chain data and carries Fast-forward \
             on.",
            folder.display()
        )
    } else {
        format!(
            "Fast-forward could not put all of the old chain data back, so easyNode has not \
             started the node. What is not back yet is in {}. Start the node again to try once \
             more.",
            folder.display()
        )
    }
}

// ── State ───────────────────────────────────────────────────────────────────

/// A driver is at work in this run of the app: a run, or the watch of one it
/// resumed. One at a time.
static DRIVING: AtomicBool = AtomicBool::new(false);

/// Set by the start path when a load failed during a run; the watch takes it
/// and rolls back.
static FAILURE: Mutex<Option<String>> = Mutex::new(None);

/// A roll-back in this run of the app whose restore could not begin: the
/// sentence, and no start until the app is opened again (controller note 2b).
static STUCK: Mutex<Option<String>> = Mutex::new(None);

/// The one serialisation point for the datadir's chain data and the run's
/// files.
static DISK: Mutex<()> = Mutex::new(());

/// Releases [`DRIVING`], a panic included.
struct Driving;

impl Drop for Driving {
    fn drop(&mut self) {
        DRIVING.store(false, Ordering::SeqCst);
    }
}

fn begin_driving() -> Option<Driving> {
    (!DRIVING.swap(true, Ordering::SeqCst)).then_some(Driving)
}

/// Holds `AppState::start_in_flight`, so no start runs, until dropped.
struct NoStarts<'a>(&'a AtomicBool);

impl Drop for NoStarts<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run `f` with the datadir to itself. Call only where blocking is fine: in
/// `spawn_blocking` ([`on_disk`]), or in code that is already there.
pub(crate) fn with_disk<T>(f: impl FnOnce() -> T) -> T {
    let _one = DISK.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// [`with_disk`] off the async runtime. `None` if the task panicked.
async fn on_disk<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tauri::async_runtime::spawn_blocking(move || with_disk(f))
        .await
        .ok()
}

fn log(datadir: &Path, msg: &str) {
    eprintln!("[fast-forward] {msg}");
    setup_log(datadir, &format!("fast-forward: {msg}"));
}

// ── What the rest of the app asks ───────────────────────────────────────────

/// A run is under way, or cannot be ruled out: a driver is at work, a run is
/// recorded, or a record is there that cannot be read. No run is offered or
/// started then, and Remove node data waits, unless the run is one whose old
/// chain cannot come back ([`removal_waits`]).
pub(crate) fn active() -> bool {
    active_in(&node_datadir())
}

pub(crate) fn active_in(datadir: &Path) -> bool {
    DRIVING.load(Ordering::SeqCst) || !matches!(ff::read_record(datadir), Ok(None))
}

/// The node runs on a run's new chain data (its record is at
/// [`Phase::Running`]): its loads are signed-only, one that fails goes to
/// the driver, and it is no first load. For the start path.
pub(crate) fn underway(datadir: &Path) -> bool {
    matches!(ff::read_record(datadir), Ok(Some(r)) if r.phase == Phase::Running)
}

/// Pure: may Remove node data go ahead? With no driver at work: when no run
/// is recorded, and when the one recorded is at [`Phase::Running`] and its
/// roll-back could not begin (`stuck`, [`STUCK`]): its old chain cannot come
/// back, and Remove node data removes the new chain with the rest (review
/// M6). Never while a driver is at work, over a record nobody can read, or
/// over a run at any other phase.
fn removal_goes_ahead(driving: bool, stuck: bool, record: OnRecord) -> bool {
    !driving
        && match record {
            OnRecord::Nothing => true,
            OnRecord::At(Phase::Running) => stuck,
            OnRecord::Unreadable | OnRecord::At(_) => false,
        }
}

/// Remove node data's first question, before it stops the node: must it
/// wait for a run ([`removal_goes_ahead`])?
pub(crate) fn removal_waits() -> bool {
    let stuck = STUCK.lock().unwrap_or_else(|e| e.into_inner()).is_some();
    !removal_goes_ahead(
        DRIVING.load(Ordering::SeqCst),
        stuck,
        on_record(&node_datadir()),
    )
}

/// Remove node data's part in a run, with the node stopped and the datadir
/// to itself ([`with_disk`]): `Ok` when it may go ahead
/// ([`removal_goes_ahead`]), a stuck run given up first; else
/// [`REMOVE_WAITS`], and nothing changes.
pub(crate) fn clear_for_removal(datadir: &Path) -> Result<(), String> {
    clear_for_removal_with(datadir, DRIVING.load(Ordering::SeqCst), &STUCK)
}

/// [`clear_for_removal`], given whether a driver is at work and the
/// [`STUCK`] to read. A stuck run is given up (`ff::abandon`, which checks
/// on disk that its old chain cannot come back): its record goes, the sweep
/// in Remove node data takes its dated folder, and starts are no longer held
/// off for it.
fn clear_for_removal_with(
    datadir: &Path,
    driving: bool,
    stuck: &Mutex<Option<String>>,
) -> Result<(), String> {
    let mut stuck = stuck.lock().unwrap_or_else(|e| e.into_inner());
    let record = on_record(datadir);
    if !removal_goes_ahead(driving, stuck.is_some(), record) {
        return Err(REMOVE_WAITS.into());
    }
    if record == OnRecord::At(Phase::Running) {
        if let Err(e) = ff::abandon(datadir) {
            log(datadir, &format!("not giving the run up: {e}"));
            return Err(REMOVE_WAITS.into());
        }
        log(
            datadir,
            "the run whose old chain data cannot come back is given up; Remove node data takes \
             its chain data with the rest",
        );
        *stuck = None;
    }
    Ok(())
}

/// Tell the driver the run's load failed, with the reason the window shows.
/// Called by the start path instead of its own restart while a run is
/// under way.
pub(crate) fn report_failure(why: String) {
    *FAILURE.lock().unwrap_or_else(|e| e.into_inner()) = Some(why);
}

fn take_failure() -> Option<String> {
    FAILURE.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Why no run may start now, or `None`. A run over a recorded one, or over a
/// record nobody can read, would lose it; and a set-aside note waiting for
/// the next launch names a chainstate the run would set aside with the rest,
/// which a roll-back would bring back without its note (controller note 1
/// (d)).
fn refuse_run(datadir: &Path) -> Option<String> {
    match ff::read_record(datadir) {
        Ok(None) => {}
        Ok(Some(_)) => return Some(NOT_FINISHED.into()),
        Err(_) => return Some(unreadable_sentence(datadir)),
    }
    datadir
        .join(SET_ASIDE_PENDING_FILE)
        .exists()
        .then(|| NOTE_WAITS.into())
}

/// Start a run. Returns at once; the Tools section asks for the status.
pub(crate) fn spawn_run(app: AppHandle) -> Result<(), String> {
    let Some(driving) = begin_driving() else {
        return Err(ALREADY_RUNNING.into());
    };
    if let Some(why) = refuse_run(&node_datadir()) {
        return Err(why);
    }
    tauri::async_runtime::spawn(async move {
        let _driving = driving;
        let state = app.state::<AppState>();
        run(&app, &state).await;
    });
    Ok(())
}

/// A run the app was closed on, and no driver: watch it to its end. Called
/// by the start path once the node is up; [`before_start`] has given it a
/// fresh watch window.
pub(crate) fn resume_if_needed(app: &AppHandle) {
    if !underway(&node_datadir()) {
        return;
    }
    let Some(driving) = begin_driving() else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _driving = driving;
        let state = app.state::<AppState>();
        watch(&app, &state).await;
    });
}

/// Nothing answers RPC on the datadir and nothing holds it: chain data may
/// move.
async fn node_is_down(datadir: &Path) -> bool {
    rpc_already_answering(datadir).await.is_none()
        && btx_core::node::datadir_holder(datadir).await == btx_core::node::DatadirHolder::Free
}

/// Why [`undo`] (or [`at_start`]) did not put the old chain data back or let
/// the start go on: the plain sentence for the window, and whether the
/// restore could not even begin (an original missing from the dated folder,
/// or one that could not be checked). Only that one keeps every start off
/// for the rest of this run of the app ([`STUCK`], controller note 2b);
/// after any other, starting again tries once more.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NotBack {
    said: String,
    could_not_begin: bool,
}

impl NotBack {
    fn plain(said: impl Into<String>) -> Self {
        NotBack {
            said: said.into(),
            could_not_begin: false,
        }
    }
}

/// What the run's record says, for [`start_gate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnRecord {
    Nothing,
    Unreadable,
    At(Phase),
}

fn on_record(datadir: &Path) -> OnRecord {
    match ff::read_record(datadir) {
        Ok(None) => OnRecord::Nothing,
        Ok(Some(r)) => OnRecord::At(r.phase),
        Err(_) => OnRecord::Unreadable,
    }
}

/// The last outcome is a roll-back's: beside a Running record, the roll-back
/// was decided and its restore never began.
fn rolled_back(datadir: &Path) -> bool {
    matches!(ff::read_outcome(datadir), Some(Outcome::RolledBack { .. }))
}

/// What the run's phase alone says the Tools status should show, before it
/// is allowed to look at any outcome (controller note 1, for Task 4's
/// `tools_fast_forward_status`): a roll-back's outcome is written before its
/// restore begins, so it may be shown only once no run is recorded at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolsPhase {
    /// Not running, and never called "running": a plain sentence. The
    /// driver's own, with the folder path, from [`STUCK`]; the one for a
    /// record nobody can read; or [`NOT_FINISHED`] for a run recorded with
    /// no driver at work, which waits for the next start.
    Halted(String),
    /// A run is genuinely under way: this app's driver is at work, before it
    /// has written a record (checking and downloading the confirmed pair,
    /// section 10 step 1) or at any phase after.
    Running,
    /// No run recorded and no driver at work: the last outcome, if any,
    /// says what happened.
    Idle,
}

/// Pure half of [`tools_status_phase`]. Only a driver at work moves a run
/// on; with none, a record at any phase waits for the next start, which
/// carries it on or puts the old chain data back (review I4).
fn tools_phase(driving: bool, stuck: Option<&str>, record: OnRecord, datadir: &Path) -> ToolsPhase {
    if let Some(said) = stuck {
        return ToolsPhase::Halted(said.into());
    }
    match record {
        OnRecord::Unreadable => ToolsPhase::Halted(unreadable_sentence(datadir)),
        _ if driving => ToolsPhase::Running,
        OnRecord::At(_) => ToolsPhase::Halted(NOT_FINISHED.into()),
        OnRecord::Nothing => ToolsPhase::Idle,
    }
}

/// What Task 4's Tools status asks first, before it looks at the last
/// outcome.
pub(crate) fn tools_status_phase(datadir: &Path) -> ToolsPhase {
    let stuck = STUCK.lock().unwrap_or_else(|e| e.into_inner()).clone();
    tools_phase(
        DRIVING.load(Ordering::SeqCst),
        stuck.as_deref(),
        on_record(datadir),
        datadir,
    )
}

/// What a start may do, from [`start_gate`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Gate {
    /// Launch: this app's driver is at work and the chain data is not
    /// moving, so it has done what a start needs.
    Start,
    /// No driver at work: the sweep and the recorded run come first
    /// ([`at_start`]).
    AtStart,
    /// Not now, and the plain sentence why.
    Refuse(String),
}

/// Pure: may a start go ahead? `stuck` ([`STUCK`]) keeps every start off.
/// While this app's driver is at work (`driving`), only on chain data that
/// is not moving: no run, a run under way (not being rolled back:
/// `rolling_back`, [`rolled_back`]) or one that is done. A run being set
/// aside or put back is the driver's, and a record nobody can read is no
/// one's to start over. With no driver, [`at_start`] decides.
fn start_gate(
    driving: bool,
    stuck: Option<&str>,
    record: OnRecord,
    rolling_back: bool,
    datadir: &Path,
) -> Gate {
    if let Some(said) = stuck {
        return Gate::Refuse(said.into());
    }
    if !driving {
        return Gate::AtStart;
    }
    match record {
        OnRecord::Nothing | OnRecord::At(Phase::Done) => Gate::Start,
        OnRecord::At(Phase::Running) if !rolling_back => Gate::Start,
        OnRecord::Unreadable => Gate::Refuse(unreadable_sentence(datadir)),
        OnRecord::At(_) => Gate::Refuse(MOVING.into()),
    }
}

/// At a start, before anything reads the chain data or the settings: the
/// gate ([`start_gate`]), then, with no driver at work, the sweep and the run
/// a previous start left ([`at_start`]). An undo needs the node down: a
/// record cut off while its chain data moved refuses the start while a node
/// is using the datadir, and a roll-back decided before the app stopped is
/// left to the watch then, which can stop the node. An `Err` is a plain
/// sentence, and the node is not started. A roll-back whose restore cannot
/// begin keeps the node from starting in this run of the app.
pub(crate) async fn before_start(datadir: &Path) -> Result<(), String> {
    let stuck = STUCK.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let record = on_record(datadir);
    let rolling_back = rolled_back(datadir);
    match start_gate(
        DRIVING.load(Ordering::SeqCst),
        stuck.as_deref(),
        record,
        rolling_back,
        datadir,
    ) {
        Gate::Start => return Ok(()),
        Gate::Refuse(said) => return Err(said),
        Gate::AtStart => {}
    }
    let moved = matches!(
        record,
        OnRecord::At(Phase::SettingAside | Phase::Undoing | Phase::Restoring)
    );
    let decided = record == OnRecord::At(Phase::Running) && rolling_back;
    let node_down = (moved || decided) && node_is_down(datadir).await;
    if moved && !node_down {
        log(
            datadir,
            "a run is being undone, and a node this app did not start is using the data folder; \
             not starting",
        );
        return Err(NODE_IN_THE_WAY.into());
    }
    let dd = datadir.to_path_buf();
    match on_disk(move || at_start(&dd, now(), node_down)).await {
        Some(Ok(())) => Ok(()),
        Some(Err(not_back)) => {
            if not_back.could_not_begin {
                *STUCK.lock().unwrap_or_else(|e| e.into_inner()) = Some(not_back.said.clone());
            }
            Err(not_back.said)
        }
        // The task stopped unexpectedly: what the run had got to is on disk,
        // and starting again reads it.
        None => Err(MOVE_CUT_OFF.into()),
    }
}

/// [`before_start`]'s part on disk. The sweep first; then a run at
/// [`Phase::Running`] gets a fresh watch window and carries on, unless its
/// roll-back was decided before the app stopped ([`rolled_back`]): then,
/// with the node down (`node_down`), it is rolled back here. One at
/// [`Phase::Done`] is finished, and one cut off while its chain data moved
/// ([`Phase::SettingAside`], [`Phase::Undoing`], [`Phase::Restoring`]) is
/// rolled back here, before any launch; call with the node down then. A
/// record nobody can read stops the start and nothing is touched.
fn at_start(datadir: &Path, now_unix: u64, node_down: bool) -> Result<(), NotBack> {
    for gone in ff::sweep(datadir) {
        log(
            datadir,
            &format!("removed {}, which no run needs", gone.display()),
        );
    }
    let record = match ff::resume(datadir, now_unix) {
        Ok(Some(r)) => r,
        Ok(None) => return Ok(()),
        Err(e) => {
            log(
                datadir,
                &format!("the run's record cannot be read ({e}); not starting the node"),
            );
            return Err(NotBack::plain(unreadable_sentence(datadir)));
        }
    };
    match record.phase {
        Phase::Running if node_down && rolled_back(datadir) => {
            log(
                datadir,
                "the roll-back decided before the app stopped is done before the launch",
            );
            undo(datadir)
        }
        Phase::Running if rolled_back(datadir) => {
            log(
                datadir,
                "a node is using the data folder, so the roll-back decided before the app \
                 stopped is left to the watch, which stops it first",
            );
            Ok(())
        }
        Phase::Running => {
            log(
                datadir,
                &format!(
                    "the run to block {} is watched again from this start",
                    record.height
                ),
            );
            Ok(())
        }
        Phase::Done => {
            // The new chain is kept either way; a finish cut off again is
            // carried on at the next start.
            if let Err(e) = ff::finish(datadir) {
                log(
                    datadir,
                    &format!("could not finish the run that is done: {e}"),
                );
            }
            Ok(())
        }
        Phase::SettingAside | Phase::Undoing | Phase::Restoring => {
            let why = match ff::judge(&record, &Look::default(), now_unix) {
                Verdict::RollBack(why) => why,
                _ => "the app stopped in the middle of Fast-forward".into(),
            };
            // A roll-back that was cut off wrote its real reason before its
            // restore began (controller note 2); the run clears any older
            // outcome before it sets anything aside.
            if ff::read_outcome(datadir).is_none() {
                ff::write_outcome(
                    datadir,
                    &Outcome::RolledBack {
                        reason: why.clone(),
                    },
                );
            }
            log(datadir, &format!("rolling back before the launch: {why}"));
            undo(datadir)
        }
    }
}

/// What the watch makes of one look: a roll-back decided before the app
/// stopped (its outcome beside the Running record) is carried out, however
/// the node looks; otherwise `ff::judge`.
fn verdict(record: &Record, look: &Look, outcome: Option<Outcome>, now_unix: u64) -> Verdict {
    match outcome {
        Some(Outcome::RolledBack { reason }) if record.phase == Phase::Running => {
            Verdict::RollBack(reason)
        }
        _ => ff::judge(record, look, now_unix),
    }
}

// ── On disk, each under `with_disk` ─────────────────────────────────────────

/// The two settings a run changes, as they are now.
fn run_settings(datadir: &Path) -> Before {
    let s = NodeAppSettings::load(datadir);
    Before {
        snapshot_loaded: s.snapshot_loaded,
        first_load_pending: s.first_load_pending,
    }
}

/// Write the two settings, and the snapshot marker as "snapshot loaded"
/// says.
fn put_settings(datadir: &Path, to: Before) {
    NodeAppSettings::update(datadir, |s| {
        s.snapshot_loaded = to.snapshot_loaded;
        s.first_load_pending = to.first_load_pending;
    });
    if to.snapshot_loaded {
        btx_core::snapshot::mark_snapshot_marker(datadir);
    } else {
        btx_core::snapshot::clear_snapshot_marker(datadir);
    }
}

fn settings_before(record: &Record) -> Before {
    Before {
        snapshot_loaded: record.snapshot_loaded_before,
        first_load_pending: record.first_load_pending_before,
    }
}

/// Section 10, step 2, with the node stopped: the chain data and the start
/// record go aside and the run is recorded, the settings as they were with
/// it (`ff::set_aside`). Before anything moves, "snapshot loaded" is reset,
/// so the loaders load again and the watch waits for the loader's own word
/// ([`loaded_by_the_loader`]), and "first load still to come" too: a run is
/// no first load (controller note 1 (a)), and the start path keeps it so
/// while the run is under way. When nothing moved after all, both go back;
/// when what moved could not all go back, the record keeps them for the
/// restore. A set-aside note written since the run's own check (a load
/// refused meanwhile) stops it here, with the datadir to itself: the note's
/// chainstate would go aside with the rest, and a roll-back would bring it
/// back without its note (controller note 1 (d)).
fn set_aside_for_run(datadir: &Path, height: u64, now_unix: u64) -> Result<Record, MoveError> {
    if datadir.join(SET_ASIDE_PENDING_FILE).exists() {
        return Err(MoveError::Untouched(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "a set-aside note is waiting for the next launch",
        )));
    }
    let _engine = engine_lock(datadir).map_err(MoveError::Untouched)?;
    let before = run_settings(datadir);
    put_settings(datadir, Before::default());
    let set = ff::set_aside(datadir, height, before, now_unix);
    if matches!(set, Err(MoveError::Untouched(_))) {
        put_settings(datadir, before);
    }
    set
}

/// The engine's own lock on the data folder, held for a move of chain data
/// (review I5): while it is held, no btxd starts on the folder, whichever
/// app launches it (the easyBTX miner shares it). One that cannot be had
/// means a node holds the folder: the node is not down, and nothing moves.
/// That error's kind is `WouldBlock`. Not for [`ff::finish`], which runs
/// beside this app's own node on the new chain (it holds the lock itself,
/// so no other btxd starts then either) and touches only the dated folder,
/// which no engine reads.
fn engine_lock(datadir: &Path) -> std::io::Result<btx_core::fsx::EngineLock> {
    use btx_core::fsx::{EngineLock, EngineLockError};
    EngineLock::take(datadir).map_err(|e| {
        log(datadir, &format!("not moving chain data: {e}"));
        match e {
            EngineLockError::Held => {
                std::io::Error::new(std::io::ErrorKind::WouldBlock, e.to_string())
            }
            EngineLockError::Io(e) => e,
        }
    })
}

/// Why the chain data was not set aside (`error`, what
/// [`set_aside_for_run`] said), for the Tools status.
fn why_not_set_aside(datadir: &Path, error: &std::io::Error) -> &'static str {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        WHY_NOT_STOPPED
    } else if datadir.join(SET_ASIDE_PENDING_FILE).exists() {
        WHY_NOTE_APPEARED
    } else {
        WHY_NOT_SET_ASIDE
    }
}

/// Remove the snapshot chainstates the attempt's loads were refused and set
/// aside (`commands::set_aside_refused_snapshot`): those named after a time
/// at or after the run began (`since`). Older ones, and anything else by
/// that name (a file, a link, a name without a time), are left to the
/// weekly sweep (`btx_core::disk`).
fn remove_attempts_refused(datadir: &Path, since: u64) {
    let prefix = btx_core::confirmed_load::REFUSED_CHAINSTATE_PREFIX;
    let Ok(entries) = std::fs::read_dir(datadir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let made_by_the_attempt = name
            .to_str()
            .and_then(|n| n.strip_prefix(prefix))
            .and_then(|t| t.parse::<u64>().ok())
            .is_some_and(|t| t >= since);
        // `file_type` does not follow a link.
        if !made_by_the_attempt || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        match std::fs::remove_dir_all(entry.path()) {
            Ok(()) => log(
                datadir,
                &format!(
                    "removed {}, which the attempt set aside",
                    entry.path().display()
                ),
            ),
            Err(e) => log(
                datadir,
                &format!(
                    "could not remove {} (non-fatal): {e}",
                    entry.path().display()
                ),
            ),
        }
    }
}

/// Put the old chain data back, with the node stopped and the engine's own
/// lock on the folder held throughout ([`engine_lock`]; not to be had, the
/// run stands as it is and the sentence says so). The settings from
/// before the run go back first, from the record, so a crash in the restore
/// cannot lose them, and again once it returned `Ok` (controller notes 2 and
/// 2c). Then the markers of the attempt's launches go (a mirror launch or a
/// header bootstrap on the old chain would be wrong), and so does a
/// set-aside note the attempt wrote: a run never starts while one waits, so
/// it names the attempt's chainstate; and so do the chainstates its loads
/// were refused and set aside ([`remove_attempts_refused`]). `Err` is a
/// plain sentence: the old chain data is not all back, and the node must not
/// start. A restore that could not begin changed nothing, and the decision
/// to roll back goes with it: the run stands, and is watched again from the
/// next opening of the app (controller note 2b).
fn undo(datadir: &Path) -> Result<(), NotBack> {
    let record = match ff::read_record(datadir) {
        Ok(Some(r)) => r,
        Ok(None) => return Ok(()),
        Err(e) => {
            log(datadir, &format!("the run's record cannot be read ({e})"));
            return Err(NotBack::plain(unreadable_sentence(datadir)));
        }
    };
    // Held until the old chain data is back, or the restore has stopped.
    let Ok(_engine) = engine_lock(datadir) else {
        return Err(NotBack::plain(NOT_STOPPED));
    };
    let during = run_settings(datadir);
    let before = settings_before(&record);
    put_settings(datadir, before);
    match ff::restore(datadir) {
        Ok(_) => {
            put_settings(datadir, before);
            btx_core::node::end_mirror_load(datadir);
            btx_core::node::end_header_bootstrap(datadir);
            match std::fs::remove_file(datadir.join(SET_ASIDE_PENDING_FILE)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => log(
                    datadir,
                    &format!("could not remove the attempt's set-aside note: {e}"),
                ),
                _ => {}
            }
            remove_attempts_refused(datadir, record.started_at);
            log(datadir, "the old chain data is back");
            Ok(())
        }
        Err(e) => {
            log(
                datadir,
                &format!("putting the old chain data back stopped: {e}"),
            );
            // A restore that could not begin changed nothing: the run
            // stands, with its own settings and launches.
            let nothing_removed =
                matches!(ff::read_record(datadir), Ok(Some(r)) if r.phase == Phase::Running);
            if nothing_removed {
                put_settings(datadir, during);
                ff::clear_outcome(datadir);
            } else {
                btx_core::node::end_mirror_load(datadir);
                btx_core::node::end_header_bootstrap(datadir);
            }
            Err(match e {
                MoveError::Stranded { folder, .. } => NotBack {
                    said: stranded_sentence(&folder, nothing_removed),
                    could_not_begin: nothing_removed,
                },
                MoveError::Untouched(_) => NotBack::plain(UNDO_FAILED),
            })
        }
    }
}

/// Pure: the start record the load wrote, when the run may be kept by it: a
/// snapshot the operators confirmed, at or above the run's height (the start
/// path may have found a newer one), whose base block is the one the node's
/// snapshot chainstate is built on (`node_base`, `getchainstates`). The base
/// check is the one `ff::judge` leaves to the driver, before `ff::finish`.
/// Otherwise the reason for the roll-back.
fn confirmed_start(
    run_height: u64,
    start: Option<StartRecord>,
    node_base: Option<&str>,
) -> Result<StartRecord, String> {
    let start = start
        .filter(|s| s.source == StartSource::Confirmed && s.height >= run_height)
        .ok_or("the node's new start point is not a snapshot the operators confirmed")?;
    if node_base.is_some_and(|h| h.eq_ignore_ascii_case(&start.block_hash)) {
        Ok(start)
    } else {
        Err("the snapshot the node runs on is not the one the operators confirmed".into())
    }
}

/// A run judged done, its snapshot checked: the outcome first, naming who
/// confirmed it (the start record holds only operators whose signatures the
/// app verified), then `ff::finish`, which records the run done before any
/// old chain data goes (controller note 1 (c)).
fn keep(datadir: &Path, start: &StartRecord) -> std::io::Result<()> {
    ff::write_outcome(
        datadir,
        &Outcome::Done {
            height: start.height,
            operators: start.operators.clone(),
        },
    );
    ff::finish(datadir)
}

/// `ff::sweep`, and the bytes it freed: each dated folder it removed,
/// measured first. For Remove node data's report.
pub(crate) fn sweep_measured(datadir: &Path) -> u64 {
    let sizes: Vec<(PathBuf, u64)> = std::fs::read_dir(datadir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.file_type().is_ok_and(|t| t.is_dir())
                        && e.file_name()
                            .to_str()
                            .is_some_and(|n| n.starts_with("fast-forward-"))
                })
                .map(|e| {
                    let p = e.path();
                    let bytes = btx_core::disk::dir_size_bytes(&p);
                    (p, bytes)
                })
                .collect()
        })
        .unwrap_or_default();
    let removed = ff::sweep(datadir);
    for gone in &removed {
        log(
            datadir,
            &format!("removed {}, which no run needs", gone.display()),
        );
    }
    sizes
        .into_iter()
        .filter(|(p, _)| removed.contains(p))
        .map(|(_, bytes)| bytes)
        .sum()
}

// ── The run ─────────────────────────────────────────────────────────────────

/// Pure: did the driver's own start meet a start someone else began the
/// moment the driver let the start guard go? That one launches the same
/// node through the same gate, so it counts as the driver's.
fn started_elsewhere(error: &str) -> bool {
    error == ALREADY_STARTING
}

/// The driver's start: `start_node_inner`, with a failure projected into the
/// phase as `commands::start_node_projected` does, and a start already under
/// way ([`started_elsewhere`]) counted as this one.
async fn start_node(app: &AppHandle, state: &State<'_, AppState>) -> Result<(), String> {
    match start_node_inner(app, state).await {
        Err(e) if started_elsewhere(&e) => {
            log(
                &node_datadir(),
                "a start already under way launches the node",
            );
            Ok(())
        }
        Err(message) => {
            let shown = NodePhase::Error {
                message: message.clone(),
            };
            set_phase(app, state, shown).await;
            Err(message)
        }
        Ok(()) => Ok(()),
    }
}

/// A run that ended before anything moved: why, for the Tools status.
async fn not_started(datadir: &Path, why: &str) {
    log(datadir, &format!("not started: {why}"));
    let (dd, reason) = (datadir.to_path_buf(), why.to_string());
    on_disk(move || ff::write_outcome(&dd, &Outcome::RolledBack { reason })).await;
}

/// Does this host follow signatures, or check blocks itself? Its lasting
/// role, as the start path reads it.
pub(crate) fn follows_signatures_here(datadir: &Path) -> bool {
    btx_core::node::host_follows_signatures(
        &nominal_btxd_path(),
        datadir,
        btx_core::backend::node_host_backend(),
    )
}

/// What the confirmed snapshot is judged with before a run, by Tools' check
/// and by step 1: the view the run's load will have
/// (`confirmed_load::launch_view`). A node that follows signatures loads in
/// place, with its own engine's pins; a node that checks blocks loads in its
/// one mirror launch, which pins every compiled key, while its running
/// engine may pin none.
pub(crate) async fn run_view(
    rpc: &btx_core::rpc::RpcClient,
    follows_signatures: bool,
) -> btx_core::confirmed_snapshot::NodeView {
    btx_core::confirmed_load::launch_view(
        rpc,
        &btx_core::node::BTX_TRUSTED_ATTESTATION_PUBKEYS,
        btx_core::attested_snapshot::fallback_start(snapshot_spec().anchor_height),
        follows_signatures,
    )
    .await
}

/// Step 1: the confirmed pair, checked as the loader checks it and on disk.
/// A disputed `latest` stops here. The error is for the log.
async fn prepare(
    rpc: &btx_core::rpc::RpcClient,
    datadir: &Path,
) -> Result<btx_core::attested_snapshot::ReadyPair, String> {
    use btx_core::attested_snapshot as attested;
    let view = run_view(rpc, follows_signatures_here(datadir)).await;
    let client = attested::http_client()?;
    let regtest_env = btx_core::operators::regtest_env();
    tokio::time::timeout(
        PREPARE_DEADLINE,
        attested::prepare_confirmed(
            &client,
            attested::CONFIRMED_POINTER_URL,
            datadir,
            &view,
            regtest_env.as_deref(),
            attested::confirmed_url_allowed,
        ),
    )
    .await
    .map_err(|_| "the check and download took more than 30 minutes".to_string())?
}

/// Pure: after step 1, is the confirmed snapshot still above the node's tip
/// (`tip`, `None` when the node did not say)? The button's rule is applied
/// when Tools opens, the second click can come any time later, and the node
/// keeps syncing through a download that may take half an hour: a node that
/// passed the snapshot meanwhile must not have its further, checked chain
/// set aside for an older one. Otherwise the reason, and nothing moves.
fn still_ahead(pair_height: u64, tip: Option<u64>) -> Result<(), &'static str> {
    match tip {
        Some(tip) if pair_height > tip => Ok(()),
        Some(_) => Err(WHY_CAUGHT_UP),
        None => Err(WHY_NO_TIP),
    }
}

async fn run(app: &AppHandle, state: &State<'_, AppState>) {
    let datadir = node_datadir();
    let dd = datadir.clone();
    // The watch reads a roll-back's outcome beside a Running record as a
    // roll-back decided ([`verdict`]), so none may be left from before.
    let cleared = on_disk(move || {
        ff::clear_outcome(&dd);
        ff::read_outcome(&dd).is_none()
    })
    .await;
    if cleared != Some(true) {
        log(
            &datadir,
            "the last run's outcome could not be removed; not starting",
        );
        return not_started(&datadir, WHY_NOT_CLEARED).await;
    }
    take_failure();

    if let Err(e) = destructive_allowed(node_ownership(state, &datadir).await) {
        log(
            &datadir,
            &format!("the node is not this app's to stop: {e}"),
        );
        return not_started(&datadir, WHY_NOT_OURS).await;
    }
    let rpc = state.rpc.lock().await.clone();
    let Some(rpc) = rpc else {
        return not_started(&datadir, WHY_NOT_RUNNING).await;
    };

    // Section 10, step 1: check and download while the node keeps running.
    let pair = match prepare(&rpc, &datadir).await {
        Ok(pair) => pair,
        Err(e) => {
            log(
                &datadir,
                &format!("the confirmed snapshot is not ready: {e}"),
            );
            return not_started(&datadir, WHY_NOT_CHECKED).await;
        }
    };
    // The node kept syncing through step 1: still behind the snapshot?
    let tip = btx_core::node_api::get_blockchain_info(&rpc)
        .await
        .ok()
        .map(|info| info.blocks);
    if let Err(why) = still_ahead(pair.height, tip) {
        log(
            &datadir,
            &format!(
                "the confirmed snapshot {} is no longer above the node's tip ({tip:?}); nothing \
                 moved",
                pair.height
            ),
        );
        return not_started(&datadir, why).await;
    }
    btx_core::attested_snapshot::prune_others(&datadir, pair.height);
    log(
        &datadir,
        &format!(
            "confirmed snapshot {} ready; setting the chain data aside",
            pair.height
        ),
    );
    if state.quitting.load(Ordering::SeqCst) {
        return;
    }

    // Step 2: stop, and set the chain data aside, with no start under way.
    let Some(no_starts) = state
        .start_in_flight
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
        .then(|| NoStarts(&state.start_in_flight))
    else {
        return not_started(&datadir, WHY_STARTING).await;
    };
    stop_node_inner(state).await;
    set_phase(app, state, NodePhase::Stopped).await;
    if !node_is_down(&datadir).await {
        drop(no_starts);
        not_started(&datadir, WHY_NOT_STOPPED).await;
        let _ = start_node(app, state).await;
        return;
    }
    let (dd, height, at) = (datadir.clone(), pair.height, now());
    match on_disk(move || set_aside_for_run(&dd, height, at)).await {
        Some(Ok(record)) => log(
            &datadir,
            &format!(
                "chain data set aside in {}; starting the node",
                record.aside
            ),
        ),
        Some(Err(MoveError::Untouched(e))) => {
            log(
                &datadir,
                &format!("the chain data could not be set aside: {e}"),
            );
            drop(no_starts);
            not_started(&datadir, why_not_set_aside(&datadir, &e)).await;
            let _ = start_node(app, state).await;
            return;
        }
        Some(Err(MoveError::Stranded { folder, error })) => {
            // What moved could not all go back: the record stays, the next
            // start carries the put-back on, and nothing starts before it.
            log(
                &datadir,
                &format!("setting aside failed and not all went back ({error})"),
            );
            not_started(&datadir, WHY_NOT_SET_ASIDE).await;
            let message = stranded_sentence(&folder, false);
            set_phase(app, state, NodePhase::Error { message }).await;
            return;
        }
        None => {
            log(&datadir, "the set-aside task stopped unexpectedly");
            let message = MOVE_CUT_OFF.to_string();
            set_phase(app, state, NodePhase::Error { message }).await;
            return;
        }
    }
    // A person asked for this: a load that failed or was refused earlier in
    // this run of the app does not stand in its way (a validating node's
    // mirror launch waits on both). During the run a failure goes to the
    // driver, not to the start path's restarts, so these still bound them.
    SIGNED_LOAD_FAILED.store(false, Ordering::SeqCst);
    state.load_failure_restarted.store(false, Ordering::SeqCst);
    drop(no_starts);

    // Step 3: start. The start path does the rest: the header bootstrap of
    // the empty datadir, a validating node's one mirror launch, the load
    // with every check (signed-only during a run), the restart as a
    // validating node, and a failed load handed to this driver.
    if let Err(e) = start_node(app, state).await {
        log(&datadir, &format!("the node did not start: {e}"));
        roll_back(app, state, WHY_NOT_STARTED.into()).await;
        return;
    }
    watch(app, state).await;
}

/// What the node shows now, and the base block of its snapshot chainstate.
async fn look(state: &AppState, datadir: &Path) -> (Look, Option<String>) {
    let rpc = state.rpc.lock().await.clone();
    let mut base = None;
    let mut snapshot_base_height = None;
    if let Some(rpc) = &rpc {
        if let Ok(cs) = btx_core::node_api::get_chainstates(rpc).await {
            base = cs.snapshot().and_then(|c| c.snapshot_blockhash.clone());
        }
        if let Some(hash) = &base {
            use btx_core::rpc::Rpc as _;
            snapshot_base_height = rpc
                .call("getblockheader", serde_json::json!([hash, true]))
                .await
                .ok()
                .and_then(|h| h["height"].as_u64());
        }
    }
    let look = Look {
        snapshot_base_height,
        mirror_load_pending: btx_core::node::mirror_load_marker_exists(datadir),
        header_bootstrap_pending: btx_core::node::header_bootstrap_pending(datadir),
        running: rpc.is_some(),
        load_failed: take_failure(),
        loaded: loaded_by_the_loader(datadir),
    };
    (look, base)
}

/// Has the loader said this run's snapshot is loaded? The app's "a snapshot
/// was loaded" setting: the run resets it before anything moves
/// ([`set_aside_for_run`]), and only the loader sets it again, once its own
/// check after the load has passed (`btx_core::snapshot`'s
/// `after_signed_load`), on a validating node's mirror launch before the
/// relaunch. The engine shows the snapshot chainstate before that check
/// ends, so the watch waits for this too (`ff::judge`).
fn loaded_by_the_loader(datadir: &Path) -> bool {
    NodeAppSettings::load(datadir).snapshot_loaded
}

/// Step 4: look until the run is done or has to be undone. A quit leaves
/// the record, and the next start resumes the watch; the limit bounds it.
async fn watch(app: &AppHandle, state: &State<'_, AppState>) {
    let datadir = node_datadir();
    loop {
        tokio::time::sleep(LOOK_EVERY).await;
        if state.quitting.load(Ordering::SeqCst) {
            return;
        }
        let record = match ff::read_record(&datadir) {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(e) => {
                log(&datadir, &format!("the run's record cannot be read ({e})"));
                return;
            }
        };
        let (look, base) = look(state, &datadir).await;
        match verdict(&record, &look, ff::read_outcome(&datadir), now()) {
            Verdict::Continue => {}
            Verdict::Done => return finish_run(app, state, &record, base).await,
            Verdict::RollBack(why) => return roll_back(app, state, why).await,
        }
    }
}

/// A run judged done: kept once its snapshot is the one the operators
/// confirmed, else rolled back.
async fn finish_run(
    app: &AppHandle,
    state: &State<'_, AppState>,
    record: &Record,
    base: Option<String>,
) {
    let datadir = node_datadir();
    if record.phase == Phase::Done {
        // Checked before it was recorded done; only the finish is left.
        let dd = datadir.clone();
        if let Some(Err(e)) = on_disk(move || ff::finish(&dd)).await {
            log(
                &datadir,
                &format!("could not finish the run that is done: {e}"),
            );
        }
        return;
    }
    let start = btx_core::snapshot_start::read(&datadir);
    let start = match confirmed_start(record.height, start, base.as_deref()) {
        Ok(start) => start,
        Err(why) => return roll_back(app, state, why).await,
    };
    let (dd, height) = (datadir.clone(), start.height);
    match on_disk(move || keep(&dd, &start)).await {
        Some(Ok(())) => log(
            &datadir,
            &format!("done: the node now starts from block {height}"),
        ),
        Some(Err(e)) => log(
            &datadir,
            &format!(
                "done, but the old chain data is not removed yet ({e}); the next start carries on"
            ),
        ),
        None => log(&datadir, "the task that keeps the run stopped unexpectedly"),
    }
}

/// Stop, put the old chain data back, start as before, and say why. The
/// reason is written first, so a restore cut off by a crash keeps it
/// (controller note 2). Starts are held off from before the second stop
/// until the old chain data is back; the node starts again only once
/// [`undo`] returned `Ok`, and otherwise the window says, in plain words,
/// where the old chain data is, and [`start_gate`] keeps starts off.
async fn roll_back(app: &AppHandle, state: &State<'_, AppState>, why: String) {
    let datadir = node_datadir();
    log(&datadir, &format!("rolling back: {why}"));
    let (dd, reason) = (datadir.clone(), why);
    on_disk(move || ff::write_outcome(&dd, &Outcome::RolledBack { reason })).await;

    // No start may launch btxd while the chain data moves back. A start
    // under way ends by its own limits; the stop ends its wait for RPC.
    stop_node_inner(state).await;
    let mut held = None;
    for _ in 0..HOLD_STARTS_TRIES {
        if state
            .start_in_flight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            held = Some(NoStarts(&state.start_in_flight));
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    let Some(no_starts) = held else {
        log(
            &datadir,
            "a start did not end, so the old chain data was not put back",
        );
        let message = NOT_STOPPED.to_string();
        set_phase(app, state, NodePhase::Error { message }).await;
        return;
    };
    // A start that ended meanwhile may have left a node running.
    stop_node_inner(state).await;
    set_phase(app, state, NodePhase::Stopped).await;
    if !node_is_down(&datadir).await {
        log(
            &datadir,
            "the node did not stop, so the old chain data was not put back",
        );
        let message = NOT_STOPPED.to_string();
        set_phase(app, state, NodePhase::Error { message }).await;
        drop(no_starts);
        return;
    }
    let dd = datadir.clone();
    let undone = on_disk(move || undo(&dd))
        .await
        .unwrap_or_else(|| Err(NotBack::plain(MOVE_CUT_OFF)));
    match undone {
        Ok(()) => {
            // The old chain data is back: starts may run again.
            drop(no_starts);
            if !state.quitting.load(Ordering::SeqCst) {
                let _ = start_node(app, state).await;
            }
        }
        Err(not_back) => {
            // Starts stay off while the window is told; after that the
            // gate keeps them off (STUCK, or the record's phase).
            if not_back.could_not_begin {
                *STUCK.lock().unwrap_or_else(|e| e.into_inner()) = Some(not_back.said.clone());
            }
            let message = not_back.said;
            set_phase(app, state, NodePhase::Error { message }).await;
            drop(no_starts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use btx_core::attested_snapshot::PairKind;
    use btx_core::snapshot::{mark_snapshot_marker, snapshot_marker_present};
    use btx_core::snapshot_start::{StartRecord, StartSource};

    const CHAIN_DIRS: [&str; 5] = [
        "blocks",
        "chainstate",
        "chainstate_snapshot",
        "indexes",
        "shielded_state",
    ];

    /// A datadir holding a chain (each chain folder has a file `old`), its
    /// start record, a wallet, and the app's two settings as `before` says.
    fn datadir_with_chain(before: Before) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        for name in CHAIN_DIRS {
            std::fs::create_dir_all(d.join(name)).unwrap();
            std::fs::write(d.join(name).join("old"), name).unwrap();
        }
        std::fs::write(d.join("snapshot-start.json"), b"old start").unwrap();
        std::fs::create_dir_all(d.join("wallets/main")).unwrap();
        std::fs::write(d.join("wallets/main/wallet.dat"), b"keys").unwrap();
        NodeAppSettings::update(d, |s| {
            s.snapshot_loaded = before.snapshot_loaded;
            s.first_load_pending = before.first_load_pending;
        });
        if before.snapshot_loaded {
            mark_snapshot_marker(d);
        }
        tmp
    }

    /// What an attempt leaves: new chain data, the start record its load
    /// wrote, and the markers of its launches.
    fn attempt(d: &Path) {
        for name in ["blocks", "chainstate", "chainstate_snapshot"] {
            std::fs::create_dir_all(d.join(name)).unwrap();
            std::fs::write(d.join(name).join("new"), b"new").unwrap();
        }
        std::fs::write(d.join("snapshot-start.json"), b"new start").unwrap();
        btx_core::node::begin_header_bootstrap(d);
        btx_core::node::begin_mirror_load(d, PairKind::Confirmed, 232_000).unwrap();
    }

    /// The run's record, rewritten at `phase`, as a crash between two steps
    /// leaves it.
    fn record_at(d: &Path, phase: Phase) {
        let mut r = ff::read_record(d).unwrap().unwrap();
        r.phase = phase;
        std::fs::write(
            d.join(".fast-forward.json"),
            serde_json::to_vec(&r).unwrap(),
        )
        .unwrap();
    }

    fn launch_markers(d: &Path) -> bool {
        btx_core::node::mirror_load_marker_exists(d) || d.join(".header-bootstrap").exists()
    }

    /// The datadir as [`datadir_with_chain`] made it, with no run left and
    /// no marker of the attempt's launches.
    fn assert_old_chain_back(d: &Path, when: &str) {
        for name in CHAIN_DIRS {
            let entries: Vec<String> = std::fs::read_dir(d.join(name))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            assert_eq!(entries, ["old"], "{name}, {when}");
        }
        assert_eq!(
            std::fs::read_to_string(d.join("snapshot-start.json")).unwrap(),
            "old start",
            "{when}"
        );
        assert!(d.join("wallets/main/wallet.dat").exists(), "{when}");
        let left: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| {
                n.starts_with("fast-forward-")
                    || n == ".fast-forward.json"
                    || n == ".fast-forward-undo.json"
            })
            .collect();
        assert!(left.is_empty(), "{left:?} left, {when}");
        assert!(
            !launch_markers(d),
            "the attempt's launch markers go, {when}"
        );
    }

    fn entries_named(d: &Path, prefix: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(prefix))
            .collect();
        v.sort();
        v
    }

    /// Controller note 1 (a0): a record nobody can read stops the start
    /// with one plain sentence, and nothing is touched: not the record, not
    /// the dated folder, not the chain data in place. No run starts over it
    /// and Remove node data waits.
    #[test]
    fn a_record_that_cannot_be_read_keeps_the_node_stopped_and_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::create_dir_all(d.join("blocks")).unwrap();
        std::fs::write(d.join("blocks/new"), b"new").unwrap();
        std::fs::create_dir_all(d.join("fast-forward-5/blocks")).unwrap();
        std::fs::write(d.join("fast-forward-5/blocks/old"), b"old").unwrap();
        std::fs::write(d.join(".fast-forward.json"), b"{\"height\":").unwrap();

        let not_back = at_start(d, 1_000, true).unwrap_err();
        assert!(!not_back.could_not_begin);
        let said = not_back.said;
        assert_eq!(said, unreadable_sentence(d));
        assert!(said.contains(".fast-forward.json"), "{said}");
        assert!(said.contains(&d.display().to_string()), "{said}");
        assert!(
            !said.contains("EOF") && !said.contains("line 1"),
            "no raw error: {said}"
        );
        assert_eq!(said.matches(". ").count(), 0, "one sentence: {said}");

        assert_eq!(
            std::fs::read(d.join(".fast-forward.json")).unwrap(),
            b"{\"height\":"
        );
        assert!(d.join("fast-forward-5/blocks/old").exists(), "not swept");
        assert!(d.join("blocks/new").exists());
        assert!(active_in(d));
        assert!(!underway(d));
        assert_eq!(refuse_run(d), Some(said));
    }

    /// Controller notes 1 (a0, b), 2 and 2b: a run the app stopped in the
    /// middle of moving chain data, either way, is put back before anything
    /// is launched, whatever the node would show: cut off while setting
    /// aside, while the attempt's data was going, and half-way through
    /// putting the old data back. The settings from before the run come
    /// back, and the attempt's launch markers go.
    #[test]
    fn a_run_cut_off_while_chain_data_moved_is_put_back_before_the_launch() {
        let before = Before {
            snapshot_loaded: true,
            first_load_pending: false,
        };
        for phase in [Phase::SettingAside, Phase::Undoing, Phase::Restoring] {
            let tmp = datadir_with_chain(before);
            let d = tmp.path();
            let record = set_aside_for_run(d, 232_000, 100).unwrap();
            let folder = d.join(&record.aside);
            match phase {
                // Two entries never left.
                Phase::SettingAside => {
                    for name in ["indexes", "shielded_state"] {
                        std::fs::rename(folder.join(name), d.join(name)).unwrap();
                    }
                }
                // Some of the attempt's data is gone already.
                Phase::Undoing => {
                    attempt(d);
                    std::fs::remove_dir_all(d.join("blocks")).unwrap();
                }
                // Half-restored: the attempt's data is gone, three are back.
                Phase::Restoring => {
                    btx_core::node::begin_header_bootstrap(d);
                    for name in ["blocks", "chainstate", "snapshot-start.json"] {
                        std::fs::rename(folder.join(name), d.join(name)).unwrap();
                    }
                }
                _ => unreachable!(),
            }
            record_at(d, phase);
            let when = format!("{phase:?}");
            assert_eq!(at_start(d, 50_000, true), Ok(()), "{when}");
            assert_old_chain_back(d, &when);
            assert_eq!(run_settings(d), before, "{when}");
            assert!(snapshot_marker_present(d), "{when}");
            assert!(
                matches!(ff::read_outcome(d), Some(Outcome::RolledBack { .. })),
                "{when}"
            );
            assert_eq!(refuse_run(d), None, "{when}");
        }
    }

    /// Controller note 2: the reason a roll-back wrote before its restore
    /// was cut off is the one the window shows; a run cut off while setting
    /// aside, which wrote none, gets the reason `judge` gives.
    #[test]
    fn a_roll_back_cut_off_keeps_its_reason() {
        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        set_aside_for_run(d, 232_000, 100).unwrap();
        attempt(d);
        let real = Outcome::RolledBack {
            reason: WHY_NOT_LOADED.into(),
        };
        ff::write_outcome(d, &real);
        record_at(d, Phase::Undoing);
        assert_eq!(at_start(d, 50_000, true), Ok(()));
        assert_eq!(ff::read_outcome(d), Some(real));

        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        set_aside_for_run(d, 232_000, 100).unwrap();
        record_at(d, Phase::SettingAside);
        assert_eq!(at_start(d, 50_000, true), Ok(()));
        match ff::read_outcome(d) {
            Some(Outcome::RolledBack { reason }) => {
                assert!(reason.contains("setting the chain data aside"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    /// Controller note 1 (a0): a run at phase Running is watched again from
    /// this start, with a fresh window (time the app was closed does not
    /// count), and the attempt's chain and launches carry on.
    #[test]
    fn a_running_run_is_watched_again_from_this_start() {
        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        set_aside_for_run(d, 232_000, 100).unwrap();
        attempt(d);
        assert_eq!(at_start(d, 50_000, true), Ok(()));
        let r = ff::read_record(d).unwrap().unwrap();
        assert_eq!(r.phase, Phase::Running);
        assert_eq!(r.watch_started_at, 50_000);
        assert_eq!(r.started_at, 100);
        assert_eq!(
            std::fs::read_to_string(d.join("blocks/new")).unwrap(),
            "new"
        );
        assert!(launch_markers(d), "the start path carries on the run");
        assert!(underway(d));
        assert!(active_in(d));
    }

    /// Controller notes 1 (a0, c): a run recorded done is finished at the
    /// start: the new chain stays, the old one goes, nothing is left.
    #[test]
    fn a_done_run_is_finished_at_the_start() {
        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        set_aside_for_run(d, 232_000, 100).unwrap();
        attempt(d);
        record_at(d, Phase::Done);
        assert_eq!(at_start(d, 50_000, true), Ok(()));
        assert!(ff::read_record(d).unwrap().is_none());
        assert!(entries_named(d, "fast-forward-").is_empty());
        assert!(d.join("blocks/new").exists());
        assert!(!d.join("blocks/old").exists());
        assert!(!active_in(d));
    }

    /// Controller note 1 (c): the start sweeps dated folders no run needs,
    /// and nothing else.
    #[test]
    fn the_start_removes_old_chain_data_no_run_needs() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        for dir in [
            "fast-forward-7/blocks",
            "fast-forward-8.discard/chainstate",
            "chainstate_snapshot.refused-9/x",
            "blocks",
        ] {
            std::fs::create_dir_all(d.join(dir)).unwrap();
        }
        assert_eq!(at_start(d, 50_000, true), Ok(()));
        assert!(entries_named(d, "fast-forward-").is_empty());
        assert!(d.join("chainstate_snapshot.refused-9/x").exists());
        assert!(d.join("blocks").exists());

        // Remove node data sweeps them too, and says how much it freed.
        std::fs::create_dir_all(d.join("fast-forward-10/blocks")).unwrap();
        std::fs::write(d.join("fast-forward-10/blocks/old"), vec![0u8; 4096]).unwrap();
        assert!(sweep_measured(d) >= 4096);
        assert!(entries_named(d, "fast-forward-").is_empty());
    }

    /// Controller note 1 (a): during a run the loaders load again and it is
    /// no first load; a roll-back puts back both settings as they were
    /// before the run, whatever they were and whatever the attempt set.
    #[test]
    fn a_roll_back_puts_back_the_settings_from_before_the_run() {
        for (snapshot_loaded, first_load_pending) in
            [(true, false), (false, true), (false, false), (true, true)]
        {
            let before = Before {
                snapshot_loaded,
                first_load_pending,
            };
            let tmp = datadir_with_chain(before);
            let d = tmp.path();
            let record = set_aside_for_run(d, 232_000, 100).unwrap();
            assert_eq!(record.snapshot_loaded_before, snapshot_loaded);
            assert_eq!(record.first_load_pending_before, first_load_pending);
            assert_eq!(run_settings(d), Before::default(), "{before:?}, during");
            assert!(!snapshot_marker_present(d), "{before:?}, during");
            attempt(d);
            NodeAppSettings::update(d, |s| s.snapshot_loaded = true);
            mark_snapshot_marker(d);
            assert_eq!(undo(d), Ok(()), "{before:?}");
            assert_eq!(run_settings(d), before, "{before:?}");
            assert_eq!(snapshot_marker_present(d), snapshot_loaded, "{before:?}");
            assert_old_chain_back(d, &format!("{before:?}"));
        }
    }

    /// Controller notes 2 and 2b. A roll-back whose restore cannot begin
    /// (an original is missing from the dated folder) removes nothing and
    /// leaves the run as it is, its settings and launches included, and
    /// says where the old chain data is. One stopped after it began has put
    /// the settings from before the run back first, and says where the rest
    /// is. Neither lets the node start.
    #[test]
    fn a_roll_back_that_cannot_finish_says_where_the_old_chain_is() {
        let before = Before {
            snapshot_loaded: false,
            first_load_pending: true,
        };
        let tmp = datadir_with_chain(before);
        let d = tmp.path();
        let record = set_aside_for_run(d, 232_000, 100).unwrap();
        let folder = d.join(&record.aside);
        attempt(d);
        NodeAppSettings::update(d, |s| s.snapshot_loaded = true);
        std::fs::remove_dir_all(folder.join("indexes")).unwrap();
        ff::write_outcome(
            d,
            &Outcome::RolledBack {
                reason: WHY_NOT_LOADED.into(),
            },
        );
        let not_back = undo(d).unwrap_err();
        assert!(
            not_back.could_not_begin,
            "the one that keeps the node stopped"
        );
        let said = not_back.said;
        assert_eq!(said, stranded_sentence(&folder, true));
        assert_eq!(
            ff::read_outcome(d),
            None,
            "the decision goes with the undo that cannot happen: the next opening watches the \
             run again (controller note 2b)"
        );
        assert!(said.contains(&folder.display().to_string()), "{said}");
        assert_eq!(
            ff::read_record(d).unwrap().unwrap().phase,
            Phase::Running,
            "controller note 2b: the record stays at Running"
        );
        assert_eq!(
            run_settings(d),
            Before {
                snapshot_loaded: true,
                first_load_pending: false
            },
            "the run's own settings"
        );
        assert!(d.join("blocks/new").exists(), "nothing removed");
        assert!(launch_markers(d), "nothing removed");

        let tmp = datadir_with_chain(before);
        let d = tmp.path();
        let record = set_aside_for_run(d, 232_000, 100).unwrap();
        let folder = d.join(&record.aside);
        attempt(d);
        // Stopped part-way through the put-back, with the attempt's
        // `blocks`, `chainstate`, `chainstate_snapshot` and start record
        // still in the places the old ones go back to: those cannot go back.
        record_at(d, Phase::Restoring);
        let not_back = undo(d).unwrap_err();
        assert!(!not_back.could_not_begin, "starting again tries once more");
        let said = not_back.said;
        assert_eq!(said, stranded_sentence(&folder, false));
        assert_eq!(run_settings(d), before, "put back before the restore");
        assert_eq!(ff::read_record(d).unwrap().unwrap().phase, Phase::Restoring);
        assert_eq!(
            at_start(d, 50_000, true).map_err(|e| e.said),
            Err(said),
            "and the start stops too"
        );
    }

    /// Controller note 2c and `judge`'s follow-up: a run is kept only when
    /// the start record the load wrote says the operators confirmed the
    /// snapshot, at or above the run's height, and names the base block
    /// the node's snapshot chainstate is built on. The outcome then names
    /// who confirmed it, and `finish` removes the old chain.
    #[test]
    fn a_kept_run_says_who_confirmed_it_and_removes_the_old_chain() {
        let hash = "ab".repeat(32);
        let start = StartRecord {
            height: 232_500,
            block_hash: hash.clone(),
            source: StartSource::Confirmed,
            operators: vec!["Mende".into(), "jpp".into()],
        };
        assert_eq!(
            confirmed_start(232_000, Some(start.clone()), Some(&hash.to_uppercase())),
            Ok(start.clone())
        );
        let other = |f: fn(&mut StartRecord)| {
            let mut s = start.clone();
            f(&mut s);
            Some(s)
        };
        let cd = "cd".repeat(32);
        for (what, s, base) in [
            ("no start record", None, Some(hash.as_str())),
            (
                "pinned",
                other(|s| s.source = StartSource::Pinned),
                Some(hash.as_str()),
            ),
            (
                "engine",
                other(|s| s.source = StartSource::Engine),
                Some(hash.as_str()),
            ),
            (
                "below the run",
                other(|s| s.height = 231_999),
                Some(hash.as_str()),
            ),
            ("another base", Some(start.clone()), Some(cd.as_str())),
            ("no base", Some(start.clone()), None),
        ] {
            let why = confirmed_start(232_000, s, base).unwrap_err();
            assert!(
                !why.contains('\u{2014}') && !why.ends_with('.'),
                "{what}: {why}"
            );
        }

        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        set_aside_for_run(d, 232_000, 100).unwrap();
        attempt(d);
        keep(d, &start).unwrap();
        assert_eq!(
            ff::read_outcome(d),
            Some(Outcome::Done {
                height: 232_500,
                operators: vec!["Mende".into(), "jpp".into()],
            })
        );
        assert!(ff::read_record(d).unwrap().is_none());
        assert!(entries_named(d, "fast-forward-").is_empty());
        assert!(d.join("blocks/new").exists());
    }

    /// Controller note 1 (d): no run starts while a snapshot the app
    /// refused waits to be set aside (a roll-back would bring it back
    /// without its note), nor over a run that is recorded.
    #[test]
    fn a_run_waits_for_a_set_aside_note_and_for_the_last_run() {
        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        assert_eq!(refuse_run(d), None);
        assert!(!active_in(d));
        std::fs::write(d.join(crate::commands::SET_ASIDE_PENDING_FILE), b"{}").unwrap();
        assert_eq!(refuse_run(d).as_deref(), Some(NOTE_WAITS));
        std::fs::remove_file(d.join(crate::commands::SET_ASIDE_PENDING_FILE)).unwrap();
        set_aside_for_run(d, 232_000, 100).unwrap();
        assert_eq!(refuse_run(d).as_deref(), Some(NOT_FINISHED));
        assert!(active_in(d));
    }

    /// A set-aside note the attempt wrote names the attempt's chainstate,
    /// which the roll-back removes: it goes too, so the next launch does
    /// not set aside the chainstate that came back.
    #[test]
    fn a_roll_back_drops_the_set_aside_note_the_attempt_wrote() {
        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        set_aside_for_run(d, 232_000, 100).unwrap();
        attempt(d);
        std::fs::write(d.join(crate::commands::SET_ASIDE_PENDING_FILE), b"{}").unwrap();
        assert_eq!(undo(d), Ok(()));
        assert!(!d.join(crate::commands::SET_ASIDE_PENDING_FILE).exists());
    }

    /// Review, IMPORTANT: which start may launch btxd, for every combination.
    /// A roll-back that could not begin keeps every start off. While this
    /// app's driver is at work a start goes ahead only on chain data that is
    /// not moving: no run, a run under way, or one that is done; never while
    /// the chain data moves or is to be moved back, nor over a record nobody
    /// can read. With no driver the start carries the recorded run on first.
    #[test]
    fn the_start_gate_decides_every_combination() {
        let d = Path::new("/Users/someone/.easybtx");
        let records = [
            OnRecord::Nothing,
            OnRecord::Unreadable,
            OnRecord::At(Phase::SettingAside),
            OnRecord::At(Phase::Running),
            OnRecord::At(Phase::Undoing),
            OnRecord::At(Phase::Restoring),
            OnRecord::At(Phase::Done),
        ];
        let stuck = "the roll-back could not begin.";
        let mut seen = 0;
        for driving in [false, true] {
            for stuck in [None, Some(stuck)] {
                for record in records {
                    for rolling_back in [false, true] {
                        seen += 1;
                        let gate = start_gate(driving, stuck, record, rolling_back, d);
                        let want = if let Some(said) = stuck {
                            Gate::Refuse(said.into())
                        } else if !driving {
                            Gate::AtStart
                        } else {
                            match record {
                                OnRecord::Nothing | OnRecord::At(Phase::Done) => Gate::Start,
                                OnRecord::At(Phase::Running) if !rolling_back => Gate::Start,
                                OnRecord::Unreadable => Gate::Refuse(unreadable_sentence(d)),
                                _ => Gate::Refuse(MOVING.into()),
                            }
                        };
                        assert_eq!(
                            gate, want,
                            "driving {driving}, stuck {stuck:?}, {record:?}, rolling back \
                             {rolling_back}"
                        );
                    }
                }
            }
        }
        assert_eq!(seen, 56);
        // The ones the review named.
        for phase in [Phase::SettingAside, Phase::Undoing, Phase::Restoring] {
            assert_eq!(
                start_gate(true, None, OnRecord::At(phase), false, d),
                Gate::Refuse(MOVING.into()),
                "{phase:?}"
            );
        }
        assert_eq!(
            start_gate(true, Some(stuck), OnRecord::At(Phase::Running), false, d),
            Gate::Refuse(stuck.into())
        );
    }

    /// Controller note 1: Task 4's Tools status reads the phase before any
    /// outcome. `STUCK` always wins; then an unreadable record is the same
    /// kind of plain sentence. A run is "running" only while this app's
    /// driver is at work: before it has written a record (checking and
    /// downloading), and at every phase after. With no driver at work
    /// nothing moves the run on until the next start (review I4): a record
    /// at any phase then (a put-back that stopped part-way, a set-aside that
    /// could not put back, a roll-back decided beside a `Running` record, a
    /// start that failed before the watch resumed) says so, never
    /// "running"; with no record either, the last outcome speaks.
    #[test]
    fn the_tools_phase_decides_every_combination() {
        let d = Path::new("/Users/someone/.easybtx");
        let stuck = "the roll-back could not begin.";
        let records = [
            OnRecord::Nothing,
            OnRecord::Unreadable,
            OnRecord::At(Phase::SettingAside),
            OnRecord::At(Phase::Running),
            OnRecord::At(Phase::Undoing),
            OnRecord::At(Phase::Restoring),
            OnRecord::At(Phase::Done),
        ];
        let mut seen = 0;
        for driving in [false, true] {
            for stuck in [None, Some(stuck)] {
                for record in records {
                    seen += 1;
                    let want = match (stuck, record) {
                        (Some(said), _) => ToolsPhase::Halted(said.into()),
                        (None, OnRecord::Unreadable) => ToolsPhase::Halted(unreadable_sentence(d)),
                        (None, _) if driving => ToolsPhase::Running,
                        (None, OnRecord::Nothing) => ToolsPhase::Idle,
                        (None, OnRecord::At(_)) => ToolsPhase::Halted(NOT_FINISHED.into()),
                    };
                    assert_eq!(
                        tools_phase(driving, stuck, record, d),
                        want,
                        "driving {driving}, stuck {stuck:?}, {record:?}"
                    );
                }
            }
        }
        assert_eq!(seen, 28);
        // The ones the review named: no driver, and the run stuck part-way
        // or waiting for the next start.
        for phase in [
            Phase::SettingAside,
            Phase::Running,
            Phase::Undoing,
            Phase::Restoring,
        ] {
            assert_eq!(
                tools_phase(false, None, OnRecord::At(phase), d),
                ToolsPhase::Halted(NOT_FINISHED.into()),
                "{phase:?}"
            );
            assert_eq!(
                tools_phase(true, None, OnRecord::At(phase), d),
                ToolsPhase::Running,
                "{phase:?}, the driver at work"
            );
        }
        assert_eq!(
            tools_phase(true, None, OnRecord::Nothing, d),
            ToolsPhase::Running,
            "checking and downloading, before a record exists"
        );
    }

    /// Review, minor 2: a roll-back is decided for good when its reason is
    /// written, since a run clears the outcome before it sets anything
    /// aside. A Running record beside a roll-back's outcome (the app was cut
    /// off, or could not stop the node, before the restore began) is rolled
    /// back at the next start, before the launch when the node is down, and
    /// by the watch's first look when a node is up. Never watched as if
    /// nothing had been decided.
    #[test]
    fn a_roll_back_decided_before_a_crash_is_carried_out_at_the_next_start() {
        let before = Before {
            snapshot_loaded: true,
            first_load_pending: false,
        };
        let decided = Outcome::RolledBack {
            reason: WHY_NOT_LOADED.into(),
        };
        let tmp = datadir_with_chain(before);
        let d = tmp.path();
        set_aside_for_run(d, 232_000, 100).unwrap();
        attempt(d);
        ff::write_outcome(d, &decided);
        assert_eq!(at_start(d, 50_000, true), Ok(()));
        assert_old_chain_back(d, "node down");
        assert_eq!(run_settings(d), before);
        assert_eq!(
            ff::read_outcome(d),
            Some(decided.clone()),
            "the real reason"
        );

        let tmp = datadir_with_chain(before);
        let d = tmp.path();
        set_aside_for_run(d, 232_000, 100).unwrap();
        attempt(d);
        ff::write_outcome(d, &decided);
        assert_eq!(at_start(d, 50_000, false), Ok(()), "a node is up");
        assert!(underway(d), "left for the watch, which can stop the node");
        let record = ff::read_record(d).unwrap().unwrap();
        let on_the_snapshot = Look {
            snapshot_base_height: Some(232_000),
            running: true,
            loaded: true,
            ..Look::default()
        };
        assert_eq!(
            verdict(&record, &on_the_snapshot, ff::read_outcome(d), 50_005),
            Verdict::RollBack(WHY_NOT_LOADED.into()),
            "however the node looks"
        );
        // Without that outcome the watch judges as always; a run judged
        // done whose finish was cut off is judged done again.
        assert_eq!(
            verdict(&record, &on_the_snapshot, None, 50_005),
            Verdict::Done
        );
        let done = Outcome::Done {
            height: 232_000,
            operators: vec![],
        };
        assert_eq!(
            verdict(&record, &on_the_snapshot, Some(done), 50_005),
            Verdict::Done
        );
    }

    /// Controller note 2b with the decision on disk: a roll-back whose
    /// restore cannot begin at the start keeps the node from starting in
    /// this run of the app, and drops the decision, so the next opening
    /// watches the intact new chain again rather than failing the same way.
    ///
    /// This test sets and clears the global [`STUCK`], which [`before_start`]
    /// and [`tools_status_phase`] read: it must stay the only test that
    /// calls `before_start`, or tests running beside it would see its value.
    /// Reading it back here, at points this test's own actions make certain,
    /// is safe.
    #[tokio::test]
    async fn a_roll_back_that_cannot_begin_keeps_the_node_stopped_until_the_next_opening() {
        // Review minor 1: a put-back that began and stopped keeps this start
        // off, but not the next: starting again tries once more.
        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        let record = set_aside_for_run(d, 232_000, 100).unwrap();
        let folder = d.join(&record.aside);
        attempt(d);
        record_at(d, Phase::Restoring);
        let said = stranded_sentence(&folder, false);
        assert_eq!(before_start(d).await, Err(said));
        assert_eq!(
            *STUCK.lock().unwrap(),
            None,
            "not the one that could not begin"
        );
        // Task 4's Tools status: no driver is at work and the put-back
        // waits for the next start, so never "running" (review I4).
        assert_eq!(
            tools_status_phase(d),
            ToolsPhase::Halted(NOT_FINISHED.into())
        );
        // Clear what stood in the way: the attempt's entries in the places
        // the old ones go back to.
        for name in ["blocks", "chainstate", "chainstate_snapshot"] {
            std::fs::remove_dir_all(d.join(name)).unwrap();
        }
        std::fs::remove_file(d.join("snapshot-start.json")).unwrap();
        assert_eq!(before_start(d).await, Ok(()), "tried once more");
        assert_old_chain_back(d, "tried once more");
        assert_eq!(
            tools_status_phase(d),
            ToolsPhase::Idle,
            "put back, nothing left recorded"
        );

        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        let record = set_aside_for_run(d, 232_000, 100).unwrap();
        let folder = d.join(&record.aside);
        attempt(d);
        ff::write_outcome(
            d,
            &Outcome::RolledBack {
                reason: WHY_NOT_LOADED.into(),
            },
        );
        std::fs::remove_dir_all(folder.join("indexes")).unwrap();
        let said = stranded_sentence(&folder, true);
        assert_eq!(before_start(d).await, Err(said.clone()));
        assert_eq!(ff::read_outcome(d), None);
        assert!(underway(d));
        // Task 4's Tools status: STUCK wins over the record that is still
        // there, and shows the driver's own sentence, never "running".
        assert_eq!(tools_status_phase(d), ToolsPhase::Halted(said.clone()));
        // Put the missing part back: this run of the app still does not
        // start the node.
        std::fs::create_dir_all(folder.join("indexes")).unwrap();
        assert_eq!(before_start(d).await, Err(said));
        *STUCK.lock().unwrap() = None;
        assert_eq!(before_start(d).await, Ok(()), "the next opening");
        assert!(underway(d), "watched again");
        assert_eq!(
            tools_status_phase(d),
            ToolsPhase::Halted(NOT_FINISHED.into()),
            "until the watch resumes, once the node is up"
        );
    }

    /// Review I2: the watch reads the one fact the loader sets once its
    /// check after the load has passed. The run resets it before anything
    /// moves, and puts it back when nothing moved after all; the loader's
    /// mark sets it; a roll-back puts back what it was before the run.
    #[test]
    fn the_watch_waits_for_the_loaders_own_word() {
        use btx_core::snapshot::SnapshotFlags as _;
        let before = Before {
            snapshot_loaded: true,
            first_load_pending: false,
        };
        let tmp = datadir_with_chain(before);
        let d = tmp.path();
        assert!(loaded_by_the_loader(d), "loaded before the run");
        // A dated folder in the way: nothing moves, and nothing changes.
        std::fs::write(d.join(ff::aside_name(100)), b"in the way").unwrap();
        assert!(matches!(
            set_aside_for_run(d, 232_000, 100),
            Err(MoveError::Untouched(_))
        ));
        assert_eq!(run_settings(d), before);
        assert!(snapshot_marker_present(d));
        std::fs::remove_file(d.join(ff::aside_name(100))).unwrap();

        set_aside_for_run(d, 232_000, 100).unwrap();
        assert!(!loaded_by_the_loader(d), "reset by the run");
        attempt(d);
        crate::state::NodeAppSnapshotFlags {
            datadir: d.to_path_buf(),
            run: None,
        }
        .mark_loaded();
        assert!(loaded_by_the_loader(d), "the loader's word");
        assert_eq!(undo(d), Ok(()));
        assert!(loaded_by_the_loader(d), "as before the run");
    }

    /// Review, minor 3: a set-aside note written between the run's check
    /// and the move (a load refused meanwhile) stops the move, with the node
    /// stopped and the datadir to itself, and nothing changes.
    #[test]
    fn a_set_aside_note_that_appears_before_the_move_stops_it() {
        let before = Before {
            snapshot_loaded: true,
            first_load_pending: false,
        };
        let tmp = datadir_with_chain(before);
        let d = tmp.path();
        std::fs::write(d.join(crate::commands::SET_ASIDE_PENDING_FILE), b"{}").unwrap();
        assert!(matches!(
            set_aside_for_run(d, 232_000, 100),
            Err(MoveError::Untouched(_))
        ));
        assert!(ff::read_record(d).unwrap().is_none());
        assert!(d.join("blocks/old").exists());
        assert!(entries_named(d, "fast-forward-").is_empty());
        assert_eq!(run_settings(d), before);
        assert!(snapshot_marker_present(d));
        let other = std::io::Error::other("x");
        assert_eq!(why_not_set_aside(d, &other), WHY_NOTE_APPEARED);
        std::fs::remove_file(d.join(crate::commands::SET_ASIDE_PENDING_FILE)).unwrap();
        assert_eq!(why_not_set_aside(d, &other), WHY_NOT_SET_ASIDE);
    }

    /// Review, minor 4: "remove what the attempt made" includes the
    /// snapshot chainstates the attempt's loads were refused and set aside
    /// (named after the time, at or after the run began). Older ones, and
    /// anything else, are left to the weekly sweep.
    #[test]
    fn a_roll_back_removes_the_refused_chainstates_the_attempt_made() {
        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        let prefix = btx_core::confirmed_load::REFUSED_CHAINSTATE_PREFIX;
        std::fs::create_dir_all(d.join(format!("{prefix}99/x"))).unwrap();
        set_aside_for_run(d, 232_000, 100).unwrap();
        attempt(d);
        for t in ["100", "5000"] {
            std::fs::create_dir_all(d.join(format!("{prefix}{t}/x"))).unwrap();
        }
        std::fs::create_dir_all(d.join(format!("{prefix}later/x"))).unwrap();
        std::fs::write(d.join(format!("{prefix}6000")), b"a file").unwrap();
        assert_eq!(undo(d), Ok(()));
        assert_eq!(
            entries_named(d, prefix),
            [
                format!("{prefix}6000"),
                format!("{prefix}99"),
                format!("{prefix}later"),
            ]
        );
        assert_old_chain_back(d, "after the refused chainstates");
    }

    /// What [`a_move_waits_for_the_engines_lock`] runs in another process:
    /// this test binary again, with only this test. It takes the engine's
    /// lock on `EASYNODE_FF_LOCK_DIR` and holds it until `release` appears
    /// in `EASYNODE_FF_SIGNALS` (a bounded wait), writing `held` there once
    /// it has it. Run on its own, with no folder named, it does nothing.
    #[test]
    fn engine_lock_holder() {
        let (Some(dir), Some(signals)) = (
            std::env::var_os("EASYNODE_FF_LOCK_DIR"),
            std::env::var_os("EASYNODE_FF_SIGNALS"),
        ) else {
            return;
        };
        let signals = PathBuf::from(signals);
        let lock = btx_core::fsx::EngineLock::take(Path::new(&dir)).unwrap();
        std::fs::write(signals.join("held"), b"").unwrap();
        for _ in 0..600 {
            if signals.join("release").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        drop(lock);
    }

    /// Another process holding the engine's lock on `dir`, as a btxd does,
    /// until [`Holder::release`].
    struct Holder {
        child: std::process::Child,
        signals: tempfile::TempDir,
    }

    impl Holder {
        fn on(dir: &Path) -> Self {
            let signals = tempfile::tempdir().unwrap();
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "fast_forward::tests::engine_lock_holder",
                    "--nocapture",
                ])
                .env("EASYNODE_FF_LOCK_DIR", dir)
                .env("EASYNODE_FF_SIGNALS", signals.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let held = signals.path().join("held");
            for _ in 0..600 {
                if held.exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            assert!(held.exists(), "the holder took the lock");
            Holder { child, signals }
        }

        fn release(mut self) {
            std::fs::write(self.signals.path().join("release"), b"").unwrap();
            assert!(self.child.wait().unwrap().success());
        }
    }

    /// Review I5: every move of chain data holds the engine's own lock on
    /// the folder, so no btxd, whichever app launches it, starts on it
    /// meanwhile; and one that cannot be had means a node holds the folder,
    /// so nothing moves. Setting aside says the node did not stop; a
    /// roll-back leaves the run as it is and says so in the plain sentence.
    #[test]
    fn a_move_waits_for_the_engines_lock() {
        let before = Before {
            snapshot_loaded: true,
            first_load_pending: false,
        };
        let tmp = datadir_with_chain(before);
        let d = tmp.path();
        let holder = Holder::on(d);
        let e = match set_aside_for_run(d, 232_000, 100) {
            Err(MoveError::Untouched(e)) => e,
            other => panic!("{other:?}"),
        };
        assert_eq!(why_not_set_aside(d, &e), WHY_NOT_STOPPED);
        assert!(ff::read_record(d).unwrap().is_none(), "nothing recorded");
        assert!(d.join("blocks/old").exists(), "nothing moved");
        assert!(entries_named(d, "fast-forward-").is_empty());
        assert_eq!(run_settings(d), before, "nothing changed");
        holder.release();

        let record = set_aside_for_run(d, 232_000, 100).unwrap();
        attempt(d);
        let holder = Holder::on(d);
        assert_eq!(undo(d), Err(NotBack::plain(NOT_STOPPED)));
        assert_eq!(ff::read_record(d).unwrap(), Some(record), "the run stands");
        assert!(d.join("blocks/new").exists(), "nothing removed");
        assert_eq!(run_settings(d), Before::default(), "the run's own");
        holder.release();
        assert_eq!(undo(d), Ok(()));
        assert_old_chain_back(d, "once the lock was free");
        assert_eq!(run_settings(d), before);
    }

    /// Review M6: Remove node data goes ahead, with no driver at work, when
    /// no run is recorded, and when the one recorded is at Running and its
    /// roll-back could not begin ([`STUCK`]): its old chain cannot come
    /// back, and the node stays stopped on the new chain. Never while a
    /// driver is at work, over a record nobody can read, or over a run at
    /// any other phase.
    #[test]
    fn remove_node_data_waits_except_for_a_run_whose_old_chain_cannot_come_back() {
        let records = [
            OnRecord::Nothing,
            OnRecord::Unreadable,
            OnRecord::At(Phase::SettingAside),
            OnRecord::At(Phase::Running),
            OnRecord::At(Phase::Undoing),
            OnRecord::At(Phase::Restoring),
            OnRecord::At(Phase::Done),
        ];
        let goes_ahead = [
            (false, false, OnRecord::Nothing),
            (false, true, OnRecord::Nothing),
            (false, true, OnRecord::At(Phase::Running)),
        ];
        let mut seen = 0;
        for driving in [false, true] {
            for stuck in [false, true] {
                for record in records {
                    seen += 1;
                    assert_eq!(
                        removal_goes_ahead(driving, stuck, record),
                        goes_ahead.contains(&(driving, stuck, record)),
                        "driving {driving}, stuck {stuck}, {record:?}"
                    );
                }
            }
        }
        assert_eq!(seen, 28);
    }

    /// Review M6, on disk: a roll-back whose restore could not begin leaves
    /// the run at Running, and the node stopped. Remove node data gives the
    /// run up (`ff::abandon`): its record goes, starts are no longer held off
    /// for it, and the sweep takes its dated folder. A run whose old chain
    /// can still come back, or any run while the driver is at work, keeps
    /// Remove node data waiting, and nothing changes.
    #[test]
    fn remove_node_data_gives_up_a_run_whose_old_chain_cannot_come_back() {
        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        let record = set_aside_for_run(d, 232_000, 100).unwrap();
        let folder = d.join(&record.aside);
        attempt(d);
        let waits = Err(REMOVE_WAITS.to_string());
        let nothing = Mutex::new(None);
        assert_eq!(clear_for_removal_with(d, false, &nothing), waits);
        std::fs::remove_dir_all(folder.join("indexes")).unwrap();
        let not_back = undo(d).unwrap_err();
        assert!(not_back.could_not_begin);
        let stuck = Mutex::new(Some(not_back.said.clone()));
        assert_eq!(clear_for_removal_with(d, true, &stuck), waits, "a driver");
        assert!(underway(d), "nothing changed");
        assert_eq!(clear_for_removal_with(d, false, &stuck), Ok(()));
        assert_eq!(*stuck.lock().unwrap(), None, "starts may run again");
        assert!(ff::read_record(d).unwrap().is_none());
        assert!(!active_in(d));
        assert!(sweep_measured(d) > 0);
        assert!(entries_named(d, "fast-forward-").is_empty());
        assert!(
            d.join("blocks/new").exists(),
            "the rest is Remove node data's"
        );

        // The missing part came back meanwhile: the run can be rolled back
        // again at the next opening, so it is not given up.
        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        let record = set_aside_for_run(d, 232_000, 100).unwrap();
        let folder = d.join(&record.aside);
        attempt(d);
        std::fs::rename(folder.join("indexes"), d.join("indexes-elsewhere")).unwrap();
        let not_back = undo(d).unwrap_err();
        assert!(not_back.could_not_begin);
        std::fs::rename(d.join("indexes-elsewhere"), folder.join("indexes")).unwrap();
        let stuck = Mutex::new(Some(not_back.said.clone()));
        assert_eq!(clear_for_removal_with(d, false, &stuck), waits);
        assert_eq!(*stuck.lock().unwrap(), Some(not_back.said));
        assert!(underway(d));
        assert!(folder.join("indexes/old").exists());
    }

    /// Review I3: the snapshot must still be above the node's tip once step
    /// 1 is over, or nothing moves; a node that does not say is not moved
    /// either.
    #[test]
    fn a_node_that_caught_up_meanwhile_is_not_moved() {
        assert_eq!(still_ahead(233_800, Some(232_000)), Ok(()));
        assert_eq!(still_ahead(233_800, Some(233_799)), Ok(()), "one block");
        assert_eq!(still_ahead(233_800, Some(233_800)), Err(WHY_CAUGHT_UP));
        assert_eq!(still_ahead(233_800, Some(240_000)), Err(WHY_CAUGHT_UP));
        assert_eq!(still_ahead(233_800, None), Err(WHY_NO_TIP));
    }

    /// Review, minor 1: the driver's own start, made just after it lets the
    /// start guard go, can meet a start someone else began that moment. That
    /// one is as good (it launches the same node), so it is not a failure to
    /// roll back on; any other error is.
    #[test]
    fn a_start_already_under_way_counts_as_the_drivers_own() {
        assert!(started_elsewhere(crate::commands::ALREADY_STARTING));
        assert!(!started_elsewhere("couldn't start the node: no btxd"));
        assert!(!started_elsewhere(MOVING));
    }

    /// Review I1, in the code: Tools' check and step 1 judge the confirmed
    /// snapshot with the view the run's load will have (`run_view`, whose
    /// pins `confirmed_load::launch_view`'s own test holds for both host
    /// kinds), never with the running engine's pins alone.
    #[test]
    fn the_check_and_step_one_judge_with_the_pins_of_the_runs_load() {
        let body = |src: &'static str, start: &str, end: &str| -> &'static str {
            src.split(start)
                .nth(1)
                .and_then(|s| s.split(end).next())
                .unwrap()
        };
        let check = body(
            include_str!("tools.rs"),
            "pub async fn tools_fast_forward_check(",
            "\npub async fn tools_fast_forward_run(",
        );
        let prepare = body(
            include_str!("fast_forward.rs"),
            "\nasync fn prepare(",
            "\nasync fn run(",
        );
        for (what, src) in [("the check", check), ("step 1", prepare)] {
            assert!(
                src.contains("run_view(&rpc") || src.contains("run_view(rpc"),
                "{what}"
            );
            assert!(!src.contains("node_view("), "{what}");
        }
    }

    /// Everything the driver says: plain, no em-dash; the sentences end in
    /// a full stop, the reasons fit `copy::rolled_back`, and a folder the
    /// old chain data is in is named.
    #[test]
    fn what_the_driver_says_is_plain() {
        let folder = Path::new("/Users/someone/.easybtx/fast-forward-100");
        let stranded = [
            stranded_sentence(folder, true),
            stranded_sentence(folder, false),
        ];
        for s in &stranded {
            assert!(
                s.contains("/Users/someone/.easybtx/fast-forward-100"),
                "{s}"
            );
        }
        // Controller note 2b: one plain sentence, like the unreadable one.
        for s in [
            &stranded[0],
            &unreadable_sentence(Path::new("/Users/someone/.easybtx")),
        ] {
            assert_eq!(s.matches(". ").count(), 0, "one sentence: {s}");
        }
        let sentences = [
            ALREADY_RUNNING.to_string(),
            NOT_FINISHED.to_string(),
            NOTE_WAITS.to_string(),
            REMOVE_WAITS.to_string(),
            NODE_IN_THE_WAY.to_string(),
            NOT_STOPPED.to_string(),
            UNDO_FAILED.to_string(),
            MOVE_CUT_OFF.to_string(),
            MOVING.to_string(),
            unreadable_sentence(Path::new("/Users/someone/.easybtx")),
        ];
        for s in sentences.iter().chain(&stranded) {
            assert!(!s.contains('\u{2014}'), "no em-dash: {s}");
            assert!(s.ends_with('.'), "{s}");
        }
        for why in [
            WHY_NOT_OURS,
            WHY_NOT_RUNNING,
            WHY_NOT_CHECKED,
            WHY_STARTING,
            WHY_NOT_STOPPED,
            WHY_NOT_SET_ASIDE,
            WHY_NOT_STARTED,
            WHY_CAUGHT_UP,
            WHY_NO_TIP,
            WHY_NOT_LOADED,
            WHY_REFUSED,
            WHY_NOTE_APPEARED,
            WHY_NOT_CLEARED,
        ] {
            assert!(!why.contains('\u{2014}') && !why.ends_with('.'), "{why}");
            assert!(why.starts_with(|c: char| c.is_lowercase()), "{why}");
            let shown = btx_core::fast_forward::copy::rolled_back(why);
            assert!(shown.ends_with(&format!("{why}.")), "{shown}");
        }
    }
}
