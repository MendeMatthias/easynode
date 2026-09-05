# Efficiency 3.0: nodes that orient themselves, all the time

Written 2026-09-06 after the 210496 split, from the owner's brief: "these nodes
need to be oriented and check things all the time, so they do not make
mistakes and are synced", and "connect it with other pools somehow".

## What the split proved

A node's own view is not enough. On 2026-09-05 every reachable node this app
knew, the official seeds, the explorer and the pool this app seeds all sat on
the minority branch, each one healthy by its own measure. The information that
would have told them otherwise existed the whole time, in three places nobody
was reading on a schedule:

1. **Other nodes' headers.** Two chains have two sets of hashes. The census on
   easybtx.com/nodes now reads every reachable node's headers every 30 minutes
   and says which chain each follows, which chain carries the most work, and
   whether the network is split. (EasyBTX #461, #463.)
2. **The pools.** A pool that is paying out is on the chain its miners mine.
   At 22:40Z on 2026-09-05: btx-pool.com mines chain B (height 210,886,
   blocks paid at 210,880 to 210,885), luckypool.io mines chain A (every one
   of the last twelve blocks at 211,350 carries its tag), Byron Bay is paused.
   So even "what the pools do" is split, and a node needs to see that, not be
   told one answer.
3. **Its own peers.** The engine banned the live chain's peers for asking
   about blocks it did not have (btxchain/btx#142) and refused to fetch from
   peers that had never served it a body (SF-3). A node cannot fix the engine,
   but it can notice that a heavier chain exists and dial a peer that serves
   it, which is exactly what fixed the validator by hand.

## The loop

Every easyNode, every ten minutes, on the existing status refresher:

1. **Read the census**: `GET https://easybtx.com/api/nodes` (same feed the page
   reads; edge-cached, no per-node cost). Take `chains` (each with `tipHeight`,
   `tipHash` prefix, `forkHeight`, work, node count, `heaviest`) and, new,
   `pools` (each with `name`, `height`, `chain`, `state`) and `live_peers`
   (one or two addresses of consenting nodes measured on the heaviest chain
   in that run; today that is the shipped seed 89.85.40.184 and
   13.140.141.180).
2. **Locate itself**: `getblockhash` at the heaviest chain's `forkHeight + 1`
   (or its tip height when there is no fork) and compare with the census's
   hash for that height. Same hash: on the heaviest chain. Different: on
   another chain, and the census says which.
3. **Say it**: the fork detector already renders "a longer chain exists that
   your node cannot obtain"; add the orientation: "The census measures your
   node on chain B; chain A carries the most work and is mined by luckypool.io;
   btx-pool.com mines chain B." No verdict on right or wrong, the facts.
4. **Act, within the engine's rules**: if not on the heaviest chain and the
   census publishes live peers, `addnode <peer> add` for one of them (manual
   peers pass SF-3 and are exempt from the getmmattest ban). With parking off
   since 0.6.19 the engine reorganises by itself once bodies arrive. Log the
   dial, show it in the UI, never restart the node.
5. **Report back, opt-in**: the check-in (`checkin.rs`, unwired since it would
   make "nothing phones home" false) becomes a setting, off by default, that
   posts height, tip hash prefix and chain to `/api/node-checkin` so a node
   behind a router, like the Serbian ones, still counts and still gets seen.

## The census side

- `pools` in the feed: fetch each pool's public stats (btx-pool.com
  `/api/stats` carries `height` and `recent_blocks`; Byron Bay's stats.json is
  the pause page when paused; luckypool.io publishes no API found on
  2026-09-05, so it is recognised by its coinbase tag on the heaviest chain's
  recent blocks, which the checker can read from the block bodies of one
  serving node). Classify a pool by the chain whose recent heights contain its
  paid blocks, or by height when the chains' tips are far apart. State: paying
  (a block in the last 24 h), quiet, paused, unreachable.
- `live_peers` in the feed: addresses of nodes on the heaviest chain that
  answered bodies this run AND are on the consent list (`nodes.json` entries
  with `seed: true`, or the shipped seed list). Never a community node's
  address: the anonymity model stays.
- The feed already carries `chains` and `nodes`; the page gains a "Pools"
  line in the chain box and the app reads the same JSON.

## What it costs

One HTTPS read per node per ten minutes against an edge-cached JSON; one RPC
(`getblockhash`) per read; one `addnode` when out of line, rate-limited to one
per hour. Nothing new is served by the node. The census's pool fetches are two
HTTP calls per 30 minutes.

## What it does not do

It does not decide which chain is right. It shows every node the same picture
the page shows, and lets the engine's own most-work rule act on peers it can
now reach. If the pools and the work disagree, the node says so and stays
where the engine puts it.

## Order

1. Census `pools` and `live_peers` (site, one PR).
2. App: read the feed, compare, render (0.6.20), with the `addnode` step behind
   a setting that defaults ON for consensus nodes and OFF for mirrors.
3. Check-in opt-in (0.6.20 or 0.6.21), with the changelog line that says what
   leaves the machine.
