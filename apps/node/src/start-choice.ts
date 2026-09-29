/**
 * The setup screen's one question, and the notice when the engine turns a
 * Mac's chip down (docs/decisions/2026-09-29-quick-start-full-check-and-
 * progress.md, section 1). Pure, so the rules are tested without a window.
 */

/** The names `begin_setup` reads (btx_core::node::StartChoice). */
export type StartChoice = "quick_start" | "full_check";

/** Shown under a greyed-out Full check. */
export const NO_GPU_REASON = "This computer has no graphics card the BTX engine can check blocks with.";

export type StartChoiceView = { selected: StartChoice; fullCheckDisabled: boolean };

/** Which choice is selected, and whether Full check can be picked. Both
 *  answers come from Rust (`full_check_possible`, `full_check_first`); the
 *  owner's rule of 2026-09-29 behind them:
 *  - an NVIDIA machine: Full check selected first;
 *  - a Mac: Quick start selected first, Full check still there to pick;
 *  - a machine that cannot check blocks: Quick start, Full check greyed out.
 *  The owner's own pick, once made, holds across polls. */
export function startChoiceView(
  fullCheckPossible: boolean,
  fullCheckFirst: boolean,
  picked: StartChoice | null,
): StartChoiceView {
  if (!fullCheckPossible) return { selected: "quick_start", fullCheckDisabled: true };
  const first: StartChoice = fullCheckFirst ? "full_check" : "quick_start";
  return { selected: picked ?? first, fullCheckDisabled: false };
}

/** Whether the line under the choices ends "You can switch later in
 *  Settings.": only where Full check is possible. Settings shows the switch
 *  only where it is a choice (validation.ts `followRowVisible`), and a machine
 *  that cannot check blocks follows signatures with no marker and no switch,
 *  so there the sentence would not be true. */
export function switchLaterShown(fullCheckPossible: boolean): boolean {
  return fullCheckPossible;
}

/** The arguments `begin_setup` takes. */
export function setupArgs(choice: StartChoice): { choice: StartChoice } {
  return { choice };
}

/** Whether the status screen shows "This Mac's graphics chip didn't pass the
 *  engine's check, so your node follows signatures. Nothing for you to do."
 *
 *  Only while it is true: the chip was refused and the node follows
 *  signatures. Once per engine: `seenForTag` is the engine tag the owner
 *  dismissed it for, and a newer engine clears the refusal and tries the chip
 *  again, so a second refusal is news. */
export function chipNoticeVisible(
  chipRefused: boolean,
  followsSignatures: boolean,
  seenForTag: string | null,
  tag: string,
): boolean {
  return chipRefused && followsSignatures && seenForTag !== tag;
}
