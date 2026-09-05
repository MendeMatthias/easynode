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

export interface NodeStatusInfo {
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
  /** What we are really providing: `state` is serving_history |
   *  degraded_to_live_window | not_serving | unknown. */
  archive_service: { state: string; blocks_behind?: number } | null;
  // The same verdict as a sentence, rendered in Rust so the copy lives in one
  // place. Null until the refresher has completed a tick.
  archive_service_message: string | null;
  archive_service_needs_attention: boolean;
  /** A longer chain this node cannot obtain blocks for; null when healthy or
   *  not yet measured. `kind` is longer_branch | headers_ahead. The sentence
   *  is rendered in Rust (fork_message), like archive_service_message. */
  fork: { kind: string; since_secs?: number } | null;
  fork_message: string | null;
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
  /** Whether the bundled engine can honour the keeper profile yet. */
  keeper_engine_ready: boolean;
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

function renderStatus(status: NodeStatusInfo) {
  showScreen("status");
  const p = status.phase;
  const orb = $("status-orb");
  const badge = $("status-badge");
  const sub = $("status-sub");
  const errCard = $("status-error");

  reflectPeerNames(status);
  reflectFork(status);

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
        badge.textContent = "LIVE";
        sub.textContent = `Your node is live, still catching up — ${fmtInt(p.blocks_behind)} blocks behind`;
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
    reflectWalletEnabled(status.wallet_enabled);
    if (status.setup_complete) setupDone = true;

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
    $<HTMLInputElement>("report-toggle").checked = lastStatus.service_report_enabled;
    $<HTMLInputElement>("wallet-toggle").checked = lastStatus.wallet_enabled;
    reflectOnClose(lastStatus.on_close);
    reflectKeeperRow(lastStatus);
  }
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
  if (!running || !status.fork_message) {
    card.hidden = true;
    return;
  }
  card.hidden = false;
  $("fork-msg").textContent = status.fork_message;
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

async function updateCheck(manual = false): Promise<void> {
  let update: Awaited<ReturnType<typeof checkForUpdate>>;
  try {
    update = await checkForUpdate();
  } catch (e) {
    // Quiet on the automatic path; a MANUAL check must never end in silence,
    // because that reads as a dead button.
    if (manual) setUpdateResult(`Couldn't check right now — are you online? (${String(e).slice(0, 80)})`);
    return;
  }

  if (!update) {
    if (manual) {
      setUpdateResult(
        appVersion ? `You're on the latest version (v${appVersion}).` : "You're on the latest version."
      );
    }
    return;
  }

  showUpdateBanner(`Update available: v${update.version}`, "— downloading…");
  setUpdateResult(`Update available: v${update.version} — downloading…`);

  try {
    await update.downloadAndInstall();
  } catch (e) {
    // Always visible, manual or not, and never worded as a network problem:
    // the common cause is a package format this updater cannot replace.
    showUpdateBanner(
      `Update v${update.version} couldn't install`,
      `— download it from ${MANUAL_DOWNLOAD}`
    );
    setUpdateResult(
      `Automatic update failed — get v${update.version} from ${MANUAL_DOWNLOAD} (${String(e).slice(0, 80)})`
    );
    return;
  }

  showUpdateBanner(`v${update.version} ready`, "— restarting…");
  try {
    await relaunch();
  } catch (e) {
    showUpdateBanner(`v${update.version} is installed`, "— restart the app to finish");
    setUpdateResult(`Installed. Restart to finish. (${String(e).slice(0, 80)})`);
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
  setInterval(() => void updateCheck(), 6 * 60 * 60 * 1000);
})();
