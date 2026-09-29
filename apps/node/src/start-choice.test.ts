import { describe, expect, it } from "vitest";
import { NO_GPU_REASON, chipNoticeVisible, setupArgs, startChoiceView } from "./start-choice";

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
