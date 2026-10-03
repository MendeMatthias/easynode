/**
 * "Same chain as other sources": does this node have the same block as the
 * public sources, at a height below every tip? The comparison and every
 * sentence are btx_core::chain_agreement's, in the BTX Verdict Spec's words
 * (btx-verdicts/0.1), the same words easybtx.com uses. This only picks the
 * tone and the order of the lines.
 *
 * The tone follows the row status and nothing else: green only for OK, grey
 * for UNKNOWN and for any status this code does not know, so a word it has
 * never seen can never read better than "could not be read".
 */

/** `NodeStatusInfo.chain_agreement` (btx_core::chain_agreement::ScreenRow). */
export type ChainAgreementRow = {
  /** ok | caution | warning | unknown */
  status: string;
  /** AGREE | BEHIND | STALE TIP | DISAGREE | NOT ENOUGH SOURCES | NOT RUN */
  word: string;
  headline: string;
  scope: string;
  meaning: string | null;
  /** "as of 14:05 UTC" */
  as_of: string;
};

export type ChainAgreementView = {
  tone: "is-ok" | "is-caution" | "is-warning" | "is-unknown";
  word: string;
  /** Who agrees, at which block, and when. */
  scope: string;
  /** The headline, then what it means, when there is a meaning. */
  lines: string[];
};

const TONES: Record<string, ChainAgreementView["tone"]> = {
  ok: "is-ok",
  caution: "is-caution",
  warning: "is-warning",
  unknown: "is-unknown",
};

export function chainAgreementView(
  row: ChainAgreementRow | null | undefined,
  running: boolean,
): ChainAgreementView | null {
  if (!running || !row) return null;
  const lines = [row.headline];
  if (row.meaning) lines.push(row.meaning);
  return {
    tone: Object.prototype.hasOwnProperty.call(TONES, row.status) ? TONES[row.status] : "is-unknown",
    word: row.word,
    scope: `${row.scope} · ${row.as_of}`,
    lines,
  };
}
