import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { AmbientLine } from "./ambient";
import {
  type GateEnv,
  type MotionInputs,
  IDLE_PAUSE_MS,
  MotionGate,
  UNFOCUSED_PAUSE_MS,
  motionAllowed,
} from "./motion-gate";

const CSS = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

const seen: MotionInputs = {
  hidden: false,
  focused: true,
  unfocusedForMs: 0,
  idleForMs: 0,
  reducedMotion: false,
};

describe("motionAllowed", () => {
  it("animates a window someone is looking at", () => {
    expect(motionAllowed(seen)).toBe(true);
  });

  it("stops in a hidden or minimised window", () => {
    expect(motionAllowed({ ...seen, hidden: true })).toBe(false);
  });

  it("stops after a minute out of focus, not before", () => {
    const away = { ...seen, focused: false };
    expect(motionAllowed({ ...away, unfocusedForMs: UNFOCUSED_PAUSE_MS - 1 })).toBe(true);
    expect(motionAllowed({ ...away, unfocusedForMs: UNFOCUSED_PAUSE_MS })).toBe(false);
    expect(UNFOCUSED_PAUSE_MS).toBe(60_000);
  });

  it("stops after five minutes without input", () => {
    expect(motionAllowed({ ...seen, idleForMs: IDLE_PAUSE_MS - 1 })).toBe(true);
    expect(motionAllowed({ ...seen, idleForMs: IDLE_PAUSE_MS })).toBe(false);
    expect(IDLE_PAUSE_MS).toBe(300_000);
  });

  it("never animates with prefers-reduced-motion", () => {
    expect(motionAllowed({ ...seen, reducedMotion: true })).toBe(false);
  });
});

function fakeGate(opts: { focused?: boolean; reduced?: boolean } = {}) {
  const s = {
    now: 0,
    hidden: false,
    reduced: opts.reduced ?? false,
    win: new Map<string, (() => void)[]>(),
    doc: new Map<string, (() => void)[]>(),
    ticks: [] as (() => void)[],
  };
  const add = (m: Map<string, (() => void)[]>, t: string, fn: () => void) =>
    m.set(t, [...(m.get(t) ?? []), fn]);
  const env: GateEnv = {
    now: () => s.now,
    hidden: () => s.hidden,
    focused: () => opts.focused ?? true,
    reducedMotion: () => s.reduced,
    onWindow: (t, fn) => add(s.win, t, fn),
    onDocument: (t, fn) => add(s.doc, t, fn),
    every: (_ms, fn) => void s.ticks.push(fn),
  };
  const changes: boolean[] = [];
  const gate = new MotionGate((a) => changes.push(a), env);
  const fire = (m: Map<string, (() => void)[]>, t: string) => (m.get(t) ?? []).forEach((f) => f());
  return {
    s,
    gate,
    changes,
    win: (t: string) => fire(s.win, t),
    doc: (t: string) => fire(s.doc, t),
    tick: () => s.ticks.forEach((f) => f()),
  };
}

describe("MotionGate", () => {
  it("pauses a minute after the window loses focus and resumes on focus", () => {
    const g = fakeGate();
    expect(g.gate.allowed).toBe(true);
    g.win("blur");
    g.s.now = UNFOCUSED_PAUSE_MS - 1;
    g.tick();
    expect(g.gate.allowed).toBe(true);
    g.s.now = UNFOCUSED_PAUSE_MS;
    g.tick();
    expect(g.changes).toEqual([false]);
    g.win("focus");
    expect(g.changes).toEqual([false, true]);
  });

  it("pauses at once when the page is hidden and resumes when it is back", () => {
    const g = fakeGate();
    g.s.hidden = true;
    g.doc("visibilitychange");
    expect(g.changes).toEqual([false]);
    g.s.now = 10 * IDLE_PAUSE_MS;
    g.s.hidden = false;
    g.doc("visibilitychange");
    // Back from the tray counts as being looked at, however long it was away.
    expect(g.changes).toEqual([false, true]);
  });

  it("pauses an untouched window and resumes on the first input", () => {
    const g = fakeGate();
    g.s.now = IDLE_PAUSE_MS;
    g.tick();
    expect(g.gate.allowed).toBe(false);
    g.win("pointermove");
    expect(g.gate.allowed).toBe(true);
    expect(g.changes).toEqual([false, true]);
  });

  it("starts paused with prefers-reduced-motion and follows a change", () => {
    const g = fakeGate({ reduced: true });
    expect(g.gate.allowed).toBe(false);
    g.s.reduced = false;
    g.tick();
    expect(g.gate.allowed).toBe(true);
  });

  it("a window that opens without focus pauses after the minute", () => {
    const g = fakeGate({ focused: false });
    g.s.now = UNFOCUSED_PAUSE_MS;
    g.tick();
    expect(g.gate.allowed).toBe(false);
  });
});

// The ambient line itself: a paused line draws no frames and keeps a still
// one that follows the node's state.
describe("AmbientLine under the gate", () => {
  let frames: FrameRequestCallback[] = [];
  let strokes = 0;
  const g = globalThis as unknown as Record<string, unknown>;
  const saved: Record<string, unknown> = {};

  beforeEach(() => {
    frames = [];
    strokes = 0;
    for (const k of ["requestAnimationFrame", "cancelAnimationFrame", "window", "document", "getComputedStyle"]) {
      saved[k] = g[k];
    }
    g.requestAnimationFrame = (fn: FrameRequestCallback) => {
      frames.push(fn);
      return frames.length;
    };
    g.cancelAnimationFrame = () => {
      frames = [];
    };
    g.window = { devicePixelRatio: 1 };
    g.document = { documentElement: {} };
    g.getComputedStyle = () => ({ getPropertyValue: () => "" });
  });

  afterEach(() => {
    for (const [k, v] of Object.entries(saved)) g[k] = v;
  });

  function line(): AmbientLine {
    const noop = () => {};
    const ctx = {
      setTransform: noop,
      clearRect: noop,
      beginPath: noop,
      moveTo: noop,
      lineTo: noop,
      stroke: () => void strokes++,
    };
    const canvas = { clientWidth: 100, clientHeight: 40, width: 0, height: 0, getContext: () => ctx };
    return new AmbientLine(canvas as unknown as HTMLCanvasElement);
  }

  it("animates when started and allowed", () => {
    const a = line();
    a.start();
    expect(a.animating).toBe(true);
    expect(frames).toHaveLength(1);
  });

  it("stops requesting frames when the gate closes and resumes when it opens", () => {
    const a = line();
    a.start();
    a.setAllowed(false);
    expect(a.animating).toBe(false);
    expect(frames).toHaveLength(0);
    a.setAllowed(true);
    expect(a.animating).toBe(true);
    expect(frames).toHaveLength(1);
  });

  it("a paused line still shows the node's state with one still frame", () => {
    const a = line();
    a.start();
    a.setAllowed(false);
    const before = strokes;
    a.setMode("error");
    expect(strokes).toBeGreaterThan(before);
    expect(frames).toHaveLength(0);
  });

  it("starting while paused (reduced motion) draws a still frame, no loop", () => {
    const a = line();
    a.setAllowed(false);
    a.start();
    expect(a.animating).toBe(false);
    expect(strokes).toBeGreaterThan(0);
    expect(frames).toHaveLength(0);
  });

  it("the surface not on screen is not started by the gate opening", () => {
    const a = line();
    a.setAllowed(false);
    a.setAllowed(true);
    expect(a.animating).toBe(false);
  });
});

describe("styles", () => {
  it("hold every CSS loop while the gate is closed", () => {
    expect(CSS).toMatch(/body\.motion-paused \*,[\s\S]*?animation-play-state: paused !important;/);
  });
});
