# Every node starts from a confirmed snapshot near the tip

| | |
|---|---|
| Status | proposed 2026-09-29 for 0.7.0; amended 2026-09-29 (night) after jpp's review. Now decided by the owner: the operator list and its rules (section 1; Aleksander's entry merges only once the owner confirms he agreed); the grid of 100 (section 2); sending and signing only at 144 blocks deep (sections 4 and 5); nothing recorded, produced or signed on an unvalidated snapshot chainstate (section 3); the standalone confirmer (section 5a); any disagreement freezes Fast-forward until the owner clears it (section 6a); validating nodes start from the confirmed snapshot (section 8); the pinned pair as a fallback for every node (section 9); Fast-forward from 1,000 blocks behind (section 10). Still open: publishing a fresh signed snapshot from the 3060 before 0.7.0 (section 13). Builds on what the owner approved earlier that day: a snapshot counts only with signatures from two different operators on a short list compiled into the app, checked by the app before any engine command; confirmers stay out of the mirrors' pin list (the spike's recommendation, "best to do whats recommended"); snapshots at fixed heights; Fast-forward instead of "request snapshot"; and "the easynode should urge to find the latest trusted / verified snapshot instead of starting from early blocks". The amended text waits for the owner's review before any code |
| Date | 2026-09-29, amended the same night |
| Supersedes | the single-signed pointer of 0.6.31 (`utxo-snapshot-latest`, never published); the rule that a validating node starts from the engine's compiled start point; the keeper's "every 500 blocks from whenever it last ran" (`snapshot_serve::refresh_due`) and its 10-confirmation wait (`CONFIRMATIONS_REQUIRED`, `snapshot_serve.rs:69`); `scripts/publish-attested-snapshot.sh` as the only publisher. From this document's first draft: the grid of 200, the 10-confirmation depth, and the diary's test for the `attested_assumeutxo` file |
| Leaves alone | the engine, and its background check, which keeps running by default; the mirrors' pinned keys and threshold (`BTX_TRUSTED_ATTESTATION_PUBKEYS`, threshold 1); the held branches (`known_invalid.rs`); the node check-in, which never counts as a confirmation; the fallbacks to the pinned pair and the compiled start point |
| Origin | the owner's plan of 2026-09-29, phase 2; the spike of 2026-09-29 (`~/Desktop/easyNode-0.7.0-spike-cosigned-snapshots-2026-09-29.md`); the mainnet test on the owner's Mac the same day (`~/Desktop/easyNode-0.7.0-mainnet-test-on-this-Mac-2026-09-29.md`); Aleksander's nodes, three days from the tip on 29 September after starting at 219,000; jpp's review of the first draft, the night of 29 September |
| Code | new in `crates/btx-core`: `snapshot-operators.json` (the list), `src/operators.rs` (compiles it in), `src/confirmed_snapshot.rs` (read, verify and trim a manifest; build a dissent), `src/diary.rs`, `src/confirm.rs` (the four checks and the sign-or-dissent verdict, shared by both confirmers), `src/catchup_assist.rs`, `src/bin/btx-confirmer.rs`; `deploy/esplora/btx-confirmer.service.template`. Changed: `attested_snapshot.rs`, `snapshot.rs`, `snapshot_serve.rs`, `node.rs` (pins on the validating arm), `apps/node/src-tauri/src/commands.rs` (setup, keeper, refresher, Fast-forward, the start record), `apps/node/src/main.ts` (who confirmed the start point), `crates/btx-core/Cargo.toml` (the `ecdsa` feature of `k256`, which adds the small RustCrypto crates `rfc6979` and `hmac` to the lock file), `deploy/esplora/README.md`. In the website repository: `site/src/lib/snapshot-operators.json` (the same file) and a CI step that compares it, `site/src/pages/api/snapshots/*.ts`, `site/src/lib/snapshotRendezvous.mjs` (with the dispute rule), `scripts/clear-snapshot-dispute.mjs`, their tests, `@noble/curves` as a direct dependency |

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
  statement about block 100. Before signing it checks only that the
  statement's chain id and replay context are its own and that its key is not
  blocklisted (`src/matmul/trusted_exact_replay_attestation.cpp:1387-1403`).
  It does not look at the file fields.
- `gettxoutsetinfo` at the tip returns the statement's `hash_serialized_3`,
  block hash and coin count. The statement's transaction count is the chain's
  running total (`getchaintxstats` `txcount`).
- A base block whose header is marked failed is refused: "The base block header
  (...) is part of an invalid chain" (`validation.cpp:17539`). A block the app
  refuses with `invalidateblock` should mark every block above it that way;
  the rehearsal proves it on regtest before anything relies on it.

### What the engine's code says (read at 84b998b4, compared with v0.34.12)

- **`getchainstates` says whether a snapshot is still unchecked.** Each entry
  in `chainstates` has `"validated"`, false for a chainstate built from a
  snapshot while the background chainstate still exists, true otherwise
  (`src/rpc/blockchain.cpp:5219`, emitted at `:5200`, help text at `:5141`).
  An entry built from a snapshot also carries `snapshot_blockhash` (`:5197`).
  This is the same for upstream's plain assumeutxo snapshot and for a signed
  one. Only a signed load writes `chainstate_snapshot/attested_assumeutxo`
  (`validation.cpp:17791` and `:17818`).
- **Limited peers serve 288 blocks.** A `NETWORK_LIMITED` peer serves only its
  last 288 (`NODE_NETWORK_LIMITED_MIN_BLOCKS`, `net_processing.cpp:512`), and
  the downloader asks one for a block only while it is less than 286 below that
  peer's best block (`:6992`).
- **Reorg profiles** (`init.cpp:686-687`): the default emergency profile parks
  rewrites deeper than 6 blocks, and the archive profile "alarms after 72".
  easyNode turns parking off with `-parkdeepreorg=0` (`node.rs:805` on main).
- **The replay context depends only on the chain.** It is computed from the
  chain parameters and a schema version in the code
  (`ComputeMatMulReplayAuthorityContext`, `src/node/blockstorage.cpp:430`).
  0.34.12 changes nothing in it, and it was measured identical on 0.34.9 and
  0.34.12: `32ad5c2e148149752a312561dc0b6879c9cc41fdf4bc09edcdd5e2bd09af7188`.
- **The shielded commitment is fixed near the tip.** The mainnet shielded pool
  closed at 199,300 (`src/kernel/chainparams.cpp:57`). Every compiled snapshot
  since carries the same commitment, `94343b76...4541` (`chainparams.cpp:1278`
  for 219,000, and the same at 228,000 in 0.34.12), and the attested export
  writes that value for a closed pool (`src/node/attested_utxo_snapshot.cpp:110-116`,
  `validation.cpp:19051`).
- **A statement without a file cannot be loaded.** The engine refuses a
  statement with a null shielded commitment, a zero file size or a null file
  hash (`src/matmul/trusted_utxo_snapshot_attestation.cpp:90-93` and `:102-108`).

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

The statement is 229 bytes (`src/matmul/trusted_utxo_snapshot_attestation.h:31-70`):
version (1 byte), chain id (32), block hash (32), height (4),
`hash_serialized_3` (32), coin count (8), chain transaction count (8),
shielded commitment (32), replay context (32), file size (8), file hash (32),
chunk size (4) and chunk count (4). A signature therefore already binds the
height, block hash, `hash_serialized_3`, coin count, chain transaction count
and file hash, as jpp's point 2 asks; this design adds no field. The statement
has no field for signers, operators or thresholds.

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
  confirmations (`snapshot_serve.rs:69`) for at most 90 minutes (`:93`) and
  checks the base is still on the active chain. It does not check held
  branches.
- No diary exists, and nothing calls `gettxoutsetinfo`.
- `k256` 0.13.4 is in the tree with `ecdsa` 0.16.9 already locked. Its `ecdsa`
  feature verifies these signatures and adds `rfc6979` and `hmac` to the lock
  file; nothing else new.
- The website runs Astro on Vercel functions (Node runtime). A request body is
  capped at about 4.5 MB. `@vercel/blob` is in use. Nothing verifies a
  secp256k1 signature yet; `@noble/curves` is present only as a dependency of
  a dependency.
- `btx-witness` (`crates/btx-core/src/bin/btx-witness.rs`) is the pattern for
  a binary that runs beside a btxd the app does not supervise: it reads the
  node's `.cookie`, and `deploy/esplora/btx-witness.service.template` runs it
  under systemd.

## Decision

### 1. The operator list, published

jpp's point 1: count operators, not keys; publish the list; rotate and revoke
only by release.

One file, `crates/btx-core/snapshot-operators.json`, compiled into the app by
`operators.rs`. Each entry is one operator: a name and one or more compressed
public keys. All of an operator's keys together count once. The list for
0.7.0:

- **Mende**: `02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675`
  (the 3060).
- **Aleksander**:
  `026da4e3a07676cada5488123033fb1057faeb7fe6a7d50b2ae3921431e0f4bbf1` (zbtx1),
  `03047189023913e1922c80c895ee2a9e2eff6df05438654749e1a4f95019578a24` (zbtx2)
  and `02c9cfb77d7e4dce0cd6b7968fee1dd53d31ef06c764185e120c825fab8c0572a0`
  (zbtx3). Three keys, one operator. The owner confirms that Aleksander has
  agreed before the change to the list merges.
- **jpp**: `02e0a9653b49ad74900dec86b86770735579e8e64859028a76129c71a70e1cadb5`.
  Listed in 0.7.0. He asked to stay out of the confirmer count until his node's
  background check of history below 228,000 finishes. Nothing in the list does
  that: his confirmer refuses to sign while his node reports an unvalidated
  snapshot chainstate (section 3), which it does until that check finishes.

**Published.** The same file, byte for byte, is
`site/src/lib/snapshot-operators.json` in the website repository, and the
website's CI fails if the two differ. Anyone can read who counts and with
which keys. The website uses its copy only to filter.

**It also lists the keys every node pins** (`mirror_pins`), because a node can
load only a manifest that carries a signature from a key it pins (section 7,
step 1), and today only one listed operator's key, the 3060's, is among them.
An app test fails if `mirror_pins` differs from
`BTX_TRUSTED_ATTESTATION_PUBKEYS`, so the file cannot drift from what the
nodes actually pin. The list is not a second authority: it only lets the site
avoid serving a snapshot no node could load.

**Rotated and revoked only by a signed app release.** Adding or removing a key
or an operator is a change to this file, shipped in an app update. Statements
carry nothing an operator can change about who counts: the engine's statement
has no field for signers, operators or thresholds (Context), and the app counts
only against its compiled list.

While fewer than two listed operators confirm, nothing counts as confirmed and
every node uses the fallbacks in section 9.

### 2. Fixed heights: every multiple of 100

Decided: 100, instead of the approved 500 and the 200 first proposed here, on
jpp's figures. A snapshot is sent and signed only once its block is 144 deep
(sections 4 and 5), so with a grid of 100 the newest confirmed snapshot is 144
to 243 blocks behind the tip, plus the few blocks that the confirmers'
ten-minute poll (about 7 at 40 an hour) and the load add. The first block after
it is then inside the 288 that `NETWORK_LIMITED` peers serve (286 with the
downloader's margin, Context), so any peer can send the rest. With a grid of
200 it would be 144 to 343 behind, and about 27% of the time the blocks after
it would be older than 288, which is where catching up stalls today.

100 still divides 1,000, so upstream's compiled heights (219,000, 228,000) are
on the grid and in the diary. The cost is twice the work of a 200 grid for
producers and confirmers: one export and one signature each about every two and
a half hours. The site still keeps the newest five confirmed pairs (section 6).

### 3. The diary, on every validating node

When the node's tip reaches a multiple of 100, the app calls `gettxoutsetinfo`
and keeps the answer only if its `height` and `bestblock` are that block. It
records height, block hash, `hash_serialized_3`, coin count (`txouts`) and the
chain's transaction count (`getchaintxstats` at that block) in
`<datadir>/snapshot-diary.json`, newest 100 entries (10,000 blocks, about ten
days at 40 an hour, longer than the seven days `pending` covers), written
atomically. A height the tip passed before the app could read it is skipped;
the next chance is 100 blocks later.

The diary reads the tip the moment it reaches the grid, and a sibling can
still replace that block. So an entry counts only while its block is still the
block at that height on the node's active chain; an entry whose block has left
it is dropped and never compared.

The diary holds no shielded commitment. No read-only RPC returns one
(`gettxoutsetinfo` does not; only the export RPCs do), and none is needed near
the tip: on mainnet it is the fixed value in the Context, compiled into the app
per engine beside the chain id and replay context.

**It records only on a chain the node checked itself** (jpp's point 4). Before
every entry the app reads `getchainstates`. While any entry there shows
`"validated": false`, it records nothing. The same test gates producing
(section 4) and confirming (section 5). It replaces this document's first test,
whether the engine's `attested_assumeutxo` file exists: a node on upstream's
plain assumeutxo snapshot (0.34.12 ships one at 228,000, and jpp's own box is
on it today) has no such file and would have recorded. With either kind of
snapshot, the node records again once its background check has finished and
the engine reports every chainstate validated. Otherwise a snapshot could vouch
for the next one.

### 4. Producers

A node with "Serve a chain snapshot" on, that validates, whose key is on the
list, and whose `getchainstates` shows no entry with `"validated": false`. It
exports with `dumptxoutsetattested` when its tip is exactly on the grid, since
the export always takes the tip (`snapshot_serve.rs`, module header). It sends
nothing until that block is at least 144 blocks deep on its own active chain.
144 keeps the blocks after a snapshot inside the 288 that limited peers serve
(section 2), and is twice the depth at which the engine's archive profile
alarms about a reorg (72; the default emergency profile parks beyond 6, which
easyNode turns off). At that depth it checks, again, that:

- the base is still the block at that height on its active chain;
- every held root it knows is refused on this node;
- its own diary entry for that height matches the statement field by field;
- `getchainstates` still shows no unvalidated chainstate.

Then it sends the statement and the file to easybtx.com, and keeps offering the
pair over P2P as today. A producer never sends a pair that failed any check.

Because 144 is more than 100, two exports can be waiting at once. The keeper
waits per exported height instead of one cycle at a time; its maturation
deadline (`MATURE_DEADLINE`, 90 minutes today, `snapshot_serve.rs:93`) grows to
six hours, since 144 blocks take about three and a half hours at 40 an hour;
and it keeps up to four pairs on disk instead of two (`KEEP_PAIRS`, `:84`): the
offered one, the one before it and two waiting, about 36 MB at today's 9 MB
file. A base that leaves the chain while it waits is dropped; the next chance
is the next grid height.

### 5. Confirmers

A node that validates, signs, and whose key is on the list: the app's
confirmer, or `btx-confirmer` beside a plain btxd (section 5a). Both run the
same code. About every ten minutes it reads `getchainstates`; while any entry
shows `"validated": false`, it stops there and signs nothing (section 3).
Otherwise it asks easybtx.com for statements waiting for confirmation and takes
each one that is not a dissent through four checks:

1. The block is at least 144 blocks deep on its own active chain. If not, it
   waits.
2. Its diary has an entry for that height whose block is still on its active
   chain. If not, it neither signs nor dissents.
3. The chain id and replay context are its own node's, and the shielded
   commitment is the compiled one. If not, it logs and neither signs nor
   dissents; the site refuses such a statement anyway.
4. Height, block hash, `hash_serialized_3`, coin count and chain transaction
   count equal the diary entry.

If all four pass, it runs `signutxosnapshotmanifest` on a copy and sends back
the one new signature. If only the fourth fails, it does not sign; it sends a
dissent (section 6a). It never signs without a diary match. Every mismatch is
logged and counted in Copy diagnostics.

### 5a. `btx-confirmer`, beside a plain btxd

jpp's point 5: an operator whose node the app does not run still counts.
`btx-confirmer` is a small binary in `crates/btx-core/src/bin/`, beside
`btx-witness`, built and installed the same way
(`cargo build --release --bin btx-confirmer`, then `/usr/local/bin`).

- **It runs beside any btxd whose chain id and replay context match the app's
  compiled ones.** The replay context depends only on the chain parameters
  (Context), so 0.34.9 and 0.34.12 both qualify. It reads both from the node at
  start, as the app does in section 7, and refuses to run on a mismatch.
- **It does the confirmer's job and nothing else:** the diary (section 3,
  reading the tip every few seconds), the `getchainstates` rule, the four
  checks (section 5), a signature from the node's own
  `signutxosnapshotmanifest`, and the upload of that signature or of a dissent.
  It never exports, loads, pins or restarts anything.
- **It cannot drift from the app.** The diary, the manifest parser, the four
  checks and the dissent are the btx-core modules the app uses (`diary.rs`,
  `confirmed_snapshot.rs`, `confirm.rs`); the binary adds argument parsing and
  the loop. One test suite covers both.
- **What it touches.** It reads the node's `.cookie`, as `btx-witness` does,
  and keeps its diary in `--state <dir>`. `signutxosnapshotmanifest` takes a
  path on the node's own machine, relative to the node's datadir
  (`blockchain.cpp:4631-4632`), so the copy to be signed goes in
  `<datadir>/snapshot-confirmer/`. The node must have its signing key
  configured, as it does to sign blocks; the RPC refuses otherwise
  (`blockchain.cpp:4624-4629`). It listens on nothing.
- **The unit** is `deploy/esplora/btx-confirmer.service.template`, in the shape
  of `btx-witness.service.template`: the same `User=`, `Restart=always` and
  hardening, plus `ReadWritePaths=` for the state directory and
  `<datadir>/snapshot-confirmer/` only. The esplora README gains a short
  section; `install-systemd.sh` does not change.
- **jpp's box counts through it.** It runs plain btxd 0.34.12 on upstream's
  228,000 snapshot, so the `getchainstates` rule keeps it from recording or
  signing until its background check below 228,000 finishes, as he asked.

### 6. The meeting point on easybtx.com

Nothing here is trusted by the app. It filters, stores and points.

- **`POST /api/snapshots/statement`**, a manifest of at most 64 KB. Accepted
  only if every signature is valid, strict DER and low-S, and from a key on the
  list; the chain id, replay context and shielded commitment are the expected
  ones; the height is on the grid and not closed (section 6a); and the file
  size is at most 64 MB, or all four file fields are zero, which marks a
  dissent (section 6a). Signatures on the same statement are merged, one per
  key.
- **The file**, uploaded in parts of at most 4 MB through the same functions
  (Vercel Blob multipart), only for a statement already stored that is not a
  dissent. When the last part is in, the server hashes the whole file and keeps
  it only if size and hash match the statement.
- **`GET /api/snapshots/pending`**: statements and dissents of the last seven
  days, dissents marked as such, and who has signed each, for confirmers.
- **`GET /api/snapshots/latest`**: the newest statement signed by two
  different operators, at least one signature from a key in `mirror_pins`
  (section 1), whose file is stored and checked. Height, block hash,
  statement hash, file and manifest addresses, sizes and hashes, and which
  operators signed. A newer statement confirmed by two operators without a
  pinned signature stays in `pending`, where the 3060's confirmer can still
  sign it, and is not served: every node would refuse it at section 7 step 1
  and fall back to 225,927, skipping the older snapshot it could have loaded.
  While a dispute stands it serves none (section 6a).
- **`GET /api/snapshots/disputes`**: open disputes, with both statements, who
  signed each, the fields that differ and the owner's alert (section 6a).
- **Storage stays small.** The newest five confirmed pairs, about 45 MB at
  today's 9 MB file, unchanged by the grid. A pending pair below the newest
  confirmed height can never become `latest`, so its file is deleted. Above
  it, a week without a second operator would leave about 67 grid heights at
  40 blocks an hour, about 600 MB per producer, so the site keeps files only
  for the newest ten pending statements, about 90 MB. Statements, dissents and
  dispute records are a few hundred bytes each.

### 6a. Any disagreement freezes Fast-forward

jpp's point 6: no majority. If listed operators disagree about a grid height,
Fast-forward stops for every node until the owner has looked.

**What counts as a dispute.** The site holds two statements for the same grid
height, each signed by at least one listed operator, that differ in any chain
field: block hash, `hash_serialized_3`, coin count or chain transaction count
(the height is equal by definition). A wrong chain id, replay context or
shielded commitment is refused at intake (section 6), so it never reaches this
comparison. So is any statement with a signature from a key not on the list:
an outsider's conflicting statement is never stored and disputes nothing.

**A difference in the file alone is not a dispute.** Two statements whose chain
fields agree but whose file size, file hash or chunk layout differ say the same
thing about the chain. Each file is checked against its own statement when it
is uploaded and again when a node loads it (section 7), and each statement is
confirmed on its own. If both are confirmed, `latest` serves the one confirmed
first.

**The dissent.** A confirmer whose diary disagrees (section 5, check 4) does not
sign. It sends a dissent: a statement built from its own diary entry (height,
block hash, `hash_serialized_3`, coin count, chain transaction count), with its
node's chain id and replay context and the compiled shielded commitment, and
all four file fields zero (file size 0, file hash zero, chunk size 0, chunk
count 0). It signs it with its node's `signutxosnapshotmanifest` and posts it
like any statement, once per height.

- **Only a listed operator can send one.** `signutxosnapshotmanifest` is the
  only way to sign with an operator's key, which lives in the node, and it
  signs any statement that carries the node's chain id and replay context
  (Context). The site checks a dissent's signature with the same code and
  against the same list as a confirmation, so an outsider cannot freeze
  Fast-forward. A listed operator can, on purpose: that is what no majority
  means, and the alert names who dissented.
- **It can never be loaded.** The engine refuses a zero file size and a null
  file hash outright (`trusted_utxo_snapshot_attestation.cpp:102-108`), the
  app's parser refuses a dissent (section 7), and the site never takes a file
  for one. However many operators sign a dissent, it never becomes a
  snapshot.
- **It disputes exactly when the chain facts differ.** The site compares chain
  fields only, and a dissent's chain fields are the confirmer's own diary.

**While a dispute stands:**

- `GET /api/snapshots/latest` serves no confirmed snapshot at all, at any
  height. It answers only the disputed heights: `{"disputed": [233800]}`.
- Setup uses the fallbacks in section 9.
- The app shows Fast-forward off, with one sentence naming the newest disputed
  height: "Fast-forward is off while the snapshot operators disagree about
  block 233,800."
- Nothing already loaded is undone. A node that started from an earlier
  snapshot keeps running as it was.
- Producers and confirmers carry on at newer heights, so the newest confirmed
  snapshot is ready the moment the dispute clears.
- The owner is alerted (below).

The app cannot check a dispute and does not need to: a false one only turns
Fast-forward off, which the site can already do by answering nothing
(Rollback).

**The alert.** The site writes it once, when a height first becomes disputed,
into the dispute record that `GET /api/snapshots/disputes` returns. Getting it
to the owner is Orca's part (the owner's alerting bot, another project's); this
design only writes the text. A dissent's line says "Dissent from" instead of
"Signed by".

```
easyNode snapshots: block <height> is disputed. Fast-forward is off for every node until you clear it.
1: block <hash>, UTXO hash <hash>, <coins> coins, <txs> transactions. Signed by <operators>.
2: block <hash>, UTXO hash <hash>, <coins> coins, <txs> transactions. Signed by <operators>.
They differ in: <fields>.
Check both nodes, then clear it: node scripts/clear-snapshot-dispute.mjs <height>
```

**How it clears.** Only the owner clears a dispute, since no count of operators
settles one. He runs `scripts/clear-snapshot-dispute.mjs <height>` in the
website repository with the site's storage write token
(`BLOB_READ_WRITE_TOKEN`), which only he and the site's own functions hold; no
endpoint clears a dispute. It moves every statement, dissent and file at that
height into an archive folder, deletes the dispute record, and closes the
height: the site refuses any statement at a closed height from then on, so a
copy of an old statement or dissent cannot bring the dispute back. The next
grid height, 100 blocks later, starts clean. If the operators still disagree
there, it is disputed again.

### 7. Loading: one path for setup, Fast-forward and every kind of node

1. Read `latest`. If it answers a dispute, stop and use section 9. Otherwise
   download the manifest and check it in the app before the engine sees
   anything: parse strictly (no trailing bytes); version 2; chain id and
   replay context as compiled for this engine and as the running node reports;
   the shielded commitment as compiled; file fields not zero (a dissent is
   never loaded); height on the grid and above the start point the node would
   otherwise use; every signature strict DER, low-S and valid over the
   statement; signers grouped by operator, **at least two different
   operators**; and at least one signature from a key the node pins.
2. Download the file, hashing as it streams. Size and hash must match the
   statement.
3. Just before loading, refuse every held root the node knows, as the fork
   check already does every 30 seconds. The engine then refuses a base above
   any of them.
4. Record in `<datadir>/snapshot-start.json` the height, the block hash and
   the operators whose signatures step 1 verified. Then write a trimmed
   manifest that keeps **every** signature from a key the node pins, in order,
   and drops the rest. The trimmed manifest loses the confirmers' signatures,
   so this record is where their names survive.
5. Load it with `loadtxoutsetattested`. The engine allows this only in mirror
   mode, so a node that validates is started once as a mirror for the load.
6. After the load, check that the block at each held root's height is not the
   root. If it ever were, stop the node and restore the chain data, as
   Fast-forward does (section 10, step 4).
7. A node that validates restarts as a validating node straight away, under
   section 8's pin rule. From then on it checks every new block itself.

**The background check keeps running** (jpp's point 7). The engine runs it
after every snapshot load by default, and nothing in this design stops it.

**Who confirmed the start point, on screen** (jpp's point 7). The app names only
the operators whose signatures it verified itself against its compiled list,
from `snapshot-start.json`; the list of signers `latest` returns is never shown.
Names are in the list's order: "Mende and jpp", "Mende, Aleksander and jpp".

- The Fast-forward prompt (the Tools decision fixes where it sits and how it
  asks): "Fast-forward to block 233,800, confirmed by Mende and jpp? Your node
  stops for a few minutes to load it, then carries on from there."
- The history-check line (UI decision, section 2) gains a second sentence:
  "Checking older history: 131,200 of 233,800 (56%). Started from block
  233,800, confirmed by Mende and jpp."
- From a fallback (section 9), the second sentence names the source instead:
  "Started from block 225,927, built into this app." or "Started from block
  228,000, built into the BTX engine."
- When the check is done the line goes, as the UI decision says. Copy
  diagnostics keeps the start point and who confirmed it.

### 8. A validating node on a signed snapshot

- While `<datadir>/chainstate_snapshot/attested_assumeutxo` exists, the
  validating arm also pins the mirrors' keys, beside its own. In consensus
  mode the engine treats them as telemetry: they never skip a check and never
  steer which chain it follows. Without them it does not start.
- The file stays when the background check finishes. It is still under
  `chainstate_snapshot/` at the node's next start, which re-checks the stored
  manifest, pins and all. On that start the engine moves the folder to
  `chainstate/`, and the file with it, where it no longer counts, so the pins
  go from the start after. Whether dropping the pins then costs the node
  anything is to be measured on a real data folder before it ships; if it
  does, they stay.
- It keeps signing blocks: a block signature vouches for the node's own proof
  check, which it does.
- It keeps no diary, produces nothing and confirms nothing while
  `getchainstates` shows an unvalidated chainstate (section 3).
- Honest words on screen: it checks every new block itself; its older history
  is checked in the background, with progress and who confirmed its start
  point shown (section 7 and the UI decision).

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
snapshot. While a dispute stands it is off and says why (section 6a).

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

1,000 blocks stays the threshold with the grid of 100: while producers and
confirmers keep up, a confirmed snapshot is 144 to about 250 blocks old, and a
node less than 1,000 behind it catches up faster through section 11 than
through a stop, a download and a load.

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

After a load from a confirmed snapshot the node is normally 144 to about 250
blocks behind, inside what limited peers serve, so the engine fetches them itself and
this help should stay idle. It matters for the fallbacks (section 9), which
start thousands of blocks back, and for any node that fell behind later. The
grid and depth change none of its thresholds.

### 12. Rules the spike wrote

- A test fails if a release removes a key from `BTX_TRUSTED_ATTESTATION_PUBKEYS`
  or raises the mirrors' threshold. The test compares against a checked-in list
  of every key ever shipped. A key is retired by blocklisting it
  (`-matmulattestationblocklist`), which the engine tolerates at restart.
- `scripts/check-engine-tag.sh` gains a regtest check before any engine bump:
  the replay context and the shielded commitment equal the compiled ones, and
  a node on a signed snapshot still restarts as a validating node.

### 13. Right now, before 0.7.0

Not code, and outward-facing, so the owner's decision: publish a fresh signed
snapshot from the machine whose keeper serves it, and schedule
`publish-attested-snapshot.sh` there. Mirrors on 0.6.32 would then start within
a day of the tip instead of at 225,927.

## Tests

- **Vectors shared by the app and the website**: the real 225,927 manifest and
  file (statement hash, file hash, the 3060's signature, and its shielded
  commitment against the compiled one), the regtest manifests from the spike
  with one, two and three signers, and a dissent.
- **The list**: `snapshot-operators.json` parses, every key is a valid
  compressed key, and no key sits under two operators. `mirror_pins` equals
  `BTX_TRUSTED_ATTESTATION_PUBKEYS`. The website's CI fails on one changed
  byte in its copy.
- **Parser**: truncated, trailing bytes, an oversized signature count, a
  version-1 statement, a dissent offered for loading.
- **Signatures**: valid; high-S; non-strict DER; the right signature under the
  wrong key; two keys of one operator counted once (Aleksander's three count
  as one); an unknown key ignored for the count and dropped from the trimmed
  file.
- **Trimming**: byte-identical to the producer's file when only the pinned
  signature is kept. The start record names only operators whose signatures
  were verified, in the list's order.
- **Diary comparison**: one sabotage case per field (block hash, UTXO hash,
  coin count, transaction count, chain id, replay context, shielded
  commitment, fewer than 144 blocks deep, a base off the node's chain, a diary
  entry whose block left the chain).
- **The `validated` refusal**: a recorded `getchainstates` answer with one
  snapshot chainstate at `"validated": false` records, produces and signs
  nothing, both for a plain assumeutxo snapshot, with no `attested_assumeutxo`
  file, and for a signed one. The same answer with every chainstate validated
  records again.
- **Sign or dissent**: a chain-field mismatch sends a dissent built from the
  diary, with zero file fields and no signature on the statement; a missing
  diary entry, a block less than 144 deep and a foreign replay context send
  neither.
- **Grid** of 100 (on- and off-grid heights; 219,000 and 228,000 on it),
  fallbacks, the pin rule of section 8, and pins only growing.
- **Catch-up help**: target and peer choice over recorded RPC answers.
- **Website**: the same vectors in vitest; merging; the choice of `latest`;
  `latest` skips a newer two-operator statement with no pinned signature and
  serves the older one that has one; refusals for an unknown key, a wrong
  chain, a wrong shielded commitment,
  off-grid heights, a closed height, oversize bodies, a file whose hash does
  not match, a file for a dissent. Disputes: two statements signed by listed
  operators that differ at one height leave `latest` serving nothing, at any
  height; a difference in the file fields only is not a dispute; a conflicting
  statement from an unlisted key is refused and changes nothing; a dissent
  disputes a statement whose chain fields differ; after the owner's clear, the
  height is closed and a resubmitted copy of either statement is refused.
- **`btx-confirmer` against a plain btxd on regtest**: it keeps its diary,
  signs a matching statement through the node's RPC, dissents on a sabotaged
  one, refuses to start on a foreign replay context, and signs nothing while
  the node's `getchainstates` shows an unvalidated snapshot chainstate (after
  a plain `loadtxoutset`).
- **Rehearsal on regtest with test keys** (step 4 of the owner's plan): two
  confirmers, one producer and one mirror, plus one validating node for the
  restart. Proven refused: a statement signed by one operator, a statement on a
  held branch, a diary mismatch, a wrong file hash, and a disputed height.
  Proven working: the validating restart on a signed snapshot, Fast-forward's
  roll-back after a failure, and the owner's clear.
- **Real machines**: a validating node on a card the engine qualifies, loaded
  from a signed snapshot, checking new blocks. Not possible on the owner's
  Mac.

## What this does not do

- It does not let a validating node skip checks. Every block after the snapshot
  is checked by the node; history before it is checked in the background.
- It does not stop or shorten the background check.
- It does not make confirmers authorities over blocks. The mirrors' pins stay
  as they are.
- It does not settle a disagreement by majority. Any dispute freezes
  Fast-forward until the owner clears it.
- It does not trust easybtx.com, GitHub or the node check-ins.
- It does not change the engine.

## Rollback

- Website: `latest` returns nothing, and every node falls back (section 9). A
  dispute does the same on its own.
- App: the confirmed path, the validating restart and the catch-up help each
  sit behind one constant. The diary is harmless on its own. As built, a
  validating node's one mirror launch is switched off by `MIRROR_LOAD_ENABLED`
  in the app's start path, the confirmed path by the website's pointer
  (`latest` answering nothing), and both on one machine by
  `EASYBTX_NODE_TRUSTED_MIRROR=0`.
- `btx-confirmer` is its own unit; stopping it stops that operator's
  confirmations and nothing else.

## Choices for the owner

1. The grid: **approved**, at 100 instead of 200 after jpp's review
   (section 2).
2. Validating nodes start from the confirmed snapshot as section 8 describes,
   with that trust stated plainly on screen: **approved**.
3. The pinned pair as a fallback for validating nodes as well (section 9):
   **approved**.
4. Fast-forward from 1,000 blocks behind: **approved**.
5. Who is on the list: **answered** (section 1). Mende, Aleksander with three
   keys (the owner confirms he agreed before the list change merges) and jpp.
   An operator counts only through a node whose `getchainstates` shows no
   unvalidated chainstate. Aleksander's nodes started at 219,000 and jpp's box
   is on 228,000; whether their background checks have finished was not
   measured, so each operator's `getchainstates` is read once before 0.7.0
   ships to know who can confirm on the first day. Until two can, nothing
   counts as confirmed and every node uses section 9.
6. Section 13: publish a fresh single-signed snapshot from the 3060 before
   0.7.0? **Still open**: it needs the owner's go.

## Upstream issues to draft for the owner

Exempting the background check from the cadence hold; separate snapshot keys
and threshold; co-signing that checks the signer's own chain; a snapshot hash
at a past height. Two new ones from this work: an official way for a
validating node to start from a signed snapshot, and the block downloader
asking limited peers for older blocks sooner than its 120-second rescue.
