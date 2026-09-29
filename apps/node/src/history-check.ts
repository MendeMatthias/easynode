/**
 * The background check of a snapshot's older history, as one line and a thin
 * bar on the status screen (docs/decisions/2026-09-29-quick-start-full-check-
 * and-progress.md, section 2).
 *
 * A node that starts from a snapshot checks every new block from there on,
 * and re-checks the history below the snapshot in the background, from block
 * 0 up to the snapshot's own height. Until this line, only a wallet caveat
 * said so. No time estimate on purpose: the engine paces the check, and its
 * speed says nothing reliable yet.
 */

/** `NodeStatusInfo.history_check` (btx_core::node_api::HistoryCheck). */
export type HistoryCheck = { checked: number; base: number };

export type HistoryCheckView = { line: string; pct: number };

/** "Checking older history: 131,200 of 225,927 (58%)" and the bar's width.
 *  Rounded down, so the bar never reads 100% before the engine reports the
 *  check done, which is when the field goes away. */
export function historyCheckView(h: HistoryCheck | null): HistoryCheckView | null {
  if (!h || !(h.base > 0) || !Number.isFinite(h.checked)) return null;
  const checked = Math.min(Math.max(0, h.checked), h.base);
  const pct = Math.floor((checked / h.base) * 100);
  const fmt = (n: number) => n.toLocaleString("en-US");
  return { line: `Checking older history: ${fmt(checked)} of ${fmt(h.base)} (${pct}%)`, pct };
}
