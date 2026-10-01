import { describe, expect, it } from "vitest";
import {
  followRowVisible,
  followToggleOn,
  stalledFollowOffer,
  validationView,
  type FollowInput,
  type ValidationInput,
} from "./validation";

const base: ValidationInput = {
  running: true,
  uptime_secs: 900, // past the checking window unless a test says otherwise
  rc_mode: null,
  rc_validates_independently: false,
  rc_may_fall_behind: false,
  rc_stalled: false,
  rc_trusted_mirror: false,
};

describe("validationView", () => {
  it("lets the engine's standing warning outrank the startup verdict", () => {
    // The log said Full at startup; the engine has since quarantined the chip.
    const note = "This machine's graphics chip did not pass the node engine's own check.";
    const v = validationView({
      ...base,
      rc_mode: "strict-device",
      rc_validates_independently: true,
      rc_stalled: true,
      rc_unverifiable_message: note,
    });
    expect(v.state).toBe("Stopped");
    expect(v.note).toBe(note);
    expect(v.cls).toBe("is-stalled");
    // And it does not wait for a log verdict that may never be readable.
    expect(
      validationView({ ...base, uptime_secs: 30, rc_mode: null, rc_unverifiable_message: note })
        .state,
    ).toBe("Stopped");
  });

  it("keeps the generic stalled sentence when the engine gives none", () => {
    const v = validationView({ ...base, rc_mode: "strict-device", rc_stalled: true });
    expect(v.state).toBe("Stopped");
    expect(v.note).toMatch(/cannot check the new proof of work/);
  });

  it("explains the startup episode instead of showing a blank card", () => {
    // btxd runs one full production episode before it will judge the machine —
    // 102-218 s measured on an M2 Pro. The card used to be hidden for that whole
    // time while the GPU sat pegged, explaining nothing.
    const v = validationView({ ...base, uptime_secs: 30, rc_mode: null });
    expect(v.state).toBe("Checking…");
    expect(v.note).toMatch(/few minutes/i);
    expect(v.cls).toBe("");
  });

  it("stops claiming to check once the window has passed", () => {
    // A btxd older than v0.33.2 never logs a verdict at all; saying "checking"
    // forever would be a lie, so fall silent instead.
    expect(validationView({ ...base, uptime_secs: 601, rc_mode: null }).state).toBeNull();
  });

  it("says nothing until the node is running and has reported a policy", () => {
    // Not running at all.
    expect(validationView({ ...base, running: false, rc_mode: "strict-device" }).state).toBeNull();
    // Running but btxd has not logged its policy yet (early startup, or a
    // pre-v0.33.2 node that never logs one).
    expect(validationView({ ...base, running: true, rc_mode: null }).state).toBeNull();
  });

  it("reports full validation on a qualified device", () => {
    const v = validationView({
      ...base,
      rc_mode: "strict-device",
      rc_validates_independently: true,
    });
    expect(v.state).toBe("Full");
    expect(v.cls).toBe("");
  });

  it("warns, without alarm, when the machine can only check on the processor", () => {
    const v = validationView({
      ...base,
      rc_mode: "auto-fallback",
      rc_may_fall_behind: true,
    });
    expect(v.state).toBe("Basic (light)");
    expect(v.cls).toBe("is-degraded");
    // The Basic node does not "fall behind" — it STOPS at 185,000 and stays
    // there. The old wording implied it kept crawling along, which is the
    // impression the whole Block-checking readout exists to prevent.
    expect(v.note).toMatch(/stops at block 185,000/i);
    expect(v.note).not.toMatch(/fall behind/i);
  });

  // "Basic" next to "Full" reads like a lesser KIND of node, and people ask
  // whether they are running something light or partial. They are not: every
  // install keeps the whole chain (prune=0) and serves it to peers. Only the
  // checking of the newest proof of work differs. Say so, or the label quietly
  // tells people their node matters less than it does.
  it("tells a Basic node it still keeps and shares the whole chain", () => {
    const v = validationView({
      ...base,
      rc_mode: "auto-fallback",
      rc_may_fall_behind: true,
    });
    expect(v.note).toMatch(/whole chain|full chain|every block it has/i);
  });

  // "Basic" next to "Full" reads like a lesser KIND of node, and people ask
  // whether they are running something light or partial. They are not: every
  // install keeps the whole chain (prune=0) and serves it to peers. Only the
  // checking of the newest proof of work differs. Say so, or the label quietly
  // tells people their node matters less than it does.
  it("tells a Basic node it still keeps and shares the whole chain", () => {
    const v = validationView({
      ...base,
      rc_mode: "auto-fallback",
      rc_may_fall_behind: true,
    });
    expect(v.note).toMatch(/whole chain|full chain|every block it has/i);
  });

  it("reports the stall as stopped, and takes priority over every other state", () => {
    const v = validationView({
      ...base,
      rc_mode: "strict-device",
      // Even if some other flag were somehow set, the failure must win: this is
      // the state where the node looks healthy but is not following the chain.
      rc_validates_independently: true,
      rc_stalled: true,
    });
    expect(v.state).toBe("Stopped");
    expect(v.cls).toBe("is-stalled");
    expect(v.note).toMatch(/already downloaded is safe/i);
  });

  it("treats cpu-diagnostic as a can-fall-behind mode", () => {
    // The Rust side sets may_fall_behind for cpu-diagnostic, so this is the
    // combination that actually reaches the UI.
    const v = validationView({
      ...base,
      rc_mode: "cpu-diagnostic",
      rc_may_fall_behind: true,
    });
    expect(v.state).toBe("Basic (light)");
    expect(v.cls).toBe("is-degraded");
  });

  it("shows a mode we have no copy for plainly, instead of inventing a meaning", () => {
    // A future btxd mode: no flags set, because the Rust side would not know it
    // either. Showing the raw string beats guessing at reassurance.
    const v = validationView({ ...base, rc_mode: "some-future-mode" });
    expect(v.state).toBe("some-future-mode");
    expect(v.note).toBe("");
    expect(v.cls).toBe("");
  });
  // A machine that cannot check the proof itself now FOLLOWS the chain through
  // an attestation quorum instead of parking at 184,999. Its policy line is
  // shaped exactly like a stall (strict-device, ready=0), so this branch has to
  // win before the raw-mode fallback prints "strict-device" at the user.
  it("names the trusted mirror instead of leaking a raw mode string", () => {
    const v = validationView({
      ...base,
      rc_mode: "strict-device",
      rc_trusted_mirror: true,
    });
    expect(v.state).toBe("Mirror");
    // Since the 23 September split one key signs the valid chain; the copy
    // must not promise two independent operators, or unconditional balance
    // checks for a node that started from a snapshot.
    expect(v.note).not.toMatch(/two independent/);
    expect(v.note).toMatch(/one signer/);
    expect(v.note).toMatch(/re-checks the snapshot it started from/);
    expect(v.state).not.toBe("strict-device");
    expect(v.note).toMatch(/block 185,000/);
    // It must not repeat the old promise that the node stops there.
    expect(v.note).not.toMatch(/stops at block/i);
  });

  it("says plainly that the mirror is a trust trade, not a free upgrade", () => {
    const v = validationView({ ...base, rc_mode: "strict-device", rc_trusted_mirror: true });
    expect(v.note).toMatch(/trusting/i);
  });
});

describe("trusted mirror archive-authority escalation", () => {
  it("escalates a mirror with ZERO authority archive peers before the height freezes", () => {
    const v = validationView({
      ...base,
      rc_mode: "strict-device",
      rc_trusted_mirror: true,
      archive_authority: 0,
    });
    expect(v.state).toBe("Mirror: waiting for a source");
    expect(v.cls).toBe("is-stalled");
    expect(v.note).toContain("not connected to any node allowed to hand them over");
  });

  it("keeps the calm Mirror copy when authority count is unknown (null)", () => {
    const v = validationView({
      ...base,
      rc_mode: "strict-device",
      rc_trusted_mirror: true,
      archive_authority: null,
    });
    expect(v.state).toBe("Mirror");
  });

  it("keeps the calm Mirror copy when at least one authority archive exists", () => {
    const v = validationView({
      ...base,
      rc_mode: "strict-device",
      rc_trusted_mirror: true,
      archive_authority: 4,
    });
    expect(v.state).toBe("Mirror");
    expect(v.cls).toBe("is-degraded");
  });
});

describe("classified stall verdicts", () => {
  it("outranks the mode copy with the discriminator's own summary", () => {
    const v = validationView({
      ...base,
      rc_mode: "strict-device",
      rc_trusted_mirror: true,
      stall: {
        class: "no_qualifying_peer",
        summary: "no connected peer is allowed to hand this node signed confirmations",
      },
    });
    expect(v.state).toBe("Needs attention");
    expect(v.cls).toBe("is-stalled");
    expect(v.note).toContain("signed confirmations");
  });

  it("null stall changes nothing", () => {
    const v = validationView({
      ...base,
      rc_mode: "strict-device",
      rc_trusted_mirror: true,
      stall: null,
    });
    expect(v.state).toBe("Mirror");
  });
});

describe("the way out for a machine whose chip cannot check blocks", () => {
  const checking: FollowInput = {
    rc_stalled: false,
    rc_trusted_mirror: false,
    rc_validates_independently: true,
    follow_signatures: false,
  };
  // Stalled: the chip failed the check at startup (the log says ready=0), or
  // the engine's standing warning says so later in the run.
  const stalled: FollowInput = { ...checking, rc_stalled: true, rc_validates_independently: false };

  it("offers to follow signatures, and says what the switch costs", () => {
    const offer = stalledFollowOffer(stalled);
    expect(offer).toMatch(/cannot check new blocks itself/);
    expect(offer).toMatch(/follow signatures instead/);
    expect(offer).toMatch(/cannot sign/);
    expect(offer).toMatch(/switch back in Settings/);
  });

  it("stays quiet where there is nothing to offer", () => {
    // A machine that checks blocks: the slow-machine offer decides there.
    expect(stalledFollowOffer(checking)).toBeNull();
    // A mirror already follows signatures, and the owner who chose already did.
    expect(stalledFollowOffer({ ...stalled, rc_trusted_mirror: true })).toBeNull();
    expect(stalledFollowOffer({ ...stalled, follow_signatures: true })).toBeNull();
  });

  it("keeps the Settings switch on the machine that needs it most", () => {
    // The old rule showed it only where the node checks blocks, which a
    // stalled machine does not, so the one machine with no other way out had
    // no switch either.
    expect(followRowVisible(stalled)).toBe(true);
    expect(followRowVisible(checking)).toBe(true);
    const chosen = { ...checking, rc_validates_independently: false, follow_signatures: true };
    expect(followRowVisible(chosen)).toBe(true);
    // A machine that follows signatures anyway has no choice to make.
    const mirror: FollowInput = {
      rc_stalled: false,
      rc_trusted_mirror: true,
      rc_validates_independently: false,
      follow_signatures: false,
    };
    expect(followRowVisible(mirror)).toBe(false);
  });
});

describe("a machine whose graphics card hung the engine's start-up check (0.7.1)", () => {
  const hungMirror: FollowInput = {
    rc_stalled: false,
    rc_trusted_mirror: true,
    rc_validates_independently: false,
    follow_signatures: false,
    gpu_start_hung: true,
  };

  it("says why the node follows signatures, and the way back", () => {
    const v = validationView({
      ...base,
      rc_mode: "strict-device",
      rc_trusted_mirror: true,
      gpu_start_hung: true,
    });
    expect(v.state).toBe("Mirror");
    expect(v.note).toMatch(/graphics card did not finish the engine's start-up check/);
    expect(v.note).toMatch(/follows signatures for now/);
    expect(v.note).toMatch(/Check blocks in Settings tries the card again/);
    expect(v.note.includes(String.fromCharCode(0x2014))).toBe(false); // no em-dash
  });

  it("says chip on a Mac, as the rest of the app does", () => {
    const v = validationView({
      ...base,
      rc_mode: "strict-device",
      rc_trusted_mirror: true,
      gpu_start_hung: true,
      graphics_word: "graphics chip",
    });
    expect(v.note).toMatch(/This machine's graphics chip did not finish/);
    expect(v.note).toMatch(/tries the chip again/);
    expect(v.note).not.toMatch(/card/);
  });

  it("keeps the plain mirror note where the card did not hang", () => {
    const v = validationView({ ...base, rc_mode: "strict-device", rc_trusted_mirror: true });
    expect(v.note).not.toMatch(/start-up check/);
  });

  it("still says so first when no source hands over confirmations", () => {
    const v = validationView({
      ...base,
      rc_mode: "strict-device",
      rc_trusted_mirror: true,
      gpu_start_hung: true,
      archive_authority: 0,
    });
    expect(v.state).toBe("Mirror: waiting for a source");
  });

  it("shows the Settings switch, switched on, so one click tries the card again", () => {
    expect(followRowVisible(hungMirror)).toBe(true);
    expect(followToggleOn(hungMirror)).toBe(true);
    expect(followToggleOn({ ...hungMirror, gpu_start_hung: false })).toBe(false);
    expect(
      followToggleOn({ ...hungMirror, gpu_start_hung: false, follow_signatures: true }),
    ).toBe(true);
  });
});
