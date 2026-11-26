// The window's heartbeat, for the watchdog in Rust (window_watchdog.rs).
//
// Every 5 s the page sends a "timer" beat, which says its script runs, and
// asks for one animation frame; the frame's callback sends a "raf" beat,
// which says the page is still being given frames. Both carry
// document.hidden, because a hidden or minimised page legitimately gets no
// frames and the watchdog must not take that for a freeze.
//
// Cheap on purpose: a timer, not a requestAnimationFrame loop. One frame is
// requested per tick and nothing on the page changes for it, so it never
// makes the page paint; two IPC calls per 5 s, and at most one frame request
// outstanding, so a stalled frame never piles up callbacks. With the
// watchdog switched off the app answers false and the beating stops.

export const HEARTBEAT_MS = 5_000;

export type BeatKind = "timer" | "raf";

export interface BeatEnv {
  hidden(): boolean;
  /** Run `fn` every `ms`; returns how to stop it. */
  every(ms: number, fn: () => void): () => void;
  frame(fn: () => void): void;
  onFocus(fn: () => void): void;
}

export function browserBeatEnv(): BeatEnv {
  return {
    hidden: () => document.hidden,
    every: (ms, fn) => {
      const id = window.setInterval(fn, ms);
      return () => window.clearInterval(id);
    },
    frame: (fn) => void window.requestAnimationFrame(() => fn()),
    onFocus: (fn) => window.addEventListener("focus", fn),
  };
}

/** Start beating. A focus beats at once, so a click on a frozen window is
 *  judged in seconds. `send` may answer false (the watchdog is off), which
 *  stops the beating for good. */
export function startHeartbeat(
  send: (kind: BeatKind, hidden: boolean) => Promise<boolean | void> | void,
  env: BeatEnv = browserBeatEnv(),
): void {
  let framePending = false;
  let stopped = false;
  let cancel: () => void = () => {};
  const beat = (kind: BeatKind) => {
    const answer = send(kind, env.hidden());
    if (answer && typeof answer.then === "function") {
      answer.then((keep) => {
        if (keep === false && !stopped) {
          stopped = true;
          cancel();
        }
      }, () => {});
    }
  };
  const tick = () => {
    if (stopped) return;
    beat("timer");
    if (framePending) return;
    framePending = true;
    env.frame(() => {
      framePending = false;
      if (!stopped) beat("raf");
    });
  };
  tick();
  cancel = env.every(HEARTBEAT_MS, tick);
  env.onFocus(tick);
}
