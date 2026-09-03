// "Ask your node": the FAQ-accordion overlay. Every answer renders a headline
// number, one plain-language line, and a citation pill — data comes from the
// ask_* Tauri commands (the local node's RPC or pure consensus math), never an
// external service. Commands resolve to a tagged Ask state; nothing throws.

import { invoke } from "@tauri-apps/api/core";

// ── Wire types (mirror apps/node/src-tauri/src/ask.rs) ──────────────────────

type Ask<T> =
  | { state: "ready"; data: T }
  | { state: "stopped" }
  | { state: "warming" }
  | { state: "unavailable"; data: { message: string } };

interface ChainProgress {
  height: number;
  headers: number;
  progress: number;
  near_tip: boolean;
  peers: number;
}
interface SupplyAnswer {
  mined_btx: number;
  cap_btx: number;
  pct: number;
  height: number;
}
interface HalvingAnswer {
  blocks_remaining: number;
  at_height: number;
  est_secs: number;
  from_reward_btx: number;
  to_reward_btx: number;
  height: number;
}
interface FeesAnswer {
  feerate_btx_kvb: number | null;
  mempool_txs: number;
  mempool_vsize: number;
}
interface MiningAnswer {
  difficulty: number;
  network_hashps: number;
  height: number;
}
interface BlockAnswer {
  height: number;
  hash: string;
  time: number;
  n_tx: number;
  size: number;
}
type TxLookup =
  | {
      kind: "found";
      txid: string;
      confirmations: number;
      block_height: number | null;
      block_time: number | null;
      vsize: number;
      vin_count: number;
      vout_count: number;
      total_out_btx: number;
    }
  | { kind: "needs_index" }
  | { kind: "building"; pct: number }
  | { kind: "not_found" };
interface TxIndexAnswer {
  enabled: boolean;
  configured: boolean;
  synced: boolean;
  pct: number;
}

// ── Free-text router (pure, unit-tested) ────────────────────────────────────

export type AskQueryKind = "empty" | "height" | "hash_or_txid" | "invalid";

export function classifyAskQuery(q: string): { kind: AskQueryKind } {
  const t = q.trim().replace(/,/g, "");
  if (t === "") return { kind: "empty" };
  if (/^\d+$/.test(t)) return { kind: "height" };
  if (/^[0-9a-fA-F]{64}$/.test(t)) return { kind: "hash_or_txid" };
  return { kind: "invalid" };
}

// ── Formatting ───────────────────────────────────────────────────────────────

const fmtInt = (n: number) => n.toLocaleString("en-US");
const fmtBtx = (n: number) =>
  n.toLocaleString("en-US", { maximumFractionDigits: n >= 1000 ? 0 : 4 });
const fmtWhen = (unixSecs: number) => new Date(unixSecs * 1000).toLocaleString();
/** ~90 s blocks → a rough, honest date ("around March 2027"). */
function fmtEta(estSecs: number): string {
  const d = new Date(Date.now() + estSecs * 1000);
  return d.toLocaleDateString("en-US", { year: "numeric", month: "long" });
}
const esc = (s: string) =>
  s.replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] as string
  );

// ── Rendering ────────────────────────────────────────────────────────────────

const $ = <T extends HTMLElement = HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

function cite(source: string): string {
  return `<div class="ask-cite"><span class="cite-dot"></span> via ${esc(source)}</div>`;
}

function answerHtml(headline: string, line: string, source: string): string {
  return (
    `<div class="ask-headline">${headline}</div>` +
    `<div class="ask-line">${line}</div>` +
    cite(source)
  );
}

function stateHtml(a: Ask<unknown>): string | null {
  switch (a.state) {
    case "stopped":
      return `<div class="ask-line">Start your node to ask it questions.</div>`;
    case "warming":
      return `<div class="ask-line">Your node is still catching up — ask again in a moment.</div>`;
    case "unavailable":
      return `<div class="ask-line">${esc(a.data.message)}</div>`;
    default:
      return null;
  }
}

// Renderers per question — each returns the .ask-a innerHTML.

function renderChain(a: Ask<ChainProgress>): string {
  const s = stateHtml(a);
  if (s) return s;
  const d = (a as { state: "ready"; data: ChainProgress }).data;
  const pct = Math.min(100, d.progress * 100);
  const head = d.near_tip
    ? `Block ${fmtInt(d.height)} — at the tip`
    : `Block ${fmtInt(d.height)} of ~${fmtInt(d.headers)}`;
  const line = d.near_tip
    ? `Your copy of the chain is current, connected to ${fmtInt(d.peers)} peer${d.peers === 1 ? "" : "s"}.`
    : `Catching up — about ${pct.toFixed(1)}% verified so far.`;
  return answerHtml(head, line, "getblockchaininfo + getchainstates");
}

function renderSupply(a: Ask<SupplyAnswer>): string {
  const s = stateHtml(a);
  if (s) return s;
  const d = (a as { state: "ready"; data: SupplyAnswer }).data;
  return answerHtml(
    `${fmtBtx(d.mined_btx)} BTX`,
    `Mined so far, of the ${fmtBtx(d.cap_btx)} cap — ${(d.pct * 100).toFixed(1)}%, from the subsidy schedule at block ${fmtInt(d.height)}.`,
    "block height × subsidy schedule"
  );
}

function renderHalving(a: Ask<HalvingAnswer>): string {
  const s = stateHtml(a);
  if (s) return s;
  const d = (a as { state: "ready"; data: HalvingAnswer }).data;
  return answerHtml(
    `${fmtInt(d.blocks_remaining)} blocks to go`,
    `At block ${fmtInt(d.at_height)} the reward goes ${fmtBtx(d.from_reward_btx)} → ${fmtBtx(d.to_reward_btx)} BTX — around ${fmtEta(d.est_secs)} at 90-second blocks.`,
    "block height math"
  );
}

function renderFees(a: Ask<FeesAnswer>): string {
  const s = stateHtml(a);
  if (s) return s;
  const d = (a as { state: "ready"; data: FeesAnswer }).data;
  if (d.feerate_btx_kvb === null) {
    return answerHtml(
      d.mempool_txs === 0 ? "Quiet right now" : `${fmtInt(d.mempool_txs)} waiting`,
      d.mempool_txs === 0
        ? "The mempool is empty — not enough recent activity for a fee estimate."
        : `Not enough recent activity for an estimate; the mempool holds ${fmtInt(d.mempool_txs)} transactions (${fmtInt(d.mempool_vsize)} vB).`,
      "getmempoolinfo"
    );
  }
  return answerHtml(
    `${(d.feerate_btx_kvb * 100_000).toFixed(2)} sats/vB`,
    `Estimated for ~6-block confirmation; ${fmtInt(d.mempool_txs)} transaction${d.mempool_txs === 1 ? "" : "s"} waiting.`,
    "estimatesmartfee + getmempoolinfo"
  );
}

function renderMining(a: Ask<MiningAnswer>): string {
  const s = stateHtml(a);
  if (s) return s;
  const d = (a as { state: "ready"; data: MiningAnswer }).data;
  const hs =
    d.network_hashps >= 1e6
      ? `${(d.network_hashps / 1e6).toFixed(1)}M`
      : fmtInt(Math.round(d.network_hashps));
  return answerHtml(
    `Difficulty ${d.difficulty.toLocaleString("en-US", { maximumFractionDigits: 2 })}`,
    `The network is solving about ${hs} matrix problems per second (an estimate from recent blocks, at height ${fmtInt(d.height)}).`,
    "getmininginfo"
  );
}

function renderBlock(a: Ask<BlockAnswer>): string {
  const s = stateHtml(a);
  if (s) return s;
  const d = (a as { state: "ready"; data: BlockAnswer }).data;
  return answerHtml(
    `Block ${fmtInt(d.height)}`,
    `${fmtWhen(d.time)} · ${fmtInt(d.n_tx)} transaction${d.n_tx === 1 ? "" : "s"} · ${fmtInt(d.size)} bytes<br><span class="ask-hash">${esc(d.hash)}</span>`,
    "getblockhash + getblock"
  );
}

function renderTx(a: Ask<TxLookup>): string {
  const s = stateHtml(a);
  if (s) return s;
  const d = (a as { state: "ready"; data: TxLookup }).data;
  switch (d.kind) {
    case "found": {
      const conf =
        d.confirmations === 0
          ? "In the mempool — not confirmed yet"
          : `${fmtInt(d.confirmations)} confirmation${d.confirmations === 1 ? "" : "s"}`;
      const where = d.block_height !== null ? ` · block ${fmtInt(d.block_height)}` : "";
      const when = d.block_time !== null ? ` · ${fmtWhen(d.block_time)}` : "";
      return answerHtml(
        conf,
        `${d.vin_count} in → ${d.vout_count} out · ${fmtBtx(d.total_out_btx)} BTX total moved · ${fmtInt(d.vsize)} vB${where}${when}`,
        "getrawtransaction"
      );
    }
    case "building":
      return `<div class="ask-line">Building the transaction index… ${(d.pct * 100).toFixed(0)}% — your node stays running; this works when it finishes.</div>`;
    case "needs_index":
      return (
        `<div class="ask-line">Historical lookups need <strong>Explorer mode</strong>: your node builds a one-time transaction index in the background (extra disk, node stays usable). Mempool transactions work without it.</div>` +
        `<button id="ask-enable-explorer" class="btn-secondary" type="button">Turn on Explorer mode</button>`
      );
    case "not_found":
      return `<div class="ask-line">No transaction with that id on your node.</div>`;
  }
}

// ── Wiring ───────────────────────────────────────────────────────────────────

type AskKey = "chain" | "supply" | "halving" | "fees" | "mining" | "block" | "tx";

async function fetchAnswer(key: AskKey, arg?: string): Promise<string> {
  try {
    switch (key) {
      case "chain":
        return renderChain(await invoke<Ask<ChainProgress>>("ask_chain_progress"));
      case "supply":
        return renderSupply(await invoke<Ask<SupplyAnswer>>("ask_supply"));
      case "halving":
        return renderHalving(await invoke<Ask<HalvingAnswer>>("ask_next_halving"));
      case "fees":
        return renderFees(await invoke<Ask<FeesAnswer>>("ask_fees"));
      case "mining":
        return renderMining(await invoke<Ask<MiningAnswer>>("ask_mining"));
      case "block":
        return renderBlock(await invoke<Ask<BlockAnswer>>("ask_block", { query: arg ?? "" }));
      case "tx": {
        if (!arg) {
          return `<div class="ask-line">Paste a transaction id (64 hex characters) in the box below, then Look up.</div>`;
        }
        return renderTx(await invoke<Ask<TxLookup>>("ask_transaction", { txid: arg }));
      }
    }
  } catch (e) {
    return `<div class="ask-line">${esc(String(e))}</div>`;
  }
}

export function initAsk(): void {
  const overlay = $("ask-overlay");
  let indexPoll: ReturnType<typeof setInterval> | undefined;

  const rowOf = (key: AskKey) =>
    overlay.querySelector<HTMLElement>(`.ask-item[data-ask="${key}"]`)!;

  function collapse(item: HTMLElement): void {
    item.querySelector(".ask-q")!.setAttribute("aria-expanded", "false");
    (item.querySelector(".ask-a") as HTMLElement).hidden = true;
  }

  async function expand(key: AskKey, arg?: string): Promise<void> {
    const item = rowOf(key);
    const btn = item.querySelector(".ask-q")!;
    const body = item.querySelector(".ask-a") as HTMLElement;
    btn.setAttribute("aria-expanded", "true");
    body.hidden = false;
    body.innerHTML = `<div class="ask-line ask-loading">Asking your node…</div>`;
    body.innerHTML = await fetchAnswer(key, arg);
    wireExplorerButton(body);
  }

  // The just-in-time Explorer-mode prompt lives inside the tx answer.
  function wireExplorerButton(scope: HTMLElement): void {
    const btn = scope.querySelector<HTMLButtonElement>("#ask-enable-explorer");
    if (!btn) return;
    btn.addEventListener("click", async () => {
      btn.disabled = true;
      btn.textContent = "Turning on… (restarts the node)";
      try {
        await invoke("set_explorer_mode", { on: true });
        startIndexPoll();
      } catch (e) {
        btn.disabled = false;
        btn.textContent = "Turn on Explorer mode";
        scope.insertAdjacentHTML(
          "beforeend",
          `<div class="ask-line">${esc(String(e))}</div>`
        );
      }
    });
  }

  // While the index builds, keep the tx row's progress line fresh.
  function startIndexPoll(): void {
    stopIndexPoll();
    const body = rowOf("tx").querySelector(".ask-a") as HTMLElement;
    const tickOnce = async () => {
      try {
        const st = await invoke<Ask<TxIndexAnswer>>("ask_tx_index_status");
        if (st.state !== "ready") return;
        if (st.data.synced) {
          stopIndexPoll();
          body.innerHTML = `<div class="ask-line">Explorer mode is ready — paste a transaction id below and Look up.</div>`;
          return;
        }
        body.innerHTML = `<div class="ask-line">Building the transaction index… ${(st.data.pct * 100).toFixed(0)}% — your node stays running.</div>`;
      } catch {
        /* keep polling; transient during the restart */
      }
    };
    void tickOnce();
    indexPoll = setInterval(() => void tickOnce(), 3000);
  }
  function stopIndexPoll(): void {
    if (indexPoll !== undefined) clearInterval(indexPoll);
    indexPoll = undefined;
  }

  // Accordion: tap toggles; expanding re-fetches (fresh numbers every open).
  overlay.querySelectorAll<HTMLElement>(".ask-item").forEach((item) => {
    const key = item.dataset.ask as AskKey;
    item.querySelector(".ask-q")!.addEventListener("click", () => {
      const expanded =
        item.querySelector(".ask-q")!.getAttribute("aria-expanded") === "true";
      if (expanded) collapse(item);
      else void expand(key);
    });
  });

  // Free-text router: height → block row; 64-hex → block first, then tx.
  async function lookup(): Promise<void> {
    const input = $<HTMLInputElement>("ask-search");
    const q = input.value;
    const cls = classifyAskQuery(q);
    if (cls.kind === "empty") return;
    if (cls.kind === "invalid" || cls.kind === "height") {
      // ask_block answers invalid input with its calm one-line explanation.
      await expand("block", q);
      return;
    }
    // 64-hex: try a block hash first; fall through to a tx lookup.
    const blockAns = await invoke<Ask<BlockAnswer>>("ask_block", { query: q.trim() });
    if (blockAns.state === "ready") {
      const item = rowOf("block");
      item.querySelector(".ask-q")!.setAttribute("aria-expanded", "true");
      const body = item.querySelector(".ask-a") as HTMLElement;
      body.hidden = false;
      body.innerHTML = renderBlock(blockAns);
      return;
    }
    await expand("tx", q.trim().toLowerCase());
  }
  $("ask-search-btn").addEventListener("click", () => void lookup());
  $<HTMLInputElement>("ask-search").addEventListener("keydown", (e) => {
    if (e.key === "Enter") void lookup();
  });

  // Node-stopped gate: probed once per open with the lightest command
  // (ask_supply → one getchainstates call; pure math otherwise).
  async function refreshGate(): Promise<void> {
    try {
      const probe = await invoke<Ask<SupplyAnswer>>("ask_supply");
      $("ask-gate").hidden = probe.state !== "stopped";
    } catch {
      $("ask-gate").hidden = true; // outside Tauri (browser QA) — no gate
    }
  }
  $("ask-start-btn").addEventListener("click", async () => {
    const b = $<HTMLButtonElement>("ask-start-btn");
    b.disabled = true;
    b.textContent = "Starting…";
    try {
      await invoke("start_node");
      await refreshGate();
    } catch (e) {
      console.error("start from ask panel failed", e);
    } finally {
      b.disabled = false;
      b.textContent = "Start node";
    }
  });

  // Open / close.
  $("ask-btn").addEventListener("click", () => {
    overlay.hidden = false;
    void refreshGate();
  });
  $("ask-close").addEventListener("click", () => {
    overlay.hidden = true;
    stopIndexPoll();
  });
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) {
      overlay.hidden = true;
      stopIndexPoll();
    }
  });

  $("ask-global-stats").addEventListener("click", () => {
    void invoke("open_global_stats").catch(() => {});
  });

  // QA hook (harmless in production, mirrors __ambient/__core).
  (window as unknown as Record<string, unknown>).__ask = {
    open: () => {
      overlay.hidden = false;
    },
  };
}
