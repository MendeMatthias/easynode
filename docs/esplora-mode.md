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
GET  /blocks                  -> the recent block listing (chain age)
GET  /block-height/<height>   -> the block hash there, bare 64 hex
POST /tx
```

**Ten routes. Serve them at the root of the origin, with no `/api` prefix.**
Verified against the live reference: `api.btxscan.io/blocks/tip/height` answers
200 and `api.btxscan.io/api/blocks/tip/height` answers 404. The wallet agrees —
`AZURE_ESPLORA` is commented "our own node — Esplora at ROOT, no /api". Only
minebtx ever served under `/api`, which is why the wallet keeps the path in its
UI list and pins only scheme+host in the Rust gate.

### ⚠ The witness half is BUILT, and this section used to say it was not

**Corrected 2026-09-06 against `MendeMatthias/pq-wallet@169b413`.** Every
paragraph that used to stand here was written from the wallet as it was on
2026-09-04 and is now wrong. It said the wallet denied `/blocks` and
`/block-height`, with tests pinning the denial; that the divergence check
compared heights across `OFFICIAL_EXPLORERS`; that a one-element list made it
inert; and that the witness half therefore had to be planned and built. Anyone
following that would plan work that already exists.

What the wallet actually does today, read from its source:

- `validate_esplora_route` **permits** `GET /blocks` and
  `GET /block-height/<h>`. The height gate, `is_height_segment`, is stricter
  than anything this document asked for: ASCII digits only, one to nine of
  them, no leading zero so a padded encoding cannot smuggle extra capacity,
  and exactly three path segments. Its comment sizes the residual at about 30
  bits.
- Divergence does **not** run over `OFFICIAL_EXPLORERS`. There is a separate
  `CHAIN_WITNESSES` list, and the separation is the design: a witness answers
  one question, whether an independent source holds the same block hash at a
  height both hold, and is never asked about balances, UTXOs, fees or history.
  That is why Byron Bay is still useful there while it is banned from the money
  path — wrong about money, sound about blocks.
- The comparison is two requests, `/blocks/tip/height` then
  `/block-height/<h>`, at the **highest shared height**. That is a correction
  to this document's own instinct: comparing low returns "agree" on a genuinely
  forked chain, because every block below a fork point is byte-identical on
  both sides.
- `judgeDivergence` returns `agree | differ | unknown`, and its comment says
  what matters most: "a check that did not run is not a check that passed".

So the sentence at the top of this document, that when Byron goes nothing can
settle a fork for the wallet, is right about the **network** and wrong about
the **code**. The machinery exists and has exactly one witness, which is
retiring.

### What this means for the work

The two halves are still two halves, but only one of them is unbuilt:

| half | where | state |
|---|---|---|
| data | easyNode | built here: `deploy/esplora/`, the app switch, the gate |
| witness machinery | the wallet | **already built** (`ui/tipwatch.js`, the three block routes) |
| witnesses to point it at | the wallet's lists | **the actual gap**: `CHAIN_WITNESSES` has one entry, Byron Bay, which is retiring |

The remaining wallet change is therefore not an arm in a validator and not an
architecture change. It is one or two entries in `CHAIN_WITNESSES` (and, for a
node fit to serve money routes too, in `OFFICIAL_EXPLORERS` plus
`PRODUCTION_EXPLORER_ORIGINS`, which the endpoints test keeps in lockstep). It
is blocked on the same two things as everything else here: a DNS name, and an
acceptance PASS behind it. Adding a name that does not resolve buys nothing and
risks a user selecting a dead endpoint, so it waits.

`scripts/verify-esplora.sh` treats all three block routes as **required**, and
did not before. It labelled them FUTURE, on the finding above, and counted them
separately so they could not refuse an endpoint. That was right while the
wallet denied them and is wrong now: an endpoint that 404s `/blocks` breaks the
chain-age reading, and one that cannot answer `/block-height/<h>` cannot
witness at all, which is the single capability this whole mode exists to
restore. Byron Bay would now fail the gate on that route, which is correct: it
is a witness the wallet already cannot rely on.

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
the electrs index. The archive half is now measured: **123.8 GiB of blocks on
2026-09-04**, method in [archival-capacity.md](archival-capacity.md). The electrs
index on top of it is **not** measured, so we still decline to quote a total —
but the part we do know, we now say. Treat it as a server-class commitment.

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
UTXO set it will not bless without a comparison. Since 2026-09-06 its `/blocks`
404 and its diverging heights count too, because the wallet now calls those
routes. That is the second time this line has moved, in both directions, and
the rule behind it is the stable part: the gate requires exactly what the
wallet actually calls, and reads the wallet's own validator to find out.

Re-run it rather than quoting a count from here; the numbers moved once the two
sets were separated, and they will move again.

Byron's `/blocks` 404 is reported by the gate as a **current** failure since
2026-09-06, and used to be reported as a future one. The wallet calls that
route today (the correction above), so an endpoint that does not answer it
cannot serve this wallet, and saying otherwise would hand a wallet an endpoint
that half-works.

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

*Updated 2026-09-06. The earlier version of this section said the electrs
fork, the decode crate, the Caddy configuration, the units and the guardian
were "not present on this machine" and "not built here". They are now.*

**Built and tested in this repository:**

- [`deploy/esplora/`](../deploy/esplora/): the whole serving stack, ported from
  the deployment behind api.btxscan.io at commit `c77fa40` of the private
  `btx-esplora` repository. electrs and rust-btx are vendored verbatim
  (`PROVENANCE.md` there records the commit, the exclusions and every
  hand-made difference), the Caddy front was verified against the original by
  a comment-stripped diff, and the build scripts, units, guardian, health
  check and decoder scan were ported by hand. rust-btx's suite passes in its
  new home; `.github/workflows/esplora.yml` runs it and checks electrs
  whenever the trees change.
- `crates/btx-core/src/esplora.rs`: the precondition gate, with the three-way
  distinction above and its tests, plus the two measurements it needs
  (`conf_prune`, `node_prune_posture`).
- `crates/btx-core/src/esplora_freshness.rs`: the freshness guardian, judged
  against the chain census rather than any explorer (next section), pure and
  tested against the feed as read on 2026-09-05.
- `crates/btx-core/src/esplora_sidecar.rs`: electrs and the Caddy front as
  children of the app, beside btxd.
- The app: a "Serve wallets (Esplora API)" switch in Settings that runs the
  gate and shows its sentence, refuses on a pruned datadir or a missing
  binary rather than staying on and inert, starts the two sidecars with the
  node, runs the guardian every 30 s, and shows the verdict beside the switch.
- `scripts/verify-esplora.sh`: the acceptance gate, now checking the reference
  and the candidate against the census's heaviest chain before it compares
  anything.
- [`esplora-api-contract.md`](esplora-api-contract.md): the API the electrs
  fork serves, verbatim from the source repository.

**Not built, and said plainly:**

- An acceptance run against a real endpoint (below).
- Bundling electrs and Caddy in the AppImage. Both are built, not downloaded;
  `deploy/esplora/build-electrs.sh` and `build-caddy.sh` are the route, and the
  app names the missing one. Shipping them in the release is a packaging
  change for another day, and it should wait for the acceptance run.
- `recentHashes` in the public census feed, which would remove the one-block
  orphan caveat in the guardian.

## Freshness comes from the census, not from an explorer

The guardian ported from api.btxscan.io compared the local tip height with one
explorer's. That failed three separate ways: explorer.minebtx.com died and its
absence was read as health (2026-08-13); esplora.btxbyronbay.com followed the
unattested branch, so its height meant nothing (2026-08-14); and on 2026-09-05
api.btxscan.io itself sat on a minority branch for a day, which as a witness
would have called a live-chain node "stale" and a dead-branch node "fresh".

Since 2026-09-05 the site's `/api/nodes` publishes `chains`: every chain any
reachable node follows, measured from the nodes' own headers, with the one
carrying the most work marked `heaviest`, its tip height, a 16-character prefix
of its tip hash, and the height at which it left the others. That is a
measurement of the network, and it is the witness now.

### What the census can and cannot witness, measured

The first version of these rules compared the served block at the census's
heaviest tip and called a mismatch "another chain". Running it found that
wrong, twice in half an hour:

| read | census says heaviest | this box's validator, same minute |
|---|---|---|
| 2026-09-05 23:27Z | chain B, tip 211381 (`2218b55a…`) | `2218b55a…` is a one-block `valid-headers` side tip; active chain at 211391 |
| 2026-09-06 00:00Z | chain A, tip 211404 (`d5cdc194…`) | `d5cdc194…` is a one-block `valid-headers` side tip; active chain at 211416, and its block at 211404 is `a433ed21…` |

At 00:00Z `api.btxscan.io` served `a433ed21…` at 211404 too, agreeing with this
node at every settled height. The first rules would have called it "on another
chain", and the acceptance gate did exactly that: it aborted.

So the census is a **strong** witness for "this endpoint is on a deep minority
branch", which is the 2026-09-05 shape (chain C, forked 389 blocks down, and
still there a day later). It is a **weak** witness for "this endpoint holds the
exact best block", because BTX mines races and a race flips both the heaviest
flag and the published tip hash within a block or two. The rules follow that
distinction now, and the threshold between the two is `RACE_DEPTH`, six blocks.

### Settled blocks made this provable

The paragraph that used to end this section asked for exactly one thing: hashes
below the racing window, so an endpoint could be placed on a chain positively
rather than by elimination. [EasyBTX#468](https://github.com/MendeMatthias/EasyBTX/pull/468)
publishes them. Each chain in the feed now carries up to ten
`(height, hash-prefix)` pairs, every one at least six blocks below that chain's
tip and above its fork, drawn from the chain's own witness headers.

Six is `SPLIT_MIN_LEAD` on the site, the number of blocks it already required
before calling a branch a chain, and `RACE_DEPTH` here. The two were chosen
independently and agree, which is the right answer for the same reason in both
places: it is btxd's own emergency park depth.

The **newest askable pair decides**. Below a fork every chain is byte-identical,
so a match deep down proves nothing about which side an endpoint is on; the
deepest useful question is the shallowest settled one.

### The rules

Implemented in `esplora_freshness.rs` and identically in
`deploy/esplora/btx-staleness-check.sh`; the acceptance gate applies the same
deep-branch test. Both are executed by tests —
`deploy/esplora/test-guardian.sh` runs the shell against stubs and pins that
the two agree.

| in this order | evidence | verdict |
|---|---|---|
| 1 | the served tip is unknown | `unverified` |
| 2 | no census, older than 30 min, or no heaviest chain with a usable tip | `unverified` |
| 3 | a settled block of the heaviest chain **matches** | `fresh`, or `stale` when more than 3 below its tip |
| 4 | a settled block of the heaviest chain **differs** | `unverified`, a real divergence; the chain it is on is named when the feed allows |
| 5 | the endpoint holds the tip of a competing chain that forked more than 6 blocks below the heaviest tip | `unverified`, and the chain and its fork height are named |
| 6 | at or past the census tip, and the block served there IS the census tip | `fresh` |
| 7 | at or past the census tip, and it is not | `unverified`, said as a probable race, not an accusation |
| 8 | more than 3 blocks below the census tip | `stale` |
| 9 | within 3 blocks, not comparable | `unverified` |

Rules 3 and 4 are the only ones that prove anything; everything below them is
inference from heights and from a tip hash that is regularly an orphan. They
run first for that reason. Rules 5 to 9 remain because a feed published before
#468 carries no settled pairs, and a guardian that broke on an older feed would
be worse than one that is merely less certain.

Rule 5 still runs before any height comparison: an endpoint on a minority
branch is `unverified` however current it looks, because an overstated balance
from the wrong chain reaching a signing wallet is worse than a stale one. The
Caddy front answers `unverified` when no marker exists at all
(`PROVENANCE.md` says why that differs from the original).

The measured case, end to end: an endpoint at 211416 that does **not** hold the
census's heaviest tip 211404, because that tip was a one-block orphan. Before
settled pairs it read `unverified`. It now reads `fresh`, proven at the settled
block 211398, and both the Rust and the shell say so with the height that
proved it.

## Acceptance status, 2026-09-06

`scripts/verify-esplora.sh` was **not** run against an easyNode endpoint,
because none exists yet that can pass its precondition:

- The release box's validator is the project's signer and its datadir is
  pruned (`getblockchaininfo` read at 23:25:42Z on 2026-09-05: `pruned: true`,
  `pruneheight: 184942`). It cannot serve Esplora without a resync and it will
  not be resynced.
- The box has 54 GB free against a 124 GiB chain plus an unmeasured index, so
  a second unpruned datadir cannot live there either.

What was run instead, all on 2026-09-05/06 and all in the pull request:

- rust-btx's suite in its vendored location: 25 tests, pass.
- `build-electrs.sh` on this box with no root, which is how it found that the
  pip `libclang` ships no builtin headers and needs gcc's include directory;
  the script now handles that. The binary runs.
- That binary against this box's validator: it read the cookie, connected, and
  refused with `pruned node is not supported (use '-prune=0' bitcoind flag)`,
  which is the precondition `crates/btx-core/src/esplora.rs` exists to catch
  before an operator spends hours on it.
- btx-core (339 tests) and the app (40) with clippy's correctness and
  suspicious lints; the web layer's typecheck, 55 tests and bundle.
- The gate against `api.btxscan.io`, twice: once to find the false abort
  described above, and once after the fix, where it passes 19 checks including
  a UTXO set comparison and a `POST /tx` round-trip. Note what that run is and
  is not: with no second endpoint to compare against, candidate and reference
  were the same host, so the set comparison compared it with itself. It
  exercises the gate; it accepts nothing.

The acceptance run needs a machine with the whole chain: sync one with
`prune=0` (140 GiB free, a qualified GPU for blocks past 185,000), build
electrs and Caddy with the two scripts, switch the setting on, let electrs
index, then run the gate with two or three addresses that have spend history.
Only a PASS is a reason to give the endpoint a name.

## Hostnames and DNS: the runbook

Option (a) from the decision above. Nothing here has been done yet; it is the
owner's to do, in this order, after an acceptance PASS:

1. Pick the operator behind `esplora-1.easybtx.com`. Their app's "Esplora
   address" setting becomes that hostname; Caddy obtains the certificate, so
   ports 80 and 443 must reach the machine.
2. Create the DNS `A`/`AAAA` record for `esplora-1.easybtx.com` at the
   provider that serves `easybtx.com`.
3. Re-run the gate against `https://esplora-1.easybtx.com`. PASS, or stop.
4. Repeat for `esplora-2` with a different operator, so the wallet has two
   sources to compare hashes between.
5. Only then: the wallet change, a wallet release, and the link from the site.
   The change is two list entries, not machinery — the machinery is already
   there (the correction above). A node that has passed the gate goes in
   `CHAIN_WITNESSES`; one that should also serve balances goes in
   `OFFICIAL_EXPLORERS` **and** `PRODUCTION_EXPLORER_ORIGINS`, which
   `ui/endpoints.test.js` keeps in lockstep. Witnessing is the scarcer job:
   `CHAIN_WITNESSES` has one entry today and it is Byron Bay, which is
   retiring.

Repointing a name at another operator later is a DNS change, not an app
release. That is the whole reason for (a).
