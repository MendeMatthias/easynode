// The window's heartbeat, for the watchdog in Rust (window_watchdog.rs).
//
// Every 5 s the page sends a "timer" beat, which says its script runs, and
// asks for one animation frame; the frame's callback sends a "raf" beat,
// which says the page is still being given frames. Both carry
// document.hidden, because a hidden or minimised page legitimately gets no
// frames and the watchdog must not take that for a freeze.
//
// Cheap on purpose: two IPC calls per 5 s, and at most one frame request
// outstanding, so a stalled frame never piles up callbacks.

export const HEARTBEAT_MS = 5_000;

export type BeatKind = "timer" | "raf";

export interface BeatEnv {
  hidden(): boolean;
  every(ms: number, fn: () => void): void;
  frame(fn: () => void): void;
  onFocus(fn: () => void): void;
}

export function browserBeatEnv(): BeatEnv {
  return {
    hidden: () => document.hidden,
    every: (ms, fn) => void window.setInterval(fn, ms),
    frame: (fn) => void window.requestAnimationFrame(() => fn()),
    onFocus: (fn) => window.addEventListener("focus", fn),
  };
}

/** Start beating. A focus beats at once, so a click on a frozen window is
 *  judged in seconds. */
export function startHeartbeat(
  send: (kind: BeatKind, hidden: boolean) => void,
  env: BeatEnv = browserBeatEnv(),
): void {
  let framePending = false;
  const tick = () => {
    send("timer", env.hidden());
    if (framePending) return;
    framePending = true;
    env.frame(() => {
      framePending = false;
      send("raf", env.hidden());
    });
  };
  tick();
  env.every(HEARTBEAT_MS, tick);
  env.onFocus(tick);
}
