import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import {
  NO_GPU_REASON,
  chipNoticeVisible,
  setupArgs,
  startChoiceView,
  switchLaterShown,
} from "./start-choice";
import { type FollowInput, followRowVisible } from "./validation";

const INDEX = readFileSync(new URL("../index.html", import.meta.url), "utf8").replace(/\r\n/g, "\n");
/** An element's text in index.html, tags dropped and spaces collapsed. */
const textOf = (id: string): string => {
  const m = INDEX.match(new RegExp(`<(\\w+)[^>]*\\bid="${id}"[^>]*>([\\s\\S]*?)</\\1>`));
  if (!m) throw new Error(`no #${id} in index.html`);
  return m[2].replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
};

describe("startChoiceView", () => {
  it("selects Full check first on an NVIDIA machine", () => {
    expect(startChoiceView(true, true, null)).toEqual({ selected: "full_check", fullCheckDisabled: false });
  });

  it("selects Quick start first on a Mac, and Full check can still be picked", () => {
    expect(startChoiceView(true, false, null)).toEqual({ selected: "quick_start", fullCheckDisabled: false });
    expect(startChoiceView(true, false, "full_check")).toEqual({
      selected: "full_check",
      fullCheckDisabled: false,
    });
  });

  it("keeps the owner's pick across polls", () => {
    expect(startChoiceView(true, true, "quick_start")).toEqual({
      selected: "quick_start",
      fullCheckDisabled: false,
    });
    expect(startChoiceView(true, true, "full_check").selected).toBe("full_check");
  });

  it("does not let a machine with no usable GPU pick Full check", () => {
    expect(startChoiceView(false, false, null)).toEqual({ selected: "quick_start", fullCheckDisabled: true });
    expect(startChoiceView(false, false, "full_check")).toEqual({
      selected: "quick_start",
      fullCheckDisabled: true,
    });
    // Rust never says "first" without "possible", but if it did, greyed out wins.
    expect(startChoiceView(false, true, null)).toEqual({ selected: "quick_start", fullCheckDisabled: true });
    expect(NO_GPU_REASON).toBe(
      "This computer has no graphics card the BTX engine can check blocks with.",
    );
  });
});

describe("the line under the choices", () => {
  it("reads as the owner worded it on 30 September", () => {
    expect(textOf("start-choice-note")).toBe(
      "Both start from a recent signed snapshot and check the older history in the background. " +
        "Full check also checks every new block on this computer's graphics card. " +
        "You can switch later in Settings.",
    );
    // The last sentence stands alone, so main.ts can leave it out.
    expect(textOf("start-choice-switch")).toBe("You can switch later in Settings.");
  });

  // The Settings switch shows only where it is a choice (validation.ts
  // followRowVisible). After setup, on each kind of machine, the sentence
  // must say what Settings will show.
  it("promises the Settings switch only where Settings will show it", () => {
    // A Mac or an NVIDIA machine after Quick start: the marker is written.
    const quickOnGpu: FollowInput = {
      rc_stalled: false,
      rc_trusted_mirror: true,
      rc_validates_independently: false,
      follow_signatures: true,
    };
    // After Full check: the node checks blocks itself.
    const fullOnGpu: FollowInput = {
      ...quickOnGpu,
      rc_trusted_mirror: false,
      rc_validates_independently: true,
      follow_signatures: false,
    };
    expect(switchLaterShown(true)).toBe(true);
    expect(followRowVisible(quickOnGpu)).toBe(true);
    expect(followRowVisible(fullOnGpu)).toBe(true);

    // A machine that cannot check blocks: no marker, a mirror, no switch.
    const noGpu: FollowInput = { ...quickOnGpu, follow_signatures: false };
    expect(switchLaterShown(false)).toBe(false);
    expect(followRowVisible(noGpu)).toBe(false);
  });
});

describe("setupArgs", () => {
  it("hands begin_setup the choice under the names the Rust side reads", () => {
    expect(setupArgs("quick_start")).toEqual({ choice: "quick_start" });
    expect(setupArgs("full_check")).toEqual({ choice: "full_check" });
  });
});

describe("chipNoticeVisible", () => {
  it("shows once per engine after the chip is refused and the node follows signatures", () => {
    expect(chipNoticeVisible(true, true, null, "v0.34.9")).toBe(true);
    expect(chipNoticeVisible(true, true, "v0.34.9", "v0.34.9")).toBe(false);
    // A newer engine tries the chip again; refused again, it says so again.
    expect(chipNoticeVisible(true, true, "v0.34.9", "v0.34.12")).toBe(true);
  });

  it("says nothing it cannot stand behind", () => {
    expect(chipNoticeVisible(false, true, null, "v0.34.9")).toBe(false);
    // Refused, but not (yet) following signatures: "Nothing for you to do"
    // would not be true.
    expect(chipNoticeVisible(true, false, null, "v0.34.9")).toBe(false);
  });
});
