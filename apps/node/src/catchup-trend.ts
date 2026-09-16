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

export type CatchupSample = { at: number; behind: number };

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

/** Keep the sample list bounded and in order. The caller holds it across
 *  renders; without the trim it grows for as long as the app is open. */
export const pushSample = (samples: CatchupSample[], sample: CatchupSample, now: number): CatchupSample[] =>
  [...samples, sample].filter((s) => now - s.at <= TREND_WINDOW_MS).slice(-240);
