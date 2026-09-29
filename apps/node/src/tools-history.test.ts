import { describe, expect, it } from "vitest";
import { History, RestartArm, capForDisplay, DISPLAY_LIMIT } from "./tools-history";

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
    const arm = new RestartArm();
    expect(arm.armed).toBe(false);
    expect(arm.click()).toBe(false);
    expect(arm.armed).toBe(true);
    expect(arm.click()).toBe(true);
    expect(arm.armed).toBe(false);
  });
  it("disarms by timeout, so the next click only arms again", () => {
    const arm = new RestartArm();
    arm.click();
    arm.disarm(); // the 5s timer fired
    expect(arm.armed).toBe(false);
    expect(arm.click()).toBe(false);
  });
  it("disarms when the overlay closes, so reopening starts over", () => {
    const arm = new RestartArm();
    arm.click();
    arm.disarm(); // the overlay closed
    expect(arm.armed).toBe(false);
    expect(arm.click()).toBe(false);
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
