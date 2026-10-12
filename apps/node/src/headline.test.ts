import { describe, expect, it } from "vitest";
import { type HeadlineFacts, type HeadlinePhase, headline, tipAgeWords, waitingLine } from "./headline";

const facts = (over: Partial<HeadlineFacts> = {}): HeadlineFacts => ({
  rcStalled: false,
  rcMayFallBehind: false,
  stale: null,
  forkMessage: null,
  behindSignersMessage: null,
  tipAgeSecs: 300,
  stallClass: null,
  agreement: null,
  catchup: null,
  ...over,
});

const ready: HeadlinePhase = { phase: "ready", blocks_behind: 0 };
const STALE = {
  message:
    "The newest block this node has is 6 hours old. That is measured against the clock, not against what its peers report, so it holds even when every peer agrees with it. The node is not following the chain.",
  tone: "amber" as const,
};

describe("headline on a node at its tip", () => {
  it("says LIVE and green when nothing below says otherwise", () => {
    expect(headline(ready, facts())).toEqual({
      badge: "LIVE",
      sub: "Your node is live and helping the network",
      mode: "ready",
    });
  });

  // 2026-10-12: a node following signatures sat on block 245,498 for six
  // hours under this green LIVE, above an amber "The node is not following
  // the chain". The headline now says what the card says.
  it("is not LIVE when the chain card says the tip is hours old", () => {
    const h = headline(ready, facts({ stale: STALE, tipAgeSecs: 6 * 3600 + 300 }));
    expect(h.badge).toBe("NOT FOLLOWING");
    expect(h.sub).toBe("Your node is running, but its newest block is 6 hours old");
    expect(h.mode).toBe("stalled");
  });

  it("says why when the signatures it trusts stopped arriving", () => {
    const h = headline(
      ready,
      facts({ stale: STALE, tipAgeSecs: 6 * 3600 + 300, stallClass: "pins_reject_every_signature" }),
    );
    expect(h.badge).toBe("NOT FOLLOWING");
    expect(h.sub).toBe(
      "Your node is running, but its newest block is 6 hours old. No signature it trusts has arrived since",
    );
  });

  // The watchdog's verdict on a node following signatures that has not moved
  // for 15 minutes (btx_core::watchdog FROZEN_VERDICT_SECS), well before the
  // tip is two hours old. Node mode already says "Needs attention" with it.
  it("is not LIVE once the watchdog calls a follower frozen, before the 2-hour rule", () => {
    const h = headline(ready, facts({ tipAgeSecs: 62 * 60, stallClass: "pins_reject_every_signature" }));
    expect(h).toEqual({
      badge: "NOT FOLLOWING",
      sub: "Your node is running, but no signature it trusts is arriving, so it cannot move",
      mode: "stalled",
    });
  });

  it("says it stopped moving for a stall of any other kind", () => {
    const h = headline(ready, facts({ tipAgeSecs: 40 * 60, stallClass: "no_qualifying_peer" }));
    expect(h).toEqual({
      badge: "NOT FOLLOWING",
      sub: "Your node is running, but it has stopped moving. Node mode below says why",
      mode: "stalled",
    });
  });

  it("puts a frozen node ahead of a branch it may also be on", () => {
    const h = headline(ready, facts({ stallClass: "body_missing", agreement: "differs" }));
    expect(h.badge).toBe("NOT FOLLOWING");
  });

  it("does not name signatures for a stall of another kind", () => {
    const h = headline(ready, facts({ stale: STALE, tipAgeSecs: 3 * 3600, stallClass: "body_missing" }));
    expect(h.sub).toBe("Your node is running, but its newest block is 3 hours old");
  });

  it("says days from two days on, as the role card does", () => {
    const h = headline(ready, facts({ stale: STALE, tipAgeSecs: 3 * 86_400 + 60 }));
    expect(h.sub).toBe("Your node is running, but its newest block is 3 days old");
  });

  it("keeps LIVE while the stale sentence is only the quiet 'Checking...' line", () => {
    const h = headline(
      { phase: "ready", blocks_behind: 40 },
      facts({
        stale: { message: "Checking whether your node is catching up...", tone: "neutral" },
        catchup: "Your node is live, still catching up: 40 blocks behind",
      }),
    );
    expect(h.badge).toBe("LIVE");
    expect(h.sub).toBe("Your node is live, still catching up: 40 blocks behind");
    expect(h.mode).toBe("ready");
  });

  it("says BEHIND when the chain card holds a fork sentence", () => {
    const h = headline(
      ready,
      facts({ forkMessage: "A longer chain exists (height 245,800, 41 blocks ahead of this node ...)" }),
    );
    expect(h).toEqual({
      badge: "BEHIND",
      sub: "Your node is running, but newer blocks exist that it does not have yet",
      mode: "stalled",
    });
  });

  it("says BEHIND when the signers have confirmed blocks it does not have", () => {
    const h = headline(ready, facts({ behindSignersMessage: "The network's signers have confirmed ..." }));
    expect(h.badge).toBe("BEHIND");
    expect(h.mode).toBe("stalled");
  });

  it("puts an old tip ahead of a fork, as the chain card does", () => {
    const h = headline(
      ready,
      facts({ stale: STALE, tipAgeSecs: 6 * 3600, forkMessage: "A longer chain exists ..." }),
    );
    expect(h.badge).toBe("NOT FOLLOWING");
  });

  it("says DIFFERENT BRANCH when no other operator holds this node's block", () => {
    const h = headline(ready, facts({ agreement: "differs" }));
    expect(h).toEqual({
      badge: "DIFFERENT BRANCH",
      sub: "Your node is running, on a different branch than the other sources",
      mode: "stalled",
    });
  });

  it("gives a softer hint, still green, when other sources are only ahead", () => {
    // The early sign of a frozen node, minutes in, long before its tip is
    // two hours old. The agreement card already says BEHIND; the headline
    // points at it without the alarm that two hours earns.
    const h = headline(ready, facts({ agreement: "behind" }));
    expect(h).toEqual({
      badge: "LIVE",
      sub: "Your node is live, but other sources have newer blocks",
      mode: "ready",
    });
  });

  it("lets its own catch-up line outrank the agreement hint", () => {
    const h = headline(
      { phase: "ready", blocks_behind: 40 },
      facts({ agreement: "behind", catchup: "Your node is live, still catching up: 40 blocks behind" }),
    );
    expect(h.sub).toBe("Your node is live, still catching up: 40 blocks behind");
  });

  it("keeps the processor line for a node that may fall behind", () => {
    const h = headline(ready, facts({ rcMayFallBehind: true }));
    expect(h).toEqual({
      badge: "LIVE",
      sub: "Your node is helping the network, checking blocks on the processor",
      mode: "ready",
    });
  });

  it("says NOT FOLLOWING when the engine cannot check new blocks, before anything else", () => {
    const h = headline(ready, facts({ rcStalled: true, stale: STALE, agreement: "differs" }));
    expect(h).toEqual({
      badge: "NOT FOLLOWING",
      sub: "Your node is running but cannot check new blocks on this machine",
      mode: "stalled",
    });
  });
});

describe("headline while syncing", () => {
  const syncing: HeadlinePhase = { phase: "syncing", height: 200_000, headers: 245_760, progress: 0.913 };

  it("shows the percentage and the target", () => {
    expect(headline(syncing, facts())).toEqual({
      badge: "SYNCING 91.3%",
      sub: "Catching up to block 245,760",
      mode: "syncing",
    });
  });

  it("counts headers before the first block", () => {
    const h = headline({ phase: "syncing", height: 0, headers: 120_000, progress: 0.42 }, facts());
    expect(h).toEqual({
      badge: "FETCHING HEADERS 42%",
      sub: "Counting the chain: 120,000 block headers so far",
      mode: "syncing",
    });
  });

  it("turns amber when the watchdog calls it frozen", () => {
    const h = headline(syncing, facts({ stallClass: "attestation_missing" }));
    expect(h.sub).toBe("Syncing, but not keeping up with the chain right now");
    expect(h.mode).toBe("stalled");
  });

  it("turns amber when the chain card says it is not keeping up", () => {
    const h = headline(syncing, facts({ stale: STALE }));
    expect(h.badge).toBe("SYNCING 91.3%");
    expect(h.sub).toBe("Syncing, but not keeping up with the chain right now");
    expect(h.mode).toBe("stalled");
  });
});

describe("headline when the node is not running", () => {
  it("says the error is just below rather than leaving the line empty", () => {
    expect(headline({ phase: "error", message: "btxd exited" }, facts())).toEqual({
      badge: "NEEDS ATTENTION",
      sub: "Your node stopped. What happened is just below",
      mode: "error",
    });
  });

  it("says stopped", () => {
    expect(headline({ phase: "stopped" }, facts())).toEqual({
      badge: "STOPPED",
      sub: "The node is not running",
      mode: "stopped",
    });
  });

  it("ignores chain facts left over from the last run", () => {
    const h = headline({ phase: "stopped" }, facts({ stale: STALE, agreement: "differs" }));
    expect(h.badge).toBe("STOPPED");
    expect(h.mode).toBe("stopped");
  });

  it("says starting, loading and warming as before", () => {
    expect(headline({ phase: "starting" }, facts()).badge).toBe("STARTING");
    expect(headline({ phase: "loading_snapshot" }, facts()).badge).toBe("LOADING SNAPSHOT");
    const w = headline({ phase: "warming", message: "Verifying blocks…" }, facts());
    expect(w.badge).toBe("GETTING READY");
    expect(w.sub).toBe(
      "Verifying blocks… Nothing is wrong. Your node is checking its data. This can take a while after a hard shutdown.",
    );
    expect(w.mode).toBe("syncing");
  });
});

describe("tipAgeWords", () => {
  it("reads hours, then days from 48 hours, like btx_core::role", () => {
    expect(tipAgeWords(2 * 3600 + 1)).toBe("2 hours old");
    expect(tipAgeWords(47 * 3600 + 3599)).toBe("47 hours old");
    expect(tipAgeWords(48 * 3600)).toBe("2 days old");
  });
});

describe("waitingLine", () => {
  it("says nothing while a poll is quick", () => {
    expect(waitingLine(0)).toBeNull();
    expect(waitingLine(9_999)).toBeNull();
  });

  // get_node_status asks btxd twice with a 60 s timeout each; a btxd that
  // accepts the connection and never answers held the last screen, LIVE
  // included, for up to two minutes with nothing saying so.
  it("says the screen is waiting once a poll has taken 10 seconds", () => {
    expect(waitingLine(10_000)).toBe("Waiting for your node to answer. This screen is 10 s old");
    expect(waitingLine(65_400)).toBe("Waiting for your node to answer. This screen is 65 s old");
  });

  it("switches to minutes from two minutes on", () => {
    expect(waitingLine(120_000)).toBe("Waiting for your node to answer. This screen is 2 min old");
    expect(waitingLine(605_000)).toBe("Waiting for your node to answer. This screen is 10 min old");
  });
});
