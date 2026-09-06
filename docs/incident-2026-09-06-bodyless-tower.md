# 2026-09-06: a header tower nobody serves bodies for, and the 0.6.20 hold

0.6.20 is built, gated and **not signed**. This records why, so the next person
does not rebuild it or, worse, override the gate to get past a condition they
have not read.

## What happened

The validator advanced 544 blocks through the day and reached **211,960 at
14:25:28Z**. It has not advanced since. `headers` kept climbing: 211,961 at
14:25Z, 211,984 at 15:02Z. `scripts/observer-ok.sh` turned `FORK` at
**14:38:11Z** and refuses to sign, which is what it is for.

## What it was

Not a minority branch. btxd names it itself, once per 30 seconds in
`debug.log`:

```
Convergence note: tip-critical block 16440a395bb2a768…9413 height=211959 has
been requested for 2164s with no delivery -- no connected peer is serving this
BODY. The node is at the served body tip (headers ahead may be bodyless
competing towers), waiting on the network -- NOT an RC/verify/connect stall or
node fault.
```

Corroborating reads at 14:57Z–15:02Z:

- `getchaintips` shows the tower as **`headers-only`**, branchlen 23, at
  211,981. Not `invalid`: we are not rejecting those blocks, we do not have
  them. `last_common` with it is 211,958.
- Three peers on `/BTX:0.34.6/` carry headers to 211,983. None delivered body
  211,959 in 36 minutes of asking.
- Nineteen peers connected throughout. The eight manual `addnode` slots are
  full, and dialling does not help when no peer has the body.

## Why it looks exactly like a fork, and is not

The census at `easybtx.com/api/nodes` builds each chain's `settled` pairs from
**witness headers**. So a chain it calls `heaviest`, with ten nodes on it, can
be a tower whose bodies nobody serves. Compared height by height at 15:00Z, this
node **agreed with chain A at 211,951–211,958 and differed at 211,959 and
211,960** — the two newest settled pairs, and the two the rule says to trust.
That reads precisely like the 2026-09-05 split
(`docs/incident-2026-09-05-fork.md`), and it is not one.

The numbers cannot tell the two apart. `debug.log` and the `getchaintips`
status can. Read those before writing a sentence about which chain anyone is
on.

## What the release did right

The judge shipped in 0.6.20 was asked this question on live data at 15:01Z and
declined to guess:

```
state=unverified why=off-heaviest-at-settled-height at=211960
serves_chain=none local=211960 census=211966
```

It compared at the **newest** askable settled height, found this node's block
is not the heaviest measured chain's, and refused to call it fresh. That is the
right answer even though the cause is the network's rather than this node's: a
witness reports what it can prove. `deploy/esplora/test-witness.sh` passed
21/21 around it, so the routes and the refusals are intact; only the freshness
verdict changed, which is the verdict that is supposed to change.

## State of 0.6.20

Done: merged to `main` (#57), built on this box, and through every gate —
`.btxd-version` v0.34.6, both bundled binaries run under a clean environment,
`ldd` resolves inside the tree, `cuobjdump` lists all pinned architectures, the
packaged app carries the witness routes, `check-engine-tag.sh` and
`check-engine-fleet-ready.sh` OK, and upstream's `verify_release_btxd.py` — the
recipe's ship blocker — PASS for both binaries. The engine in these bundles is
**byte-for-byte identical to the shipped 0.6.19**, all 21 files.

Not done, and not to be done until the gate clears: signing, the feed, the
GitHub release, the site pin.

## To resume

1. `scripts/observer-ok.sh` must exit 0 on its own. Do not set
   `OBSERVER_OVERRIDE=1` to get past this; that is an owner decision and it is
   the exact shortcut the 2026-09-05 release took.
2. Re-read chain truth from the node: `getchaintips` and `getblockchaininfo`
   within ten minutes of signing, plus the census, never an explorer alone and
   never btxscan.
3. Then `tools/release-helpers/sign_feed_0620.sh`, then
   `publish_draft_0620.sh`. The artifacts are already gated and named in
   `~/release-0.6.20`; do not rebuild them.
