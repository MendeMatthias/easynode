import { describe, expect, it, vi } from "vitest";
import {
  History,
  ReportCopy,
  RestartArm,
  ARM_DOUBLE_CLICK_GUARD_MS,
  capForDisplay,
  DISPLAY_LIMIT,
  FAST_FORWARD_ARM_MS,
  FF_NOTE,
  fastForwardView,
} from "./tools-history";

describe("History", () => {
  it("keeps the last 50 and recalls them with up and down", () => {
    const h = new History(50);
    for (let i = 0; i < 60; i++) h.push({ line: `cmd ${i}`, answer: "" });
    expect(h.entries.length).toBe(50);
    expect(h.entries[0].line).toBe("cmd 10");
    expect(h.up()).toBe("cmd 59");
    expect(h.up()).toBe("cmd 58");
    expect(h.down()).toBe("cmd 59");
    expect(h.down()).toBe("");
  });
  it("starts recall again after a new command", () => {
    const h = new History(50);
    h.push({ line: "a", answer: "" });
    h.up();
    h.push({ line: "b", answer: "" });
    expect(h.up()).toBe("b");
  });
  it("copies everything as plain text", () => {
    const h = new History(50);
    h.push({ line: "getblockcount", answer: "233480" });
    h.push({ line: "uptime", answer: "120" });
    expect(h.allText()).toBe("> getblockcount\n233480\n\n> uptime\n120");
  });
});

describe("RestartArm", () => {
  it("arms on the first click and restarts on the second", () => {
    vi.useFakeTimers();
    try {
      const arm = new RestartArm();
      expect(arm.armed).toBe(false);
      expect(arm.click()).toBe(false);
      expect(arm.armed).toBe(true);
      vi.advanceTimersByTime(ARM_DOUBLE_CLICK_GUARD_MS);
      expect(arm.click()).toBe(true);
      expect(arm.armed).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });
  it("disarms by timeout, so the next click only arms again", () => {
    const arm = new RestartArm();
    arm.click();
    arm.disarm(); // the 5s timer fired
    expect(arm.armed).toBe(false);
    expect(arm.click()).toBe(false);
  });
  it("ignores a second click inside the double-click guard, so a double-click cannot both arm and run it", () => {
    vi.useFakeTimers();
    try {
      const arm = new RestartArm();
      expect(arm.click()).toBe(false); // arms
      vi.advanceTimersByTime(ARM_DOUBLE_CLICK_GUARD_MS - 1);
      expect(arm.click()).toBe(false); // still inside the guard: ignored, still armed
      expect(arm.armed).toBe(true);
      vi.advanceTimersByTime(1);
      expect(arm.click()).toBe(true); // a real second click now runs it
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("ReportCopy", () => {
  it("builds on the first click and says so on the button", () => {
    const r = new ReportCopy();
    expect(r.label).toBe("Copy diagnostics");
    expect(r.text).toBeNull(); // nothing to copy yet: this click builds
  });
  it("copies the report already built on the second click", () => {
    const r = new ReportCopy();
    r.ready("easyNode diagnostics, 2026-09-29 14:05 UTC");
    expect(r.label).toBe("Copy report");
    expect(r.text).toBe("easyNode diagnostics, 2026-09-29 14:05 UTC");
    expect(r.text).toBe("easyNode diagnostics, 2026-09-29 14:05 UTC"); // and again
  });
  it("starts over when Tools closes", () => {
    const r = new ReportCopy();
    r.ready("an old report");
    expect(r.text).toBe("an old report");
    r.reset();
    expect(r.label).toBe("Copy diagnostics");
    expect(r.text).toBeNull();
  });
});

describe("capForDisplay", () => {
  it("shows up to 256 KB and says Copy takes the rest", () => {
    expect(DISPLAY_LIMIT).toBe(256 * 1024);
    expect(capForDisplay("short")).toBe("short");
    const long = "x".repeat(DISPLAY_LIMIT + 10);
    const shown = capForDisplay(long);
    expect(shown.startsWith("x".repeat(100))).toBe(true);
    expect(shown).toContain("Copy takes the whole answer");
    expect(shown.length).toBeLessThan(long.length);
  });
});

describe("Fast-forward arm", () => {
  it("gives ten seconds for the second click", () => {
    expect(FAST_FORWARD_ARM_MS).toBe(10_000);
  });
  it("runs only on the second click, and a disarm starts over", () => {
    vi.useFakeTimers();
    try {
      const arm = new RestartArm();
      expect(arm.click()).toBe(false); // shows what will happen
      arm.disarm(); // ten seconds passed
      expect(arm.click()).toBe(false);
      vi.advanceTimersByTime(ARM_DOUBLE_CLICK_GUARD_MS);
      expect(arm.click()).toBe(true); // runs
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("Fast-forward section", () => {
  const offer = {
    kind: "offer" as const,
    height: 233800,
    button: "Fast-forward to block 233,800",
    confirm: "Fast-forward to block 233,800, confirmed by Mende and jpp? ...",
    note: FF_NOTE,
  };
  it("offers the button with the note Rust sent", () => {
    expect(fastForwardView(offer, { running: false, message: null })).toEqual({
      section: true,
      button: "Fast-forward to block 233,800",
      note: FF_NOTE,
    });
    const early =
      "Your node's peers are not sending the older blocks it needs, so Fast-forward is offered sooner than usual. It works the same way as always.";
    expect(fastForwardView({ ...offer, note: early }, null).note).toBe(early);
  });
  it("says it is off, with no button, while the operators disagree", () => {
    const sentence = "Fast-forward is off while the snapshot operators disagree about block 233,800.";
    expect(fastForwardView({ kind: "off", height: 233800, sentence }, null)).toEqual({
      section: true,
      button: null,
      note: sentence,
    });
  });
  it("shows no button during a run, and nothing when there is nothing to say", () => {
    expect(fastForwardView(offer, { running: true, message: "Fast-forward is running." }).button).toBeNull();
    expect(fastForwardView({ kind: "none" }, { running: false, message: null }).section).toBe(false);
    expect(fastForwardView(null, { running: false, message: "Done." }).section).toBe(true);
  });
});
