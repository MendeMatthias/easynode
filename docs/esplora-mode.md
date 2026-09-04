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

Authoritative, from the wallet's own egress validator. The wallet issues no other
request and refuses any endpoint that cannot answer these. Serve them at the
**root of the origin, with no `/api` prefix**.

```
GET  /address/<addr>
GET  /address/<addr>/txs
GET  /address/<addr>/utxo
GET  /address/<addr>/txs/chain/<txid>
GET  /tx/<txid>
GET  /mempool                 -> {count, vsize, total_fee, fee_histogram}
GET  /blocks/tip/height       -> a bare decimal integer, nothing else
GET  /blocks                  -> the 10 most recent blocks
GET  /block-height/<height>   -> a bare 64-hex block hash   <-- THE WITNESS ROUTE
POST /tx                      -> raw tx hex in, txid out
```

`/block-height/<h>` is the route that lets the fleet replace Byron as a fork
witness. Do not skip it and do not let it 404. **Byron answers `/blocks` with
404**, which silently broke the wallet's divergence check for weeks: it looked
like it had run, and it had not.

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

**Proof the gate works.** Run against Byron Bay it fails 9 checks and refuses the
endpoint; run against the reference it passes 16 and fails only the deliberate
refusal to bless an endpoint whose UTXO sets were never compared.

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
