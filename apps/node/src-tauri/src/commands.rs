//! easyBTX Node commands: the first-run setup pipeline, node start/stop, and
//! the status snapshot the UI polls. All heavy lifting lives in btx-core; this
//! file is orchestration + phase reporting.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

use btx_core::backend::Backend;
use btx_core::error::AppError;
use btx_core::installer::{
    install_dir, resolve_bundled_node_pkg, returning_launch_paths, FaststartResult,
};
use btx_core::node::{DatadirHolder, NodeController, BTX_BOOTSTRAP_PEERS};
use btx_core::node_api::{get_blockchain_info, get_chainstates};
use btx_core::rpc::RpcClient;
use btx_core::setup::{
    enough_free_disk, ensure_addnodes_in_conf, free_disk_bytes, wait_for_node_rpc, RPC_URL,
};
use btx_core::snapshot::SnapshotSpec;

use crate::state::{
    node_datadir, AppState, AttachedTo, NodeAppSettings, NodeAppSnapshotFlags, NodePhase,
};

/// The BTX release this app installs and runs — the network's current version
/// (btxprice.com network_update_notice signals when this must move). The
/// bundled package tree is staged by `scripts/stage-node-pkg.sh`.
///
/// **v0.33.2 was not optional** — it carried the MatMul v4.7 proof-of-work
/// change that activated unconditionally at mainnet block 185,000. A v0.33.1
/// node does not fork off so much as stop: it cannot evaluate the new work and
/// simply stalls at the activation height.
///
/// **v0.33.3 is not optional either, for a different reason.** v0.33.2 shipped
/// with a networking defect upstream calls "the single worst failure mode since
/// Epoch-A activation": an in-memory MatMul async-verification marker never
/// expired, so blocks could be permanently skipped for download and the node
/// wedged until someone restarted it by hand. v0.33.3 expires those markers,
/// stops the consensus tier deadlocking CPU-only nodes, and demotes that tier
/// from a hard gate to a preference.
///
/// ⚠ **This is built from upstream PR #105 (`pr/0.33.3-network-stability`), not
/// a tagged release**, because the network needed the stall fix before upstream
/// tagged anything. Re-pin to the real tag the moment it lands.
///
/// **`-pr105b` advances the pin to branch commit `1e51f0d1`** (2026-08-12).
/// The earlier `-pr105` build (b3bf6911) still carried the defect that wedged
/// the fleet one block below Epoch-A: `PreferTrustAdjustedHeader` scores
/// unattested header work as zero, so the best-header pointer parked at 184,999
/// and the continuing chain's bodies were never requested. 1e51f0d1 adds the
/// bounded escape valve + download-eligibility-by-claimed-work fixes (verified
/// here: headers advanced 184,999 → network tip within minutes of the swap),
/// plus root-first block download and a snapshot-RPC crash fix.
///
/// The tag carries a `-pr105` suffix on purpose. The branch never bumped
/// `CLIENT_VERSION_BUILD`, so the binary still reports `v0.33.2` — identical to
/// the released node it replaces. Our upgrade path only re-provisions when THIS
/// constant changes, so the tag has to differ even though the version does not.
/// The package declares the version it really carries in `.btxd-version`, and
/// provisioning verifies against that.
///
/// ⚠ Do NOT close that gap by patching BTX's source to report 0.33.3. btxd
/// embeds whether its source tree was dirty and then fails its own production
/// canary with `build_provenance_mismatch`, leaving a node that runs and syncs
/// but refuses to validate. Measured here: editing one line of CMakeLists.txt
/// turned a passing canary (`ready=1`) into `ready=0`.
///
/// Returning users are carried across by the upgrade path in `start_node_inner`,
/// which re-provisions from the bundle whenever this constant moves — the chain
/// in `~/.easybtx` is untouched, so this costs a ~25 MB swap, not a resync.
// v0.33.4.1: the OFFICIAL upstream release tag (2026-08-24), self-built on this
// Mac at the tag commit `0d3e384f` from a CLEAN tree.
//
// Self-built for the same reason every release since 0.6.2 is: upstream's macOS
// tarball dynamically links Homebrew libevent/libomp, which a consumer Mac does
// not have, so it cannot run inside the app bundle. This build vendors nothing
// and links only system frameworks and /usr/lib — verified identical `otool -L`
// output to the v0.33.3 package this replaces.
//
// ⚠ Timing note, because it caught this release out. When the pin was chosen
// the tag carried ONLY `btx-0.33.4.1-linux-x86_64-{cpu,cuda}.tar.gz` and no
// macOS asset at all. `btx-0.33.4.1-macos-arm64-metal.tar.gz` appeared later
// the same day. It does not change the decision above, but "upstream ships no
// Mac binary" is a claim with a shelf life of hours — re-read the asset list
// before repeating it. Whether that tarball has the Homebrew linkage is NOT
// measured here; the self-build sidesteps the question either way.
//
// 🔴 THIS PIN IS FORKED. Read before choosing any successor. Corrected
// 2026-08-28; the paragraph that stood here said the opposite and it is left
// described rather than deleted so the mistake is legible.
//
// It used to say: "v0.33.4 bakes EncDr stall recovery at mainnet height 199299
// and a node still on v0.33.3 at that height forks off." That was written on
// 2026-08-25 when the re-anchor was believed to be the correct rule. It is
// backwards now. Upstream WITHDREW the re-anchor on 2026-08-27 (commit
// 1a58e07a, first shipped in v0.34.2) after measuring that it partitioned its
// own nodes onto a minority branch. The majority chain is the plain-ASERT
// chain, so it is v0.33.4.x that forks off and v0.33.3 that does not.
//
// Measured per tag, mainnet, 2026-08-28:
//
//     v0.33.3                                        absent
//     v0.33.4 v0.33.4.1 v0.33.4.2 v0.34 v0.34.1      = 199'299
//     v0.34.2 v0.34.3 v0.34.4                        = int32 max
//
// pow.cpp applies it with no version gate, and MatMulAsert compares
// next_height with `==`, so divergence begins at exactly 199299. The anchor
// code is byte identical between v0.33.4.1 and v0.34.1.
//
// RESOLVED 2026-08-31. NODE_RELEASE_TAG below is v0.34.5, which sets the
// constant to INT32_MAX on mainnet and therefore does not diverge. Verified by
// grepping the TAG object, not the working tree, which sits at v0.33.2:
//
//     v0.33.4  v0.33.4.1  v0.33.4.2  v0.34.1   = 199'299   (forked)
//     v0.34.2  v0.34.4    v0.34.5              = INT32_MAX (clean)
//
// The old text here said "0.34.5 is not tagged. Engine bumps are FROZEN." Both
// premises are dead. The tag exists and upstream's own STOP banner now ends
// "0.34.5 is the sealed binary that actually bootstraps". We measured that
// ourselves rather than taking it: a fresh datadir on this engine reached
// headers 204385, past the fork, and began downloading blocks.
//
// ⚠ Two things still hold and must not be dropped.
//   1. Run scripts/check-engine-tag.sh AND scripts/check-engine-fleet-ready.sh
//      on any candidate. The first only checks the constant; a tag can pass it
//      and still have no startable mode on most of the fleet, which is exactly
//      what v0.34.4 does.
//   2. snapshot_spec() moves WITH this constant. The 203000 assumeutxo base is
//      compiled into v0.34.5 and is absent from v0.33.4.x, so an engine
//      downgrade without a matching spec downgrade makes loadtxoutset refuse.
//
// Read docs/node-release-recipe.md before cutting a release; it covers the
// assumeutxo and trusted-mirror conditions the constant check does not.
//
// Still true and still useful: height 199298 is unchanged from 0.33.3 and sits
// on the attested 199297 parent
// `6a651911077a52f9488607da23a85a62532e4945dc23f54dc92080d2a6f8c775`
// (re-verified 2026-08-28 against esplora.btxbyronbay.com/block-height/199297).
// Upstream names 199298 `be78622c…` as the last block common to both chains.
//
// ⚠ Use `.1`, not the bare `v0.33.4` tag. v0.33.4.1 is a provenance RESEAL
// (upstream #120): same consensus, same ExactReplay digest (`b4777985…`),
// resealed golden manifest. A binary built from `v0.33.4` fails its own
// production canary with `canary=build_provenance_mismatch` and then runs and
// syncs while silently refusing to validate — the worst available failure mode.
// Verified on the build this pin ships: the canary does NOT report a provenance
// mismatch, and the embedded revision is the tag commit.
//
// `btxd --version` reports the full `v0.33.4.1` here even though CLIENT_VERSION
// stays 0.33.4, so the provisioning version gate matches on the tag directly.
// The `.btxd-version` marker in the staged package is what that gate actually
// reads — keep the staging script's regex able to capture four segments.
// ── THE SECOND QUESTION: CAN THIS ENGINE KEEP UP? ──────────────────────────
//
// Everything above answers "does this tag diverge at 199299". It does not, and
// two gates enforce that. Nothing above answers a question that matters just as
// much to somebody running a node, and the answer is uncomfortable.
//
// Measured 2026-09-02 on one box, ONE variable changed — same datadir, same
// arguments, same peers, only the binary swapped:
//
//     v0.34.5    0.68 blocks/min while connected
//     v0.34.6    3.80 blocks/min while connected
//     the chain  0.95 blocks/min produced
//
// v0.34.5 ingests more slowly than the chain produces. A node on it that falls
// behind does not catch up — it loses ground permanently, and it does so
// QUIETLY. It does not error. It downloads blocks, reports itself healthy, and
// the gap grows. One was watched sitting 950 behind with in_flight_global=0 on
// 45,298 of 45,914 log samples.
//
// ⚠ SO WHY IS THE PIN STILL v0.34.5? Because there is nothing to move it to.
// Verified against the upstream API on 2026-09-04: the newest TAG on
// btxchain/btx is v0.34.5 and the newest RELEASE is v0.34.5. `release/0.34.6`
// exists only as a BRANCH, head 9eb4e005. This constant names a tag that
// staging fetches a release tarball for and that both gates read files from, so
// pointing it at an untagged branch breaks the release path rather than fixing
// the fleet.
//
// The machine that measured this runs a source build of that branch installed
// over the shipped binaries, which is why it converges and why its app UI still
// says v0.34.5. That is a local workaround on one box, not a shipped fix, and
// it should not be mistaken for one.
//
// WHEN 0.34.6 TAGS: run both gates on it, then move this constant and re-check
// snapshot_spec() per the note above. Until then a Linux release either ships
// an engine that cannot converge or it waits, and that is a release decision
// rather than something to change quietly here.
pub const NODE_RELEASE_TAG: &str = "v0.34.5";

/// The pinned assumeutxo snapshot this app bootstraps from: the v0.33.2
/// release's own asset (height 179000), pinned from its snapshot.manifest.json.
/// The pin MUST track the release — upstream regenerates snapshot.dat assets in
/// place, so the SHA gate is what catches a mismatched or superseded asset.
/// (v0.33.2's asset is genuinely different bytes from v0.33.1's: 452282113 vs
/// 448392435, anchor 179000 vs 155700.)
pub fn snapshot_spec() -> SnapshotSpec {
    btx_core::snapshot::v0_34_5_spec()
}

/// The matmul backend env for btxd. The node app never mines, but block
/// VALIDATION evaluates MatMul proofs too — on Apple Silicon the Metal backend
/// does that on the GPU. Everywhere else: CPU.
///
/// ⚠ "btxd falls back to CPU safely if Metal is unavailable" USED to be true and
/// is not true after the MatMul v4.7 fork (mainnet block 185,000). Under the
/// default `strict-device` RC execution policy a CPU fallback is exactly what
/// btxd refuses: the GEMM backend is zeroed and the node stalls instead of
/// validating. Since #331, `node::rc_execution_mode` hands every non-Metal
/// host `strict-device` PLUS trusted-mirror mode (`-matmulvalidation=trusted`
/// + the two signer pubkeys) — NOT `auto-fallback`, which a 16-hour field run
/// showed pegging a core, deadlocking `btx-cli stop`, and bricking a datadir
/// on the forced kill. The UI reports btxd's OWN policy rather than assuming
/// this function's choice succeeded.
fn node_backend() -> Backend {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Backend::Metal
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        Backend::Cpu
    }
}

/// Wait budget for a freshly-spawned node's RPC: 360 × 500 ms = 3 min covers a
/// cold start / slow disk; a node that is ALIVE but warming (RPC_IN_WARMUP)
/// keeps the wait going inside wait_for_node_rpc's poll loop, and a healthy
/// node proceeds in 1–3 s.
const RPC_WAIT_POLLS: u32 = 360;
const RPC_WAIT_POLL_MS: u64 = 500;
/// Warmup budget: a node answering RPC_IN_WARMUP is ALIVE (an unclean
/// shutdown's shielded-state rebuild runs ~8+ min; slow disks and a large
/// chain much longer). 8 HOURS: the 45-min ceiling proved too small in the
/// field — a healthy "Verifying blocks…" rebuild timed into a red error card
/// that reads as broken. The warmup watcher (spawned alongside the wait)
/// surfaces the calm Warming phase the whole time, so the long budget never
/// leaves the user staring at a silent "Starting".
const RPC_WAIT_WARMUP_POLLS: u32 = 57_600; // × 500 ms = 8 h

/// Post-stop wait for an unmanaged btxd to actually free the datadir lock
/// before we spawn (force-kill fallback only after this): btxd's flush after
/// `stop` ran 90–120 s on the dev M2 Pro at height ~185k, so this is the
/// controller's own 90 s stop grace plus slow-disk headroom. Bounded so a
/// truly wedged holder can't hang an upgrade start forever.
const UNMANAGED_STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(180);
/// A btxd that loses the datadir-lock race prints "Cannot obtain a lock on
/// directory…" and exits in well under a second; a child alive this long owns
/// the lock and deserves the full RPC wait.
const LAUNCH_SURVIVAL_WATCH: std::time::Duration = std::time::Duration::from_secs(5);
/// Spawn attempts before giving up. The 2026-08-12 0.6.1→0.6.2 self-update
/// failure ended with NO node running precisely because the old flow had zero
/// retries after a lost lock race.
const LAUNCH_ATTEMPTS: u32 = 3;
/// How many times a launch looks again at a holder it must not disturb before
/// it stops waiting and decides. Six looks, five seconds apart, is about half a
/// minute: long enough for another app's btxd to finish binding RPC after its
/// own start (the legitimate reason for this wait), short enough that a person
/// watching "Starting" does not conclude the app is wedged. Zero would be the
/// old behaviour — refuse on the first look — which is the bug.
const HOLDER_RECHECKS: u32 = 6;
const HOLDER_RECHECK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// Flush budget when stopping a node we ATTACHED to rather than spawned. Must
/// match the managed path's grace: after a self-update relaunch the app is
/// always attached (it adopted the orphan), so this is the budget most quits
/// actually use.
const ATTACHED_STOP_GRACE: std::time::Duration =
    std::time::Duration::from_secs(btx_core::node::SHUTDOWN_GRACE_SECS);
/// Backstop on the whole graceful quit: a wedged btxd must never turn "quit"
/// back into a force-quit. Strictly larger than the stop grace, so a healthy
/// flush finishes on its own terms rather than being cut off by the backstop.
const QUIT_GRACE: std::time::Duration = std::time::Duration::from_secs(95);

/// Single writer for the phase: also mirrors it onto the tray, so tray text
/// can never go stale (reflecting at call sites proved easy to forget — the
/// setup pipeline's Downloading/Preparing phases never reached the tray).
async fn set_phase(app: &AppHandle, state: &AppState, phase: NodePhase) {
    *state.phase.lock().await = phase.clone();
    crate::tray::reflect_phase(app, &phase);
}

/// One quick probe: does a node already answer RPC against this datadir?
/// (Shared-datadir reality: a previous run of this app, or any btxd someone
/// started by hand, may already be serving — attaching beats churning it.)
async fn rpc_already_answering(datadir: &Path) -> Option<RpcClient> {
    let cookie = datadir.join(".cookie");
    let client = RpcClient::from_cookie(RPC_URL, &cookie).ok()?;
    get_blockchain_info(&client).await.ok()?;
    Some(client)
}

/// How `start_node_inner` must reconcile with whatever btxd may already hold
/// the shared datadir, BEFORE it launches from the tag it resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreLaunchPlan {
    /// A node is serving RPC with the binaries we'd launch (or a live app
    /// owns it): use it as-is.
    Attach,
    /// A node is serving RPC, but THIS start just provisioned new binaries
    /// under it (tag migration): stop it gracefully, wait for the datadir
    /// lock, then spawn from the new tag.
    RestartForUpgrade,
    /// No RPC answer, but an ORPHANED btxd still holds the datadir (mid-
    /// shutdown flush, or busy past the probe timeout): stop/wait it out,
    /// then spawn — never race its `.lock`.
    ClearStaleHolder,
    /// No RPC answer and the holder is PROVEN to be a btxd belonging to a live
    /// parent app (the miner's solo node, or another instance of this app),
    /// and it stayed that way across the whole recheck budget: hands off —
    /// error out honestly instead of stopping it or racing its lock.
    ManagedElsewhereNoRpc,
    /// No RPC answer and a holder we must not disturb — but not one we have
    /// watched long enough to be sure of. Wait `HOLDER_RECHECK_WAIT` and
    /// evaluate the whole picture again.
    RecheckHolder,
    /// The recheck budget is spent and `btxd.pid` still names a live process we
    /// could not identify: launch anyway. See `pre_launch_plan` for why this
    /// beats standing down.
    AdoptUnprovenHolder,
    /// Nothing holds the datadir: plain spawn.
    SpawnFresh,
}

/// The tag-migration + attach decision, pure and regression-tested.
///
/// THE 2026-08-12 REGRESSION (0.6.1 → 0.6.2 self-update, dev M2 Pro): after
/// the updater relaunched the app, the tag migration provisioned the new
/// btxd, and the old btxd — still healthy, orphaned by the relaunch as
/// designed — held `~/.easybtx`. The old code attached whenever RPC answered
/// (running pre-upgrade binaries forever), and when the probe missed (the old
/// node was busy validating post-fork blocks past the RPC timeout, or already
/// mid-shutdown) it spawned straight into the held lock: the new btxd died
/// ("Cannot obtain a lock…"), the old one was stopped by `stop_stale`'s
/// side-effect, and NOTHING was left running until a manual restart.
///
/// THE 2026-09-04 STAND-DOWN (Linux signer rig). The holder used to be a pair
/// of booleans derived from `kill(pid, 0)` on `<datadir>/btxd.pid`. After a
/// restart that file still read 717 from a btxd that had died without cleaning
/// up, the OS had recycled 717 onto an unrelated process, and so a start with
/// no btxd on the machine at all — nothing listening on 19334 — planned
/// `ManagedElsewhereNoRpc`, returned an error naming an app that was not
/// running, and did it again on every subsequent start. Two things were wrong,
/// and both are fixed here:
///
///   * The holder was COUNTED, not identified. `btx_core::node::datadir_holder`
///     now requires the pid to be a live process actually named btxd, the same
///     standard `stop_stale` and the force-kill path already applied before
///     they signal anything. A recycled pid classifies as `Free` and we launch.
///   * The refusal was PERMANENT. `ManagedElsewhereNoRpc` disarms the launch
///     record and returns `Err`, so nothing retried; the app's own message
///     ("give it a moment") described a wait it never performed. A holder we
///     must not disturb now buys a bounded wait — `RecheckHolder`, at most
///     `HOLDER_RECHECKS` looks — and the decision is taken again each time,
///     because "another app's node is still warming up" is a state that ends.
///
/// WHAT HAPPENS WHEN THE BUDGET RUNS OUT depends on what we could prove, and
/// this is the deliberate part:
///
///   * A PROVEN live app's btxd keeps the hands-off refusal. That is not a
///     failure to recover: bouncing the miner's solo node would fight its own
///     recovery supervisor, and the message is now true when it prints, so the
///     user has something they can act on.
///   * An UNIDENTIFIABLE holder (`ps`/`tasklist` could not name a live pid) is
///     adopted instead — `AdoptUnprovenHolder`, which spawns. We refuse to keep
///     a home node off the network on the strength of a number we could not
///     even attach a name to. The downside is bounded and self-announcing: if
///     some real btxd does hold the lock, our spawn loses the race, says so,
///     and `spawn_node_with_lock_retry` retries. Never starting is the only
///     failure with no way out. Note what this does NOT do: no process is ever
///     stopped, signalled or killed on the strength of an unproven holder.
///
/// Inputs:
///   - `rpc_answering`: a node answered `getblockchaininfo` on this datadir.
///   - `upgraded_this_start`: THIS call provisioned new-tag binaries (the
///     persisted tag moved to `NODE_RELEASE_TAG`); a serving node therefore
///     runs the OLD binaries and must be restarted to pick the upgrade up.
///   - `holder`: what actually holds the datadir, identified — see
///     [`DatadirHolder`]. An unidentifiable holder counts as hands-off for the
///     upgrade-restart question too, which is a small deliberate change: we no
///     longer bounce a serving node on behalf of an upgrade when we cannot name
///     the process holding its pidfile. The upgrade lands on its next restart.
///   - `rechecks_exhausted`: the caller has already spent its `RecheckHolder`
///     budget on this start, so this decision is final.
pub(crate) fn pre_launch_plan(
    rpc_answering: bool,
    upgraded_this_start: bool,
    holder: DatadirHolder,
    rechecks_exhausted: bool,
) -> PreLaunchPlan {
    // A holder we must not disturb: a btxd another live app supervises, or one
    // we could not identify at all.
    let hands_off = matches!(
        holder,
        DatadirHolder::ManagedBtxd { .. } | DatadirHolder::Unidentifiable { .. }
    );
    if rpc_answering {
        if upgraded_this_start && !hands_off {
            PreLaunchPlan::RestartForUpgrade
        } else {
            PreLaunchPlan::Attach
        }
    } else {
        match holder {
            DatadirHolder::Free => PreLaunchPlan::SpawnFresh,
            DatadirHolder::OrphanedBtxd { .. } => PreLaunchPlan::ClearStaleHolder,
            _ if !rechecks_exhausted => PreLaunchPlan::RecheckHolder,
            DatadirHolder::ManagedBtxd { .. } => PreLaunchPlan::ManagedElsewhereNoRpc,
            DatadirHolder::Unidentifiable { .. } => PreLaunchPlan::AdoptUnprovenHolder,
        }
    }
}

/// Spawn (or attach to) the node and bring the app to a running state:
/// RPC client armed, snapshot-load guaranteed in the background, keep-awake
/// held, and the status refresher loop driving the phase.
pub(crate) async fn start_node_inner(app: &AppHandle, state: &AppState) -> Result<(), String> {
    // One start at a time, held through a drop guard so a panic releases it.
    // The double-spawn guard further down keys on a live child in
    // `state.node`, and there is none between a stop and the next spawn,
    // during which `Stopped` is actionable on both surfaces. Two starts in
    // that window race for btxd.pid; the loser lands on ManagedElsewhereNoRpc,
    // disarms the shared launch record, and the winner's healthy btxd gets
    // SIGKILLed at the next stop for want of it.
    if state
        .start_in_flight
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("the node is already starting, give it a moment".to_string());
    }
    struct InFlight<'a>(&'a std::sync::atomic::AtomicBool);
    impl Drop for InFlight<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _in_flight = InFlight(&state.start_in_flight);

    let datadir = node_datadir();
    let settings = NodeAppSettings::load(&datadir);
    let mut tag = settings
        .btx_release_tag
        .clone()
        .unwrap_or_else(|| NODE_RELEASE_TAG.to_string());
    let tag_before = tag.clone();
    let mut upgraded_this_start = false;

    // Node upgrade path for RETURNING users: when an app update ships a newer
    // bundled node package (NODE_RELEASE_TAG moved, e.g. for a network flag
    // day), provision it before launch — otherwise the persisted tag keeps
    // resolving the old binaries forever and the node forks off at the flag
    // height while the UI shows Ready.
    if tag != NODE_RELEASE_TAG {
        let resource_dir = app.path().resource_dir().ok();
        if let Some(pkg) = resolve_bundled_node_pkg(
            resource_dir.as_deref(),
            &[Path::new(env!("CARGO_MANIFEST_DIR"))],
        ) {
            if let Some(install_root) = install_dir(NODE_RELEASE_TAG) {
                // Which conf goes in is a DISK question before it is a profile
                // question. This path rewrites faststart.conf wholesale and used
                // to do so with no free-space check at all, so a keeper who had
                // switched to Full and then took an app update got `prune=0`
                // written here and started backfilling ~124 GiB, silently, into
                // a disk that had held 25. The engine upgrade itself must still
                // land (it is what keeps the node on the right side of a flag
                // day), so when the disk cannot take the posture change the
                // datadir keeps the posture it has, and the change is deferred
                // with a line in setup.log saying so.
                let selected =
                    btx_core::installer::conf_for_profile(&settings.node_profile, NODE_RELEASE_TAG);
                let conf_path = datadir.join("faststart").join("faststart.conf");
                let was_pruned = btx_core::setup::conf_is_pruned(&conf_path);
                let need = btx_core::setup::disk_required_for_conf(
                    datadir.join("blocks").exists(),
                    was_pruned,
                    selected != btx_core::installer::NODE_FASTSTART_CONF,
                );
                let disk_ok = match free_disk_bytes(&datadir) {
                    Some(free) => enough_free_disk(free, need),
                    None => true, // unmeasured never blocks, as in the preflight
                };
                let choice = upgrade_conf_choice(selected, was_pruned, disk_ok);
                if choice != Some(selected) {
                    let gib = 1024 * 1024 * 1024;
                    let msg = format!(
                        "profile change to '{}' deferred: it needs about {} GiB free and this \
                         volume has {} GiB; the node keeps its current prune posture",
                        settings.node_profile,
                        need / gib,
                        free_disk_bytes(&datadir).unwrap_or(0) / gib
                    );
                    eprintln!("[node-app] {msg}");
                    setup_log(&datadir, &msg);
                }
                let dd = datadir.clone();
                let provisioned = match choice {
                    Some(conf) => tauri::async_runtime::spawn_blocking(move || {
                        btx_core::installer::provision_node_package(&pkg, &install_root, &dd, conf)
                    })
                    .await
                    .map_err(|e| format!("upgrade provisioning panicked: {e}"))?,
                    None => Err(btx_core::error::AppError::Config(
                        "this engine cannot keep the datadir pruned and there is not enough \
                         disk to un-prune it; leaving the installed engine alone"
                            .to_string(),
                    )),
                };
                match provisioned {
                    Ok(_) => {
                        eprintln!(
                            "[node-app] provisioned node binaries {tag} → {NODE_RELEASE_TAG}"
                        );
                        // A new engine may ship the ExactReplay golden this Mac
                        // was missing, so re-measure instead of inheriting the
                        // old verdict. Without this a Mac downgraded to a
                        // trusted mirror once would stay one across every
                        // future upgrade, silently declining to be the
                        // independent validator it had become capable of being.
                        btx_core::node::clear_matmul_consensus_refused(&datadir);
                        // The persisted tag flips only after the new-tag node
                        // actually RUNS (end of this function). If this start
                        // dies half-way (quit mid-stop, lost lock race), the
                        // next start re-enters this branch — re-provisioning
                        // is an idempotent ~25 MB copy — instead of treating
                        // a leftover old-binaries node as already upgraded.
                        // (The 2026-08-12 0.6.1→0.6.2 failure mode; see
                        // `pre_launch_plan`.)
                        upgraded_this_start = true;
                        tag = NODE_RELEASE_TAG.to_string();
                    }
                    Err(e) => {
                        // Non-fatal: keep launching the old, known-good tag.
                        eprintln!("[node-app] node upgrade provisioning failed (using {tag}): {e}");
                    }
                }
            }
        }
    }

    let paths: FaststartResult = returning_launch_paths(&datadir, &tag)
        .ok_or_else(|| "node binaries not installed yet — run setup first".to_string())?;

    // Double-spawn guard: a controller whose child is still alive means a
    // previous start is in flight (e.g. a long warmup). Overwriting it would
    // DROP the old controller and kill_on_drop would SIGKILL the warming btxd
    // mid-rebuild — the exact corruption the graceful paths exist to avoid.
    if let Some(controller) = state.node.lock().await.as_mut() {
        if controller.child_has_exited() == Some(false) {
            return Err(
                "the node is still starting up (this can take a while after an unclean shutdown) — give it a moment"
                    .to_string(),
            );
        }
    }

    // Bootstrap peers belong in the conf on every start (idempotent) so even a
    // hand-started btxd against this datadir reaches the sparse BTX network.
    let _ = ensure_addnodes_in_conf(&paths.faststart_conf, BTX_BOOTSTRAP_PEERS);

    // Archive peers + their noban whitelist: on a trusted mirror these ARE the
    // sync path — `IsTrustedMirrorAuthorityPeer()` only asks manual/noban
    // archive peers for attestations, and `fPreferredDownload` (block download)
    // runs the same check. Without these lines a post-#331 trusted mirror can
    // sit at a frozen height with healthy-looking peers (the api.btxscan.io
    // incident class, 2026-08-14..17). Idempotent, asserted on every start.
    //
    // The whitelist is a MANAGED BLOCK, not an append: noban is a security
    // grant and must stay revocable — the block is rewritten each start to
    // the pinned IPs plus a live DNS resolution of the hostname archives, so
    // an address that leaves the census loses its grant on the next start
    // (see set_managed_whitelist_block). DNS runs off the executor.
    let _ = ensure_addnodes_in_conf(&paths.faststart_conf, btx_core::node::BTX_ARCHIVE_PEERS);

    // ...and then PRUNE, which is the half that was missing. The two ensure
    // calls above only ever append, so until now the peer set btxd dialled was
    // the union of every census we have ever shipped: adding a seed reached
    // existing installs, retiring one never did. Measured on a real install on
    // 0.6.17, the release whose whole point was the corrected census, the conf
    // still carried the stale branch seed 0.6.14 removed.
    //
    // The whitelist a few lines below already had this property and says so:
    // "an address that leaves the census loses its grant on the next start".
    // The addnode list now behaves the same way, so a seat we give up is
    // actually given up.
    let retired: Vec<&str> = BTX_BOOTSTRAP_PEERS
        .iter()
        .chain(btx_core::node::BTX_ARCHIVE_PEERS.iter())
        .copied()
        .collect();
    let pruned = btx_core::setup::prune_retired_addnodes_in_conf(&paths.faststart_conf, &retired);
    if pruned > 0 {
        eprintln!(
            "[node-app] conf: dropped {pruned} retired addnode line(s) left by an earlier version"
        );
    }

    let whitelist_ips =
        tauri::async_runtime::spawn_blocking(btx_core::node::resolve_archive_whitelist_ips)
            .await
            .unwrap_or_else(|_| {
                btx_core::node::BTX_ARCHIVE_WHITELIST_IPS
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            });
    let _ = btx_core::setup::set_managed_whitelist_block(&paths.faststart_conf, &whitelist_ips);

    // Re-assert the app-owned conf keys on EVERY start, not just after setup.
    //
    // Before the opt-in keys below: put back anything missing from the BASE
    // conf. Nothing on this path used to do that. `provision_node_package`
    // writes faststart.conf once — at first setup, or when the pinned tag moves
    // — and after that the only writers read-modify-rewrite it, plus the miner
    // through the shared datadir. So a conf that loses its body never got it
    // back, and btxd launched with the datadir's own remembered `prune` value
    // and no reorg parking, silently and with nothing on screen.
    //
    // Only ever ADDS a missing key; a present value is never overwritten, so a
    // keeper stays pruned and a hand-tuned value survives. See
    // `ensure_base_conf_keys` for why these five and not the whole conf.
    // `&tag`, not NODE_RELEASE_TAG: the binaries this start launches come from
    // `tag`, which only advances to the pin when this start's provisioning
    // succeeded. Judging the keeper gate against an engine that is not
    // installed could restore prune=10000 for one of the four engines the gate
    // exists to keep the pruned conf away from.
    match btx_core::setup::ensure_base_conf_keys(
        &paths.faststart_conf,
        btx_core::installer::conf_for_profile(&settings.node_profile, &tag),
    ) {
        Ok(added) if !added.is_empty() => {
            // Loud on purpose. Healing this silently would hide a conf that is
            // being damaged repeatedly by something we have not found yet.
            eprintln!(
                "[node-app] faststart.conf was missing {} base key(s), restored: {}",
                added.len(),
                added.join(" ")
            );
            setup_log(
                &datadir,
                &format!("restored missing conf keys: {}", added.join(" ")),
            );
        }
        Ok(_) => {}
        Err(e) => eprintln!("[node-app] could not restore base conf keys: {e}"),
    }

    // `provision_node_package` writes faststart.conf unconditionally, so the
    // node-upgrade path above (which fires whenever NODE_RELEASE_TAG moves —
    // e.g. v0.33.1 → v0.33.2 for the MatMul v4.7 fork) silently drops
    // `txindex=1` while `NodeAppSettings.txindex_enabled` stays true. The user
    // then sees Explorer mode ON in Settings while `ask_transaction` answers
    // "not found" for transactions that plainly exist. Re-applying here is
    // idempotent, costs one small file read, and also self-heals a conf the
    // MINER rewrote through the shared ~/.easybtx datadir.
    //
    // Deliberately NOT part of NODE_FASTSTART_CONF: Explorer mode is opt-in,
    // and the default conf must stay the lean one.
    if let Err(e) = btx_core::setup::set_conf_kv(
        &paths.faststart_conf,
        "txindex",
        settings.txindex_enabled.then_some("1"),
    ) {
        // Never fatal: a node that starts without the index is still a node.
        eprintln!("[node-app] could not re-apply txindex to the conf: {e}");
    }

    // The public nickname, asserted for the same reason as txindex above: a
    // provision or an engine upgrade rewrites faststart.conf wholesale and
    // would otherwise silently drop it, so a user who named their node would
    // quietly go back to being anonymous at the next release. Empty removes the
    // key rather than writing `uacomment=`, which btxd renders as `()`.
    //
    // Two rules, both learned the hard way in this same function. The value is
    // VALIDATED here, not only in the Settings command: this is the path that
    // runs unattended on every launch, and a settings file edited by hand (or
    // by the miner, which shares the datadir) is the one route by which a
    // comment btxd rejects could reach the conf and stop the node starting,
    // persistently, with nothing on screen naming the cause. And an EMPTY
    // setting leaves the key alone rather than deleting it: the attestation
    // block below was fixed for exactly that asymmetry, which silently stripped
    // an operator's hand-added value on every start. Clearing a nickname is
    // done explicitly by `set_node_nickname`, where it belongs.
    match conf_nickname(&settings.node_nickname) {
        Some(nick) => {
            if let Err(e) =
                btx_core::setup::set_conf_kv(&paths.faststart_conf, "uacomment", Some(&nick))
            {
                // Never fatal: a node without its nickname is still a node.
                eprintln!("[node-app] could not re-apply the nickname to the conf: {e}");
            }
        }
        None if !settings.node_nickname.trim().is_empty() => {
            setup_log(
                &datadir,
                "the saved nickname is not one btxd would accept; not writing it to the conf",
            );
            eprintln!(
                "[node-app] saved nickname {:?} rejected by the validator; leaving uacomment alone",
                settings.node_nickname
            );
        }
        None => {}
    }

    // Attestation serving (opt-in): asserted like txindex so the provision/
    // upgrade path can't silently drop it — but NEVER deleted here. The old
    // `set_conf_kv(.., None)` call REMOVED the key whenever the setting was
    // off, which silently stripped a hand-added `matmulattestationserve=1`
    // from operators' confs on every start. A hand-set flag is instead
    // ADOPTED into settings, so the UI and the service report tell the truth;
    // the only place the key is ever removed is the explicit Settings toggle
    // (`set_attestation_serve`).
    let conf_serve_on = btx_core::setup::conf_kv(&paths.faststart_conf, "matmulattestationserve")
        .map(|v| v != "0")
        .unwrap_or(false);
    let mut serve_on = settings.attestation_serve_enabled;
    if conf_serve_on && !serve_on {
        NodeAppSettings::update(&datadir, |s| s.attestation_serve_enabled = true);
        serve_on = true;
        eprintln!("[node-app] adopted matmulattestationserve=1 from the conf into settings");
    }
    if serve_on {
        if let Err(e) =
            btx_core::setup::set_conf_kv(&paths.faststart_conf, "matmulattestationserve", Some("1"))
        {
            eprintln!("[node-app] could not re-apply matmulattestationserve to the conf: {e}");
        }
    }

    // Record the stop paths BEFORE anything can fail: if the RPC wait times
    // out below, quit must still be able to stop btxd gracefully (btx-cli
    // stop + the 90 s flush grace) instead of dropping into a SIGKILL.
    *state.launch.lock().await = Some((paths.btx_cli.clone(), datadir.clone()));

    // A new run re-qualifies from scratch (different hardware state, a repaired
    // Metal toolchain, a different -matmulrcexecution): never inherit the last
    // run's verdict. Same rule for the stall verdict and the peer census — a
    // previous run's stall verdict otherwise survives a manual stop/start and
    // renders as a CURRENT stall on a node that just booted.
    *state.rc_status_cache.lock().await = None;
    *state.stall_verdict.lock().await = None;
    *state.archive_peers_cache.lock().await = None;
    state.peer_nicknames_cache.lock().await.clear();
    *state.archive_service.lock().await = None;

    set_phase(app, state, NodePhase::Starting).await;

    // Reconcile with whatever btxd may already hold the shared datadir. Two
    // observed facts feed ONE pure, regression-tested decision
    // (`pre_launch_plan`) — see its doc comment for the two failures this
    // exists to prevent.
    //
    // The loop is the second half of the 2026-09-04 fix: a holder we must not
    // disturb gets looked at again rather than ending the start on the spot.
    // Both facts are re-read every pass, because either can change — the other
    // app's node finishes warming up and answers RPC, or its btxd exits.
    let mut rechecks = 0u32;
    let (plan, probe, holder) = loop {
        let probe = rpc_already_answering(&datadir).await;
        let holder = btx_core::node::datadir_holder(&datadir).await;
        let plan = pre_launch_plan(
            probe.is_some(),
            upgraded_this_start,
            holder,
            rechecks >= HOLDER_RECHECKS,
        );
        if plan != PreLaunchPlan::RecheckHolder {
            if rechecks > 0 {
                set_phase(app, state, NodePhase::Starting).await;
            }
            break (plan, probe, holder);
        }
        rechecks += 1;
        eprintln!(
            "[node-app] {holder:?} holds this datadir and nothing is answering RPC yet; \
             looking again in {}s ({rechecks}/{HOLDER_RECHECKS})",
            HOLDER_RECHECK_WAIT.as_secs()
        );
        // A silent "Starting" through this wait reads as a hang. Say what is
        // being waited for, the same way the lock-race wait does.
        set_phase(
            app,
            state,
            NodePhase::Warming {
                message: "Waiting for the node already using this folder…".to_string(),
            },
        )
        .await;
        tokio::time::sleep(HOLDER_RECHECK_WAIT).await;
    };

    let rpc = match plan {
        PreLaunchPlan::Attach => {
            // Invariant: `pre_launch_plan` only answers Attach when the probe
            // answered (shared datadir: never spawn a second daemon).
            let client = probe.expect("Attach implies an answering RPC probe");
            eprintln!("[node-app] a node is already serving RPC on this datadir; attaching");
            // Remember WHOSE node this is. `state.rpc` will hold its client
            // from here on, and that slot is what the destructive commands
            // used to read as "ours", which in this arm is exactly wrong.
            *state.attached_to.lock().await = Some(match holder {
                DatadirHolder::ManagedBtxd { .. } => AttachedTo::AnotherApp,
                DatadirHolder::OrphanedBtxd { .. } => AttachedTo::OurOrphan,
                DatadirHolder::Free | DatadirHolder::Unidentifiable { .. } => AttachedTo::Unknown,
            });
            if upgraded_this_start {
                // Only reachable when the serving node is another live app's:
                // never bounce a node out from under its supervisor. The new
                // binaries are provisioned; they apply on its next restart.
                eprintln!(
                    "[node-app] node upgrade to {NODE_RELEASE_TAG} provisioned, but a live app \
                     (the miner?) manages the running node — leaving it; the upgrade applies \
                     on that node's next restart"
                );
            }
            client
        }
        PreLaunchPlan::RecheckHolder => {
            unreachable!("the loop above only breaks on a final plan")
        }
        PreLaunchPlan::ManagedElsewhereNoRpc => {
            // Reachable only for a holder we PROVED is a btxd with a live
            // parent app, and only after the whole recheck budget: every word
            // of the message below is now something we checked.
            let DatadirHolder::ManagedBtxd { pid } = holder else {
                unreachable!("ManagedElsewhereNoRpc is only planned for a proven btxd holder")
            };
            let waited = HOLDER_RECHECKS as u64 * HOLDER_RECHECK_WAIT.as_secs();
            eprintln!(
                "[node-app] a live app's btxd (pid {pid}) holds this datadir and did not answer \
                 RPC in {waited}s; standing down instead of stopping it or racing its lock"
            );
            // Hands-off includes quit: with the launch record armed, quitting
            // after this error would gracefully stop the OTHER app's node via
            // the attached-mode stop path. We never adopted it — disarm.
            *state.launch.lock().await = None;
            return Err(format!(
                "another easyBTX app (the miner, or a second window of this app) is running the \
                 node in this folder — btxd, process {pid} — and it did not answer in {waited}s. \
                 Give it another moment, or quit that app and try again"
            ));
        }
        PreLaunchPlan::RestartForUpgrade
        | PreLaunchPlan::ClearStaleHolder
        | PreLaunchPlan::AdoptUnprovenHolder
        | PreLaunchPlan::SpawnFresh => {
            match plan {
                PreLaunchPlan::RestartForUpgrade => eprintln!(
                    "[node-app] node upgrade: a node from the previous binaries ({tag_before}) is \
                     still serving this datadir — stopping it gracefully, then launching {NODE_RELEASE_TAG}"
                ),
                PreLaunchPlan::ClearStaleHolder => eprintln!(
                    "[node-app] a btxd nobody manages still holds this datadir without answering \
                     RPC (mid-shutdown, or busy past the probe timeout) — stopping it before launch"
                ),
                PreLaunchPlan::AdoptUnprovenHolder => eprintln!(
                    "[node-app] btxd.pid names a live process we could not identify and nothing \
                     answered RPC in {}s — launching rather than refusing forever. If a real btxd \
                     holds the lock this spawn loses the race and says so, which is recoverable; \
                     never starting is not",
                    HOLDER_RECHECKS as u64 * HOLDER_RECHECK_WAIT.as_secs()
                ),
                _ => {}
            }
            // A node we spawn is ours by construction.
            *state.attached_to.lock().await = None;
            spawn_node_with_lock_retry(app, state, &datadir, &paths).await?
        }
    };

    *state.rpc.lock().await = Some(rpc.clone());
    *state.started_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());

    // The upgrade is DONE only now — a node launched from the new tag (or a
    // fresh spawn of it) is serving. Attach-mode never flips: the running node
    // still carries the old binaries, so the next start must re-reconcile.
    if upgraded_this_start && plan != PreLaunchPlan::Attach {
        NodeAppSettings::update(&datadir, |s| {
            s.btx_release_tag = Some(NODE_RELEASE_TAG.to_string())
        });
        eprintln!("[node-app] node upgrade complete: {tag_before} → {NODE_RELEASE_TAG} is running");
    }

    // Keep-awake while the node runs (user-toggleable).
    if NodeAppSettings::load(&datadir).keep_awake {
        *state.sleep_guard.lock().unwrap_or_else(|e| e.into_inner()) = Some(
            btx_core::power::SleepAssertion::hold("easyBTX Node is supporting the BTX network"),
        );
    }

    // Guarantee the snapshot gets loaded (background, idempotent, best-effort).
    let spec = snapshot_spec();

    // UPGRADE PATH. begin_setup is the only caller of download_snapshot, and an
    // existing install never re-enters it, so a release that repins the snapshot
    // leaves the PREVIOUS release's snapshot.dat on disk while anchor_height has
    // already moved. ensure_snapshot_loaded would then wait for headers to reach
    // the NEW anchor and load the OLD file, which is strictly worse than not
    // repinning. Measured upgrading a live 0.6.12 to 0.6.13 on 2026-08-31: the
    // 452 MB height-179000 asset was still there, anchor already 203000.
    //
    // Refresh it here, BEFORE ensure_snapshot_loaded, because that function
    // returns early when snapshot.dat is absent, so the correct file has to be in
    // place first. Awaited rather than spawned for the same ordering reason. A
    // failure is logged and startup continues: the node still syncs the slow way,
    // which is exactly what it would have done with no snapshot at all.
    //
    // ...but not for a node that has already loaded one. `ensure_snapshot_loaded`
    // returns on its fast path when `snapshot_loaded` is set, so nothing would
    // ever read the file we just fetched: the download is ~452 MB spent to
    // produce a file the very next call ignores, on every start until the pin
    // moves again. The staleness refresh exists to make the LOAD correct, so it
    // is only worth doing when a load can still happen.
    let already_loaded = NodeAppSettings::load(&datadir).snapshot_loaded;
    if already_loaded && btx_core::snapshot::snapshot_file_is_stale_for_spec(&spec, &datadir) {
        eprintln!(
            "[snapshot] snapshot.dat is from an older pin, but this node has \
             already loaded one; not re-downloading a file nothing would read"
        );
    }
    if !already_loaded && btx_core::snapshot::snapshot_file_is_stale_for_spec(&spec, &datadir) {
        eprintln!(
            "[snapshot] snapshot.dat on disk is not the pinned one (anchor {}); refreshing",
            spec.anchor_height
        );
        btx_core::snapshot::clear_snapshot_marker(&datadir);
        if let Err(e) = btx_core::snapshot::download_snapshot(&spec, &datadir, &|_| {}).await {
            eprintln!("[snapshot] refresh failed ({e}); continuing without fast start");
        }
    }

    btx_core::snapshot::ensure_snapshot_loaded(
        rpc.clone(),
        paths.btx_cli.clone(),
        datadir.clone(),
        spec.anchor_height,
        Arc::new(NodeAppSnapshotFlags {
            datadir: datadir.clone(),
        }),
    );

    set_phase(app, state, NodePhase::LoadingSnapshot).await;
    spawn_status_refresher(app.clone(), state);
    Ok(())
}

/// Spawn btxd with the datadir-lock discipline the 2026-08-12 self-update
/// failure demanded: make sure any unmanaged holder has actually RELEASED the
/// lock (graceful stop + bounded wait, force-kill only as a last resort),
/// spawn, then watch the child briefly — a btxd that lost a residual lock
/// race dies in under a second, and burning the full 3-minute RPC wait on a
/// dead child is exactly how the old flow ended with no node running at all.
/// Bounded attempts, then an honest error.
async fn spawn_node_with_lock_retry(
    app: &AppHandle,
    state: &AppState,
    datadir: &Path,
    paths: &FaststartResult,
) -> Result<RpcClient, String> {
    for attempt in 1..=LAUNCH_ATTEMPTS {
        // A quit that started mid-retry must win: spawning after the graceful
        // quit's stop pass has already run would orphan a fresh btxd.
        if state.quitting.load(Ordering::SeqCst) {
            return Err("the app is quitting — not starting the node".to_string());
        }
        // Whether this attempt had to clear a holder decides two things: the
        // long stop-wait below, and whether the post-spawn survival watch runs
        // at all — a CLEAN datadir can't lose a lock race, so the ordinary
        // boot path keeps its instant hand-off to the RPC wait.
        // Identified, not counted (2026-09-04): a `btxd.pid` whose number has
        // been recycled onto an unrelated process would otherwise cost every
        // attempt the full stop grace — minutes of waiting out a daemon that
        // does not exist — before a spawn that was safe from the first second.
        let raced_holder = btx_core::node::datadir_holder(datadir).await != DatadirHolder::Free;
        if raced_holder {
            eprintln!(
                "[node-app] waiting out the btxd holding this datadir (attempt \
                 {attempt}/{LAUNCH_ATTEMPTS}, up to {}s before force-kill)…",
                UNMANAGED_STOP_GRACE.as_secs()
            );
            // The wait can legitimately run minutes (the previous node's disk
            // flush); a silent "Starting" that long reads as a hang. Surface
            // it the same calm way btxd's own warmup is surfaced.
            set_phase(
                app,
                state,
                NodePhase::Warming {
                    message: "Waiting for the previous node to finish shutting down…".to_string(),
                },
            )
            .await;
            btx_core::node::stop_unmanaged_node(datadir, &paths.btx_cli, UNMANAGED_STOP_GRACE)
                .await;
            set_phase(app, state, NodePhase::Starting).await;
        }

        // Automatic housekeeping on a fresh spawn (btxd is down right here, so
        // the LevelDB deletes and debug.log truncation can't race a writer):
        // strip unused indexes, sweep a loaded snapshot.dat, cap the log. Same
        // launch-reclaim habit as the miner. First attempt only — a retry's
        // job is the lock, not more housekeeping.
        if attempt == 1 {
            let dd = datadir.to_path_buf();
            let conf = paths.faststart_conf.clone();
            let loaded = NodeAppSettings::load(datadir).snapshot_loaded;
            let report = tauri::async_runtime::spawn_blocking(move || {
                btx_core::disk::reclaim_disk(&dd, &conf, loaded)
            })
            .await
            .unwrap_or_default();
            if report.freed_mb > 0 || !report.items.is_empty() {
                eprintln!(
                    "[node-app] auto-reclaimed {} MB on start ({})",
                    report.freed_mb,
                    report.items.join(", ")
                );
            }
        }

        let mut controller = NodeController::new();
        controller
            .start(
                &paths.btxd,
                datadir,
                &paths.faststart_conf,
                node_backend(),
                &paths.btx_cli,
            )
            .await
            .map_err(|e| format!("couldn't start the node: {e}"))?;
        // Park the controller in the shared slot BEFORE the survival watch so
        // a quit landing inside the watch still finds — and gracefully stops —
        // the child instead of orphaning it.
        *state.node.lock().await = Some(controller);

        // Watch the child on EVERY attempt, not just one that cleared a holder.
        //
        // This used to be `if !raced_holder { true }`, on the reasoning that a
        // clean datadir cannot lose a lock race. True, and beside the point: a
        // lock race is not the only thing that kills btxd inside a second. An
        // engine that refuses to start as a MatMul consensus validator exits
        // during init too, and skipping the watch sent that case into the full
        // RPC wait — three silent minutes ending in "the node didn't become
        // ready", which names neither the cause nor the fix.
        //
        // The cost is bounded and small: `LAUNCH_SURVIVAL_WATCH` (5s) against
        // an RPC budget of 180s plus warmup, and only on a start that is
        // already underway. This function's own doc comment argued for exactly
        // this ("burning the full 3-minute RPC wait on a dead child is exactly
        // how the old flow ended with no node running at all"); the code just
        // scoped it too narrowly.
        let survived = {
            let mut guard = state.node.lock().await;
            match guard.as_mut() {
                Some(c) => {
                    btx_core::node::child_survives_launch_watch(c, LAUNCH_SURVIVAL_WATCH).await
                }
                // The slot emptied under us: a stop/quit took the node — done.
                None => return Err("the node was stopped while it was starting".to_string()),
            }
        };
        if survived {
            spawn_warmup_watcher(app.clone(), state, datadir.to_path_buf());
            return wait_for_node_rpc(
                datadir,
                RPC_URL,
                RPC_WAIT_POLLS,
                RPC_WAIT_POLL_MS,
                RPC_WAIT_WARMUP_POLLS,
            )
            .await
            .map_err(|e| {
                format!(
                    "the node didn't become ready: {e}. \
                     See easybtx-node.log in {} for details.",
                    datadir.display()
                )
            });
        }
        *state.node.lock().await = None;

        // Did the engine refuse to be an independent MatMul consensus
        // validator on this machine? That is not a lock race and retrying it
        // unchanged fails identically three times. Record the verdict so
        // `build_node_command` adds the trusted-mirror flags, and let the loop
        // take the next attempt — which is the one that starts.
        //
        // Measured on an Apple M5 (Mac17,2) against btxd v0.33.4.1: upstream's
        // golden manifest carries `metal|m4_class` and nothing for `m5_class`,
        // so consensus mode exits during init while the same binary with
        // `-matmulvalidation=trusted` reaches "Done loading". v0.33.3 behaves
        // the same, so this covers the engine already in the field as well.
        let tail = btx_core::node::node_log_tail(datadir, 64 * 1024);
        if btx_core::node::log_shows_matmul_consensus_refused(&tail)
            && !btx_core::node::matmul_consensus_was_refused(datadir)
        {
            btx_core::node::record_matmul_consensus_refused(datadir);
            eprintln!(
                "[node-app] this Mac has no reviewed ExactReplay golden in the bundled \
                 engine, so btxd refused to start as an independent consensus validator. \
                 Retrying as a trusted mirror (attempt {attempt}/{LAUNCH_ATTEMPTS})."
            );
            continue;
        }

        eprintln!(
            "[node-app] btxd exited within {}s of spawning, attempt \
             {attempt}/{LAUNCH_ATTEMPTS}. Cause read from its log: {}",
            LAUNCH_SURVIVAL_WATCH.as_secs(),
            btx_core::node::launch_failure_hint(&tail).unwrap_or("not recognised"),
        );
    }
    // Report what btxd's own log says, not a guess.
    //
    // This used to end on a fixed sentence blaming the datadir lock, on every
    // early exit, whether or not anything had ever looked at a lock. On
    // 2026-08-31 that sentence was wrong on a real install: nothing held the
    // lock, and btxd had refused because the datadir was pruned while the conf
    // asked to keep every block. btxd had printed both the cause and the fix;
    // the app overwrote them with a cause it had not checked.
    //
    // Re-read the tail here because the one inside the loop is scoped to the
    // attempt that produced it, and the last attempt is the one worth quoting.
    let tail = btx_core::node::node_log_tail(datadir, 64 * 1024);
    let cause = btx_core::node::launch_failure_hint(&tail)
        .unwrap_or("its log does not say why in a way this app recognises.");
    Err(format!(
        "the node kept exiting right after launch: {cause} \
         See easybtx-node.log in {} for details.",
        datadir.display()
    ))
}

/// While the (long) startup RPC wait runs, surface warmup as a CALM phase:
/// probe the node every 2 s and, whenever it answers RPC_IN_WARMUP (-28),
/// project btxd's own progress line ("Verifying blocks…") into
/// `NodePhase::Warming`. Without this, a long post-crash rebuild sat behind a
/// silent "Starting" until the old 45-min budget expired into a red error —
/// a screenshot of a WORKING node that looked broken. Exits as soon as the
/// RPC is armed (start succeeded), the node slot clears (stop/error), or the
/// warmup probe starts succeeding.
fn spawn_warmup_watcher(app: AppHandle, state: &AppState, datadir: PathBuf) {
    let rpc_slot = state.rpc.clone();
    let node_slot = state.node.clone();
    let phase_slot = state.phase.clone();
    tauri::async_runtime::spawn(async move {
        let cookie = datadir.join(".cookie");
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if rpc_slot.lock().await.is_some() || node_slot.lock().await.is_none() {
                return; // started, stopped, or errored — the main path owns the phase now
            }
            let Ok(client) = btx_core::rpc::RpcClient::from_cookie(RPC_URL, &cookie) else {
                continue; // no cookie yet — still booting
            };
            match get_blockchain_info(&client).await {
                Err(AppError::Rpc { code: -28, message }) => {
                    // Only step on Starting/Warming — any other phase means a
                    // later owner (error path, stop) took over: stand down.
                    let mut guard = phase_slot.lock().await;
                    if !matches!(*guard, NodePhase::Starting | NodePhase::Warming { .. }) {
                        return;
                    }
                    let p = NodePhase::Warming { message };
                    *guard = p.clone();
                    drop(guard);
                    crate::tray::reflect_phase(&app, &p);
                }
                Ok(_) => return, // healthy — wait_for_node_rpc lands momentarily
                Err(_) => {}     // transient; keep watching
            }
        }
    });
}

/// The status refresher: every 3 s read chain info + chainstates + peers and
/// project them into the phase (Syncing / Ready). Exits when superseded by a
/// newer generation (restart) or when the node is stopped. Repeated RPC
/// failures surface a calm error phase instead of a silent freeze.
///
/// The refresher is also the app's ONE peer-census pipeline (a single
/// getpeerinfo per tick feeds the status cache, the watchdog and the service
/// report) and it arms the stall watchdog ITSELF — it must never depend on
/// the UI polling `get_node_status` for any of its inputs, because with the
/// window hidden nothing polls.
fn spawn_status_refresher(app: AppHandle, state: &AppState) {
    let gen = state.refresher_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let gen_counter = state.refresher_gen.clone();
    let rpc_slot = state.rpc.clone();
    let node_slot = state.node.clone();
    let phase_slot = state.phase.clone();
    let stall_slot = state.stall_verdict.clone();
    let rc_cache_slot = state.rc_status_cache.clone();
    let archive_slot = state.archive_peers_cache.clone();
    let nickname_slot = state.peer_nicknames_cache.clone();
    let archive_service_slot = state.archive_service.clone();
    let anchor = snapshot_spec().anchor_height;

    tauri::async_runtime::spawn(async move {
        // This run's start, for the service report's uptime field (the
        // command layer's started_at lives behind a non-Arc sync Mutex; the
        // refresher is spawned at the same moment, so this is the same
        // clock to within milliseconds).
        let run_started = std::time::Instant::now();
        let mut consecutive_failures: u32 = 0;
        let mut snapshot_swept = false;
        // Trusted-mirror stall watchdog state (Paper 3 §3, progress rule
        // refined — see btx_core::watchdog): while a connectable gap exists,
        // only BLOCK movement is progress; at the frontier and during
        // presync, any change counts. A verdict requires a sustained freeze.
        // Remediation is peer dialling only, rate-limited; a restart is
        // never automated.
        let mut wd_prev_heights: Option<(u64, u64)> = None;
        let mut wd_frozen_since: Option<std::time::Instant> = None;
        let mut wd_last_dial: Option<std::time::Instant> = None;
        let mut wd_last_dial_ok = true;
        // Service report (opt-in, local JSON): write every ~100 ticks (~5 min).
        let mut report_tick: u32 = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            if gen_counter.load(Ordering::SeqCst) != gen {
                return; // superseded by a restart / stop
            }
            let Some(rpc) = rpc_slot.lock().await.clone() else {
                return; // node stopped — the stop path owns the phase
            };
            match get_blockchain_info(&rpc).await {
                Ok(chain) => {
                    consecutive_failures = 0;
                    // Housekeeping: free the ~450 MB bootstrap snapshot the
                    // moment its load is confirmed (C3-gated; safe while btxd
                    // runs — it never holds the file after loadtxoutset).
                    // Without this it sat until the next restart's reclaim.
                    if !snapshot_swept {
                        let dd = node_datadir();
                        let loaded = NodeAppSettings::load(&dd).snapshot_loaded;
                        if let Some(bytes) = btx_core::disk::sweep_loaded_snapshot(&dd, loaded) {
                            eprintln!(
                                "[node-app] swept loaded snapshot.dat ({} MB)",
                                bytes / (1024 * 1024)
                            );
                        }
                        snapshot_swept = loaded; // stop probing once confirmed
                    }
                    // Independent reads — one round-trip instead of two.
                    // getpeerinfo replaces getconnectioncount (its length IS
                    // the peer count), and its summary lands in the shared
                    // cache: ONE census per tick for the status snapshot,
                    // the watchdog and the service report, instead of the
                    // three duplicate pipelines this used to run (the UI
                    // poll fetched its own full getpeerinfo every ~1.5 s).
                    let (chainstates, peer_infos) = tokio::join!(
                        get_chainstates(&rpc),
                        btx_core::node_api::get_peer_info(&rpc)
                    );
                    let chainstates = chainstates.unwrap_or_default();
                    let peer_infos = peer_infos.ok();
                    let peers = peer_infos.as_ref().map(|p| p.len() as i64).unwrap_or(0);
                    let archive_summary = peer_infos
                        .as_ref()
                        .map(|ps| btx_core::node_api::summarize_archive_peers(ps));
                    *archive_slot.lock().await = archive_summary.clone();
                    // Free: the peer objects are already in hand this tick.
                    *nickname_slot.lock().await = peer_infos
                        .as_ref()
                        .map(|ps| {
                            btx_core::nickname::peer_nicknames(ps.iter().map(|p| p.subver.as_str()))
                        })
                        .unwrap_or_default();
                    let readiness = btx_core::health::sync_readiness(&chainstates, &chain, anchor);
                    let new_phase = if readiness.is_near_tip() {
                        NodePhase::Ready {
                            height: readiness.height(),
                            peers,
                            // Already in hand at this tick and, until now, used
                            // only to discriminate stalls ninety lines below —
                            // never shown to the person reading the badge.
                            blocks_behind: chain.headers.saturating_sub(readiness.height()),
                        }
                    } else {
                        let mut headers = chain.headers;
                        let mut progress = readiness.progress();
                        // Headers phase (chain height still 0): RPC reports
                        // headers=0 through PRE-sync and a ~0 verification
                        // progress through all of it — the only LIVE percent
                        // is btxd's own log line. Surface it so the first
                        // minutes visibly count instead of sitting at
                        // "headers at 0" (a dead-looking screen reads as
                        // "not working" — field feedback).
                        if readiness.height() == 0 {
                            if let Some((h, pct)) =
                                btx_core::node::read_header_presync(&node_datadir())
                            {
                                headers = headers.max(h);
                                progress = progress.max(pct);
                            }
                        }
                        NodePhase::Syncing {
                            height: readiness.height(),
                            headers,
                            progress,
                            peers,
                        }
                    };
                    *phase_slot.lock().await = new_phase.clone();
                    crate::tray::reflect_phase(&app, &new_phase);

                    // Latch btxd's rc verdict OURSELVES (bounded tail read,
                    // only until latched — same rule as get_node_status).
                    // The cache used to be filled solely by the UI poll, so
                    // with the window hidden/tray'd nothing polled, the cache
                    // stayed empty and the watchdog below was silently
                    // unarmed on exactly the unattended mirrors it guards.
                    let trusted_mirror = {
                        let mut cache = rc_cache_slot.lock().await;
                        if cache.is_none() {
                            let fresh = btx_core::node::node_rc_status(&node_datadir());
                            if fresh.0.is_some() || fresh.1 {
                                *cache = Some(fresh);
                            }
                        }
                        cache
                            .as_ref()
                            .and_then(|c| c.0.as_ref().map(|p| p.trusted_mirror))
                            .unwrap_or(false)
                    };

                    // ── What is this node actually providing? ───────────────
                    // frontier.rs could answer this from the day it was written
                    // and nothing called it. get_attested_tip was reachable ONLY
                    // from the watchdog branch below — inside `else`, inside a
                    // trusted_mirror test, inside a sustained-freeze test — so
                    // the signed frontier was read only on a mirror that had
                    // ALREADY frozen. A healthy node could advertise the archive
                    // bit, sit far enough behind the frontier that btxd had
                    // quietly narrowed it to the live window, and say nothing.
                    // That is the one state worth reporting, and it was the one
                    // state unreachable.
                    //
                    // The fix costs one RPC. Measured on the release box
                    // 2026-09-04: getmatmulattestedtip runs ~10 ms, the same
                    // order as the getblockchaininfo (7 ms), getnetworkinfo
                    // (6 ms) and getpeerinfo (8 ms) this tick already makes,
                    // against a 3-second period. About a third of one percent.
                    //
                    // And only nodes it can tell something ever pay it: a node
                    // that does not serve attestations is given the answer
                    // directly and the RPC is skipped. That is also why the
                    // settings read below is not wasteful — on a non-serving
                    // node it REPLACES the RPC rather than adding to it.
                    {
                        let serving =
                            NodeAppSettings::load(&node_datadir()).attestation_serve_enabled;
                        let blocks_behind = if serving {
                            btx_core::node_api::get_attested_tip(&rpc)
                                .await
                                .ok()
                                .and_then(|t| t.blocks_behind)
                        } else {
                            None
                        };
                        *archive_service_slot.lock().await =
                            Some(btx_core::frontier::archive_service(serving, blocks_behind));
                    }

                    // ── Trusted-mirror stall watchdog tick ──────────────────
                    let heights = (readiness.height(), chain.headers);
                    // Progress rule (refined — see btx_core::watchdog): while
                    // a connectable gap exists, only BLOCK movement counts.
                    // BTX mints a header every ~90 s, so the old any-change
                    // rule reset the freeze window on every header and the
                    // watchdog could never fire while the network was alive —
                    // precisely the starving-mirror case it exists for. At
                    // the frontier and during presync, any change still
                    // resets (pre-sync laps and paused networks must never
                    // accumulate freeze).
                    let progressed = match wd_prev_heights {
                        None => true,
                        Some(prev) => {
                            let gap = heights.0 > 0 && heights.1 > heights.0;
                            if gap {
                                prev.0 != heights.0
                            } else {
                                prev != heights
                            }
                        }
                    };
                    wd_prev_heights = Some(heights);
                    if progressed {
                        // Progress resets the window and clears any published
                        // verdict — the UI must never show a stale stall.
                        wd_frozen_since = None;
                        if stall_slot.lock().await.take().is_some() {
                            eprintln!("[node-app] watchdog: progress resumed, verdict cleared");
                        }
                    } else {
                        let frozen_secs = wd_frozen_since
                            .get_or_insert_with(std::time::Instant::now)
                            .elapsed()
                            .as_secs();
                        // Mirrors only: the strict-device stall has its own
                        // existing path (rc_stalled), and non-mirror sync
                        // hiccups are not this discriminator's business.
                        if trusted_mirror && frozen_secs >= btx_core::watchdog::FROZEN_VERDICT_SECS
                        {
                            // This tick's census — the shared fetch above, no
                            // second getpeerinfo.
                            let summary = archive_summary.clone();
                            // The signed frontier: is there anything to fetch at
                            // all? Without this the watchdog treats every attestor
                            // pause as this node's stall (measured 2026-08-19: a
                            // ~100-minute quiet frontier had field nodes redialling
                            // archives on a loop with nothing signed to collect).
                            // Both fields matter: the lag alone cannot be trusted
                            // when the frontier we can see sits on a fork.
                            let attested = btx_core::node_api::get_attested_tip(&rpc).await.ok();
                            let frontier_lag = attested.as_ref().and_then(|t| t.blocks_behind);
                            let frontier_on_active_chain =
                                attested.as_ref().and_then(|t| t.on_active_chain);
                            let facts = btx_core::watchdog::StallFacts {
                                blocks: heights.0,
                                headers: heights.1,
                                frontier_lag,
                                frontier_on_active_chain,
                                frozen_secs,
                                retryable_marker: btx_core::node::node_log_has_retryable_marker(
                                    &node_datadir(),
                                ),
                                archive_authority: summary.as_ref().map(|s| s.authority),
                                // From the SAME census fetched above, so this
                                // costs no extra RPC. Separates "nobody will
                                // serve me" from "I am asking nobody"; see
                                // StallFacts::blocks_in_flight.
                                blocks_in_flight: summary.as_ref().map(|s| s.blocks_in_flight),
                                // v1: no per-process CPU sampler yet — class D
                                // (spin) stays undetectable here; classes A–C,
                                // the remediable ones, classify without it.
                                cpu_pct_one_core: None,
                                trusted_mirror: true,
                            };
                            let verdict = btx_core::watchdog::discriminate(&facts);
                            if let Some(v) = verdict.as_ref() {
                                // Remediation — the ONE automated action, from
                                // the one mechanism with a production receipt
                                // (archive handshake → unstuck in 21 s): dial
                                // the archive list via RPC addnode (manual ⇒
                                // passes the authority gate, no restart).
                                // At most once per 10 minutes after a dial
                                // that reached the node; once per minute
                                // after one that failed outright — a failed
                                // redial must not burn the whole budget (the
                                // node dialled NOTHING). Never restarts.
                                let retry_after: u64 = if wd_last_dial_ok { 600 } else { 60 };
                                let due = wd_last_dial
                                    .map(|t| t.elapsed().as_secs() >= retry_after)
                                    .unwrap_or(true);
                                if due {
                                    wd_last_dial = Some(std::time::Instant::now());
                                    let mut dialled = 0usize;
                                    for host in btx_core::node::BTX_ARCHIVE_PEERS {
                                        match btx_core::node_api::add_node(&rpc, host).await {
                                            Ok(()) => dialled += 1,
                                            Err(e) => eprintln!(
                                                "[node-app] watchdog: redial {host} failed: {e}"
                                            ),
                                        }
                                    }
                                    wd_last_dial_ok = dialled > 0;
                                    eprintln!(
                                        "[node-app] watchdog: {:?} after {}s frozen at {:?} — redialled {}/{} archive peers",
                                        v.class,
                                        frozen_secs,
                                        heights,
                                        dialled,
                                        btx_core::node::BTX_ARCHIVE_PEERS.len()
                                    );
                                }
                            }
                            *stall_slot.lock().await = verdict;
                        }
                    }

                    // ── Opt-in local service report (~ every 5 min) ─────────
                    report_tick += 1;
                    if report_tick >= 100 {
                        report_tick = 0;
                        let dd = node_datadir();
                        let settings = NodeAppSettings::load(&dd);
                        if settings.service_report_enabled {
                            let now_unix = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            let mut r = btx_core::service_report::ServiceReport::new(now_unix);
                            r.uptime_secs = run_started.elapsed().as_secs();
                            r.blocks = readiness.height();
                            r.headers = chain.headers;
                            r.peers = peers;
                            r.bytes_sent = btx_core::node_api::get_net_totals(&rpc)
                                .await
                                .ok()
                                .map(|t| t.total_bytes_sent);
                            // This tick's shared census — no dedicated fetch.
                            r.archive_peers = archive_summary.clone();
                            r.stall = stall_slot.lock().await.clone();
                            r.trusted_mirror = trusted_mirror;
                            r.serving_attestations = settings.attestation_serve_enabled;
                            r.nickname = settings.node_nickname.clone();
                            if let Err(e) = btx_core::service_report::write_service_report(&dd, &r)
                            {
                                eprintln!("[node-app] service report write failed: {e}");
                            }
                        }
                    }
                }
                Err(AppError::Rpc { code: -28, .. }) => {
                    // Warming up (shielded rebuild / verify) — alive, keep calm.
                    consecutive_failures = 0;
                }
                Err(_) => {
                    consecutive_failures += 1;
                    // ~1 min of continuous silence → tell the user instead of
                    // freezing on a stale number. No destructive action.
                    if consecutive_failures >= 20 {
                        // Unwedge so Start actually works: clear the RPC slot
                        // (start_node's already-running guard keys on it) and
                        // drop the controller — kill_on_drop reaps the wedged
                        // btxd so a fresh spawn doesn't lose the datadir-lock
                        // race. Leaving rpc set here made Start a permanent
                        // no-op with the UI stuck on this very error.
                        *rpc_slot.lock().await = None;
                        *node_slot.lock().await = None;
                        let p = NodePhase::Error {
                            message: "The node stopped responding. Press Start to relaunch it."
                                .into(),
                        };
                        *phase_slot.lock().await = p.clone();
                        crate::tray::reflect_phase(&app, &p);
                        return;
                    }
                }
            }
        }
    });
}

/// Graceful stop shared by the command, the tray, and app exit.
pub async fn stop_node_inner(state: &AppState) {
    // Kill the refresher first so it can't overwrite the Stopped phase.
    state.refresher_gen.fetch_add(1, Ordering::SeqCst);
    let launch = state.launch.lock().await.clone();
    {
        let mut guard = state.node.lock().await;
        if let Some(controller) = guard.as_mut() {
            if let Some((btx_cli, datadir)) = launch.as_ref() {
                let _ = controller.stop(btx_cli, datadir).await;
            }
            *guard = None;
        } else if let Some((btx_cli, datadir)) = launch.as_ref() {
            // Attached mode (we never spawned it): ask it to stop gracefully
            // and wait out the SAME flush budget a node of our own gets. This
            // is the common case, not the exotic one — after a self-update
            // relaunch the app has adopted the previous instance's orphan, so
            // the very next quit takes this branch. The old 10 s wait here
            // (inherited from stop_foreign_node) force-killed btxd mid-flush
            // and cost a multi-minute shielded-state rebuild on the next start.
            btx_core::node::stop_unmanaged_node(datadir, btx_cli, ATTACHED_STOP_GRACE).await;
        }
    }
    *state.rpc.lock().await = None;
    *state.attached_to.lock().await = None;
    *state.started_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // A stopped node has no CURRENT stall verdict or peer census — leaving
    // them made get_node_status report the dead run's stall as live.
    *state.stall_verdict.lock().await = None;
    *state.archive_peers_cache.lock().await = None;
    state.peer_nicknames_cache.lock().await.clear();
    *state.archive_service.lock().await = None;
    // Release the keep-awake assertion — the Mac may sleep again.
    *state.sleep_guard.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Gracefully stop the node OFF the main thread, then exit. This is the whole
/// fix for "I have to force-quit": the old path ran the (up to ~90s) shielded
/// flush synchronously in the ExitRequested handler, freezing the UI. Here the
/// stop is awaited on the async runtime while the window shows "stopping…", and
/// only then does the process exit. Idempotent — the first caller wins.
pub fn spawn_graceful_quit(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.quitting.swap(true, Ordering::SeqCst) {
        return; // a quit is already in flight
    }
    // Show progress instead of a frozen dock icon — reassuring even on a tray
    // quit where the window was hidden.
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.emit("app-quitting", ());
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        // Backstop: normally the graceful stop is quick, and btxd's shielded
        // flush is worth waiting for (a clean stop = an instant next start). But
        // a wedged btxd must NEVER turn "quit" back into a force-quit — so if the
        // stop overruns the grace window, exit anyway. btxd's pidfile/lock
        // recovery handles the hard stop on the next launch.
        let _ = tokio::time::timeout(QUIT_GRACE, stop_node_inner(&state)).await;
        app.exit(0);
    });
}

/// Settings: change what the red X does. Only the three known modes are
/// accepted, so a bad value from the UI can't wedge the close handler.
#[tauri::command]
pub fn set_on_close(mode: String) -> Result<(), String> {
    if !matches!(mode.as_str(), "ask" | "tray" | "quit") {
        return Err(format!("unknown close mode: {mode}"));
    }
    NodeAppSettings::update(&node_datadir(), |s| s.on_close = mode.clone());
    Ok(())
}

/// The close dialog's answer. `quit` stops the node and exits; otherwise the
/// window hides to the tray and the node keeps running. `remember` persists the
/// choice so the red X stops asking.
#[tauri::command]
pub fn close_choice(app: AppHandle, quit: bool, remember: bool) -> Result<(), String> {
    if remember {
        let mode = if quit { "quit" } else { "tray" };
        NodeAppSettings::update(&node_datadir(), |s| s.on_close = mode.to_string());
    }
    if quit {
        spawn_graceful_quit(&app);
    } else if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    Ok(())
}

// ── The status snapshot the UI polls ────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct NodeStatusInfo {
    /// Whether a node run is active (rpc armed).
    pub running: bool,
    /// The current phase (tagged enum — carries height/peers/progress).
    pub phase: NodePhase,
    /// Seconds since this run started (0 when stopped).
    pub uptime_secs: u64,
    /// Free MB on the datadir volume (0 = not measured).
    pub disk_free_mb: u64,
    /// Low-disk warning thresholds (MB free), from the ONE canonical definition
    /// in btx_core::disk — the frontend compares against these instead of
    /// hardcoding numbers that could drift from the Rust side.
    pub disk_warn_mb: u64,
    pub disk_critical_mb: u64,
    /// What a fresh install of the CURRENTLY SELECTED profile needs free,
    /// from the same `disk_required` the preflight applies, so a keeper reads
    /// 20 GiB and a full node 140. The wizard renders this instead of a
    /// hardcoded string: "~105 GB" and 140 GiB were on screen at the same time
    /// once, and then 140 was shown to keepers the preflight would pass at 20.
    ///
    /// Always the FRESH figure, even for a resume gated at 2 GiB: overstating
    /// is the safe direction, and the resume figure is not what the disk will
    /// hold once the sync finishes.
    pub disk_required_mb: u64,
    /// Size of the datadir in MB (cached, refreshed ≤ every 60 s).
    pub datadir_size_mb: u64,
    /// The datadir path (Settings display).
    pub datadir: String,
    /// The btxd release tag installed/launched.
    pub node_tag: String,
    /// Whether the node binaries are present for that tag.
    pub installed: bool,
    /// Whether first-run setup has completed.
    pub setup_complete: bool,
    /// Keep-awake toggle state.
    pub keep_awake: bool,
    /// Whether "keep awake" does anything on this build. The guard is an inert
    /// zero-sized value off macOS, so on Linux and Windows this switch was
    /// shown, defaulted ON, and prevented nothing — and toggling it off and
    /// back on, the obvious recovery, also did nothing and still returned Ok.
    pub keep_awake_supported: bool,
    /// What this platform calls the place a closed window keeps running: the
    /// "menu bar" on macOS, the "system tray" everywhere else.
    ///
    /// The copy said "menu bar" unconditionally - in the first-run pitch, the
    /// close dialog, the close-behaviour setting and a button label. On Windows
    /// and Linux that is a place the user does not have, in a dialog asking
    /// them to choose it. The frontend has no platform signal of its own, so
    /// the noun is supplied here and swapped into the sentence rather than the
    /// sentences being duplicated per platform.
    pub tray_term: String,
    /// Explorer mode (txindex) — the persisted user choice.
    pub txindex_enabled: bool,
    /// Attestation serving (`matmulattestationserve=1`) — the persisted user
    /// choice, or a hand-set conf flag the start path adopted. A change
    /// applies on the next node (re)start.
    pub attestation_serve_enabled: bool,
    /// What this node is really providing to other nodes right now: at the
    /// signed frontier and serving history, advertising the archive bit while
    /// silently degraded to the live window, or not serving at all. `None`
    /// until the refresher has completed one tick.
    pub archive_service: Option<btx_core::frontier::ArchiveService>,
    /// The same verdict as one sentence for a human, and whether it is the
    /// state that deserves attention.
    ///
    /// Carried BESIDE the tagged enum rather than inside it, on purpose. The
    /// enum's wire shape is pinned field-by-field by a test in btx-core, and
    /// the sentences live in `ArchiveService::message()` — so shipping the
    /// rendered string keeps the copy in one place instead of growing a second
    /// copy in TypeScript that drifts from the first.
    pub archive_service_message: Option<String>,
    pub archive_service_needs_attention: bool,
    /// The nickname the user has chosen (empty = none). This is what WILL be
    /// broadcast; `subversion` below is what IS.
    pub node_nickname: String,
    /// Our user agent exactly as peers see it, e.g. `/BTX:0.34.6(alice)/`.
    /// `None` when the node is not running or did not answer. Shown rather than
    /// derived, because btxd builds this once at init: a nickname set after the
    /// node started is not live until the next start, and the honest way to say
    /// that is to display the real string beside the setting.
    pub subversion: Option<String>,
    /// The nickname btxd is broadcasting RIGHT NOW, parsed out of `subversion`.
    /// `None` when the node is down, has not answered, or carries no comment.
    /// This, not `node_nickname`, is what "you are X" must be rendered from: a
    /// name saved on a running node is not on the wire until the next start,
    /// and a name cleared on a running node is still on it.
    pub broadcast_nickname: Option<String>,
    /// Nicknames of the peers we are connected to. Untrusted display text
    /// chosen by strangers — filtered and capped by `btx_core::nickname`.
    pub peer_nicknames: Vec<String>,
    /// The local service report (`service-report.json` in the datadir) — the
    /// persisted user choice. Local file only; nothing is uploaded.
    pub service_report_enabled: bool,
    /// Wallet view toggle (Settings) — drives the header icon's visibility.
    pub wallet_enabled: bool,
    /// What the red X does: "ask" | "tray" | "quit" (Settings segmented control).
    pub on_close: String,
    /// How btxd is executing MatMul RC ExactReplay — its OWN reported mode
    /// (`strict-device` / `auto-fallback` / `cpu-diagnostic`), not our guess.
    /// `None` until btxd has logged it (early startup, or a pre-v0.33.2 node).
    pub rc_mode: Option<String>,
    /// True only when this node validates MatMul consensus independently, i.e.
    /// strict-device on a qualified device. This is the honest answer to "am I
    /// a real full node?" after the v4.7 fork.
    pub rc_validates_independently: bool,
    /// True when the node keeps validating but may drift behind the tip because
    /// it is replaying RC episodes without a qualified accelerator.
    pub rc_may_fall_behind: bool,
    /// btxd's own reason string for the mode it picked, when it gave one.
    pub rc_reason: Option<String>,
    /// btxd is in `strict-device` mode with a provider that did NOT qualify —
    /// the stall. The node is up and looks healthy but cannot advance past the
    /// fork height. Derived from btxd's own `ready=` flag, not from scanning
    /// for a failure string (see `node::node_rc_status`).
    pub rc_stalled: bool,
    /// This node follows the chain past the MatMul v4.7 fork through a quorum
    /// of signed attestations rather than replaying the proof itself. True on
    /// machines btxd will not accept, which would otherwise park at 184,999.
    pub rc_trusted_mirror: bool,
    /// Bytes this node has uploaded to peers this run (`getnettotals`).
    ///
    /// Feeds the "Helping the network" card: chain data other people actually
    /// took from this machine. `None` (not 0) when the node is stopped or did
    /// not answer, so the UI hides the claim instead of rendering "0 B served"
    /// at the user who is deciding whether any of this is worth it.
    pub bytes_sent: Option<u64>,
    /// Peers that connected to US this run (`getnetworkinfo.connections_in`),
    /// or `None` when the node is stopped or did not answer.
    ///
    /// The direction is the point. Outbound peers are ones we dialled and take
    /// the chain FROM. Inbound are ones that reached us, which is only possible
    /// if this machine is actually reachable, and it is the only way an ordinary
    /// node supplies the network instead of only consuming it. The contribution
    /// card used to read the TOTAL and say "N nodes are connected to you", which
    /// on a normal home Mac was the wrong way round for every one of them.
    ///
    /// `None`, not 0, on no measurement: same discipline as `bytes_sent`, so the
    /// card says nothing rather than accusing a node of being unreachable on the
    /// strength of a failed RPC call.
    pub inbound_peers: Option<u64>,
    /// Trusted-mirror peer health: how many archive peers we see, how many
    /// pass the authority gate (manual/noban — the ones the node will actually
    /// ask), and live attestation flow both ways. `authority == 0` on a
    /// trusted mirror is the root cause of the silent-stall class and should
    /// be surfaced BEFORE the height freezes. `None` when stopped/unanswered.
    pub archive_peers: Option<btx_core::node_api::ArchivePeerSummary>,
    /// The stall discriminator's verdict once the tip has been frozen past the
    /// verdict window (mirrors only). `None` = healthy or no verdict. The
    /// refresher clears it the moment progress resumes.
    pub stall: Option<btx_core::watchdog::StallVerdict>,
    /// The user's node profile CHOICE ("full" | "keeper").
    pub node_profile: String,
    /// Whether the bundled engine can honour the keeper profile. When the
    /// choice is "keeper" and this is false, the UI says the profile arrives
    /// with the next engine update — the choice is stored, not lost.
    pub keeper_engine_ready: bool,
}

#[tauri::command]
pub async fn get_node_status(state: State<'_, AppState>) -> Result<NodeStatusInfo, String> {
    let datadir = node_datadir();
    let settings = NodeAppSettings::load(&datadir);
    let tag = settings
        .btx_release_tag
        .clone()
        .unwrap_or_else(|| NODE_RELEASE_TAG.to_string());

    let phase = state.phase.lock().await.clone();
    let running = state.rpc.lock().await.is_some();
    let uptime_secs = state
        .started_at
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);

    // Cheap every poll: statvfs. On a true first run the datadir doesn't exist
    // yet (setup creates it) and statvfs would report 0 ("not measured") on
    // exactly the screen whose job is "do I have enough space?" — probe the
    // nearest existing ancestor instead.
    let disk_probe = if datadir.exists() {
        datadir.clone()
    } else {
        btx_core::platform::home_dir().unwrap_or_else(|| PathBuf::from("."))
    };
    let disk_free_mb = btx_core::disk::free_disk_mb(&disk_probe);
    // Heavy (recursive walk of a ~124 GiB tree on a full node): refresh at most
    // once a minute,
    // OFF the async executor (spawn_blocking) and WITHOUT holding the cache
    // lock during the walk — a cold-cache walk can take seconds and would
    // otherwise stall every concurrent status poll behind the mutex.
    let datadir_size_mb = {
        let (cached_mb, measured_at) = *state
            .datadir_size_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let stale = measured_at
            .map(|t| t.elapsed().as_secs() >= 60)
            .unwrap_or(true);
        let walking = state.size_walk_running.clone();
        if stale
            && walking
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            let dd = datadir.clone();
            let cache = state.datadir_size_cache_handle();
            tauri::async_runtime::spawn_blocking(move || {
                let bytes = btx_core::disk::dir_size_bytes(&dd);
                *cache.lock().unwrap_or_else(|e| e.into_inner()) =
                    (bytes / (1024 * 1024), Some(std::time::Instant::now()));
                walking.store(false, Ordering::SeqCst);
            });
        }
        cached_mb // serve the last measurement; the walk lands for a later poll
    };

    // How is btxd actually validating MatMul RC? Ask btxd, don't infer it from
    // the platform: a Mac whose Metal shaders fail to build at runtime falls
    // back to CPU and would then stall under strict-device, while a
    // platform-based guess would still claim "qualified".
    //
    // Read the log only until btxd has told us, then cache for the rest of the
    // run. btxd logs its verdict ONCE at startup, behind a production canary
    // that takes minutes, and the poll can only afford a bounded tail read — so
    // on a node up for hours that line has long scrolled out of the window.
    // Without the cache the Block-checking card would vanish on exactly the
    // long-running nodes it exists to describe. Caching also drops the per-poll
    // file read to zero once the answer is known.
    let (rc_policy, rc_stalled) = if running {
        let cached = state.rc_status_cache.lock().await.clone();
        match cached {
            Some(hit) => hit,
            None => {
                let fresh = btx_core::node::node_rc_status(&datadir);
                // Only latch once btxd has actually said something: a `None`
                // policy just means "not logged yet" (early startup), and
                // latching that would freeze the card hidden forever.
                if fresh.0.is_some() || fresh.1 {
                    *state.rc_status_cache.lock().await = Some(fresh.clone());
                }
                fresh
            }
        }
    } else {
        (None, false)
    };

    // Take a CLONE of the handle and drop the guard before any await. Holding
    // `state.rpc` across a network round-trip is what every other call site in
    // this file avoids (ask.rs, wallet.rs, and the refresher below all clone
    // first), and this was the one place that did not: two of these blocks in
    // a row each held the shared mutex across an RPC whose timeout is 60 s.
    // A btxd that accepts the connection and never answers therefore froze the
    // dashboard on stale numbers and starved the refresher that is supposed to
    // notice and say "The node stopped responding."
    let rpc = if running {
        state.rpc.lock().await.clone()
    } else {
        None
    };

    // Cheap counter read on a live node; skipped entirely when stopped. Any
    // failure degrades to None so a node that does not answer simply drops the
    // claim rather than reporting a zero it never earned.
    let bytes_sent = match rpc.as_ref() {
        Some(rpc) => btx_core::node_api::get_net_totals(rpc)
            .await
            .ok()
            .map(|t| t.total_bytes_sent),
        None => None,
    };

    // Same shape and the same degrade-to-None discipline as bytes_sent above.
    // The whole struct is kept because `subversion` rides along on the same
    // getnetworkinfo — showing a user exactly what the network sees them as
    // therefore costs no extra RPC.
    let net = match rpc.as_ref() {
        Some(rpc) => btx_core::node_api::get_connection_counts(rpc).await.ok(),
        None => None,
    };
    let inbound_peers = net.as_ref().map(|c| c.inbound);
    let subversion = net.map(|c| c.subversion).filter(|s| !s.is_empty());

    // Read the frontier verdict ONCE: the payload carries the tagged value for
    // machines and the rendered sentence for people, and taking the lock twice
    // could ship two different ticks in one status.
    let archive_service = state.archive_service.lock().await.clone();

    // Archive-peer census for the trusted-mirror health card: served from the
    // refresher's per-tick cache (≤3 s old) instead of running a second full
    // getpeerinfo on every ~1.5 s UI poll. Same degrade-to-None discipline as
    // bytes_sent: no measurement, no claim.
    let peer_nicknames = if running {
        state.peer_nicknames_cache.lock().await.clone()
    } else {
        Vec::new()
    };

    let archive_peers = if running {
        state.archive_peers_cache.lock().await.clone()
    } else {
        None
    };

    Ok(NodeStatusInfo {
        running,
        phase,
        uptime_secs,
        bytes_sent,
        inbound_peers,
        disk_free_mb,
        disk_warn_mb: btx_core::disk::NODE_DISK_WARN_MB,
        disk_critical_mb: btx_core::disk::NODE_DISK_CRITICAL_MB,
        disk_required_mb: btx_core::setup::disk_required(
            true,
            btx_core::installer::conf_for_profile(&settings.node_profile, NODE_RELEASE_TAG)
                != btx_core::installer::NODE_FASTSTART_CONF,
        ) / (1024 * 1024),
        datadir_size_mb,
        datadir: datadir.display().to_string(),
        // setup_complete implies provisioned binaries; the recursive
        // returning_launch_paths walk (2 full install-dir traversals) is only
        // worth paying while setup hasn't happened yet.
        installed: settings.setup_complete || returning_launch_paths(&datadir, &tag).is_some(),
        node_tag: tag,
        setup_complete: settings.setup_complete,
        keep_awake: settings.keep_awake,
        keep_awake_supported: btx_core::power::sleep_assertion_supported(),
        tray_term: if cfg!(target_os = "macos") {
            "menu bar".to_string()
        } else {
            "system tray".to_string()
        },
        txindex_enabled: settings.txindex_enabled,
        attestation_serve_enabled: settings.attestation_serve_enabled,
        archive_service: archive_service.clone(),
        archive_service_message: archive_service.as_ref().map(|a| a.message()),
        node_nickname: settings.node_nickname.clone(),
        broadcast_nickname: subversion
            .as_deref()
            .and_then(btx_core::nickname::nickname_from_subver),
        subversion,
        peer_nicknames,
        archive_service_needs_attention: archive_service
            .as_ref()
            .is_some_and(|a| a.needs_attention()),
        service_report_enabled: settings.service_report_enabled,
        wallet_enabled: settings.wallet_enabled,
        on_close: settings.on_close.clone(),
        rc_mode: rc_policy.as_ref().map(|p| p.mode.clone()),
        rc_validates_independently: rc_policy
            .as_ref()
            .map(|p| p.validates_independently())
            .unwrap_or(false),
        rc_may_fall_behind: rc_policy
            .as_ref()
            .map(|p| p.may_fall_behind())
            .unwrap_or(false),
        rc_reason: rc_policy.as_ref().and_then(|p| p.reason.clone()),
        rc_stalled,
        rc_trusted_mirror: rc_policy.as_ref().is_some_and(|p| p.trusted_mirror),
        archive_peers,
        stall: state.stall_verdict.lock().await.clone(),
        node_profile: settings.node_profile.clone(),
        keeper_engine_ready: btx_core::installer::engine_supports_keeper_profile(NODE_RELEASE_TAG),
    })
}

// ── First-run setup pipeline ────────────────────────────────────────────────

#[tauri::command]
pub async fn begin_setup(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    guarded_setup(&app, &state).await
}

/// Append a timestamped line to `<datadir>/setup.log`. A GUI app has no
/// visible stderr — on Windows especially, a failed or stalled first-run
/// setup used to leave ZERO trace on the machine. This file is the durable
/// answer to "it's stuck, what happened?": every step and every error lands
/// here. Best-effort by design; logging must never break setup itself.
fn setup_log(datadir: &Path, msg: &str) {
    use std::io::Write;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = btx_core::platform::open_private_append(&datadir.join("setup.log")) {
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}

/// The single guarded entry to the setup pipeline — used by the wizard button
/// AND the E2E seam, so guard/error-projection fixes can never diverge.
/// "Already running" is a distinct, non-success answer: returning Ok here made
/// the frontend latch "setup done" while the real pipeline was mid-download.
pub(crate) async fn guarded_setup(
    app: &AppHandle,
    state: &State<'_, AppState>,
) -> Result<(), String> {
    if state
        .setup_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("setup is already running".to_string());
    }
    let result = run_setup_pipeline(app, state).await;
    state.setup_running.store(false, Ordering::SeqCst);
    if let Err(msg) = &result {
        setup_log(&node_datadir(), &format!("ERROR: {msg}"));
        set_phase(
            app,
            state,
            NodePhase::Error {
                message: msg.clone(),
            },
        )
        .await;
    }
    result
}

async fn run_setup_pipeline(app: &AppHandle, state: &State<'_, AppState>) -> Result<(), String> {
    let datadir = node_datadir();
    std::fs::create_dir_all(&datadir)
        .map_err(|e| format!("couldn't create {}: {e}", datadir.display()))?;
    setup_log(
        &datadir,
        &format!("setup started (app v{})", env!("CARGO_PKG_VERSION")),
    );

    // 1. Disk preflight. A fresh install (no chain yet) must fit the whole
    //    un-pruned chain, so it gates on DISK_REQUIRED_FRESH; a resume needs only
    //    operating headroom. Do NOT restate the chain size here — this comment
    //    carried the pre-2026-07 18 GiB figure long after the gate moved. The
    //    number, its date and its method live on the constant.
    //    "Unknown" free space never blocks.
    let fresh = !datadir.join("blocks").exists();
    // Which chain are we about to provision? The gate has to read the same
    // inputs as the conf that gets written twenty lines below, or it judges an
    // install nobody asked for. It used to gate every fresh install on the full
    // un-pruned chain while writing `prune=10000` for a keeper — refusing a
    // ~10 GiB node for want of 140 GiB, on the tier this network is shortest of
    // and which the app's own copy calls exactly that. The settings gear is in
    // the global header, outside the wizard screen, so a first-run user can and
    // does choose Keeper before pressing Install.
    let pruned = btx_core::installer::conf_for_profile(
        &NodeAppSettings::load(&datadir).node_profile,
        NODE_RELEASE_TAG,
    ) != btx_core::installer::NODE_FASTSTART_CONF;
    let required = btx_core::setup::disk_required(fresh, pruned);
    if let Some(free) = free_disk_bytes(&datadir) {
        if !enough_free_disk(free, required) {
            let need_gib = required / (1024 * 1024 * 1024);
            let free_gib = free / (1024 * 1024 * 1024);
            return Err(format!(
                "Not enough free disk space: the node needs about {need_gib} GiB \
                 and this volume has {free_gib} GiB free. Free up some space, then try again."
            ));
        }
    }

    // 2. Download the assumeutxo snapshot (slow, failure-prone step FIRST — a
    //    failed download must leave no installed-looking state; see the miner's
    //    run_native_setup ordering rationale).
    set_phase(app, state, NodePhase::Downloading { progress: 0.0 }).await;
    {
        let phase_slot = state.phase.clone();
        let app2 = app.clone();
        // Synchronous try_lock keeps progress writes ORDERED and strictly
        // before download_snapshot returns — a spawned task per step could
        // land late and clobber the Error phase set on a failed download.
        let progress = move |ratio: f64| {
            let p = NodePhase::Downloading { progress: ratio };
            if let Ok(mut g) = phase_slot.try_lock() {
                *g = p.clone();
            }
            crate::tray::reflect_phase(&app2, &p);
        };
        setup_log(&datadir, "downloading snapshot…");
        btx_core::snapshot::download_snapshot(&snapshot_spec(), &datadir, &progress).await?;
        setup_log(&datadir, "snapshot downloaded + SHA verified");
    }

    // 3. Provision the bundled node package (fast, local).
    set_phase(app, state, NodePhase::Preparing).await;
    let resource_dir = app.path().resource_dir().ok();
    let pkg = resolve_bundled_node_pkg(
        resource_dir.as_deref(),
        &[Path::new(env!("CARGO_MANIFEST_DIR"))],
    )
    .ok_or_else(|| {
        "this build doesn't include the node package — run \
         apps/node/scripts/stage-node-pkg.sh and rebuild"
            .to_string()
    })?;
    let install_root: PathBuf = install_dir(NODE_RELEASE_TAG)
        .ok_or_else(|| "couldn't resolve the install directory (HOME unset)".to_string())?;
    {
        let pkg = pkg.clone();
        let install_root = install_root.clone();
        let dd = datadir.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let conf = btx_core::installer::conf_for_profile(
                &NodeAppSettings::load(&dd).node_profile,
                NODE_RELEASE_TAG,
            );
            btx_core::installer::provision_node_package(&pkg, &install_root, &dd, conf)
        })
        .await
        .map_err(|e| format!("provisioning task panicked: {e}"))?
        .map_err(|e| e.to_string())?
    };

    setup_log(&datadir, "node package provisioned");

    // 4. Record the tag BEFORE starting (returning_launch_paths keys on it).
    NodeAppSettings::update(&datadir, |s| {
        s.btx_release_tag = Some(NODE_RELEASE_TAG.to_string());
    });

    // 5. Start the node (spawn → RPC wait → snapshot-load guarantee → refresher).
    setup_log(&datadir, "starting btxd…");
    start_node_inner(app, state).await?;
    setup_log(&datadir, "node started, RPC up");

    // 6. Done: returning launches skip the wizard and auto-start.
    NodeAppSettings::update(&datadir, |s| s.setup_complete = true);
    setup_log(&datadir, "setup complete");
    Ok(())
}

// ── Start / stop ────────────────────────────────────────────────────────────

/// `start_node_inner`, with a failure projected into [`NodePhase::Error`].
///
/// Use this from EVERY path that restarts the node. `set_phase` is documented
/// as the single writer of the phase, and a start that fails without writing
/// one leaves whatever was there before standing — which for the restart paths
/// meant `Stopped` at best and a green `Ready { height, peers }` over a dead
/// RPC at worst. Both control surfaces key on the phase (the tray's Start/Stop
/// and the window's own button), so a failure that writes nothing disables the
/// two places a user would go to recover.
pub async fn start_node_projected(
    app: &AppHandle,
    state: &State<'_, AppState>,
) -> Result<(), String> {
    let result = start_node_inner(app, state).await;
    if let Err(msg) = &result {
        set_phase(
            app,
            state,
            NodePhase::Error {
                message: msg.clone(),
            },
        )
        .await;
    }
    result
}

/// Stop the node and start it again, leaving the phase truthful at every step.
///
/// The intermediate `Stopped` matters: without it an early bail inside the start
/// leaves the pre-stop `Ready { height, peers }` standing over an RPC that is
/// now `None`, which renders as a green LIVE readout on a node that is not
/// running. Callers that bounce the node for a config change should use this
/// rather than open-coding stop-then-start.
pub async fn restart_node_projected(
    app: &AppHandle,
    state: &State<'_, AppState>,
) -> Result<(), String> {
    stop_node_inner(state).await;
    set_phase(app, state, NodePhase::Stopped).await;
    start_node_projected(app, state).await
}

#[tauri::command]
pub async fn start_node(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if state.rpc.lock().await.is_some() {
        return Ok(()); // already running
    }
    start_node_projected(&app, &state).await
}

#[tauri::command]
pub async fn stop_node(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    stop_node_inner(&state).await;
    set_phase(&app, &state, NodePhase::Stopped).await;
    Ok(())
}

// ── Settings / maintenance ──────────────────────────────────────────────────

#[tauri::command]
pub async fn open_data_folder() -> Result<String, String> {
    let dir = node_datadir();
    btx_core::platform::open_path(&dir).map_err(|e| e.to_string())?;
    Ok(dir.display().to_string())
}

/// Settings: serve historical attestations back to the network
/// (`matmulattestationserve=1` — ~208 bytes/block, protocol-rate-limited, and
/// the scarcest service on today's network). Persists the choice and asserts
/// or removes the conf key EXPLICITLY: this toggle is the ONE place the key
/// is ever removed — the start path only adds or adopts, so an operator's
/// hand-set flag survives every start (it used to be deleted on each one).
/// btxd reads the flag at startup, so a change applies on the next (re)start.
#[tauri::command]
pub async fn set_attestation_serve(on: bool) -> Result<(), String> {
    let datadir = node_datadir();
    NodeAppSettings::update(&datadir, |s| s.attestation_serve_enabled = on);
    let conf = datadir.join("faststart").join("faststart.conf");
    if conf.exists() {
        btx_core::setup::set_conf_kv(&conf, "matmulattestationserve", on.then_some("1"))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Which conf the engine-upgrade path writes: the selected profile's when the
/// disk can take it, otherwise the posture the datadir already has. `None`
/// means there is no safe conf and the engine must not be re-provisioned this
/// start. Pure, so the four corners are tested rather than trusted.
///
/// Selecting keeper is always honoured: pruning frees disk, it never needs
/// any. The only refused change is un-pruning a pruned datadir without the
/// room to hold the chain; and if the engine being installed cannot keep a
/// datadir pruned at all (the keeper gate says no), there is no conf that is
/// both safe and affordable, so nothing is written.
pub fn upgrade_conf_choice(
    selected: &'static str,
    was_pruned: Option<bool>,
    disk_ok: bool,
) -> Option<&'static str> {
    let selected_pruned = selected != btx_core::installer::NODE_FASTSTART_CONF;
    if disk_ok || selected_pruned {
        return Some(selected);
    }
    match was_pruned {
        Some(true) => {
            let keep = btx_core::installer::conf_for_profile("keeper", NODE_RELEASE_TAG);
            if keep == btx_core::installer::NODE_FASTSTART_CONF {
                None
            } else {
                Some(keep)
            }
        }
        Some(false) => Some(btx_core::installer::NODE_FASTSTART_CONF),
        None => None,
    }
}

/// The nickname a persisted setting is allowed to put in the conf, or `None`
/// to leave the key untouched. Pure: this is the whole reason a settings value
/// cannot reach `faststart.conf` without passing the same validator the
/// Settings box uses, on the path that runs unattended on every start.
pub fn conf_nickname(setting: &str) -> Option<String> {
    match btx_core::nickname::validate_nickname(setting) {
        Ok(Some(n)) => Some(n),
        _ => None,
    }
}

/// Set (or clear) the public nickname other nodes see.
///
/// Writes `uacomment` into the conf and persists the choice. It applies at the
/// next node start, like every other conf-level setting — btxd builds its user
/// agent once at init and there is no RPC to change it live.
///
/// Returns the CLEANED value so the settings box can show what was actually
/// stored: outer whitespace trimmed, inner runs collapsed. An invalid nickname
/// is refused with a sentence the UI can print verbatim, and nothing is written
/// — which matters more than usual here, because btxd fails to start on a
/// comment it rejects, so a bad value would turn a cosmetic setting into a node
/// that will not come back up.
#[tauri::command]
pub async fn set_node_nickname(name: String) -> Result<String, String> {
    let cleaned = btx_core::nickname::validate_nickname(&name)
        .map_err(|e| e.message())?
        .unwrap_or_default();

    let datadir = node_datadir();
    NodeAppSettings::update(&datadir, |s| s.node_nickname = cleaned.clone());

    let conf = datadir.join("faststart").join("faststart.conf");
    if conf.exists() {
        // An empty nickname REMOVES the key rather than writing `uacomment=`,
        // which btxd would read as an empty comment and render as `()`.
        btx_core::setup::set_conf_kv(
            &conf,
            "uacomment",
            if cleaned.is_empty() {
                None
            } else {
                Some(cleaned.as_str())
            },
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(cleaned)
}

/// Turn the local service report on or off.
///
/// It writes `service-report.json` into the datadir every ~5 minutes and does
/// NOTHING else: no network call, no upload, no identifier. It is the opt-in
/// seed for a keepers dashboard that would READ it, and "Nothing phones home"
/// stays true with this switched on.
///
/// ⚠ Why this exists at all. `service_report_enabled` has been read on every
/// refresher tick since it was added and written by nothing, so the report
/// could not be turned on — the branch ran, looked complete, and was
/// unreachable. That is worse than either shipping the feature or deleting it,
/// because it reads as done. This is the "expose it" half of that choice.
#[tauri::command]
pub async fn set_service_report(on: bool) -> Result<(), String> {
    NodeAppSettings::update(&node_datadir(), |s| s.service_report_enabled = on);
    Ok(())
}

#[tauri::command]
pub async fn set_node_profile(profile: String) -> Result<(), String> {
    if profile != "full" && profile != "keeper" {
        return Err(format!("unknown profile: {profile}"));
    }
    let datadir = node_datadir();
    // Say it NOW, at the moment of choosing, rather than at the next upgrade:
    // going Full on a pruned datadir means holding the whole chain, and a
    // choice the disk cannot honour should be refused where the person can
    // see it, not deferred into a log line months later.
    if profile == "full" {
        let conf_path = datadir.join("faststart").join("faststart.conf");
        let need = btx_core::setup::disk_required_for_conf(
            datadir.join("blocks").exists(),
            btx_core::setup::conf_is_pruned(&conf_path),
            false,
        );
        if let Some(free) = free_disk_bytes(&datadir) {
            if !enough_free_disk(free, need) {
                let gib = 1024 * 1024 * 1024;
                return Err(format!(
                    "A full node needs about {} GiB free to hold the whole chain, and this \
                     volume has {} GiB. Free some space first, or stay a keeper.",
                    need / gib,
                    free / gib
                ));
            }
        }
    }
    NodeAppSettings::update(&datadir, |s| {
        // Keeper implies serving — that is the profile's point. Choosing full
        // again does NOT clear a separately-made serve choice.
        if profile == "keeper" {
            s.attestation_serve_enabled = true;
        }
        s.node_profile = profile.clone();
    });
    // The conf takes effect at the next provisioning/start; the caller's UI
    // explains that (and the engine gate) — no silent node surgery here.
    Ok(())
}

#[tauri::command]
pub async fn set_keep_awake(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    let datadir = node_datadir();
    NodeAppSettings::update(&datadir, |s| s.keep_awake = on);
    // Apply live: hold/release the assertion to match, but only hold while a
    // node is actually running.
    let running = state.rpc.lock().await.is_some();
    let mut guard = state.sleep_guard.lock().unwrap_or_else(|e| e.into_inner());
    *guard = if on && running {
        Some(btx_core::power::SleepAssertion::hold(
            "easyBTX Node is supporting the BTX network",
        ))
    } else {
        None
    };
    Ok(())
}

/// Who owns the node on this datadir, from this app's point of view. The one
/// question the destructive commands need answered, and the one that
/// `state.rpc.is_some()` answered WRONG: in attach mode that slot holds the
/// other app's client, so the old check passed precisely when the datadir
/// belonged to somebody else. And `stop_node_inner` is not a no-op there: it
/// gracefully stops and then force-kills the attached node before the delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeOwnership {
    /// We spawned it. Stop it, then proceed.
    OurChild,
    /// Our previous instance's orphan, adopted after a self-update relaunch.
    /// Ours: the stop path handles it and the delete is ours to make.
    OurAdoptedOrphan,
    /// The miner's, or another window's. Never stopped, never deleted under.
    AnotherApp,
    /// We are attached to a node nobody could classify. Not ours to touch.
    AttachedToUnknown,
    /// Our own child is up but has not answered RPC yet (a long rebuild).
    Warming,
    /// Nothing holds the datadir. Proceed.
    Nobody,
    /// We hold nothing, and a live btxd we did not start does.
    ForeignLive,
    /// We hold nothing, and something we cannot identify holds the pidfile.
    ForeignUnknown,
}

/// The decision, pure so it can be tested for every variant. Refusals say who
/// is in the way and what to do, because the previous wording blamed the miner
/// for this app's own warming node.
pub fn destructive_allowed(owner: NodeOwnership) -> Result<(), String> {
    use NodeOwnership::*;
    match owner {
        OurChild | OurAdoptedOrphan | Nobody => Ok(()),
        Warming => Err(
            "the node is still starting up (this can take a while after an unclean shutdown), \
             give it a moment and try again"
                .to_string(),
        ),
        AnotherApp | ForeignLive => Err(
            "Another app is running the node in this data folder, probably the easyBTX \
             miner or a second copy of this app. Stop it first, then try again."
                .to_string(),
        ),
        AttachedToUnknown | ForeignUnknown => Err(
            "A node this app did not start is using this data folder and could not be \
             identified. Stop it first, then try again."
                .to_string(),
        ),
    }
}

/// Work out [`NodeOwnership`] from what this app knows, probing the datadir
/// only when it holds nothing itself.
async fn node_ownership(state: &AppState, datadir: &Path) -> NodeOwnership {
    if let Some(attached) = *state.attached_to.lock().await {
        return match attached {
            AttachedTo::AnotherApp => NodeOwnership::AnotherApp,
            AttachedTo::OurOrphan => NodeOwnership::OurAdoptedOrphan,
            AttachedTo::Unknown => NodeOwnership::AttachedToUnknown,
        };
    }
    if state.rpc.lock().await.is_some() {
        // Not attached and RPC is up: we spawned it.
        return NodeOwnership::OurChild;
    }
    let child_alive = state
        .node
        .lock()
        .await
        .as_mut()
        .map(|c| c.child_has_exited() == Some(false))
        .unwrap_or(false);
    if child_alive {
        return NodeOwnership::Warming;
    }
    match btx_core::node::datadir_holder(datadir).await {
        DatadirHolder::Free => NodeOwnership::Nobody,
        DatadirHolder::ManagedBtxd { .. } | DatadirHolder::OrphanedBtxd { .. } => {
            NodeOwnership::ForeignLive
        }
        DatadirHolder::Unidentifiable { .. } => NodeOwnership::ForeignUnknown,
    }
}

/// Reclaim disk: bounce the node if needed, strip the unused indexes + the
/// (post-load) snapshot + cap debug.log, then bring the node back. Mirrors the
/// miner's reclaim semantics — deleting LevelDB dirs under a LIVE btxd can
/// crash it, so the node MUST be down during the reclaim.
#[tauri::command]
pub async fn reclaim_disk_now(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<btx_core::disk::ReclaimReport, String> {
    let datadir = node_datadir();
    let conf_path = datadir.join("faststart").join("faststart.conf");
    let owner = node_ownership(&state, &datadir).await;
    destructive_allowed(owner)?;
    let was_running = matches!(
        owner,
        NodeOwnership::OurChild | NodeOwnership::OurAdoptedOrphan
    );

    if was_running {
        stop_node_inner(&state).await;
        set_phase(&app, &state, NodePhase::Stopped).await;
    }

    let snapshot_loaded = NodeAppSettings::load(&datadir).snapshot_loaded;
    let report = {
        let dd = datadir.clone();
        tauri::async_runtime::spawn_blocking(move || {
            btx_core::disk::reclaim_disk(&dd, &conf_path, snapshot_loaded)
        })
        .await
        .map_err(|e| format!("reclaim task panicked: {e}"))?
    };
    // Invalidate the datadir-size cache so the UI reflects the reclaim now.
    *state
        .datadir_size_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = (0, None);

    if was_running {
        start_node_projected(&app, &state).await?;
    }
    Ok(report)
}

// ── Footprint: what the node costs to run right now ─────────────────────────

/// Live resource cost of the running btxd — the "it's a peaceful product"
/// numbers for the info overlay. `ps`/`tasklist` keep it dependency-free;
/// on any failure the panel just shows dashes.
#[derive(Debug, Clone, Serialize)]
pub struct NodeFootprint {
    pub running: bool,
    /// Percent of ONE core (`ps` semantics — 100 = one full core). `None`
    /// when this platform can't measure it cheaply (Windows).
    pub cpu_pct: Option<f64>,
    pub mem_mb: u64,
    /// Chain size from the cached datadir walk (0 = not measured yet).
    pub chain_mb: u64,
}

/// `ps -o %cpu=,rss= -p <pid>` → (cpu % of one core, resident KB).
#[cfg(unix)]
fn proc_stats(pid: u32) -> Option<(Option<f64>, u64)> {
    let out = std::process::Command::new("ps")
        .args(["-o", "%cpu=,rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut it = text.split_whitespace();
    let cpu = it.next()?.parse::<f64>().ok()?;
    let rss_kb = it.next()?.parse::<u64>().ok()?;
    Some((Some(cpu), rss_kb))
}

/// Windows: `tasklist /FI "PID eq <pid>" /FO CSV /NH` → memory from the last
/// quoted "Mem Usage" field (e.g. `"12,345 K"`; separators vary by locale).
/// There is no cheap per-process CPU% without sampling, so cpu is `None` and
/// the panel shows a dash for it.
#[cfg(windows)]
fn proc_stats(pid: u32) -> Option<(Option<f64>, u64)> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // `"btxd.exe","1234","Console","1","12,345 K"` — the "INFO: No tasks…"
    // no-match line has no quotes, so the rsplit below yields None for it.
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    let mem_field = line.rsplit('"').nth(1)?;
    let kb: u64 = mem_field
        .trim_end_matches(|c: char| c == 'K' || c.is_whitespace())
        .replace([',', '.', '\u{a0}'], "")
        .parse()
        .ok()?;
    Some((None, kb))
}

#[tauri::command]
pub async fn node_footprint(state: State<'_, AppState>) -> Result<NodeFootprint, String> {
    let datadir = node_datadir();
    let running = state.rpc.lock().await.is_some();
    let stats = std::fs::read_to_string(datadir.join("btxd.pid"))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .and_then(proc_stats);
    let (cpu_pct, mem_mb) = stats.map(|(c, kb)| (c, kb / 1024)).unwrap_or((None, 0));
    let chain_mb = {
        let (cached, _) = *state
            .datadir_size_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cached
    };
    Ok(NodeFootprint {
        running,
        cpu_pct,
        mem_mb,
        chain_mb,
    })
}

#[cfg(test)]
mod tests {
    use super::{pre_launch_plan, PreLaunchPlan, NODE_RELEASE_TAG};
    use btx_core::node::DatadirHolder;

    // The four things `<datadir>/btxd.pid` can turn out to mean, named so the
    // tables below read as sentences. That they carry a pid at all is the
    // 2026-09-04 change: a holder is a process, not a boolean.
    const NOTHING: DatadirHolder = DatadirHolder::Free;
    const AN_ORPHAN: DatadirHolder = DatadirHolder::OrphanedBtxd { pid: 717 };
    const ANOTHER_APPS: DatadirHolder = DatadirHolder::ManagedBtxd { pid: 717 };
    const UNNAMEABLE: DatadirHolder = DatadirHolder::Unidentifiable { pid: 717 };
    // The recheck budget, before and after it is spent.
    const FIRST_LOOK: bool = false;
    const BUDGET_SPENT: bool = true;

    // ── The tag-migration + attach decision ─────────────────────────────────
    // Regression tests for the 2026-08-12 0.6.1→0.6.2 self-update failure:
    // see `pre_launch_plan`'s doc comment for the observed sequence.

    /// Flavor A of the regression: a healthy old-binaries node answers RPC
    /// right after a tag migration. Attaching (the old behavior) silently
    /// keeps the fleet on the pre-upgrade consensus code — for 0.6.2 that
    /// meant staying wedged below Epoch-A. The upgrade start must bounce it.
    #[test]
    fn upgrade_start_restarts_a_serving_orphan_instead_of_attaching() {
        assert_eq!(
            pre_launch_plan(true, true, AN_ORPHAN, FIRST_LOOK),
            PreLaunchPlan::RestartForUpgrade
        );
    }

    /// The spec's guard: when the persisted tag already matches, the serving
    /// node runs the binaries we'd launch — plain attach stays correct.
    #[test]
    fn plain_start_attaches_when_the_tag_did_not_move() {
        assert_eq!(
            pre_launch_plan(true, false, AN_ORPHAN, FIRST_LOOK),
            PreLaunchPlan::Attach
        );
        // Managed by a live app (miner solo): also attach.
        assert_eq!(
            pre_launch_plan(true, false, ANOTHER_APPS, FIRST_LOOK),
            PreLaunchPlan::Attach
        );
    }

    /// Never bounce a node another LIVE app supervises, even for an upgrade —
    /// stopping the miner's solo node would fight its recovery supervisor.
    /// The upgrade lands on that node's next natural restart instead.
    #[test]
    fn upgrade_start_leaves_a_live_apps_node_alone() {
        assert_eq!(
            pre_launch_plan(true, true, ANOTHER_APPS, FIRST_LOOK),
            PreLaunchPlan::Attach
        );
        // Same restraint when the holder is merely unnameable: an upgrade is
        // not a reason to bounce a serving node we cannot identify. It lands
        // on that node's next restart.
        assert_eq!(
            pre_launch_plan(true, true, UNNAMEABLE, FIRST_LOOK),
            PreLaunchPlan::Attach
        );
    }

    /// Flavor B of the regression — the observed crash: the probe missed (old
    /// node busy past the RPC timeout / mid-shutdown), an orphaned btxd still
    /// held the datadir, and the old code spawned straight into the held lock.
    /// The new plan stops/waits the orphan out first — upgrade or not, and
    /// without spending the recheck budget: an orphan is proven and actionable
    /// on the first look.
    #[test]
    fn an_unmanaged_holder_without_rpc_is_stopped_and_waited_out_not_raced() {
        assert_eq!(
            pre_launch_plan(false, true, AN_ORPHAN, FIRST_LOOK),
            PreLaunchPlan::ClearStaleHolder
        );
        assert_eq!(
            pre_launch_plan(false, false, AN_ORPHAN, FIRST_LOOK),
            PreLaunchPlan::ClearStaleHolder
        );
    }

    // ── Identifying the holder, and not refusing forever ────────────────────
    // Regression tests for the 2026-09-04 stand-down on the Linux signer rig:
    // see `pre_launch_plan`'s doc comment for the observed sequence.

    /// THE REGRESSION. `btxd.pid` held 717 from a btxd that had died without
    /// cleaning up; after the restart the OS had given 717 to an unrelated
    /// process. There was no btxd on the machine and nothing on 19334, yet the
    /// old code read "a live pid" as another app's node and refused to start —
    /// every time, forever. A recycled pid holds nothing, so the plan is the
    /// only one that ends with the user having a node.
    #[test]
    fn a_recycled_pid_is_not_a_reason_to_refuse_to_start() {
        assert_eq!(
            pre_launch_plan(false, false, NOTHING, FIRST_LOOK),
            PreLaunchPlan::SpawnFresh
        );
        assert_eq!(
            pre_launch_plan(false, true, NOTHING, BUDGET_SPENT),
            PreLaunchPlan::SpawnFresh
        );
    }

    /// A live app's node that isn't answering (still starting, or busy) is not
    /// ours to stop OR to race — but the first look is not the last word. It is
    /// looked at again for the whole budget, and only a holder still proven to
    /// be another app's btxd at the end of it fails the start.
    #[test]
    fn a_live_apps_node_that_is_not_answering_is_waited_on_then_left_alone() {
        assert_eq!(
            pre_launch_plan(false, true, ANOTHER_APPS, FIRST_LOOK),
            PreLaunchPlan::RecheckHolder
        );
        assert_eq!(
            pre_launch_plan(false, false, ANOTHER_APPS, FIRST_LOOK),
            PreLaunchPlan::RecheckHolder
        );
        assert_eq!(
            pre_launch_plan(false, true, ANOTHER_APPS, BUDGET_SPENT),
            PreLaunchPlan::ManagedElsewhereNoRpc
        );
        assert_eq!(
            pre_launch_plan(false, false, ANOTHER_APPS, BUDGET_SPENT),
            PreLaunchPlan::ManagedElsewhereNoRpc
        );
    }

    /// A holder we could not name gets the same bounded benefit of the doubt,
    /// and then we launch. Standing down permanently on a pid we could not even
    /// attach a name to is how a home node stays off the network all day; a
    /// lost lock race, by contrast, announces itself and is retried.
    #[test]
    fn an_unnameable_holder_is_waited_on_then_adopted() {
        assert_eq!(
            pre_launch_plan(false, false, UNNAMEABLE, FIRST_LOOK),
            PreLaunchPlan::RecheckHolder
        );
        assert_eq!(
            pre_launch_plan(false, false, UNNAMEABLE, BUDGET_SPENT),
            PreLaunchPlan::AdoptUnprovenHolder
        );
    }

    /// The budget has to be a real wait and a bounded one. Zero rechecks is the
    /// old refuse-on-first-look behaviour; a budget of minutes turns "the miner
    /// is warming up" into an app that looks hung.
    #[test]
    fn the_holder_recheck_budget_is_bounded_and_not_zero() {
        let total = super::HOLDER_RECHECKS as u64 * super::HOLDER_RECHECK_WAIT.as_secs();
        assert!(
            super::HOLDER_RECHECKS >= 1,
            "zero rechecks restores the permanent refusal this exists to fix"
        );
        assert!(
            (10..=120).contains(&total),
            "the whole recheck budget is {total}s — long enough to be a wait, \
             short enough that a person does not read it as a hang"
        );
    }

    #[test]
    fn nothing_holding_the_datadir_spawns_fresh() {
        assert_eq!(
            pre_launch_plan(false, true, NOTHING, FIRST_LOOK),
            PreLaunchPlan::SpawnFresh
        );
        assert_eq!(
            pre_launch_plan(false, false, NOTHING, FIRST_LOOK),
            PreLaunchPlan::SpawnFresh
        );
    }

    /// The two quit budgets are INDEPENDENT literals, and their order is the
    /// invariant: the backstop must outlast the flush it is backstopping.
    /// Lowering `QUIT_GRACE` below the stop grace would silently re-introduce
    /// the mid-flush kill from the other direction — the quit would force-exit
    /// while btxd was still writing, with no log line to say so.
    /// (`ATTACHED_STOP_GRACE` is *derived* from the managed path's grace, so
    /// the two stop paths cannot drift apart at all — that needs no test.)
    #[test]
    fn the_quit_backstop_outlasts_the_flush_it_backstops() {
        assert!(
            super::ATTACHED_STOP_GRACE < super::QUIT_GRACE,
            "stop grace {:?} must fit inside the quit backstop {:?}",
            super::ATTACHED_STOP_GRACE,
            super::QUIT_GRACE
        );
    }

    /// The keeper gate is a DENYLIST, so a pin bump silently changes its answer.
    /// It answered NO for v0.33.3-pr105b, the docs said so in the present tense,
    /// and when the pin moved to v0.34.5 the answer flipped to YES with nothing
    /// recording that anyone decided it. Assert the value this release intends,
    /// so the next bump has to look at this line.
    #[test]
    fn the_upgrade_path_never_unprunes_a_datadir_the_disk_cannot_hold() {
        use super::upgrade_conf_choice;
        use btx_core::installer::{NODE_FASTSTART_CONF, NODE_KEEPER_CONF};
        // Disk fine: the selected profile wins, whatever the datadir was.
        assert_eq!(
            upgrade_conf_choice(NODE_FASTSTART_CONF, Some(true), true),
            Some(NODE_FASTSTART_CONF)
        );
        assert_eq!(
            upgrade_conf_choice(NODE_KEEPER_CONF, Some(false), true),
            Some(NODE_KEEPER_CONF)
        );
        // Selecting keeper never needs disk: honoured even when the disk is full.
        assert_eq!(
            upgrade_conf_choice(NODE_KEEPER_CONF, Some(false), false),
            Some(NODE_KEEPER_CONF)
        );
        // Full selected, pruned datadir, no room: keep it pruned (the shipped
        // engine admits the keeper conf, so there IS a safe conf).
        assert_eq!(
            upgrade_conf_choice(NODE_FASTSTART_CONF, Some(true), false),
            Some(NODE_KEEPER_CONF)
        );
        // Full selected, already un-pruned, no room: nothing changes posture.
        assert_eq!(
            upgrade_conf_choice(NODE_FASTSTART_CONF, Some(false), false),
            Some(NODE_FASTSTART_CONF)
        );
        // Full selected, unknown posture, no room: refuse rather than guess.
        assert_eq!(upgrade_conf_choice(NODE_FASTSTART_CONF, None, false), None);
    }

    #[test]
    fn destructive_ops_are_allowed_only_for_a_node_that_is_ours_or_nobodys() {
        use super::{destructive_allowed, NodeOwnership::*};
        for ok in [OurChild, OurAdoptedOrphan, Nobody] {
            assert!(destructive_allowed(ok).is_ok(), "{ok:?}");
        }
        for refused in [
            AnotherApp,
            AttachedToUnknown,
            Warming,
            ForeignLive,
            ForeignUnknown,
        ] {
            let msg = destructive_allowed(refused).unwrap_err();
            assert!(!msg.is_empty(), "{refused:?}");
        }
        // The refusal for our own warming node must not blame the miner: that
        // wording sent people to quit an app that was not running.
        let warming = destructive_allowed(Warming).unwrap_err();
        assert!(warming.contains("still starting up"), "{warming}");
        assert!(!warming.contains("miner"), "{warming}");
    }

    #[test]
    fn a_saved_nickname_reaches_the_conf_only_through_the_validator() {
        use super::conf_nickname;
        // The Settings box validates; this is the OTHER path, the one that
        // runs on every start from a file anyone can edit.
        assert_eq!(conf_nickname("alice").as_deref(), Some("alice"));
        assert_eq!(conf_nickname("  alice  ").as_deref(), Some("alice"));
        assert_eq!(conf_nickname(""), None);
        // btxd refuses to START on these; they must never reach the conf.
        assert_eq!(conf_nickname("rig/01"), None);
        assert_eq!(conf_nickname("a(b)"), None);
        assert_eq!(conf_nickname("a\nrpcallowip=0.0.0.0/0"), None);
    }

    #[test]
    fn the_shipped_pin_makes_a_deliberate_keeper_decision() {
        assert!(
            btx_core::installer::engine_supports_keeper_profile(NODE_RELEASE_TAG),
            "pin {NODE_RELEASE_TAG} no longer admits the keeper conf; if that is              intended, update this assertion and say so in the CHANGELOG"
        );
        // And the conf that follows from it, so the two cannot drift apart.
        assert_eq!(
            btx_core::installer::conf_for_profile("keeper", NODE_RELEASE_TAG),
            btx_core::installer::NODE_KEEPER_CONF
        );
        assert_eq!(
            btx_core::installer::conf_for_profile("full", NODE_RELEASE_TAG),
            btx_core::installer::NODE_FASTSTART_CONF
        );
    }

    #[test]
    fn proc_stats_reads_our_own_process() {
        let (cpu, rss_kb) = super::proc_stats(std::process::id()).expect("stats for our own pid");
        #[cfg(unix)]
        assert!(cpu.expect("unix measures cpu%") >= 0.0);
        #[cfg(windows)]
        assert!(cpu.is_none(), "windows reports no cheap per-process cpu%");
        assert!(rss_kb > 0, "a live process has resident memory");
    }
}

/// Remove the node's chain data entirely and return the app to the setup
/// screen — the "give me my disk back" lever for a full chain measured at
/// ~124 GiB (see setup.rs::MEASURED_CHAIN_PAYLOAD_GIB).
/// Stops the node gracefully first, deletes the chain dirs (btx-core's
/// remove_node_data: blocks/chainstate/indexes/faststart + debug.log — never
/// wallets or the miner's state) plus the node's sidecar files, and resets
/// setup_complete so the wizard runs a clean re-setup whenever the user wants
/// the node again.
#[tauri::command]
pub async fn remove_node_data_now(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<btx_core::disk::ReclaimReport, String> {
    let datadir = node_datadir();
    // Ownership first, before the stop: in attach mode stop_node_inner is NOT
    // a no-op, it gracefully stops and then force-kills the attached node, so
    // the order here is what keeps another app's btxd out of the delete.
    destructive_allowed(node_ownership(&state, &datadir).await)?;
    stop_node_inner(&state).await;

    let report = {
        let dd = datadir.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let mut report = btx_core::disk::remove_node_data(&dd);
            // Node sidecars btx-core's helper leaves behind (it serves the
            // miner's lite-pool too): snapshot-era dirs + p2p/mempool state +
            // this app's node logs. All rebuilt by a fresh setup.
            for name in ["chainstate_snapshot", "shielded_state"] {
                let dir = dd.join(name);
                if dir.is_dir() {
                    let bytes = btx_core::disk::dir_size_bytes(&dir);
                    if std::fs::remove_dir_all(&dir).is_ok() {
                        report.freed_mb += bytes / (1024 * 1024);
                        report.items.push(name.to_string());
                    }
                }
            }
            for name in [
                "mempool.dat",
                "peers.dat",
                "banlist.json",
                "fee_estimates.dat",
                "easybtx-node.log",
                "easybtx-node.log.prev",
                "btxd.pid",
            ] {
                let f = dd.join(name);
                if let Ok(meta) = std::fs::metadata(&f) {
                    let bytes = meta.len();
                    if std::fs::remove_file(&f).is_ok() {
                        report.freed_mb += bytes / (1024 * 1024);
                    }
                }
            }
            report
        })
        .await
        .map_err(|e| format!("removal task panicked: {e}"))?
    };

    // Back to factory: the wizard owns the next setup. Explorer mode's
    // txindex flag resets too (its index was just deleted with the chain).
    NodeAppSettings::update(&datadir, |s| {
        s.setup_complete = false;
        s.snapshot_loaded = false;
        s.txindex_enabled = false;
    });
    *state
        .datadir_size_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = (0, None);
    set_phase(&app, &state, NodePhase::Welcome).await;
    Ok(report)
}

/// Open the network-wide stats page (btxprice.com/stats) in the default
/// browser. Fixed URL on purpose: the webview never chooses what to open,
/// so there is no arbitrary-URL surface.
#[tauri::command]
pub async fn open_global_stats() -> Result<(), String> {
    const URL: &str = "https://btxprice.com/stats";
    btx_core::platform::open_url(URL).map_err(|e| format!("couldn't open the stats page: {e}"))
}
