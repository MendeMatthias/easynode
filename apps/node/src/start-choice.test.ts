import { describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import {
  ANNOUNCE_HOLD_MS,
  NO_GPU_REASON,
  announcer,
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

describe("the setup screen and the chip notice for a screen reader", () => {
  const tagOf = (id: string): string => {
    const m = INDEX.match(new RegExp(`<[^>]*\\bid="${id}"[^>]*>`));
    if (!m) throw new Error(`no #${id} in index.html`);
    return m[0];
  };

  it("reads the line under the choices as the choice's description", () => {
    expect(tagOf("start-choice")).toContain('role="radiogroup"');
    expect(tagOf("start-choice")).toContain('aria-describedby="start-choice-note"');
  });

  it("announces the chip notice through a live region that is always there", () => {
    // A region that appears together with its text may not be announced
    // (VoiceOver on WebKit), so the one that speaks is present from the
    // start, outside both screens, visually hidden but in the tree.
    const announce = tagOf("chip-announce");
    expect(announce).toContain('role="status"');
    expect(announce).toContain('class="visually-hidden"');
    expect(announce).not.toMatch(/\bhidden\b(?!")/);
    expect(INDEX.indexOf('id="chip-announce"')).toBeLessThan(INDEX.indexOf('id="screen-wizard"'));
    const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
    const rule = css.match(/\.visually-hidden\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(rule).toContain("clip");
    expect(rule).not.toMatch(/display:\s*none|visibility:\s*hidden/);
    // The card shows and hides; it is not a live region itself, so the
    // sentence is not said twice.
    expect(tagOf("chip-card")).not.toContain('role="status"');
  });
});

describe("announcer", () => {
  // The hidden live region says the chip notice, then lets it go: the card
  // still shows the sentence, and a screen reader reading the page should
  // meet it once.
  it("holds the text a few seconds, then clears it", () => {
    vi.useFakeTimers();
    try {
      const region = { textContent: "" as string | null };
      const say = announcer(region);
      say("This Mac's graphics chip didn't pass the engine's check.");
      expect(region.textContent).toBe("This Mac's graphics chip didn't pass the engine's check.");
      vi.advanceTimersByTime(ANNOUNCE_HOLD_MS - 1);
      expect(region.textContent).toBe("This Mac's graphics chip didn't pass the engine's check.");
      vi.advanceTimersByTime(1);
      expect(region.textContent).toBe("");
      expect(ANNOUNCE_HOLD_MS).toBeGreaterThanOrEqual(3_000);
      expect(ANNOUNCE_HOLD_MS).toBeLessThanOrEqual(10_000);
    } finally {
      vi.useRealTimers();
    }
  });

  it("clears at once on empty text, and a new text restarts the hold", () => {
    vi.useFakeTimers();
    try {
      const region = { textContent: "" as string | null };
      const say = announcer(region, 1_000);
      say("first");
      say("");
      expect(region.textContent).toBe("");
      say("second");
      vi.advanceTimersByTime(900);
      say("third");
      vi.advanceTimersByTime(900);
      expect(region.textContent).toBe("third");
      vi.advanceTimersByTime(100);
      expect(region.textContent).toBe("");
    } finally {
      vi.useRealTimers();
    }
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
