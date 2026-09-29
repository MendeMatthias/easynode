# Every node starts from a confirmed snapshot near the tip

| | |
|---|---|
| Status | proposed 2026-09-29 for 0.7.0. Builds on what the owner approved on 2026-09-29: a snapshot counts only with signatures from two different operators on a short list compiled into the app, checked by the app before any engine command; confirmers stay out of the mirrors' pin list (the spike's recommendation, "best to do whats recommended"); snapshots at fixed heights; Fast-forward instead of "request snapshot". Adds what the owner asked for the same day: "the easynode should urge to find the latest trusted / verified snapshot instead of starting from early blocks". This document waits for the owner's review before any code |
| Date | 2026-09-29 |
| Supersedes | the single-signed pointer of 0.6.31 (`utxo-snapshot-latest`, never published); the rule that a validating node starts from the engine's compiled start point; the keeper's "every 500 blocks from whenever it last ran" (`snapshot_serve::refresh_due`); `scripts/publish-attested-snapshot.sh` as the only publisher |
| Leaves alone | the engine; the mirrors' pinned keys and threshold (`BTX_TRUSTED_ATTESTATION_PUBKEYS`, threshold 1); the held branches (`known_invalid.rs`); the node check-in, which never counts as a confirmation; the fallbacks to the pinned pair and the compiled start point |
| Origin | the owner's plan of 2026-09-29, phase 2; the spike of 2026-09-29 (`~/Desktop/easyNode-0.7.0-spike-cosigned-snapshots-2026-09-29.md`); the mainnet test on the owner's Mac the same day (`~/Desktop/easyNode-0.7.0-mainnet-test-on-this-Mac-2026-09-29.md`); Aleksander's nodes, three days from the tip on 29 September after starting at 219,000 |
| Code | new in `crates/btx-core/src`: `operators.rs` (the list), `confirmed_snapshot.rs` (read, verify and trim a manifest), `diary.rs`, `catchup_assist.rs`. Changed: `attested_snapshot.rs`, `snapshot.rs`, `snapshot_serve.rs`, `node.rs` (pins on the validating arm), `apps/node/src-tauri/src/commands.rs` (setup, keeper, refresher, Fast-forward), `crates/btx-core/Cargo.toml` (the `ecdsa` feature of `k256`, which adds the small RustCrypto crates `rfc6979` and `hmac` to the lock file). In the website repository: `site/src/pages/api/snapshots/*.ts`, `site/src/lib/snapshotRendezvous.mjs`, their tests, `@noble/curves` as a direct dependency |

## Context

### What the engine does (v0.34.9, measured on regtest with throwaway keys)

- `loadtxoutsetattested` refuses the whole manifest if any signature comes
  from a key the node does not pin (`untrusted-signer`), in any order.
- A signature covers only the statement, so dropping signatures is lossless:
  removing two co-signatures gave back the producer's file byte for byte, and
  it loaded.
- The engine stores the manifest with the snapshot and re-checks it at every
  start against the pins, threshold and replay context of that moment. If the
  check fails, the node does not start ("Attested snapshot manifest is present
  but failed verification under the current authority configuration") until
  its background check has finished, which takes days on mainnet.
- `signutxosnapshotmanifest` signs blindly: a node at height 0 co-signed a
  statement about block 100.
- `gettxoutsetinfo` at the tip returns the statement's `hash_serialized_3`,
  block hash and coin count. The statement's transaction count is the chain's
  running total (`getchaintxstats` `txcount`).
- A base block whose header is marked failed is refused: "The base block header
  (...) is part of an invalid chain" (`validation.cpp:17539`). A block the app
  refuses with `invalidateblock` should mark every block above it that way;
  the rehearsal proves it on regtest before anything relies on it.

### What the mainnet test on the owner's Mac showed (29 September)

- **No near-tip snapshot has ever been published.** `utxo-snapshot-latest` does
  not exist on EasyBTX-releases, so every node that follows signatures has
  started at 225,927 (21 September) since 0.6.31, today 7,500 blocks back.
- **A node that loaded a signed snapshot as a mirror restarts as a validating
  node,** as long as the snapshot's signer stays pinned. The engine logged
  "restored attested-fast-forward assumeutxo override at height 225927" in
  consensus mode. Without the pin it refuses to start (regtest, both with and
  without a signing key of its own). Not yet seen: such a node checking new
  blocks, because the owner's M2 Pro fails the engine's device self-test.
- **Catching up stalls on block delivery, not on checking.** Almost every peer
  on our chain advertises only recent history (`NETWORK_LIMITED`, the last 288
  blocks), as any node started from a snapshot does until its background check
  finishes. The engine asks such a peer for an older block only through its
  120-second rescue: about 26 blocks an hour, below the chain's 40. Asked by
  name with `getblockfrompeer`, the archive peer 109.199.124.187 served 7,490
  blocks in 8 minutes.

### The statement, verified independently

The engine hashes a statement as double SHA-256 of the length-prefixed domain
`BTX_TRUSTED_UTXO_SNAPSHOT_ATTESTATION_V2` followed by the 229-byte statement,
and signs that digest with strict-DER, low-S ECDSA on secp256k1. The file hash
is double SHA-256 of the file. A 60-line Python check reproduced all of it on
the published 225,927 pair: statement hash `d3ee9312...0482`, file hash
`f234192d...eb2c`, the 3060's signature valid. The mainnet chain id is
`75a998a3...4601` and the replay context of 0.34.9 is `32ad5c2e...7188`. These
are the test vectors for both the app and the website.

### What exists

- `attested_snapshot.rs` fetches the pointer, checks only its shape, and never
  parses the manifest; the engine does all the trusting.
- The keeper exports every 500 blocks counted from its last run, waits 10
  confirmations and checks the base is still on the active chain. It does not
  check held branches.
- No diary exists, and nothing calls `gettxoutsetinfo`.
- `k256` 0.13.4 is in the tree with `ecdsa` 0.16.9 already locked. Its `ecdsa`
  feature verifies these signatures and adds `rfc6979` and `hmac` to the lock
  file; nothing else new.
- The website runs Astro on Vercel functions (Node runtime). A request body is
  capped at about 4.5 MB. `@vercel/blob` is in use. Nothing verifies a
  secp256k1 signature yet; `@noble/curves` is present only as a dependency of
  a dependency.

## Decision

### 1. The operator list

Compiled into the app as `operators.rs`: each operator has a name and one or
more public keys, and all of an operator's keys together count as one. The
first list:

- **Mende**: `02d5efca...4675` (the 3060).
- **Aleksander**: `03047189...a24` (zbtx2) and `02c9cfb7...0a0` (zbtx3), only
  once he has agreed.
- **jpp**: only once he has agreed and sent a key.

The list changes only with a signed app update. The website keeps a copy for
filtering, and a check in the website repository compares it with this file.
While the list holds a single operator, nothing counts as confirmed and every
node uses the fallbacks in section 8.

### 2. Fixed heights: every multiple of 200

Proposed instead of the approved 500, because of the 288-block limit above. A
snapshot is at most about 200 blocks plus confirmation time old, so the blocks
after it are usually within what every peer serves. 200 divides 1,000, so the
diary also covers upstream's compiled heights (219,000, 228,000).

### 3. The diary, on every validating node

When the node's tip reaches a multiple of 200, the app calls `gettxoutsetinfo`
and keeps the answer only if its `height` and `bestblock` are that block. It
records height, block hash, `hash_serialized_3`, coin count (`txouts`) and the
chain's transaction count (`getchaintxstats` at that block) in
`<datadir>/snapshot-diary.json`, newest 100 entries, written atomically. A
height the tip passed before the app could read it is skipped; the next chance
is 200 blocks later.

It records only while the node's chain state rests on its own checks and the
engine's compiled start point. While the node runs on a signed snapshot whose
background check has not finished (the engine's `attested_assumeutxo` file
exists), it records nothing. Otherwise a snapshot could vouch for the next one.

### 4. Producers

A node with "Serve a chain snapshot" on, that validates, and whose key is on
the list. It exports with `dumptxoutsetattested` when its tip is exactly on the
grid. After 10 confirmations it checks, again, that:

- the base is still the block at that height on its active chain;
- every held root it knows is refused on this node;
- its own diary entry for that height matches the statement field by field.

Then it sends the statement and the file to easybtx.com, and keeps offering the
pair over P2P as today. A producer never sends a pair that failed any check.

### 5. Confirmers

A node that validates, signs, and whose key is on the list. About every ten
minutes it asks easybtx.com for statements waiting for confirmation. For each
one whose height is in its diary, it checks every field against the diary, the
chain id and replay context against its own node, and that the block has at
least 10 confirmations on its own active chain. Only if everything matches
does it run `signutxosnapshotmanifest` on a copy and send back the one new
signature. It never signs without a diary match. A mismatch is logged, counted
in Copy diagnostics, and not signed.

### 6. The meeting point on easybtx.com

Nothing here is trusted by the app. It filters, stores and points.

- **`POST /api/snapshots/statement`**, a manifest of at most 64 KB. Accepted
  only if every signature is valid, strict DER and low-S, and from a key on the
  list; the chain id and replay context are the expected ones; the height is on
  the grid; the file size is at most 64 MB. Signatures on the same statement
  are merged, one per key.
- **The file**, uploaded in parts of at most 4 MB through the same functions
  (Vercel Blob multipart), only for a statement already stored. When the last
  part is in, the server hashes the whole file and keeps it only if size and
  hash match the statement.
- **`GET /api/snapshots/pending`**: statements of the last seven days and who
  has signed each, for confirmers.
- **`GET /api/snapshots/latest`**: the newest statement signed by two
  different operators whose file is stored and checked. Height, block hash,
  statement hash, file and manifest addresses, sizes and hashes, and which
  operators signed.
- Storage stays small: the newest five confirmed pairs and the pending ones.

### 7. Loading: one path for setup, Fast-forward and every kind of node

1. Read `latest`. Download the manifest and check it in the app before the
   engine sees anything: parse strictly (no trailing bytes); version 2; chain
   id and replay context as compiled for this engine and as the running node
   reports; height on the grid and above the start point the node would
   otherwise use; every signature strict DER, low-S and valid over the
   statement; signers grouped by operator, **at least two different
   operators**; and at least one signature from a key the node pins.
2. Download the file, hashing as it streams. Size and hash must match the
   statement.
3. Just before loading, refuse every held root the node knows, as the fork
   check already does every 30 seconds. The engine then refuses a base above
   any of them.
4. Write a trimmed manifest that keeps **every** signature from a key the node
   pins, in order, and drops the rest.
5. Load it with `loadtxoutsetattested`. The engine allows this only in mirror
   mode, so a node that validates is started once as a mirror for the load.
6. After the load, check that the block at each held root's height is not the
   root. If it ever were, stop the node and restore the chain data (section 9).
7. A node that validates restarts as a validating node straight away, under
   section 8's pin rule. From then on it checks every new block itself.

### 8. A validating node on a signed snapshot

- While `<datadir>/chainstate_snapshot/attested_assumeutxo` exists, the
  validating arm also pins the mirrors' keys, beside its own. In consensus
  mode the engine treats them as telemetry: they never skip a check and never
  steer which chain it follows. Without them it does not start.
- The file goes away when the background check finishes and the engine retires
  the snapshot. Whether dropping the pins then costs the node anything is to be
  measured on a real data folder before it ships; if it does, they stay.
- It keeps signing blocks: a block signature vouches for the node's own proof
  check, which it does.
- It keeps no diary and confirms nothing until the background check is done
  (section 3).
- Honest words on screen: it checks every new block itself; its older history
  is checked in the background, with progress shown (UI decision).

Upstream refuses signed snapshots on validating nodes on purpose. This path
passes the engine's own checks at every start, but it bends that intent, so an
upstream issue is drafted for the owner (below).

### 9. Fallbacks, the same for every node

The confirmed snapshot, else the app's pinned pair (225,927), else the engine's
compiled start point (219,000; 228,000 once 0.34.12 ships). The pinned pair
carries one operator's signature, but its hashes are compiled into the app, so
it is trusted as the app is.

### 10. Fast-forward

The Tools button (the Tools decision fixes where it sits and how it asks),
for any node the app owns that is more than 1,000 blocks behind a confirmed
snapshot:

1. Check and download the snapshot (section 7, steps 1 and 2) while the node
   keeps running.
2. Stop the node. Move `blocks`, `chainstate`, `chainstate_snapshot` and the
   indexes into a dated folder beside them. Wallets, keys, the conf, settings,
   peers, bans and the diary stay where they are.
3. Start as a mirror, wait for headers to reach the base, load (section 7,
   steps 3 to 6), and restart as a validating node if it is one.
4. On success, delete the dated folder. On any failure, stop, move it back,
   start as before, and say what failed in one sentence.

A wallet last used below the snapshot height opens once the background check
reaches that height; the confirmation says so.

### 11. Catch-up help, on every node

When the tip is at least 20 blocks behind, and the next block has not been
requested from anyone for 30 seconds, the app asks the archive peers it dials
for the next 100 blocks by name (`getblockfrompeer`). It then waits for them to
connect before asking for more, and rotates to the next archive peer if a
batch has not connected within three minutes. It follows the chain toward the
signed frontier the node already reads (`getmatmulattestedtip`); a node that
reads none follows its best header. It never adds, bans or disconnects a peer,
and it stops as soon as the engine is fetching on its own again. The watchdog's
`BlockFetchGated` verdict names this help instead of "nothing in this app does
yet".

### 12. Rules the spike wrote

- A test fails if a release removes a key from `BTX_TRUSTED_ATTESTATION_PUBKEYS`
  or raises the mirrors' threshold. The test compares against a checked-in list
  of every key ever shipped. A key is retired by blocklisting it
  (`-matmulattestationblocklist`), which the engine tolerates at restart.
- `scripts/check-engine-tag.sh` gains a regtest check before any engine bump:
  the replay context equals the compiled one, and a node on a signed snapshot
  still restarts as a validating node.

### 13. Right now, before 0.7.0

Not code, and outward-facing, so the owner's decision: publish a fresh signed
snapshot from the machine whose keeper serves it, and schedule
`publish-attested-snapshot.sh` there. Mirrors on 0.6.32 would then start within
a day of the tip instead of at 225,927.

## Tests

- **Vectors shared by the app and the website**: the real 225,927 manifest and
  file (statement hash, file hash, the 3060's signature), and the regtest
  manifests from the spike with one, two and three signers.
- **Parser**: truncated, trailing bytes, an oversized signature count, a
  version-1 statement.
- **Signatures**: valid; high-S; non-strict DER; the right signature under the
  wrong key; two keys of one operator counted once; an unknown key ignored for
  the count and dropped from the trimmed file.
- **Trimming**: byte-identical to the producer's file when only the pinned
  signature is kept.
- **Diary comparison**: one sabotage case per field (block hash, UTXO hash, coin
  count, transaction count, chain id, replay context, fewer than 10
  confirmations, a base off the node's chain).
- **Grid, fallbacks, the pin rule** of section 8, and pins only growing.
- **Catch-up help**: target and peer choice over recorded RPC answers.
- **Website**: the same vectors in vitest; merging; the choice of `latest`;
  refusals for an unknown key, a wrong chain, off-grid heights, oversize
  bodies, a file whose hash does not match.
- **Rehearsal on regtest with test keys** (step 4 of the owner's plan): two
  confirmers, one producer, one mirror, one validating node. Proven refused: a
  statement signed by one operator, a statement on a held branch, a diary
  mismatch, a wrong file hash. Proven working: the validating restart on a
  signed snapshot, and Fast-forward's roll-back after a failure.
- **Real machines**: a validating node on a card the engine qualifies, loaded
  from a signed snapshot, checking new blocks. Not possible on the owner's
  Mac.

## What this does not do

- It does not let a validating node skip checks. Every block after the snapshot
  is checked by the node; history before it is checked in the background.
- It does not make confirmers authorities over blocks. The mirrors' pins stay
  as they are.
- It does not trust easybtx.com, GitHub or the node check-ins.
- It does not change the engine.

## Rollback

- Website: `latest` returns nothing, and every node falls back (section 9).
- App: the confirmed path, the validating restart and the catch-up help each
  sit behind one constant. The diary is harmless on its own.

## Choices for the owner

1. The grid of 200 instead of 500 (section 2).
2. Validating nodes start from the confirmed snapshot as section 8 describes,
   with that trust stated plainly on screen.
3. The pinned pair as a fallback for validating nodes as well (section 9).
4. Fast-forward from 1,000 blocks behind.
5. **Who has agreed to be on the list?** Aleksander and jpp, per the message
   you sent the group. Until a second operator is on it, nothing counts as
   confirmed.
6. Section 13: publish a fresh single-signed snapshot now?

## Upstream issues to draft for the owner

Exempting the background check from the cadence hold; separate snapshot keys
and threshold; co-signing that checks the signer's own chain; a snapshot hash
at a past height. Two new ones from this work: an official way for a
validating node to start from a signed snapshot, and the block downloader
asking limited peers for older blocks sooner than its 120-second rescue.
