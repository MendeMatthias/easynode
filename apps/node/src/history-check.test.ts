import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { historyCheckView, type HistoryCheck } from "./history-check";

const INDEX = readFileSync(new URL("../index.html", import.meta.url), "utf8");
/** An element's opening tag in index.html. */
const tagOf = (id: string): string => {
  const m = INDEX.match(new RegExp(`<[^>]*\\bid="${id}"[^>]*>`));
  if (!m) throw new Error(`no #${id} in index.html`);
  return m[0];
};

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

  it("shows nothing when the object is missing a key", () => {
    // The type promises both keys, but an older or mismatched backend could
    // send a `history_check` object without one of them. Missing `base` is
    // `undefined`, which fails `h.base > 0`; missing `checked` is
    // `undefined`, which fails `Number.isFinite`. Either way the guard falls
    // back to the same "no check" result as a null or an out-of-range value.
    expect(historyCheckView({ base: 225_927 } as HistoryCheck)).toBeNull();
    expect(historyCheckView({ checked: 131_200 } as HistoryCheck)).toBeNull();
  });

  // The confirmed-snapshot decision, section 7: the line gains a second
  // sentence, on a line of its own, saying where the node started.
  it("says where the node started, as the decision wrote it", () => {
    expect(
      historyCheckView(
        { checked: 131_200, base: 233_800 },
        "Started from block 233,800, confirmed by Mende and jpp.",
      ),
    ).toEqual({
      line: "Checking older history: 131,200 of 233,800 (56%).\nStarted from block 233,800, confirmed by Mende and jpp.",
      pct: 56,
    });
    expect(
      historyCheckView({ checked: 131_200, base: 225_927 }, "Started from block 225,927, built into this app.")
        ?.line,
    ).toBe("Checking older history: 131,200 of 225,927 (58%).\nStarted from block 225,927, built into this app.");
  });

  it("goes with the check, whatever the start point", () => {
    expect(historyCheckView(null, "Started from block 233,800, confirmed by Mende and jpp.")).toBeNull();
    expect(historyCheckView({ checked: 131_200, base: 225_927 }, null)?.line).toBe(
      "Checking older history: 131,200 of 225,927 (58%)",
    );
    expect(historyCheckView({ checked: 131_200, base: 225_927 }, "  ")?.line).toBe(
      "Checking older history: 131,200 of 225,927 (58%)",
    );
  });
});

describe("the history line for a screen reader", () => {
  it("is heard once, as the bar's name, with the bar's value after it", () => {
    expect(tagOf("history-bar")).toContain('role="progressbar"');
    expect(tagOf("history-bar")).toContain('aria-labelledby="history-line"');
    // Without this the line is read twice: as text, then as the bar's name.
    expect(tagOf("history-line")).toContain('aria-hidden="true"');
  });
});

describe("the history line's second sentence", () => {
  it("sits on a line of its own", () => {
    const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
    expect(css).toMatch(/\.history-line\s*\{[^}]*white-space:\s*pre-line/);
  });
});
