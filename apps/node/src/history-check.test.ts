import { describe, expect, it } from "vitest";
import { historyCheckView } from "./history-check";

describe("historyCheckView", () => {
  it("reads as the decision wrote it", () => {
    expect(historyCheckView({ checked: 131_200, base: 225_927 })).toEqual({
      line: "Checking older history: 131,200 of 225,927 (58%)",
      pct: 58,
    });
  });

  it("rounds down, so the bar never reads 100% before the engine says done", () => {
    expect(historyCheckView({ checked: 225_926, base: 225_927 })?.pct).toBe(99);
    expect(historyCheckView({ checked: 0, base: 225_927 })?.line).toBe(
      "Checking older history: 0 of 225,927 (0%)",
    );
  });

  it("never counts past the base", () => {
    expect(historyCheckView({ checked: 230_000, base: 225_927 })).toEqual({
      line: "Checking older history: 225,927 of 225,927 (99%)",
      pct: 99,
    });
  });

  it("never reads 100% while the check is still reported, even at the base", () => {
    // The field goes away only when the engine reports the check done, so
    // while it is here the check is not done, whatever the numbers say.
    expect(historyCheckView({ checked: 225_927, base: 225_927 })).toEqual({
      line: "Checking older history: 225,927 of 225,927 (99%)",
      pct: 99,
    });
    expect(historyCheckView({ checked: 1, base: 1 })?.pct).toBe(99);
  });

  it("shows nothing without a check", () => {
    expect(historyCheckView(null)).toBeNull();
    expect(historyCheckView({ checked: 5, base: 0 })).toBeNull();
    expect(historyCheckView({ checked: Number.NaN, base: 225_927 })).toBeNull();
  });
});
