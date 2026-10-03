import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { chainAgreementView, type ChainAgreementRow } from "./chain-agreement";

const INDEX = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const CSS = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

const row = (over: Partial<ChainAgreementRow> = {}): ChainAgreementRow => ({
  status: "ok",
  word: "AGREE",
  headline: "Nothing wrong seen. This is not a guarantee.",
  scope: "easyBTX/btxscan and Byron Bay agree with your node at block 240,000",
  meaning: null,
  as_of: "as of 14:05 UTC",
  ...over,
});

describe("chainAgreementView", () => {
  it("is hidden while the node is not running or before the first comparison", () => {
    expect(chainAgreementView(row(), false)).toBeNull();
    expect(chainAgreementView(null, true)).toBeNull();
  });

  it("shows the word, the scope with its time, and the calm line", () => {
    expect(chainAgreementView(row(), true)).toEqual({
      tone: "is-ok",
      word: "AGREE",
      scope: "easyBTX/btxscan and Byron Bay agree with your node at block 240,000 · as of 14:05 UTC",
      lines: ["Nothing wrong seen. This is not a guarantee."],
    });
  });

  it("a disagreement leads with what differs, then what it means", () => {
    const v = chainAgreementView(
      row({
        status: "warning",
        word: "DISAGREE",
        headline: "Byron Bay explorer has a different block at height 240,000 than your node.",
        meaning: "Byron Bay explorer is on a different branch than your node and easyBTX/btxscan.",
        scope: "easyBTX/btxscan agrees with your node at block 240,000",
      }),
      true,
    );
    expect(v?.tone).toBe("is-warning");
    expect(v?.lines).toEqual([
      "Byron Bay explorer has a different block at height 240,000 than your node.",
      "Byron Bay explorer is on a different branch than your node and easyBTX/btxscan.",
    ]);
  });

  it("not enough sources, a stale comparison or an unknown status is grey, never green", () => {
    expect(chainAgreementView(row({ status: "unknown", word: "NOT ENOUGH SOURCES" }), true)?.tone).toBe(
      "is-unknown",
    );
    expect(chainAgreementView(row({ status: "unknown", word: "NOT RUN" }), true)?.tone).toBe("is-unknown");
    expect(chainAgreementView(row({ status: "something new" }), true)?.tone).toBe("is-unknown");
    expect(chainAgreementView(row({ status: "caution", word: "BEHIND" }), true)?.tone).toBe("is-caution");
  });
});

describe("the Same chain card", () => {
  it("exists, hidden until there is something to show, with the word as text", () => {
    expect(INDEX).toMatch(/<div class="card agree-card" id="agree-card" hidden>/);
    expect(INDEX).toContain('id="agree-word"');
    expect(INDEX).toContain("Same chain as other sources");
  });

  it("colours only the row status, and grey has its own rule", () => {
    for (const tone of ["is-ok", "is-caution", "is-warning", "is-unknown"]) {
      expect(CSS).toContain(`.agree-card.${tone}`);
    }
  });
});
