# 2026-09-06: a header tower nobody serves bodies for, and the 0.6.20 hold

0.6.20 was built and gated, then held unsigned for 47 minutes while this
played out. It cleared on its own and the release went ahead; see the
Resolution at the end. This records the shape, because every number a release
script reads said "chain split" and it was not one.

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

Then the gate cleared on its own and the rest ran: signed at 15:12:54Z,
verified against the app key two independent ways, a Linux-only feed, and
**draft** `node-v0.6.20` on `MendeMatthias/EasyBTX-releases` with all four
assets re-downloaded from GitHub and byte-compared against the gate run.

Not done, and each an explicit decision: flipping the draft live, the site feed
and pins, and syncing this changelog into the monorepo.

## To resume

1. `scripts/observer-ok.sh` must exit 0 on its own. Do not set
   `OBSERVER_OVERRIDE=1` to get past this; that is an owner decision and it is
   the exact shortcut the 2026-09-05 release took. On this occasion waiting was
   the whole fix: 47 minutes, and nothing touched.
2. Re-read chain truth from the node: `getchaintips` and `getblockchaininfo`
   within ten minutes of signing, plus the census, never an explorer alone and
   never btxscan.
3. Then `tools/release-helpers/sign_feed_0620.sh`, then
   `publish_draft_0620.sh`. The artifacts are already gated and named in
   `~/release-0.6.20`; do not rebuild them. `finish_0620.sh` runs both behind
   the chain gate and stops at the draft.

## Resolution, 15:10:37Z

It cleared on its own, 45 minutes after it started, with nothing done to the
node. The bodies arrived and btxd connected them in one step: **211,960 →
211,990**, and the headers/blocks gap went to 0. `observer-ok.sh` returned `ok`
at 15:10:08Z.

Read at 15:12Z, after the jump:

- `getchaintips` active 211,993. The only branch ahead leads by **1**, an
  ordinary one-block race, far inside the 6 that separates a race from a
  branch.
- The branch this node had been on, `639483535dd86ca1` at 211,960, is now a
  `valid-fork` 33 behind. It was a two-block losing branch, exactly as the
  shape suggested and not what the settled comparison made it look like.
- The census reports **`split: false`**, one chain, 8 nodes, and this node
  matches **all ten** of its settled hashes (211,974–211,983).

So the whole episode was one stall in body delivery, not a chain split, and
the numbers a release script reads said "split" throughout it. The point of
this note stands: `debug.log` and the `getchaintips` status were the only
sources that distinguished them at the time, and they said so from the first
minute.

0.6.20 was signed at 15:12:54Z once the gate cleared on its own, and nothing
was overridden to get there.
