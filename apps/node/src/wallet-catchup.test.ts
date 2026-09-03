// The catch-up estimate a user reads while their balance is wrong. It must
// always be a RANGE: the real duration moves by about a factor of two purely on
// peer quality, and the previous version printed the slow end alone as if it
// were the expected case. See CATCHUP_SLOW_PER_HOUR in wallet.ts for where the
// two ends come from and why the RTX 5080 figure must not be reused here.
import { describe, it, expect } from "vitest";
import {
  catchupEta,
  CATCHUP_FAST_PER_HOUR,
  CATCHUP_SLOW_PER_HOUR,
  CATCHUP_APPLE_SILICON_SANITY_CEILING,
} from "./wallet";

describe("catchupEta", () => {
  it("gives a range, never a single number, for a normal gap", () => {
    // 5,300 behind was the real user situation this was written for.
    const s = catchupEta(5300);
    expect(s).toMatch(/roughly \d+ to \d+ days/);
    // 5300/113 = 47h ~ 2d, 5300/58 = 92h ~ 4d.
    expect(s).toBe("roughly 2 to 4 days");
  });

  it("uses hours while hours are still readable", () => {
    // 600/113 = 6h, 600/58 = 11h.
    expect(catchupEta(600)).toBe("roughly 6 to 11 hours");
  });

  it("never prints a degenerate range like 1 to 1", () => {
    // A small gap rounds both ends to the same hour. Show one figure instead.
    for (const n of [51, 60, 80, 100, 113]) {
      const s = catchupEta(n);
      expect(s).not.toMatch(/(\d+) to \1\b/);
    }
    expect(catchupEta(51)).toBe("about 1 hour");
  });

  it("always keeps the slow end at or above the fast end", () => {
    // The bug this guards: independent rounding of the two ends can invert them
    // at a boundary, and "roughly 4 to 2 days" is worse than saying nothing.
    for (let n = 51; n < 40000; n += 137) {
      const s = catchupEta(n);
      const m = s.match(/roughly (\d+) to (\d+)/);
      if (m) expect(Number(m[2])).toBeGreaterThan(Number(m[1]));
    }
  });

  it("does not claim a speed Apple Silicon cannot reach", () => {
    // Assert the CONSTANT, not a phrasing.
    //
    // This test used to read `expect(catchupEta(5300)).not.toMatch(/hours/)`
    // and it was vacuous. Setting the fast end to the RTX 5080's 775 blocks/h
    // makes the panel say "roughly 1 to 4 days", which contains no "hours" for
    // that check to catch, so the guard passed with the bug present. Verified
    // by running it both ways rather than by reading it.
    //
    // The lesson, which is the recurring bug in this codebase: a guard that
    // cannot fail is not a guard. Assert the thing you care about.
    expect(CATCHUP_FAST_PER_HOUR).toBeLessThanOrEqual(CATCHUP_APPLE_SILICON_SANITY_CEILING);
    expect(CATCHUP_FAST_PER_HOUR).toBeGreaterThan(CATCHUP_SLOW_PER_HOUR);
  });

  it("keeps the fast end slower than the slow end is fast", () => {
    // The behavioural half of the same guard: a 5,300 block gap must never
    // collapse to hours, and its fast end must stay at 2 days or more. With
    // 775 the fast end drops to 1 day and this fails.
    const s = catchupEta(5300);
    expect(s).not.toMatch(/hours/);
    const m = s.match(/roughly (\d+) to (\d+) days/);
    expect(m).not.toBeNull();
    expect(Number(m![1])).toBeGreaterThanOrEqual(2);
  });

  it("handles the degenerate inputs without producing nonsense", () => {
    expect(catchupEta(0)).toBe("about 1 hour");
    expect(catchupEta(1)).toBe("about 1 hour");
  });
});
