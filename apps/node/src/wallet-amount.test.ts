// Money-parsing rules for the wallet send form. These guard real BTX spends, so
// they're pinned here rather than trusted. See parseBtxAmount / fmtExact.
import { describe, it, expect } from "vitest";
import { parseBtxAmount, fmtExact } from "./wallet";

describe("parseBtxAmount", () => {
  it("accepts plain decimals and a German decimal comma", () => {
    expect(parseBtxAmount("0.5")).toBe(0.5);
    expect(parseBtxAmount("0,5")).toBe(0.5); // comma == decimal separator
    expect(parseBtxAmount("1234.564")).toBe(1234.564);
    expect(parseBtxAmount("0.12345678")).toBe(0.12345678); // full 8-dp precision
    expect(parseBtxAmount(" 1.5 ")).toBe(1.5); // trimmed
  });

  it("rejects anything that isn't a clean single-number amount", () => {
    for (const bad of ["", "abc", "1.2.3", "1.234,56", "1.5.0", "0x10", "-1", "1e3", "  "]) {
      expect(parseBtxAmount(bad), bad).toBeNaN();
    }
  });

  it("rejects more than 8 decimals so the confirm figure is exact", () => {
    expect(parseBtxAmount("0.123456789")).toBeNaN();
    expect(parseBtxAmount("1.000000009")).toBeNaN();
  });

  it("reads a single comma as a decimal, consistently (confirm screen catches the thousands habit)", () => {
    expect(parseBtxAmount("1,000")).toBe(1); // 1.000, not one thousand
    expect(parseBtxAmount("12,50")).toBe(12.5);
  });
});

describe("fmtExact as the spendable ceiling", () => {
  // The panel prints a ceiling and then compares typed input against the
  // UNROUNDED balance. fmtBtx rounds half-up, so it could print a figure
  // strictly larger than the balance - and a user typing exactly what the
  // wallet showed them was told it was more than they could spend.
  const fmtBtx = (n: number) =>
    n.toLocaleString("en-US", { maximumFractionDigits: n >= 1000 ? 2 : 6 });

  // `spendable` is `d.trusted` straight off the node - never computed here -
  // so it always has at most 8 decimals and toFixed(8) reproduces it exactly.
  const NODE_BALANCES = [0.1234565, 0.9999999, 1234.565, 0.12345678, 8, 1000.005];

  it("never advertises more than the balance, where the rounding one does", () => {
    for (const bal of NODE_BALANCES) {
      expect(Number(fmtExact(bal)), `fmtExact(${bal})`).toBeLessThanOrEqual(bal);
    }
    // The concrete failure this replaced: half-up puts the printed ceiling
    // ABOVE the balance, and the guard compares against the unrounded number,
    // so typing the figure the wallet just printed was rejected as an overspend.
    for (const bal of [0.1234565, 1000.005, 1234.565, 0.9999999]) {
      // fmtBtx groups thousands, so strip separators before comparing values.
      const shown = Number(fmtBtx(bal).replace(/,/g, ""));
      expect(shown, `fmtBtx(${bal})`).toBeGreaterThan(bal);
    }
  });

  it("prints exactly what the Max button fills in", () => {
    // Max does `spendable.toFixed(8)`. If the ceiling text and Max disagree,
    // one of them is wrong and the user cannot tell which.
    for (const bal of NODE_BALANCES) {
      expect(parseBtxAmount(bal.toFixed(8)), `${bal}`).toBe(Number(fmtExact(bal)));
    }
  });

  it("round-trips through the form's own parser, so the printed max is typable", () => {
    // The Max button and the ceiling must agree with what parseBtxAmount will
    // accept, or the wallet prints a number its own form rejects.
    for (const bal of [0.12345678, 1234.564, 1, 100]) {
      expect(parseBtxAmount(fmtExact(bal)), fmtExact(bal)).toBe(bal);
    }
  });

  it("shows the exact amount the node will spend, trailing zeros trimmed", () => {
    expect(fmtExact(1234.564)).toBe("1234.564"); // fmtBtx would round to 1,234.56
    expect(fmtExact(0.12345678)).toBe("0.12345678"); // fmtBtx would round to 0.123457
    expect(fmtExact(1)).toBe("1");
    expect(fmtExact(1.5)).toBe("1.5");
    expect(fmtExact(100)).toBe("100");
  });
});
