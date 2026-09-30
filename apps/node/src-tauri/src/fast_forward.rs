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
//! * [`before_start`]: at every start while no driver is at work, the sweep,
//!   then the run a previous start left, by its phase. The node is launched
//!   only once any undo it needs returned `Ok`.
//! * [`resume_if_needed`]: a run the app was closed on is watched again.
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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use btx_core::fast_forward::{
    self as ff, Before, Look, MoveError, Outcome, Phase, Record, Verdict,
};
use btx_core::snapshot_start::{StartRecord, StartSource};
use tauri::{AppHandle, Manager, State};

use crate::commands::{
    destructive_allowed, node_ownership, rpc_already_answering, set_phase, setup_log,
    snapshot_spec, start_node_projected, stop_node_inner, SET_ASIDE_PENDING_FILE,
    SIGNED_LOAD_FAILED,
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
pub(crate) const NOT_FINISHED: &str =
    "A Fast-forward has not finished yet. easyNode carries it on when it next starts the node.";
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
     so it left everything as it is. It carries on the next time easyNode starts the node.";
const UNDO_FAILED: &str = "Fast-forward could not put the old chain data back, so easyNode has \
     not started the node. Start the node again to try once more.";
const MOVE_CUT_OFF: &str = "Fast-forward stopped unexpectedly while it was moving the chain \
     data. Start the node again, and easyNode sorts it out first.";

const WHY_NOT_OURS: &str = "another app is running the node in this data folder";
const WHY_NOT_RUNNING: &str = "the node was not running";
const WHY_NOT_CHECKED: &str = "the confirmed snapshot could not be downloaded and checked";
const WHY_STARTING: &str = "the node was starting or restarting";
const WHY_NOT_STOPPED: &str = "the node did not stop";
const WHY_NOT_SET_ASIDE: &str = "the chain data could not be set aside";
const WHY_NOT_STARTED: &str = "the node did not start on the new chain data";
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
/// from the dated folder), so the run stands as it was, and the node is not
/// started again in this run of the app (controller note 2b).
fn stranded_sentence(folder: &Path, nothing_removed: bool) -> String {
    if nothing_removed {
        format!(
            "Fast-forward could not put the old chain data back because part of it is missing \
             from {}, so easyNode leaves the node stopped until it is opened again.",
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
/// started then, and Remove node data waits.
pub(crate) fn active() -> bool {
    active_in(&node_datadir())
}

fn active_in(datadir: &Path) -> bool {
    DRIVING.load(Ordering::SeqCst) || !matches!(ff::read_record(datadir), Ok(None))
}

/// The node runs on a run's new chain data (its record is at
/// [`Phase::Running`]): its loads are signed-only, one that fails goes to
/// the driver, and it is no first load. For the start path.
pub(crate) fn underway(datadir: &Path) -> bool {
    matches!(ff::read_record(datadir), Ok(Some(r)) if r.phase == Phase::Running)
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

/// At a start, before anything reads the chain data. While this app's
/// driver is at work it has done what a start needs, and it holds starts
/// while chain data moves, so nothing then. Otherwise: the sweep, then the
/// run a previous start left, by its phase ([`at_start`]). An `Err` is a
/// plain sentence, and the node is not started.
pub(crate) async fn before_start(datadir: &Path) -> Result<(), String> {
    if DRIVING.load(Ordering::SeqCst) {
        return Ok(());
    }
    let stuck = STUCK.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(said) = stuck {
        return Err(said);
    }
    let undoes = matches!(
        ff::read_record(datadir),
        Ok(Some(r)) if matches!(r.phase, Phase::SettingAside | Phase::Undoing | Phase::Restoring)
    );
    if undoes && !node_is_down(datadir).await {
        log(
            datadir,
            "a run is being undone, and a node this app did not start is using the data folder; \
             not starting",
        );
        return Err(NODE_IN_THE_WAY.into());
    }
    let dd = datadir.to_path_buf();
    on_disk(move || at_start(&dd, now()))
        .await
        .unwrap_or_else(|| Err(MOVE_CUT_OFF.into()))
}

/// [`before_start`]'s part on disk, with the node stopped. The sweep first;
/// then a run at [`Phase::Running`] gets a fresh watch window and carries on,
/// one at [`Phase::Done`] is finished, and one cut off while its chain data
/// moved ([`Phase::SettingAside`], [`Phase::Undoing`], [`Phase::Restoring`])
/// is rolled back here, before any launch. A record nobody can read stops
/// the start and nothing is touched.
fn at_start(datadir: &Path, now_unix: u64) -> Result<(), String> {
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
            return Err(unreadable_sentence(datadir));
        }
    };
    match record.phase {
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
/// it (`ff::set_aside`). Then "snapshot loaded" is reset, so the loaders load
/// again, and "first load still to come" too: a run is no first load
/// (controller note 1 (a)), and the start path keeps it so while the run is
/// under way.
fn set_aside_for_run(datadir: &Path, height: u64, now_unix: u64) -> Result<Record, MoveError> {
    let record = ff::set_aside(datadir, height, run_settings(datadir), now_unix)?;
    put_settings(datadir, Before::default());
    Ok(record)
}

/// Put the old chain data back, with the node stopped. The settings from
/// before the run go back first, from the record, so a crash in the restore
/// cannot lose them, and again once it returned `Ok` (controller notes 2 and
/// 2c). Then the markers of the attempt's launches go (a mirror launch or a
/// header bootstrap on the old chain would be wrong), and so does a
/// set-aside note the attempt wrote: a run never starts while one waits, so
/// it names the attempt's chainstate. `Err` is a plain sentence: the old
/// chain data is not all back, and the node must not start.
fn undo(datadir: &Path) -> Result<(), String> {
    let record = match ff::read_record(datadir) {
        Ok(Some(r)) => r,
        Ok(None) => return Ok(()),
        Err(e) => {
            log(datadir, &format!("the run's record cannot be read ({e})"));
            return Err(unreadable_sentence(datadir));
        }
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
            } else {
                btx_core::node::end_mirror_load(datadir);
                btx_core::node::end_header_bootstrap(datadir);
            }
            Err(match e {
                MoveError::Stranded { folder, .. } => stranded_sentence(&folder, nothing_removed),
                MoveError::Untouched(_) => UNDO_FAILED.into(),
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

/// A run that ended before anything moved: why, for the Tools status.
async fn not_started(datadir: &Path, why: &str) {
    log(datadir, &format!("not started: {why}"));
    let (dd, reason) = (datadir.to_path_buf(), why.to_string());
    on_disk(move || ff::write_outcome(&dd, &Outcome::RolledBack { reason })).await;
}

/// Step 1: the confirmed pair, checked as the loader checks it and on disk.
/// A disputed `latest` stops here. The error is for the log.
async fn prepare(
    rpc: &btx_core::rpc::RpcClient,
    datadir: &Path,
) -> Result<btx_core::attested_snapshot::ReadyPair, String> {
    use btx_core::attested_snapshot as attested;
    let anchor = snapshot_spec().anchor_height;
    let view = btx_core::confirmed_load::node_view(
        rpc,
        &btx_core::node::BTX_TRUSTED_ATTESTATION_PUBKEYS,
        attested::fallback_start(anchor),
    )
    .await;
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

async fn run(app: &AppHandle, state: &State<'_, AppState>) {
    let datadir = node_datadir();
    let dd = datadir.clone();
    on_disk(move || ff::clear_outcome(&dd)).await;
    take_failure();
    // A person asked for this: a load that failed or was refused earlier in
    // this run of the app does not stand in its way (a validating node's
    // mirror launch waits on both). During the run a failure goes to the
    // driver, not to the start path's restarts, so these still bound them.
    SIGNED_LOAD_FAILED.store(false, Ordering::SeqCst);
    state.load_failure_restarted.store(false, Ordering::SeqCst);

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
        let _ = start_node_projected(app, state).await;
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
            not_started(&datadir, WHY_NOT_SET_ASIDE).await;
            let _ = start_node_projected(app, state).await;
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
    drop(no_starts);

    // Step 3: start. The start path does the rest: the header bootstrap of
    // the empty datadir, a validating node's one mirror launch, the load
    // with every check (signed-only during a run), the restart as a
    // validating node, and a failed load handed to this driver.
    if let Err(e) = start_node_projected(app, state).await {
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
    };
    (look, base)
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
        match ff::judge(&record, &look, now()) {
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
/// (controller note 2). The node starts again only once [`undo`] returned
/// `Ok`; before that the window says, in plain words, where the old chain
/// data is.
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
        drop(no_starts);
        log(
            &datadir,
            "the node did not stop, so the old chain data was not put back",
        );
        let message = NOT_STOPPED.to_string();
        set_phase(app, state, NodePhase::Error { message }).await;
        return;
    }
    let dd = datadir.clone();
    let undone = on_disk(move || undo(&dd))
        .await
        .unwrap_or_else(|| Err(MOVE_CUT_OFF.into()));
    drop(no_starts);
    match undone {
        Ok(()) => {
            if !state.quitting.load(Ordering::SeqCst) {
                let _ = start_node_projected(app, state).await;
            }
        }
        Err(message) => {
            if underway(&datadir) {
                *STUCK.lock().unwrap_or_else(|e| e.into_inner()) = Some(message.clone());
            }
            set_phase(app, state, NodePhase::Error { message }).await;
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

        let said = at_start(d, 1_000).unwrap_err();
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
            assert_eq!(at_start(d, 50_000), Ok(()), "{when}");
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
        assert_eq!(at_start(d, 50_000), Ok(()));
        assert_eq!(ff::read_outcome(d), Some(real));

        let tmp = datadir_with_chain(Before::default());
        let d = tmp.path();
        set_aside_for_run(d, 232_000, 100).unwrap();
        record_at(d, Phase::SettingAside);
        assert_eq!(at_start(d, 50_000), Ok(()));
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
        assert_eq!(at_start(d, 50_000), Ok(()));
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
        assert_eq!(at_start(d, 50_000), Ok(()));
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
        assert_eq!(at_start(d, 50_000), Ok(()));
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
        let said = undo(d).unwrap_err();
        assert_eq!(said, stranded_sentence(&folder, true));
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
        // Stopped part-way through the put-back, with `blocks` in its
        // place and in the dated folder: it cannot go back.
        record_at(d, Phase::Restoring);
        let said = undo(d).unwrap_err();
        assert_eq!(said, stranded_sentence(&folder, false));
        assert_eq!(run_settings(d), before, "put back before the restore");
        assert_eq!(ff::read_record(d).unwrap().unwrap().phase, Phase::Restoring);
        assert_eq!(at_start(d, 50_000), Err(said), "and the start stops too");
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
        let sentences = [
            ALREADY_RUNNING.to_string(),
            NOT_FINISHED.to_string(),
            NOTE_WAITS.to_string(),
            REMOVE_WAITS.to_string(),
            NODE_IN_THE_WAY.to_string(),
            NOT_STOPPED.to_string(),
            UNDO_FAILED.to_string(),
            MOVE_CUT_OFF.to_string(),
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
            WHY_NOT_LOADED,
            WHY_REFUSED,
        ] {
            assert!(!why.contains('\u{2014}') && !why.ends_with('.'), "{why}");
            assert!(why.starts_with(|c: char| c.is_lowercase()), "{why}");
            let shown = btx_core::fast_forward::copy::rolled_back(why);
            assert!(shown.ends_with(&format!("{why}.")), "{shown}");
        }
    }
}
