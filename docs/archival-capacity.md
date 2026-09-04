# Archival capacity, and what it costs to add some

*Measured on one mainnet node on 2026-09-04. Companion to
[always-on.md](always-on.md), which counted hash publishers; this counts the
machines that can still hand you a block body. The disk section continues
what PR #14 started in [always-on.md](always-on.md) — "roughly 10 GB for a
keeper, more for a full archive" — by trying to put a number on "more".*

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

Less than we thought, and the number that has been quoted was measuring the
wrong directory.

The datadir on this box is 27 GB, and that figure has been repeated as "the
pruned chain". It is not. Measured:

| | |
|---|---:|
| `blocks/` (the live pruned chain, 25k blocks retained) | 284 MB |
| `chainstate/` | 13 MB |
| `shielded_state/` | 447 MB |
| `engines/` | 960 MB |
| `blocks.preadopt-1788268845/` | **26 GB** |

The 26 GB is a backup the node kept when it adopted a faststart snapshot: 202
block files from an earlier full download, with the undo data essentially empty
(202 `rev*.dat` totalling 9.2 MB, so those blocks were fetched but never
connected). It is not the chain the node is running on, and nothing reads it.

It is tempting to read that 25 GB as the price of a full history, and this
document said so in an earlier draft. It does not support that claim, for a
reason visible in the same measurement: those blocks were downloaded and never
connected, and the sync was abandoned in favour of a snapshot. A store that was
never finished is not evidence of a finished store's size. BTX also keeps
orphaned blocks, and it forks hard enough for that to matter — 638 competing
branches were known to this node in a day.

**The repository does not agree with itself here either**, which is worth fixing
before anybody quotes any of it:

| where | what it says |
|---|---|
| `crates/btx-core/src/setup.rs:20` | chain measured **~105 GB** on 2026-07-12, from `size_on_disk` after a complete sync |
| `crates/btx-core/src/setup.rs:25` | `DISK_REQUIRED_FRESH` = **120 GB**, the fresh-install gate |
| `crates/btx-core/src/datadir.rs:4` | the chain is **~50 GB** un-pruned |
| `apps/node/CHANGELOG.md:881` | about **105 GB** today, growing |

105 and 50 cannot both be right, and the 105 GB one is the only one with a date
and a method attached, so it is the one to trust until somebody re-measures.

That gate is the part that matters. `DISK_REQUIRED_FRESH` is 120 GB, so a fresh
install is refused on any machine with less free space than that. If the real
figure is nearer 50 GB — or nearer what this box suggests — then the app is
turning away home machines that could comfortably run a node, which is the exact
opposite of what this project is for. Nobody should adjust that constant from an
inference. Somebody should run one unpruned sync to completion and read
`size_on_disk`, and that is the measurement this document is missing.

## What we have not done

- **Not tested whether the preadopt blocks can seed an archive** instead of
  re-downloading 25 GB. It is the obvious move and it may well work via a
  reindex, but it means handling a live validator's datadir, and this node is
  one of very few current ones on the network. Not worth improvising on.
- **Not switched this box to `prune=0`.** It advertises `NETWORK_LIMITED` today:
  it validates and signs, and does not serve history.
- **Not measured a completed unpruned sync.** That is the one number that
  would settle the table above and tell us whether the 120 GB install gate is
  keeping capable machines out. It needs a spare box and a few days, not a
  clever argument.
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
costed choice — roughly 30 GB, shown honestly before it is switched on — the
scarcest resource on this network is one a home machine can actually supply.

That is a product decision and a consent decision, not something to default on.
It is written down here so it can be decided rather than rediscovered.
