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

// The note counts against DISPLAY_LIMIT itself, so the shown text (content
// plus note) never exceeds the cap and is always shorter than the original.
const TRUNCATION_NOTE = "\n\n(The answer is longer than this window shows. Copy takes the whole answer.)";

export function capForDisplay(text: string): string {
  if (text.length <= DISPLAY_LIMIT) return text;
  const sliceLen = Math.max(0, DISPLAY_LIMIT - TRUNCATION_NOTE.length);
  return `${text.slice(0, sliceLen)}${TRUNCATION_NOTE}`;
}
