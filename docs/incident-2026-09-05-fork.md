# 2026-09-05: a longer header chain from height 210496 that no peer serves

**Tracking issue: [#35](https://github.com/MendeMatthias/easynode/issues/35)** — status, checklist, and where to help. Sub-issues: #36 fork detector in the app, #37 observer alarm + release-script guard, #38 peering / seed list.

Raw facts first, read-only, from the project's Linux validator on the release
box (`~/.easybtx`, btxd 0.34.6 at `9eb4e005`, hand-launched). Every number here
was read from that node between 18:41Z and 19:05Z on 2026-09-05; nothing on the
node was changed. Interpretation is at the end and is marked as such.

## What the node sees

```
getblockchaininfo   blocks 210865   headers 211167   initialblockdownload false
                    tip time 2026-09-05T16:49:41Z (block 210865, connected here at 18:26:18Z)
getchaintips
  height=211167  branchlen=671  headers-only   da321c55…   newest header time 18:22:07Z
  height=211077  branchlen=581  headers-only   07475dcb…
  height=210865  branchlen=0    active         3e6c8e67…
  (plus single-block valid-forks / headers-only stubs at 210771, 210681, 210566, 210492, 210483)
```

Both long headers-only branches fork from the active chain at **height 210496**
(211167 − 671 = 211077 − 581 = 210496). Block 210496 is timestamped 00:33:15Z.
The first block after it on our branch is timestamped 00:33:33Z; the first block
after it on the 211167 branch is timestamped 00:39:20Z (same version bits
`0x20000000`, same `bits 1e43f96c`). From that point the 211167 branch carries
671 blocks to our 369 over the same wall-clock span, and it was still being
extended at 18:22Z. Our branch has advanced about 10 blocks an hour since
14:35Z, against a chain that was producing ~0.95 a minute before.

## When it became visible here

`~/node-observer.tsv` (2-minute cadence):

```
14:22:45Z  blocks 210816  headers 210816  behind 0
14:31:14Z  blocks 210820  headers 210822  behind 2
14:41:51Z  blocks 210825  headers 211000  behind 175      <- ~500 headers arrived at once
15:xxZ     blocks 210860  headers 211014  behind 154
16:xxZ     blocks 210864  headers 211100  behind 236
17:xxZ     blocks 210864  headers 211143  behind 279
18:40:35Z  blocks 210865  headers 211167  behind 302
```

The 0.6.18 release went live at 16:24Z–16:47Z. The release notes, the pin
comment and the site said this box "has held the tip since 2 September"; by
then it had been ≥150 blocks behind its best header for two hours. That
wording is corrected in the same PR that adds this file.

## The node's mode, checked rather than inferred

The datadir's `btx_rw.conf` still carries the mirror-era lines
(`matmulvalidation=trusted`, one pinned pubkey, `matmultrustedthreshold=1`,
`matmulattestationserve=1`, a local signer key), and the process was launched
without `-matmulvalidation=consensus`. That looks like a trusted mirror. It is
not. `getmatmultrustedstatus`:

```
matmul_validation_mode   consensus
trusted_mirror           false
local_signer             true       chain_oracle  true      serves_attestations  true
single_key_pin           true       trusted_signers 2       threshold 1
heard_attestations       0
```

and `getnetworkinfo.localservicesnames` lists `MATMUL_CONSENSUS`. The engine
degrades the leftover pin to telemetry, which is what `crates/btx-core/src/node.rs`
says it does; the recurring `debug.log` warning

```
16:37:16Z [warning] matmul trusted mirror stall: tip_height=210862 needed_height=210863
          authority_frontier=210862 outstanding_slots=171/1024 rejected_unattestable=1235
```

is that telemetry (the "authority" frontier is this node's own signed
frontier, `getmatmulattestedtip.signed_frontier.height = 210865`), not a gate
on block acceptance. The meaning of `rejected_unattestable` was not
established and is not relied on here.

## Why the longer chain is headers-only

```
18:26:04Z stalled-tower-drive: getdata pass peer=327 tip=210864 best_header=211167
          advertised=210860 best_known=210865 in_flight_global=0 … should_request=1
```

The node wants to request (`should_request=1`) and has nothing in flight: no
connected peer is a source for those blocks. The headers arrived through two
**inbound, block-relay-only** connections, and the engine says what it does
with such peers:

```
17:49:34Z New block-relay-only v2 peer connected: version: 800002, blocks=211143, peer=350
17:49:34Z Ignoring non-authority inbound feefilter (GPU attestors are the only inbound block source, peer=350)
18:24:52Z New block-relay-only v2 peer connected: version: 800002, blocks=211167, peer=355
18:24:52Z initial getheaders (210863) to peer=355 (startheight:211167)
18:24:52Z Saw new header hash=da321c55… height=211167
16:17:38Z getmmattest peer=322 block=07475dcb… height=211077 reason=not_canonical
```

By policy this engine takes blocks from inbound peers only when they are GPU
attestors; these were not, both connections were short-lived, and none of the
20 outbound peers has the branch. So the headers are known and the blocks are
not, and that is a peering fact, not a validation one. At 18:41Z, 20 peers,
none above 210865:

```
hdr=210865 blk=210865 /BTX:0.34.6/ 108.165.189.106   services … MATMUL_TRUSTED_MIRROR, MATMUL_ATTESTATION_ARCHIVE
hdr=210862 blk=210862 /BTX:0.34.5/ 194.93.48.158
hdr=210771 blk=209859 /BTX:0.34.5/ 20.86.181.203:19338
hdr=209476 blk=209476 /BTX:0.34.5/ node.btxchain.org, node.btx.tools, node.btx.dev, 20.86.181.203:19335
hdr=209476 blk=209476 /BTX:0.34.4/ 195.137.245.82
```

`https://api.btxscan.io/blocks/tip/height` → **210865** (the explorer is on our
branch). `esplora.btxbyronbay.com` answers `not found`.

## 19:30Z: the owner settles it

**Owner's statement, 2026-09-05 ~19:30Z: btxscan.io is not on top any more and
is being served the wrong chain.** With that, reading 1 below is the working
truth: the 210865 branch that this node, the other `/BTX:0.34.6/` node, the
three official seeds and btxscan.io follow is a **minority fork from 210496**,
and the live chain is the 211167 branch this side holds only headers for.
Consequences: any 0.6.18 node peered like this one does not follow the live
chain; the site's "follows the live chain" is false until that is fixed;
nothing read from btxscan.io is a source of truth until it is back on the live
chain, and it is never again the only source. The plan is in the evening
handoff (`HANDOFF-easynode-2026-09-05-evening.md`, "First tasks"): find the
live chain's nodes, decide with the owner how this validator gets their blocks
(`addnode`, possibly a restart; the datadir is otherwise untouchable), check
what a fresh install peers with, add a fork detector to the app and an alarm to
the observer, then correct the site.

## Interpretation, as written at 18:55Z before the owner's statement

Two chains extend from 210496. Every peer this node can reach, and the
explorer, follow the shorter one; the longer one (~65 % of blocks since the
fork, still growing) is known here only as headers, because nobody connected
serves its blocks. Two readings fit and only block delivery can separate them:

1. **A partition.** Miners on a segment not connected to ours have been mining
   from 210496 since 00:39Z. When their blocks reach consensus nodes, those
   nodes validate them and, if valid and heavier, reorganise — a ~369-block
   reorg on this side. This node would do the same; it is in consensus mode.
2. **A withheld or invalid branch.** Valid-looking headers whose blocks are
   never served, or are invalid under MatMul consensus when they arrive.

Nothing here says which. It also does not say the 0.34.6 engine is at fault:
the node is not stalled on validation, it has nothing to validate.

## What to do next (owner's call; nothing below was done)

1. Find a node that serves the 211167 branch's blocks (the announcing peers'
   addresses are in `debug.log` around 14:35Z and 18:24Z; `addnode` one of
   them, or ask upstream which nodes carry that chain) and watch
   `getchaintips`: the branch becomes `active` (reading 1) or `invalid`
   (reading 2). **Not on the release box** — its datadir is the project's
   node and the standing rule is not to touch it; any other machine with the
   0.6.18 app and a capable card can answer this.
2. Ask the operator of `108.165.189.106` and upstream what their nodes show.
3. Until 1 is answered, avoid "follows the live chain" in new copy; the site's
   current wording predates this and is left standing, noted here.
4. Re-read `~/node-observer.tsv` before publishing any claim about the tip.

Everything above is reproducible read-only with `btx-cli getchaintips`,
`getpeerinfo`, `getblockheader`, `getmatmultrustedstatus`, and
`tail ~/node-observer.tsv`.
