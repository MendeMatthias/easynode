// When the window's animations may run.
//
// The ambient line, the power core and the CSS loops (the orb's breathing,
// the live dot) kept drawing whether anyone looked or not: about a fifth of a
// core in WebKitWebProcess around the clock on the WSL box (2026-11-26),
// whose GPU signs blocks. They now run only while someone can see them:
//
// * not while the page is hidden (tray, minimised);
// * not when the window has been out of focus for a minute;
// * not when nobody has touched the window for five minutes;
// * never with prefers-reduced-motion, which gets still frames instead.
//
// Any input, a focus, or the page coming back starts them again at once.

export const UNFOCUSED_PAUSE_MS = 60_000;
export const IDLE_PAUSE_MS = 5 * 60_000;
export const GATE_CHECK_MS = 5_000;

export interface MotionInputs {
  hidden: boolean;
  focused: boolean;
  /** How long the window has been out of focus (0 while focused). */
  unfocusedForMs: number;
  /** How long since the last pointer, key or wheel input in the window. */
  idleForMs: number;
  reducedMotion: boolean;
}

export function motionAllowed(i: MotionInputs): boolean {
  if (i.hidden || i.reducedMotion) return false;
  if (!i.focused && i.unfocusedForMs >= UNFOCUSED_PAUSE_MS) return false;
  if (i.idleForMs >= IDLE_PAUSE_MS) return false;
  return true;
}

export interface GateEnv {
  now(): number;
  hidden(): boolean;
  focused(): boolean;
  reducedMotion(): boolean;
  onWindow(type: string, fn: () => void): void;
  onDocument(type: string, fn: () => void): void;
  every(ms: number, fn: () => void): void;
}

export function browserGateEnv(): GateEnv {
  const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)");
  return {
    now: () => performance.now(),
    hidden: () => document.hidden,
    focused: () => document.hasFocus(),
    reducedMotion: () => reduced?.matches ?? false,
    onWindow: (type, fn) => window.addEventListener(type, fn, { passive: true }),
    onDocument: (type, fn) => document.addEventListener(type, fn),
    every: (ms, fn) => void window.setInterval(fn, ms),
  };
}

const INPUT_EVENTS = ["pointermove", "pointerdown", "keydown", "wheel", "touchstart"];

export class MotionGate {
  allowed: boolean;
  private lastInput: number;
  private blurredAt: number | null;
  private readonly onChange: (allowed: boolean) => void;
  private readonly env: GateEnv;

  constructor(onChange: (allowed: boolean) => void, env: GateEnv = browserGateEnv()) {
    this.onChange = onChange;
    this.env = env;
    const t = env.now();
    this.lastInput = t;
    this.blurredAt = env.focused() ? null : t;
    for (const type of INPUT_EVENTS) env.onWindow(type, () => this.input());
    env.onWindow("focus", () => {
      this.blurredAt = null;
      this.input();
    });
    env.onWindow("blur", () => {
      this.blurredAt = this.env.now();
      this.evaluate();
    });
    env.onDocument("visibilitychange", () => {
      // Coming back from the tray counts as being looked at.
      if (!this.env.hidden()) this.lastInput = this.env.now();
      this.evaluate();
    });
    env.every(GATE_CHECK_MS, () => this.evaluate());
    this.allowed = motionAllowed(this.inputs());
  }

  inputs(): MotionInputs {
    const now = this.env.now();
    const focused = this.blurredAt === null;
    return {
      hidden: this.env.hidden(),
      focused,
      unfocusedForMs: focused ? 0 : now - (this.blurredAt ?? now),
      idleForMs: now - this.lastInput,
      reducedMotion: this.env.reducedMotion(),
    };
  }

  evaluate(): void {
    const next = motionAllowed(this.inputs());
    if (next === this.allowed) return;
    this.allowed = next;
    this.onChange(next);
  }

  private input(): void {
    this.lastInput = this.env.now();
    // pointermove fires often; only a paused gate has anything to do.
    if (!this.allowed) this.evaluate();
  }
}
