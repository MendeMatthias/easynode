import { describe, expect, it } from "vitest";
import {
  CHAIN_BLOCKS_PER_HOUR,
  HEADERS_AHEAD_IS_BEHIND,
  HISTORY_WINDOW_MS,
  MAX_SAMPLES,
  PACE_MIN_SPAN_MS,
  SAMPLE_SPACING_MS,
  TREND_MIN_CLOSED,
  TREND_MIN_SPAN_MS,
  TREND_WINDOW_MS,
  type CatchupSample,
  type TrendPhase,
  type TrendReading,
  TOO_SLOW_FOR_THE_CHAIN_PER_HOUR,
  CHECKING_CATCHUP,
  cannotCatchUp,
  catchupLine,
  catchupPace,
  catchupTrend,
  chainCardMessage,
  paceSentence,
  pushSample,
  recordReading,
  staleCard,
  timeToGo,
  trendReading,
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
      "Your node is live, still catching up: 10,737 blocks behind, about 2 days to go at this pace",
    );
  });

  it("tells a mirror that is racing to the tip it is nearly there", () => {
    // Measured before 0.6.30: a mirror from 219,000 reached the tip in 34 minutes.
    const s = [
      { at: T0, behind: 10_800, height: 219_000 },
      { at: T0 + min(16), behind: 5_600, height: 224_210 },
    ];
    expect(catchupLine(5_600, s, T0 + min(16))).toBe(
      "Your node is live, still catching up: 5,600 blocks behind, less than an hour to go at this pace",
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
      "Your node is live, still catching up: 10,784 blocks behind",
    );
    expect(catchupLine(95, [], T0)).toBe("Your node is live, still catching up: 95 blocks behind");
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

describe("cannotCatchUp", () => {
  it("names the slow node's pace, the case the switch exists for", () => {
    expect(cannotCatchUp(slowNode(), T0 + min(30))).toBe(8);
  });

  it("names a node that has not moved as adding none", () => {
    const s = [
      { at: T0, behind: 400, height: 227_312 },
      { at: T0 + min(20), behind: 413, height: 227_312 },
    ];
    expect(cannotCatchUp(s, T0 + min(20))).toBe(0);
  });

  it("never offers the switch for the cadence hold, which it would not fix", () => {
    // 39 an hour against the chain's 40: a node held to the chain's pace.
    const s = [
      { at: T0, behind: 90, height: 227_000 },
      { at: T0 + min(10), behind: 90, height: 227_006 },
      { at: T0 + min(20), behind: 91, height: 227_013 },
    ];
    expect(cannotCatchUp(s, T0 + min(20))).toBeNull();
    expect(TOO_SLOW_FOR_THE_CHAIN_PER_HOUR).toBeLessThan(CHAIN_BLOCKS_PER_HOUR);
  });

  it("never offers it to a node that is catching up, or before the pace is measured", () => {
    const closing = [
      { at: T0, behind: 10_800, height: 219_000 },
      { at: T0 + min(10), behind: 10_769, height: 219_038 },
      { at: T0 + min(20), behind: 10_737, height: 219_076 },
    ];
    expect(cannotCatchUp(closing, T0 + min(20))).toBeNull();
    const early = slowNode().filter((x) => x.at <= T0 + min(10));
    expect(cannotCatchUp(early, T0 + min(10))).toBeNull();
    expect(cannotCatchUp([], T0)).toBeNull();
  });

  it("offers it on the M2 Pro's measured pace", () => {
    // 27 an hour checking blocks, from the 0.6.29 changelog.
    const s = [
      { at: T0, behind: 5_000, height: 220_000 },
      { at: T0 + min(10), behind: 5_002, height: 220_004 },
      { at: T0 + min(20), behind: 5_004, height: 220_009 },
    ];
    expect(cannotCatchUp(s, T0 + min(20))).toBe(27);
  });
});

// The chain card's stale sentence (docs/decisions/2026-09-29-quick-start-full-
// check-and-progress.md, section 3). btx2 and btx3 on 28 September read "The
// node is not following the chain" under "about 5 days to go".
const STALE =
  "The newest block this node has is 200 hours old. That is measured against the clock, not " +
  "against what its peers report, so it holds even when every peer agrees with it. The node is " +
  "not following the chain.";

// A node from 219,000 closing its gap at about 94 blocks an hour.
const closingNode = (): CatchupSample[] => [
  { at: T0, behind: 10_800, height: 219_000 },
  { at: T0 + min(10), behind: 10_769, height: 219_038 },
  { at: T0 + min(20), behind: 10_737, height: 219_076 },
];

// What the stale card is told about the phase: a live node this many blocks
// behind, a node syncing blocks, one fetching headers only, one not running.
const live = (behind: number): TrendReading =>
  trendReading({ phase: "ready", height: 219_076, blocks_behind: behind }, T0);
const syncingAt = (height: number, headers: number): TrendReading =>
  trendReading({ phase: "syncing", height, headers }, T0);
const HEADERS_ONLY = trendReading({ phase: "syncing", height: 0, headers: 180_000 }, T0);
const STOPPED = trendReading({ phase: "stopped" }, T0);

/** Feed the phases through the poll's own path, one reading per entry. */
const feed = (readings: [number, TrendPhase][]): CatchupSample[] =>
  readings.reduce<CatchupSample[]>((s, [at, phase]) => recordReading(s, trendReading(phase, at), at), []);

describe("trendReading", () => {
  it("records a live node's gap, as the catch-up line has always read it", () => {
    expect(trendReading({ phase: "ready", height: 226_000, blocks_behind: 7_400 }, T0)).toEqual({
      kind: "live",
      behind: 7_400,
      sample: { at: T0, behind: 7_400, height: 226_000 },
    });
  });

  it("records a syncing node too, marked, with its headers beyond its blocks as the gap", () => {
    expect(trendReading({ phase: "syncing", height: 130_000, headers: 226_000 }, T0)).toEqual({
      kind: "syncing",
      behind: 96_000,
      sample: { at: T0, behind: 96_000, height: 130_000, syncing: true },
    });
  });

  it("records nothing while only headers are fetched, and keeps what there is", () => {
    // Headers grow while the height stays 0: as a gap that reads as stalled.
    expect(HEADERS_ONLY).toEqual({ kind: "headers" });
    expect(trendReading({ phase: "syncing", height: 0, headers: 0 }, T0)).toEqual({ kind: "headers" });
    const kept = closingNode();
    expect(recordReading(kept, HEADERS_ONLY, T0 + min(21))).toBe(kept);
  });

  it("starts over on a stop, an error or a fresh start", () => {
    for (const phase of ["stopped", "error", "starting", "loading_snapshot", "warming"] as const) {
      expect(trendReading({ phase }, T0)).toEqual({ kind: "stopped" });
    }
    expect(recordReading(closingNode(), STOPPED, T0 + min(21))).toEqual([]);
  });

  it("pushes a reading's sample like any other", () => {
    const at = T0 + min(21);
    const reading = trendReading({ phase: "syncing", height: 130_000, headers: 226_000 }, at);
    expect(recordReading([], reading, at)).toEqual([{ at, behind: 96_000, height: 130_000, syncing: true }]);
  });
});

describe("staleCard", () => {
  it("says nothing when the tip is not stale", () => {
    expect(staleCard(null, live(10_812), slowNode(), T0 + min(30))).toBeNull();
  });

  it("row 1: behind and closing, no stale sentence at all", () => {
    expect(staleCard(STALE, live(10_737), closingNode(), T0 + min(20))).toBeNull();
  });

  it("row 2: behind before the trend is measured, neutral and not amber", () => {
    expect(staleCard(STALE, live(7_500), [], T0)).toEqual({ message: CHECKING_CATCHUP, tone: "neutral" });
    const firstMinutes = [
      { at: T0, behind: 7_500, height: 225_927 },
      { at: T0 + min(3), behind: 7_420, height: 226_010 },
    ];
    expect(staleCard(STALE, live(7_420), firstMinutes, T0 + min(3))).toEqual({
      message: "Checking whether your node is catching up...",
      tone: "neutral",
    });
  });

  it("row 3: behind and not closing, amber with the pace", () => {
    expect(staleCard(STALE, live(10_812), slowNode(), T0 + min(30))).toEqual({
      message: `${STALE} It adds about 8 blocks an hour while the network adds about ${CHAIN_BLOCKS_PER_HOUR}.`,
      tone: "amber",
    });
  });

  it("row 3 before the pace is measured: amber with today's sentence alone", () => {
    // The trend reads stalled from four minutes on; the pace needs fifteen,
    // and a height at both ends. Until then the card says only what it knows.
    const noHeights = [
      { at: T0, behind: 90 },
      { at: T0 + min(8), behind: 92 },
    ];
    expect(staleCard(STALE, live(92), noHeights, T0 + min(8))).toEqual({ message: STALE, tone: "amber" });
    const eightMinutes = [
      { at: T0, behind: 400, height: 227_312 },
      { at: T0 + min(8), behind: 405, height: 227_312 },
    ];
    expect(catchupPace(eightMinutes, T0 + min(8))).toBeNull();
    expect(staleCard(STALE, live(405), eightMinutes, T0 + min(8))).toEqual({ message: STALE, tone: "amber" });
  });

  it("row 4: no newer block known and the newest is old, today's sentence unchanged", () => {
    expect(staleCard(STALE, live(0), [], T0)).toEqual({ message: STALE, tone: "amber" });
    // A closing history does not soften it: with nothing newer known, an old
    // tip is the real "not following the chain".
    expect(staleCard(STALE, live(0), closingNode(), T0 + min(20))).toEqual({ message: STALE, tone: "amber" });
    expect(staleCard(STALE, STOPPED, [], T0)).toEqual({ message: STALE, tone: "amber" });
  });

  it("calls one header ahead a block in flight, as the role card does", () => {
    // role.rs HEADERS_AHEAD_IS_BEHIND: a body follows its header by seconds,
    // so one header ahead is at the tip, and with the newest block two hours
    // old that is today's sentence, on a live node and a syncing one alike.
    expect(HEADERS_AHEAD_IS_BEHIND).toBe(2);
    expect(staleCard(STALE, live(1), [], T0)).toEqual({ message: STALE, tone: "amber" });
    expect(staleCard(STALE, live(1), closingNode(), T0 + min(20))).toEqual({ message: STALE, tone: "amber" });
    expect(staleCard(STALE, syncingAt(130_000, 130_001), [], T0)).toEqual({ message: STALE, tone: "amber" });
    // Two ahead is behind, on both cards.
    expect(staleCard(STALE, live(2), [], T0)).toEqual({ message: CHECKING_CATCHUP, tone: "neutral" });
    expect(staleCard(STALE, syncingAt(130_000, 130_002), [], T0)).toEqual({
      message: CHECKING_CATCHUP,
      tone: "neutral",
    });
  });

  it("turns amber when a closing gap stops closing", () => {
    const s = closingNode();
    expect(staleCard(STALE, live(10_737), s, T0 + min(20))).toBeNull();
    s.push({ at: T0 + min(26), behind: 10_738, height: 219_081 });
    s.push({ at: T0 + min(31), behind: 10_739, height: 219_086 });
    // The last ten minutes closed nothing. Over the hour it still added more
    // than the chain, so the pace is left out rather than contradict the card.
    expect(staleCard(STALE, live(10_739), s, T0 + min(31))).toEqual({ message: STALE, tone: "amber" });
  });

  it("starts over after a restart", () => {
    expect(staleCard(STALE, live(10_812), slowNode(), T0 + min(30))?.tone).toBe("amber");
    // The window clears its samples on a stop or a start; the first reading
    // after it cannot judge anything yet.
    const after = pushSample([], { at: T0 + min(33), behind: 10_815, height: 219_005 }, T0 + min(33));
    expect(staleCard(STALE, live(10_815), after, T0 + min(33))).toEqual({
      message: CHECKING_CATCHUP,
      tone: "neutral",
    });
  });
});

// A syncing node's headers can run far ahead of its blocks on a sync from far
// back, and keep growing while they do, so its gap can widen while it adds
// blocks quickly. The card judges it by the blocks it adds: amber only below
// the chain's 40 an hour, the pace logic a live node already uses.
describe("staleCard on a syncing node", () => {
  it("does not turn amber while blocks arrive quickly, however fast the headers grow", () => {
    // 1,500 blocks in 20 minutes while the headers grew by 6,000.
    const s = feed(
      [0, 5, 10, 15, 20].map((m): [number, TrendPhase] => [
        T0 + min(m),
        { phase: "syncing", height: 120_000 + 75 * m, headers: 150_000 + 300 * m },
      ]),
    );
    // As a gap it widened by 4,500, which a live node's rule calls stalled.
    expect(catchupTrend(s, T0 + min(20))).toBe("stalled");
    expect(staleCard(STALE, syncingAt(121_500, 156_000), s, T0 + min(20))).toBeNull();
  });

  it("stays neutral until the blocks it adds are measured", () => {
    const s = feed([
      [T0, { phase: "syncing", height: 130_000, headers: 226_000 }],
      [T0 + min(10), { phase: "syncing", height: 130_000, headers: 226_000 }],
    ]);
    expect(staleCard(STALE, syncingAt(130_000, 226_000), s, T0 + min(10))).toEqual({
      message: CHECKING_CATCHUP,
      tone: "neutral",
    });
  });

  it("turns amber when its height stops, once the measuring window has passed", () => {
    const s = feed(
      [0, 5, 10, 15, 20].map((m): [number, TrendPhase] => [
        T0 + min(m),
        { phase: "syncing", height: 130_000, headers: 226_000 },
      ]),
    );
    expect(staleCard(STALE, syncingAt(130_000, 226_000), s, T0 + min(20))).toEqual({
      message: `${STALE} It has added no blocks in the last 20 minutes.`,
      tone: "amber",
    });
  });

  it("turns amber on a node adding fewer blocks than the chain, with both rates", () => {
    // 12 an hour, a slow machine checking blocks, still syncing.
    const s = feed(
      [0, 10, 20, 30].map((m, i): [number, TrendPhase] => [
        T0 + min(m),
        { phase: "syncing", height: 130_000 + [0, 2, 4, 6][i], headers: 226_000 + [0, 7, 13, 20][i] },
      ]),
    );
    expect(staleCard(STALE, syncingAt(130_006, 226_020), s, T0 + min(30))).toEqual({
      message: `${STALE} It adds about 12 blocks an hour while the network adds about ${CHAIN_BLOCKS_PER_HOUR}.`,
      tone: "amber",
    });
  });

  /** A syncing node adding one block every `secondsPerBlock`, the first
   *  `offsetS` seconds into a block, polled every 1.5 s through the window's
   *  own path and carrying the last card as main.ts does. The card at every
   *  poll. */
  const syncingCards = (secondsPerBlock: number, toMin: number, offsetS = 0) => {
    let s: CatchupSample[] = [];
    let wasAmber = false;
    const cards: { m: number; tone: string; message?: string }[] = [];
    for (let at = T0; at <= T0 + min(toMin); at += 1500) {
      const height = 130_000 + Math.floor((at - T0 + offsetS * 1000) / (secondsPerBlock * 1000));
      const reading = trendReading({ phase: "syncing", height, headers: 226_000 }, at);
      s = recordReading(s, reading, at);
      const card = staleCard(STALE, reading, s, at, wasAmber);
      wasAmber = card?.tone === "amber";
      cards.push({ m: (at - T0) / 60_000, tone: card?.tone ?? "hidden", message: card?.message });
    }
    return cards;
  };
  const tonesFrom = (cards: { m: number; tone: string }[], fromMin: number) =>
    new Set(cards.filter((c) => c.m >= fromMin).map((c) => c.tone));

  it("does not blink on a node adding blocks at about the chain's own rate", () => {
    // 89.2 s a block is 40.4 an hour. Read off an hour or less, one block in
    // or out of the window moves the pace by a block's worth, and a plain
    // "below 40" flipped the card on most blocks: 81 times in an hour at
    // 89.2 s, 80 at 90 s, and through the first hour on the hour's own read.
    for (const secondsPerBlock of [85, 89.2, 90]) {
      for (const offsetS of [0, 30, 60]) {
        const cards = syncingCards(secondsPerBlock, 150, offsetS);
        expect(tonesFrom(cards.filter((c) => c.m < 15), 0)).toEqual(new Set(["neutral"]));
        expect(tonesFrom(cards, 15)).toEqual(new Set(["hidden"]));
      }
    }
  });

  it("turns a node a little slower than the chain amber, and keeps it amber", () => {
    // 95 s a block is 37.9 an hour: it never catches up.
    for (const offsetS of [0, 30, 60]) {
      const cards = syncingCards(95, 150, offsetS);
      const first = cards.findIndex((c) => c.tone === "amber");
      expect(first).toBeGreaterThan(-1);
      expect(cards[first].m).toBeLessThan(60);
      expect(tonesFrom(cards.slice(first), 0)).toEqual(new Set(["amber"]));
      expect(cards[cards.length - 1].message).toBe(
        `${STALE} It adds about 38 blocks an hour while the network adds about ${CHAIN_BLOCKS_PER_HOUR}.`,
      );
    }
  });

  it("lets the card go once the hour's pace is back at the chain's or above", () => {
    // 95 s a block for an hour, then a block a minute: amber, then hidden
    // for good once the hour reads 40 or more.
    let s: CatchupSample[] = [];
    let wasAmber = false;
    const tones: { m: number; tone: string }[] = [];
    for (let at = T0; at <= T0 + min(180); at += 1500) {
      const m = (at - T0) / 60_000;
      const height = 130_000 + (m <= 60 ? Math.floor((m * 60) / 95) : Math.floor(3_600 / 95) + Math.floor(m - 60));
      const reading = trendReading({ phase: "syncing", height, headers: 226_000 }, at);
      s = recordReading(s, reading, at);
      const card = staleCard(STALE, reading, s, at, wasAmber);
      wasAmber = card?.tone === "amber";
      tones.push({ m, tone: card?.tone ?? "hidden" });
    }
    expect(tones.find((t) => t.m >= 59 && t.m <= 60)?.tone).toBe("amber");
    const back = tones.findIndex((t) => t.m > 60 && t.tone === "hidden");
    expect(back).toBeGreaterThan(-1);
    expect(tones[back].m).toBeLessThan(120);
    expect(new Set(tones.slice(back).map((t) => t.tone))).toEqual(new Set(["hidden"]));
  });

  it("turns amber about twenty minutes after a fast sync stops, not an hour later", () => {
    // Scripted through the real poll: 1,000 blocks an hour for an hour, then
    // stuck. Averaged over the hour, the pace stays above the chain's 40
    // until minute 117; read off the last twenty minutes, the stall shows
    // about twenty minutes after it starts.
    let s: CatchupSample[] = [];
    let firstAmber: number | null = null;
    for (let at = T0; at <= T0 + min(150) && firstAmber === null; at += 1500) {
      const m = (at - T0) / 60_000;
      const height = 120_000 + Math.floor((Math.min(m, 60) * 1_000) / 60);
      const reading = trendReading({ phase: "syncing", height, headers: 226_000 }, at);
      s = recordReading(s, reading, at);
      if (staleCard(STALE, reading, s, at)?.tone === "amber") firstAmber = m;
    }
    expect(firstAmber).not.toBeNull();
    expect(firstAmber!).toBeGreaterThan(60 + 15);
    expect(firstAmber!).toBeLessThanOrEqual(60 + 21);
  });

  it("never turns the header fetch amber", () => {
    // Every fresh install: the tip is the genesis block, days old by the
    // clock, while headers are counted and the height stays 0.
    const s = feed(
      [0, 10, 20, 40].map((m): [number, TrendPhase] => [
        T0 + min(m),
        { phase: "syncing", height: 0, headers: 1_000 * m },
      ]),
    );
    expect(s).toEqual([]);
    expect(staleCard(STALE, HEADERS_ONLY, s, T0 + min(40))).toEqual({ message: CHECKING_CATCHUP, tone: "neutral" });
    const noHeadersYet = trendReading({ phase: "syncing", height: 0, headers: 0 }, T0);
    expect(staleCard(STALE, noHeadersYet, [], T0)).toEqual({ message: CHECKING_CATCHUP, tone: "neutral" });
  });

  it("leaves a live node's judgement as it was", () => {
    // Readings taken while syncing sit at both ends of a live node's history.
    // A run of lost getchainstates can read as syncing: the refresher then
    // falls back to getblockchaininfo's `blocks`, the active chain's height.
    // Whatever their numbers (far off here on purpose, so a leak would show),
    // they are not part of its gap.
    const syncing = (at: number): CatchupSample => ({ at, behind: 98_612, height: 131_200, syncing: true });
    const mixed = [syncing(T0 - min(5)), ...slowNode(), syncing(T0 + min(31))];
    const now = T0 + min(31);
    expect(staleCard(STALE, live(10_812), mixed, now)).toEqual({
      message: `${STALE} It adds about 8 blocks an hour while the network adds about ${CHAIN_BLOCKS_PER_HOUR}.`,
      tone: "amber",
    });
    expect(staleCard(STALE, live(10_812), mixed, now)).toEqual(staleCard(STALE, live(10_812), slowNode(), now));
    expect(catchupLine(10_812, mixed, now)).toBe(catchupLine(10_812, slowNode(), now));
    expect(cannotCatchUp(mixed, now)).toBe(8);
  });

  it("starts a live node's trend afresh when it leaves syncing", () => {
    // Synced 1,500 blocks from genesis, then the snapshot loaded at 219,000:
    // read as one gap, that is 217,000 blocks closed in minutes.
    const s = feed([
      [T0, { phase: "syncing", height: 1_500, headers: 233_000 }],
      [T0 + min(10), { phase: "syncing", height: 1_600, headers: 233_007 }],
      [T0 + min(12), { phase: "ready", height: 219_000, blocks_behind: 14_010 }],
      [T0 + min(14), { phase: "ready", height: 219_010, blocks_behind: 14_001 }],
    ]);
    expect(staleCard(STALE, live(14_001), s, T0 + min(14))).toEqual({ message: CHECKING_CATCHUP, tone: "neutral" });
    // And no time to go read off that jump.
    const later = feed([[T0 + min(16), { phase: "ready", height: 219_020, blocks_behind: 13_993 }]]);
    expect(catchupLine(13_993, [...s, ...later], T0 + min(16))).toBe(
      "Your node is live, still catching up: 13,993 blocks behind",
    );
  });
});

// Plan decision 1: an amber stale sentence outranks a fork and "behind the
// signers"; the neutral line is not a verdict, so it yields to both.
describe("chainCardMessage", () => {
  const FORK = "A longer chain exists that this node cannot obtain blocks for.";
  const SIGNERS = "Other nodes have signed blocks this one does not have yet.";
  const amber = { message: STALE, tone: "amber" as const };
  const neutral = { message: CHECKING_CATCHUP, tone: "neutral" as const };

  it("puts an amber stale tip first", () => {
    expect(chainCardMessage(amber, FORK, SIGNERS)).toEqual({ message: STALE, calm: false });
    expect(chainCardMessage(amber, null, SIGNERS)).toEqual({ message: STALE, calm: false });
  });

  it("puts a fork before behind the signers", () => {
    expect(chainCardMessage(null, FORK, SIGNERS)).toEqual({ message: FORK, calm: false });
    expect(chainCardMessage(null, null, SIGNERS)).toEqual({ message: SIGNERS, calm: false });
  });

  it("lets the neutral line yield to a fork or behind the signers", () => {
    expect(chainCardMessage(neutral, FORK, SIGNERS)).toEqual({ message: FORK, calm: false });
    expect(chainCardMessage(neutral, null, SIGNERS)).toEqual({ message: SIGNERS, calm: false });
  });

  it("says the neutral line quietly when nothing else is wrong", () => {
    expect(chainCardMessage(neutral, null, null)).toEqual({ message: CHECKING_CATCHUP, calm: true });
  });

  it("hides the card when there is nothing to say", () => {
    expect(chainCardMessage(null, null, null)).toBeNull();
  });
});

describe("paceSentence", () => {
  it("is what the catch-up line and the stale card both say", () => {
    const pace = catchupPace(slowNode(), T0 + min(30));
    expect(paceSentence(pace)).toBe(
      `It adds about 8 blocks an hour while the network adds about ${CHAIN_BLOCKS_PER_HOUR}.`,
    );
    expect(catchupLine(10_812, slowNode(), T0 + min(30))).toBe(
      `Your node is live but not catching up: 10,812 blocks behind. ${paceSentence(pace)}`,
    );
    expect(paceSentence(null)).toBeNull();
    expect(paceSentence({ addedPerHour: 50, closingPerHour: 0, spanMs: min(60) })).toBeNull();
    expect(paceSentence({ addedPerHour: 0, closingPerHour: -40, spanMs: min(20) })).toBe(
      "It has added no blocks in the last 20 minutes.",
    );
  });
});
