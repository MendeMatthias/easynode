import { describe, expect, it } from "vitest";
import { validationView, type ValidationInput } from "./validation";

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
    expect(v.state).toBe("Mirror — waiting for a source");
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
