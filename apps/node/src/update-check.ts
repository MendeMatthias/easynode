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
