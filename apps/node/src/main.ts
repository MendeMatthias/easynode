// easyBTX Node frontend: a calm, glanceable status surface over the Rust
// lifecycle commands. One poll loop drives everything — the phase enum from
// the backend decides which screen renders.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { check as checkForUpdate } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { AmbientLine } from "./ambient";
import { validationView } from "./validation";
import {
  CHAIN_BLOCKS_PER_HOUR,
  type CatchupSample,
  cannotCatchUp,
  catchupLine,
  pushSample,
} from "./catchup-trend";
import {
  classifyCheckFailure,
  checkFailureMessage,
  installErrorFromDetail,
  lastCheckLine,
  updateCheckRecord,
  type LastUpdateCheck,
  type UpdateCheckBranch,
  type UpdateCheckEvent,
} from "./update-check";
import { contributionView } from "./contribution";
import { mountPowerCore } from "./power-core";
import { initAsk } from "./ask";
import { initWallet, reflectWalletEnabled } from "./wallet";
type PowerCore = ReturnType<typeof mountPowerCore>;

// ── Accent theme bootstrap (before anything renders) ────────────────────────
const ACCENT_KEY = "ebtx-node.accent";
{
  // BTX Node v0.2 brand reset: green is the brand default. v0.1.x auto-saved
  // "ember" on first launch (a default, not a choice), so ember migrates to
  // the brand green ONCE; ocean/nova were deliberate picks and survive. A
  // user re-picking ember in Settings after the migration keeps it.
  const MIGRATED_KEY = "ebtx-node.accent-v2";
  const saved = localStorage.getItem(ACCENT_KEY);
  if (!localStorage.getItem(MIGRATED_KEY)) {
    localStorage.setItem(MIGRATED_KEY, "1");
    if (!saved || saved === "ember") localStorage.setItem(ACCENT_KEY, "btx");
  }
  const accent = localStorage.getItem(ACCENT_KEY) ?? "btx";
  if (accent !== "ember") document.documentElement.dataset.accent = accent;
}

// ── Visual style: "calm" frequency line (default) or "energy" power core ────
const VISUAL_KEY = "ebtx-node.visual";
type Visual = "calm" | "energy";
let visual: Visual = localStorage.getItem(VISUAL_KEY) === "energy" ? "energy" : "calm";
document.documentElement.dataset.visual = visual;

// ── Types mirrored from src-tauri/src/state.rs + commands.rs ────────────────

type NodePhase =
  | { phase: "welcome" }
  | { phase: "downloading"; progress: number }
  | { phase: "preparing" }
  | { phase: "starting" }
  | { phase: "warming"; message: string }
  | { phase: "loading_snapshot" }
  | { phase: "syncing"; height: number; headers: number; progress: number; peers: number }
  | { phase: "ready"; height: number; peers: number; blocks_behind: number }
  | { phase: "stopped" }
  | { phase: "error"; message: string };

export interface SnapshotServeStatus {
  /** true while the offer is on the wire; null when unmeasured. */
  offering: boolean | null;
  base_height: number | null;
  base_hash: string | null;
  /** Blocks the tip has moved past the offered base. */
  stale_by: number | null;
  file_size: number | null;
  sha256: string | null;
  /** Peers that advertise a snapshot of their own. */
  peers_offering: number | null;
  /** Where a cycle is, while one runs. */
  phase: { kind: string } | null;
  /** The sentence beside the switch. */
  message: string;
  /** The message names something the operator has to change. */
  needs_attention: boolean;
}

interface NodeStatusInfo {
  running: boolean;
  phase: NodePhase;
  uptime_secs: number;
  disk_free_mb: number;
  disk_warn_mb: number;
  disk_critical_mb: number;
  disk_required_mb: number;
  datadir_size_mb: number;
  datadir: string;
  node_tag: string;
  installed: boolean;
  setup_complete: boolean;
  /** False only on a brand new install that has not seen the welcome panel. */
  welcome_shown: boolean;
  keep_awake: boolean;
  keep_awake_supported: boolean;
  tray_term: string;
  txindex_enabled: boolean;
  /**
   * Serve historical signed confirmations back to the network
   * (matmulattestationserve). Persisted choice or an adopted hand-set conf
   * flag; a change applies on the next node (re)start.
   */
  attestation_serve_enabled: boolean;
  /** The signer role (btx_core::signer): the switch; whether this host can
   *  sign at all (null before the first start, false on a mirror host); the
   *  public key to show and copy (null when off or when there is no key);
   *  and whether the engine reports this node holding a key AND validating,
   *  which is the only state in which quitting freezes somebody's mirror. */
  signer_enabled: boolean;
  signer_applies_here: boolean | null;
  signer_pubkey: string | null;
  signing_live: boolean;
  /** Offer the public key to easybtx.com so mirror operators can pin it, and
   *  what came of the last attempt this run (null until one has been made). */
  signer_publish_enabled: boolean;
  signer_offer: { delivered: boolean; at: string; detail: string } | null;
  /** What we are really providing: `state` is serving_history |
   *  degraded_to_live_window | not_serving | unknown. */
  archive_service: { state: string; blocks_behind?: number } | null;
  // The same verdict as a sentence, rendered in Rust so the copy lives in one
  // place. Null until the refresher has completed a tick.
  archive_service_message: string | null;
  archive_service_needs_attention: boolean;
  /**
   * Which role this node fills on the network, decided in Rust
   * (btx_core::role) from the engine's own answers, never from a setting.
   * Null when the node is stopped or getnetworkinfo did not answer.
   * `validation_mode` is consensus | trusted | relay | unknown;
   * `holds_signing_key` is null when the engine did not answer, which is not
   * evidence of no key.
   */
  role: {
    validation_mode: string;
    holds_signing_key: boolean | null;
    advertises_consensus: boolean;
    advertises_archive: boolean;
    reachable_inbound: boolean;
    inbound: number;
    uptime_secs: number;
    blocks_behind: number | null;
    /** How many of the newest blocks carry this node's own signature, read
     *  from its stored attestations; null until read or when there is no
     *  key. `signed` is out of `seen`, not out of `window`. */
    signed_recent: { signed: number; seen: number; window: number } | null;
  } | null;
  /** One line per fact with its sentence pre-rendered in Rust, like
   *  archive_service_message: the copy lives in one place, with tests. Empty
   *  when `role` is null. `helps` is true | false | null — null when the
   *  engine did not say, or when the fact is simply neutral. */
  role_lines: { label: string; value: string; helps: boolean | null; note: string }[];
  /** A longer chain this node cannot obtain blocks for; null when healthy or
   *  not yet measured. `kind` is longer_branch | waiting_for_bodies |
   *  headers_ahead — waiting_for_bodies is the same branch shape as
   *  longer_branch with btxd itself saying no peer is serving the bodies, i.e.
   *  a propagation race rather than a split. The sentence
   *  is rendered in Rust (fork_message), like archive_service_message. */
  fork: { kind: string; since_secs?: number } | null;
  fork_message: string | null;
  /** The tip's own age against the wall clock, not against what our peers
   *  believe. Every other chain signal here — blocks, headers, getchaintips,
   *  and so `fork` too — is derived from the peers we happen to have, so a
   *  node stuck together with its peers looks healthy in all of them. A block
   *  timestamp cannot agree with a stuck peer set. The sentence is rendered in
   *  Rust, like fork_message. */
  tip_stale: boolean;
  tip_age_secs: number | null;
  tip_stale_message: string | null;
  node_nickname: string;
  broadcast_nickname: string | null;
  subversion: string | null;
  peer_nicknames: string[];
  service_report_enabled: boolean;
  wallet_enabled: boolean;
  on_close: string;
  /** btxd's OWN MatMul RC execution mode. Null until it has logged one. */
  rc_mode: string | null;
  rc_validates_independently: boolean;
  rc_may_fall_behind: boolean;
  rc_reason: string | null;
  rc_stalled: boolean;
  /** Following the chain via an attestation quorum instead of local replay. */
  rc_trusted_mirror: boolean;
  /** The owner chose to follow signatures on a machine that could validate. */
  follow_signatures: boolean;
  /**
   * Bytes uploaded to peers this run. Null when stopped or when the node did
   * not answer `getnettotals` — the UI drops the claim rather than showing a
   * zero the node never earned.
   */
  bytes_sent: number | null;
  inbound_peers: number | null;
  /**
   * Trusted-mirror peer health from `getpeerinfo`: archive peers seen, how
   * many pass the authority gate (the ones the node will actually ask for
   * signed confirmations), and attestation flow both ways. Null when stopped
   * or unanswered.
   */
  archive_peers: {
    archive_bit: number;
    authority: number;
    feeding_us: number;
    served_by_us: number;
  } | null;
  /**
   * The stall discriminator's verdict for a frozen trusted mirror (null =
   * healthy / no verdict). class is one of "body_missing" |
   * "attestation_missing" | "no_qualifying_peer" | "msghand_spin".
   */
  stall: { class: string; summary: string } | null;
  /** The user's node profile CHOICE ("full" | "keeper"). */
  node_profile: string;
  /** The folder already deleted blocks, whatever the profile says. */
  datadir_pruned: boolean;
  /** Whether the bundled engine can honour the keeper profile yet. */
  keeper_engine_ready: boolean;
  /** Esplora mode: serve the Esplora REST API to wallets from this node. */
  esplora_enabled: boolean;
  esplora_listen: string;
  /** The address the RUNNING front is bound to; null when nothing runs. Not
   *  the same as the setting, which applies at the next start. */
  esplora_serving_on: string | null;
  esplora_running: boolean;
  /** Both sidecars alive, no tip yet: a first index, which takes hours. */
  esplora_indexing: boolean;
  /** fresh | stale | unverified from the census guardian; null until judged. */
  esplora_freshness: string | null;
  /** The guardian's reason while running, else why the front is not. */
  esplora_message: string | null;
  /** Answer block-hash questions for wallets. Needs no second binary and no
   *  particular prune posture: the server is compiled into this app. */
  witness_enabled: boolean;
  witness_listen: string;
  /** The address the running server is BOUND to, which is not always the
   *  saved one: the setting applies at the next node start. */
  witness_serving_on: string | null;
  witness_running: boolean;
  /** True when the bind address accepts connections from other machines. */
  witness_public: boolean;
  witness_message: string | null;
  /** Produce and serve an attested snapshot of the chain state: the choice,
   *  and the keeper's last word (null while off or the node is down). */
  snapshot_serve_enabled: boolean;
  snapshot_serve: SnapshotServeStatus | null;
  /** The last self-update check as the backend persisted it: when it finished
   *  (RFC 3339, UTC), one of the five words in UPDATE_CHECK_OUTCOMES, and the
   *  short detail recorded with it. Null/empty until the first check finishes.
   *  The "Last check" line under the Check-now button is rendered from these
   *  on every tick, so the automatic path is no longer silent. */
  last_update_check_at: string | null;
  last_update_check_outcome: string | null;
  last_update_check_detail: string;
}

/**
 * What `esplora_preflight` answers: can this machine serve Esplora, what will
 * it cost, and are the two binaries even here. Read BEFORE the operator flips
 * the switch, which is the only moment any of it is useful.
 */
interface EsploraPreflight {
  allowed: boolean;
  /** The gate's sentence when it refuses: a pruning conf, an already-pruned
   *  datadir, or the keeper profile. Each needs a different conversation. */
  blocker: string | null;
  /** Non-blocking costs, chiefly the disk one. */
  warnings: string[];
  /** Where the binaries were found, or null when they are not installed. */
  electrs_found: string | null;
  caddy_found: string | null;
  listen: string;
}

interface ReclaimReport {
  freed_mb: number;
  items: string[];
}

// ── DOM handles ──────────────────────────────────────────────────────────────

const $ = <T extends HTMLElement = HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const ambientMain = new AmbientLine($("ambient-canvas") as unknown as HTMLCanvasElement);
const ambientCompact = new AmbientLine($("compact-canvas") as unknown as HTMLCanvasElement);
// QA hook: lets a test harness step frames / flip modes without reaching into
// module scope. Harmless in production.
(window as unknown as Record<string, unknown>).__ambient = { main: ambientMain, compact: ambientCompact };

// ── Power core (energy pulse): one instance on the currently-VISIBLE surface ─
// The core is a full WebGL context, so we mount it only for the surface on
// screen (status or compact) and dispose on switch — never two contexts, and
// never one drawing into a hidden canvas.
let core: PowerCore | null = null;
let coreSurface: "main" | "compact" | null = null;
let lastActive = false; // whether the node is running (drives the core's intensity)

function ensureCore(surface: "main" | "compact"): void {
  if (core && coreSurface === surface) return;
  core?.dispose();
  const id = surface === "compact" ? "compact-power-canvas" : "power-canvas";
  core = mountPowerCore($(id) as unknown as HTMLCanvasElement);
  coreSurface = surface;
  // Calm tier: intensity stays < 0.5 so the fusion-flash never fires, and
  // power well below the overdrive band so no shockwave — the energy pulse
  // "without the heavy explosions".
  core.setPower(28);
  core.setActive(lastActive);
}
function disposeCore(): void {
  core?.dispose();
  core = null;
  coreSurface = null;
}
// QA hook (harmless in production, mirrors __ambient): drive the running state
// so a headless capture can show the active vs idle core.
(window as unknown as Record<string, unknown>).__core = {
  setActive: (b: boolean) => {
    lastActive = b;
    core?.setActive(b);
  },
};

/** Apply the chosen visual to whichever surface (status/compact) is on screen. */
function applyVisual(): void {
  const compact = document.body.classList.contains("compact");
  document.documentElement.dataset.visual = visual;
  if (visual === "energy") {
    ambientMain.stop();
    ambientCompact.stop();
    ensureCore(compact ? "compact" : "main");
  } else {
    disposeCore();
    if (compact) {
      ambientMain.stop();
      ambientCompact.start();
    } else {
      ambientCompact.stop();
      ambientMain.start();
    }
  }
}
let lastPulseHeight = 0;

const screenWizard = $("screen-wizard");
const screenStatus = $("screen-status");
const wizardProgress = $("wizard-progress");
const wizardError = $("wizard-error");
const setupBtn = $<HTMLButtonElement>("setup-btn");
const setupBtnLabel = $("setup-btn-label");
const wizardIdleNote = $("wizard-idle-note");
const wizardSetupNote = $("wizard-setup-note");
const toggleNodeBtn = $<HTMLButtonElement>("toggle-node-btn");

/** Put the setup button into its live "working" look, or back to idle.
 *  `label` becomes the button text; while loading it also carries the current
 *  phase (e.g. "Downloading… 34%") so the button itself is the progress. */
function setSetupButton(loading: boolean, label: string): void {
  setupBtn.classList.toggle("is-loading", loading);
  setupBtn.disabled = loading;
  setupBtnLabel.textContent = label;
  wizardIdleNote.hidden = loading;
  wizardSetupNote.hidden = !loading;
}

/** Plain-language, live button text for each setup phase. */
function setupPhaseLabel(phase: NodePhase): string {
  switch (phase.phase) {
    case "downloading":
      return `Downloading the snapshot… ${Math.round(phase.progress * 100)}%`;
    case "preparing":
      return "Preparing the node…";
    case "starting":
    case "warming":
      return "Starting your node…";
    case "loading_snapshot":
      return "Loading the snapshot…";
    default:
      return "Setting up your node…";
  }
}

// ── Formatting helpers ───────────────────────────────────────────────────────

// How far behind the best header the active chain has to be before the badge
// says so. Small enough that a real catch-up shows, large enough that ordinary
// reorg churn (this chain rebuilds its tip ~91x/day) does not make the line
// flicker. Presentation only — nothing decides behaviour on it.
const LAG_WORTH_SAYING = 10;

function fmtGB(mb: number): string {
  if (mb <= 0) return "—";
  // The value is mebibytes and the divisor is 1024, so the unit is GiB. It said
  // GB, which understated every figure it printed by about 7% against the
  // backend's own message for the same quantity.
  return `${(mb / 1024).toFixed(1)} GiB`;
}

function fmtUptime(secs: number): string {
  if (secs <= 0) return "—";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const sec = Math.floor(secs % 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  // Under an hour: tick visibly every poll — a moving number is the
  // "something is alive" signal (same instinct as the miner's live stats).
  if (m > 0) return `${m}m ${sec.toString().padStart(2, "0")}s`;
  return `${sec}s`;
}

function fmtInt(n: number): string {
  return n.toLocaleString("en-US");
}

// One derivation of "what state is the node in" — the orbs, the ambient line,
// and the Start/Stop button all read THIS, so they can never disagree.
type VisualMode = "ready" | "syncing" | "stopped" | "error" | "stalled";
/**
 * `rcStalled` = btxd reported it cannot check the new proof of work. The process
 * is up and answering RPC, so every phase signal still says "ready" — but the
 * chain is not advancing. Without this the app shows a confident green LIVE on a
 * node that stopped following BTX, which is the worst thing it could say.
 */
function visualMode(p: NodePhase, rcStalled = false): VisualMode {
  switch (p.phase) {
    case "ready":
      return rcStalled ? "stalled" : "ready";
    case "syncing":
    case "starting":
    case "warming":
    case "loading_snapshot":
      return "syncing";
    case "error":
      return "error";
    default:
      return "stopped";
  }
}
/**
 * Phases where a node run is active (Stop is the meaningful action). "stalled"
 * counts: the process really is running, and Stop is exactly what the user may
 * want to do about it.
 */
function isNodeActive(p: NodePhase, rcStalled = false): boolean {
  const m = visualMode(p, rcStalled);
  return m === "ready" || m === "syncing" || m === "stalled";
}
/** The ambient line has no "stalled" of its own; it reads as a problem. */
function ambientMode(m: VisualMode): "ready" | "syncing" | "stopped" | "error" {
  return m === "stalled" ? "error" : m;
}

// ── Screen routing ───────────────────────────────────────────────────────────

// Once setup has ever completed we never fall back to the wizard, even through
// transient error phases — errors then render on the status screen.
let setupDone = false;
// While a setup click is in flight the wizard owns the screen.
let setupInFlight = false;

function showScreen(which: "wizard" | "status") {
  screenWizard.hidden = which !== "wizard";
  screenStatus.hidden = which !== "status";
}

// ── Wizard rendering ─────────────────────────────────────────────────────────

const STEP_IDS = ["step-download", "step-prepare", "step-start", "step-load"] as const;

function setStep(active: number, downloadPct?: number) {
  STEP_IDS.forEach((id, i) => {
    const el = $(id);
    el.classList.toggle("is-done", i < active);
    el.classList.toggle("is-active", i === active);
  });
  $("download-pct").textContent =
    downloadPct !== undefined ? `${Math.round(downloadPct * 100)}%` : "";
  // The track shows a real % during download; otherwise (right after the click,
  // and the quick prepare/start/load steps) it sweeps an indeterminate band so
  // it never looks stuck at 0%.
  const track = $("progress-track");
  const fill = $("setup-progress-fill");
  const hasPct = downloadPct !== undefined && downloadPct > 0;
  track.classList.toggle("is-indeterminate", !hasPct);
  if (hasPct) {
    fill.style.width = `${Math.round(downloadPct! * 100)}%`;
  } else {
    fill.style.width = ""; // let the indeterminate animation own the width
  }
}

function renderWizard(status: NodeStatusInfo) {
  showScreen("wizard");
  const p = status.phase;
  const inProgress =
    setupInFlight ||
    p.phase === "downloading" ||
    p.phase === "preparing" ||
    p.phase === "starting" ||
    p.phase === "warming" ||
    p.phase === "loading_snapshot";

  // The button IS the live readout while setting up; idle otherwise.
  if (inProgress) setSetupButton(true, setupPhaseLabel(p));
  else if (p.phase !== "error") setSetupButton(false, "Set up my node");
  wizardProgress.hidden = !inProgress;
  wizardError.hidden = p.phase !== "error";

  $("wizard-free-disk").textContent = fmtGB(status.disk_free_mb);
  // The fresh-install figure for the SELECTED profile, from the same
  // disk_required the preflight applies — 20 GiB for a keeper, 140 for a full
  // node — never a string in the markup. It used to render the full-node
  // constant unconditionally, which told keepers they needed 140 GiB for an
  // install the preflight would pass at 20. Always the fresh figure: a resume
  // is gated lower, but overstating is the direction that never strands anyone.
  $("wizard-disk-needed").textContent = fmtGB(status.disk_required_mb);

  switch (p.phase) {
    case "downloading":
      setStep(0, p.progress);
      break;
    case "preparing":
      setStep(1);
      break;
    case "starting":
    case "warming":
      setStep(2);
      break;
    case "loading_snapshot":
      setStep(3);
      break;
    case "error":
      $("wizard-error-msg").textContent = p.message;
      setSetupButton(false, "Set up my node");
      setupBtn.disabled = true;
      // One-shot: bring the error into view the tick it appears (it lives
      // below the button and used to be unreachable past the fold), but never
      // fight the user's scrolling on subsequent polls.
      if (!wizardErrorShown) {
        wizardErrorShown = true;
        wizardError.scrollIntoView({ behavior: "smooth", block: "nearest" });
      }
      break;
    default:
      break;
  }
  if (p.phase !== "error") wizardErrorShown = false;
}

// ── Status rendering ─────────────────────────────────────────────────────────

function renderValidation(status: NodeStatusInfo) {
  const card = $("validation-card");
  const view = validationView({
    ...status,
    archive_authority: status.archive_peers?.authority ?? null,
    stall: status.stall,
  });
  if (view.state === null) {
    card.hidden = true;
    return;
  }
  card.hidden = false;
  card.classList.remove("is-degraded", "is-stalled");
  if (view.cls) card.classList.add(view.cls);
  $("validation-state").textContent = view.state;
  $("validation-note").textContent = view.note;
}

/**
 * "Helping the network" — what this node gives back.
 *
 * The peer count only rides on the `ready` phase, so a syncing node passes
 * null rather than 0: it is not evidence of no peers, it is absence of
 * evidence, and contributionView treats the two differently on purpose.
 */
function renderContribution(status: NodeStatusInfo) {
  const card = $("contribution-card");
  const view = contributionView({
    running: status.running,
    peers:
      status.phase.phase === "ready" || status.phase.phase === "syncing"
        ? status.phase.peers
        : null,
    uptimeSecs: status.uptime_secs,
    bytesSent: status.bytes_sent ?? null,
    inboundPeers: status.inbound_peers ?? null,
    attestationsServedPeers: status.archive_peers?.served_by_us ?? null,
  });
  if (view.headline === null) {
    card.hidden = true;
    return;
  }
  card.hidden = false;
  card.classList.remove("is-live", "is-waiting");
  if (view.cls) card.classList.add(view.cls);
  $("contribution-headline").textContent = view.headline;
  $("contribution-detail").textContent = view.detail;
}

/** The one-time greeting on a brand new install.
 *
 *  Shown once, and only when the backend says so. `welcome_shown` is false
 *  only when there was no settings file at all, so an existing user updating
 *  the app is never greeted for a node they set up weeks ago
 *  (NodeAppSettings::welcome_shown carries the serde trick that makes those
 *  two cases differ).
 *
 *  The services it lists are already ON, from the defaults. This panel exists
 *  so that is said out loud rather than done quietly, and so the node can be
 *  given a name while somebody is looking at it. */
let welcomeOpen = false;

/** Whether the panel's signing item was on screen when it was closed, so the
 *  "launch at login turns on with this" sentence in it is kept. */
let welcomeOfferedSigning = false;

function maybeShowWelcome(status: NodeStatusInfo) {
  if (welcomeOpen || status.welcome_shown || !status.setup_complete) return;
  welcomeOpen = true;
  // The signing item is for a host that can sign. `signer_applies_here` is
  // null until the first start decides; a panel that waited for it would miss
  // the moment, so null is taken as "probably" and the item is shown, which
  // is the honest default on every machine with an NVIDIA driver.
  welcomeOfferedSigning = status.signer_enabled && status.signer_applies_here !== false;
  $("welcome-signer-item").hidden = !welcomeOfferedSigning;
  const input = $<HTMLInputElement>("welcome-nickname");
  // An existing install reaching this panel through the contribution
  // migration may already be named. Showing an empty box would read as "your
  // node has no name", and leaving it empty on Done would look like it had
  // been cleared. Prefill so the name is visible and survives untouched.
  input.value = status.node_nickname ?? "";
  $("welcome-overlay").hidden = false;
  input.focus();
}

async function closeWelcome() {
  const input = $<HTMLInputElement>("welcome-nickname");
  const err = $("welcome-nickname-error");
  const btn = $<HTMLButtonElement>("welcome-done");
  btn.disabled = true;
  try {
    // The box is PREFILLED with the stored name, so it now reflects the true
    // current value and an empty box is a deliberate "remove my name" rather
    // than "nothing typed". Send whatever it holds. Skipping the empty case
    // was correct only while the box always started blank.
    await invoke<string>("set_node_nickname", { name: input.value });
  } catch (e) {
    // Same rule as the Settings field: the Rust side refuses anything btxd
    // would reject, so this is a sentence about what to type. Keep the panel
    // open so the name is not silently lost.
    err.textContent = String(e);
    err.hidden = false;
    btn.disabled = false;
    return;
  }
  // A signer is only useful while it is up. The panel said launch at login
  // turns on with signing, and this is where it does, once, best-effort: the
  // plugin is unavailable in dev and the switch in Settings remains the
  // operator's. Keep-awake is already on by default for the same reason.
  if (welcomeOfferedSigning) {
    try {
      await enable();
    } catch (e) {
      console.warn("could not turn launch-at-login on for the signer", e);
    }
  }
  // Mark it seen only after the name is settled, so a refusal above cannot
  // cost the user the panel.
  try {
    await invoke("mark_welcome_shown");
  } catch (e) {
    console.error("could not mark the welcome as shown", e);
  }
  $("welcome-overlay").hidden = true;
  btn.disabled = false;
}

/**
 * The signer row in Settings. Three things it must say: whether this host can
 * sign at all (a mirror host cannot, and a switch that pretends otherwise is
 * worse than none), what the public key is, and that what is being asked for
 * is trust. The trust sentence is static HTML under the key; this only fills
 * in the facts.
 */
function reflectSignerRow(status: NodeStatusInfo): void {
  const t = $<HTMLInputElement>("signer-toggle");
  if (document.activeElement !== t) t.checked = status.signer_enabled;
  const desc = $("signer-desc");
  if (status.signer_applies_here === false) {
    desc.textContent =
      "This machine follows other nodes' signatures rather than checking blocks itself, so it cannot sign. The setting is kept for a machine with a graphics card the engine accepts";
  } else if (status.signing_live) {
    desc.textContent =
      "Your node signs each block it checks, and the engine confirms it is signing now. The nodes that cannot check blocks themselves, the explorer's included, can follow yours";
  } else {
    desc.textContent =
      "Your node signs each block it checks, so the nodes that cannot check blocks themselves, the explorer's included, can follow yours. Applies on next start";
  }
  const row = $("signer-key-row");
  const key = status.signer_enabled ? status.signer_pubkey : null;
  row.hidden = !key;
  if (key) $("signer-pubkey").textContent = key;

  // The delivery row only exists while there is a key to deliver.
  const pubRow = $("signer-publish-row");
  pubRow.hidden = !key;
  const pubToggle = $<HTMLInputElement>("signer-publish-toggle");
  if (document.activeElement !== pubToggle) pubToggle.checked = status.signer_publish_enabled;
  const line = $("signer-publish-status");
  const offer = status.signer_offer;
  if (!status.signer_publish_enabled) {
    line.textContent =
      "Not being offered. Send the key above to a mirror operator yourself if you want it pinned.";
    line.hidden = false;
  } else if (offer) {
    // The backend's sentence, with the local time appended: a person wants to
    // know it happened and when, and the backend has no business formatting a
    // clock for them.
    const when = new Date(offer.at);
    const stamp = Number.isNaN(when.getTime()) ? "" : ` (${when.toLocaleTimeString()})`;
    line.textContent = `${offer.detail}${stamp}`;
    line.hidden = false;
  } else if (status.signing_live) {
    line.textContent = "Your key goes out with the next check-in, within fifteen minutes.";
    line.hidden = false;
  } else {
    line.hidden = true;
  }
  line.classList.toggle("needs-attention", !!offer && !offer.delivered);
}

$<HTMLInputElement>("signer-publish-toggle").addEventListener("change", async (e) => {
  const box = e.target as HTMLInputElement;
  const on = box.checked;
  const result = $("signer-publish-result");
  box.disabled = true;
  try {
    const msg = await invoke<string>("set_signer_publish", { on });
    result.classList.remove("is-error");
    result.textContent = msg;
  } catch (err) {
    box.checked = !on;
    result.classList.add("is-error");
    result.textContent = String(err);
  }
  result.hidden = false;
  box.disabled = false;
  void tick();
});

/** The status screen's offer to follow signatures: only on a machine that
 *  checks blocks itself, running, behind, and measured adding far fewer blocks
 *  an hour than the chain makes (catchup-trend.ts `cannotCatchUp`, which the
 *  cadence hold never trips). Nothing changes until the owner clicks. */
function reflectFollowOffer(status: NodeStatusInfo): void {
  const card = $("follow-card");
  const p = status.phase;
  const added =
    p.phase === "ready" &&
    !status.rc_stalled &&
    status.rc_validates_independently &&
    p.blocks_behind >= LAG_WORTH_SAYING
      ? cannotCatchUp(catchupSamples, Date.now())
      : null;
  if (added === null) {
    card.hidden = true;
    return;
  }
  card.hidden = false;
  const pace = added === 0 ? "has added no blocks lately" : `adds about ${added} block${added === 1 ? "" : "s"} an hour`;
  $("follow-msg").textContent =
    `This computer checks every block itself and ${pace}, fewer than the ${CHAIN_BLOCKS_PER_HOUR} ` +
    "the network makes, so it will not catch up this way. It can follow signatures instead, as a " +
    "Windows PC does: it then stops checking the proof of work itself and cannot sign. You can " +
    "switch back in Settings.";
}

function reflectFollowRow(status: NodeStatusInfo): void {
  // Only where it is a choice: a machine that checks blocks, or one whose
  // owner already chose. A machine that follows signatures anyway has none.
  $("follow-row").hidden = !(status.follow_signatures || status.rc_validates_independently);
  const t = $<HTMLInputElement>("follow-toggle");
  if (document.activeElement !== t) t.checked = status.follow_signatures;
}

async function setFollowSignatures(on: boolean): Promise<void> {
  await invoke("set_follow_signatures", { on });
  void tick();
}

// Two steps, like Remove: the first click says what happens, the second does
// it. The node restarts, so this is not a toggle to brush by accident.
let followArmTimer: ReturnType<typeof setTimeout> | undefined;
$("follow-btn").addEventListener("click", async () => {
  const btn = $<HTMLButtonElement>("follow-btn");
  if (btn.dataset.armed !== "1") {
    btn.dataset.armed = "1";
    btn.textContent = "Click again to restart and follow signatures";
    clearTimeout(followArmTimer);
    followArmTimer = setTimeout(() => {
      btn.dataset.armed = "";
      btn.textContent = "Follow signatures instead";
    }, 6000);
    return;
  }
  clearTimeout(followArmTimer);
  btn.dataset.armed = "";
  btn.disabled = true;
  btn.textContent = "Restarting…";
  try {
    await setFollowSignatures(true);
    $("follow-card").hidden = true;
    catchupSamples = [];
  } catch (err) {
    showToast(String(err));
  } finally {
    btn.disabled = false;
    btn.textContent = "Follow signatures instead";
  }
});

$<HTMLInputElement>("follow-toggle").addEventListener("change", async (e) => {
  const box = e.target as HTMLInputElement;
  const on = box.checked;
  const result = $("follow-result");
  box.disabled = true;
  try {
    await setFollowSignatures(on);
    result.classList.remove("is-error");
    result.textContent = on
      ? "Your node restarted and follows signatures now."
      : "Your node restarted and checks blocks itself again.";
    catchupSamples = [];
  } catch (err) {
    box.checked = !on;
    result.classList.add("is-error");
    result.textContent = String(err);
  }
  result.hidden = false;
  box.disabled = false;
});

$<HTMLInputElement>("signer-toggle").addEventListener("change", async (e) => {
  const box = e.target as HTMLInputElement;
  const on = box.checked;
  const result = $("signer-result");
  box.disabled = true;
  try {
    const msg = await invoke<string>("set_signer", { on });
    result.classList.remove("is-error");
    result.textContent = msg;
  } catch (err) {
    box.checked = !on;
    result.classList.add("is-error");
    result.textContent = String(err);
  }
  result.hidden = false;
  box.disabled = false;
  // The key row appears the moment the key exists, not at the next poll.
  void tick();
});

$("signer-copy-btn").addEventListener("click", async () => {
  const btn = $<HTMLButtonElement>("signer-copy-btn");
  const text = ($("signer-pubkey").textContent ?? "").trim();
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
    btn.textContent = "Copied";
  } catch {
    btn.textContent = "Couldn't copy";
  }
  setTimeout(() => (btn.textContent = "Copy key"), 1500);
});

/** The gap samples behind the "catching up" wording. Module scope because the
 *  poll loop calls renderStatus repeatedly and the trend needs history; the
 *  arithmetic itself is pure and tested in catchup-trend.test.ts. */
let catchupSamples: CatchupSample[] = [];

function renderStatus(status: NodeStatusInfo) {
  showScreen("status");
  const p = status.phase;
  const orb = $("status-orb");
  const badge = $("status-badge");
  const sub = $("status-sub");
  const errCard = $("status-error");

  reflectPeerNames(status);
  reflectFork(status);
  renderRole(status);
  reflectSignerRow(status);
  // The close dialog's warning follows the wire on every tick, so a dialog
  // opened an hour into a run says what the node is doing now.
  $("close-signer-warning").hidden = !status.signing_live;
  reflectEsploraRow(status);
  reflectWitnessRow(status);
  reflectSnapshotServeRow(status);
  reflectFollowRow(status);

  const mode = visualMode(p, status.rc_stalled);
  orb.className = `status-orb is-${mode}`;
  errCard.hidden = true;

  // Ambient line + compact orb mirror the main orb's state — one mapping.
  ambientMain.setMode(ambientMode(mode));
  ambientCompact.setMode(ambientMode(mode));
  $("compact-orb").className = `status-orb compact-orb is-${mode}`;
  // The power core (energy visual) reads the same signal: a running node is an
  // active, breathing core; a stopped/error node a dim gathering one.
  lastActive = mode === "ready" || mode === "syncing";
  core?.setActive(lastActive);

  // Record the gap on every poll while the node claims to be ready, so the
  // wording below can tell a closing gap from a pinned one. Cheap, bounded,
  // and the only place the sample is available.
  if (p.phase === "ready") {
    catchupSamples = pushSample(
      catchupSamples,
      { at: Date.now(), behind: p.blocks_behind, height: p.height },
      Date.now(),
    );
  } else if (p.phase !== "syncing") {
    // A stop, an error or a fresh start invalidates the history: a gap
    // measured before a restart says nothing about the one after it.
    catchupSamples = [];
  }

  reflectFollowOffer(status);

  let height = 0;
  switch (p.phase) {
    case "ready":
      if (status.rc_stalled) {
        // Running, connected, answering RPC — and NOT following the chain.
        // Say that plainly instead of the confident green LIVE.
        badge.textContent = "NOT FOLLOWING";
        sub.textContent = "Your node is running but cannot check new blocks on this machine";
      } else if (status.rc_may_fall_behind) {
        badge.textContent = "LIVE";
        sub.textContent = "Your node is helping the network, checking blocks on the processor";
      } else if (p.blocks_behind >= LAG_WORTH_SAYING) {
        // "Near tip" is a boolean with no lag term: it flips the moment the
        // snapshot chainstate loads at a height fixed in the release, so a
        // fresh install reads LIVE while still thousands of blocks short and
        // grinding. Still LIVE — it is running and connected — but say the gap
        // rather than let "helping the network" stand on its own.
        //
        // "Still catching up" is a claim about the FUTURE, and it is not always
        // true. A node more than three blocks behind is paced to one block per
        // block interval by the cadence burst hold (btxchain/btx#140), which is
        // the one speed at which a gap never closes: measured 2026-09-15, a Mac
        // held between 84 and 99 behind for hours while this line promised it
        // was catching up. That wording reads as slow internet and sends people
        // hunting peers, which is what happened on 2026-09-06 and 2026-09-13.
        // So say which of the two is happening, from the gap's own trend.
        //
        // And say how long: the line carries the node's own measured pace, a
        // time to go while the gap closes and the two rates side by side when
        // it does not (catchup-trend.ts, where the wording is tested).
        badge.textContent = "LIVE";
        sub.textContent = catchupLine(p.blocks_behind, catchupSamples, Date.now());
      } else {
        // "LIVE", not "READY": the node is running and serving the network now —
        // "ready" reads like it's waiting to do something.
        badge.textContent = "LIVE";
        sub.textContent = "Your node is live and helping the network";
      }
      height = p.height;
      $("stat-peers").textContent = fmtInt(p.peers);
      break;
    case "syncing":
      if (p.height === 0 && p.headers > 0) {
        // Headers phase (pre-sync/sync): the chain itself hasn't started —
        // show the header count doing the moving.
        badge.textContent = `FETCHING HEADERS ${(p.progress * 100).toFixed(0)}%`;
        sub.textContent = `Counting the chain — ${fmtInt(p.headers)} block headers so far`;
      } else {
        badge.textContent = `SYNCING ${(p.progress * 100).toFixed(1)}%`;
        sub.textContent = `Catching up — headers at ${fmtInt(p.headers)}`;
      }
      height = p.height;
      $("stat-peers").textContent = fmtInt(p.peers);
      break;
    case "starting":
    case "loading_snapshot":
      badge.textContent = p.phase === "starting" ? "STARTING" : "LOADING SNAPSHOT";
      sub.textContent = "This can take a moment";
      break;
    case "warming":
      badge.textContent = "GETTING READY";
      sub.textContent = `${p.message} — nothing is wrong, your node is checking its data. This can take a while after a hard shutdown.`;
      break;
    case "stopped":
      badge.textContent = "STOPPED";
      sub.textContent = "The node is not running";
      break;
    case "error":
      badge.textContent = "NEEDS ATTENTION";
      sub.textContent = "";
      errCard.hidden = false;
      $("status-error-msg").textContent = p.message;
      break;
    default:
      badge.textContent = "…";
      sub.textContent = "";
  }

  // A new block arrived — send one gentle note through the frequency line.
  if (height > lastPulseHeight && lastPulseHeight > 0) {
    ambientMain.pulse();
    ambientCompact.pulse();
  }
  if (height > 0) lastPulseHeight = height;

  $("stat-height").textContent = height > 0 ? fmtInt(height) : "—";
  // Only blank the peer count for phases that genuinely have no number.
  // "syncing" DOES carry one and it is the longest phase of a first run, so
  // blanking it there made a working node look unconnected for ~2 hours.
  if (p.phase !== "ready" && p.phase !== "syncing") {
    $("stat-peers").textContent = "—";
  }
  $("stat-uptime").textContent = fmtUptime(status.uptime_secs);
  $("stat-version").textContent = status.node_tag;

  $("disk-used").textContent = fmtGB(status.datadir_size_mb);
  $("disk-free").textContent = fmtGB(status.disk_free_mb);
  const warn = $("disk-warning");
  // Thresholds come from the backend (btx_core::disk canonical values), not
  // hardcoded here — one definition, no TS/Rust drift.
  if (status.disk_free_mb > 0 && status.disk_free_mb < status.disk_critical_mb) {
    warn.hidden = false;
    warn.classList.add("is-critical");
    warn.textContent =
      "Very low disk space — the node may stop. Free some space or use Reclaim in Settings.";
  } else if (status.disk_free_mb > 0 && status.disk_free_mb < status.disk_warn_mb) {
    warn.hidden = false;
    warn.classList.remove("is-critical");
    warn.textContent = "Disk space is getting low. The chain grows over time.";
  } else {
    warn.hidden = true;
  }

  renderContribution(status);
  renderValidation(status);

  const running = isNodeActive(p, status.rc_stalled);
  toggleNodeBtn.textContent = running ? "Stop node" : "Start node";
  toggleNodeBtn.classList.toggle("is-stop", running);
  toggleNodeBtn.disabled = p.phase === "starting";
}

// ── Poll loop ────────────────────────────────────────────────────────────────

let lastStatus: NodeStatusInfo | null = null;
/** One-shot latch so the wizard error card scrolls into view only when it first appears. */
let wizardErrorShown = false;

async function tick() {
  try {
    const status = await invoke<NodeStatusInfo>("get_node_status");
    lastStatus = status;
    // From the tick, not from renderStatus: a fresh install never reaches
    // renderStatus, and the first-run wizard is where a Windows user meets the
    // close dialog whose button used to read "Keep running in the menu bar".
    applyTrayTerm(status);
    // The serve row lives in the Settings overlay, which is reachable from any
    // screen and stays open across ticks; refresh it here so a verdict that
    // changes — or vanishes when the node stops — is reflected while it is
    // being looked at, not only on the next open.
    reflectArchiveService(status);
    // Same reason, same place: the "Last check" line is in Settings and is
    // rendered from what the backend persisted, so it is right at launch and
    // right within a tick of a check finishing, whoever started the check.
    reflectLastUpdateCheck(status);
    reflectWalletEnabled(status.wallet_enabled);
    if (status.setup_complete) setupDone = true;
    maybeShowWelcome(status);

    if (setupDone && !setupInFlight) {
      renderStatus(status);
    } else if (
      status.phase.phase === "welcome" ||
      setupInFlight ||
      status.phase.phase === "downloading" ||
      status.phase.phase === "preparing" ||
      (!setupDone &&
        (status.phase.phase === "starting" ||
          status.phase.phase === "warming" ||
          status.phase.phase === "loading_snapshot" ||
          status.phase.phase === "error"))
    ) {
      renderWizard(status);
    } else {
      renderStatus(status);
    }
  } catch (e) {
    console.error("status poll failed", e);
    // First poll failed (e.g. plain-browser dev without Tauri IPC): show the
    // wizard shell rather than a blank window.
    if (!lastStatus) showScreen("wizard");
  }
}

// ── Actions ──────────────────────────────────────────────────────────────────

async function beginSetup() {
  setupInFlight = true;
  wizardError.hidden = true;
  // Immediate feedback on the click: the button becomes a spinner + live label
  // and the "come back later" note appears, before any backend round-trip.
  setSetupButton(true, "Setting up your node…");
  $<HTMLButtonElement>("retry-btn").disabled = true;
  wizardProgress.hidden = false;
  setStep(0);
  // Bring the just-revealed progress card into view — on Windows the taller
  // text metrics used to push it entirely below the (formerly unscrollable)
  // fold, which read as "the button did nothing".
  wizardProgress.scrollIntoView({ behavior: "smooth", block: "nearest" });
  try {
    // Completion truth comes from the polled status.setup_complete — a
    // resolved invoke is NOT proof (the backend rejects a duplicate run with
    // an error, and older builds resolved it silently).
    await invoke("begin_setup");
  } catch (e) {
    $("wizard-error-msg").textContent = String(e);
    wizardError.hidden = false;
    wizardProgress.hidden = true;
    setSetupButton(false, "Set up my node"); // back to a clickable idle button
  } finally {
    setupInFlight = false;
    $<HTMLButtonElement>("retry-btn").disabled = false;
    void tick();
  }
}

setupBtn.addEventListener("click", () => void beginSetup());
$("retry-btn").addEventListener("click", () => void beginSetup());

toggleNodeBtn.addEventListener("click", async () => {
  const running = lastStatus
    ? isNodeActive(lastStatus.phase, lastStatus.rc_stalled)
    : false;
  toggleNodeBtn.disabled = true;
  try {
    await invoke(running ? "stop_node" : "start_node");
  } catch (e) {
    showToast(String(e));
  } finally {
    toggleNodeBtn.disabled = false;
    void tick();
  }
});

// ── Settings overlay ─────────────────────────────────────────────────────────

const overlay = $("settings-overlay");

$("settings-btn").addEventListener("click", async () => {
  overlay.hidden = false;
  $("reclaim-result").hidden = true;
  if (lastStatus) {
    $("setting-datadir").textContent = lastStatus.datadir;
    $<HTMLInputElement>("keepawake-toggle").checked = lastStatus.keep_awake;
    // Only macOS can actually hold the assertion. Rather than leave a switch
    // that is on and inert, say what the machine will really do. The row stays
    // visible because sleep is still the user's problem to solve — it just
    // stops claiming this app solves it.
    const awakeRow = $("keepawake-toggle").closest(".setting-row");
    if (awakeRow && !lastStatus.keep_awake_supported) {
        $<HTMLInputElement>("keepawake-toggle").disabled = true;
        $<HTMLInputElement>("keepawake-toggle").checked = false;
        const desc = awakeRow.querySelector(".setting-desc");
        if (desc) {
            desc.textContent =
                "Not available on this system — set your computer's own sleep settings to Never";
        }
    }
    $<HTMLInputElement>("serve-toggle").checked = lastStatus.attestation_serve_enabled;
    // What the node is ACTUALLY providing, next to the switch that claims to
    // control it. frontier.rs has computed this since #21 and the payload has
    // carried it since; nothing displayed it, so a node advertising the archive
    // bit while silently degraded to the live window looked completely fine.
    reflectArchiveService(lastStatus);
    reflectNickname(lastStatus);
    reflectSignerRow(lastStatus);
    $<HTMLInputElement>("report-toggle").checked = lastStatus.service_report_enabled;
    $<HTMLInputElement>("wallet-toggle").checked = lastStatus.wallet_enabled;
    reflectOnClose(lastStatus.on_close);
    reflectKeeperRow(lastStatus);
    reflectEsploraRow(lastStatus);
    reflectWitnessRow(lastStatus);
    reflectSnapshotServeRow(lastStatus);
  }
  // The preflight is a question about the machine, so it is asked when the
  // panel opens rather than on the status poll: it reads the conf, the node
  // and the disk, and none of that changes second to second.
  void refreshEsploraPreflight();
  try {
    $<HTMLInputElement>("autostart-toggle").checked = await isEnabled();
  } catch {
    /* plugin unavailable in dev — leave unchecked */
  }
});
$("settings-close").addEventListener("click", () => (overlay.hidden = true));
overlay.addEventListener("click", (e) => {
  if (e.target === overlay) overlay.hidden = true;
});

$("global-stats-link").addEventListener("click", () => {
  void invoke("open_global_stats").catch((e) => showToast(String(e)));
});

$("open-datadir-btn").addEventListener("click", () => {
  void invoke("open_data_folder").catch((e) => showToast(String(e)));
});

$<HTMLInputElement>("autostart-toggle").addEventListener("change", async (e) => {
  const on = (e.target as HTMLInputElement).checked;
  try {
    if (on) await enable();
    else await disable();
  } catch (err) {
    showToast(String(err));
    (e.target as HTMLInputElement).checked = !on;
  }
});

$<HTMLInputElement>("keepawake-toggle").addEventListener("change", (e) => {
  const on = (e.target as HTMLInputElement).checked;
  void invoke("set_keep_awake", { on }).catch((err) => showToast(String(err)));
});

$<HTMLInputElement>("keeper-toggle").addEventListener("change", (e) => {
  const on = (e.target as HTMLInputElement).checked;
  void invoke("set_node_profile", { profile: on ? "keeper" : "full" }).catch((err) =>
    showToast(String(err))
  );
  // The conf applies at the next start; keeperReflect explains the engine gate.
});

// Esplora mode: serve wallets from this node. ON runs the prune gate in Rust
// and shows its sentence — the switch springs back rather than staying on and
// inert. A missing electrs or caddy is refused the same way, naming the build
// script. The address row applies at the next start of the front.
let esploraToggleBusy = false;
$<HTMLInputElement>("esplora-toggle").addEventListener("change", async (e) => {
  const box = e.target as HTMLInputElement;
  const on = box.checked;
  const result = $("esplora-result");
  // Turning this on spawns two processes and waits to see they survived, so it
  // is seconds, not milliseconds. Without a guard a second click issues a
  // concurrent set_esplora and the two race over the same slot.
  if (esploraToggleBusy) {
    box.checked = !on;
    return;
  }
  esploraToggleBusy = true;
  box.disabled = true;
  result.classList.remove("is-error");
  result.textContent = on ? "Starting electrs and the front…" : "Stopping…";
  result.hidden = false;
  try {
    const msg = await invoke<string>("set_esplora", { on });
    result.classList.remove("is-error");
    result.textContent = msg;
  } catch (err) {
    box.checked = !on;
    result.classList.add("is-error");
    result.textContent = String(err); // the Rust side's sentence, not a stack trace
  }
  result.hidden = false;
  box.disabled = false;
  esploraToggleBusy = false;
  void refreshEsploraPreflight();
});

async function saveEsploraListen(): Promise<void> {
  const input = $<HTMLInputElement>("esplora-listen");
  const result = $("esplora-result");
  try {
    const msg = await invoke<string>("set_esplora_listen", { listen: input.value });
    result.classList.remove("is-error");
    result.textContent = msg;
  } catch (err) {
    result.classList.add("is-error");
    result.textContent = String(err);
  }
  result.hidden = false;
}
$("esplora-listen-save").addEventListener("click", () => void saveEsploraListen());
$("esplora-listen").addEventListener("keydown", (e) => {
  if ((e as KeyboardEvent).key === "Enter") void saveEsploraListen();
});

/**
 * Ask whether this machine can serve Esplora, and say so under the switch.
 *
 * Everything here was already computed and thrown away: the prune gate's
 * three-way refusal, the measured disk cost, and whether electrs and caddy are
 * installed at all. Rendering it before the switch is flipped is the whole
 * point of a preflight — the alternative is an operator learning the answer
 * from a refusal.
 */
async function refreshEsploraPreflight(): Promise<void> {
  const el = $("esplora-preflight");
  let p: EsploraPreflight;
  try {
    p = await invoke<EsploraPreflight>("esplora_preflight");
  } catch {
    el.hidden = true; // never a scary line for a failure the operator cannot act on
    return;
  }
  const lines: string[] = [];
  el.classList.toggle("is-error", !p.allowed);
  if (!p.allowed && p.blocker) {
    lines.push(p.blocker);
  } else {
    // Name what is missing and the script that builds it. Nothing is
    // downloaded by the app, so this is the operator's next command.
    const missing: string[] = [];
    if (!p.electrs_found) missing.push("electrs (deploy/esplora/build-electrs.sh)");
    if (!p.caddy_found) missing.push("caddy with the rate-limit plugin (deploy/esplora/build-caddy.sh)");
    if (missing.length) lines.push("Not installed yet: " + missing.join("; ") + ".");
    else lines.push(`Ready: electrs at ${p.electrs_found}, caddy at ${p.caddy_found}.`);
  }
  for (const w of p.warnings) lines.push(w);
  el.textContent = lines.join(" ");
  el.hidden = lines.length === 0;
}

// Answering block-hash questions is cheap and needs nothing installed, so the
// copy here is about what it does for the network rather than about disk. It
// also has to be honest that switching it on does not by itself make wallets
// use this node: a wallet only asks addresses in its own built-in list.
const WITNESS_STATIC_COPY =
  "Answer one question for wallets: which block is at a given height. It is how a wallet checks it is on the right chain, it costs almost nothing, and your node can already do it";

// The two limits the release note promises are on screen. They are properties
// of the feature, not of the bind address, so they belong in every state the
// switch is on in — including the loopback default, which is where a user
// actually lands. The first is the one that stops somebody believing they are
// now helping the network; the second is the promise this must never quietly
// grow out of.
const WITNESS_LIMITS =
  "A wallet uses a witness only when its address is in the wallet's own built-in list. This answers block hashes and refuses every other question.";

$<HTMLInputElement>("witness-toggle").addEventListener("change", async (e) => {
  const box = e.target as HTMLInputElement;
  const on = box.checked;
  const result = $("witness-result");
  box.disabled = true;
  try {
    const msg = await invoke<string>("set_witness", { on });
    result.classList.remove("is-error");
    result.textContent = msg;
  } catch (err) {
    box.checked = !on;
    result.classList.add("is-error");
    result.textContent = String(err);
  }
  result.hidden = false;
  box.disabled = false;
});

async function saveWitnessListen(): Promise<void> {
  const input = $<HTMLInputElement>("witness-listen");
  const result = $("witness-result");
  try {
    const msg = await invoke<string>("set_witness_listen", { listen: input.value });
    result.classList.remove("is-error");
    result.textContent = msg;
  } catch (err) {
    result.classList.add("is-error");
    result.textContent = String(err);
  }
  result.hidden = false;
}
$("witness-listen-save").addEventListener("click", () => void saveWitnessListen());
$("witness-listen").addEventListener("keydown", (e) => {
  if ((e as KeyboardEvent).key === "Enter") void saveWitnessListen();
});

/**
 * The witness row. Three things it must not do: claim to be helping when it is
 * only reachable from this machine, claim wallets are using it (they only ask
 * addresses in their own built-in list), and colour a stopped node as a
 * problem.
 */
function reflectWitnessRow(status: NodeStatusInfo): void {
  const t = $<HTMLInputElement>("witness-toggle");
  if (document.activeElement !== t) t.checked = status.witness_enabled;
  const input = $<HTMLInputElement>("witness-listen");
  if (document.activeElement !== input) input.value = status.witness_listen;

  const desc = $("witness-desc");
  desc.classList.remove("needs-attention");
  if (!status.witness_enabled) {
    desc.textContent = WITNESS_STATIC_COPY;
  } else if (status.witness_running) {
    // The address it is BOUND to, never the saved one. Saying "answering on
    // 127.0.0.1" because that is what was typed, while the live server is on
    // 0.0.0.0, tells somebody they closed a port they did not close.
    const on = status.witness_serving_on ?? status.witness_listen;
    const pending =
      status.witness_serving_on && status.witness_serving_on !== status.witness_listen
        ? ` ${status.witness_listen} applies at the next node start`
        : "";
    desc.textContent = status.witness_public
      ? `Answering on ${on}. Other machines can ask.${pending} ${WITNESS_LIMITS}`
      : `Answering on ${on}, this machine only. Set the address to 0.0.0.0 to let other machines ask.${pending} ${WITNESS_LIMITS}`;
  } else if (status.witness_message) {
    desc.textContent = status.witness_message;
    desc.classList.add("needs-attention");
  } else {
    desc.textContent = "Saved — it starts with the node";
  }

  const listenDesc = $("witness-listen-desc");
  listenDesc.classList.toggle("needs-attention", status.witness_enabled && status.witness_public);
  listenDesc.textContent = status.witness_public
    ? "Any machine that can reach this address can ask. Nothing about your wallet or your coins is exposed: it answers block hashes and refuses everything else"
    : "This machine only, unless you change it. Use 0.0.0.0 to let other machines ask. Applies at the next node start";
}

const SNAPSHOT_SERVE_STATIC_COPY =
  "Export the chain state at the tip, sign it, and let new nodes fetch it instead of catching up from a months-old file. Needs a node that checks blocks itself and signs. About 9 MB, refreshed every 500 blocks";

$<HTMLInputElement>("snapshot-serve-toggle").addEventListener("change", async (e) => {
  const box = e.target as HTMLInputElement;
  const on = box.checked;
  const result = $("snapshot-serve-result");
  box.disabled = true;
  try {
    const msg = await invoke<string>("set_snapshot_serve", { on });
    result.classList.remove("is-error");
    result.textContent = msg;
  } catch (err) {
    box.checked = !on;
    result.classList.add("is-error");
    result.textContent = String(err);
  }
  result.hidden = false;
  box.disabled = false;
});

/**
 * The snapshot row says what the keeper last said: off, saved and waiting
 * for the node, refused (the gate's sentence, in amber), a cycle's phase, or
 * what is on offer and how old it is. It never claims an offer is live on the
 * strength of the setting; `offering` comes from the node's own service bits.
 */
function reflectSnapshotServeRow(status: NodeStatusInfo): void {
  const t = $<HTMLInputElement>("snapshot-serve-toggle");
  if (document.activeElement !== t) t.checked = status.snapshot_serve_enabled;
  const desc = $("snapshot-serve-desc");
  desc.classList.remove("needs-attention");
  if (!status.snapshot_serve_enabled) {
    desc.textContent = SNAPSHOT_SERVE_STATIC_COPY;
  } else if (status.snapshot_serve) {
    desc.textContent = status.snapshot_serve.message;
    desc.classList.toggle("needs-attention", status.snapshot_serve.needs_attention);
  } else {
    desc.textContent = "Saved — it starts with the node";
  }
}

const ESPLORA_STATIC_COPY =
  "Serve the Esplora API to wallets from your own full node. Needs the whole chain on disk (never a pruned node), electrs and Caddy built from deploy/esplora, and disk for the index";

/**
 * The Esplora row says what is true: off, saved-but-waiting for the node,
 * refused (the gate's sentence, in amber), or serving with the guardian's
 * freshness verdict — amber for anything but `fresh`, because `unverified`
 * and `stale` are exactly the states a wallet operator should read.
 */
function reflectEsploraRow(status: NodeStatusInfo): void {
  const t = $<HTMLInputElement>("esplora-toggle");
  if (document.activeElement !== t) t.checked = status.esplora_enabled;
  const input = $<HTMLInputElement>("esplora-listen");
  if (document.activeElement !== input) input.value = status.esplora_listen;
  const desc = $("esplora-desc");
  desc.classList.remove("needs-attention");
  if (!status.esplora_enabled) {
    desc.textContent = ESPLORA_STATIC_COPY;
  } else if (status.esplora_indexing) {
    // Work in progress, NOT a problem: no amber. A first index on a full chain
    // runs for hours and the node serves the network the whole time.
    desc.textContent = status.esplora_message ?? "electrs is building its index.";
  } else if (status.esplora_running) {
    // The address the front is BOUND to, not the setting. Changing the setting
    // while the front is up applies at the next start, so naming the setting
    // here would tell someone to point a wallet at a port nothing is on.
    const where = status.esplora_serving_on ?? status.esplora_listen;
    const pending =
      status.esplora_serving_on && status.esplora_serving_on !== status.esplora_listen
        ? ` (${status.esplora_listen} applies at the next start)`
        : "";
    const fresh = status.esplora_freshness ?? "not judged yet";
    desc.textContent =
      `Serving on ${where}${pending} — freshness: ${fresh}` +
      (status.esplora_message ? ` (${status.esplora_message})` : "");
    if (status.esplora_freshness !== "fresh") desc.classList.add("needs-attention");
  } else if (status.esplora_message) {
    desc.textContent = status.esplora_message;
    desc.classList.add("needs-attention");
  } else if (status.running) {
    // On, the node is up, and nothing is serving — with no recorded reason.
    // "starts with the node" was shown here and is simply false: the node has
    // already started.
    desc.textContent =
      "On, but electrs and the front are not running. Stop and start the node to try again; the log is in the esplora folder inside your data folder.";
    desc.classList.add("needs-attention");
  } else {
    desc.textContent = "Saved — electrs and the front start with the node";
  }
}

/**
 * Reflect the profile choice + engine gate in Settings. The choice is stored
 * either way; on an engine that predates the pruned-serving fixes the row says
 * plainly that it activates with the next node engine update — a stored
 * promise, not a silent no-op.
 */
// Swap the platform's own word for the tray into every string that names it.
//
// The copy is macOS-native throughout - "menu bar" in the first-run pitch, the
// close dialog, the close-behaviour setting and a button label - and on Windows
// and Linux that is a place the user does not have, in a dialog asking them to
// choose it. The sentences stay in the markup and only the noun moves, so there
// is one copy of each string rather than one per platform.
//
// Runs once: the term cannot change while the app is open.
let trayTermApplied = false;
function applyTrayTerm(status: NodeStatusInfo): void {
  if (trayTermApplied || !status.tray_term || status.tray_term === "menu bar") {
    // "menu bar" is what the markup already says, so macOS needs no pass at all.
    trayTermApplied = true;
    return;
  }
  const title = status.tray_term.charAt(0).toUpperCase() + status.tray_term.slice(1);
  for (const el of document.querySelectorAll<HTMLElement>("[data-tray-term]")) {
    // textContent, and only these marked elements: this rewrites shipped copy,
    // so it must not be able to touch markup or an element nobody vetted.
    el.textContent = el.textContent!.replace(/Menu bar/g, title).replace(/menu bar/g, status.tray_term);
  }
  trayTermApplied = true;
}

function reflectNickname(status: NodeStatusInfo): void {
  const input = $<HTMLInputElement>("nickname-input");
  // Never clobber what somebody is in the middle of typing.
  if (document.activeElement !== input) input.value = status.node_nickname;

  const desc = $("nickname-desc");
  // Show the REAL user agent, not one derived from the setting. btxd builds it
  // once at init, so a nickname saved while the node is up is not live until
  // the next start — and the honest way to say that is to print what peers are
  // actually seeing right now.
  if (status.subversion) {
    desc.textContent = "Other nodes see you as ";
    const wire = document.createElement("span");
    wire.className = "nickname-wire";
    wire.textContent = status.subversion; // textContent: this came off the node
    desc.append(wire);
    if (status.node_nickname && !status.subversion.includes(`(${status.node_nickname})`)) {
      desc.append(" — your new name applies the next time the node starts");
    }
  } else {
    desc.textContent =
      "Optional. Every node you connect to sees this name. Leave empty to stay anonymous";
  }
}

/// Names of the peers we can see. The whole point of a nickname is that other
/// people have one too, so say how many are out there — including when the
/// answer is none, which is what it is on this network today.
function reflectPeerNames(status: NodeStatusInfo): void {
  const el = document.getElementById("peer-names");
  if (!el) return;
  const names = status.peer_nicknames;
  // Your own name belongs beside theirs — but from the WIRE, not the setting.
  // btxd builds its user agent once at init, so a name saved on a running node
  // is not broadcast until the next start, and a name cleared on a running
  // node is still being broadcast. broadcast_nickname is parsed from the real
  // subversion and is null whenever we do not know what is on the wire; in
  // that state saying nothing is the honest answer.
  const me = status.broadcast_nickname ?? "";
  if (names.length === 0 && !me) {
    el.textContent = "";
    el.hidden = true;
    return;
  }
  el.hidden = false;
  // textContent throughout: peer strings were chosen by strangers and arrived
  // over the wire. btx_core::nickname filters and caps them; this is the second
  // layer, and it is the one that makes markup impossible rather than unlikely.
  const parts: string[] = [];
  if (me) parts.push(`You are ${me}`);
  if (names.length > 0) parts.push(`connected to ${names.join(", ")}`);
  else if (me) parts.push("no other named nodes in sight yet");
  el.textContent = parts.join(" · ");
}

function reflectArchiveService(status: NodeStatusInfo): void {
  const row = $("serve-toggle").closest(".setting-row");
  const desc = row?.querySelector(".setting-desc");
  if (!desc) return;
  // Keep the markup's own sentence the first time through, so it can come
  // BACK. The previous version overwrote it and then "left it up" on a null
  // verdict — which left the last live verdict up instead, amber class and
  // all, on a node that had since stopped. A stopped node claiming to serve
  // history is the exact lie this row exists to prevent.
  const el = desc as HTMLElement;
  el.dataset.staticCopy ??= el.textContent ?? "";
  if (!status.archive_service_message) {
    el.textContent = el.dataset.staticCopy;
    el.classList.remove("needs-attention");
    return;
  }
  el.textContent = status.archive_service_message;
  el.classList.toggle("needs-attention", status.archive_service_needs_attention);
}

/**
 * A longer chain exists that this node cannot obtain blocks for. Shown in
 * amber beside the height and never guessed: the sentence is btx_core::fork's,
 * the facts are btxd's own getchaintips. Hidden the moment the verdict
 * clears, so a stale alarm never outlives the condition, and hidden on any
 * phase that is not running: a stopped node has no view of the chain to be
 * behind with.
 */
function reflectFork(status: NodeStatusInfo): void {
  const card = $("fork-card");
  const running = status.phase.phase === "ready" || status.phase.phase === "syncing";
  // A stale tip outranks a fork verdict. A fork says "there is a better chain
  // we cannot reach"; a stale tip says "the newest block we have is hours old
  // however healthy everything else reads", which is the condition every
  // peer-derived signal in this app is blind to by construction.
  const message = status.tip_stale_message ?? status.fork_message;
  if (!running || !message) {
    card.hidden = true;
    return;
  }
  card.hidden = false;
  $("fork-msg").textContent = message;
}

/**
 * "Your node's role": one line per fact about what this machine is actually
 * doing for the network, each with a sentence on whether that helps. The facts
 * and the sentences are btx_core::role's — this only lays them out. Hidden on
 * any phase that is not running, like the fork card: a stopped node fills no
 * role, and the last live answer must not stay up as if it did.
 */
function renderRole(status: NodeStatusInfo): void {
  const card = $("role-card");
  const running = status.phase.phase === "ready" || status.phase.phase === "syncing";
  if (!running || !status.role || status.role_lines.length === 0) {
    card.hidden = true;
    return;
  }
  card.hidden = false;
  const list = $("role-lines");
  // Elements and textContent, never innerHTML: the strings are ours today, and
  // the habit is what keeps that safe on the day one of them is not.
  list.textContent = "";
  for (const line of status.role_lines) {
    const li = document.createElement("li");
    const verdict =
      line.helps === true ? "is-helps" : line.helps === false ? "is-not" : "is-unknown";
    li.className = `role-line ${verdict}`;
    const row = document.createElement("div");
    row.className = "role-row";
    const mark = document.createElement("span");
    mark.className = "role-mark";
    // The mark is the only place the verdict is not spelled out in words, so
    // give a screen reader the words.
    mark.setAttribute("role", "img");
    mark.setAttribute(
      "aria-label",
      line.helps === true
        ? "helps the network"
        : line.helps === false
          ? "does not help the network"
          : "not known",
    );
    const label = document.createElement("span");
    label.className = "role-label";
    label.textContent = line.label;
    const value = document.createElement("span");
    value.className = "role-value";
    value.textContent = line.value;
    row.append(mark, label, value);
    const note = document.createElement("p");
    note.className = "role-note";
    note.textContent = line.note;
    li.append(row, note);
    list.append(li);
  }
}

function reflectKeeperRow(status: NodeStatusInfo) {
  const t = $<HTMLInputElement>("keeper-toggle");
  if (document.activeElement !== t) t.checked = status.node_profile === "keeper";
  const desc = $("keeper-desc");
  if (status.node_profile === "keeper" && !status.keeper_engine_ready) {
    desc.textContent =
      "Saved — Keeper mode switches on with the next node engine update (this engine cannot yet prune + serve safely)";
  } else if (status.node_profile === "keeper") {
    desc.textContent =
      "Small node (~10 GB) serving signed confirmations. Applies fully at the next node start";
  } else if (status.datadir_pruned) {
    // The switch is off, so the row would otherwise offer a small node as if
    // this were a full one. It is not: the folder deleted old blocks in an
    // earlier run and cannot be talked out of it — asking it to keep them all
    // is what used to stop the node starting. Say what it is, and say that the
    // thing this release is about still works, because it does.
    desc.textContent =
      "This folder already deleted old blocks in an earlier run, so it stays small whatever this switch says. Getting every block back means downloading the chain again. It can still help wallets check the chain: that needs the list of blocks, not the blocks";
  } else {
    desc.textContent =
      "Small node (~10 GB) that serves signed confirmations — the network's scarcest service";
  }
}

// Serving is independent of the profile: Keeper mode implies it, and a FULL
// node can flip it here too — a full-history node that serves is the most
// valuable archive the network has (there is currently ~one).
// Local file only — no network, no upload. It records what this node has
// served, plus the public nickname if one is set (which every peer can already
// see). The copy says so, because a node operator has every reason to ask
// before switching on anything that sounds like telemetry.
$<HTMLInputElement>("report-toggle").addEventListener("change", (e) => {
  const on = (e.target as HTMLInputElement).checked;
  void invoke("set_service_report", { on })
    .then(() =>
      showToast(
        on
          ? "Writing a local service report every few minutes"
          : "Service report off",
      ),
    )
    .catch(() => showToast("Could not change that setting"));
});

// Saving a nickname is a deliberate act with a Save button, not a live-as-you-
// type setting. Two reasons: it is written into the conf that starts btxd, and
// it is the one setting other people can see, so committing to it should be a
// decision rather than a side effect of tabbing away.
async function saveNickname(): Promise<void> {
  const input = $<HTMLInputElement>("nickname-input");
  const btn = $<HTMLButtonElement>("nickname-save");
  const result = $("nickname-result");
  btn.disabled = true;
  try {
    const stored = await invoke<string>("set_node_nickname", { name: input.value });
    input.value = stored;
    result.classList.remove("is-error");
    result.textContent = stored
      ? `Saved. Other nodes will see "${stored}" from the next time your node starts.`
      : "Nickname cleared. Your node is anonymous again from its next start.";
    result.hidden = false;
  } catch (e) {
    // The Rust side refuses rather than writes on anything btxd would reject,
    // so this is a sentence about what to type, not a stack trace.
    result.classList.add("is-error");
    result.textContent = String(e);
    result.hidden = false;
  } finally {
    btn.disabled = false;
  }
}

$("nickname-save").addEventListener("click", () => void saveNickname());
$("welcome-done").addEventListener("click", () => void closeWelcome());
$("nickname-input").addEventListener("keydown", (e) => {
  if ((e as KeyboardEvent).key === "Enter") void saveNickname();
});

$<HTMLInputElement>("serve-toggle").addEventListener("change", (e) => {
  const on = (e.target as HTMLInputElement).checked;
  void invoke("set_attestation_serve", { on })
    .then(() =>
      showToast(
        on
          ? "Serving on — applies the next time the node starts"
          : "Serving off — applies the next time the node starts",
      ),
    )
    .catch((err) => showToast(String(err)));
});

$<HTMLInputElement>("wallet-toggle").addEventListener("change", (e) => {
  const on = (e.target as HTMLInputElement).checked;
  reflectWalletEnabled(on); // instant; the next status poll confirms
  void invoke("set_wallet_enabled", { on }).catch((err) => showToast(String(err)));
});

// Remove node data: destructive, so a two-step confirm — first click arms,
// second click (within 6 s) fires. Frees the chain (~124 GiB for a full node,
// ~10 GiB for a keeper) and returns to
// the setup screen; wallets and the miner's files are never touched.
let removeArmTimer: ReturnType<typeof setTimeout> | undefined;
$("remove-node-btn").addEventListener("click", async () => {
  const btn = $<HTMLButtonElement>("remove-node-btn");
  if (btn.dataset.armed !== "1") {
    btn.dataset.armed = "1";
    const gb = lastStatus ? (lastStatus.datadir_size_mb / 1024).toFixed(0) : "?";
    btn.textContent = `Click again to remove ~${gb} GB`;
    clearTimeout(removeArmTimer);
    removeArmTimer = setTimeout(() => {
      btn.dataset.armed = "";
      btn.textContent = "Remove…";
    }, 6000);
    return;
  }
  clearTimeout(removeArmTimer);
  btn.dataset.armed = "";
  btn.disabled = true;
  btn.textContent = "Removing…";
  try {
    const report = await invoke<ReclaimReport>("remove_node_data_now");
    const out = $("remove-node-result");
    out.hidden = false;
    out.textContent = `Freed ${fmtGB(report.freed_mb)}. Your node is removed — set it up again anytime.`;
    setupDone = false; // back to the wizard on the next poll
    overlay.hidden = true;
  } catch (e) {
    showToast(String(e));
  } finally {
    btn.disabled = false;
    btn.textContent = "Remove…";
    void tick();
  }
});

$("reclaim-btn").addEventListener("click", async () => {
  const btn = $<HTMLButtonElement>("reclaim-btn");
  btn.disabled = true;
  btn.textContent = "Working…";
  try {
    const report = await invoke<ReclaimReport>("reclaim_disk_now");
    const out = $("reclaim-result");
    out.hidden = false;
    out.textContent =
      report.freed_mb > 0
        ? `Freed ${fmtGB(report.freed_mb)} (${report.items.join(", ")})`
        : "Nothing to reclaim right now.";
  } catch (e) {
    showToast(String(e));
  } finally {
    btn.disabled = false;
    btn.textContent = "Reclaim";
  }
});

// ── Compact mode: only the light ─────────────────────────────────────────────

async function setCompact(on: boolean) {
  document.body.classList.toggle("compact", on);
  $("compact-light").hidden = !on;
  // applyVisual owns which loop/core runs on the now-visible surface.
  applyVisual();
  try {
    // Lazy window handle: getCurrentWindow() THROWS outside a Tauri webview
    // (plain-browser dev/QA); resolving it at module scope would abort the
    // whole module and silently kill every listener wired after it.
    const appWindow = getCurrentWindow();
    if (on) {
      await appWindow.setDecorations(false);
      await appWindow.setSize(new LogicalSize(170, 170));
    } else {
      await appWindow.setDecorations(true);
      await appWindow.setSize(new LogicalSize(560, 780));
    }
  } catch (e) {
    showToast(String(e));
  }
}

$("compact-btn").addEventListener("click", () => void setCompact(true));
$("expand-btn").addEventListener("click", () => void setCompact(false));
$("compact-light").addEventListener("dblclick", () => void setCompact(false));

// ── Info + Future overlays ───────────────────────────────────────────────────

const infoOverlay = $("info-overlay");
const futureOverlay = $("future-overlay");

interface NodeFootprint {
  running: boolean;
  /** null when this platform can't measure per-process CPU cheaply (Windows). */
  cpu_pct: number | null;
  mem_mb: number;
  chain_mb: number;
}
let footprintTimer: ReturnType<typeof setInterval> | undefined;
async function refreshFootprint(): Promise<void> {
  try {
    const f = await invoke<NodeFootprint>("node_footprint");
    $("fp-cpu").textContent =
      f.running && f.cpu_pct !== null ? `${f.cpu_pct.toFixed(1)}% of one core` : "—";
    $("fp-mem").textContent = f.running && f.mem_mb > 0 ? `${fmtInt(f.mem_mb)} MB` : "—";
    $("fp-disk").textContent = f.chain_mb > 0 ? fmtGB(f.chain_mb) : "—";
    $("fp-note").textContent = f.running
      ? "Live numbers from this computer, refreshed while this panel is open."
      : "Start your node to see live numbers.";
  } catch {
    /* outside tauri — leave dashes */
  }
}
function stopFootprint(): void {
  if (footprintTimer !== undefined) clearInterval(footprintTimer);
  footprintTimer = undefined;
}
$("info-btn").addEventListener("click", () => {
  infoOverlay.hidden = false;
  void refreshFootprint();
  stopFootprint();
  footprintTimer = setInterval(() => void refreshFootprint(), 3000);
});
$("info-close").addEventListener("click", () => {
  infoOverlay.hidden = true;
  stopFootprint();
});
infoOverlay.addEventListener("click", (e) => {
  if (e.target === infoOverlay) {
    infoOverlay.hidden = true;
    stopFootprint();
  }
});
$("future-btn").addEventListener("click", () => {
  infoOverlay.hidden = true;
  // Every other way of leaving the info overlay stops the poll; this one did
  // not, so ps/tasklist kept being spawned every 3 s for the rest of the
  // session to update a panel nobody was looking at.
  stopFootprint();
  futureOverlay.hidden = false;
});
$("future-btn-settings").addEventListener("click", () => {
  overlay.hidden = true;
  futureOverlay.hidden = false;
});
$("future-close").addEventListener("click", () => (futureOverlay.hidden = true));
futureOverlay.addEventListener("click", (e) => {
  if (e.target === futureOverlay) futureOverlay.hidden = true;
});

// ── Accent picker ────────────────────────────────────────────────────────────

function applyAccent(name: string) {
  if (name === "ember") delete document.documentElement.dataset.accent;
  else document.documentElement.dataset.accent = name;
  localStorage.setItem(ACCENT_KEY, name);
  document.querySelectorAll<HTMLButtonElement>(".accent-dot").forEach((d) => {
    d.classList.toggle("is-active", d.dataset.accent === name);
  });
}

document.querySelectorAll<HTMLButtonElement>(".accent-dot").forEach((d) => {
  d.addEventListener("click", () => applyAccent(d.dataset.accent ?? "ember"));
});
applyAccent(localStorage.getItem(ACCENT_KEY) ?? "btx");

// ── Look: Calm line ⇄ Energy pulse ───────────────────────────────────────────
function applyVisualPref(v: Visual): void {
  visual = v;
  localStorage.setItem(VISUAL_KEY, v);
  // Scope to the visual seg — other .seg controls (e.g. on-close) share the class.
  document.querySelectorAll<HTMLButtonElement>(".seg-btn[data-visual]").forEach((b) => {
    b.classList.toggle("is-active", b.dataset.visual === v);
  });
  applyVisual();
  // ensureCore() already seeds the core's active state from lastActive, so the
  // switch reflects the current phase immediately — no re-render needed.
}
document.querySelectorAll<HTMLButtonElement>(".seg-btn[data-visual]").forEach((b) => {
  b.addEventListener("click", () => applyVisualPref((b.dataset.visual as Visual) ?? "calm"));
});
applyVisualPref(visual);

// On-close behavior seg (Ask / Menu bar / Quit) — persisted server-side.
function reflectOnClose(mode: string): void {
  document.querySelectorAll<HTMLButtonElement>("#onclose-seg .seg-btn").forEach((b) => {
    b.classList.toggle("is-active", b.dataset.onclose === mode);
  });
}
document.querySelectorAll<HTMLButtonElement>("#onclose-seg .seg-btn").forEach((b) => {
  b.addEventListener("click", () => {
    const mode = b.dataset.onclose ?? "ask";
    reflectOnClose(mode);
    void invoke("set_on_close", { mode }).catch((e) => showToast(String(e)));
  });
});

// ── Toast ────────────────────────────────────────────────────────────────────

let toastTimer: ReturnType<typeof setTimeout> | undefined;
function showToast(msg: string) {
  const t = $("toast");
  t.textContent = msg;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), 5000);
}

// ── Auto-update ──────────────────────────────────────────────────────────────
// Same policy as the miner (updater on, periodic recheck): a support node
// left running for months must keep itself current — flag-day btxd upgrades
// ship inside app updates (see start_node_inner's re-provision path). The
// relaunch does NOT interrupt the node: the new instance attaches to the
// running btxd over RPC.
//
// The install stays AUTOMATIC, but it is no longer silent: an accent-framed
// banner appears under the header the moment an update is found (ported from
// the miner's "UPDATE AVAILABLE" cue), and Settings has a "Check now" button
// so nobody has to wait for the 6-hour timer or a relaunch.
//
// Two of the three triggers run here: the check at launch (boot, below) and
// the button, both through updateCheck(). The six-hourly recheck does NOT: it
// is a tokio timer in src-tauri/src/update_timer.rs, which runs the same
// plugin check, records through the same update_log, and emits an
// "update-check" event that onUpdateCheckEvent paints through the same
// functions updateCheck() paints with. Why it moved, stated as the hypothesis
// it is, is in that file's comment; in one line: a setInterval in a hidden
// WebKitGTK window may never fire, and this app lives hidden.
let appVersion = "";

/** Show the banner as "<strong>{head}</strong> {tail}". DOM-built, never
 *  innerHTML — the version string originates in the (signed, but still
 *  remote) update feed, and remote data never becomes markup. */
function showUpdateBanner(head: string, tail: string): void {
  const el = $("update-banner-text");
  el.replaceChildren();
  const strong = document.createElement("strong");
  strong.textContent = head;
  el.append(strong, tail ? ` ${tail}` : "");
  $("update-banner").hidden = false;
}

function setUpdateResult(text: string): void {
  $("update-check-result").textContent = text;
}

// Failing to CHECK and failing to INSTALL are different events and must not
// share a catch. A failed check is usually just being offline, and there is
// nothing for the user to do about it. A failed INSTALL is permanent for that
// build — a Linux .deb cannot be replaced by the updater at all — and the old
// single catch swallowed it on the automatic path, leaving the banner reading
// "Update available: vX — downloading…" indefinitely, repainted identically at
// every launch and every six-hour tick. That is worse than silence: it is an
// aria-live region asserting that something is in progress which has already
// failed and will fail again.
const MANUAL_DOWNLOAD = "easybtx.com/node";

// The permanent "Last check" line under the Check-now button, rendered from
// what the backend persisted (see recordUpdateCheck) on every status tick.
// Until 2026-09-15 the automatic check painted nothing at all: this project's
// own signer box ran 0.6.21 for eight hours after the feed served 0.6.22, two
// six-hourly checks fell due in that window, and nothing on the machine could
// say whether they ran, failed, or found nothing. Rendering from the persisted
// value rather than from this session's memory is what makes the line true at
// launch, before any check has run, and after the relaunch an install causes.
function reflectLastUpdateCheck(status: NodeStatusInfo): void {
  paintLastUpdateCheck({
    at: status.last_update_check_at,
    outcome: status.last_update_check_outcome,
    detail: status.last_update_check_detail,
  });
}

/** The one place the "Last check" line is written, whether the record came
 *  from the status tick (persisted) or from the Rust timer's event (just
 *  recorded), so the two cannot render the same record differently. */
function paintLastUpdateCheck(last: LastUpdateCheck): void {
  const el = $("update-last-check");
  el.textContent = lastCheckLine(last, new Date(), MANUAL_DOWNLOAD);
  // The recorded detail (automatic or pressed, the error text) on hover; the
  // line itself stays in plain words.
  el.title = last.detail;
}

/**
 * What the screen shows while a check that found something runs its course:
 * the banner under the header and the sentence beside the button. Shared by
 * updateCheck() and by onUpdateCheckEvent, so a check the Rust timer ran looks
 * exactly like one that ran here. The two quiet outcomes, no-update and
 * check-failed, paint nothing on the automatic path on either side (a manual
 * press paints its own sentence in updateCheck); the "Last check" line is
 * where they show. `error` is the install error's text, used only by
 * install-failed.
 */
function paintUpdateProgress(outcome: string, version: string, error: string): void {
  switch (outcome) {
    case "found":
      showUpdateBanner(`Update available: v${version}`, "— downloading…");
      setUpdateResult(`Update available: v${version} — downloading…`);
      break;
    case "install-failed":
      // Always visible, manual or not, and never worded as a network problem:
      // the common cause is a package format this updater cannot replace.
      showUpdateBanner(`Update v${version} couldn't install`, `— download it from ${MANUAL_DOWNLOAD}`);
      setUpdateResult(
        `Automatic update failed — get v${version} from ${MANUAL_DOWNLOAD} (${error.slice(0, 80)})`
      );
      break;
    case "installed":
      showUpdateBanner(`v${version} ready`, "— restarting…");
      break;
    default:
      break;
  }
}

// A check the Rust timer ran has settled (src-tauri/src/update_timer.rs). The
// backend has already written the record; this paints what updateCheck()
// would have painted had the check run here, through the same two functions,
// and the "Last check" line from the record itself rather than waiting for
// the next status tick to read it back.
function onUpdateCheckEvent(ev: UpdateCheckEvent): void {
  paintUpdateProgress(ev.outcome, ev.version, installErrorFromDetail(ev.detail));
  paintLastUpdateCheck({ at: ev.at, outcome: ev.outcome, detail: ev.detail });
}

// Write down how this check ended: one line in <datadir>/update-check.log and
// the last outcome in the settings file, through the backend. Called at EVERY
// exit of updateCheck below, and update-check.test.ts reads this file to make
// sure that stays true. Fire-and-forget by contract: a failure to record is a
// console warning and changes nothing about the update itself. The settled
// promise is returned for the one caller that is about to end the process and
// wants the line on disk first.
function recordUpdateCheck(branch: UpdateCheckBranch, manual: boolean): Promise<void> {
  const rec = updateCheckRecord(branch, manual ? "manual" : "automatic");
  return invoke("record_update_check", { outcome: rec.outcome, detail: rec.detail }).then(
    () => undefined,
    (e) => console.warn("update-check: could not record the outcome", e),
  );
}

async function updateCheck(manual = false): Promise<void> {
  let update: Awaited<ReturnType<typeof checkForUpdate>>;
  try {
    update = await checkForUpdate();
  } catch (e) {
    // Quiet on the automatic path; a MANUAL check must never end in silence,
    // because that reads as a dead button. But WHAT it says matters: a release
    // that has no build for this platform is not a connectivity problem, and
    // saying it was sent Mac owners hunting a fault that did not exist on the
    // day 0.6.20 shipped Linux-only. classifyCheckFailure tells the two apart
    // by the plugin's own wording; update-check.test.ts pins both strings.
    // Quiet on screen is no longer quiet everywhere: the outcome is recorded
    // either way, with the classified reason.
    const failure = classifyCheckFailure(e);
    if (manual) {
      setUpdateResult(checkFailureMessage(failure, appVersion, MANUAL_DOWNLOAD));
    }
    void recordUpdateCheck({ branch: "check-failed", failure }, manual);
    return;
  }

  if (!update) {
    if (manual) {
      setUpdateResult(
        appVersion ? `You're on the latest version (v${appVersion}).` : "You're on the latest version."
      );
    }
    void recordUpdateCheck({ branch: "no-update", currentVersion: appVersion }, manual);
    return;
  }

  paintUpdateProgress("found", update.version, "");
  // Recorded before the download, so a check that found something and then
  // died mid-download still left the finding behind.
  void recordUpdateCheck({ branch: "found", version: update.version }, manual);

  try {
    await update.downloadAndInstall();
  } catch (e) {
    paintUpdateProgress("install-failed", update.version, String(e));
    void recordUpdateCheck({ branch: "install-failed", version: update.version, error: e }, manual);
    return;
  }

  paintUpdateProgress("installed", update.version, "");
  // Awaited, with a bound, unlike the others: relaunch() ends this process,
  // and a record still in the IPC queue when it does is a record that was
  // never written. Two seconds is a bound on a local file append that takes
  // microseconds; it exists so a wedged backend cannot hold the restart
  // hostage. The promise never rejects, so the flow cannot change here.
  await Promise.race([
    recordUpdateCheck({ branch: "installed", version: update.version }, manual),
    new Promise<void>((resolve) => setTimeout(resolve, 2000)),
  ]);
  try {
    await relaunch();
  } catch (e) {
    showUpdateBanner(`v${update.version} is installed`, "— restart the app to finish");
    setUpdateResult(`Installed. Restart to finish. (${String(e).slice(0, 80)})`);
    void recordUpdateCheck({ branch: "relaunch-failed", version: update.version, error: e }, manual);
  }
}

$<HTMLButtonElement>("update-check-btn").addEventListener("click", async () => {
  const btn = $<HTMLButtonElement>("update-check-btn");
  btn.disabled = true;
  setUpdateResult("Checking…");
  try {
    await updateCheck(true);
  } finally {
    btn.disabled = false;
  }
});

// ── Close dialog (red X → "ask each time") ───────────────────────────────────
// The Rust close handler prevents the close and emits "close-requested" when the
// on_close setting is "ask". We offer keep-running vs quit; the choice (and an
// optional "remember") goes back via close_choice. A separate "app-quitting"
// event (fired by any graceful-quit path, including Cmd+Q) shows the reassuring
// "stopping safely…" state so a hidden window still explains the brief wait.
function initCloseDialog(): void {
  const overlay = $("close-overlay");
  const ask = $("close-ask");
  const quitting = $("close-quitting");
  const remember = $<HTMLInputElement>("close-remember");

  const showAsk = () => {
    ask.hidden = false;
    quitting.hidden = true;
    remember.checked = false;
    overlay.hidden = false;
  };
  const showQuitting = () => {
    ask.hidden = true;
    quitting.hidden = false;
    overlay.hidden = false;
  };

  void listen("close-requested", showAsk);
  void listen("app-quitting", showQuitting);

  $("close-keep-btn").addEventListener("click", () => {
    overlay.hidden = true;
    void invoke("close_choice", { quit: false, remember: remember.checked }).catch((e) =>
      showToast(String(e))
    );
  });
  $("close-quit-btn").addEventListener("click", () => {
    showQuitting();
    void invoke("close_choice", { quit: true, remember: remember.checked }).catch((e) =>
      showToast(String(e))
    );
  });
}

// ── Boot ─────────────────────────────────────────────────────────────────────

void (async () => {
  // Wire the close dialog FIRST — before any await and before the other inits.
  // The red X (on_close="ask") emits close-requested the instant it's clicked;
  // if listen() hasn't been issued yet the event is dropped and the X looks
  // dead. Issuing it at the very top of boot (not after getVersion's IPC
  // round-trip) shrinks that window to nothing, and putting it before
  // initAsk/initWallet means a throw in those can't leave the X unwired.
  initCloseDialog();
  initAsk();
  initWallet();
  try {
    const v = await getVersion();
    appVersion = v;
    $("brand-version").textContent = `v${v}`;
    $("settings-footer").textContent = `BTX Node v${v} · by easyBTX`;
  } catch {
    /* dev without tauri */
  }
  await tick();
  setInterval(() => void tick(), 1500);
  void updateCheck();
  // The six-hourly recheck used to be a setInterval here. It is now the Rust
  // timer in src-tauri/src/update_timer.rs, so there is ONE periodic path and
  // it is not a JavaScript timer in a window that spends its life hidden;
  // this is how its results reach the screen. Registered after the launch
  // check is started, which is fine: the timer's first tick is two minutes
  // out, and an event that arrives before a listener exists is dropped by
  // Tauri, never queued to be painted twice.
  void listen<UpdateCheckEvent>("update-check", (e) => onUpdateCheckEvent(e.payload));
})();
