import { describe, expect, it } from "vitest";
import {
  CHAIN_BLOCKS_PER_HOUR,
  HISTORY_WINDOW_MS,
  MAX_SAMPLES,
  PACE_MIN_SPAN_MS,
  SAMPLE_SPACING_MS,
  TREND_MIN_CLOSED,
  TREND_MIN_SPAN_MS,
  TREND_WINDOW_MS,
  type CatchupSample,
  catchupLine,
  catchupPace,
  catchupTrend,
  pushSample,
  timeToGo,
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
    const out = pushSample(old, { at: T0 + HISTORY_WINDOW_MS + 1000, behind: 88 }, T0 + HISTORY_WINDOW_MS + 1000);
    expect(out).toEqual([{ at: T0 + HISTORY_WINDOW_MS + 1000, behind: 88 }]);
  });

  it("keeps an hour of history at the real poll rate, bounded and spaced", () => {
    // The status poll runs every 1.5 s. Three hours of it is 7,200 pushes.
    let s: CatchupSample[] = [];
    const end = T0 + 3 * 60 * min(1);
    for (let at = T0; at <= end; at += 1500) s = pushSample(s, { at, behind: 90, height: 219_000 }, at);
    expect(s.length).toBeLessThanOrEqual(MAX_SAMPLES);
    // The newest always tracks the node...
    expect(s[s.length - 1].at).toBe(end);
    // ...the oldest reaches back most of the hour the pace reads...
    expect(end - s[0].at).toBeGreaterThan(HISTORY_WINDOW_MS - SAMPLE_SPACING_MS * 2);
    expect(end - s[0].at).toBeLessThanOrEqual(HISTORY_WINDOW_MS);
    // ...and every kept sample behind the newest is at least the spacing apart.
    for (let i = 1; i < s.length - 1; i++) expect(s[i].at - s[i - 1].at).toBeGreaterThanOrEqual(SAMPLE_SPACING_MS);
  });

  it("stays bounded no matter how long the app is open", () => {
    let s: { at: number; behind: number }[] = [];
    for (let i = 0; i < 1000; i++) s = pushSample(s, { at: T0 + i * 100, behind: 90 }, T0 + i * 100);
    expect(s.length).toBeLessThanOrEqual(240);
  });
});

// The node that started this: a fresh install from the 219,000 snapshot,
// 10,800 behind, reported 4 blocks in 30 minutes on 2026-09-26 and had to ask
// a person whether that would ever finish. The chain adds 20 in the same half
// hour, so the gap grew by 16.
const slowNode = (): CatchupSample[] => [
  { at: T0, behind: 10_796, height: 219_000 },
  { at: T0 + min(10), behind: 10_801, height: 219_001 },
  { at: T0 + min(20), behind: 10_807, height: 219_003 },
  { at: T0 + min(30), behind: 10_812, height: 219_004 },
];

describe("catchupPace", () => {
  it("reads blocks added and gap closed per hour off the two ends", () => {
    const pace = catchupPace(slowNode(), T0 + min(30));
    expect(pace).not.toBeNull();
    expect(pace!.addedPerHour).toBeCloseTo(8, 6);
    expect(pace!.closingPerHour).toBeCloseTo(-32, 6);
    expect(pace!.spanMs).toBe(min(30));
  });

  it("refuses to state a pace on less than the minimum history", () => {
    const s = [
      { at: T0, behind: 100, height: 1_000 },
      { at: T0 + PACE_MIN_SPAN_MS - 1000, behind: 50, height: 1_060 },
    ];
    expect(catchupPace(s, T0 + PACE_MIN_SPAN_MS - 1000)).toBeNull();
    expect(PACE_MIN_SPAN_MS).toBeGreaterThan(TREND_MIN_SPAN_MS);
  });

  it("needs a height at both ends, and ignores samples older than the history", () => {
    expect(catchupPace([{ at: T0, behind: 90 }, { at: T0 + min(30), behind: 80 }], T0 + min(30))).toBeNull();
    const s = [
      { at: T0, behind: 5_000, height: 100 },
      { at: T0 + HISTORY_WINDOW_MS + min(5), behind: 900, height: 219_000 },
      { at: T0 + HISTORY_WINDOW_MS + min(35), behind: 880, height: 219_060 },
    ];
    // The first sample is stale; what is left is 60 added and 20 closed in 30 minutes.
    const pace = catchupPace(s, T0 + HISTORY_WINDOW_MS + min(35));
    expect(pace!.addedPerHour).toBeCloseTo(120, 6);
    expect(pace!.closingPerHour).toBeCloseTo(40, 6);
  });
});

describe("timeToGo", () => {
  it("says nothing when the gap is not closing", () => {
    expect(timeToGo(10_000, 0)).toBeNull();
    expect(timeToGo(10_000, -32)).toBeNull();
    expect(timeToGo(0, 100)).toBeNull();
    expect(timeToGo(10_000, Number.NaN)).toBeNull();
  });

  it("uses hours below two days and days above, never a long hour count", () => {
    expect(timeToGo(100, 19_000)).toBe("less than an hour");
    expect(timeToGo(100, 100)).toBe("about 1 hour");
    expect(timeToGo(600, 100)).toBe("about 6 hours");
    // 47.6 hours rounds to 48, which is two days, not "about 48 hours".
    expect(timeToGo(4_760, 100)).toBe("about 2 days");
    // An NVIDIA machine from 219,000: 228 added an hour, 188 closed.
    expect(timeToGo(10_800, 188)).toBe("about 2 days");
    expect(timeToGo(10_800, 10)).toBe("about 45 days");
    expect(timeToGo(10_800, 5)).toBe("more than two months");
  });
});

describe("catchupLine", () => {
  it("tells the slow node it will not arrive, with both rates", () => {
    expect(catchupLine(10_812, slowNode(), T0 + min(30))).toBe(
      "Your node is live but not catching up: 10,812 blocks behind. " +
        `It adds about 8 blocks an hour while the network adds about ${CHAIN_BLOCKS_PER_HOUR}.`,
    );
  });

  it("names a node that has not moved at all", () => {
    const s = [
      { at: T0, behind: 400, height: 227_312 },
      { at: T0 + min(20), behind: 413, height: 227_312 },
    ];
    expect(catchupLine(413, s, T0 + min(20))).toBe(
      "Your node is live but not catching up: 413 blocks behind. It has added no blocks in the last 20 minutes.",
    );
  });

  it("explains the cadence hold with the two equal rates", () => {
    // 2026-09-15: a Mac held 84 to 99 behind for hours, adding one block per
    // block interval, the one pace at which a gap never closes.
    const s = [
      { at: T0, behind: 90, height: 227_000 },
      { at: T0 + min(8), behind: 92, height: 227_005 },
      { at: T0 + min(15), behind: 91, height: 227_010 },
    ];
    expect(catchupLine(91, s, T0 + min(15))).toBe(
      "Your node is live but not catching up: 91 blocks behind. It adds about 40 blocks an hour while the network adds about 40.",
    );
  });

  it("gives a closing node its time to go", () => {
    const s = [
      { at: T0, behind: 10_800, height: 219_000 },
      { at: T0 + min(10), behind: 10_769, height: 219_038 },
      { at: T0 + min(20), behind: 10_737, height: 219_076 },
    ];
    expect(catchupLine(10_737, s, T0 + min(20))).toBe(
      "Your node is live, still catching up — 10,737 blocks behind, about 2 days to go at this pace",
    );
  });

  it("tells a mirror that is racing to the tip it is nearly there", () => {
    // Measured before 0.6.30: a mirror from 219,000 reached the tip in 34 minutes.
    const s = [
      { at: T0, behind: 10_800, height: 219_000 },
      { at: T0 + min(16), behind: 5_600, height: 224_210 },
    ];
    expect(catchupLine(5_600, s, T0 + min(16))).toBe(
      "Your node is live, still catching up — 5,600 blocks behind, less than an hour to go at this pace",
    );
  });

  it("keeps the old wording until the pace has been measured", () => {
    const early = slowNode().filter((x) => x.at <= T0 + min(10));
    expect(catchupLine(10_801, early, T0 + min(10))).toBe(
      "Your node is live but not catching up. It is 10,801 blocks behind and the gap is not closing.",
    );
    const closing = [
      { at: T0, behind: 10_800, height: 219_000 },
      { at: T0 + min(5), behind: 10_784, height: 219_019 },
    ];
    expect(catchupLine(10_784, closing, T0 + min(5))).toBe(
      "Your node is live, still catching up — 10,784 blocks behind",
    );
    expect(catchupLine(95, [], T0)).toBe("Your node is live, still catching up — 95 blocks behind");
  });

  it("never shows a pace that contradicts 'not catching up'", () => {
    // Over the hour this node added 50 a hour, faster than the chain, but a
    // burst of blocks in the last ten minutes left the gap flat. "Adds 50
    // while the network adds 40" under "not catching up" would read as a bug.
    const s = [
      { at: T0, behind: 120, height: 227_000 },
      { at: T0 + min(50), behind: 100, height: 227_042 },
      { at: T0 + min(60), behind: 100, height: 227_050 },
    ];
    expect(catchupLine(100, s, T0 + min(60))).toBe(
      "Your node is live but not catching up. It is 100 blocks behind and the gap is not closing.",
    );
  });

  it("says one block, not one blocks", () => {
    const s = [
      { at: T0, behind: 500, height: 227_000 },
      { at: T0 + min(60), behind: 539, height: 227_001 },
    ];
    expect(catchupLine(539, s, T0 + min(60))).toContain("It adds about 1 block an hour while");
  });

  it("measures a pace through the real poll loop", () => {
    // pushSample at the app's 1.5 s poll for 30 minutes, the slow node's rates:
    // 8 added and 40 made an hour. The spacing must not starve the pace.
    let s: CatchupSample[] = [];
    let at = T0;
    for (; at <= T0 + min(30); at += 1500) {
      const hours = (at - T0) / 3_600_000;
      const height = 219_000 + Math.floor(8 * hours);
      const behind = 10_796 + Math.floor(40 * hours) - Math.floor(8 * hours);
      s = pushSample(s, { at, behind, height }, at);
    }
    const now = at - 1500;
    const pace = catchupPace(s, now);
    expect(pace!.addedPerHour).toBeGreaterThan(6);
    expect(pace!.addedPerHour).toBeLessThan(10);
    expect(catchupLine(10_812, s, now)).toContain("not catching up");
    expect(catchupLine(10_812, s, now)).toContain("It adds about 8 blocks an hour");
  });
});
