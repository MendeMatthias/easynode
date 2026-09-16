import { describe, expect, it } from "vitest";
import {
  TREND_MIN_CLOSED,
  TREND_MIN_SPAN_MS,
  TREND_WINDOW_MS,
  catchupTrend,
  pushSample,
} from "./catchup-trend";

const T0 = 1_789_500_000_000;
const min = (n: number) => n * 60 * 1000;

describe("catchupTrend", () => {
  it("refuses to judge with no history", () => {
    expect(catchupTrend([], T0)).toBe("unknown");
    expect(catchupTrend([{ at: T0, behind: 90 }], T0)).toBe("unknown");
  });

  it("refuses to judge on a span shorter than the minimum", () => {
    const s = [
      { at: T0, behind: 98 },
      { at: T0 + min(3), behind: 90 },
    ];
    expect(catchupTrend(s, T0 + min(3))).toBe("unknown");
  });

  // THE CASE THIS EXISTS FOR. Real 2026-09-15 readings: the gap drifted
  // between 84 and 99 for hours while the card said "still catching up".
  it("calls the cadence-hold trap stalled, not catching up", () => {
    const s = [
      { at: T0, behind: 90 },
      { at: T0 + min(2), behind: 92 },
      { at: T0 + min(4), behind: 91 },
      { at: T0 + min(6), behind: 94 },
      { at: T0 + min(8), behind: 98 },
    ];
    expect(catchupTrend(s, T0 + min(8))).toBe("stalled");
  });

  it("does not read a gap closing by one or two as progress", () => {
    const s = [
      { at: T0, behind: 90 },
      { at: T0 + min(9), behind: 88 },
    ];
    expect(catchupTrend(s, T0 + min(9))).toBe("stalled");
    expect(TREND_MIN_CLOSED).toBeGreaterThan(2);
  });

  // The regression guard: a genuine catch-up must NOT be called stalled, the
  // same intent as node-observer.sh's "4853 behind and closing" stays calm.
  it("calls a real catch-up converging, however far behind", () => {
    const s = [
      { at: T0, behind: 4860 },
      { at: T0 + min(5), behind: 4700 },
      { at: T0 + min(9), behind: 4520 },
    ];
    expect(catchupTrend(s, T0 + min(9))).toBe("converging");
  });

  it("converges on exactly the threshold, not one short of it", () => {
    const span = TREND_MIN_SPAN_MS + 1000;
    const closed = (n: number) => catchupTrend(
      [{ at: T0, behind: 50 }, { at: T0 + span, behind: 50 - n }],
      T0 + span,
    );
    expect(closed(TREND_MIN_CLOSED)).toBe("converging");
    expect(closed(TREND_MIN_CLOSED - 1)).toBe("stalled");
  });

  it("ignores samples older than the window", () => {
    const s = [
      { at: T0, behind: 900 },
      { at: T0 + TREND_WINDOW_MS + min(2), behind: 95 },
      { at: T0 + TREND_WINDOW_MS + min(7), behind: 94 },
    ];
    // The 900 is stale; what is left is 95 -> 94, which is not progress.
    expect(catchupTrend(s, T0 + TREND_WINDOW_MS + min(7))).toBe("stalled");
  });

  it("tolerates unordered and non-finite input rather than throwing", () => {
    const s = [
      { at: T0 + min(9), behind: 80 },
      { at: T0, behind: 95 },
      { at: Number.NaN, behind: 5 },
      { at: T0 + min(4), behind: Number.NaN },
    ];
    expect(catchupTrend(s, T0 + min(9))).toBe("converging");
  });
});

describe("pushSample", () => {
  it("drops samples that fell out of the window", () => {
    const old = [{ at: T0, behind: 90 }];
    const out = pushSample(old, { at: T0 + TREND_WINDOW_MS + 1000, behind: 88 }, T0 + TREND_WINDOW_MS + 1000);
    expect(out).toEqual([{ at: T0 + TREND_WINDOW_MS + 1000, behind: 88 }]);
  });

  it("stays bounded no matter how long the app is open", () => {
    let s: { at: number; behind: number }[] = [];
    for (let i = 0; i < 1000; i++) s = pushSample(s, { at: T0 + i * 100, behind: 90 }, T0 + i * 100);
    expect(s.length).toBeLessThanOrEqual(240);
  });
});
