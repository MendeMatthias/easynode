/**
 * What a failed update CHECK actually means, and what to say about it.
 *
 * A single catch used to answer every check failure with "Couldn't check right
 * now — are you online?". That is right for a real network failure and wrong
 * for the commonest one: a release that does not include this platform.
 *
 * An omitted platform is the SAFE state and the normal cadence here — 0.6.18,
 * 0.6.19 and 0.6.20 were all Linux-only at some point, and build-node-feed.sh
 * documents that those clients simply stay where they are. But the updater does
 * not report it as "no update"; `get_urls` runs BEFORE the version comparison,
 * so the client gets an ERROR. Worded as a connectivity problem it sent Mac
 * owners looking for a fault that did not exist, on the one release where they
 * needed to be told to download by hand instead.
 *
 * The discriminator is the plugin's own words. tauri-plugin-updater 2.11:
 *
 *   TargetNotFound   "the platform `{0}` was not found in the response
 *                     `platforms` object"
 *   TargetsNotFound  "None of the fallback platforms `{0:?}` were found in the
 *                     response `platforms` object"
 *
 * Nothing else in that error enum mentions the response's `platforms` object,
 * so that phrase identifies these two and only these two. Matching on the
 * MESSAGE rather than a code is not ideal; it is what the plugin gives the
 * front end, and the tests below pin both strings so a plugin upgrade that
 * rewords them fails here rather than in front of a user.
 */

/** Why a check failed, as far as we can tell from what the plugin said. */
export type CheckFailure =
  | { kind: "no-build-for-this-platform" }
  | { kind: "unknown"; detail: string };

const NO_BUILD = /`platforms` object/i;

export function classifyCheckFailure(e: unknown): CheckFailure {
  const detail = String(e);
  return NO_BUILD.test(detail)
    ? { kind: "no-build-for-this-platform" }
    : { kind: "unknown", detail };
}

/**
 * What to show for a failed check on a MANUAL press. The automatic path stays
 * silent either way: a six-hourly banner about a platform that is simply not
 * in this release would be noise, and there is nothing to act on.
 *
 * `currentVersion` may be empty — it is read from the backend and the check can
 * run first — so the copy must not depend on having it.
 */
export function checkFailureMessage(
  failure: CheckFailure,
  currentVersion: string,
  downloadsAt: string,
): string {
  if (failure.kind === "no-build-for-this-platform") {
    const staying = currentVersion
      ? `This copy stays on v${currentVersion}.`
      : "This copy stays where it is.";
    return (
      `The current release has no build for this platform, so there is nothing ` +
      `to install. ${staying} Downloads for every platform are at ${downloadsAt}.`
    );
  }
  return `Couldn't check right now — are you online? (${failure.detail.slice(0, 80)})`;
}

// ── What a check ended as, written down ─────────────────────────────────────
//
// updateCheck() in main.ts painted its result into the Settings pane and
// nowhere else, and on the automatic path a failed check returned in silence.
// Measured 2026-09-15: this project's own signer box ran 0.6.21 for eight
// hours after the feed served 0.6.22, with two six-hourly checks due in that
// window, and nothing anywhere said whether they ran, failed, or found
// nothing. Every exit of updateCheck() now records one of the words below
// through the `record_update_check` command (src-tauri/src/update_log.rs),
// which appends a line to <datadir>/update-check.log and keeps the last
// outcome in the settings file for the pane's "Last check" line.

/**
 * The closed set of things a check can end as. `UPDATE_CHECK_OUTCOMES` in
 * src-tauri/src/update_log.rs is the same five words and the Rust side refuses
 * anything else unwritten; update-check.test.ts reads that file and keeps the
 * two lists equal.
 */
export const UPDATE_CHECK_OUTCOMES = [
  "no-update",
  "check-failed",
  "found",
  "install-failed",
  "installed",
] as const;
export type UpdateCheckOutcome = (typeof UPDATE_CHECK_OUTCOMES)[number];

/** Who started the check. A manual press that fails is on screen already; an
 *  automatic one that fails was, until now, nowhere. */
export type UpdateCheckTrigger = "manual" | "automatic";

/**
 * The six ways out of updateCheck(). "installed" and "relaunch-failed" are the
 * same outcome with a different detail: the update IS installed either way,
 * and whether the restart worked is the detail.
 */
export type UpdateCheckBranch =
  | { branch: "check-failed"; failure: CheckFailure }
  | { branch: "no-update"; currentVersion: string }
  | { branch: "found"; version: string }
  | { branch: "install-failed"; version: string; error: unknown }
  | { branch: "installed"; version: string }
  | { branch: "relaunch-failed"; version: string; error: unknown };

export interface UpdateCheckRecord {
  outcome: UpdateCheckOutcome;
  detail: string;
}

/** An updater error can run to a stack of URLs; the log keeps this much. */
const DETAIL_ERROR_CHARS = 200;

function errorText(e: unknown): string {
  return String(e).replace(/\s+/g, " ").trim().slice(0, DETAIL_ERROR_CHARS);
}

/**
 * The word and the short detail for one exit of updateCheck(). Pure, so the
 * mapping from branch to outcome is a test rather than a reading of main.ts.
 */
export function updateCheckRecord(
  branch: UpdateCheckBranch,
  trigger: UpdateCheckTrigger,
): UpdateCheckRecord {
  switch (branch.branch) {
    case "check-failed":
      return {
        outcome: "check-failed",
        detail:
          branch.failure.kind === "no-build-for-this-platform"
            ? `${trigger}: no build for this platform`
            : `${trigger}: ${errorText(branch.failure.detail)}`,
      };
    case "no-update":
      return {
        outcome: "no-update",
        detail: branch.currentVersion
          ? `${trigger}: v${branch.currentVersion} is current`
          : trigger,
      };
    case "found":
      return {
        outcome: "found",
        detail: `${trigger}: v${branch.version} offered, downloading`,
      };
    case "install-failed":
      return {
        outcome: "install-failed",
        detail: `${trigger}: v${branch.version}: ${errorText(branch.error)}`,
      };
    case "installed":
      return {
        outcome: "installed",
        detail: `${trigger}: v${branch.version}, restarting`,
      };
    case "relaunch-failed":
      return {
        outcome: "installed",
        detail: `${trigger}: v${branch.version}, restart failed: ${errorText(branch.error)}`,
      };
  }
}

/** The last recorded check as `get_node_status` carries it: null/empty until
 *  the first check has finished. */
export interface LastUpdateCheck {
  at: string | null;
  outcome: string | null;
  detail: string;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function hhmm(d: Date): string {
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function sameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

/** "today 14:03", "yesterday 09:12", "12 Sep 14:03", "3 Jan 2025 08:00". Local
 *  time, because the person reading it is sitting at the machine. */
export function describeWhen(at: Date, now: Date): string {
  if (Number.isNaN(at.getTime())) return "at an unknown time";
  if (sameDay(at, now)) return `today ${hhmm(at)}`;
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1);
  if (sameDay(at, yesterday)) return `yesterday ${hhmm(at)}`;
  const year = at.getFullYear() === now.getFullYear() ? "" : ` ${at.getFullYear()}`;
  return `${at.getDate()} ${MONTHS[at.getMonth()]}${year} ${hhmm(at)}`;
}

const VERSION_IN_DETAIL = /\bv\d+\.\d+\.\d+\b/;

/** The outcome in plain words. `detail` only lends the version number. An
 *  outcome this build does not know (a file written by a newer one) is shown
 *  as its word rather than hidden. */
export function plainOutcome(outcome: string, detail: string, downloadsAt: string): string {
  const v = VERSION_IN_DETAIL.exec(detail)?.[0];
  switch (outcome) {
    case "no-update":
      return "you're on the latest version";
    case "check-failed":
      return /no build for this platform/.test(detail)
        ? "no build for this platform yet"
        : "couldn't check";
    case "found":
      return v ? `found ${v}` : "found an update";
    case "install-failed":
      return `${v ?? "an update"} couldn't install — get it from ${downloadsAt}`;
    case "installed":
      return `${v ?? "an update"} installed`;
    default:
      return outcome;
  }
}

/**
 * The permanent line under "Check now". Rendered from the persisted values on
 * every status tick, so the automatic path shows what it did without anybody
 * pressing anything, and it reads the same after a relaunch.
 */
export function lastCheckLine(last: LastUpdateCheck, now: Date, downloadsAt: string): string {
  if (!last.at || !last.outcome) return "Last check: none yet";
  const when = describeWhen(new Date(last.at), now);
  return `Last check: ${when} — ${plainOutcome(last.outcome, last.detail, downloadsAt)}`;
}
