import { describe, expect, it } from "vitest";
import { classifyAskQuery } from "./ask";

describe("classifyAskQuery", () => {
  it("routes digits to a height lookup", () => {
    expect(classifyAskQuery("155700")).toEqual({ kind: "height" });
    expect(classifyAskQuery(" 155,700 ")).toEqual({ kind: "height" });
  });
  it("routes 64-hex to hash-or-txid", () => {
    expect(classifyAskQuery("a".repeat(64))).toEqual({ kind: "hash_or_txid" });
    expect(classifyAskQuery("A0".repeat(32))).toEqual({ kind: "hash_or_txid" });
  });
  it("flags empty and invalid input", () => {
    expect(classifyAskQuery("")).toEqual({ kind: "empty" });
    expect(classifyAskQuery("   ")).toEqual({ kind: "empty" });
    expect(classifyAskQuery("btx1zsomeaddress")).toEqual({ kind: "invalid" });
    expect(classifyAskQuery("zz".repeat(32))).toEqual({ kind: "invalid" });
  });
});
