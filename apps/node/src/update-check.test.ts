// The update check tells a user why it failed, and it got that wrong for every
// Mac and Windows copy the day 0.6.20 shipped Linux-only. These pin the two
// error strings tauri-plugin-updater 2.11 actually produces, so a plugin
// upgrade that rewords them fails here instead of in front of a user.
//
// The second half is about the check saying what it did at all. Measured
// 2026-09-15: this project's own signer box ran 0.6.21 for eight hours after
// the feed served 0.6.22, two six-hourly checks fell due, and nothing on the
// machine could say whether they ran, failed, or found nothing. These pin the
// outcome vocabulary closed on both sides of the app, and read main.ts to make
// sure no exit of updateCheck() is silent again.
//
// The last part is about WHERE the six-hourly check runs. The hypothesis for
// those eight hours is a webview timer that never fired in a hidden window, so
// the recheck moved to a tokio timer in src-tauri/src/update_timer.rs. These
// read main.ts and that file to pin that there is one periodic path, that it
// speaks the same five words, and that its results are painted by the same
// functions the JavaScript check paints with.
import { readFileSync } from "node:fs";
import { describe, it, expect } from "vitest";
import {
  classifyCheckFailure,
  checkFailureMessage,
  describeWhen,
  handInstallBanner,
  handInstallNotice,
  HAND_INSTALL_MARK,
  installErrorFromDetail,
  lastCheckLine,
  noUpdateMessage,
  plainOutcome,
  updateCheckRecord,
  UPDATE_CHECK_OUTCOMES,
  type UpdateCheckBranch,
  type UpdateCheckOutcome,
} from "./update-check";

// Normalise line endings: a Windows checkout with autocrlf hands these files
// over as CRLF, and every slice below looks for "\n}\n". Measured on the
// node-win-installer run of 2026-09-15: two of these tests failed there and
// nowhere else, for exactly that reason.
const read = (rel: string) =>
  readFileSync(new URL(rel, import.meta.url), "utf8").replace(/\r\n/g, "\n");

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

// ── The outcome is written down, in the same five words on both sides ───────

describe("the outcome vocabulary", () => {
  it("is the same list in TypeScript and in Rust", () => {
    // The Rust side refuses anything outside its list unwritten, so a word
    // added on one side only would be a check that logs nothing. Read the
    // constant out of update_log.rs rather than restating it here.
    const src = read("../src-tauri/src/update_log.rs");
    const m = /UPDATE_CHECK_OUTCOMES: \[&str; (\d+)\] =\s*\[([^\]]*)\]/.exec(src);
    expect(m, "UPDATE_CHECK_OUTCOMES in update_log.rs").not.toBeNull();
    const rust = [...m![2].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
    expect(rust.length).toBe(Number(m![1]));
    expect([...UPDATE_CHECK_OUTCOMES]).toEqual(rust);
  });

  const every: UpdateCheckBranch[] = [
    { branch: "check-failed", failure: { kind: "unknown", detail: "dns error" } },
    { branch: "check-failed", failure: { kind: "no-build-for-this-platform" } },
    { branch: "no-update", currentVersion: "0.6.22" },
    { branch: "no-update", currentVersion: "" },
    { branch: "found", version: "0.6.23" },
    { branch: "install-failed", version: "0.6.23", error: new Error("deb: not supported") },
    { branch: "installed", version: "0.6.23" },
    { branch: "relaunch-failed", version: "0.6.23", error: "spawn failed" },
  ];

  it("is closed: every branch maps to one of the words, and every word is reachable", () => {
    const seen = new Set<UpdateCheckOutcome>();
    for (const b of every) {
      for (const t of ["manual", "automatic"] as const) {
        const rec = updateCheckRecord(b, t);
        expect(UPDATE_CHECK_OUTCOMES).toContain(rec.outcome);
        expect(rec.detail.startsWith(t), rec.detail).toBe(true);
        seen.add(rec.outcome);
      }
    }
    // A word nothing produces is a lie in the list.
    expect([...seen].sort()).toEqual([...UPDATE_CHECK_OUTCOMES].sort());
  });

  it("maps each exit to the expected word", () => {
    const word = (b: UpdateCheckBranch) => updateCheckRecord(b, "automatic").outcome;
    expect(word({ branch: "check-failed", failure: classifyCheckFailure("dns") })).toBe(
      "check-failed",
    );
    expect(word({ branch: "no-update", currentVersion: "0.6.22" })).toBe("no-update");
    expect(word({ branch: "found", version: "0.6.23" })).toBe("found");
    expect(word({ branch: "install-failed", version: "0.6.23", error: "x" })).toBe(
      "install-failed",
    );
    expect(word({ branch: "installed", version: "0.6.23" })).toBe("installed");
    // A restart that failed is still an installed update; the detail says so.
    const r = updateCheckRecord(
      { branch: "relaunch-failed", version: "0.6.23", error: "spawn failed" },
      "automatic",
    );
    expect(r.outcome).toBe("installed");
    expect(r.detail).toContain("restart failed");
    expect(r.detail).toContain("spawn failed");
  });

  it("keeps the detail short, on one line, and says whether anyone pressed the button", () => {
    const long = "x".repeat(1000) + "\n\nsecond line";
    for (const b of [
      { branch: "check-failed", failure: classifyCheckFailure(long) },
      { branch: "install-failed", version: "0.6.23", error: long },
      { branch: "relaunch-failed", version: "0.6.23", error: long },
    ] as UpdateCheckBranch[]) {
      const d = updateCheckRecord(b, "manual").detail;
      expect(d.length).toBeLessThan(260);
      expect(d).not.toMatch(/\n/);
      expect(d.startsWith("manual: ")).toBe(true);
    }
    // The platform case names the reason, never the plugin's sentence.
    const noBuild = updateCheckRecord(
      { branch: "check-failed", failure: classifyCheckFailure(TARGET_NOT_FOUND) },
      "automatic",
    );
    expect(noBuild.detail).toBe("automatic: no build for this platform");
    // The version offered rides along where there is one.
    expect(
      updateCheckRecord({ branch: "found", version: "0.6.23" }, "automatic").detail,
    ).toContain("v0.6.23");
    expect(
      updateCheckRecord({ branch: "no-update", currentVersion: "0.6.22" }, "manual").detail,
    ).toBe("manual: v0.6.22 is current");
    expect(
      updateCheckRecord({ branch: "no-update", currentVersion: "" }, "automatic").detail,
    ).toBe("automatic");
  });
});

// ── No exit of updateCheck() is silent ───────────────────────────────────────
// Read from main.ts, the way the vocabulary test reads update_log.rs. The
// function is a thin Tauri veneer that cannot run under vitest, but its shape
// can be pinned: every `return` sits right after a record, every branch of the
// union is named, and the one record that precedes the process ending is
// awaited.

describe("every exit of updateCheck() records an outcome", () => {
  const main = read("./main.ts");
  const start = main.indexOf("async function updateCheck(");
  const end = main.indexOf("\n}\n", start);
  const body = main.slice(start, end);
  const lines = body.split("\n");
  const isBlankOrComment = (l: string) => /^\s*(\/\/.*)?$/.test(l);
  const statementAbove = (i: number) => {
    let j = i - 1;
    while (j >= 0 && isBlankOrComment(lines[j])) j--;
    return lines[j];
  };

  it("is where the test expects it", () => {
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    expect(lines.length).toBeGreaterThan(20);
  });

  it("records right before every return", () => {
    const returns = lines
      .map((l, i) => [l, i] as const)
      .filter(([l]) => /^\s*return;?\s*$/.test(l));
    // check failed, no update, install failed.
    expect(returns.length).toBeGreaterThanOrEqual(3);
    for (const [, i] of returns) {
      expect(statementAbove(i), `the statement before the return on line ${i}`).toContain(
        "recordUpdateCheck(",
      );
    }
  });

  it("names every branch of the union, including the two that do not return", () => {
    for (const b of [
      "check-failed",
      "no-update",
      "found",
      "install-failed",
      "installed",
      "relaunch-failed",
    ]) {
      expect(body, b).toContain(`branch: "${b}"`);
    }
  });

  it("waits, with a bound, for the installed record before ending the process", () => {
    const i = lines.findIndex((l) => l.includes('branch: "installed"'));
    expect(i).toBeGreaterThan(-1);
    // Not fire-and-forget like the others: raced against a timeout, awaited.
    expect(lines[i]).not.toMatch(/^\s*void /);
    expect(statementAbove(i)).toMatch(/await Promise\.race\(\[/);
    expect(body.indexOf("await relaunch()")).toBeGreaterThan(body.indexOf('branch: "installed"'));
  });

  it("never lets recording change the flow", () => {
    // No bare await on the recorder: either voided or raced with a timeout.
    expect(body).not.toMatch(/^\s*await recordUpdateCheck\(/m);
    // And the recorder itself turns a rejection into a console warning.
    const recStart = main.indexOf("function recordUpdateCheck(");
    expect(recStart).toBeGreaterThan(-1);
    const rec = main.slice(recStart, main.indexOf("\n}\n", recStart));
    expect(rec).toContain("console.warn");
    expect(rec).toContain('invoke("record_update_check"');
  });
});

// ── The permanent line under the button ──────────────────────────────────────

describe("the Last check line", () => {
  // Local time throughout, like the line itself: 15 Sep 2026, 16:30.
  const now = new Date(2026, 8, 15, 16, 30);
  const at1403 = new Date(2026, 8, 15, 14, 3).toISOString();
  const line = (outcome: string, detail: string) =>
    lastCheckLine({ at: at1403, outcome, detail }, now, "easybtx.com/node");

  it("says none yet before the first check has finished", () => {
    expect(lastCheckLine({ at: null, outcome: null, detail: "" }, now, "e")).toBe(
      "Last check: none yet",
    );
  });

  it("renders each outcome in plain words", () => {
    expect(line("no-update", "automatic: v0.6.22 is current")).toBe(
      "Last check: today 14:03 — you're on the latest version",
    );
    expect(line("check-failed", "automatic: dns error")).toBe(
      "Last check: today 14:03 — couldn't check",
    );
    expect(line("check-failed", "automatic: no build for this platform")).toBe(
      "Last check: today 14:03 — no build for this platform yet",
    );
    expect(line("found", "automatic: v0.6.23 offered, downloading")).toBe(
      "Last check: today 14:03 — found v0.6.23",
    );
    expect(line("install-failed", "automatic: v0.6.23: deb is not supported")).toBe(
      "Last check: today 14:03 — v0.6.23 couldn't install — get it from easybtx.com/node",
    );
    expect(line("installed", "automatic: v0.6.23, restarting")).toBe(
      "Last check: today 14:03 — v0.6.23 installed",
    );
  });

  it("never leaks a log word or the trigger into the sentence", () => {
    for (const o of UPDATE_CHECK_OUTCOMES) {
      const l = line(o, "automatic: v0.6.23 offered, downloading");
      expect(l).not.toContain("automatic");
      // "found" and "installed" are English; the hyphenated log words are not.
      expect(l).not.toMatch(/no-update|check-failed|install-failed/);
    }
  });

  it("does not invent a version it was not given", () => {
    expect(plainOutcome("installed", "automatic", "e")).toBe("an update installed");
    expect(plainOutcome("found", "manual", "e")).toBe("found an update");
    expect(plainOutcome("install-failed", "manual: deb", "e.com")).toBe(
      "an update couldn't install — get it from e.com",
    );
  });

  it("shows a word this build does not know rather than hiding it", () => {
    expect(plainOutcome("skipped", "", "e")).toBe("skipped");
  });

  it("tells the time the way a person at the machine would", () => {
    expect(describeWhen(new Date(2026, 8, 15, 9, 5), now)).toBe("today 09:05");
    expect(describeWhen(new Date(2026, 8, 14, 23, 59), now)).toBe("yesterday 23:59");
    expect(describeWhen(new Date(2026, 8, 12, 14, 3), now)).toBe("12 Sep 14:03");
    expect(describeWhen(new Date(2025, 0, 3, 8, 0), now)).toBe("3 Jan 2025 08:00");
    // Yesterday across a month boundary.
    expect(describeWhen(new Date(2026, 7, 31, 12, 0), new Date(2026, 8, 1, 8, 0))).toBe(
      "yesterday 12:00",
    );
    expect(describeWhen(new Date("garbage"), now)).toBe("at an unknown time");
  });

  it("survives a timestamp it cannot read", () => {
    expect(lastCheckLine({ at: "not a date", outcome: "no-update", detail: "" }, now, "e")).toBe(
      "Last check: at an unknown time — you're on the latest version",
    );
  });
});

// ── The six-hourly recheck lives in Rust ─────────────────────────────────────
// Source introspection again, on both files: main.ts must have exactly the two
// JavaScript triggers left (launch and the button) plus a listener, and
// update_timer.rs must speak the vocabulary this file already pins.

describe("the six-hourly recheck lives in Rust", () => {
  const main = read("./main.ts");
  const timer = read("../src-tauri/src/update_timer.rs");
  const fnBody = (name: string, src: string) => {
    const s = src.indexOf(`function ${name}(`);
    expect(s, `function ${name} in source`).toBeGreaterThan(-1);
    return src.slice(s, src.indexOf("\n}\n", s));
  };

  it("no longer wraps updateCheck() in a setInterval", () => {
    expect(main).not.toMatch(/setInterval\([^\n]*updateCheck/);
    // The launch check and the button stay in JavaScript, through updateCheck.
    expect(main).toMatch(/^\s*void updateCheck\(\);/m);
    expect(main).toContain("await updateCheck(true)");
    // And the Rust side has a first delay and a period, not a fire-at-once.
    expect(timer).toMatch(/pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs\(2 \* 60\)/);
    expect(timer).toMatch(/pub const CHECK_PERIOD: Duration = Duration::from_secs\(6 \* 60 \* 60\)/);
    expect(timer).toContain("interval_at(");
  });

  it("listens for the timer's event under the name the Rust side emits", () => {
    const m = /UPDATE_CHECK_EVENT: &str = "([^"]+)"/.exec(timer);
    expect(m, "UPDATE_CHECK_EVENT in update_timer.rs").not.toBeNull();
    expect(main).toContain(`listen<UpdateCheckEvent>("${m![1]}"`);
    expect(main).toContain("onUpdateCheckEvent(e.payload)");
    // The payload's fields are the ones the TypeScript interface names.
    for (const field of ["outcome", "version", "detail", "at"]) {
      expect(timer, field).toMatch(new RegExp(`^\\s*pub ${field}: `, "m"));
    }
  });

  it("paints the event through the functions updateCheck() paints with", () => {
    const handler = fnBody("onUpdateCheckEvent", main);
    expect(handler).toContain("paintUpdateProgress(");
    expect(handler).toContain("paintLastUpdateCheck(");
    expect(handler).toContain("installErrorFromDetail(ev.detail)");
    const check = fnBody("updateCheck", main);
    for (const o of ["found", "install-failed", "installed"]) {
      expect(check, o).toContain(`paintUpdateProgress("${o}"`);
    }
    // The status tick's line goes through the same painter, and nothing else
    // writes that element.
    expect(fnBody("reflectLastUpdateCheck", main)).toContain("paintLastUpdateCheck(");
    expect(main.match(/\$\("update-last-check"\)/g)).toHaveLength(1);
    // The painter covers exactly the outcomes that show something.
    const paint = fnBody("paintUpdateProgress", main);
    for (const o of ["found", "install-failed", "installed"]) {
      expect(paint, o).toContain(`case "${o}":`);
    }
    expect(paint).not.toContain('case "no-update"');
    expect(paint).not.toContain('case "check-failed"');
  });

  it("speaks the same five words, always as automatic, on the Rust side", () => {
    const used = [...timer.matchAll(/settle\(app, &datadir, "([a-z-]+)"/g)].map((x) => x[1]);
    expect([...new Set(used)].sort()).toEqual([...UPDATE_CHECK_OUTCOMES].sort());
    // The details are the shapes updateCheckRecord produces for "automatic".
    expect(timer).toContain('"automatic: no build for this platform"');
    expect(timer).toContain('"automatic: v{current} is current"');
    expect(timer).toContain('"automatic: v{version} offered, downloading"');
    expect(timer).toContain('"automatic: v{version}: {}"');
    expect(timer).toContain('"automatic: v{version}, restarting"');
    expect(timer).not.toMatch(/"manual:/);
    // The record is written before the event, and a failed record is logged,
    // never propagated.
    const settle = timer.slice(timer.indexOf("fn settle("), timer.indexOf("\n}\n", timer.indexOf("fn settle(")));
    expect(settle.indexOf("update_log::record(")).toBeLessThan(settle.indexOf(".emit("));
    expect(settle).toContain("eprintln!");
    expect(settle).not.toMatch(/\?;|unwrap\(\)|expect\(/);
  });

  it("states the hypothesis as a hypothesis, with the measurement", () => {
    expect(timer).toMatch(/hypothesis/i);
    expect(timer).toContain("2026-09-15");
    expect(timer).toContain("eight hours");
    expect(timer).toMatch(/not a finding/);
    expect(timer).toMatch(/confirms or refutes/);
  });
});

describe("installErrorFromDetail", () => {
  it("returns the error alone out of an install-failed detail", () => {
    const rec = updateCheckRecord(
      { branch: "install-failed", version: "0.6.23", error: new Error("Failed to install .deb package") },
      "automatic",
    );
    expect(installErrorFromDetail(rec.detail)).toBe("Error: Failed to install .deb package");
    expect(installErrorFromDetail("manual: v0.6.23: deb: not supported")).toBe("deb: not supported");
  });

  it("hands every other detail back whole", () => {
    for (const d of [
      "automatic: v0.6.23 offered, downloading",
      "automatic: v0.6.23, restarting",
      "automatic: v0.6.23, restart failed: spawn failed",
      "automatic: v0.6.22 is current",
      "automatic: no build for this platform",
      "automatic",
      "",
    ]) {
      expect(installErrorFromDetail(d)).toBe(d);
    }
  });
});

// ── An update this copy will not download on its own ─────────────────────────
// update_binding.rs declines two offers before anything is downloaded: a .deb
// copy offered only the AppImage, and, on an automatic check, a version whose
// install already failed here. Its notices are the strings below, verbatim
// (update_binding.rs pins the same text in its own tests).

const DEB_NOTICE =
  "v0.6.34 is out. This copy came from a .deb, so install it by hand: " +
  "get the .deb from easybtx.com/node and run sudo apt install ./BTX-Node_0.6.34_amd64.deb";
const FAILED_NOTICE =
  "v0.6.34 failed to install here, so it is not downloaded again. " +
  "Press Check now to try again, or install it by hand from easybtx.com/node";
const REFUSED = "refused v0.9.0: its linux-x86_64 build is signed as X, not Y";

describe("an update this copy will not download on its own", () => {
  it("is found by the same phrase in TypeScript and in Rust", () => {
    const src = read("../src-tauri/src/update_binding.rs");
    const m = /HAND_INSTALL_MARK: &str = "([^"]+)"/.exec(src);
    expect(m, "HAND_INSTALL_MARK in update_binding.rs").not.toBeNull();
    expect(HAND_INSTALL_MARK).toBe(m![1]);
  });

  it("reads a notice from a refusal or from a recorded detail", () => {
    expect(handInstallNotice(DEB_NOTICE)).toBe(DEB_NOTICE);
    expect(handInstallNotice(`automatic: ${DEB_NOTICE}`)).toBe(DEB_NOTICE);
    expect(handInstallNotice(`manual: ${FAILED_NOTICE}`)).toBe(FAILED_NOTICE);
    // A refused feed and an ordinary detail are not notices.
    expect(handInstallNotice(REFUSED)).toBeNull();
    expect(handInstallNotice(`automatic: ${REFUSED}`)).toBeNull();
    expect(handInstallNotice("automatic: v0.6.33 is current")).toBeNull();
    expect(handInstallNotice("")).toBeNull();
  });

  it("puts the version in the banner and sends the reader to Settings", () => {
    expect(handInstallBanner(DEB_NOTICE)).toEqual({
      head: "v0.6.34 is out.",
      tail: "Install it by hand, the steps are in Settings.",
    });
    expect(handInstallBanner("install it by hand").head).toBe("An update is out.");
  });

  it("never says 'latest version' after a pressed check that was declined or refused", () => {
    expect(noUpdateMessage(null, "0.6.33", "easybtx.com/node")).toBe(
      "You're on the latest version (v0.6.33).",
    );
    expect(noUpdateMessage(null, "", "easybtx.com/node")).toBe("You're on the latest version.");
    // A notice is written for people already, and it carries the command.
    expect(noUpdateMessage(DEB_NOTICE, "0.6.33", "easybtx.com/node")).toBe(DEB_NOTICE);
    const refused = noUpdateMessage(REFUSED, "0.6.33", "easybtx.com/node");
    expect(refused).not.toMatch(/latest version/);
    expect(refused).toContain(REFUSED);
    expect(refused).toContain("easybtx.com/node");
    expect(noUpdateMessage("x".repeat(500), "0.6.33", "e.com").length).toBeLessThan(260);
  });

  it("shows the Last check line in plain words, with the version", () => {
    const now = new Date(2026, 8, 29, 16, 30);
    const at = new Date(2026, 8, 29, 14, 3).toISOString();
    for (const notice of [DEB_NOTICE, FAILED_NOTICE]) {
      expect(
        lastCheckLine(
          { at, outcome: "check-failed", detail: `automatic: ${notice}` },
          now,
          "easybtx.com/node",
        ),
      ).toBe("Last check: today 14:03 — v0.6.34 is out, install it by hand from easybtx.com/node");
    }
    // A refusal still reads as a failed check.
    expect(plainOutcome("check-failed", `automatic: ${REFUSED}`, "e")).toBe("couldn't check");
  });

  it("adds no em-dash of its own", () => {
    for (const text of [
      handInstallBanner(DEB_NOTICE).head,
      handInstallBanner(DEB_NOTICE).tail,
      noUpdateMessage(REFUSED, "0.6.33", "easybtx.com/node"),
      plainOutcome("check-failed", DEB_NOTICE, "easybtx.com/node"),
    ]) {
      expect(text).not.toContain("—");
    }
  });
});

// ── main.ts: the failed version, the declined offer, and "Check now" ─────────
// Read from main.ts, the way the tests above read it: updateCheck() is a thin
// Tauri veneer that cannot run under vitest, but its order can be pinned.

describe("updateCheck() keeps a failed install from downloading again, and a press still tries", () => {
  const main = read("./main.ts");
  const lib = read("../src-tauri/src/lib.rs");
  const fn = (name: string) => {
    const s = main.indexOf(`function ${name}(`);
    expect(s, `function ${name} in main.ts`).toBeGreaterThan(-1);
    return main.slice(s, main.indexOf("\n}\n", s));
  };
  const body = fn("updateCheck");

  it("clears the failed version before a pressed check, and only then", () => {
    const forget = body.indexOf('invoke("forget_failed_update")');
    expect(forget).toBeGreaterThan(-1);
    expect(forget).toBeLessThan(body.indexOf("await checkForUpdate()"));
    expect(body.slice(0, forget)).toMatch(/if \(manual\) \{\s*await $/);
  });

  it("never lets clearing the failed version or reading the refusal end a press in silence", () => {
    // Without the catch a failing forget rejects updateCheck(), and a press
    // ends with nothing on screen. Each failure is a console warning instead.
    expect(body).toMatch(
      /await invoke\("forget_failed_update"\)\.catch\(\(e\) =>\s*console\.warn\("update-check: could not clear the failed version", e\),?\s*\);/,
    );
    expect(fn("peekUpdateRefusal")).toMatch(
      /invoke<string \| null>\("peek_update_refusal"\)\.catch\(\(e\) => \{\s*console\.warn\("update-check: could not read the refusal", e\);\s*return null;\s*\}\);/,
    );
  });

  it("downloads, then installs, and remembers a version only when the install failed", () => {
    expect(body).not.toContain("downloadAndInstall(");
    const download = body.indexOf("await update.download()");
    const downloaded = body.indexOf("downloaded = true;");
    const install = body.indexOf("await update.install()");
    const remember = body.indexOf('invoke("remember_failed_update"');
    expect(download).toBeGreaterThan(-1);
    expect(download < downloaded && downloaded < install && install < remember).toBe(true);
    expect(body.slice(install, remember)).toMatch(/if \(downloaded\) \{\s*void $/);
  });

  it("frees the downloaded package when the download or the install fails", () => {
    // The plugin frees the package (150-470 MB) only after an install that
    // succeeded. Without the close, every failed launch check or press holds
    // it in memory until the app quits.
    const install = body.indexOf("await update.install()");
    const caught = body.indexOf("} catch (e) {", install);
    expect(caught).toBeGreaterThan(install);
    const failed = body.slice(caught, body.indexOf("return;", caught));
    expect(failed).toMatch(/void update\.close\(\)\.catch\(/);
    // And the calls around it keep their order: remember, paint, record.
    const remember = failed.indexOf('invoke("remember_failed_update"');
    const paint = failed.indexOf('paintUpdateProgress("install-failed"');
    const record = failed.indexOf('branch: "install-failed"');
    expect(remember, "remember inside the catch").toBeGreaterThan(-1);
    expect(remember, "remember before paint").toBeLessThan(paint);
    expect(paint, "paint before record").toBeLessThan(record);
  });

  it("reads why an offer was declined before the record takes it, and shows it", () => {
    const peek = body.indexOf("await peekUpdateRefusal()");
    expect(peek).toBeGreaterThan(-1);
    expect(peek).toBeLessThan(body.indexOf('branch: "no-update"'));
    expect(body).toContain("paintHandInstall(declined");
    expect(body).toContain("noUpdateMessage(declined, appVersion, MANUAL_DOWNLOAD)");
  });

  it("shows a notice from the six-hourly timer the same way", () => {
    expect(fn("onUpdateCheckEvent")).toContain("paintHandInstall(ev.detail)");
    const paint = fn("paintHandInstall");
    expect(paint).toContain("handInstallNotice(");
    expect(paint).toContain("showUpdateBanner(");
    expect(paint).toContain("setUpdateResult(");
  });

  it("calls only commands the backend registers", () => {
    for (const name of [
      "record_update_check",
      "remember_failed_update",
      "forget_failed_update",
      "peek_update_refusal",
    ]) {
      expect(main, name).toMatch(new RegExp(`invoke(<[^>]*>)?\\("${name}"`));
      expect(lib, name).toContain(`commands::${name},`);
    }
  });
});
