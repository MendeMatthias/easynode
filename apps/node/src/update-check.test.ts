// The update check tells a user why it failed, and it got that wrong for every
// Mac and Windows copy the day 0.6.20 shipped Linux-only. These pin the two
// error strings tauri-plugin-updater 2.11 actually produces, so a plugin
// upgrade that rewords them fails here instead of in front of a user.
import { describe, it, expect } from "vitest";
import { classifyCheckFailure, checkFailureMessage } from "./update-check";

// Verbatim from tauri-plugin-updater 2.11.0 src/error.rs.
const TARGET_NOT_FOUND =
  "the platform `darwin-aarch64` was not found in the response `platforms` object";
const TARGETS_NOT_FOUND =
  'None of the fallback platforms `["darwin-aarch64", "darwin-x86_64"]` were found ' +
  "in the response `platforms` object";

describe("classifyCheckFailure", () => {
  it("recognises both of the plugin's missing-platform errors", () => {
    expect(classifyCheckFailure(TARGET_NOT_FOUND).kind).toBe("no-build-for-this-platform");
    expect(classifyCheckFailure(TARGETS_NOT_FOUND).kind).toBe("no-build-for-this-platform");
    expect(classifyCheckFailure(new Error(TARGET_NOT_FOUND)).kind).toBe(
      "no-build-for-this-platform",
    );
  });

  it("leaves a real network failure alone", () => {
    for (const e of [
      "error sending request for url (https://easybtx.com/updater/latest-node.json)",
      new Error("dns error: failed to lookup address information"),
      "operation timed out",
      "signature verification failed",
    ]) {
      expect(classifyCheckFailure(e).kind).toBe("unknown");
    }
  });
});

describe("checkFailureMessage", () => {
  it("never blames the network when the release simply omits this platform", () => {
    const m = checkFailureMessage(
      classifyCheckFailure(TARGET_NOT_FOUND), "0.6.19", "easybtx.com/node",
    );
    expect(m).not.toMatch(/online/i);
    expect(m).not.toMatch(/couldn't check/i);
    expect(m).toContain("no build for this platform");
    expect(m).toContain("v0.6.19");
    expect(m).toContain("easybtx.com/node");
  });

  it("still says it plainly when the version is not known yet", () => {
    // appVersion is read from the backend and the check can win that race.
    const m = checkFailureMessage(
      { kind: "no-build-for-this-platform" }, "", "easybtx.com/node",
    );
    expect(m).toContain("stays where it is");
    expect(m).not.toContain("v.");
    expect(m).not.toMatch(/undefined|NaN/);
  });

  it("keeps the network wording for a network failure", () => {
    const m = checkFailureMessage(
      classifyCheckFailure("dns error"), "0.6.20", "easybtx.com/node",
    );
    expect(m).toMatch(/are you online/i);
    expect(m).toContain("dns error");
  });

  it("truncates a runaway error rather than pasting it into the UI", () => {
    const m = checkFailureMessage(classifyCheckFailure("x".repeat(500)), "0.6.20", "e.com");
    expect(m.length).toBeLessThan(200);
  });
});
