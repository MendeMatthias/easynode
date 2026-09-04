# Esplora mode: letting the fleet serve wallets and witness forks

*Design and acceptance criteria. Everything measured here was measured on
2026-09-04 from a mainnet node and against the live endpoints. Where something
is not built yet, it says so.*

---

## Why

The BTX PQ wallet reads all chain data over the Esplora REST API. Measured today:

| endpoint | tip | `/blocks` | verdict |
|---|---:|---|---|
| `api.btxscan.io` | 210266 | 200, 10 entries | the only working source |
| `esplora.btxbyronbay.com` | 209778 | **404** | being retired, and frozen |
| `explorer.minebtx.com` | — | — | **503**, "paused while V4 chaos is resolved" |

Byron's tip did not move between two probes minutes apart. It is not lagging, it
is stopped.

The **witness** half matters as much as the data half. A wallet settles a fork by
comparing the block *hash* at a height two sources both hold — a height alone
proves nothing, because on 2026-08-24 two mirrors agreed on 199,296 and both were
wrong. Byron is currently the only independent source publishing hashes.
btxprice publishes a height with no hash; btx-pool publishes a template height
with no hash and tracked an unattested fork during the August stall.

**When Byron goes, nothing in the world can settle a BTX fork for the wallet.**

That is what this mode is for.

## The route contract

Read from the wallet's own egress validator,
`pq-wallet/src-tauri/src/main.rs::validate_esplora_route`. The wallet issues no
other request, and anything else is refused before it leaves the process.

```
GET  /address/<addr>
GET  /address/<addr>/txs
GET  /address/<addr>/utxo
GET  /address/<addr>/txs/chain/<txid>
GET  /tx/<txid>
GET  /mempool
GET  /blocks/tip/height       -> a bare decimal integer, nothing else
POST /tx
```

**Eight routes. Serve them at the root of the origin, with no `/api` prefix.**
Verified against the live reference: `api.btxscan.io/blocks/tip/height` answers
200 and `api.btxscan.io/api/blocks/tip/height` answers 404. The wallet agrees —
`AZURE_ESPLORA` is commented "our own node — Esplora at ROOT, no /api". Only
minebtx ever served under `/api`, which is why the wallet keeps the path in its
UI list and pins only scheme+host in the Rust gate.

### ⚠ The witness route is not in that list, and that is the finding

An earlier version of this document, following the brief it came from, listed
ten routes including `GET /blocks` and `GET /block-height/<height>`, and called
the latter "THE WITNESS ROUTE — do not skip it".

**The wallet does not permit either one.** They are not omitted, they are
actively denied, and there are tests pinning the denial:

```rust
assert!(validate_esplora_route("GET", "/blocks", None).is_err());          // block listing
assert!(validate_esplora_route("GET", "/blocks/tip/hash", None).is_err()); // not needed
```

`/block-height` appears nowhere in the wallet at all — not in the validator, not
in the UI, not in a test.

So the claim that Byron's `/blocks` 404 "silently broke the wallet's divergence
check" cannot be right in that form: the wallet never calls `/blocks`, and its
own gate would refuse the attempt. What the divergence check actually does, in
`ui/main.js::checkTipFreshness`, is fetch `/blocks/tip/height` from every entry
in `OFFICIAL_EXPLORERS` and compare the **heights**.

Which is worse than described, for a different reason. `OFFICIAL_EXPLORERS`
currently has **one entry**. Byron and minebtx were removed from it on
2026-08-19, with a comment saying not to restore either without re-running the
UTXO set comparison. A one-element list compared against itself is not a weak
divergence check, it is an inert one — and no server-side change fixes that.

### What this means for the work

Serving Esplora from the fleet takes **two halves, and neither alone is enough**:

| half | where | what it needs |
|---|---|---|
| data | easyNode | serve the eight routes above; this is the part an operator can do today |
| witness | the wallet | permit `/block-height/<h>`, and carry more than one endpoint |

A height alone proves nothing — on 2026-08-24 two mirrors agreed on 199,296 and
both were wrong — so the witness half has to compare **hashes**, and the wallet
cannot ask for a hash today. Building only the server side yields more copies of
the same data and no fork-settling ability whatsoever.

The good news is that the second half is small: one arm in `validate_esplora_route`,
its test, and more than one entry in the curated list. It is a wallet change, not
an architecture change. But it must be *planned*, because the server work alone
buys none of the capability this was started for.

`scripts/verify-esplora.sh` therefore separates the two sets. Routes the wallet
requires **today** are PASS/FAIL and decide the verdict; the witness routes
(`/blocks`, `/block-height/<h>`) are labelled **FUTURE**, counted separately, and
cannot refuse an endpoint. Getting that wrong withheld wallet-fit endpoints for
failing a capability no wallet can use yet, on a network whose whole problem is
that there is one source left.

Be exact about coverage: the gate probes `/blocks/tip/height`, `/mempool`,
`/address/<a>/utxo`, `/tx/<t>/outspend/<v>` and `POST /tx` out of the eight, plus
CORS, freshness headers and a UTXO-set comparison. The remaining three are
served by the same electrs and stand or fall with it, but they are not
independently proven and this document should not imply they are.

## Hard requirements

**1. `prune=0`, and easyNode refuses otherwise.** electrs builds its index from
btxd's block files on disk. A pruned datadir deletes them as they age, so the
index can never be built and cannot be completed later without a full resync. A
node on `prune=5000` validates, signs and serves a tip — it can never serve
Esplora. `crates/btx-core/src/esplora.rs` is the gate, and it distinguishes three
cases that need three different conversations: a conf that *asks* for pruning
(change it and restart), a datadir that has *already* pruned (only a resync
fixes it), and the keeper profile (pruned on purpose — a choice, not an error).

**2. The reverse proxy is the sole CORS authority.** Strip electrs' own headers
downstream and emit `Access-Control-Allow-Origin: *` and `GET, POST, OPTIONS`
exactly once. Duplicates are rejected by browsers outright and broke the web
wallet once already.

**3. Never fail over to another chain. Declare freshness instead.**

```
X-Btx-Upstream: local
X-Btx-Freshness: fresh | stale | unverified
```

An overstated balance reaching a signing wallet is worse than a stale one, so
always serve your own node and tell the caller how fresh it is. `unverified`
means no witness was reachable and the node declines to claim it is current.
`api.btxscan.io` publishes exactly this today — and it currently reports
`unverified`, which is the header doing its job as Byron dies.

**4. Do not call the node stale for the attestation gap.** BTX has an attested
tip (the signed frontier, where balances are read) and a proof-of-work tip. The
attested tip *may* trail the mined tip by a few blocks, and that is healthy.

⚠ It does not always trail. Measured here: pow tip 210266, attested tip 210266,
`blocks_behind: 0`, quorum true — the gap was zero at that moment. So tolerate a
small gap; do not *assume* one, and do not treat its absence as suspicious.

**5. Opt-in, off by default, honest about cost.** This wants a full archive plus
the electrs index. We deliberately do **not** quote a total, because this project
does not have a trustworthy one: `setup.rs` says the unpruned chain is ~105 GB,
`datadir.rs` says ~50 GB, and neither has been re-measured against a completed
sync. See [archival-capacity.md](archival-capacity.md).

## The acceptance gate

`scripts/verify-esplora.sh` decides whether an endpoint may be advertised. It is
a gate, not a health check — Byron was *up* and answering every route while its
address index did not record spends, reporting 664.40757255 BTX on an address
whose true balance was 157.34199443. That is 507 BTX of phantom unspent outputs
across 116 entries, invisible to a health check and fatal to a signing wallet:
coin selection spends outputs that no longer exist, the build succeeds locally,
and every broadcast fails.

So it compares **sets, not balances**, then independently proves each
claimed-unspent output really is unspent, and checks the witness route against a
reference.

```bash
scripts/verify-esplora.sh https://my-node.example <addr-with-spend-history> ...
```

`REF_API` defaults to `https://api.btxscan.io`. It is deliberately **not**
minebtx: that host answers 503, and a dead reference silently turns every
comparison into a pass.

**Proof the gate works.** Run against Byron Bay it refuses the endpoint, on the
routes the wallet actually calls: no CORS headers, no freshness headers, and a
UTXO set it will not bless without a comparison. Its `/blocks` 404 and its two
diverging heights are reported as FUTURE notes and do not contribute to that
refusal — before the split they did, which is how a wallet-fit endpoint could be
turned away for a capability no wallet can use.

Re-run it rather than quoting a count from here; the numbers moved once the two
sets were separated, and they will move again.

Note that Byron's `/blocks` 404 is reported by the gate as a **future** problem,
not a current one: the wallet cannot call `/blocks` today. It is worth serving
anyway, because the witness half needs it and because an endpoint that already
answers it is one less thing to coordinate when the wallet changes.

The reorg check is worth stating exactly, because the boundary was described
wrongly before. Measured across three sources:

| height | api.btxscan.io | byronbay | our node |
|---:|---|---|---|
| 187660 | `90913421…` | `90913421…` | `90913421…` |
| **187661** | `ad62b638…` | **`2d85ef53…`** | `ad62b638…` |
| **187662** | `5135bb90…` | **`a59a2433…`** | `5135bb90…` |
| 190000 | `f58e1f48…` | `f58e1f48…` | `f58e1f48…` |

The divergence begins **at** 187661, not below it, and it is a bounded window —
Byron's height index never rolled back a short reorg and then rejoins. Checking
only *below* the line would have found nothing.

Note the third column: **our own node agrees with the good source at every
height, including the divergent ones.** That is the evidence that an easyNode can
be a correct witness, rather than the hope that it could.

## The hostname decision — recommendation

The wallet ships a hardcoded curated list, and every origin must appear in both
`OFFICIAL_EXPLORERS` and `PRODUCTION_EXPLORER_ORIGINS` or the Rust egress gate
refuses the request. The two move in lockstep and changing them means an
app-store release. A fleet of easyNodes on changing home IPs cannot be hardcoded.

**Recommendation: (a), with the hostnames under a domain we control.**

The usual objection to (a) is that hardcoding means an app release whenever the
set changes. That objection dissolves if the hardcoded names are *ours*:
hardcode `esplora-1.easybtx.com`, `esplora-2.easybtx.com`, and repoint **DNS** at
a different operator whenever one retires. The wallet's list never changes; the
machine behind it does. That is the flexibility (b) was meant to buy, at none of
its cost.

What it costs: DNS plus a TLS certificate per name, and the operators behind
those names are trusted by every wallet user. So the verification gate must run
**continuously**, not once at onboarding — an endpoint that was correct in
September can develop Byron's exact defect in October, and nothing about the
name would change.

Why not (b): a signed endpoint directory is a change to the wallet's *trust
model*, not just its configuration. It needs a signing key, key distribution,
revocation, and an egress gate that accepts origins it has never seen — which is
precisely the thing the current gate exists to prevent. That is worth building
when the fleet is large enough for a curated list to be the bottleneck. Today the
bottleneck is that there is **one** source, and (a) fixes that this month.

## What is built here, and what is not

**Built and tested in this repository:**

- [`deploy/esplora/`](../deploy/esplora/) — the Caddy front, the electrs unit,
  and the freshness guardian with its timer, ported from the deployment behind
  api.btxscan.io. Its README lists what was deliberately left behind.
- `crates/btx-core/src/esplora.rs` — the precondition gate, with the three-way
  distinction above and 8 tests.
- `scripts/verify-esplora.sh` — the acceptance gate, exercised against both a
  known-bad endpoint and a known-good one.
- This document, including the route contract and the hostname recommendation.

**Not built here, and deliberately not invented:** electrs itself, the BTX
consensus decode crate, the Caddy configuration, the systemd units and the
freshness guardian that writes `/run/btx-{fresh,stale,unverified}`. A working
implementation of all of that already exists in the `btx-esplora` repository —
the thing serving `api.btxscan.io` today — and it is not present on this machine.
Porting it is the next step, and it should be a port rather than a rewrite: the
`Caddyfile` in particular encodes decisions that were paid for with incidents.
