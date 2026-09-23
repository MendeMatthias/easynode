# Serving a chain snapshot

The "Serve a chain snapshot" switch in Settings. This is the producer half of
BTX's attested-snapshot mechanism, as a role the app runs on its own. The
code is `crates/btx-core/src/snapshot_serve.rs` (every decision, with tests)
and the keeper loop in `apps/node/src-tauri/src/commands.rs`.

## The problem

Every new node bootstraps from a UTXO snapshot compiled into the engine. The
newest one published with a release is `utxo-btx-main-203000.dat` from
v0.34.5. Upstream's standalone `assumeutxo-219000` pre-release needs a
chainparams entry that v0.34.9 carries (its comment says 0.34.7, a version
upstream never tagged), and the app pins it from the v0.34.9 engine on. Either
way the file lags the chain by thousands of blocks, a new node closes that gap
itself, and a consumer machine in consensus mode splits its
accelerator between the tip and the background re-verification: measured on
an M5, 17.1 blocks/h each against a chain doing 39/h. It loses ground.

BTX has had a complete answer since 0.34: `dumptxoutsetattested` exports the
chain state with this node's signature in a manifest,
`offerattestedutxosnapshot` serves it over P2P in 1 MiB chunks and sets
service bit 32 (`NODE_ATTESTED_UTXO_SNAPSHOT`), `fetchattestedutxosnapshot`
pulls it, and `loadtxoutsetattested` activates it at a height that need not be
compiled in, because `m_attested_assumeutxo` overrides
`chainparams.AssumeutxoForHeight()`. Measured on 2026-09-20 across 62
reachable peers: zero advertised bit 32. Nobody had used it.

On 2026-09-21 one RTX 3060 in a home produced the first one (base 225,186),
served it, and a throwaway second node fetched all 9 chunks in 19 s and
activated it in 2 s. It took four shell scripts and a person watching them.

## What the role does

1. **Gate.** Refuses, with the reason beside the switch, unless the node
   validates itself (`consensus` mode), holds a signing key, knows the four
   RPCs (v0.34.6 and later), and has 200 MB free. Waits, without refusing,
   while the node is in initial block download or more than 5 headers behind
   its own blocks.
2. **Export at the tip** into a staging pair under `<datadir>/snapshots/`.
   Measured: 140,936 coins in 0.15 s, `cs_main` held for 7.4 ms (5.8 to 15.9
   across five runs). The node is not disturbed.
3. **Wait for ten confirmations.** The RPC takes no height argument and
   always bases on the 0-conf tip. BTX mints a competing sibling about every
   25 blocks, and the very first snapshot taken was orphaned within 40
   seconds and had already been offered. Upstream parks reorgs deeper than 6;
   ten is that with margin. A base whose confirmations reach -1 is exported
   again. The previous offer stays live throughout, so serving never stops.
4. **Re-verify, then swap.** `getblockhash` at the base height must still
   equal the base hash. Then withdraw the old offer, offer the new pair
   (offering on top of a live offer is untested), and only then write
   `current-offer.json`. A failed swap leaves the previous record, so the
   keeper falls back to the previous pair and not to nothing.
5. **Re-make the mirror links.** The engine sends its service bits exactly
   once per connection, in the VERSION message (`PushNodeVersion` sends
   `peer.m_our_services`, snapshotted when the socket is created), and
   nothing re-announces a change. A peer connected before the offer never
   learns of bit 32. Measured: the explorer's mirror connected at 14:30Z, the
   offer was re-asserted at 14:34Z, and at 16:46Z the mirror still saw no
   peer with bit 32. So after every offer the links to the mirror hosts the
   signer role already dials are disconnected and redialled.
6. **Keep two pairs**, the offered one and the one before it.
7. **Refresh every 500 blocks**, about eleven hours at 45 blocks/h.
8. **Re-offer after every node start.** The offer lives in the running
   process only; a restart drops it silently. It was lost four times in one
   day before that was understood. The keeper re-offers the recorded pair
   once the node is at the tip and the base is still on the active chain.

## What the role does not do

It does not change what any node loads. `loadtxoutsetattested` refuses on a
consensus node by design ("Strict consensus nodes must refuse this RPC"), and
`fetchattestedutxosnapshot` is only available under `matmulvalidation=trusted`.
Loading an attested snapshot therefore means running as a trusted mirror and
taking the signers' word for the chain state. At a 1-of-1 quorum that is one
key's word. easyNode's bootstrap (`btx_core::snapshot`) still loads the
compiled pin with `loadtxoutset`, and pointing it at an attested snapshot is a
trust decision to make separately, not a URL to swap.

A snapshot only helps importers that pin the producing key. A signed file
nobody pins is a signed file.

## Things that are not obvious

* The manifest is binary despite the `.json` name the engine's own examples
  use: 335 bytes starting `02 01 46 fa`, carrying the base hash, the txoutset
  hash, the shielded state pin and the signer key. Never parse or edit it.
* `offerattestedutxosnapshot` returns a `file_hash` that is the
  byte-reversed double SHA-256 of the file, not the plain sha256. The record
  keeps both because both get asked for.
* `btx-cli` cannot pass the optional `peer_id` and `timeout_ms` arguments to
  `fetchattestedutxosnapshot` (they are missing from its conversion table and
  arrive as strings). Only the three-argument form works from the CLI. Worth
  reporting upstream.
* Serving does not require being a signer. Any host with the pair may offer
  it. Producing does.

## What was served from the first machine

```text
base_height    226140
base_hash      64e144d563e6b913ae46584230a94e3a91550e5c80e33874721e3ff86d58fe11
txoutset_hash  dcf828e4fd219bbf9ab0299d90775d106843d5e609d1a675e08a5f2aa6ce4a23
file           utxo-btx-main-226140.dat   9,059,813 bytes
sha256         73ecb7a11b3dc6c9d703aaadc965b24e7e155e4a396bb2ef166561300f69e1e5
manifest       335 bytes, sha256 a226bb09c4fb054d5b092d01dfe16bba9e0fd4601707a904eeb5935e60a4f7a7
offer          9 chunks of 1 MiB, file_hash f4607b196e17b111c451e3a087fe35cbeca8c5f48ff0f2cd5669f9ecb89b9f96
```

The previous pair (225,927) is also published as release assets at
`MendeMatthias/EasyBTX-releases`, tag `utxo-snapshot-225927`, because the
manifest is self-verifying and the transport does not have to be trusted.
