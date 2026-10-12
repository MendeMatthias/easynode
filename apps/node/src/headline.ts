/**
 * The status screen's headline: the badge, the line under it, and the orb's
 * mode, which the ambient line, the compact orb and the power core follow.
 *
 * One function, so the headline cannot say LIVE while a card below it says
 * the node is not following the chain. It did on 2026-10-12: a node that
 * follows signatures sat on block 245,498 for six hours under a green "Your
 * node is live and helping the network", above an amber "The node is not
 * following the chain" and a BEHIND agreement row.
 *
 * It reaches no verdict of its own. Every problem it names is one a card
 * below already shows, in Rust's sentences:
 * - `rcStalled`: the Block checking card (btxd cannot check the new proof of
 *   work on this machine);
 * - an amber stale sentence: the chain card (catchup-trend.ts `staleCard`);
 * - the watchdog's stall verdict: Node mode's "Needs attention"
 *   (validation.ts);
 * - a fork or behind-the-signers sentence: the chain card too
 *   (`chainCardMessage`), which ranks them after the stale sentence, as this
 *   does;
 * - the agreement row, only where it is about this node
 *   (btx_core::chain_agreement `ScreenRow::this_node`).
 * The headline says which of them holds, in a few words, and the card says
 * the rest.
 */
import type { StaleCard } from "./catchup-trend";

/** The orb's states. "stalled" is running but not following: amber, still
 *  breathing, because the process really is alive. */
export type VisualMode = "ready" | "syncing" | "stopped" | "error" | "stalled";

/** The parts of a status phase the headline reads (main.ts `NodePhase`).
 *  Every phase is named, so a new one does not compile until it is decided
 *  here. */
export type HeadlinePhase =
  | { phase: "ready"; blocks_behind: number }
  | { phase: "syncing"; height: number; headers: number; progress: number }
  | { phase: "warming"; message: string }
  | { phase: "error"; message: string }
  | {
      phase: "welcome" | "downloading" | "preparing" | "starting" | "loading_snapshot" | "stopped";
    };

export type HeadlineFacts = {
  /** btxd cannot check the new proof of work here (NodeStatusInfo.rc_stalled). */
  rcStalled: boolean;
  /** btxd checks blocks on the processor and may fall behind. */
  rcMayFallBehind: boolean;
  /** The chain card's stale sentence this tick, as `staleCard` judged it. */
  stale: StaleCard | null;
  /** Rust's fork sentence and its behind-the-signers sentence. */
  forkMessage: string | null;
  behindSignersMessage: string | null;
  /** Seconds since the tip's median time; null when the node did not say. */
  tipAgeSecs: number | null;
  /** The stall verdict's class (NodeStatusInfo.stall), null when none. */
  stallClass: string | null;
  /** What the agreement row says about this node, null when nothing. */
  agreement: "behind" | "differs" | null;
  /** The catch-up line (catchup-trend.ts `catchupLine`) for a ready node far
   *  enough behind to say so, null otherwise. The caller holds the samples. */
  catchup: string | null;
};

export type Headline = { badge: string; sub: string; mode: VisualMode };

/** "6 hours old", or "3 days old" from two days on: btx_core::role
 *  `age_words`, so the headline and the Chain position line say one age. */
export function tipAgeWords(secs: number): string {
  const hours = Math.floor(secs / 3600);
  return hours >= 48 ? `${Math.floor(secs / 86_400)} days old` : `${hours} hours old`;
}

const fmtInt = (n: number): string => n.toLocaleString("en-US");

export function headline(p: HeadlinePhase, f: HeadlineFacts): Headline {
  switch (p.phase) {
    case "ready":
      return readyHeadline(f);
    case "syncing":
      if (p.height === 0 && p.headers > 0) {
        // Headers phase: the chain itself has not started, so the header
        // count is the thing moving.
        return {
          badge: `FETCHING HEADERS ${(p.progress * 100).toFixed(0)}%`,
          sub: `Counting the chain: ${fmtInt(p.headers)} block headers so far`,
          mode: "syncing",
        };
      }
      return {
        badge: `SYNCING ${(p.progress * 100).toFixed(1)}%`,
        sub: notKeepingUp(f) ? "Syncing, but not keeping up with the chain right now" : `Catching up to block ${fmtInt(p.headers)}`,
        mode: notKeepingUp(f) ? "stalled" : "syncing",
      };
    case "starting":
      return { badge: "STARTING", sub: "This can take a moment", mode: "syncing" };
    case "loading_snapshot":
      return { badge: "LOADING SNAPSHOT", sub: "This can take a moment", mode: "syncing" };
    case "warming":
      return {
        badge: "GETTING READY",
        sub: `${p.message} Nothing is wrong. Your node is checking its data. This can take a while after a hard shutdown.`,
        mode: "syncing",
      };
    case "stopped":
      return { badge: "STOPPED", sub: "The node is not running", mode: "stopped" };
    case "error":
      // The message itself is in the error card, which sits right under the
      // headline while the node is stopped. An empty line here read as
      // "nothing to say" above a red card the eye had not reached.
      return { badge: "NEEDS ATTENTION", sub: "Your node stopped. What happened is just below", mode: "error" };
    default:
      return { badge: "…", sub: "", mode: "stopped" };
  }
}

/** Amber on a syncing node: the chain card's "added no blocks in twenty
 *  minutes" or "slower than the chain over the hour", or the watchdog's
 *  frozen verdict. All are "not keeping up"; none is "stopped". */
const notKeepingUp = (f: HeadlineFacts): boolean => f.stale?.tone === "amber" || f.stallClass !== null;

function readyHeadline(f: HeadlineFacts): Headline {
  if (f.rcStalled) {
    // Running, connected, answering RPC, and not following the chain.
    return {
      badge: "NOT FOLLOWING",
      sub: "Your node is running but cannot check new blocks on this machine",
      mode: "stalled",
    };
  }
  // "pins_reject_every_signature": signatures arrive and none is accepted,
  // because the keys it trusts stopped signing or its list of them is out of
  // date. Either way no signature it trusts comes, which is the part worth
  // saying in a line; Node mode carries the rest.
  const noTrustedSignature = f.stallClass === "pins_reject_every_signature";
  if (f.stale?.tone === "amber") {
    const age = f.tipAgeSecs !== null ? tipAgeWords(f.tipAgeSecs) : null;
    let sub = age
      ? `Your node is running, but its newest block is ${age}`
      : "Your node is running, but it is not following the chain";
    if (noTrustedSignature) sub += ". No signature it trusts has arrived since";
    return { badge: "NOT FOLLOWING", sub, mode: "stalled" };
  }
  if (f.stallClass) {
    // The watchdog's verdict (btx_core::watchdog): frozen for 15 minutes,
    // which it judges "far outside normal" on its own measurements, well
    // before the tip's two hours. No age here: the tip's age is read from
    // its median time, minutes behind the block itself.
    return {
      badge: "NOT FOLLOWING",
      sub: noTrustedSignature
        ? "Your node is running, but no signature it trusts is arriving, so it cannot move"
        : "Your node is running, but it has stopped moving. Node mode below says why",
      mode: "stalled",
    };
  }
  if (f.agreement === "differs") {
    return {
      badge: "DIFFERENT BRANCH",
      sub: "Your node is running, on a different branch than the other sources",
      mode: "stalled",
    };
  }
  if (f.forkMessage || f.behindSignersMessage) {
    // A longer branch it cannot get, headers it cannot get bodies for, or
    // blocks the signers confirmed that it lacks: newer blocks exist.
    return {
      badge: "BEHIND",
      sub: "Your node is running, but newer blocks exist that it does not have yet",
      mode: "stalled",
    };
  }
  if (f.rcMayFallBehind) {
    return { badge: "LIVE", sub: "Your node is helping the network, checking blocks on the processor", mode: "ready" };
  }
  if (f.catchup) {
    // "LIVE", running and connected, with the gap said rather than letting
    // "helping the network" stand on its own (catchup-trend.ts).
    return { badge: "LIVE", sub: f.catchup, mode: "ready" };
  }
  if (f.agreement === "behind") {
    // The earliest sign of a frozen node, minutes in, long before its tip is
    // two hours old (TIP_STALE_AFTER_SECS). The agreement card already says
    // BEHIND in amber; the headline points at it and keeps the orb green,
    // because a few blocks behind for one comparison is also a slow block.
    return { badge: "LIVE", sub: "Your node is live, but other sources have newer blocks", mode: "ready" };
  }
  // "LIVE", not "READY": the node is running and serving the network now.
  return { badge: "LIVE", sub: "Your node is live and helping the network", mode: "ready" };
}
