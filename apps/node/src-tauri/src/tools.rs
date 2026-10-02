//! Tools: one overlay with quick actions, Copy diagnostics and the command
//! window. Every decision is a pure function in btx-core; this module only
//! gathers answers from the node and hands them over.
//! docs/decisions/2026-09-29-tools-and-command-window.md

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use btx_core::console_policy::{self, ConfirmBook, Decision};
use btx_core::diagnostics::{self, DiagnosticsInput, HeldBranchState, RedactionContext};
use btx_core::engine_warnings::Notice;
use btx_core::error::AppError;
use btx_core::header_path::{HeaderPath, PathStatus};
use btx_core::node_api as api;
use btx_core::rpc::{Rpc, RpcClient};
use btx_core::stuck_blocks::{self, FetchPlan};

use crate::ask::{degrade, Ask};
use crate::commands::{
    destructive_allowed, mirror_launch_available, node_ownership, restart_node_projected,
};
use crate::state::{node_datadir, AppState, NodePhase};

/// Hosts this app itself talks to that are not node peers: the update feed
/// and release host, and the block explorer API. Diagnostics must never
/// redact these out from under `published_peer_hosts`, or a report that
/// mentions them (e.g. an update-check URL) shreds the app's own name.
const APP_SERVICE_HOSTS: &[&str] = &[
    "easybtx.com",
    "witness-1.easybtx.com",
    "github.com",
    "api.btxscan.io",
    "btxscan.io",
];

/// Every host diagnostics may name: the node's own published peers plus this
/// app's own services.
fn published_hosts() -> Vec<String> {
    let mut hosts = btx_core::node::published_peer_hosts();
    hosts.extend(APP_SERVICE_HOSTS.iter().map(|s| s.to_string()));
    hosts
}

/// The person's home folder, written as `~` in a diagnostics report. No
/// `dirs` dependency here (btx-core has one; this app crate does not), so
/// read the platform's own env var directly.
fn home_dir_display() -> Option<String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|p| std::path::PathBuf::from(p).display().to_string())
}

/// Plain words for how this binary got onto the machine, from tauri's own
/// `BundleType` (`None` when tauri could not tell: a dev build, or a target
/// tauri does not recognise). One arm per variant, so a bundle format the
/// mapping has not been taught yet fails a test here, not a report someone
/// pastes into support chat with a bare `Some(Whatever)` in it.
fn install_words(bundle: Option<tauri::utils::config::BundleType>) -> &'static str {
    use tauri::utils::config::BundleType::*;
    match bundle {
        None => "the install type is unknown",
        Some(App) => "installed as a Mac app",
        Some(Dmg) => "installed as a .dmg",
        Some(Deb) => "installed as a .deb",
        Some(AppImage) => "installed as an AppImage",
        Some(Rpm) => "installed as an .rpm",
        Some(Msi) => "installed as a Windows installer (msi)",
        Some(Nsis) => "installed as a Windows installer (nsis)",
    }
}

/// Thousands-grouped digits, matching how `btx_core::diagnostics::render`
/// writes its own numbers, so the phase line reads like the rest of the
/// report it sits in.
fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The phase line of a diagnostics report: what the app's own state machine
/// is doing right now, in plain words instead of `NodePhase`'s Debug form
/// (`Syncing { height: 0, headers: 0, progress: 1.0, peers: 0 }`). One arm
/// per variant, so a new phase fails a test here rather than only ever
/// showing up as a Debug dump in a report someone pastes into support chat.
fn phase_words(phase: &NodePhase) -> String {
    match phase {
        NodePhase::Welcome => "not set up yet".into(),
        NodePhase::Downloading { progress } => {
            format!("downloading the snapshot, {:.0}%", progress * 100.0)
        }
        NodePhase::Preparing => "preparing the node".into(),
        NodePhase::Starting => "starting".into(),
        NodePhase::Warming { message } => format!("warming up: {message}"),
        NodePhase::LoadingSnapshot => "loading the snapshot".into(),
        NodePhase::Syncing {
            height,
            headers,
            peers,
            ..
        } => format!(
            "syncing, height {} of {} headers, {} peers",
            group(*height),
            group(*headers),
            peers
        ),
        NodePhase::Ready {
            height,
            peers,
            blocks_behind,
        } => format!(
            "ready at {}, {} peers, {} behind",
            group(*height),
            peers,
            group(*blocks_behind)
        ),
        NodePhase::Stopped => "stopped".into(),
        NodePhase::Error { message } => format!("error: {message}"),
    }
}

static CONFIRM: std::sync::Mutex<ConfirmBook> = std::sync::Mutex::new(ConfirmBook::new());

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConsoleAnswer {
    Output { text: String },
    Confirm { token: String, sentence: String },
    Refused { sentence: String },
    Stopped,
    Warming,
}

/// Why there is no RPC client to use. The client is only armed once a start
/// has finished, so a node that is still starting (a long rebuild can take
/// an hour) has none either; the app's own phase tells the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoNode {
    Starting,
    Stopped,
}

/// What every Tools surface says to a node that is on its way up, whether
/// the app is still starting it or the engine answers RPC_IN_WARMUP (-28).
const STILL_STARTING: &str = "Your node is still starting. Try again in a moment.";

fn no_node(phase: &NodePhase) -> NoNode {
    match phase {
        NodePhase::Preparing | NodePhase::Starting | NodePhase::Warming { .. } => NoNode::Starting,
        _ => NoNode::Stopped,
    }
}

async fn no_node_now(state: &State<'_, AppState>) -> NoNode {
    no_node(&*state.phase.lock().await)
}

/// RPC_IN_WARMUP: the engine is up but still verifying or rebuilding, as in
/// `crate::ask::degrade`.
fn is_warming(e: &AppError) -> bool {
    matches!(e, AppError::Rpc { code: -28, .. })
}

fn console_without_node(n: NoNode) -> ConsoleAnswer {
    match n {
        NoNode::Starting => ConsoleAnswer::Warming,
        NoNode::Stopped => ConsoleAnswer::Stopped,
    }
}

fn console_error(e: AppError) -> ConsoleAnswer {
    if is_warming(&e) {
        return ConsoleAnswer::Warming;
    }
    ConsoleAnswer::Output {
        text: format!("The node answered: {e}"),
    }
}

fn ask_without_node<T: Serialize>(n: NoNode) -> Ask<T> {
    match n {
        NoNode::Starting => Ask::Warming,
        NoNode::Stopped => Ask::Stopped,
    }
}

fn fetch_without_node(n: NoNode) -> String {
    match n {
        NoNode::Starting => STILL_STARTING.into(),
        NoNode::Stopped => "Start your node first.".into(),
    }
}

fn fetch_error(e: AppError) -> String {
    if is_warming(&e) {
        return STILL_STARTING.into();
    }
    e.to_string()
}

/// [`fetch_error`] for the header walk, whose own check of a header (its
/// height, its parent) fails as `AppError::Decode`: a plain sentence instead
/// of the decoder's text. A correct engine never sends such a header.
fn walk_error(e: AppError) -> String {
    match e {
        AppError::Decode(_) => "The node sent a header this app could not read.".into(),
        e => fetch_error(e),
    }
}

/// The role and engine lines of a report the node could not answer for.
/// The rest of the report (the phase, with the engine's own warm-up line,
/// and the debug.log lines) does not need the node and is always there.
fn describe_without_answers(input: &mut DiagnosticsInput, n: NoNode) {
    match n {
        NoNode::Starting => {
            input.role = "not known yet, the node is still starting".into();
            input.engine_running = Some("still starting".into());
        }
        NoNode::Stopped => input.role = "node not running".into(),
    }
}

pub(crate) fn answer_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

async fn rpc_handle(state: &State<'_, AppState>) -> Option<RpcClient> {
    state.rpc.lock().await.clone()
}

async fn run_call(state: &State<'_, AppState>, call: console_policy::Call) -> ConsoleAnswer {
    let Some(rpc) = rpc_handle(state).await else {
        return console_without_node(no_node_now(state).await);
    };
    match rpc.call(&call.method, Value::Array(call.params)).await {
        Ok(v) => ConsoleAnswer::Output {
            text: answer_text(&v),
        },
        Err(e) => console_error(e),
    }
}

#[tauri::command]
pub async fn tools_console_run(
    line: String,
    state: State<'_, AppState>,
) -> Result<ConsoleAnswer, String> {
    Ok(match console_policy::decide(&line) {
        Decision::Run(call) => run_call(&state, call).await,
        Decision::Local(text) => ConsoleAnswer::Output { text },
        Decision::Refuse(sentence) => ConsoleAnswer::Refused { sentence },
        Decision::Confirm { call, sentence } => {
            let token = console_policy::new_token();
            CONFIRM.lock().unwrap_or_else(|e| e.into_inner()).issue(
                token.clone(),
                call,
                std::time::Instant::now(),
            );
            ConsoleAnswer::Confirm { token, sentence }
        }
    })
}

#[tauri::command]
pub async fn tools_console_confirm(
    token: String,
    state: State<'_, AppState>,
) -> Result<ConsoleAnswer, String> {
    let call = CONFIRM
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .redeem(&token, std::time::Instant::now());
    Ok(match call {
        Some(call) => run_call(&state, call).await,
        None => ConsoleAnswer::Refused {
            sentence: "That confirmation has expired. Run the command again.".into(),
        },
    })
}

#[tauri::command]
pub async fn tools_engine_notices(state: State<'_, AppState>) -> Result<Ask<Vec<Notice>>, String> {
    let Some(rpc) = rpc_handle(&state).await else {
        return Ok(ask_without_node(no_node_now(&state).await));
    };
    Ok(match api::get_blockchain_info(&rpc).await {
        Ok(info) => Ask::Ready(btx_core::engine_warnings::all_notices(&info)),
        Err(e) => degrade(e),
    })
}

/// Set while a restart from Tools runs. The button is only one way in: Tools
/// can be closed and reopened mid-restart, and a second restart would stop
/// the node the first one is starting, then fail its own start on top.
static RESTARTING: AtomicBool = AtomicBool::new(false);

const ALREADY_RESTARTING: &str = "Your node is already restarting. Give it a moment.";

/// Holds a flag for as long as it lives and clears it on every way out: a
/// return, an early `?`, an error from the restart, or a panic.
struct Claim<'a>(&'a AtomicBool);

impl<'a> Claim<'a> {
    /// The flag, if nobody holds it yet.
    fn take(flag: &'a AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Claim(flag))
    }
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Why Restart node may not run right now, before asking who owns the node:
/// a restart is already running, or a start is (its stop would end the node
/// that start is bringing up).
fn restart_busy(restarting: bool, starting: bool) -> Option<&'static str> {
    if restarting {
        Some(ALREADY_RESTARTING)
    } else if starting {
        Some(STILL_STARTING)
    } else {
        None
    }
}

/// `None` when Restart node may run; otherwise the sentence saying why not.
#[tauri::command]
pub async fn tools_restart_check(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let busy = restart_busy(
        RESTARTING.load(Ordering::SeqCst),
        state.start_in_flight.load(Ordering::SeqCst),
    );
    if let Some(sentence) = busy {
        return Ok(Some(sentence.into()));
    }
    let owner = node_ownership(&state, &node_datadir()).await;
    Ok(destructive_allowed(owner).err())
}

#[tauri::command]
pub async fn tools_restart_node(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let Some(_restarting) = Claim::take(&RESTARTING) else {
        return Err(ALREADY_RESTARTING.into());
    };
    if let Some(sentence) = restart_busy(false, state.start_in_flight.load(Ordering::SeqCst)) {
        return Err(sentence.into());
    }
    destructive_allowed(node_ownership(&state, &node_datadir()).await)?;
    restart_node_projected(&app, &state).await
}

#[derive(Debug, Clone, Serialize)]
pub struct FetchOutcome {
    pub message: String,
    pub tip_before: u64,
    pub tip_after: u64,
}

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
        .map_err(walk_error)?;
    match path.status(tip_height, tip_hash) {
        PathStatus::Ready => Ok(path.next(tip_height, usize::MAX)),
        PathStatus::OtherBranch => Err("The newest headers are on another branch than your node's tip. The node decides that on its own.".into()),
        // Not reached: the target is above the tip (`stuck_blocks::target_tip`
        // picks only such), and a walk with no budget returns Ok only once it
        // has reached the tip. Kept as "nothing to ask for" rather than a
        // panic.
        PathStatus::Nothing | PathStatus::Walking => Ok(Vec::new()),
    }
}

#[tauri::command]
pub async fn tools_fetch_stuck_blocks(state: State<'_, AppState>) -> Result<FetchOutcome, String> {
    let Some(rpc) = rpc_handle(&state).await else {
        return Err(fetch_without_node(no_node_now(&state).await));
    };
    let info = api::get_blockchain_info(&rpc).await.map_err(fetch_error)?;
    let tip_hash = rpc
        .call("getbestblockhash", json!([]))
        .await
        .map_err(fetch_error)?;
    let tip_hash = tip_hash.as_str().unwrap_or("").to_string();
    let tips = api::get_chain_tips(&rpc).await.map_err(fetch_error)?;
    let done = |message: String| FetchOutcome {
        message,
        tip_before: info.blocks,
        tip_after: info.blocks,
    };
    let Some(target) = stuck_blocks::target_tip(&tips, info.blocks) else {
        return Ok(done(
            "Your node has every block it knows of. Nothing to fetch.".into(),
        ));
    };
    let missing = match missing_blocks(&rpc, target, info.blocks, &tip_hash).await {
        Ok(m) => m,
        Err(sentence) => return Ok(done(sentence)),
    };
    let peers = api::get_peer_info(&rpc).await.map_err(fetch_error)?;
    let reqs = match stuck_blocks::plan(&missing, &peers) {
        FetchPlan::Nothing(sentence) => return Ok(done(sentence)),
        FetchPlan::Ask(reqs) => reqs,
    };
    let mut asked = Vec::new();
    for r in &reqs {
        if rpc
            .call("getblockfrompeer", json!([r.hash, r.peer_id]))
            .await
            .is_ok()
        {
            asked.push(r.clone());
        }
    }
    if asked.is_empty() {
        return Ok(done(
            "The peers did not take the request. Give it a few minutes.".into(),
        ));
    }
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    let after = api::get_blockchain_info(&rpc)
        .await
        .map(|i| i.blocks)
        .unwrap_or(info.blocks);
    let moved = if after > info.blocks {
        format!(" The tip moved from {} to {}.", info.blocks, after)
    } else {
        " No block has connected yet; the node may still be checking them.".to_string()
    };
    Ok(FetchOutcome {
        message: format!("{}{}", stuck_blocks::summary(&asked), moved),
        tip_before: info.blocks,
        tip_after: after,
    })
}

#[tauri::command]
pub async fn tools_diagnostics(
    status_line: String,
    window_lines: Vec<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let datadir = node_datadir();
    let rpc = rpc_handle(&state).await;
    let mut input = DiagnosticsInput {
        generated_at: format!("{} UTC", chrono_like_now()),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        engine_pinned: format!(
            "{} ({})",
            crate::commands::NODE_RELEASE_TAG,
            &crate::commands::NODE_RELEASE_COMMIT[..8]
        ),
        platform: format!(
            "{} {}, {}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            install_words(tauri::utils::platform::bundle_type())
        ),
        status_line,
        window_lines,
        phase: phase_words(&*state.phase.lock().await),
        signer_pubkey: state.signer_pubkey.lock().await.clone(),
        stall: state
            .stall_verdict
            .lock()
            .await
            .as_ref()
            .map(|v| v.summary.to_string()),
        catch_up: state.catch_up_help.lock().await.lines.clone(),
        recovery: crate::read_block_recovery::diagnostics_lines(&datadir),
        log_warnings: diagnostics::warning_lines(&btx_core::node::debug_log_tail(
            &datadir,
            diagnostics::LOG_TAIL_BYTES,
        )),
        // Kept after the check is done, current or not: the report says which.
        start_record: btx_core::snapshot_start::read(&datadir),
        // The refresher's window, held for this run only.
        signature_window: state.signature_window.lock().await.deltas(),
        ..Default::default()
    };
    let mut answering = None;
    match &rpc {
        None => describe_without_answers(&mut input, no_node_now(&state).await),
        // While the engine warms up it answers every call with -28, so the
        // first answer decides: the rest would only repeat it.
        Some(rpc) => match api::get_blockchain_info(rpc).await {
            Err(e) if is_warming(&e) => describe_without_answers(&mut input, NoNode::Starting),
            chain => {
                input.chain = chain.ok();
                answering = Some(rpc);
            }
        },
    }
    if let Some(rpc) = answering {
        input.best_block_hash = rpc
            .call("getbestblockhash", json!([]))
            .await
            .ok()
            .and_then(|v| v.as_str().map(str::to_string));
        input.engine_running = rpc
            .call("getnetworkinfo", json!([]))
            .await
            .ok()
            .and_then(|v| v["subversion"].as_str().map(str::to_string));
        input.chainstates = api::get_chainstates(rpc).await.ok();
        input.tips = api::get_chain_tips(rpc).await.unwrap_or_default();
        input.peers = api::get_peer_info(rpc).await.unwrap_or_default();
        input.attested_tip = api::get_attested_tip(rpc).await.ok();
        let trusted = api::get_matmul_trusted_status(rpc).await.ok();
        input.role = match &trusted {
            Some(t) if t.trusted_mirror => "follows signatures".into(),
            Some(t) => format!("checks blocks itself ({})", t.matmul_validation_mode),
            None => "unknown".into(),
        };
        // The same answer feeds the Signatures block: the live pin and the
        // engine's signature counters.
        input.trusted = trusted;
        for h in btx_core::known_invalid::HELD_BRANCHES {
            let on_chain = rpc
                .call("getblockhash", json!([h.height]))
                .await
                .ok()
                .and_then(|v| v.as_str().map(|s| s == h.root));
            // "Not seen" only on the engine's own answer; a node that did not
            // answer is reported as such, not as one that never saw the root.
            let known = btx_core::known_invalid::header_known(rpc, h.root).await;
            let held_state = match (known, on_chain) {
                (_, Some(true)) => "ON THIS NODE'S CHAIN",
                (Ok(true), _) => "seen, not on this node's chain",
                (Ok(false), _) => "not seen by this node",
                (Err(_), _) => "could not ask this node",
            };
            input.held.push(HeldBranchState {
                height: h.height,
                root: h.root[..16].to_string(),
                state: held_state.into(),
            });
        }
    }
    let ctx = RedactionContext {
        home: home_dir_display(),
        secrets: secrets(&datadir),
        published_hosts: published_hosts(),
    };
    Ok(diagnostics::report(&input, &ctx))
}

/// The cookie password and the signing key's own text, read only to be removed.
fn secrets(datadir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(cookie) = std::fs::read_to_string(datadir.join(".cookie")) {
        if let Some((_, pass)) = cookie.trim().split_once(':') {
            out.push(pass.to_string());
        }
    }
    if let Ok(wif) = std::fs::read_to_string(btx_core::signer::signer_key_path(datadir)) {
        out.push(wif.trim().to_string());
    }
    out
}

/// "2026-09-29 14:05" in UTC, without a date crate.
fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant), proleptic Gregorian.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3_600,
        (rem % 3_600) / 60
    )
}

#[cfg(test)]
mod tests {
    use super::answer_text;
    use serde_json::json;

    mod restart_node {
        use super::super::*;

        /// A restart already running refuses a second one (Tools closed and
        /// reopened mid-restart), and a start in progress refuses one too:
        /// its stop would end the node that start is bringing up.
        #[test]
        fn refuses_while_a_restart_or_a_start_is_running() {
            assert_eq!(restart_busy(true, false), Some(ALREADY_RESTARTING));
            assert_eq!(restart_busy(true, true), Some(ALREADY_RESTARTING));
            assert_eq!(restart_busy(false, true), Some(STILL_STARTING));
            assert_eq!(restart_busy(false, false), None);
        }

        #[test]
        fn only_one_claim_at_a_time() {
            let flag = AtomicBool::new(false);
            let first = Claim::take(&flag);
            assert!(first.is_some());
            assert!(Claim::take(&flag).is_none(), "a second restart got in");
            drop(first);
            assert!(Claim::take(&flag).is_some(), "the flag stayed set");
        }

        /// The flag is cleared on every way out of a restart: success, an
        /// early refusal, an error from the restart itself, and a panic.
        #[test]
        fn the_claim_is_released_on_every_exit_path() {
            fn restart(flag: &AtomicBool, outcome: Result<(), &str>) -> Result<(), String> {
                let _claim = Claim::take(flag).ok_or(ALREADY_RESTARTING)?;
                assert!(flag.load(Ordering::SeqCst), "held but not set");
                outcome.map_err(str::to_string)?;
                Ok(())
            }
            let flag = AtomicBool::new(false);
            assert!(restart(&flag, Ok(())).is_ok());
            assert!(!flag.load(Ordering::SeqCst), "set after a success");
            assert!(restart(&flag, Err("the node kept exiting")).is_err());
            assert!(!flag.load(Ordering::SeqCst), "set after an error");
            let panicked = std::panic::catch_unwind(|| {
                let _claim = Claim::take(&flag).unwrap();
                assert!(flag.load(Ordering::SeqCst), "held but not set");
                panic!("the restart panicked");
            });
            assert!(panicked.is_err());
            assert!(!flag.load(Ordering::SeqCst), "set after a panic");
        }
    }

    /// One arm per `BundleType` variant in the locked tauri-utils
    /// (~/.cargo/registry/src/*/tauri-utils-2.9.3/src/config.rs), plus
    /// `None`, so a future bundle format the mapping has not been taught
    /// yet fails here instead of showing up as `Some(Whatever)` in a report.
    mod install_words {
        use super::super::*;
        use tauri::utils::config::BundleType;

        #[test]
        fn no_bundle_type_says_the_install_type_is_unknown() {
            assert_eq!(install_words(None), "the install type is unknown");
        }

        #[test]
        fn every_bundle_type_gets_plain_words() {
            assert_eq!(
                install_words(Some(BundleType::App)),
                "installed as a Mac app"
            );
            assert_eq!(install_words(Some(BundleType::Dmg)), "installed as a .dmg");
            assert_eq!(install_words(Some(BundleType::Deb)), "installed as a .deb");
            assert_eq!(
                install_words(Some(BundleType::AppImage)),
                "installed as an AppImage"
            );
            assert_eq!(install_words(Some(BundleType::Rpm)), "installed as an .rpm");
            assert_eq!(
                install_words(Some(BundleType::Msi)),
                "installed as a Windows installer (msi)"
            );
            assert_eq!(
                install_words(Some(BundleType::Nsis)),
                "installed as a Windows installer (nsis)"
            );
        }
    }

    /// One arm per `NodePhase` variant, checked against the exact wording
    /// `tools_diagnostics` puts on the report's "Phase:" line.
    mod phase_words {
        use super::super::*;

        #[test]
        fn welcome_is_not_set_up_yet() {
            assert_eq!(phase_words(&NodePhase::Welcome), "not set up yet");
        }

        #[test]
        fn downloading_shows_its_percent() {
            assert_eq!(
                phase_words(&NodePhase::Downloading { progress: 0.5 }),
                "downloading the snapshot, 50%"
            );
        }

        #[test]
        fn preparing_is_preparing_the_node() {
            assert_eq!(phase_words(&NodePhase::Preparing), "preparing the node");
        }

        #[test]
        fn starting_is_starting() {
            assert_eq!(phase_words(&NodePhase::Starting), "starting");
        }

        #[test]
        fn warming_carries_the_engine_s_own_message() {
            assert_eq!(
                phase_words(&NodePhase::Warming {
                    message: "Verifying blocks...".into(),
                }),
                "warming up: Verifying blocks..."
            );
        }

        #[test]
        fn loading_snapshot_is_loading_the_snapshot() {
            assert_eq!(
                phase_words(&NodePhase::LoadingSnapshot),
                "loading the snapshot"
            );
        }

        #[test]
        fn syncing_names_height_headers_and_peers() {
            assert_eq!(
                phase_words(&NodePhase::Syncing {
                    height: 0,
                    headers: 0,
                    progress: 1.0,
                    peers: 0,
                }),
                "syncing, height 0 of 0 headers, 0 peers"
            );
        }

        #[test]
        fn ready_groups_the_height_and_names_peers_and_blocks_behind() {
            assert_eq!(
                phase_words(&NodePhase::Ready {
                    height: 233_480,
                    peers: 8,
                    blocks_behind: 0,
                }),
                "ready at 233,480, 8 peers, 0 behind"
            );
        }

        #[test]
        fn stopped_is_stopped() {
            assert_eq!(phase_words(&NodePhase::Stopped), "stopped");
        }

        #[test]
        fn error_carries_its_own_message() {
            assert_eq!(
                phase_words(&NodePhase::Error {
                    message: "btxd exited".into(),
                }),
                "error: btxd exited"
            );
        }
    }

    mod fetch_a_stuck_block {
        use super::super::*;

        /// A header the walk cannot read (no height where it should be, no
        /// parent) gets a plain sentence, not the decoder's own text.
        #[test]
        fn a_header_the_walk_cannot_read_gets_a_plain_sentence() {
            let e = AppError::Decode("header 00ab is not at height 5".into());
            assert_eq!(
                walk_error(e),
                "The node sent a header this app could not read."
            );
            let other = || AppError::Rpc {
                code: -5,
                message: "Block not found".into(),
            };
            assert_eq!(walk_error(other()), fetch_error(other()));
        }
    }

    mod a_node_that_is_still_starting {
        use super::super::*;
        use btx_core::engine_warnings::Notice;

        const STARTING: &str = "Your node is still starting. Try again in a moment.";

        fn warming() -> NodePhase {
            NodePhase::Warming {
                message: "Verifying blocks...".into(),
            }
        }

        fn in_warmup() -> AppError {
            AppError::Rpc {
                code: -28,
                message: "Verifying blocks...".into(),
            }
        }

        /// The RPC client is only armed once a start has finished, so the
        /// phase is what tells a starting node from a stopped one.
        #[test]
        fn is_told_apart_from_a_stopped_one_by_the_phase() {
            for p in [NodePhase::Preparing, NodePhase::Starting, warming()] {
                assert_eq!(no_node(&p), NoNode::Starting, "{p:?}");
            }
            for p in [
                NodePhase::Stopped,
                NodePhase::Welcome,
                NodePhase::Error {
                    message: "x".into(),
                },
            ] {
                assert_eq!(no_node(&p), NoNode::Stopped, "{p:?}");
            }
        }

        #[test]
        fn the_command_window_says_it_is_starting_not_stopped() {
            let starting = serde_json::to_value(console_without_node(NoNode::Starting)).unwrap();
            assert_eq!(starting, json!({"kind": "warming"}));
            let stopped = serde_json::to_value(console_without_node(NoNode::Stopped)).unwrap();
            assert_eq!(stopped, json!({"kind": "stopped"}));
            let warmup = serde_json::to_value(console_error(in_warmup())).unwrap();
            assert_eq!(warmup, json!({"kind": "warming"}));
            let other = console_error(AppError::Rpc {
                code: -8,
                message: "Block height out of range".into(),
            });
            assert!(
                matches!(&other, ConsoleAnswer::Output { text } if text.contains("Block height out of range")),
                "{other:?}"
            );
        }

        #[test]
        fn engine_notices_say_it_is_starting_not_stopped() {
            let starting =
                serde_json::to_value(ask_without_node::<Vec<Notice>>(NoNode::Starting)).unwrap();
            assert_eq!(starting, json!({"state": "warming"}));
            let stopped =
                serde_json::to_value(ask_without_node::<Vec<Notice>>(NoNode::Stopped)).unwrap();
            assert_eq!(stopped, json!({"state": "stopped"}));
        }

        #[test]
        fn fetch_a_stuck_block_says_it_is_starting_not_stopped() {
            assert_eq!(fetch_without_node(NoNode::Starting), STARTING);
            assert_eq!(
                fetch_without_node(NoNode::Stopped),
                "Start your node first."
            );
            assert_eq!(fetch_error(in_warmup()), STARTING);
            assert_eq!(walk_error(in_warmup()), STARTING);
        }

        /// The report is still produced, with the phase and the log lines,
        /// and its role and engine lines say the node is starting.
        #[test]
        fn the_diagnostics_report_says_it_is_starting_not_stopped() {
            let mut input = DiagnosticsInput {
                phase: phase_words(&warming()),
                log_warnings: vec!["2026-09-29T10:00:00Z [warning] still verifying".into()],
                ..Default::default()
            };
            describe_without_answers(&mut input, NoNode::Starting);
            let report = diagnostics::render(&input);
            assert!(!report.contains("not running"), "{report}");
            assert!(
                report.contains("Role: not known yet, the node is still starting"),
                "{report}"
            );
            assert!(report.contains("(running: still starting)"), "{report}");
            assert!(
                report.contains("Phase: warming up: Verifying blocks..."),
                "{report}"
            );
            assert!(report.contains("still verifying"), "{report}");

            let mut stopped = DiagnosticsInput::default();
            describe_without_answers(&mut stopped, NoNode::Stopped);
            assert!(diagnostics::render(&stopped).contains("Role: node not running"));
        }
    }

    #[test]
    fn strings_print_raw_and_objects_print_pretty() {
        assert_eq!(
            answer_text(&json!("line one\nline two")),
            "line one\nline two"
        );
        assert_eq!(answer_text(&json!({"a": 1})), "{\n  \"a\": 1\n}");
        assert_eq!(answer_text(&json!(null)), "null");
    }

    #[test]
    fn the_date_helper_formats_like_utc() {
        let s = super::chrono_like_now();
        assert_eq!(s.len(), 16);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], " ");
    }

    /// Task 5's diagnostics::redact only ever sees hosts this app hands it
    /// through `RedactionContext.published_hosts`; if this app's own update
    /// feed / release host / explorer API is missing from that list, a
    /// report that names one of them gets shredded as if it were a stranger's
    /// peer address.
    #[test]
    fn published_hosts_include_the_node_s_own_service_hosts() {
        let hosts = super::published_hosts();
        assert!(
            hosts.iter().any(|h| h == "easybtx.com"),
            "missing easybtx.com: {hosts:?}"
        );
        assert!(
            hosts.iter().any(|h| h == "20.86.181.203"),
            "missing 20.86.181.203: {hosts:?}"
        );
    }
}

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
/// snapshot, or any distance behind one while the flag holds. Controller
/// note 3 (a validating node's mirror launch) is not this function's to
/// decide: it needs the node's role, which `tools_fast_forward_check` asks
/// separately, after this.
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
/// sentence while the operators disagree; else nothing. Withheld on a
/// validating node whose one mirror launch could not even be tried
/// (`mirror_launch_available`, controller note 3): such a run always rolls
/// back, so the button would only promise something that cannot happen.
#[tauri::command]
pub async fn tools_fast_forward_check(
    state: State<'_, AppState>,
) -> Result<FastForwardCheck, String> {
    let datadir = node_datadir();
    let owned = destructive_allowed(node_ownership(&state, &datadir).await).is_ok();
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
    // Judged with the pins the run's load will have, not the running
    // engine's: a node that checks blocks may pin none until its one mirror
    // launch (`crate::fast_forward::run_view`).
    let follows_signatures = crate::fast_forward::follows_signatures_here(&datadir);
    let view = crate::fast_forward::run_view(&rpc, follows_signatures).await;
    // A client that fails to build (never in practice) is nothing to show,
    // like every other soft failure above: no raw error text reaches the
    // window from a background check nobody asked for yet.
    let Ok(client) = btx_core::attested_snapshot::http_client() else {
        return Ok(FastForwardCheck::None);
    };
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
    let old_blocks_refused = state
        .catch_up_help
        .lock()
        .await
        .no_archive_serves_old_blocks;
    let check = fast_forward_check(owned, peek, tip, old_blocks_refused);
    if matches!(check, FastForwardCheck::Offer { .. })
        && !follows_signatures
        && !mirror_launch_available(&datadir)
    {
        return Ok(FastForwardCheck::None);
    }
    Ok(check)
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

/// The run's phase decides first (controller note 1): a `RolledBack`
/// outcome is written before its restore begins, so it may be shown only
/// once no run is recorded at all; while the driver is stuck, its record
/// cannot be read, or a run is recorded with no driver at work (it waits
/// for the next start), a plain sentence is shown, never "running"
/// (`crate::fast_forward::tools_status_phase`).
#[tauri::command]
pub async fn tools_fast_forward_status() -> Result<FastForwardStatus, String> {
    use crate::fast_forward::ToolsPhase;
    use btx_core::fast_forward::{copy, read_outcome, Outcome};
    let datadir = node_datadir();
    Ok(match crate::fast_forward::tools_status_phase(&datadir) {
        ToolsPhase::Halted(message) => FastForwardStatus {
            running: false,
            message: Some(message),
        },
        ToolsPhase::Running => FastForwardStatus {
            running: true,
            message: Some(copy::running()),
        },
        ToolsPhase::Idle => {
            let message = read_outcome(&datadir).map(|o| match o {
                Outcome::Done { height, operators } => copy::done(height, &operators),
                Outcome::RolledBack { reason } => copy::rolled_back(&reason),
            });
            FastForwardStatus {
                running: false,
                message,
            }
        }
    })
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
                    confirm
                        .starts_with("Fast-forward to block 233,800, confirmed by Mende and jpp?"),
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
        assert_eq!(
            fast_forward_check(true, None, 1, false),
            FastForwardCheck::None
        );
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
                fast_forward_check(
                    true,
                    Some(Peek::Disputed { newest: 233_800 }),
                    233_500,
                    true
                ),
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
        let off = fast_forward_check(
            true,
            Some(Peek::Disputed { newest: 233_800 }),
            233_900,
            false,
        );
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
