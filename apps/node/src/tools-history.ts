// Pure helpers for the Tools command window: the session's history and the
// display cap. The history lives in memory only and is gone when the app closes.

export interface Entry {
  line: string;
  answer: string;
}

export const DISPLAY_LIMIT = 256 * 1024;

export class History {
  entries: Entry[] = [];
  private cursor = -1;
  constructor(private readonly max: number) {}

  push(e: Entry): void {
    this.entries.push(e);
    if (this.entries.length > this.max) this.entries.shift();
    this.cursor = -1;
  }

  /** The previous command, walking back from the newest. */
  up(): string {
    if (this.entries.length === 0) return "";
    this.cursor = this.cursor === -1 ? this.entries.length - 1 : Math.max(0, this.cursor - 1);
    return this.entries[this.cursor].line;
  }

  /** The next command, or an empty line past the newest. */
  down(): string {
    if (this.cursor === -1) return "";
    this.cursor += 1;
    if (this.cursor >= this.entries.length) {
      this.cursor = -1;
      return "";
    }
    return this.entries[this.cursor].line;
  }

  allText(): string {
    return this.entries.map((e) => `> ${e.line}\n${e.answer}`).join("\n\n");
  }
}

/** A double-click's two `click` events land closer together than this;
 * `RestartArm.click` ignores the second one, so a double-click gesture
 * cannot both arm and run a two-click control in the same movement. */
export const ARM_DOUBLE_CLICK_GUARD_MS = 400;

/**
 * Restart node's two-click arm/disarm state, pulled out of the DOM glue so it
 * has a test. The actual 5-second timer and the "overlay just closed" event
 * both live in tools.ts; either one calls `disarm()` to cancel the arm.
 * Fast-forward's ten-second arm (tools.ts) uses the same class.
 */
export class RestartArm {
  private armedFlag = false;
  private armedAt = 0;

  get armed(): boolean {
    return this.armedFlag;
  }

  /**
   * A click on Restart node. The first click only arms it (returns false);
   * a second click while armed, at least ARM_DOUBLE_CLICK_GUARD_MS after the
   * first, returns true, meaning "actually restart now", and disarms so a
   * third click starts over. A second click sooner than that is a
   * double-click, not two deliberate clicks, and is ignored: still armed,
   * waiting for a real second click.
   */
  click(): boolean {
    const now = Date.now();
    if (this.armedFlag) {
      if (now - this.armedAt < ARM_DOUBLE_CLICK_GUARD_MS) return false;
      this.armedFlag = false;
      return true;
    }
    this.armedFlag = true;
    this.armedAt = now;
    return false;
  }

  /** The arm timeout elapsed, or the overlay closed: back to disarmed. */
  disarm(): void {
    this.armedFlag = false;
  }
}

/**
 * Copy diagnostics, in two clicks. Building the report takes seconds, and
 * WebKit (the webview on macOS and Linux) only lets a page write the
 * clipboard inside the click that asked for it, not seconds later. So the
 * first click builds the report and shows it, and the second click copies
 * the text already built, inside its own click.
 */
export class ReportCopy {
  private built: string | null = null;

  /** The button's label: what the next click does. */
  get label(): string {
    return this.built === null ? "Copy diagnostics" : "Copy report";
  }

  /** The report to copy on this click, or null when this click builds it. */
  get text(): string | null {
    return this.built;
  }

  /** The report is built and shown. */
  ready(text: string): void {
    this.built = text;
  }

  /** Tools closed: the next click builds a fresh report. */
  reset(): void {
    this.built = null;
  }
}

/** Fast-forward's second click must come within this long of the first
 * (the Tools decision, section 3). The arm itself is a RestartArm. */
export const FAST_FORWARD_ARM_MS = 10_000;

/** How often the overlay asks how a Fast-forward is going. */
export const FAST_FORWARD_POLL_MS = 3_000;

/** The section's line when nothing else needs saying. */
export const FF_NOTE = "A confirmed snapshot is far ahead of your node.";

/** What `tools_fast_forward_check` answers. */
export type FastForwardCheck =
  | { kind: "none" }
  | { kind: "offer"; height: number; button: string; confirm: string; note: string }
  | { kind: "off"; height: number; sentence: string };

/** What `tools_fast_forward_status` answers. */
export interface FastForwardStatus {
  running: boolean;
  message: string | null;
}

/** What the Fast-forward section shows. */
export interface FastForwardView {
  /** The section is visible. */
  section: boolean;
  /** The button's label, or null for no button. */
  button: string | null;
  /** The line above the button. */
  note: string;
}

/** Decided in one place, so it has a test: a run hides the button, a
 * dispute shows its sentence and no button (the confirmed-snapshot
 * decision, section 6a), an offer shows the button with the note Rust sent
 * (far ahead, or offered early because no archive peer serves old blocks),
 * and a last run's message keeps the section open on its own. */
export function fastForwardView(
  check: FastForwardCheck | null,
  status: FastForwardStatus | null,
): FastForwardView {
  if (status?.running) return { section: true, button: null, note: FF_NOTE };
  if (check?.kind === "off") return { section: true, button: null, note: check.sentence };
  if (check?.kind === "offer") return { section: true, button: check.button, note: check.note };
  return { section: Boolean(status?.message), button: null, note: FF_NOTE };
}

// The note counts against DISPLAY_LIMIT itself, so the shown text (content
// plus note) never exceeds the cap and is always shorter than the original.
const TRUNCATION_NOTE = "\n\n(The answer is longer than this window shows. Copy takes the whole answer.)";

export function capForDisplay(text: string): string {
  if (text.length <= DISPLAY_LIMIT) return text;
  const sliceLen = Math.max(0, DISPLAY_LIMIT - TRUNCATION_NOTE.length);
  return `${text.slice(0, sliceLen)}${TRUNCATION_NOTE}`;
}
