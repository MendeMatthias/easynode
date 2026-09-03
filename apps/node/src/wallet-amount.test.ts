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

describe("fmtExact", () => {
  it("shows the exact amount the node will spend, trailing zeros trimmed", () => {
    expect(fmtExact(1234.564)).toBe("1234.564"); // fmtBtx would round to 1,234.56
    expect(fmtExact(0.12345678)).toBe("0.12345678"); // fmtBtx would round to 0.123457
    expect(fmtExact(1)).toBe("1");
    expect(fmtExact(1.5)).toBe("1.5");
    expect(fmtExact(100)).toBe("100");
  });
});
