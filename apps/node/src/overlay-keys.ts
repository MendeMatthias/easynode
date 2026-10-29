/**
 * The keyboard around the app's panels (the `.overlay` dialogs): Escape
 * closes the one on top the way its ✕ does, focus moves into a panel when it
 * opens and back to what opened it when it closes, and Tab stays inside the
 * open panel. The panels declare `aria-modal="true"`, which no browser
 * enforces for a `div`; before this, Tab walked into the dimmed screen behind
 * and Escape did nothing.
 *
 * Escape presses the panel's own close button rather than hiding it, so each
 * panel's close handler still runs (Info stops its footprint poll, Wallet and
 * Ask reset their state).
 */

/** Each closable panel and its ✕. */
export const CLOSE_BUTTONS: Record<string, string> = {
  "settings-overlay": "settings-close",
  "info-overlay": "info-close",
  "wallet-overlay": "wallet-close",
  "ask-overlay": "ask-close",
  "tools-overlay": "tools-close",
  "future-overlay": "future-close",
};

/** Panels that ask for an answer: "Close BTX Node?" and the welcome panel.
 *  Escape leaves them, and anything under them, alone. */
export const DECISIONS = ["close-overlay", "welcome-overlay"];

/** The close button Escape presses, given the open panels in page order, or
 *  null. Every panel shares one z-index except the close dialog, which sits
 *  above them all, so the topmost is a decision if one is open and otherwise
 *  the last in the page. */
export function escapeTarget(openInPageOrder: string[]): string | null {
  if (openInPageOrder.some((id) => DECISIONS.includes(id))) return null;
  const top = openInPageOrder[openInPageOrder.length - 1];
  return top !== undefined ? (CLOSE_BUTTONS[top] ?? null) : null;
}

/** Where Tab lands inside a panel with `count` focusable controls, focus at
 *  `index` (-1 when it is outside the panel): the index to move to, or null
 *  to let the browser move it. */
export function wrapFocus(index: number, count: number, backwards: boolean): number | null {
  if (count === 0) return null;
  if (index === -1) return backwards ? count - 1 : 0;
  if (!backwards && index === count - 1) return 0;
  if (backwards && index === 0) return count - 1;
  return null;
}

const FOCUSABLE =
  'button, a[href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';

function focusables(panel: HTMLElement): HTMLElement[] {
  return Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (el) => !el.matches(":disabled") && el.getClientRects().length > 0,
  );
}

const openOverlays = (): HTMLElement[] =>
  Array.from(document.querySelectorAll<HTMLElement>(".overlay")).filter((o) => !o.hidden);

/** Wire the keyboard to every `.overlay` on the page. Call once. */
export function initOverlayKeys(): void {
  // What had focus before each panel opened, to give it back on close.
  const opener = new WeakMap<HTMLElement, HTMLElement>();
  const watch = new MutationObserver((records) => {
    for (const r of records) {
      const overlay = r.target as HTMLElement;
      if (!overlay.hidden) {
        const before = document.activeElement;
        if (before instanceof HTMLElement && !overlay.contains(before)) opener.set(overlay, before);
        // A panel that put focus somewhere itself (the welcome panel's name
        // field, Tools' result) keeps it: observers run after the code that
        // opened it.
        // `data-autofocus` names the safe answer of a decision ("Keep
        // running"); otherwise the first control, the panel's ✕.
        if (!overlay.contains(document.activeElement)) {
          const preferred = overlay.querySelector<HTMLElement>("[data-autofocus]");
          const target = preferred && preferred.getClientRects().length > 0 ? preferred : focusables(overlay)[0];
          target?.focus();
        }
      } else {
        const back = opener.get(overlay);
        opener.delete(overlay);
        const lost = !document.activeElement || document.activeElement === document.body || overlay.contains(document.activeElement);
        if (back && lost && back.isConnected && back.getClientRects().length > 0) back.focus();
      }
    }
  });
  for (const o of document.querySelectorAll<HTMLElement>(".overlay")) {
    watch.observe(o, { attributes: true, attributeFilter: ["hidden"] });
  }

  document.addEventListener("keydown", (e) => {
    const open = openOverlays();
    if (open.length === 0) return;
    if (e.key === "Escape") {
      const id = escapeTarget(open.map((o) => o.id));
      const btn = id ? document.getElementById(id) : null;
      if (btn) {
        e.preventDefault();
        btn.click();
      }
      return;
    }
    if (e.key === "Tab") {
      // The close dialog paints above the rest; otherwise the last one.
      const top = open.find((o) => o.id === "close-overlay") ?? open[open.length - 1];
      const items = focusables(top);
      const at = items.indexOf(document.activeElement as HTMLElement);
      const to = wrapFocus(at, items.length, e.shiftKey);
      if (to !== null) {
        e.preventDefault();
        items[to].focus();
      }
    }
  });
}
