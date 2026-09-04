// Optional wallet — balances, history, receive and send, all answered by the
// user's OWN node. Off from factory settings; main.ts shows the header icon
// only while the Settings toggle is on. All numbers arrive via the tagged
// Ask states (ready/stopped/warming/unavailable) — nothing throws here.
//
// While the panel is open it polls the node every POLL_MS so confirmations tick
// up live (the whole point of "your node answers"). The poll is a SOFT refresh:
// it updates balances/history but never tears down the send form, the receive
// address, or a send confirmation the user is looking at.

import { invoke } from "@tauri-apps/api/core";
import { qrcodegen } from "./qrcodegen";
import { playReceived, playConfirmed, playSent, primeAudio } from "./sfx";

/** How often the open panel re-asks the node. Blocks are ~90s, so 20s keeps a
 *  confirmation visibly fresh without hammering the RPC. */
const POLL_MS = 20_000;

/** Below this the node is close enough that saying "catching up" is noise: a
 *  handful of blocks clears in minutes. Above it the wait is long enough that a
 *  user staring at a wrong balance deserves to be told why, and told that an
 *  open wallet is making it slower. */
const CATCHUP_WARN_BLOCKS = 50;

/** The two ends of a Mac catch-up, in blocks per hour.
 *
 *  SLOW is measured: an operator ran a real 0.34.5 catch-up and got 58 blocks
 *  an hour while their GPU sat idle, because the node spends catch-up waiting
 *  for peers to serve block BODIES, not computing. FAST is the validation
 *  ceiling for an M5 class Mac, from the measured 31.9s ExactReplay episode
 *  (3600 / 31.9). A Mac cannot go faster than that no matter how good its
 *  peers are.
 *
 *  Do NOT reuse the ~775 blocks/h figure from that operator's report. That is
 *  an RTX 5080 ceiling and it is roughly seven times what Apple Silicon does,
 *  so quoting it to a Mac user would promise a speed the machine cannot reach.
 *
 *  The gap between these two is the honest uncertainty, and it is about peers.
 *  A single point estimate reads as a promise, and this one would be wrong by
 *  about a factor of two in the user's favour or against it. */
export const CATCHUP_SLOW_PER_HOUR = 58;
export const CATCHUP_FAST_PER_HOUR = 113;

/** Nothing Apple Silicon does should imply a rate near this. Exported so the
 *  test can assert the CONSTANT directly instead of hoping a phrasing check
 *  catches a wrong value. It did not: with 775 the panel says "roughly 1 to 4
 *  days", which contains no suspicious word for a string test to find. */
export const CATCHUP_APPLE_SILICON_SANITY_CEILING = 150;

/** Turn a block gap into the plain phrase we show. Returns a RANGE, never a
 *  point, and switches to days once hours stop being readable. Exported for
 *  the tests: the arithmetic is the part that goes wrong silently. */
export const catchupEta = (blocksBehind: number): string => {
  const slowH = Math.max(1, Math.ceil(blocksBehind / CATCHUP_SLOW_PER_HOUR));
  const fastH = Math.max(1, Math.ceil(blocksBehind / CATCHUP_FAST_PER_HOUR));
  // Never print "3 to 3". When rounding collapses the range, show one figure.
  if (slowH === fastH) {
    return slowH < 48 ? `about ${slowH} hour${slowH === 1 ? "" : "s"}` : `about ${Math.round(slowH / 24)} days`;
  }
  if (slowH < 48) return `roughly ${fastH} to ${slowH} hours`;
  const fastD = Math.max(1, Math.round(fastH / 24));
  const slowD = Math.max(fastD + 1, Math.round(slowH / 24));
  return `roughly ${fastD} to ${slowD} days`;
};

/** Sound on unless explicitly muted (matches the PQ wallet's default-on rule). */
const LS_SOUND = "btxnode.sound";
const soundOn = (): boolean => localStorage.getItem(LS_SOUND) !== "off";
// Every sound is best-effort and gated: audio must never throw on a money path.
const sReceived = () => { if (soundOn()) try { playReceived(); } catch { /* ignore */ } };
const sConfirmed = () => { if (soundOn()) try { playConfirmed(); } catch { /* ignore */ } };
const sSent = () => { if (soundOn()) try { playSent(); } catch { /* ignore */ } };

type Ask<T> =
  | { state: "ready"; data: T }
  | { state: "stopped" }
  | { state: "warming" }
  | { state: "unavailable"; data: { message: string } };

interface WalletTx {
  txid: string;
  category: string;
  amount: number;
  confirmations: number;
  time: number;
}
interface WalletView {
  enabled: boolean;
  imported: boolean;
  address: string | null;
  trusted: number;
  pending: number;
  immature: number;
  backfilling: boolean;
  /** The node has not accepted a block in hours, so every figure here is old. */
  tip_stale: boolean;
  /** headers - blocks. How far the node still has to climb. */
  blocks_behind: number;
  txs: WalletTx[];
}
interface ImportResult {
  rescanned: boolean;
  warning: string | null;
}
interface CreateResult {
  address: string;
  file_path: string;
}
interface SendResult {
  txid: string;
  fee: number | null;
}

const $ = <T extends HTMLElement = HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const fmtBtx = (n: number) =>
  n.toLocaleString("en-US", { maximumFractionDigits: n >= 1000 ? 2 : 6 });
/** Exact BTX amount — up to 8 decimals (BTX's smallest unit), trailing zeros
 *  trimmed. `fmtBtx` rounds for readable balances; a spend confirmation must
 *  show the figure the node will actually spend, so it uses this instead.
 *
 *  Also the spendable CEILING. `fmtBtx` rounds half-up, so it can print a
 *  number strictly larger than the balance — and the guard compares against the
 *  unrounded one. The panel advertised a ceiling it would then refuse, and the
 *  rejection quoted the same rounded figure back, so a user typing exactly what
 *  the wallet printed was told it was too much. */
export const fmtExact = (n: number) => n.toFixed(8).replace(/\.?0+$/, "");

/** Parse a typed BTX amount to a number, or NaN if it isn't a clean amount.
 *  Exported and pure so the money rules are unit-tested, not just trusted:
 *  - a German decimal comma ("0,5") is accepted (comma == decimal separator);
 *  - more than one separator ("1.234,56", "1.5.0") is rejected as ambiguous
 *    rather than silently becoming a wrong number;
 *  - more than 8 decimals is rejected — BTX has no smaller unit, and the
 *    confirm figure must be exactly what the node spends.
 *  Note "1,000" reads as 1.0 (comma is a decimal, consistently); the exact
 *  confirmation screen is what catches a thousands-separator habit. */
export const parseBtxAmount = (raw: string): number => {
  const s = raw.trim();
  if (!s || (s.match(/[.,]/g)?.length ?? 0) > 1) return NaN;
  const norm = s.replace(",", ".");
  if (!/^\d*\.?\d{0,8}$/.test(norm)) return NaN;
  return Number(norm);
};
const esc = (s: string) =>
  s.replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] as string
  );

/** Human-readable reason for the non-ready Ask states, so every caller says the same thing. */
function askMessage(st: Exclude<Ask<unknown>, { state: "ready" }>): string {
  switch (st.state) {
    case "stopped":
      return "Start your node first — it does the work.";
    case "warming":
      return "Your node is still getting ready — try again in a moment.";
    case "unavailable":
      return st.data.message;
  }
}

/** Open a txid or address on the public explorer. The host re-validates both. */
async function openExplorer(kind: "tx" | "address", id: string): Promise<void> {
  if (!id) return;
  try {
    await invoke("wallet_open_explorer", { kind, id });
  } catch {
    /* the host refused it; there is nothing useful to say */
  }
}

/** Show/hide the header wallet icon — called from main.ts's status tick. */
export function reflectWalletEnabled(enabled: boolean): void {
  $("wallet-btn").hidden = !enabled;
}

export function initWallet(): void {
  const overlay = $("wallet-overlay");
  const importSection = $("wallet-import-section");
  const viewSection = $("wallet-view-section");
  const gate = $("wallet-gate");
  const fileInput = $<HTMLInputElement>("wallet-file");
  const importBtn = $<HTMLButtonElement>("wallet-import-btn");
  const note = $("wallet-import-note");

  const sendTo = $<HTMLInputElement>("wallet-send-to");
  const sendAmt = $<HTMLInputElement>("wallet-send-amt");
  const reviewBtn = $<HTMLButtonElement>("wallet-review-btn");
  const sendNote = $("wallet-send-note");

  /** Last balances we were told about — the Max button and the amount guard read this. */
  let spendable = 0;
  /** Set only by the Max button. Drives `subtract_fee`: a whole-balance send must
   *  take the fee OUT of the amount, or btxd needs amount+fee and rejects it. */
  let maxMode = false;
  /** The address currently shown under Receive (may be fresher than the stored one). */
  let receiveAddr = "";
  let lastQrAddr = "";

  /** Per-txid confirmation count from the last render, so we can tell a NEW
   *  incoming tx (chime) from one that just CROSSED into confirmed (chime again).
   *  `soundBaseline` gates the very first render after open/import so we don't
   *  replay a coin for every tx that already existed. */
  const txConfs = new Map<string, number>();
  let soundBaseline = false;
  let showingSent = false;
  /** setInterval handle for the open-panel poll; null when the panel is closed. */
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  // Is the send confirmation the thing the user is actually looking at?
  //
  // This used to be read back out of the DOM as `!$("wallet-sent").hidden`,
  // which is not the same question. Switching tabs hides the send PANE without
  // touching that element, so after one successful send the flag stayed true for
  // the rest of the session — and every later hard refresh took the early return
  // meant to protect the confirmation screen. A stopped node then kept rendering
  // the last known balance under "verified by your node", which this file calls
  // the most damaging sentence the panel can print.

  function showOnly(section: "import" | "view" | "gate"): void {
    importSection.hidden = section !== "import";
    viewSection.hidden = section !== "view";
    gate.hidden = section !== "gate";
  }

  // ── tabs ────────────────────────────────────────────────────────────────
  type Tab = "activity" | "receive" | "send";
  function showTab(tab: Tab): void {
    for (const t of ["activity", "receive", "send"] as Tab[]) {
      $(`wallet-pane-${t}`).hidden = t !== tab;
      $(`wallet-tab-${t}`).classList.toggle("is-on", t === tab);
    }
    // Navigating anywhere means the confirmation is no longer on screen.
    showingSent = false;
    if (tab === "receive") void ensureReceiveAddr();
    if (tab === "send") resetSend();
  }

  // ── receive ─────────────────────────────────────────────────────────────
  function renderQr(addr: string): void {
    const wrap = $("wallet-qr");
    if (addr === lastQrAddr) return;
    lastQrAddr = addr;
    wrap.replaceChildren();
    if (!addr) return;
    try {
      const qr = qrcodegen.QrCode.encodeText(addr, qrcodegen.QrCode.Ecc.MEDIUM);
      const border = 3; // quiet zone, in modules
      const px = 4;
      const dim = (qr.size + border * 2) * px;
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      const canvas = document.createElement("canvas");
      canvas.width = dim * dpr;
      canvas.height = dim * dpr;
      canvas.style.width = `${dim}px`;
      canvas.style.height = `${dim}px`;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.scale(dpr, dpr);
      // Dark-on-white regardless of the dark UI, or phone cameras struggle.
      ctx.fillStyle = "#ffffff";
      ctx.fillRect(0, 0, dim, dim);
      ctx.fillStyle = "#000000";
      for (let y = 0; y < qr.size; y++) {
        for (let x = 0; x < qr.size; x++) {
          if (qr.getModule(x, y)) ctx.fillRect((x + border) * px, (y + border) * px, px, px);
        }
      }
      wrap.appendChild(canvas);
    } catch {
      /* an unrenderable QR is cosmetic — the address text below still works */
    }
  }

  function setReceiveAddr(addr: string): void {
    receiveAddr = addr;
    $("wallet-recv-addr").textContent = addr;
    renderQr(addr);
  }

  /** Show *something* immediately (the stored address), only hitting the node if we have nothing. */
  async function ensureReceiveAddr(): Promise<void> {
    if (receiveAddr) return;
    const stored = $("wallet-address").textContent ?? "";
    if (stored) setReceiveAddr(stored);
    else await newReceiveAddr();
  }

  async function newReceiveAddr(): Promise<void> {
    const btn = $<HTMLButtonElement>("wallet-newaddr-btn");
    btn.disabled = true;
    try {
      const res = await invoke<Ask<string>>("wallet_receive_address");
      if (res.state === "ready") setReceiveAddr(res.data);
    } catch {
      /* leave whatever address is already shown */
    } finally {
      btn.disabled = false;
    }
  }

  // ── send ────────────────────────────────────────────────────────────────
  function resetSend(): void {
    $("wallet-send-form").hidden = false;
    $("wallet-confirm").hidden = true;
    $("wallet-sent").hidden = true;
    showingSent = false;
    sendNote.textContent = "";
    $("wallet-send-avail").textContent = `${fmtExact(spendable)} BTX ready to spend.`;
    validateSend();
  }

  const enteredAmount = (): number => parseBtxAmount(sendAmt.value);

  /** Enable Review only for a plausible amount. The address is judged by the node, not here. */
  function validateSend(): void {
    const amt = enteredAmount();
    const typed = sendAmt.value.trim().length > 0;
    const ok =
      sendTo.value.trim().length > 0 &&
      typed &&
      Number.isFinite(amt) &&
      amt > 0 &&
      amt <= spendable + 1e-12;
    reviewBtn.disabled = !ok;
    // Tell the user WHY Review is off, so a rejected amount isn't a dead button.
    const rawAmt = sendAmt.value.trim();
    if (typed && Number.isFinite(amt) && amt > spendable + 1e-12) {
      sendNote.textContent = `That's more than the ${fmtExact(spendable)} BTX you can spend right now.`;
    } else if (rawAmt && /\.\d{9,}$/.test(rawAmt.replace(",", "."))) {
      // BEFORE the generic branch, not after. parseBtxAmount already returns
      // NaN for more than 8 decimals, so this test could never be reached
      // second and the specific explanation never appeared: someone pasting a
      // figure with more precision than BTX has was told to "enter a plain
      // number", which is exactly what they had done.
      sendNote.textContent = "BTX goes to 8 decimal places.";
    } else if (typed && !Number.isFinite(amt)) {
      sendNote.textContent = "Enter a plain number, like 0.5.";
    } else {
      sendNote.textContent = "";
    }
  }

  reviewBtn.addEventListener("click", () => {
    const amt = enteredAmount();
    // Exact, not fmtBtx: the number the user approves must equal what's sent.
    $("wallet-confirm-amt").textContent = `${fmtExact(amt)} BTX`;
    $("wallet-confirm-to").textContent = sendTo.value.trim();
    $("wallet-confirm-note").textContent = maxMode
      ? "This is your whole spendable balance, so the network fee comes out of it — the receiver gets slightly less than the number above."
      : "The network fee is added on top, and your node picks it.";
    $("wallet-send-form").hidden = true;
    $("wallet-confirm").hidden = false;
  });

  $("wallet-cancel-btn").addEventListener("click", () => {
    $("wallet-confirm").hidden = true;
    $("wallet-send-form").hidden = false;
  });

  $("wallet-send-btn").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("wallet-send-btn");
    btn.disabled = true;
    btn.textContent = "Sending…";
    try {
      const res = await invoke<Ask<SendResult>>("wallet_send", {
        address: sendTo.value.trim(),
        amount: enteredAmount(),
        subtractFee: maxMode, // Tauri v2 maps camelCase -> snake_case
      });
      if (res.state === "ready") {
        $("wallet-confirm").hidden = true;
        $("wallet-sent").hidden = false;
        showingSent = true;
        $("wallet-sent-txid").textContent = res.data.txid;
        $("wallet-sent-fee").textContent =
          res.data.fee != null
            ? `Network fee ${fmtBtx(res.data.fee)} BTX.`
            : "";
        // Remember the just-sent tx so the poll doesn't mistake it for an
        // "arrival" and chime the incoming coin for our own outgoing spend.
        if (res.data.txid) txConfs.set(res.data.txid, 0);
        sSent();
        sendTo.value = "";
        sendAmt.value = "";
        maxMode = false;
        await refresh({ soft: true });
      } else {
        $("wallet-confirm").hidden = true;
        $("wallet-send-form").hidden = false;
        sendNote.textContent = askMessage(res);
      }
    } catch (e) {
      // A rejected send (bad address, overspend) lands here with the host's message.
      $("wallet-confirm").hidden = true;
      $("wallet-send-form").hidden = false;
      sendNote.textContent = String(e);
    } finally {
      btn.disabled = false;
      btn.textContent = "Send it";
    }
  });

  $("wallet-sent-done-btn").addEventListener("click", () => showTab("activity"));
  $("wallet-sent-txid").addEventListener("click", () =>
    void openExplorer("tx", $("wallet-sent-txid").textContent ?? "")
  );

  $("wallet-max-btn").addEventListener("click", () => {
    maxMode = true;
    sendAmt.value = spendable.toFixed(8);
    validateSend();
  });
  // Any manual edit means it is no longer a whole-balance send.
  sendAmt.addEventListener("input", () => {
    maxMode = false;
    validateSend();
  });
  sendTo.addEventListener("input", validateSend);

  /** Compare this render's txs to the last and chime for what changed: a coin
   *  for a newly-seen incoming tx, a confirm tone when an incoming tx crosses
   *  0 → confirmed. At most one of each per render so a batch never machine-guns.
   *  The first render after open/import only seeds the baseline (no sound). */
  function chimeForTxChanges(txs: WalletTx[]): void {
    if (!soundBaseline) {
      for (const t of txs) if (t.txid) txConfs.set(t.txid, t.confirmations);
      soundBaseline = true;
      return;
    }
    let arrived = false;
    let confirmed = false;
    for (const t of txs) {
      if (!t.txid) continue;
      const incoming = t.amount >= 0;
      if (!txConfs.has(t.txid)) {
        if (incoming) arrived = true; // a new deposit landed (outgoing already chimed at send)
      } else if (incoming && (txConfs.get(t.txid) ?? 0) <= 0 && t.confirmations >= 1) {
        confirmed = true; // it just got its first confirmation
      }
      txConfs.set(t.txid, t.confirmations);
    }
    if (arrived) sReceived();
    if (confirmed) sConfirmed();
  }

  // ── view ────────────────────────────────────────────────────────────────
  function renderView(d: WalletView): void {
    showOnly("view");
    spendable = d.trusted;
    $("wallet-balance").textContent = `${fmtBtx(d.trusted)} BTX`;
    $("wallet-address").textContent = d.address ?? "";
    const extras: string[] = [];
    if (d.pending > 0) extras.push(`${fmtBtx(d.pending)} incoming (unconfirmed)`);
    if (d.immature > 0) extras.push(`${fmtBtx(d.immature)} maturing`);
    // A stalled node still answers, confidently, with a balance from whenever it
    // stopped. Saying "verified by your node" over that is the most damaging
    // sentence this panel can print, because the number looks authoritative and
    // is simply old. When the tip has not moved in hours, say so instead, and
    // say it INSTEAD of the reassurance rather than beside it.
    // Three states, and they need different sentences. Stopped is not the same
    // as climbing, and climbing with the wallet open is its own problem.
    if (d.tip_stale) {
      $("wallet-sub").textContent =
        "Your node stopped following the chain, so this figure is from the last block it accepted. It is not your current balance. Nothing is lost; the node needs to catch up first.";
    } else if (d.blocks_behind >= CATCHUP_WARN_BLOCKS) {
      // The wait is a RANGE and the spread is peers, not the user's Mac. See
      // CATCHUP_SLOW_PER_HOUR. We used to print one number here, computed from
      // the slow end alone, which read as a promise and was the pessimistic
      // case stated as the expected one.
      // Naming the cause matters as much as the number: a user who thinks a
      // slow catch-up is their hardware buys a faster Mac and gets the same
      // result, because the node is waiting on other people's nodes.
      // And an open wallet updates on every connected block, which slowed that
      // operator's node enough that it fell behind the tip and began
      // self-forking, while the wallet calls hung so the balance was unreadable
      // anyway. So the honest advice is to close it and let the node work.
      $("wallet-sub").textContent =
        `Your node is still catching up, about ${d.blocks_behind.toLocaleString("en-US")} blocks ` +
        `to go, ${catchupEta(d.blocks_behind)}. How long it really takes depends on how fast ` +
        `other nodes send blocks, not on how fast your computer is. This figure is not current until ` +
        `it finishes. Keeping the wallet open slows the catch-up down, so if you are not using ` +
        `it right now, closing it gets you to a correct balance sooner.`;
    } else {
      $("wallet-sub").textContent =
        extras.length > 0 ? extras.join(" · ") : "Confirmed and spendable, verified by your node.";
    }

    const list = $("wallet-txs");
    if (d.txs.length === 0) {
      list.innerHTML = `<div class="ask-line">No transactions found yet.</div>`;
    } else {
      list.innerHTML = d.txs
        .map((t) => {
          const incoming = t.amount >= 0;
          const when = t.time > 0 ? new Date(t.time * 1000).toLocaleDateString() : "";
          const conf =
            t.confirmations <= 0 ? "unconfirmed" : `${t.confirmations.toLocaleString("en-US")} conf`;
          // Rows without a txid can't be looked up, so they aren't clickable.
          const cls = t.txid ? "wallet-tx wallet-tx-link" : "wallet-tx";
          const data = t.txid ? ` data-txid="${esc(t.txid)}"` : "";
          return (
            `<div class="${cls}"${data}>` +
            `<span class="amt ${incoming ? "in" : "out"}">${incoming ? "+" : ""}${fmtBtx(t.amount)} BTX</span>` +
            `<span class="meta">${esc(t.category)} · ${conf}${when ? " · " + when : ""}</span>` +
            `</div>`
          );
        })
        .join("");
    }

    const caveat = $("wallet-caveat");
    caveat.hidden = !d.backfilling;
    if (d.backfilling) {
      caveat.textContent =
        "Your node is still backfilling older history in the background — the balance and list may still be filling in, so hold off on sending your whole balance until it settles.";
    }
    if (!$("wallet-pane-send").hidden) {
      $("wallet-send-avail").textContent = `${fmtExact(spendable)} BTX ready to spend.`;
    }

    chimeForTxChanges(d.txs);
  }

  // One delegated listener beats one per row, and it survives every re-render.
  $("wallet-txs").addEventListener("click", (e) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>(".wallet-tx-link");
    if (row?.dataset.txid) void openExplorer("tx", row.dataset.txid);
  });
  $("wallet-address").addEventListener("click", () =>
    void openExplorer("address", $("wallet-address").textContent ?? "")
  );

  $("wallet-copy-btn").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("wallet-copy-btn");
    try {
      await navigator.clipboard.writeText(receiveAddr);
      btn.textContent = "Copied";
      setTimeout(() => (btn.textContent = "Copy address"), 1500);
    } catch {
      btn.textContent = "Couldn't copy";
      setTimeout(() => (btn.textContent = "Copy address"), 1500);
    }
  });
  $("wallet-newaddr-btn").addEventListener("click", () => void newReceiveAddr());

  $("wallet-tab-activity").addEventListener("click", () => showTab("activity"));
  $("wallet-tab-receive").addEventListener("click", () => showTab("receive"));
  $("wallet-tab-send").addEventListener("click", () => showTab("send"));

  // `soft` refreshes (the poll, post-send) only re-render on a Ready+imported
  // status and otherwise do nothing — they must never yank the user out of a
  // form or off a confirmation because the node hiccuped for one tick. A hard
  // refresh (open, create, import, forget) is allowed to show the gate/import.
  async function refresh(opts?: { soft?: boolean }): Promise<void> {
    const soft = opts?.soft === true;
    // A just-completed send shows the "sent" confirmation with a clickable txid;
    // a transient non-ready status must not tear that screen down either.
    const keepSent = showingSent;
    try {
      const st = await invoke<Ask<WalletView>>("wallet_status");
      if (st.state === "ready") {
        if (st.data.imported) renderView(st.data);
        else if (!soft) showOnly("import");
        return;
      }
      if (soft || keepSent) return;
      showOnly("gate");
      $("wallet-gate-msg").textContent =
        st.state === "stopped"
          ? "Start your node to see your wallet."
          : st.state === "warming"
            ? "Your node is still getting ready — your wallet appears when it's up."
            : askMessage(st);
    } catch (e) {
      if (soft || keepSent) return;
      showOnly("gate");
      $("wallet-gate-msg").textContent = String(e);
    }
  }

  function startPoll(): void {
    stopPoll();
    pollTimer = setInterval(() => void refresh({ soft: true }), POLL_MS);
  }
  function stopPoll(): void {
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  $<HTMLButtonElement>("wallet-create-btn").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("wallet-create-btn");
    btn.disabled = true;
    btn.textContent = "Creating in your node…";
    try {
      const res = await invoke<Ask<CreateResult>>("wallet_create");
      if (res.state === "ready") {
        note.textContent = `Wallet created. Its .btxwallet file is on your Desktop — that file IS your money, keep it safe.`;
        soundBaseline = false; // re-seed silently for the new wallet
        txConfs.clear();
        await refresh();
      } else {
        note.textContent = askMessage(res);
      }
    } catch (e) {
      note.textContent = String(e);
    } finally {
      btn.disabled = false;
      btn.textContent = "Create a new wallet";
    }
  });

  fileInput.addEventListener("change", () => {
    importBtn.disabled = !fileInput.files || fileInput.files.length === 0;
    note.textContent = "";
  });

  /** Largest file we will even read. Mirrors MAX_IMPORT_BYTES in
   *  src-tauri/src/wallet.rs, which refuses anything bigger. Checking it here
   *  too is not belt and braces, it is the difference between a one line
   *  refusal and a dead window. Encoding a 200 MB pick takes the webview past
   *  1.3 GB of resident memory and blocks the main thread for seconds before
   *  the host ever gets to say no, and a pick over roughly 384 MB dies inside
   *  btoa with "RangeError: Invalid string length", which is then all the user
   *  is shown. file.size costs nothing and answers before any of that. */
  const MAX_IMPORT_BYTES = 64 * 1024 * 1024;

  const humanBytes = (n: number): string => {
    const mb = n / (1024 * 1024);
    if (mb >= 1) return `${mb >= 10 ? Math.round(mb) : mb.toFixed(1)} MB`;
    return `${Math.max(1, Math.round(n / 1024))} KB`;
  };

  /// Base64 a File without blowing the stack. The obvious one-liner,
  /// btoa(String.fromCharCode(...bytes)), spreads every byte as an argument and
  /// throws RangeError on a real wallet.dat, which is megabytes. Chunked so the
  /// argument count stays bounded regardless of file size. 32768 sits well
  /// inside the limit on both engines Tauri ships against: measured 108273
  /// arguments on V8 two thousand frames deep, 610684 on JavaScriptCore.
  async function fileToBase64(f: File): Promise<string> {
    const bytes = new Uint8Array(await f.arrayBuffer());
    const CHUNK = 0x8000;
    let binary = "";
    for (let i = 0; i < bytes.length; i += CHUNK) {
      binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
    }
    return btoa(binary);
  }

  /** The node rescans the chain on every import, and that scan routinely
   *  outlives the app's 60 second RPC wait. The raw failure then reads
   *  "http error: error sending request for url (http://127.0.0.1:...):
   *  operation timed out", which names a port instead of telling a frightened
   *  person what became of their money, and it reads as a failure when the
   *  import may well be succeeding. Say the true thing, and say the one thing
   *  they must not do, which is import again on top of a running scan. */
  const scanMayStillBeRunning = (m: string): boolean =>
    /timed out|timeout|error sending request|connection (closed|reset|refused)/i.test(m);

  const SCAN_STILL_RUNNING_NOTE =
    "Your node is still scanning the chain for this wallet's history. That scan can run for " +
    "many minutes and it outlasted the app's wait, so this is not proof the import failed. " +
    "Leave the node running, give it time, then reopen this panel. Do not import again while " +
    "the scan is going.";

  importBtn.addEventListener("click", async () => {
    const file = fileInput.files?.[0];
    if (!file) return;
    if (file.size === 0) {
      note.textContent = "That file is empty.";
      return;
    }
    if (file.size > MAX_IMPORT_BYTES) {
      note.textContent =
        `That file is ${humanBytes(file.size)}. A wallet file is kilobytes to a few megabytes, ` +
        `so this is not one. The most this accepts is ${humanBytes(MAX_IMPORT_BYTES)}.`;
      return;
    }
    importBtn.disabled = true;
    fileInput.disabled = true;
    importBtn.textContent = "Importing…";
    // The node rescans before it answers, so the invoke below can sit for
    // minutes on a synced chain. Say so. Silence on a money screen reads as a
    // hang, and a user who force quits here leaves staged key material behind.
    note.textContent =
      "Handing the file to your node. It has to scan the chain for this wallet's history, " +
      "which can take several minutes. Keep the app open.";
    try {
      // Read BYTES, not text. A wallet.dat is binary, and file.text() decodes
      // as UTF-8, so every byte that is not valid UTF-8 became U+FFFD before
      // the host ever saw it. That silently destroyed the one file format the
      // maintainer told users would "work everywhere BTX does".
      const contentB64 = await fileToBase64(file);
      const res = await invoke<Ask<ImportResult>>("wallet_import", { contentB64 });
      if (res.state === "ready") {
        note.textContent = res.data.rescanned
          ? ""
          : "Imported. Your node will fill in the history as it catches up.";
        if (res.data.warning) note.textContent += ` ${res.data.warning}`;
        // A rescan surfaces the whole history at once — seed silently, don't
        // chime a coin for every past deposit.
        soundBaseline = false;
        txConfs.clear();
        await refresh();
      } else {
        const m = askMessage(res);
        note.textContent = scanMayStillBeRunning(m) ? SCAN_STILL_RUNNING_NOTE : m;
      }
    } catch (e) {
      const m = String(e);
      note.textContent = scanMayStillBeRunning(m)
        ? SCAN_STILL_RUNNING_NOTE
        : `The import could not be completed. Your node reported: ${m}`;
    } finally {
      importBtn.disabled = false;
      fileInput.disabled = false;
      importBtn.textContent = "Import wallet";
    }
  });

  // Close this wallet — two-step, so a wallet is never dropped on a stray click.
  const forgetConfirm = $("wallet-forget-confirm");
  $("wallet-forget-btn").addEventListener("click", () => {
    forgetConfirm.hidden = false;
    forgetConfirm.scrollIntoView({ block: "nearest" });
  });
  $("wallet-forget-no").addEventListener("click", () => (forgetConfirm.hidden = true));
  $("wallet-forget-yes").addEventListener("click", async () => {
    forgetConfirm.hidden = true;
    try {
      await invoke("wallet_forget");
    } finally {
      fileInput.value = "";
      importBtn.disabled = true;
      receiveAddr = "";
      lastQrAddr = "";
      soundBaseline = false; // next wallet re-seeds silently
      txConfs.clear();
      await refresh();
    }
  });

  // Explorer + copy for the sent txid and the wallet's own address.
  $("wallet-sent-explorer-btn").addEventListener("click", () =>
    void openExplorer("tx", $("wallet-sent-txid").textContent ?? "")
  );
  wireCopy("wallet-sent-copy-btn", () => $("wallet-sent-txid").textContent ?? "", "Copy id");
  $("wallet-addr-explorer-btn").addEventListener("click", () =>
    void openExplorer("address", $("wallet-address").textContent ?? "")
  );
  wireCopy("wallet-addr-copy-btn", () => $("wallet-address").textContent ?? "", "Copy address");

  // Sound on/off — remembered in localStorage, previews the sent blip when turned on.
  const soundBtn = $<HTMLButtonElement>("wallet-sound-btn");
  function reflectSound(): void {
    const on = soundOn();
    soundBtn.setAttribute("aria-pressed", String(on));
    soundBtn.textContent = on ? "🔊 Sound" : "🔇 Muted";
  }
  soundBtn.addEventListener("click", () => {
    const next = soundOn() ? "off" : "on";
    localStorage.setItem(LS_SOUND, next);
    reflectSound();
    if (next === "on") sSent(); // a tiny preview so you hear what you enabled
  });
  reflectSound();

  $("wallet-btn").addEventListener("click", () => {
    if (soundOn()) primeAudio(); // unlock audio inside this gesture, before the poll chimes
    overlay.hidden = false;
    forgetConfirm.hidden = true; // never reopen straight into the destructive confirm
    showTab("activity");
    void refresh();
    startPoll();
  });
  function closeOverlay(): void {
    overlay.hidden = true;
    showingSent = false;
    forgetConfirm.hidden = true; // reset the two-step so it starts collapsed next time
    stopPoll();
  }
  $("wallet-close").addEventListener("click", closeOverlay);
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) closeOverlay();
  });
}

/** Wire a copy button to the current text of a target, with a brief "Copied". */
function wireCopy(btnId: string, getText: () => string, label: string): void {
  const btn = document.getElementById(btnId) as HTMLButtonElement | null;
  if (!btn) return;
  btn.addEventListener("click", async () => {
    const text = getText().trim();
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      btn.textContent = "Copied";
    } catch {
      btn.textContent = "Couldn't copy";
    }
    setTimeout(() => (btn.textContent = label), 1500);
  });
}
