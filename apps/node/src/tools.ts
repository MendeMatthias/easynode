// Tools: quick actions, Copy diagnostics and the command window. The window
// sends only the typed line; Rust decides what runs. Everything the node says
// is set as text, never HTML.

import { invoke } from "@tauri-apps/api/core";
import {
  FAST_FORWARD_ARM_MS,
  FAST_FORWARD_POLL_MS,
  FF_NOTE,
  History,
  ReportCopy,
  RestartArm,
  capForDisplay,
  fastForwardRun,
  fastForwardView,
  type FastForwardCheck,
  type FastForwardStatus,
} from "./tools-history";

type ConsoleAnswer =
  | { kind: "output"; text: string }
  | { kind: "confirm"; token: string; sentence: string }
  | { kind: "refused"; sentence: string }
  | { kind: "stopped" }
  | { kind: "warming" };

/** A node on its way up is not a stopped one: said the same on every surface. */
const STILL_STARTING = "Your node is still starting. Try again in a moment.";

interface Notice {
  raw: string;
  message: string;
  needs_attention: boolean;
  hidden_because: string | null;
}
type Ask<T> =
  | { state: "ready"; data: T }
  | { state: "stopped" }
  | { state: "warming" }
  | { state: "unavailable"; data: { message: string } };

const $ = <T extends HTMLElement = HTMLElement>(id: string): T => document.getElementById(id) as T;

/** The status line exactly as the home screen shows it. */
function statusLine(): string {
  const badge = ($("status-badge").textContent ?? "").trim();
  const sub = ($("status-sub").textContent ?? "").trim();
  return [badge, sub].filter(Boolean).join(" · ");
}

/** The fork card's message, only while that card is actually shown. */
function windowLines(): string[] {
  const card = $("fork-card");
  if (card.hidden) return [];
  const text = ($("fork-msg").textContent ?? "").trim();
  return text ? [text] : [];
}

/** Call at the top of a click handler, before any await: WebKit allows the
 * clipboard write only inside the click itself. `label` is read when the
 * button goes back, so a label that changed meanwhile is the one shown. */
async function copy(text: string, btn: HTMLButtonElement, label: string | (() => string)): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    btn.textContent = "Copied";
  } catch {
    btn.textContent = "Couldn't copy";
  }
  setTimeout(() => (btn.textContent = typeof label === "function" ? label() : label), 1500);
}

function say(text: string): void {
  const r = $("tools-action-result");
  r.textContent = text;
  r.hidden = false;
}

/** Clears the action line, so a sentence from the last open (a refusal, a
 * fetch result) is not left under buttons it may no longer describe. */
function unsay(): void {
  const r = $("tools-action-result");
  r.textContent = "";
  r.hidden = true;
}

export function initTools(): void {
  const overlay = $("tools-overlay");
  const history = new History(50);
  const restartArm = new RestartArm();
  const reportCopy = new ReportCopy();
  /** Bumped when Tools closes, so a report still being built then is dropped. */
  let reportRun = 0;
  let pendingToken: string | null = null;
  let restartArmTimer: ReturnType<typeof setTimeout> | undefined;
  /** A restart this window started is still running. Rust refuses a second
   * one either way; this only keeps the button saying so across a close. */
  let restartInFlight = false;

  const ffArm = new RestartArm();
  let ffArmTimer: ReturnType<typeof setTimeout> | undefined;
  let ffPoll: ReturnType<typeof setInterval> | undefined;
  let ffOffer: Extract<FastForwardCheck, { kind: "offer" }> | null = null;
  let ffNote = FF_NOTE;
  /** Bumped when Tools closes, so a check, a run or a poll still in flight
   * then (mirrors reportRun) touches nothing once it lands. */
  let ffRun = 0;

  /** Back to one click away, as the section says. */
  const resetFfArm = () => {
    clearTimeout(ffArmTimer);
    ffArmTimer = undefined;
    ffArm.disarm();
    const note = $("tools-ff-note");
    note.textContent = ffNote;
    note.hidden = ffNote === "";
    if (ffOffer) $("tools-ff-btn").textContent = ffOffer.button;
  };
  const stopFfPoll = () => {
    clearInterval(ffPoll);
    ffPoll = undefined;
  };
  const showFfStatus = (status: FastForwardStatus | null) => {
    const result = $("tools-ff-result");
    if (status?.message) {
      result.textContent = status.message;
      result.hidden = false;
    }
    if (status?.running) $("tools-ff").hidden = false;
  };
  /** A fresh offer or dispute is about to replace what the section says;
   * the last run's leftover message no longer describes it. */
  const clearFfResult = () => {
    const result = $("tools-ff-result");
    result.textContent = "";
    result.hidden = true;
  };
  /** While a run is going: ask every few seconds, stop when it ends or the
   * overlay closes. A poll must not outlive a closed overlay. */
  const pollFf = () => {
    stopFfPoll();
    const run = ffRun;
    ffPoll = setInterval(async () => {
      if (overlay.hidden || run !== ffRun) {
        stopFfPoll();
        return;
      }
      const status = await invoke<FastForwardStatus>("tools_fast_forward_status").catch(() => null);
      if (overlay.hidden || run !== ffRun) {
        stopFfPoll(); // closed while that call was in flight
        return;
      }
      showFfStatus(status);
      if (status && !status.running) stopFfPoll();
    }, FAST_FORWARD_POLL_MS);
  };
  const refreshFastForward = async () => {
    const run = ffRun;
    const btn = $<HTMLButtonElement>("tools-ff-btn");
    const status = await invoke<FastForwardStatus>("tools_fast_forward_status").catch(() => null);
    if (run !== ffRun) return; // Tools closed meanwhile
    if (status?.running) {
      showFfStatus(status);
      btn.hidden = true;
      pollFf();
      return;
    }
    const check = await invoke<FastForwardCheck>("tools_fast_forward_check").catch(() => null);
    if (run !== ffRun) return; // Tools closed meanwhile
    ffOffer = check?.kind === "offer" ? check : null;
    // A fresh offer or dispute replaces whatever the last run said; only a
    // "none" check (nothing new to show) keeps that message in view, which
    // is what keeps the section open on its own between checks.
    if (check?.kind === "offer" || check?.kind === "off") clearFfResult();
    else showFfStatus(status);
    const view = fastForwardView(check, status);
    ffNote = view.note;
    btn.hidden = view.button === null;
    btn.disabled = false;
    resetFfArm();
    $("tools-ff").hidden = !view.section;
  };

  /** The 5s timer elapsed, or the overlay is closing: back to one click away. */
  const resetRestartArm = () => {
    clearTimeout(restartArmTimer);
    restartArmTimer = undefined;
    restartArm.disarm();
    if (!restartInFlight) $<HTMLButtonElement>("tools-restart").textContent = "Restart node";
  };

  /** Back to "Copy diagnostics": the next open builds a fresh report, and the
   * old one is not left on screen under a button that no longer copies it. */
  const resetReport = () => {
    reportRun += 1;
    reportCopy.reset();
    const btn = $<HTMLButtonElement>("tools-diag-btn");
    btn.textContent = reportCopy.label;
    btn.disabled = false;
    const pre = $("tools-diag");
    pre.textContent = "";
    pre.hidden = true;
  };

  /** The single close path for every way the overlay can close, so nothing
   * (an armed restart, a pending confirm, a built report, the last action's
   * sentence) survives to the next open. */
  const closeTools = () => {
    overlay.hidden = true;
    resetRestartArm();
    resetFfArm();
    stopFfPoll();
    ffRun += 1;
    pendingToken = null;
    $("tools-confirm").hidden = true;
    resetReport();
    unsay();
  };

  const open = async () => {
    overlay.hidden = false;
    $("tools-now").textContent = statusLine();
    const restart = $<HTMLButtonElement>("tools-restart");
    const why = await invoke<string | null>("tools_restart_check").catch(() => null);
    restart.disabled = restartInFlight || why !== null;
    restart.title = why ?? "";
    if (why !== null) say(why);
    void refreshFastForward();
  };
  $("tools-btn").addEventListener("click", () => void open());
  $("tools-close").addEventListener("click", () => closeTools());
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) closeTools();
  });

  // Restart node: two clicks, disarmed after five seconds or on close.
  $("tools-restart").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("tools-restart");
    if (!restartArm.click()) {
      btn.textContent = "Click again to restart the node";
      clearTimeout(restartArmTimer);
      restartArmTimer = setTimeout(resetRestartArm, 5000);
      return;
    }
    clearTimeout(restartArmTimer);
    restartArmTimer = undefined;
    btn.textContent = "Restarting...";
    btn.disabled = true;
    restartInFlight = true;
    try {
      await invoke("tools_restart_node");
      say("Your node restarted.");
    } catch (e) {
      say(String(e));
    } finally {
      restartInFlight = false;
      btn.textContent = "Restart node";
      btn.disabled = false;
      btn.title = "";
    }
  });

  // Fast-forward: the first click says what will happen and who confirmed
  // the snapshot, a second within ten seconds runs it (the Tools decision,
  // section 3).
  $("tools-ff-btn").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("tools-ff-btn");
    if (!ffOffer) return;
    if (!ffArm.click()) {
      $("tools-ff-note").textContent = ffOffer.confirm;
      btn.textContent = "Click again to fast-forward";
      clearTimeout(ffArmTimer);
      ffArmTimer = setTimeout(resetFfArm, FAST_FORWARD_ARM_MS);
      return;
    }
    clearTimeout(ffArmTimer);
    ffArmTimer = undefined;
    const run = ffRun;
    btn.disabled = true;
    btn.hidden = true;
    $("tools-ff-note").textContent = ffNote;
    const { message, started } = await fastForwardRun(() => invoke<string>("tools_fast_forward_run"));
    if (run !== ffRun) return; // Tools closed meanwhile
    const result = $("tools-ff-result");
    result.textContent = message;
    result.hidden = false;
    // Hiding the button left focus nowhere; the result line is the next
    // stable thing for a keyboard user to land on (tabindex="-1" in
    // index.html), as tools-fetch and buildReport refocus their own button.
    if (!overlay.hidden) result.focus();
    // A refusal is no run: a poll would put the last run's outcome over it.
    if (started) pollFf();
  });

  $("tools-fetch").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("tools-fetch");
    btn.disabled = true;
    say("Asking peers for the next blocks...");
    try {
      const out = await invoke<{ message: string }>("tools_fetch_stuck_blocks");
      say(out.message);
    } catch (e) {
      say(String(e));
    } finally {
      btn.disabled = false;
      if (!overlay.hidden) btn.focus();
    }
  });

  $("tools-open-folder").addEventListener("click", () => {
    void invoke("open_data_folder").catch((e) => say(String(e)));
  });

  $("tools-notices-btn").addEventListener("click", async () => {
    const box = $("tools-notices");
    const btn = $("tools-notices-btn");
    if (!box.hidden) {
      box.hidden = true;
      btn.setAttribute("aria-expanded", "false");
      return;
    }
    box.replaceChildren();
    const ans = await invoke<Ask<Notice[]>>("tools_engine_notices").catch(() => null);
    const add = (text: string, cls: string) => {
      const p = document.createElement("p");
      p.className = cls;
      p.textContent = text;
      box.appendChild(p);
    };
    if (!ans) {
      add("The node is not answering yet.", "tools-note");
    } else if (ans.state === "warming") {
      add(STILL_STARTING, "tools-note");
    } else if (ans.state === "stopped") {
      add("Start your node to see its notices.", "tools-note");
    } else if (ans.state === "unavailable") {
      add(ans.data.message, "tools-note");
    } else if (ans.data.length === 0) {
      add("The engine reports nothing right now.", "tools-note");
    } else {
      for (const n of ans.data) {
        add(n.message, n.needs_attention ? "tools-notice is-attention" : "tools-notice");
        if (n.hidden_because) add(`Not shown on the home screen: ${n.hidden_because}`, "tools-note");
        add(`Engine: ${n.raw}`, "tools-raw");
      }
    }
    box.hidden = false;
    btn.setAttribute("aria-expanded", "true");
  });

  // Copy diagnostics: the first click builds the report and shows it, the
  // second copies it. See ReportCopy for why it takes two.
  const buildReport = async (btn: HTMLButtonElement) => {
    const run = reportRun;
    btn.disabled = true;
    try {
      const text = await invoke<string>("tools_diagnostics", { statusLine: statusLine(), windowLines: windowLines() });
      if (run !== reportRun) return; // Tools closed meanwhile
      const pre = $("tools-diag");
      pre.textContent = text;
      pre.hidden = false;
      reportCopy.ready(text);
    } catch (e) {
      if (run === reportRun) say(String(e));
    } finally {
      if (run === reportRun) {
        btn.disabled = false;
        btn.textContent = reportCopy.label;
        // Disabling the button while it built the report blurred it; a
        // keyboard user's next Enter would otherwise hit nothing.
        if (!overlay.hidden) btn.focus();
      }
    }
  };
  $("tools-diag-btn").addEventListener("click", () => {
    const btn = $<HTMLButtonElement>("tools-diag-btn");
    const text = reportCopy.text;
    if (text !== null) void copy(text, btn, () => reportCopy.label);
    else void buildReport(btn);
  });

  // The command window.
  const input = $<HTMLInputElement>("tools-line");
  const runBtn = $<HTMLButtonElement>("tools-run");
  const list = $("tools-history");
  /** While a console call is in flight, so a second answer cannot land out
   * of order ahead of the first. */
  const setConsoleBusy = (busy: boolean) => {
    input.disabled = busy;
    runBtn.disabled = busy;
    // Disabling the input blurs it; a keyboard user's next keystroke should
    // land back in the input, not nowhere, once the overlay is still open.
    if (!busy && !overlay.hidden) input.focus();
  };
  const render = (line: string, answer: string) => {
    history.push({ line, answer });
    const item = document.createElement("div");
    item.className = "tools-entry";
    const head = document.createElement("div");
    head.className = "tools-entry-head";
    const cmd = document.createElement("code");
    cmd.textContent = `> ${line}`;
    const btn = document.createElement("button");
    btn.className = "link-row";
    btn.type = "button";
    btn.textContent = "Copy";
    btn.addEventListener("click", () => void copy(answer, btn, "Copy"));
    head.append(cmd, btn);
    const out = document.createElement("pre");
    out.className = "tools-pre";
    out.textContent = capForDisplay(answer);
    item.append(head, out);
    list.prepend(item);
    while (list.children.length > 50) list.lastElementChild?.remove();
  };
  const show = (line: string, a: ConsoleAnswer) => {
    switch (a.kind) {
      case "output":
        render(line, a.text);
        break;
      case "refused":
        render(line, a.sentence);
        break;
      case "stopped":
        render(line, "Start your node to run commands.");
        break;
      case "warming":
        render(line, STILL_STARTING);
        break;
      case "confirm":
        pendingToken = a.token;
        $("tools-confirm-text").textContent = a.sentence;
        $("tools-confirm").hidden = false;
        $("tools-confirm").dataset.line = line;
        break;
    }
  };
  const runLine = async () => {
    const line = input.value.trim();
    if (!line) return;
    input.value = "";
    $("tools-confirm").hidden = true;
    pendingToken = null;
    setConsoleBusy(true);
    try {
      const a = await invoke<ConsoleAnswer>("tools_console_run", { line }).catch(
        (e): ConsoleAnswer => ({ kind: "refused", sentence: String(e) }),
      );
      show(line, a);
    } finally {
      setConsoleBusy(false);
    }
  };
  runBtn.addEventListener("click", () => void runLine());
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !input.disabled) void runLine();
    if (e.key === "ArrowUp") {
      input.value = history.up();
      e.preventDefault();
    }
    if (e.key === "ArrowDown") {
      input.value = history.down();
      e.preventDefault();
    }
  });
  $("tools-confirm-yes").addEventListener("click", async () => {
    const token = pendingToken;
    const line = $("tools-confirm").dataset.line ?? "";
    pendingToken = null;
    $("tools-confirm").hidden = true;
    if (!token) return;
    setConsoleBusy(true);
    try {
      const a = await invoke<ConsoleAnswer>("tools_console_confirm", { token }).catch(
        (e): ConsoleAnswer => ({ kind: "refused", sentence: String(e) }),
      );
      show(line, a);
    } finally {
      setConsoleBusy(false);
    }
  });
  $("tools-confirm-no").addEventListener("click", () => {
    pendingToken = null;
    $("tools-confirm").hidden = true;
  });
  $("tools-copy-all").addEventListener("click", () => {
    void copy(history.allText(), $<HTMLButtonElement>("tools-copy-all"), "Copy all");
  });
}
