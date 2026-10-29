import { describe, expect, it } from "vitest";
import { EXPLORER_COPY, explorerRow } from "./feature-rows";

// The owner's rule of 2026-10-12: every feature is on, one a machine cannot
// run simply does not run there (its switch greyed, with the reason), and
// anyone can switch any of them off.
describe("explorerRow", () => {
  it("is on, and switchable, on a node that keeps the whole chain", () => {
    expect(explorerRow({ txindex_enabled: true, explorer_available: true })).toEqual({
      checked: true,
      disabled: false,
      desc: EXPLORER_COPY,
      calm: false,
    });
  });

  it("can be switched off, and says the same thing off", () => {
    expect(explorerRow({ txindex_enabled: false, explorer_available: true })).toMatchObject({
      checked: false,
      disabled: false,
      desc: EXPLORER_COPY,
    });
  });

  it("is greyed out with the reason on a node that prunes, even when the setting is on", () => {
    // BTX refuses to start with pruning and a transaction index together, so
    // the switch must not look like it does anything there.
    expect(explorerRow({ txindex_enabled: true, explorer_available: false })).toEqual({
      checked: false,
      disabled: true,
      desc: "Not on this node: Explorer mode needs the whole chain, and this node keeps only the newest blocks.",
      calm: true,
    });
  });
});
