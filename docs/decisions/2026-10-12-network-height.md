# One network height for every "blocks behind" reader

| | |
|---|---|
| Status | decided for 0.7.11 under the owner's standing instruction for the overnight run of 2026-10-11 ("design one correct network height and use it in every reader ... write the decision in docs/decisions/") |
| Date | 2026-10-12 (measurements from 2026-10-11, UTC) |
| Supersedes | `getblockchaininfo.headers - blocks` as the gap, in every reader listed below |
| Leaves alone | what the app reports to the website (`checkin.rs` keeps the raw `headers`), the local service report, the header bootstrap (it compares headers with a snapshot anchor below 244,000), the fork check's reading of `getchaintips` |
| Code | new: `crates/btx-core/src/network_height.rs`. Changed: `node_api.rs` (`BlockchainInfo.signed_frontier`, `BlockchainInfo::network_height`), `engine_warnings.rs`, `catchup_assist.rs`, `snapshot_serve.rs`, `diagnostics.rs`, `fork.rs` (wording), `apps/node/src-tauri/src/commands.rs`, `wallet.rs`, `ask.rs`, `tools.rs`, `apps/node/src/main.ts` |

## Context

**What changed in the engine.** From mainnet block 244,000 BTX v0.34.15 does
not let a MatMul header that has only passed the compact-target precheck
become the best header. `AddToBlockIndex` (`src/node/blockstorage.cpp:822`
at 476f3f23) computes `phase1_must_not_lead` and skips the best-header update
for such a header unless its block was validated or carries
`BLOCK_TRUSTED_REPLAY_ATTESTED`; `MatMulDigestBelowTargetAuthenticatesWork`
(`src/pow.cpp:3764`) always answers no. The height is
`nMatMulPhase1HeaderNotMostWorkHeight`, set to 244,000 for mainnet only; the
struct default is `int32` max and regtest has no flag for it
(`src/consensus/params.h:981`), so regtest cannot reproduce what follows.

**What that did to the app.** `getblockchaininfo.headers` is the best header.
While a node catches up it now sits near `blocks` instead of near the tip, so
every reader of `headers - blocks` under-counts. Measured on a test install
(NEXT-SESSION.md of 2026-10-11): the app said 4,888 blocks behind when the
real gap was about 5,260. The readers:

- the Ready card's `blocks_behind` and the Syncing card's number
  (`commands.rs`, `phase`), which the role line, the trend line and the
  status tool read;
- the gap window that feeds the fork alarm and the macOS engine priority
  (`commands.rs`);
- the stall watchdog's heights (`commands.rs`, `watchdog::discriminate` sees
  no gap and gives no verdict);
- the catch-up help's on/off gate (`catchup_assist::tick_until`,
  `t.headers < t.blocks + MIN_BEHIND`);
- the warning about falling behind the signers (`engine_warnings::from_node`);
- the snapshot keeper's "never dump while behind" gate
  (`snapshot_serve::read_facts`);
- the wallet's catch-up note (`wallet.rs`) and "Block X of ~Y" (`ask.rs`).

Nothing restarts on these numbers, so nothing broke loudly. The catch-up help
and the stall diagnosis simply went quiet.

**What a node can see, measured read only on btxscan's btxd2 (v0.34.15, a
trusted mirror at the tip) on 2026-10-11 around 21:55 UTC:**

- `getblockchaininfo` carries `matmul_signed_frontier`: height 245,546,
  `on_active_chain` true, `on_chain_attested_height` 245,546, `blocks_behind`
  0, the same object `getmatmulattestedtip.signed_frontier` returns. The help
  text: present when `-matmultrustedpubkey` is configured; "highest height
  this node has a current-key quorum for, including hashes it has not
  fetched"; `on_active_chain` is true for an ancestor, the tip, "or descendant
  / catch-up".
- `getpeerinfo`: 27 peers; `synced_headers` was -1 for 8 of them, including
  the 3060 (inbound, `startingheight` 245,498). The rest ranged from 123,486
  to 245,546; their median was 245,512, 34 below the tip.
- `getchaintips`: 2,905 entries.

**Who pins a key, and so has a frontier** (`crates/btx-core/src/node.rs`):
every node that follows signatures, every validating node that started from a
signed snapshot (`validating_snapshot_pin_args`, which is every new install
since 0.7.0), and every signer (its own key). A validating node that started
long ago from the snapshot compiled into the engine and signs nothing pins no
key and has no frontier.

**Why not peers or chain tips.** A header that has only passed the
compact-target precheck costs nothing to make: the digest is not checked
against the work until the body is. That is exactly why upstream stopped such
headers from leading. Any peer can therefore hand a node a tower of them on
top of its own tip, and both `synced_headers` (each peer's best announced
header) and `headers-only` chain tips count it. Reading `getchaintips`
unfiltered is how the false tower after 227,313 (2026-09-23) and the body-less
tower of 2026-09-06 (`docs/incident-2026-09-06-bodyless-tower.md`) showed up
as "a longer chain exists". Filtering through the refused branches helps with
a known bad branch, not with a fresh one. And as measured above they are poor
witnesses even when everyone is honest.

## Decision

**The network height is the highest of three heights, each one something a
stranger cannot invent:**

1. `blocks`, the node's own tip;
2. `headers`, the engine's best header, which since 244,000 is a header the
   node checked or one a pinned key signed;
3. the signed frontier's height, but only while `on_active_chain` is true. A
   frontier on another branch is a signer somewhere else, never a gap this
   node can close by catching up, and the warning about it stays.

`btx_core::network_height::network_height(blocks, headers, frontier)` is the
rule; `BlockchainInfo::network_height()` applies it to one
`getblockchaininfo`, whose new `signed_frontier` field decodes
`matmul_signed_frontier` and never fails the call (an unexpected shape
decodes to `None`, as the warnings already do).

**Every reader listed in the Context uses it.** One exception, on purpose:
while a node has no block yet (`height` 0, the "Counting the chain" screen),
the Syncing card keeps counting raw headers, because that is the number that
moves there (`commands::phase_target`).

**Wording follows the number.** The main screen says "Catching up to block N"
instead of "headers at N", the status tool says "height X of N", and the fork
alarm says "newer blocks beyond its own" instead of "block headers", because
the gap may now come from signatures.

**Copy diagnostics shows it**, under Chain: the signed frontier, whether it is
on this chain, and the network height with its distance above the tip.

## What this does not do

- It does not make a node without a pinned key see further than before:
  there it is `headers`, as on 0.7.10. That is an old validating install,
  which catches up from its own validation and peers as it always did.
- It does not touch the fork check (`fork::longer_branch` over
  `getchaintips`), which keeps its own rules about towers.
- It does not change what the app tells the website (`checkin.rs` still sends
  the raw `headers`).
- It does not trust a signer more than the node already does: the frontier is
  the pins the node already follows.
- It does not count a frontier that sits just across a short fork. A node
  whose tip lost a one-block race has the winning chain's frontier off its own
  chain (`on_active_chain` false, measured below), and the rule leaves it out.
  On a node that follows signatures `headers` carries the same height there,
  so nothing is lost; a validating node in that shape reads near its own tip,
  as it did on 0.7.10. Counting a frontier whose branch leaves ours within the
  engine's park depth (6) would close that, but it needs the fork point, which
  `getblockchaininfo` does not give. Left for later.

## Validation

- Unit tests for the rule, the decoding (the measured btxd2 object, the
  catching-up shape, a node with no pin, malformed shapes), and each reader:
  the engine warning, the catch-up gate, the keeper's gate, diagnostics, the
  phase, the wallet.
- Read only on a real v0.34.15 node (btxd2, above): the frontier rides in
  `getblockchaininfo` in the shape decoded here.
- Read only on a second real v0.34.15 node, an easyNode 0.7.10 test install
  that follows signatures and has stalled behind the tip (found running on the
  release Mac on 2026-10-11 at 21:31 UTC, started by the test install, not by
  this work): `blocks` 242,639, `headers` 244,690, frontier 244,689 with
  `on_active_chain` false, api.btxscan.io's tip 245,557. Its tip `7296ebc9…`
  is a stale sibling: the chain's 242,639 is `4045784f…`, both on the same
  242,638, and the frontier `dd089a28…` is btxscan's block at 244,689. So
  `headers - blocks` said 2,051, the rule says the same, and neither can say
  more than the node has heard: it stopped hearing at 244,690.
- Not measured: a validating v0.34.15 node in the middle of a long catch-up on
  its own chain, which is the case this fixes. Regtest cannot switch the rule
  on. The first 0.7.11 install that starts from the pinned pair is that
  measurement: its Copy diagnostics prints the frontier and the network height
  beside `headers`.

## Rollback

Revert the readers to `chain.headers`. The new field is additive, and nothing
is written to disk.
