import { describe, expect, it } from "vitest";
import { type BeatEnv, type BeatKind, HEARTBEAT_MS, startHeartbeat } from "./heartbeat";

function fakeEnv() {
  const state = {
    hidden: false,
    intervals: [] as { ms: number; fn: () => void }[],
    frames: [] as (() => void)[],
    focus: [] as (() => void)[],
  };
  const env: BeatEnv = {
    hidden: () => state.hidden,
    every: (ms, fn) => {
      const entry = { ms, fn };
      state.intervals.push(entry);
      return () => {
        state.intervals = state.intervals.filter((i) => i !== entry);
      };
    },
    frame: (fn) => void state.frames.push(fn),
    onFocus: (fn) => void state.focus.push(fn),
  };
  const beats: [BeatKind, boolean][] = [];
  startHeartbeat((kind, hidden) => {
    beats.push([kind, hidden]);
  }, env);
  const runFrames = () => state.frames.splice(0).forEach((f) => f());
  return { state, beats, runFrames };
}

describe("startHeartbeat", () => {
  it("beats at once, then every five seconds", () => {
    const { state, beats } = fakeEnv();
    expect(beats).toEqual([["timer", false]]);
    expect(state.intervals.map((i) => i.ms)).toEqual([HEARTBEAT_MS]);
    expect(HEARTBEAT_MS).toBe(5_000);
  });

  it("sends a frame beat only from inside an animation frame", () => {
    const { beats, runFrames } = fakeEnv();
    expect(beats.filter(([k]) => k === "raf")).toHaveLength(0);
    runFrames();
    expect(beats).toEqual([
      ["timer", false],
      ["raf", false],
    ]);
  });

  // The measured freeze: a page given no frames keeps its timer beats and
  // loses its frame beats, and never piles up frame requests.
  it("keeps one frame request outstanding when frames stop", () => {
    const { state, beats } = fakeEnv();
    const tick = state.intervals[0].fn;
    tick();
    tick();
    tick();
    expect(beats.filter(([k]) => k === "timer")).toHaveLength(4);
    expect(beats.filter(([k]) => k === "raf")).toHaveLength(0);
    expect(state.frames).toHaveLength(1);
  });

  it("says whether the page is hidden", () => {
    const { state, beats } = fakeEnv();
    state.hidden = true;
    state.intervals[0].fn();
    expect(beats.at(-1)).toEqual(["timer", true]);
  });

  // EASYBTX_NODE_WINDOW_WATCHDOG=0: the app answers false and the page
  // stops beating, so the heartbeat's own cost can be measured.
  it("stops for good when the app answers false", async () => {
    const state = {
      intervals: [] as { ms: number; fn: () => void }[],
      frames: [] as (() => void)[],
      focus: [] as (() => void)[],
    };
    const env: BeatEnv = {
      hidden: () => false,
      every: (ms, fn) => {
        const entry = { ms, fn };
        state.intervals.push(entry);
        return () => {
          state.intervals = state.intervals.filter((i) => i !== entry);
        };
      },
      frame: (fn) => void state.frames.push(fn),
      onFocus: (fn) => void state.focus.push(fn),
    };
    let sent = 0;
    startHeartbeat(() => {
      sent++;
      return Promise.resolve(false);
    }, env);
    await Promise.resolve();
    await Promise.resolve();
    expect(state.intervals).toHaveLength(0);
    state.frames.splice(0).forEach((f) => f());
    state.focus[0]();
    expect(sent).toBe(1);
  });

  it("keeps beating while the app answers true", async () => {
    const { state, beats } = fakeEnv();
    await Promise.resolve();
    expect(state.intervals).toHaveLength(1);
    expect(beats).toHaveLength(1);
  });

  it("beats at once when the window gains focus", () => {
    const { state, beats } = fakeEnv();
    state.focus[0]();
    expect(beats.filter(([k]) => k === "timer")).toHaveLength(2);
  });
});
