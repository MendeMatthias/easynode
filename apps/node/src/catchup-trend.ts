/** Is the gap actually closing, or is the node just sitting behind?
 *
 *  `blocks_behind` alone cannot tell those apart, and the status card used to
 *  say "still catching up" for both. On 2026-09-15 a Mac sat between 84 and 99
 *  blocks behind for hours, on the correct chain, matching an independent
 *  witness at five settled heights, and never converged: the cadence burst hold
 *  (btxchain/btx#140) paces a node that is more than three blocks behind to one
 *  block per `nPowTargetSpacing`, which is the one speed at which a gap can
 *  never close. Measured 89.2 s per block over 1 h 47 m while the chain
 *  produced faster. The screen said "still catching up" throughout, which reads
 *  as slow internet and sends people hunting for peers. Two operators did
 *  exactly that on 2026-09-06 and 2026-09-13 before the cause was known.
 *
 *  So the card needs the DERIVATIVE, not the level. This is deliberately a pure
 *  function over samples the caller already has, so the arithmetic is testable
 *  without a node: that is the same reason `catchupEta` lives in wallet.ts. */

/** How far back to look. Long enough that one slow block does not read as a
 *  stall, short enough to be honest within a few minutes of opening the app. */
export const TREND_WINDOW_MS = 10 * 60 * 1000;

/** Blocks the gap must close across the window to count as converging. The
 *  trap's signature is a gap that drifts a block or two either way, so a
 *  threshold of 1 would read noise as progress. */
export const TREND_MIN_CLOSED = 3;

/** Refuse to judge on less than this much history. Without it a freshly opened
 *  app calls every node stalled for its first sample. */
export const TREND_MIN_SPAN_MS = 4 * 60 * 1000;

/** What the chain adds in an hour: one block per 90 s target spacing. Nominal,
 *  not measured, because the difficulty adjustment holds the chain near it and
 *  an hour's measured count swings by a sixth on luck alone. */
export const CHAIN_BLOCKS_PER_HOUR = 40;

/** How much history is kept, and how far back the pace is read. Longer than
 *  the trend window on purpose: a node adding eight blocks an hour moves about
 *  one block in ten minutes, and a rate read off one block is noise. */
export const HISTORY_WINDOW_MS = 60 * 60 * 1000;

/** Refuse to state a pace on less history than this. Fifteen minutes is about
 *  ten blocks of chain, enough to tell a node adding 8 an hour from one adding
 *  40. Before it, the line keeps its old wording. */
export const PACE_MIN_SPAN_MS = 15 * 60 * 1000;

/** The kept samples are at least this far apart; only the newest moves with
 *  every poll. The status poll runs every 1.5 s, and an hour of that is 2,400
 *  samples for arithmetic that only ever reads the two ends of a window. */
export const SAMPLE_SPACING_MS = 15 * 1000;

/** The most samples the history can hold: the window at the spacing, plus the
 *  newest. */
export const MAX_SAMPLES = HISTORY_WINDOW_MS / SAMPLE_SPACING_MS + 1;

/** `height` is optional so a reading without one still feeds the trend; the
 *  pace needs it at both ends of its window. */
export type CatchupSample = { at: number; behind: number; height?: number };

/** `converging` the gap is closing, `stalled` it is not, `unknown` not enough
 *  history yet. `unknown` must render as the old wording, never as an alarm. */
export type CatchupTrend = "converging" | "stalled" | "unknown";

export const catchupTrend = (samples: CatchupSample[], now: number): CatchupTrend => {
  const live = samples
    .filter((s) => Number.isFinite(s.at) && Number.isFinite(s.behind) && now - s.at <= TREND_WINDOW_MS)
    .sort((a, b) => a.at - b.at);
  if (live.length < 2) return "unknown";
  const oldest = live[0];
  const newest = live[live.length - 1];
  if (newest.at - oldest.at < TREND_MIN_SPAN_MS) return "unknown";
  return oldest.behind - newest.behind >= TREND_MIN_CLOSED ? "converging" : "stalled";
};

/** How fast this node really moves, read off the two ends of the last hour:
 *  blocks it added per hour, and blocks per hour the gap closed (negative
 *  while it grows). */
export type CatchupPace = { addedPerHour: number; closingPerHour: number; spanMs: number };

/** Null until there is enough history to say, like `catchupTrend`'s
 *  `unknown`: a pace read off two minutes would be a guess stated as a fact. */
export const catchupPace = (samples: CatchupSample[], now: number): CatchupPace | null => {
  const live = samples
    .filter(
      (s) =>
        Number.isFinite(s.at) &&
        Number.isFinite(s.behind) &&
        Number.isFinite(s.height) &&
        now - s.at <= HISTORY_WINDOW_MS,
    )
    .sort((a, b) => a.at - b.at);
  if (live.length < 2) return null;
  const oldest = live[0];
  const newest = live[live.length - 1];
  const spanMs = newest.at - oldest.at;
  if (spanMs < PACE_MIN_SPAN_MS) return null;
  const hours = spanMs / 3_600_000;
  return {
    addedPerHour: ((newest.height as number) - (oldest.height as number)) / hours,
    closingPerHour: (oldest.behind - newest.behind) / hours,
    spanMs,
  };
};

/** "about 6 hours" for a gap closing at a measured rate, or null when it is
 *  not closing. Whole hours below two days, then days: "57 hours" reads worse
 *  than "2 days", and the figure is an extrapolation, not a promise. */
export const timeToGo = (behind: number, closingPerHour: number): string | null => {
  if (!(behind > 0) || !(closingPerHour > 0)) return null;
  const hours = behind / closingPerHour;
  if (hours < 1) return "less than an hour";
  const h = Math.round(hours);
  if (h < 48) return `about ${h} hour${h === 1 ? "" : "s"}`;
  const d = Math.round(hours / 24);
  return d <= 60 ? `about ${d} days` : "more than two months";
};

/** The status line for a live node at least a few blocks behind.
 *
 *  A fresh install starts from the 219,000 snapshot, thousands of blocks below
 *  the tip, and the line used to give it no way to tell a two-hour wait from
 *  one that never ends. On 2026-09-26 a new node reported 4 blocks in 30
 *  minutes, and the only way to learn whether it would ever arrive was to ask
 *  Mende. So the line now carries the node's own measured pace: a time to go
 *  when the gap is closing, and the two rates side by side when it is not.
 *
 *  The pace sentence only appears when its numbers explain the stall. A node
 *  adding more than the chain in the last hour that still reads stalled over
 *  ten minutes (a burst of blocks, a reorg) keeps the plain wording rather than
 *  showing "adds 50 while the network adds 40" under "not catching up". */
export const catchupLine = (behind: number, samples: CatchupSample[], now: number): string => {
  const n = behind.toLocaleString("en-US");
  const pace = catchupPace(samples, now);
  let trend = catchupTrend(samples, now);
  // No recent history to judge on (a node just back from syncing, a paused
  // poll) but an hour's pace to hand: judge that span by the trend's own rule,
  // at least TREND_MIN_CLOSED blocks closed, rather than stay silent.
  if (trend === "unknown" && pace) {
    const closed = (pace.closingPerHour * pace.spanMs) / 3_600_000;
    trend = closed >= TREND_MIN_CLOSED ? "converging" : "stalled";
  }
  if (trend === "stalled") {
    const plain = `Your node is live but not catching up. It is ${n} blocks behind and the gap is not closing.`;
    if (!pace) return plain;
    const added = Math.round(pace.addedPerHour);
    if (added > CHAIN_BLOCKS_PER_HOUR) return plain;
    const head = `Your node is live but not catching up: ${n} blocks behind.`;
    if (added < 1) {
      return `${head} It has added no blocks in the last ${Math.round(pace.spanMs / 60_000)} minutes.`;
    }
    return (
      `${head} It adds about ${added} block${added === 1 ? "" : "s"} an hour ` +
      `while the network adds about ${CHAIN_BLOCKS_PER_HOUR}.`
    );
  }
  const eta = trend === "converging" && pace ? timeToGo(behind, pace.closingPerHour) : null;
  return eta
    ? `Your node is live, still catching up — ${n} blocks behind, ${eta} to go at this pace`
    : `Your node is live, still catching up — ${n} blocks behind`;
};

/** Keep the sample list bounded and in order. The caller holds it across
 *  renders; without the trim it grows for as long as the app is open.
 *
 *  The newest sample always tracks the node, so the trend reads it as it is
 *  now. The ones behind it are kept at least `SAMPLE_SPACING_MS` apart: when
 *  the last one was taken less than that after the one before it, it was only
 *  ever "the latest", and the new reading replaces it. */
export const pushSample = (samples: CatchupSample[], sample: CatchupSample, now: number): CatchupSample[] => {
  const kept = samples.filter((s) => now - s.at <= HISTORY_WINDOW_MS);
  const last = kept[kept.length - 1];
  const before = kept[kept.length - 2];
  if (last && before && last.at - before.at < SAMPLE_SPACING_MS) kept[kept.length - 1] = sample;
  else kept.push(sample);
  return kept.slice(-MAX_SAMPLES);
};
