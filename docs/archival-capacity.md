# Archival capacity, and what it costs to add some

*Measured on one mainnet node on 2026-09-04. Companion to
[always-on.md](always-on.md), which counted hash publishers; this counts the
machines that can still hand you a block body. The disk section continues
what PR #14 started in [always-on.md](always-on.md) — "roughly 10 GB for a
keeper, more for a full archive" — and as of 2026-09-04 it has a measured number
for "more": 123.8 GiB of block payload, method below.*

---

## The count

`NETWORK` means "I keep the whole chain". `NETWORK_LIMITED` means "I keep the
last few hundred blocks". A node that has fallen behind can only be rescued by a
peer that is **both archival and current**, and that intersection is the whole
story.

From a node at height 210141 with 19 peers connected:

| | |
|---|---:|
| peers connected | 19 |
| advertise `NETWORK` | 12 |
| advertise `NETWORK_LIMITED` | 4 |
| advertise neither | 3 |
| **archival _and_ current** | **1** |

The one was `89.85.40.184:19335`, running `/BTX:0.34.5/`.

The gap between 12 and 1 is the finding. Ten of those twelve archival peers
report **zero headers synced with us**, and they are running `/BTX:0.32.3/`
through `/BTX:0.33.1/` — versions from before the MatMul v4.7 fork. They are
faithfully archiving a chain we no longer share. The eleventh was 665 blocks
behind.

So the network's archival capacity is not merely thin. Most of it is on the
wrong side of a fork, and advertising `NETWORK` the whole time.

This matches what it feels like in practice. This node sat 824 blocks behind for
over an hour with no peer willing to serve a body; a single `addnode` to a known
archival peer fixed it instantly. Since 0.34.6 the engine says so itself, in a
`Convergence note: ... no connected peer is serving this BODY ... NOT an
RC/verify/connect stall or node fault`. Believe it: when a node stops advancing,
the answer is usually a peer, not a restart.

## What being archival costs

**123.8 GiB of block payload, measured 2026-09-04.** Call it 124 GiB, or about
133 GB the way a file manager counts. This is the measurement the earlier draft
of this document said it was missing, and it did not need the spare box and the
several days that a full un-pruned sync was going to cost.

It is also the opposite of the answer this document expected. It expected to
find the install gate too strict and turning capable machines away. The gate was
too loose.

### How it was measured, because this number keeps moving

BTX block sizes are **bimodal**. A block is either about 367 bytes or about
1,049,000 bytes, and there is essentially nothing in between. The large mode is
the MatMul PoW payload rather than transaction traffic — height 120000 is
1,048,948 bytes and carries exactly one transaction. Any method that averages a
coarse grid therefore invents blocks of an intermediate size that does not
exist, which is one way this number has gone wrong before.

So the estimator is not "mean block size". It is "what fraction of each height
band is in the large mode", which is the only quantity that moves the total:

| heights | mode | measured |
|---|---|---|
| 0 to ~47,000 | small | 366-367 B, 72 samples, no exceptions |
| ~47,000 to ~63,000 | mixed | p(large) 0.175, 0.025, 0.375 over three bands, n=40 each |
| ~63,000 to ~184,000 | large | ~1.05 MB, **245 samples, no exceptions** |
| ~184,000 to ~189,000 | mixed | p(large) 0.300, n=40 |
| ~189,000 to tip | small | 374-851 B, 32 samples, no exceptions |

Three independent runs over different grids gave **123.5, 123.8 and 125.2 GiB**.
The large region is at least 98.8 % saturated at 95 % confidence, so the error
budget is roughly +/- 2 GiB and nearly all of it sits in the four transition
bands.

The source was `esplora.btxbyronbay.com`, and it was **validated before it was
trusted** — hash for hash and byte for byte against this node, at every height
this node still holds:

| height | our node | explorer |
|---|---|---|
| 190000 | 300 | 300 |
| 195000 | 390 | 390 |
| 200000 | 367 | 367 |
| 205000 | 395 | 395 |
| 208000 | 383 | 383 |

`scripts/measure-chain-size.py --validate` runs both halves in minutes. Do that
rather than quoting this table in six months.

### What the datadir costs on top of the blocks

Measured on this box:

| | |
|---|---:|
| block payload (the figure above) | 123.8 GiB |
| block index (LevelDB, all 210k headers) | 56 MB |
| undo data, all `rev*.dat` | ~10 MB |
| `chainstate/` | 13 MB |
| `shielded_state/` | 447 MB |
| **datadir total, un-pruned** | **~124.3 GiB** |
| `engines/` (the app's binaries, not chain data) | 960 MB |

Undo data is small because most blocks carry one transaction. `debug.log` is the
component that can surprise you, and `disk.rs` is the code that deals with it.

### The growth rate in the old comments is dead

`setup.rs` justified its headroom with "~1 MB blocks every 90 s grow it ~1 GB a
day". That stopped being true at the fork. Blocks left the large mode around
height 185,000, and the two block stores on this box are a census of either side
of it:

| store | records | payload | mean | large |
|---|---:|---:|---:|---:|
| `blocks/`, 2026-08-10 to 2026-09-04 | 27,013 | 0.21 GiB | 8.4 kB | 0.5 % |
| `blocks.preadopt-*`, 2026-03-19 to 2026-08-29 | 85,297 | 24.96 GiB | 314 kB | 29.3 % |

8.4 kB a block at 90 s is about **8 MB a day, not 1 GB a day** — a factor of
125. Headroom on the install gate is for the chain that already exists, not for
growth. Reproduce with `scripts/blockstore-census.py`, which walks the record
framing rather than trusting `du` on preallocated files.

Note what the second row does **not** say. Those 85,297 records are 25 GiB of
blocks that were downloaded and never connected, and they are still not evidence
of a finished store's size. They are usable here only as a census of block
*sizes* over a known time span, which is a different claim.

### The repository no longer disagrees with itself

| where | said | now |
|---|---|---|
| `commands.rs` preflight comment | "the full ~18 GiB" | cites the constant, states no size of its own |
| `datadir.rs:4` | ~50 GB | ~124 GiB, dated, with a pointer here |
| `setup.rs:20` | ~105 GB, 2026-07-12 | `MEASURED_CHAIN_PAYLOAD_GIB` = 124, 2026-09-04, with the method |
| `setup.rs` `DISK_REQUIRED_FRESH` | 120 GiB | 140 GiB |
| `always-on.md` | 138 GB | the measured figure |
| this document | "roughly 30 GB" | ~124 GiB |
| `CHANGELOG.md` 0.2.1 | ~105 GB | unchanged: it was true on 2026-07-12 and history is not rewritten |

The gate is the part that mattered. `DISK_REQUIRED_FRESH` was 120 GiB against a
124 GiB chain, so **the gate had fallen below the thing it exists to gate**.
That inverts its purpose: instead of refusing an install that could never
finish, it waved through the install that fills the disk halfway. Nothing
detected it, because nothing compared the two numbers. Now `setup.rs` carries
the measurement as a constant and `disk_gate_covers_the_chain` fails the build
if the gate is ever set below it.

## What we have not done

- **Not tested whether the preadopt blocks can seed an archive** instead of
  re-downloading 25 GB. It is the obvious move and it may well work via a
  reindex, but it means handling a live validator's datadir, and this node is
  one of very few current ones on the network. Not worth improvising on.
- **Not switched this box to `prune=0`.** It advertises `NETWORK_LIMITED` today:
  it validates and signs, and does not serve history.
- **Not read `size_on_disk` after a completed un-pruned sync.** The sampled
  measurement above should be checked against one when a spare box exists. It
  would also catch anything the sample cannot see: orphan blocks that a synced
  node keeps and a height-indexed sample never visits. BTX forks hard enough for
  that to be a real term — 638 competing branches were known to this node in a
  day — so treat 124 GiB as a floor for a store that keeps them.
- **Not re-run the census over time.** It is one sample from one node's peer
  set at one moment. BTX peer sets churn; treat the shape as the finding and
  re-measure the number before quoting it.

## Reproducing the census

```bash
btx-cli -datadir=<datadir> getpeerinfo \
  | python3 -c 'import sys,json;ps=json.load(sys.stdin);
n=[p for p in ps if "NETWORK" in p.get("servicesnames",[])];
print(len(ps),"peers,",len(n),"archival")'
```

Compare each peer's `synced_headers` against your own `getblockcount`: a peer
whose headers are far from your tip advertises history it cannot help you with.

## Why this is in the easyNode repository

Because it is a fleet question before it is an ops question. easyNode decides
what a machine advertises, and right now every easyNode installation is
`NETWORK_LIMITED`. If the app offered "keep the whole chain" as a deliberate,
costed choice — about 124 GiB today, shown honestly before it is switched on —
then the scarcest resource on this network is one a home machine can actually
supply. That cost is real, and four times what an earlier draft of this page
guessed, which is exactly why it has to be shown rather than defaulted on.

That is a product decision and a consent decision, not something to default on.
It is written down here so it can be decided rather than rediscovered.
