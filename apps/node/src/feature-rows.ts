/**
 * Settings rows for features that are on by default and run only where the
 * machine can run them (the owner's rule of 2026-10-12). Pure, so what each
 * row says in each state is tested.
 */

export const EXPLORER_COPY =
  "Look up any past transaction in Ask. Builds a small index once (the whole chain has about 350,000 transactions); the node restarts to apply a change";

/** The same sentence as Rust's `ask::EXPLORER_NEEDS_WHOLE_CHAIN`. */
const EXPLORER_NOT_HERE =
  "Not on this node: Explorer mode needs the whole chain, and this node keeps only the newest blocks.";

export type ExplorerRow = { checked: boolean; disabled: boolean; desc: string; calm: boolean };

/** Explorer mode's row: on and switchable where the whole chain is kept;
 *  greyed out with the reason where it is not, because BTX refuses to start
 *  with pruning and a transaction index together. `calm` is the quiet
 *  colour for "not on this computer", which is not a fault. */
export function explorerRow(s: { txindex_enabled: boolean; explorer_available: boolean }): ExplorerRow {
  if (!s.explorer_available) return { checked: false, disabled: true, desc: EXPLORER_NOT_HERE, calm: true };
  return { checked: s.txindex_enabled, disabled: false, desc: EXPLORER_COPY, calm: false };
}
