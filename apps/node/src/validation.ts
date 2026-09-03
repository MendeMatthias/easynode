/**
 * How this machine checks blocks under the MatMul v4.7 proof of work.
 *
 * BTX replaced its proof of work at mainnet block 185,000. Checking a block now
 * means replaying a heavy "RC episode", and btxd will only do that on a
 * graphics chip it recognises. Machines outside that set either check blocks on
 * the processor (which stops at block 185,000) or stop following the chain entirely.
 *
 * Lives in its own module, away from main.ts's DOM side effects, so the copy
 * decisions are unit-testable. The rule that matters: every state here comes
 * from what btxd REPORTED about itself, never from a guess based on the
 * platform. A Mac whose Metal shaders fail to build falls back to the processor,
 * and a platform guess would cheerfully call that machine "Full".
 */

/** The subset of the node status this decision needs. */
export type ValidationInput = {
  running: boolean;
  /** Seconds since this node run started — bounds the "still checking" window. */
  uptime_secs: number;
  /** btxd's own mode string, or null before it has logged one. */
  rc_mode: string | null;
  rc_validates_independently: boolean;
  rc_may_fall_behind: boolean;
  rc_stalled: boolean;
  /** Following the chain via an attestation quorum instead of local replay. */
  rc_trusted_mirror: boolean;
  /**
   * Archive peers passing the trusted-mirror authority gate (manual or noban),
   * or null when unknown (node stopped / didn't answer / older backend). On a
   * mirror, 0 here is the root cause of the silent-stall class: the node will
   * ask NOBODY for the confirmations it needs, while every ordinary health
   * signal stays green. Only consulted on the mirror branch.
   */
  archive_authority?: number | null;
  /**
   * The backend stall discriminator's verdict once the tip has actually been
   * frozen past the verdict window. Outranks every other card state: a
   * classified stall with a plain-language summary is strictly more useful
   * than the generic mode copy.
   */
  stall?: { class: string; summary: string } | null;
};

/**
 * How long to call a missing verdict "still checking" rather than nothing.
 *
 * btxd proves out the machine by running one FULL production episode before it
 * will call itself qualified. Measured on an M2 Pro: 102 s, 172 s and 218 s
 * across three runs. Ten minutes is comfortably past that without being
 * open-ended — after it, a node that still has not reported is almost certainly
 * an older btxd that never logs a verdict at all, and claiming "checking"
 * forever would be a lie.
 */
const STILL_CHECKING_SECS = 600;

export type ValidationView = {
  /** null = say nothing at all (the card stays hidden). */
  state: string | null;
  note: string;
  /** "" | "is-degraded" | "is-stalled" */
  cls: string;
};

const HIDDEN: ValidationView = { state: null, note: "", cls: "" };

// "Full" and "Basic" grade the CHECKING, not the node. Every install keeps the
// whole chain (prune=0) and serves it to peers — there is no light mode here to
// fall back to — so "Basic" must never read as a lesser KIND of node.
export function validationView(status: ValidationInput): ValidationView {
  if (!status.running) return HIDDEN;

  // A classified stall from the backend discriminator outranks everything:
  // the tip has demonstrably been frozen past the verdict window and the
  // summary already says, in plain language, what is wrong and what the app
  // is doing about it. "Needs attention" — deliberately not the red error
  // card: the node is up, this is a network-supply condition, and the worst
  // response would be the user force-killing a healthy process.
  if (status.stall) {
    return {
      state: "Needs attention",
      note: status.stall.summary,
      cls: "is-stalled",
    };
  }

  if (!status.rc_mode) {
    // btxd has not delivered its verdict yet. On a capable machine that means
    // it is mid-episode: a couple of minutes of saturated GPU, fans up, and
    // (before this) a blank card explaining none of it. Say what is happening
    // instead. Past the window we go quiet again rather than claim progress
    // that may never arrive.
    if (status.uptime_secs < STILL_CHECKING_SECS) {
      return {
        state: "Checking…",
        note:
          "Your node is working out what this machine can do. It runs the new proof of " +
          "work once to find out, which takes a few minutes and works the graphics chip " +
          "hard. This happens once per start.",
        cls: "",
      };
    }
    return HIDDEN;
  }

  if (status.rc_stalled) {
    return {
      state: "Stopped",
      note:
        "Your node cannot check the new proof of work on this machine, so it has stopped " +
        "following new blocks. Everything already downloaded is safe. An Apple Silicon Mac " +
        "runs it fine; on a PC it needs one of the newest graphics cards.",
      cls: "is-stalled",
    };
  }
  if (status.rc_validates_independently) {
    return {
      state: "Full",
      note:
        "This node checks every block itself, including the new proof of work. That is the " +
        "strongest thing a node can do for the network.",
      cls: "",
    };
  }
  // Checked BEFORE validates_independently and before the fall-behind branch.
  // A mirror's policy line is shaped like a stall (strict-device, ready=0), so
  // without this it would fall through to the raw-mode fallback and print
  // "strict-device" at the user.
  if (status.rc_trusted_mirror) {
    // The one condition that turns a healthy-looking mirror into a stalled one
    // BEFORE the height freezes: zero peers passing the authority gate means
    // the node will not ask anyone for the signed confirmations it needs
    // (api.btxscan.io incident class). Zero only — null stays on the calm copy,
    // because "we don't know" is not "it's broken".
    if (status.archive_authority === 0) {
      return {
        state: "Mirror — waiting for a source",
        note:
          "Your node follows the chain using signed confirmations from other operators, " +
          "but right now it is not connected to any node allowed to hand them over. It " +
          "will keep trying the known sources on its own. If this message stays for more " +
          "than an hour, your node may sit at a fixed block height until one comes back — " +
          "nothing is lost, it catches up when a source appears.",
        cls: "is-stalled",
      };
    }
    return {
      state: "Mirror",
      note:
        "This machine cannot check the new proof of work itself, so instead of stopping at " +
        "block 185,000 your node follows the chain using signed confirmations from two " +
        "independent operators who did check it. Everything else, the blocks, the " +
        "transactions and the balances, your node still checks on its own. The trade is " +
        "real: for that one check you are trusting those two, not verifying it yourself.",
      cls: "is-degraded",
    };
  }
  if (status.rc_may_fall_behind) {
    // Deliberately not naming exact chips: the network's accepted-hardware list
    // lives in the node software and upstream can change it, so copy that spells
    // out models would quietly go stale. "Apple Silicon / newest cards" is true
    // today and stays roughly true if the list grows.
    return {
      state: "Basic (light)",
      note:
        "Your node still keeps the whole chain and shares it with other people, the same " +
        "as any other node here. Checking the new proof of work needs an Apple Silicon Mac " +
        "or one of the newest graphics cards, so this machine checks blocks on the " +
        "processor instead. That means it stops at block 185,000 and does not follow the " +
        "chain past it. We would rather tell you than leave it looking healthy.",
      cls: "is-degraded",
    };
  }
  // A mode we have no copy for. Show it plainly rather than inventing meaning.
  return { state: status.rc_mode, note: "", cls: "" };
}
