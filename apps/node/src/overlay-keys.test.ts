import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { CLOSE_BUTTONS, DECISIONS, escapeTarget, wrapFocus } from "./overlay-keys";

const INDEX = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const CSS = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

describe("escapeTarget", () => {
  it("is nothing when no panel is open", () => {
    expect(escapeTarget([])).toBeNull();
  });

  it("is the close button of the one open panel", () => {
    expect(escapeTarget(["settings-overlay"])).toBe("settings-close");
    expect(escapeTarget(["ask-overlay"])).toBe("ask-close");
  });

  it("closes the panel on top, which is the later one on the page", () => {
    // Every panel has the same z-index, so the later one paints on top.
    expect(escapeTarget(["info-overlay", "future-overlay"])).toBe("future-close");
  });

  it("leaves a decision alone: the close dialog and the welcome panel need an answer", () => {
    expect(escapeTarget(["close-overlay"])).toBeNull();
    expect(escapeTarget(["welcome-overlay"])).toBeNull();
    // The close dialog sits above every panel (styles.css), so with it open
    // Escape must not close the panel underneath it either.
    expect(escapeTarget(["close-overlay", "tools-overlay"])).toBeNull();
  });

  it("ignores a panel it does not know", () => {
    expect(escapeTarget(["some-new-overlay"])).toBeNull();
  });
});

describe("wrapFocus", () => {
  it("goes from the last control to the first on Tab, and back on Shift+Tab", () => {
    expect(wrapFocus(4, 5, false)).toBe(0);
    expect(wrapFocus(0, 5, true)).toBe(4);
  });

  it("lets the browser move focus anywhere else", () => {
    expect(wrapFocus(2, 5, false)).toBeNull();
    expect(wrapFocus(2, 5, true)).toBeNull();
  });

  it("pulls focus back in from outside the panel", () => {
    expect(wrapFocus(-1, 5, false)).toBe(0);
    expect(wrapFocus(-1, 5, true)).toBe(4);
  });

  it("does nothing in a panel with nothing to focus", () => {
    expect(wrapFocus(-1, 0, false)).toBeNull();
  });
});

describe("the page these act on", () => {
  it("has every close button escapeTarget names, inside its own panel", () => {
    for (const [overlay, button] of Object.entries(CLOSE_BUTTONS)) {
      const start = INDEX.indexOf(`id="${overlay}"`);
      expect(start, overlay).toBeGreaterThan(-1);
      const next = INDEX.indexOf('class="overlay"', start + 1);
      const panel = INDEX.slice(start, next === -1 ? undefined : next);
      expect(panel, overlay).toContain(`id="${button}"`);
    }
  });

  it("knows every overlay on the page, as closable or as a decision", () => {
    const ids = [...INDEX.matchAll(/class="overlay" id="([^"]+)"/g)].map((m) => m[1]);
    expect(ids.length).toBeGreaterThan(0);
    for (const id of ids) {
      expect(id in CLOSE_BUTTONS || DECISIONS.includes(id), id).toBe(true);
    }
  });

  it("paints the close dialog above every other panel", () => {
    // It opens over whatever is open when the window's close button is
    // pressed; at the same z-index it painted behind Ask, Tools and What's
    // next, which come later in the page, and the click seemed to do nothing.
    const base = Number(/\.overlay \{[^}]*z-index:\s*(\d+)/.exec(CSS)?.[1]);
    const close = Number(/#close-overlay \{[^}]*z-index:\s*(\d+)/.exec(CSS)?.[1]);
    expect(base).toBeGreaterThan(0);
    expect(close).toBeGreaterThan(base);
  });
});
