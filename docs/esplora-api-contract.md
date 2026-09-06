<!-- Ported verbatim from MendeMatthias/btx-esplora, docs/BTX-Esplora-API-Contract.md, at commit c77fa4011863a2cd8a8d8c9cee45e910d25fa0c8 (see deploy/esplora/PROVENANCE.md). It documents the API the vendored electrs fork serves. Nothing below this line was changed; where it says "our" it means that deployment. -->

# BTX Esplora API Contract

**Status:** Authoritative — external interface contract
**Audience:** Adapter authors (Byron / First Light node), btx-esplora reimplementers, incumbent minebtx operators
**Scope:** The HTTP surface the PQ wallet and btxscan consume, precise enough that an adapter author never has to guess a field.

---

## 1. Overview

### 1.1 What this is

This is the wire contract for a **BTX Esplora endpoint** — the read/broadcast HTTP API that a post-quantum BTX wallet and the btxscan explorer both speak. Two independent implementations already exist and MUST both satisfy this contract:

- **minebtx** — the incumbent explorer at `https://explorer.minebtx.com/api` (the live shapes in this doc were captured from it).
- **our electrs** — the btx-esplora reimplementation.

The contract exists so a **third** implementation — Byron's **First Light** node, fronted by an adapter that maps First Light's native API onto these routes — can be a **drop-in wallet fallback**. If an adapter serves these routes with these exact JSON paths, types, and error semantics, the wallet and btxscan work against it unchanged. Anything under-specified here is a place the wallet could silently misbehave, so this doc is deliberately field-exhaustive.

This is an Esplora **superset**. Standard Blockstream/electrs Esplora clients ignore unknown keys, so the BTX-specific additions (`witness_v2_p2mr`, `matmul_*`, shielding fields, `subsidy_sat`, …) are additive and safe. The parts that *diverge* from canonical Esplora — no weight discount, JSON error envelope, txid-cursor-only pagination — are called out explicitly in §4 and MUST be reproduced.

### 1.2 Base-URL convention

Routes may be mounted **under `/api`** or **at root** — an adapter MAY choose, and clients are configured with whichever base URL applies. **Both** the incumbent minebtx **and** our electrs mount under **`/api`** (e.g. `https://explorer.minebtx.com/api/address/<a>`). All paths in this document are written relative to that base; prepend the deployment's base URL (`…/api` or `…`).

The client sends the route path exactly as specified — it does not rewrite, add, or drop path segments — so an adapter must match the literal paths below (including the mandatory explicit index forms like `/block/:hash/txs/0`, see §3.7).

### 1.3 CORS

**CORS MUST be open for GET.** The wallet issues cross-origin GETs from a Tauri webview and btxscan from a browser SPA; both require `Access-Control-Allow-Origin: *` (or an echo of the caller origin) on all GET routes, plus the matching preflight allowance where the client sends a preflight. `POST /tx` is sent as a simple `text/plain` request to avoid a preflight, but permissive CORS on it is still recommended.

### 1.4 Content types

| Direction | Type |
|---|---|
| GET JSON routes | `application/json` |
| GET text routes (`/blocks/tip/height`, `/blocks/tip/hash`, `/block-height/:h`) | `text/plain`, bare value, at most a trailing newline |
| `POST /tx` request body | `text/plain` — raw tx hex (or `{"hex":"..."}`, see §2.7) |
| `POST /tx` success response | `text/plain` — bare 64-hex txid (see §2.7, §4.6) |
| All error bodies | **`application/json`**, envelope `{"error":"<human-readable text>"}`, on both 4xx and 5xx (see §4.6) |

### 1.5 Sats, never floats

All monetary values are **integer satoshis**, transported either as a JSON integer or an **all-digit string**. The wallet parses them through a strict `toSat`/`satFromExplorer` gate that accepts *only* an all-digits string or a non-negative integer Number and **throws on anything else** (a float, `null` where a value is required, `"0x..."`, whitespace). Amounts are never silently coerced to 0. An adapter that emits `1.5e8`, a float, or a quoted non-digit string for a required value will crash the money path by design.

---

## 2. Wallet routes (the 7)

These seven routes are the entire chain I/O surface of the PQ wallet. Every route+method not in this allowlist is rejected by the wallet's own proxy before it reaches the network, so an adapter only ever sees these. Money-path-critical routes are flagged.

Route legend: `<a>` = a `btx1z…` address (§4.1); `<txid>` = 64-hex lowercase transaction id.

---

### 2.1 `GET /address/<a>` — address summary / balance

**Purpose:** Authoritative confirmed balance and a pending-activity signal. `confirmedSat = chain_stats.funded_txo_sum − chain_stats.spent_txo_sum` is the headline spendable balance. `mempool_stats.tx_count > 0` (or `chain_stats.tx_count`) triggers a follow-up `/txs` fetch; the mempool deltas are read but deliberately **not trusted for direction** (minebtx under-reports the spent side, so a self-send's change can look like incoming).

**Real response** (chain tip height 161391 at capture):

```json
{
  "address": "btx1z7nkymajxh9s089hm8f6ztasptx2nwlmgqqeh9ruxpn6klh3qa55sxvmjs5",
  "chain_stats": {
    "funded_txo_count": 3,
    "funded_txo_sum": 773920205,
    "spent_txo_count": 2,
    "spent_txo_sum": 702500552,
    "tx_count": 4
  },
  "mempool_stats": {
    "funded_txo_count": 0, "funded_txo_sum": 0,
    "spent_txo_count": 0, "spent_txo_sum": 0, "tx_count": 0
  }
}
```

**Required fields:**

| Field | JSON path | Type | Why the wallet needs it |
|---|---|---|---|
| Confirmed funded | `chain_stats.funded_txo_sum` | int/all-digit str, **present even when 0** | Money-path: numerator of confirmed balance. Absent ⇒ parser throws. |
| Confirmed spent | `chain_stats.spent_txo_sum` | int/all-digit str, present even when 0 | Money-path: subtracted → `confirmedSat`. |
| Confirmed tx count | `chain_stats.tx_count` | int | "Is history empty?" gate. |
| Mempool funded | `mempool_stats.funded_txo_sum` | int/all-digit str (optional) | Pending delta (advisory only). |
| Mempool spent | `mempool_stats.spent_txo_sum` | int/all-digit str (optional) | Pending delta (advisory only). |
| Mempool tx count | `mempool_stats.tx_count` | int | `>0` ⇒ fetch `/txs` to classify pending activity. |

btxscan additionally computes `chain.funded − chain.spent + (mempool.funded − mempool.spent)` and therefore requires **all four `*_sum` fields**. Emit the full 5-field bucket for both `chain_stats` and `mempool_stats`, all counts/sums present (0, not absent) on an empty address.

---

### 2.2 `GET /address/<a>/txs` — newest confirmed + mempool page

**Purpose:** Page-1 transaction history: pending-tx classification, carrier/artifact discovery, UTXO cross-checks, and the history render. **Money-path-adjacent:** this is the same route the wallet uses to find artifact-**carrier** outpoints; a carrier spent as an ordinary input would **burn the NFT**, so any UTXO whose originating tx cannot be seen here (or via `/tx`) is set aside `unverified` and never spent.

**Page-1 size:** up to **25 confirmed** plus up to **50 mempool** transactions, newest first (see §4.5). Further pages come only via the `/txs/chain/<txid>` cursor.

**Real response** (one element; witness hex truncated, vouts trimmed to 1 of 74):

```json
[
  {
    "txid": "b3c258c26839c7f3dd5f50c672ba6720e1739a360c96e6e97f58651dcbf4496c",
    "version": 2,
    "locktime": 0,
    "size": 18466,
    "vsize": 18466,
    "weight": 18466,
    "fee": 21163,
    "status": {
      "confirmed": true,
      "block_height": 135668,
      "block_hash": "5177f5565d7efb9f0dcfa3ef7511e084a0b7883e42bd4c397dc0e04c3c7e9a57",
      "block_time": 1781900813
    },
    "vin": [
      {
        "is_coinbase": false,
        "txid": "e671df12098e9e6d21dd069bde0cce611ea7de4972e27c297557b7d0bcf290ef",
        "vout": 0,
        "sequence": 4294967293,
        "scriptsig": "",
        "scriptsig_asm": "",
        "witness": ["b4af7987743e3cc8929fce52796696de…"],
        "prevout": {
          "scriptpubkey_address": "btx1zthpclzp3vtxp6ets2cprk6xtteups54pxvr8p334pj56j9p0442quvxfsa",
          "scriptpubkey_type": "witness_v2_p2mr",
          "value": 2000000000
        }
      }
    ],
    "vout": [
      {
        "scriptpubkey": "52202992c00de0c9c70690a47d1121fa2df3b42550639ce836f057bde7e45ce23f3e",
        "scriptpubkey_asm": "2 2992c00de0c9c70690a47d1121fa2df3b42550639ce836f057bde7e45ce23f3e",
        "scriptpubkey_type": "witness_v2_p2mr",
        "scriptpubkey_address": "btx1z9xfvqr0qe8rsdy9y05gjr73d7w6z25rrnn5rduzhhhn7gh8z8ulqv4hprg",
        "value": 40327478
      }
    ],
    "amount_transacted_sat": 3755912342,
    "tx_kind": "regular",
    "has_shielded": false,
    "fee_is_shielded_estimate": false,
    "inferred_fee_kind": "",
    "inferred_fee_sat": null,
    "shielded_txes_in_block": 0,
    "shielded_value_balance": null
  }
]
```

**Required fields (per tx element):**

| Field | JSON path | Type | Why the wallet needs it |
|---|---|---|---|
| Txid | `[].txid` | string (64-hex) | Cursor seed, dedupe key, carrier identity, deep-link. |
| Confirmed flag | `[].status.confirmed` | bool | Mempool-vs-confirmed gate everywhere. |
| Block time | `[].status.block_time` | int (unix s) | History sort desc + "when" label. |
| Block height | `[].status.block_height` | int | Cache + display. |
| Inputs | `[].vin` | array (absent ⇒ treated as `[]`) | Direction + fee. |
| Coinbase flag | `[].vin[].is_coinbase` **or** `[].vin[].coinbase` | bool / presence | Either signals coinbase. |
| Prevout | `[].vin[].prevout` | object **or null** | Null/absent is first-class: sets `directionKnown:false`. |
| Prevout address | `[].vin[].prevout.scriptpubkey_address` | string | `== mine` ⇒ our input (drives sent/received/self). |
| Prevout value | `[].vin[].prevout.value` | int/all-digit str | `inKnown` sum → `fee = inKnown − outTotal`. |
| Outputs | `[].vout` | array (absent ⇒ `[]`) | Amounts + recipient. |
| Output address | `[].vout[].scriptpubkey_address` | string | `== mine` ⇒ change; else recipient. |
| Output value | `[].vout[].value` | int/all-digit str | `outTotal`, `outToMe`, `outToOthers`. |
| **Output script hex** | `[].vout[].scriptpubkey` | string (raw hex) | **CARRIER-BURN-CRITICAL (money-path):** the wallet decodes this on every `/txs` element to find OP_RETURN/BZA1 artifact **carriers** (`scanArtifacts`→`artifactInTx`/`carrierOutput`). A held carrier's mint/transfer tx appears in this address history, so its script hex must be present **here** — the `/tx` coverage fetch only backfills txids *absent* from the history page and never re-pulls one already in it. Omit it and the carrier looks like a plain coin, gets selected as an ordinary input, and **burns the NFT** — silently, with no error. Also drives the history-row artifact tag (`slimTx`→`artifactTxTag`) and the Artifacts tab. |

**Adapter must tolerate:** a `prevout` that is `null` or omitted (the classifier softens direction rather than erroring). Standard Esplora also puts `scriptpubkey`/`scriptpubkey_asm` inside `prevout`; minebtx omits them and the wallet does not need them there **(note: this is the `prevout` script only — the top-level `vout[].scriptpubkey` above IS required, see the carrier-burn row).** The extra top-level keys (`amount_transacted_sat`, `tx_kind`, `has_shielded`, `fee_is_shielded_estimate`, `inferred_fee_kind`, `inferred_fee_sat`, `shielded_txes_in_block`, `shielded_value_balance`) are consumed by btxscan's explorer render (see §3.6) and are safe additive fields for the wallet.

---

### 2.3 `GET /address/<a>/txs/chain/<txid>` — older history page (cursor)

**Purpose:** The only pagination mechanism for history beyond page 1. `<txid>` is the **last `txid` of the previous page**; the response is the next-older page of the same element shape as §2.2. There is **no offset/limit** — the txid is the cursor.

**Response:** identical element shape to `/address/<a>/txs`. Elements flow into the same classifier; the pager itself only reads the **last element's `txid`** to form the next cursor.

**Required fields:** same table as §2.2 — **including the carrier-burn-critical `vout[].scriptpubkey` hex** (older pages are fed into the identical carrier scan, so a carrier surfaced only on page 2+ must still carry its script hex or it burns). Additionally the endpoint MUST **page strictly older than the cursor txid**.

**Adapter must honor:**
- A **bare `/txs/chain`** with no cursor need not exist — the wallet always seeds the first cursor from `/txs`, precisely because the incumbent's bare `/txs/chain` 404s.
- An empty array ⇒ "reached end" (not an error).
- A mid-walk **404 is caught and treated as "stop, keep what we have"** — do not fail the whole history on an exhausted cursor.
- The walk is capped at 500 txs and stops if a page's last txid equals the prior cursor (loop guard). btxscan uses the same cursor for bounded 25-tx serverless walks and rejects an empty address before it can form `/address//txs` (return the same shape either way).

---

### 2.4 `GET /address/<a>/utxo` — spendable coin set — **MONEY-PATH-CRITICAL**

**Purpose:** The exact set of spendable coins fed into the transaction builder. Each element's `value` becomes a prevout amount; only confirmed coins are spendable.

**Real response** (one element):

```json
[
  {
    "status": { "block_height": 135668, "confirmed": true },
    "txid": "b3c258c26839c7f3dd5f50c672ba6720e1739a360c96e6e97f58651dcbf4496c",
    "value": 71419653,
    "vout": 24
  }
]
```

**Required fields (per UTXO):**

| Field | JSON path | Type | Why the wallet needs it |
|---|---|---|---|
| Txid | `[].txid` | string | Outpoint id; carrier / spent-outpoint match on `txid:vout`. |
| Vout | `[].vout` | int | Outpoint index. |
| Value | `[].value` | int/all-digit str | Money-path: coin amount fed to the builder. Malformed ⇒ throws before build. |
| Confirmed | `[].status.confirmed` | bool | **Only confirmed UTXOs are spendable.** |

On unconfirmed entries, `status` carries `confirmed:false` and no height, per Esplora convention. The wallet supplies its own derived `scriptPubKey` (single-address wallet), so the explorer's script for the UTXO is not required. btxscan reads the whole `status` object, so include `block_height`/`block_time` where confirmed.

---

### 2.5 `GET /tx/<txid>` — single transaction — **MONEY-PATH-CRITICAL (carrier value)**

**Purpose:** Full transaction detail. The wallet re-reads the full tx (the slim history cache drops scripts) to (a) confirm carrier coverage and (b) read the **carrier's exact on-chain value** and full output **script hex** for an artifact transfer. A carrier-value mismatch aborts the transfer build.

**Response:** a **single object** (not an array) with the **same shape as a `/address/<a>/txs` element** (§2.2). Tail confirmed `…"vsize":18466,"weight":18466}`.

**Required fields (wallet):**

| Field | JSON path | Type | Why the wallet needs it |
|---|---|---|---|
| Txid | `txid` | string | Coverage accepted only if `txid` present. |
| Outputs | `vout` | array | Carrier read + artifact scan. |
| Output script hex | `vout[].scriptpubkey` **or** `vout[].scriptPubKey` | string (full script hex) | Decodes the BZA1 / OP_RETURN artifact payload — **the sole decode gate**. |
| Carrier value | `vout[<carrier idx>].value` | int/all-digit str | Money-critical: carrier's exact value passed to the recipient output untouched. |

btxscan renders the **full** `Tx` and additionally reads `version`, `locktime`, `size`, `vsize`, `weight`, `has_shielded`, `status.block_hash`, `status.block_time`, the full `vin[]` (`is_coinbase, txid, vout, sequence, scriptsig, scriptsig_asm?, witness[], prevout?`), full `vout[]` (`scriptpubkey, scriptpubkey_asm?, scriptpubkey_type, scriptpubkey_address, value`), and the detail extras `tx_kind, fee, amount_transacted_sat, inferred_fee_sat, inferred_fee_kind, fee_is_shielded_estimate, shielded_value_balance, shielded_txes_in_block`.

**Note — `GET /tx/<txid>/hex` is NOT required and is NOT served by the incumbent** (it 404s on minebtx). The wallet works without it. An adapter MAY add it, but MUST NOT depend on the wallet calling it.

---

### 2.6 `GET /mempool` — fee presets (read-only, non-money-path)

**Purpose:** Drives dynamic fee presets (Normal ≈ 3 blocks, Priority ≈ 1 block). Fully degradable: any failure or an empty histogram falls back to static presets and the Send screen never breaks. On BTX this route legitimately reports an empty mempool (see §4.3).

**Real response** (mempool empty at capture — the normal BTX steady state):

```json
{ "count": 0, "vsize": 0, "total_fee": 0, "fee_histogram": [] }
```

**Required fields:**

| Field | JSON path | Type | Why the wallet needs it |
|---|---|---|---|
| Fee histogram | `fee_histogram` | array of `[feerate, vsize]` pairs (`pair[0]`=feerate, `pair[1]`=vsize) | Walked high→low to pick tier rates. Empty ⇒ flat quiet rates. Order not trusted (re-sorted desc). |
| Backlog vsize | `vsize` | int | `max(vsize, Σ histogram vsize)` decides congested vs clear. |

btxscan also reads `count` and `total_fee`; emit the full `{count, vsize, total_fee, fee_histogram}` object. The wallet clamps derived rates to `[1,50]` sat/vB and, on an empty/≤1-block backlog, uses flat 2/4 sat/vB.

---

### 2.7 `POST /tx` — broadcast — **MONEY-PATH-CRITICAL (funds move here)**

**Purpose:** Broadcast a signed PQ transaction. This is where funds actually move, with deterministic-rebroadcast success semantics.

**Request:**
- Method `POST`, path `/tx`, `Content-Type: text/plain`.
- **Body = raw transaction hex** (the PQ-signed tx). The incumbent also accepts `{"hex":"..."}`; the wallet sends **raw hex**, so raw hex MUST be accepted. An adapter SHOULD accept both.
- Bodies are **multi-MB**: PQ signatures (ML-DSA, ~2420 B each) make transactions large. The endpoint MUST accept large bodies without a small request-size cap (see §4.4). The wallet's own body gate requires length ≥ 100 chars, even length, all hex — so a 64-hex seed can never be smuggled in as a "transaction."

**Success:** **HTTP 200** with a body that is the **bare 64-hex txid** (lowercase-hex-matchable, no `0x`, no JSON envelope). The wallet trims and strips optional surrounding quotes; btxscan's relics worker asserts the body matches `/^[0-9a-f]{64}$/i` and throws `broadcast returned unexpected body` otherwise. Return exactly the txid. (See §4.6 for the exact wire format both clients require.)

**Rejection:** **HTTP 400** with a **human-readable text** reason (in the JSON `{"error":"..."}` envelope on the incumbent — see §4.6 for the format both the wallet regex-classifier and the relics worker match). Deterministic-rebroadcast signatures MUST be surfaced clearly:

| Node condition | Wallet result | Signature to surface |
|---|---|---|
| Already in mempool / `txn-already-known` | **SUCCESS → `pending`** | text contains "already in mempool" / "txn-already-known" |
| Already mined (RPC −27 / "already in block chain" / "outputs already in utxo set") | **SUCCESS → `confirmed`** | RPC code `-27` and/or those phrases |
| Bad tx / policy (min relay fee, `bad-txns`, decode fail, …) | **`failed`** (permanent) | any other reason, at HTTP 400 |

Because a PQ tx is **txid-deterministic** (the signature is in the witness), a lost-response retry re-submits an already-accepted tx. "Already known" and "already in chain" therefore mean *the funds already moved*, not *nothing happened* — the wallet and worker rely on these being distinguishable in the response.

**Observed rejection samples** (invalid payloads, incumbent):

```
body "00"      → 400  {"error":"sendrawtransaction: btxd RPC error (sendrawtransaction, code -22): TX decode failed. Make sure the tx has at least one input."}
empty body     → 400  {"error":"body must be raw tx hex or {\"hex\": \"...\"}"}
```

The rejection text passes the underlying `btxd sendrawtransaction` RPC error through verbatim (including RPC codes). See §4.6 for the exact producer/consumer format the relics worker regex-matches.

---

## 3. btxscan routes (explorer surface beyond the 7)

btxscan (the block explorer + relics indexer) needs these **on top of** the wallet's 7. An adapter that only targets the wallet may omit them, but a full drop-in that also backs btxscan MUST serve them. `[NEW]` = not in the wallet set; `[+fields]` = wallet already hits it, btxscan reads more or enforces a stricter body contract.

---

### 3.1 `GET /blocks/tip/height` — chain tip height — text — **[+fields: strict body]**

**Purpose:** Current chain-tip height. **Body = a bare integer.** btxscan enforces what the wallet does not: `Number(text)` must be an integer ≥ 0 or it raises a 502 — a 200 with a non-numeric body (SPA fallthrough, HTML, CDN interstitial) is rejected. Return a bare integer, at most a trailing newline (callers `.trim()`).

**Real response:** `161391`

---

### 3.2 `GET /blocks/tip/hash` — tip block hash — text — **[NEW]**

**Purpose:** Tip block hash. **Body = bare 64-hex string**, `text/plain`. No further parsing.

**Real response:** `1a908fb96e562315a0146889b30a3f8b0e5ece888e8c3c21362fe08ea899f161`

---

### 3.3 `GET /blocks` — recent block window — JSON `Block[]` — **[NEW]**

**Purpose:** The recent-block window (10 on the incumbent) for the /blocks and /txs explorer pages. Returns an array of `Block` objects (field superset in §3.5 / §3.8). All `Block` fields rendered.

---

### 3.4 `GET /blocks/<startHeight>` — block window from height — JSON `Block[]` — **[NEW]**

**Purpose:** 10 blocks **descending** from `startHeight`. Load-bearing for paging: btxscan reads `batch[batch.length-1].height` and pages down via `height − 1` until it has the requested count or hits genesis. `height` MUST be present and **strictly descending / contiguous** within each batch.

---

### 3.5 `GET /block-height/<n>` — hash at height — text — **[NEW]**

**Purpose:** The canonical block hash at height `n`. **Body = bare 64-hex hash.**

**Real response:** `/block-height/1` → `99911b8fb5433f68bfc5b5e389e87f2d001fb58fef271ef50ce61aca8475ec41`

**Canonicality oracle:** btxscan compares this live hash against its stored `blocks.hash` for each custody-relevant height; a mismatch **excludes** that height's artifact txs from custody. It MUST be cache-stable and reflect the **canonical** chain (reorgs move it). An all-digit `/block/<id>` param is routed through here first to resolve height → hash.

---

### 3.6 `GET /block/<hash>` — single block — JSON `Block` — **[NEW]**

**Purpose:** Full block detail; the indexer persists most fields, so their absence breaks the crawl's schema insert, not just rendering.

**Real response:**

```json
{
  "id": "5177f5565d7efb9f0dcfa3ef7511e084a0b7883e42bd4c397dc0e04c3c7e9a57",
  "height": 135668,
  "version": 536870912,
  "timestamp": 1781900813,
  "tx_count": 13,
  "size": 1235681,
  "weight": 1235681,
  "merkle_root": "bec1feb6d3adc06f7d22cbf73940640c2055820a5a133a0a14b3522ac39758fa",
  "previousblockhash": "c6e6a38b58554b7d9ae4db5ce0114e95e9f00930587f78ff5a9f67d35d046ee5",
  "mediantime": 1781900267,
  "nonce": 0,
  "bits": "1d03a893",
  "difficulty": 0.2733324157605636,
  "chainwork": "000000000000000000000000000000000000000000000000000009ec5a1e89c0",
  "fees_sat": 16972153,
  "subsidy_sat": 2000000000,
  "miner_tag": "byron-pool",
  "matmul_digest": "000000009371d10cff98f381e61e4894dbe79ad03b8cab4279ddfdbe95d5c46b",
  "matmul_seed_a": null,
  "matmul_seed_b": null
}
```

**Required fields:**

| Field | JSON path | Type | Why btxscan needs it |
|---|---|---|---|
| Hash | `id` | string (64-hex) | Persisted PK. |
| Height | `height` | int | Persisted; paging key. |
| Timestamp | `timestamp` | int (unix s) | Persisted; render. |
| Median time | `mediantime` | int | Persisted. |
| Tx count | `tx_count` | int | **Load-bearing:** drives the `s = 25 … tx_count` tx-paging loop (§3.7). |
| Size | `size` | int | Persisted. |
| Weight | `weight` | int (`== size`, §4.2) | Persisted. |
| Subsidy | `subsidy_sat` | int | Persisted (BTX-native). |
| Fees | `fees_sat` | int | Persisted (BTX-native). |
| Miner tag | `miner_tag` | string / null | Persisted (BTX-native). |
| Difficulty | `difficulty` | number | Persisted. |
| Bits | `bits` | **hex string** (e.g. `"1d03a893"`) | Persisted (string, not int). |
| Nonce | `nonce` | int (unused; MatMul PoW) | Persisted. |
| Version | `version` | int | Persisted. |
| Chainwork | `chainwork` | string (hex) | Persisted. |
| Merkle root | `merkle_root` | string | Persisted. |
| Prev hash | `previousblockhash` | string / **null at genesis** | Persisted. |
| MatMul digest | `matmul_digest` | string / null | Persisted (BTX-native PoW). |
| MatMul seeds | `matmul_seed_a`, `matmul_seed_b` | string / null | Declared in type (nullable). |

Standard extras `stale` / `in_best_chain` are **not** served and not required.

---

### 3.7 `GET /block/<hash>/txs/<startIndex>` — block txs page — JSON `Tx[]` — **[NEW]**

**Purpose:** A page of a block's transactions, **25 per page**. The indexer walks all pages: `for (s = 25; s < tx_count; s += 25) GET …/txs/<s>`.

**Mandatory explicit index:** the **`/txs/0`** form MUST be served — a bare `…/txs` falls through to the SPA on the incumbent, so the client always sends the explicit start index. A node that truncates at page 0 silently drops any block with > 25 txs.

**Per-tx fields the indexer reads:** `txid, size, vsize, weight, fee, tx_kind, has_shielded`; `vin[].{is_coinbase, txid, vout}`; `vout[].{value, scriptpubkey_address, scriptpubkey_type, scriptpubkey}`.

| Field | JSON path | Type | Why btxscan needs it |
|---|---|---|---|
| **Output script hex** | `[].vout[].scriptpubkey` | string (raw hex) | **CRITICAL:** the sole BZA1 decode gate, persisted nowhere else and unrecoverable after the crawl. Omit it and artifact indexing silently produces **zero** events. |
| Output value | `[].vout[].value` | int | Amounts. |
| Output address | `[].vout[].scriptpubkey_address` | string | Render / index. |
| Output type | `[].vout[].scriptpubkey_type` | string (`witness_v2_p2mr`) | Render / index. |
| Input coinbase | `[].vin[].is_coinbase` | bool | Coinbase handling. |
| Input outpoint | `[].vin[].txid`, `[].vin[].vout` | string, int | Trace / spends. |
| Tx metrics | `[].size`, `[].vsize`, `[].weight`, `[].fee` | int | Persisted; `vsize==weight==size` (§4.2). |
| Tx kind / shielded | `[].tx_kind`, `[].has_shielded` | string, bool | Persisted. |

---

### 3.8 `GET /tx/<txid>/outspend/<vout>` — outpoint spent status — JSON — **[NEW]**

**Purpose:** Whether a specific output has been spent. **Two hard contract points:**

1. The response is **`{ "spent": boolean }` and ONLY that** — the incumbent never returns a spender txid, and callers read `spent` and nothing else.
2. An **out-of-range `vout` MUST return HTTP 404** (callers catch per-outpoint). Do **not** return `{"spent":false}` for a nonexistent vout.

```json
{ "spent": false }
```

---

### 3.9 `GET /mempool/recent` — recent mempool txs — JSON — **[NEW]**

**Purpose:** Recent mempool entries for the explorer.

**Real response:**

```json
[
  { "txid": "382150ea97d9e6001722aaeb43b0cf8ba34b28f168812c130239b693f5a6a0ff",
    "fee": 84192, "vsize": 42096, "value": null }
]
```

| Field | JSON path | Type | Notes |
|---|---|---|---|
| Txid | `[].txid` | string | — |
| Fee | `[].fee` | int | — |
| Vsize | `[].vsize` | int | `== size` (§4.2). |
| Value | `[].value` | int / **null** | **Nullable** — the incumbent returns `null` here rather than the output-sum canonical Esplora emits. Treat as nullable. |

---

### 3.10 `GET /mempool/txids` — all mempool txids — JSON — **[NEW]**

**Purpose:** Flat array of current mempool txid strings.

```json
["382150ea97d9e6001722aaeb43b0cf8ba34b28f168812c130239b693f5a6a0ff"]
```

---

### 3.11 Shared address routes (btxscan reads more)

- `GET /address/<a>` — §2.1. btxscan requires all four `*_sum` fields for its balance math.
- `GET /address/<a>/txs` — §2.2. btxscan additionally does an **unpaginated full-history pull** (its main coinbase sink takes ~30 s; the client allows 40 s and 0 retries). The endpoint MUST be able to return full history in one response for this caller, in the same element shape.
- `GET /address/<a>/txs/chain/<txid>` — §2.3, same cursor. btxscan uses it for bounded 25-tx serverless walks and rejects an empty address (400) before forming `/address//txs`.
- `GET /address/<a>/utxo` — §2.4. btxscan reads the whole `status` object (not just `confirmed`).

---

## 4. BTX-specific rules an adapter MUST honor

These are the places BTX diverges from vanilla Bitcoin/Esplora. Getting any of them wrong corrupts balances, fees, pagination, or broadcast classification.

### 4.1 Addresses are `btx1z…` (witness v2 P2MR, bech32m, HRP `btx`)

BTX outputs are **post-quantum P2MR** — witness **version 2**, encoded **bech32m**, human-readable prefix **`btx`**. Rendered addresses look like `btx1z7nkymajxh9s089hm8f6ztasptx2nwlmgqqeh9ruxpn6klh3qa55sxvmjs5`. The output type string is **`"witness_v2_p2mr"`** (not `v0_p2wpkh`, `v1_p2tr`, or any standard type). An adapter MUST render and parse these accordingly — a standard bech32 (v0) or bech32m-v1 decoder will reject them. Emit `scriptpubkey_type: "witness_v2_p2mr"` and `btx1z…` addresses in every `scriptpubkey_address` / `prevout.scriptpubkey_address` field.

### 4.2 `WITNESS_SCALE_FACTOR = 1` — weight == size, vsize == weight (NO ÷4 discount)

BTX has **no segwit weight discount**. At every level — tx and block — `weight == vsize == size` (observed: tx `size=vsize=weight=18466`; block `size=weight=1235681`). An adapter MUST **NOT** apply the classic `weight = base*4 + witness` / `vsize = ceil(weight/4)` accounting. Emit `vsize == weight == size` for every transaction, and **fee-rate math MUST use `vsize` under WSF=1** (sat/vB where vsize == raw size). Any adapter that reuses a stock Bitcoin fee/vsize calc will mis-price fees by up to 4×.

### 4.3 Near-empty mempool is normal — report it honestly

BTX runs a **near-empty mempool**. `GET /mempool` legitimately returns `{"count":0,"vsize":0,"total_fee":0,"fee_histogram":[]}`, and the wallet **falls back to static fee presets** when the histogram is empty or the backlog is ≤ 1 block — this is expected, not an error. An adapter MUST NOT synthesize a fake backlog or non-empty histogram to look "busy"; it should report the mempool honestly. An empty mempool is a fully-degradable, non-money condition and the Send screen stays functional regardless.

### 4.4 PQ transactions are large — `POST /tx` must accept multi-MB bodies

Post-quantum signatures are big: **ML-DSA signatures are ~2420 bytes each**, so a signed BTX transaction is far larger than a comparable ECDSA tx and can reach **multiple megabytes**. `POST /tx` MUST accept **multi-MB hex request bodies** — no small request-size limit, no truncation. (For reference, an ordinary 25-input tx above was already ~18 KB on the wire; artifact/sweep txs are much larger.) Any proxy/body-size cap in front of the adapter must be raised accordingly.

### 4.5 `/address/<a>/txs` pagination — txid cursor, no offset/limit

Page 1 of `/address/<a>/txs` returns up to **25 confirmed + up to 50 mempool** transactions, newest first. There is **no `offset`/`limit`**. Older pages are fetched **only** via `/address/<a>/txs/chain/<last_seen_txid>`, where the cursor is the **last `txid` of the previous page** and the response pages strictly older. A **bare `/txs/chain`** (no cursor) need not exist — the wallet always seeds the first cursor from `/txs`. An empty array or a mid-walk 404 means "end / stop, keep what we have," not an error. An adapter MUST implement this **txid-cursor** model exactly; an offset/limit paginator will not be driven correctly.

### 4.6 `POST /tx` success and rejection wire format (exact)

Two independent consumers depend on the exact HTTP status + body:

**Success:**
- **HTTP 200**, body = **bare 64-hex txid**, lowercase-hex-matchable, no `0x`, no JSON envelope.
- The wallet does `.trim().replace(/^"|"$/g,'')` (tolerates surrounding quotes); btxscan's relics worker asserts `/^[0-9a-f]{64}$/i` on the trimmed body and throws `broadcast returned unexpected body` otherwise. **Return the plain txid.**

**Rejection — HTTP 400 + human-readable text.** The relics worker classifies **by HTTP status only**:

```
// producer (relics-worker/esplora.ts):
if (!res.ok) throw new Error(`broadcast rejected (${res.status}): ${text.slice(0,300)}`);
// consumer (relics-worker/worker.ts):
function isDefinitiveRejection(msg) { return /broadcast rejected \(400\)/.test(msg); }
```

- The worker keys entirely off the literal substring **`broadcast rejected (400)`** — i.e. off the HTTP **400**. The body text is not inspected by the worker (it is by the wallet, below).
- Therefore: return **HTTP 400** for **permanent** consensus/policy rejections (bad tx, `min relay fee not met`, `bad-txns`, decode failure) — these are rolled back and, after `MAX_ATTEMPTS`, parked `failed`.
- Return **5xx / timeout / network (never 400)** for **transient** conditions (node busy, unreachable) — the worker keeps the row retrying. **Getting this backwards is dangerous:** a transient error sent as 400 abandons/rebuilds a tx that may still be in flight (double-mint risk); a permanent rejection sent as 5xx wedges the row retrying forever.

**Error envelope:** the incumbent returns **all** error bodies (400 and 404 alike) as **JSON `{"error":"<text>"}`**, not the plain-text canonical Esplora returns. The reason text should pass the underlying `btxd sendrawtransaction` RPC error through verbatim (including RPC codes like `-22`). A drop-in reimplementation SHOULD reproduce this JSON error envelope.

**Deterministic-rebroadcast phrases the wallet regex-matches in the reason text** (these must be present and human-readable so the wallet can upgrade the result from `failed`):

| Result | Phrase(s) the wallet looks for |
|---|---|
| `pending` (treat as sent) | `already in mempool`, `txn-already-known` |
| `confirmed` (already mined) | RPC code `-27`, `already in block chain`, `outputs already in utxo set` |

Because BTX PQ txs are **txid-deterministic** (signature in the witness), a re-broadcast of the same signed tx is safe and idempotent; "already known / already in chain" must **surface clearly** so the wallet reads it as *funds already moved*, not *nothing happened*.

---

## 5. For an adapter author (Byron First Light → this contract)

First Light exposes: an address index (`addr_index`), spends/unspent queries, tx fetch + trace, fee estimation, `sendRawTransaction`, block/mempool queries, and orphan/reorg tracking. That is a complete enough base to satisfy this contract. Mapping hints:

| First Light capability | This contract's route(s) |
|---|---|
| `addr_index` (address funded/spent aggregates) | `GET /address/<a>` → `chain_stats` / `mempool_stats` buckets (§2.1) |
| `unspent` (per-address UTXO set) | `GET /address/<a>/utxo` (§2.4) |
| `addr_index` history + `tx` | `GET /address/<a>/txs` (page 1: 25 conf + 50 mempool) and `/address/<a>/txs/chain/<txid>` cursor (§2.2–2.3, §4.5) |
| `tx` / `trace` (full tx incl. output scripts) | `GET /tx/<txid>` (§2.5) and `GET /block/<hash>/txs/<idx>` (§3.7) — **must include raw `scriptpubkey` hex** |
| `spends` (is-outpoint-spent) | `GET /tx/<txid>/outspend/<vout>` → `{spent}` only, 404 out-of-range (§3.8) |
| `fee_estimate` + mempool state | `GET /mempool` → `{count, vsize, total_fee, fee_histogram}` (§2.6) |
| `sendRawTransaction` | `POST /tx` (raw hex body, 200+txid / 400+reason) (§2.7, §4.6) |
| `blocks` (tip, by-height, by-hash, window) | `GET /blocks/tip/height`, `/blocks/tip/hash`, `/block-height/<n>`, `/block/<hash>`, `/blocks`, `/blocks/<startHeight>` (§3.1–3.6) |
| `mempool` (recent, txids) | `GET /mempool/recent`, `/mempool/txids` (§3.9–3.10) |
| `orphans` / `reorgs` | Keep `/block-height/<n>` reflecting the **canonical** chain — it is btxscan's reorg-detecting canonicality oracle (§3.5) |

**The 2–3 gaps most likely to bite** (verify these before declaring drop-in):

1. **Exact JSON shape / nesting.** First Light's field names and nesting will differ. The adapter must reshape into the precise paths here — especially `chain_stats.*_sum` present-even-when-0, `vin[].prevout` (object or `null`, with `scriptpubkey_address` + `value`), and — **the single highest-risk field** — raw **`vout[].scriptpubkey` hex** on `/address/<a>/txs` (+ its `/txs/chain` cursor), `/tx/<txid>`, and `/block/<hash>/txs/<idx>`. Drop that hex and btxscan's artifact/BZA1 indexing silently produces zero events, **and the wallet's carrier-burn protection fails open — an artifact carrier is spent as ordinary input and the NFT burns** — both with no error.
2. **WSF=1 vsize/weight.** If First Light computes vsize/weight with the standard ÷4 segwit discount, the adapter MUST override to `vsize == weight == size` and expose fee rates as sat/vB under WSF=1 (§4.2). This is a silent mis-pricing, not a crash.
3. **Txid-cursor pagination.** First Light almost certainly paginates by offset/limit or block height; the adapter must translate that into the **`/txs/chain/<last_txid>`** cursor model, with a bare `/txs/chain` allowed to 404 and empty/404 meaning "stop" (§4.5).
4. **Error-body format.** Map First Light's broadcast outcomes onto **HTTP 400 = permanent** vs **5xx/timeout = transient**, with a human-readable reason (JSON `{"error":...}` envelope preferred) carrying the `already in mempool` / `already in block chain` / RPC-`-27` phrases so deterministic rebroadcasts classify as sent/confirmed rather than failed (§4.6). Getting the 400-vs-5xx split wrong risks double-mints or wedged retries.

Also note: First Light does **not** need a `/tx/<txid>/hex` route — the incumbent lacks it and the wallet works without it. Adding it is optional.

---

*Sources: PQ wallet field-consumption audit (`pq-wallet/ui/txview.js`, `ui/main.js`, `src-tauri/src/main.rs`); live shape capture from `https://explorer.minebtx.com/api`; btxscan route audit (`btx-apps/apps/web/src/lib/api/{client,types}.ts`, `relics-worker/{esplora,worker}.ts`, `indexer/{index,pg-index,report-collections}.ts`).*
