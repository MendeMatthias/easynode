# Snapshot Network, website side: the meeting point on easybtx.com (Implementation Plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Amended 2026-09-29 (night)**, after jpp's review and the owner's decisions of that night (the amended design, easynode `12e44c3`). What changed: the operator list is the full 0.7.0 list (Mende; Aleksander with three keys; jpp) plus `mirror_pins`, kept as `site/src/lib/snapshot-operators.json`, byte for byte easyNode's file, which CI fetches at one pinned easynode commit; the grid is 100 on mainnet too; intake also checks the shielded commitment and closed heights and takes dissents (all four file fields zero), never with a file; `latest` needs a pinned signature beside two operators and answers `{"disputed": [...]}` while listed operators disagree about any height; a new `GET /api/snapshots/disputes` with the owner's alert text; storage keeps five confirmed pairs and files for the ten newest pending statements; the owner clears a dispute with `scripts/clear-snapshot-dispute.mjs <height>`, which closes the height; the final task stops before anything leaves this machine. Anchors are rebased onto EasyBTX `origin/main` `3dcfa37a` (PR #548, 29 Sep). The code below was written against that commit; vitest was not run, test counts are derived by counting cases, and the rules and handlers passed 145 checks in memory with the curve stubbed (see Self-review).

**Goal:** easybtx.com takes signed snapshot statements, their files and dissents from producers and confirmers, points every easyNode at the newest statement two different operators signed and a key every node pins also signed, and points at nothing while listed operators disagree about any height, trusting nothing and storing little.

**Architecture:** Five Astro API routes (`site/src/pages/api/snapshots/{statement,file,pending,latest,disputes}.ts`) are one-line wrappers around handlers in `site/src/lib/snapshotRoutes.mjs`, which read and write a store (`snapshotStore.mjs`: Vercel Blob in production, a folder in tests and in the local stand-in) around pure rules (`snapshotRendezvous.mjs`: what is accepted, how signatures merge, what `latest` names, what counts as a dispute, the owner's alert, what storage keeps). `snapshotManifest.mjs` reads the engine's manifest exactly as easyNode's `confirmed_snapshot.rs` does and verifies strict-DER low-S secp256k1 signatures with `@noble/curves`; `snapshotOperators.mjs` reads the checked-in copy of easyNode's operator file (`site/src/lib/snapshot-operators.json`) and exposes its `operators` and `mirror_pins`, and `scripts/check-snapshot-operators.mjs` fails CI when that copy differs by one byte from easyNode's at the pinned commit. `scripts/clear-snapshot-dispute.mjs` is the owner's only way to clear a dispute. `site/scripts/snapshot-rendezvous-local.mjs` serves the same handlers over `node:http` for easyNode's end-to-end rehearsal.

**Tech Stack:** Astro 5 (5.18.2 locked) on Vercel (Node-runtime functions, `export const prerender = false`), `@vercel/blob` 2.4.0 (`get`, `put` with `ifMatch`, `list`, `del`), `@noble/curves` 2.2.0 (secp256k1), `node:crypto` (SHA-256), vitest 4.1.7.

**This is the website half of the snapshot network.** The app half is `2026-09-29-snapshot-network-app.md` (same folder), built on `2026-09-29-confirmed-snapshots-core.md`, whose Task 1 creates `crates/btx-core/snapshot-operators.json` on easynode's `main`; this plan's Task 2 pins a commit of easynode's `main` that carries that file, so that app task lands first. It names Aleksander, so it merges only once the owner confirms Aleksander agreed, and this plan's PR merges after it. The app's end-to-end rehearsal (app plan, Task 9) runs this plan's `site/scripts/snapshot-rendezvous-local.mjs`, so this plan's Tasks 1 to 6 come before it; the rehearsal must now also pass `SNAPSHOT_REGTEST_PINS` (the keys its mirror pins), or nothing on regtest is ever confirmed.

## Global Constraints

- Design: `docs/decisions/2026-09-29-every-node-starts-near-the-tip.md` as amended the night of 2026-09-29, on easynode's local branch `claude/cosigned-snapshots` at `12e44c3` (read it with `git -C /Users/m2promende/repos/easynode show 12e44c3:docs/decisions/2026-09-29-every-node-starts-near-the-tip.md`; `origin/claude/cosigned-snapshots` still holds the first draft, `989f30f`, until that branch is pushed). Implemented here: sections 6 and 6a, and the website half of section 1 (the copy of the list and its check). "Nothing here is trusted by the app. It filters, stores and points."
- Repository: `MendeMatthias/EasyBTX` (private). Work in a new worktree, never in `/Users/m2promende/repos/EasyBTX` itself (the owner's working tree; other sessions may be working in checkouts of it): `git -C /Users/m2promende/repos/EasyBTX fetch -q origin main && git -C /Users/m2promende/repos/EasyBTX worktree add ../EasyBTX-snapshot-rendezvous -b claude/snapshot-rendezvous origin/main` (base `3dcfa37a` or later; `3dcfa37a` is PR #548, which touched only `scripts/check-node-links.py` and `.github/workflows/site-links.yml` since the first version of this plan was measured on `e187d14c`). All paths below are relative to that worktree.
- The pointer contract is easyNode's `attested_snapshot::ConfirmedPointer` (confirmed-snapshots core plan, Task 3; fixture `crates/btx-core/tests/fixtures/confirmed_snapshot/latest.json`): `GET /api/snapshots/latest` answers HTTP 200, `content-type: application/json`, the object `{version, chain, height, block_hash, statement_hash, manifest_url, manifest_size, manifest_sha256, file_url, file_size, file_sha256, file_hash, operators, confirmed_at}` in that key order; **while any dispute on that chain stands, HTTP 200 with exactly `{"disputed": [<height>, ...]}` (ascending) and nothing else**; HTTP 404 with `{"version":1,"confirmed":null}` when nothing is confirmed; `cache-control: public, max-age=60, s-maxage=60` on all three. `manifest_url` and `file_url` are `https://<store>.public.blob.vercel-storage.com/...` in production. `block_hash`, `statement_hash` and `file_hash` are display hex (byte-reversed, as btxd prints them); `manifest_sha256` and `file_sha256` are plain SHA-256 of the bytes.
- The routes, exactly:
  - `POST /api/snapshots/statement`, body the raw manifest bytes (at most 65,536), header `x-ebtx-node: ebtx-snapshot-v1`. A statement, co-signatures for one already here, or a dissent. 200 `{statement_hash, chain, height, signers, operators, added, file}` (`file` is `missing`, `uploading`, `stored` or `dropped`; a dissent's is always `missing`); 400 empty; 403 no header; 405; 413 too big; 422 `{error}`, and at a height the owner cleared exactly `{"error":"closed-height"}`; 503 no store.
  - `POST /api/snapshots/file?statement=<hash>&action=start` → 200 `{upload, part_bytes, parts}`; 404 unknown statement; 409 file already stored, or let go to keep storage small; 422 `{"error":"a dissent has no file"}` or no listed signer left.
  - `PUT /api/snapshots/file?statement=<hash>&upload=<id>&part=<n>`, body exactly the bytes of part `n` (parts of 4,194,304 bytes, the last one the rest) → 200 `{part, bytes}`; 404 no such upload; 422 wrong size or number; 413 over 4 MiB.
  - `POST /api/snapshots/file?statement=<hash>&upload=<id>&action=complete` → 200 `{stored: true, file_url, file_sha256, confirmed}`; 422 `{"error":"the file does not match the statement"}` (parts deleted, nothing kept) or `part <n> is missing`.
  - `GET /api/snapshots/pending?chain=main|regtest` (default `main`) → 200 `{version: 1, chain, statements: [{statement_hash, height, block_hash, manifest_hex, signers, operators, file, first_seen, confirmed, disputed, dissent}]}`, statements and dissents first seen in the last 7 days, highest first; `cache-control: public, max-age=30, s-maxage=30`; 400 other chains.
  - `GET /api/snapshots/latest?chain=main|regtest` (default `main`), the contract above.
  - `GET /api/snapshots/disputes?chain=main|regtest` (default `main`) → 200 `{"disputes": [{height, statements: [{statement_hash, block_hash, hash_serialized_3, coins, chain_tx, file_size, file_hash, signers: [names], dissent}], differ: [field names], alert, opened_at}]}`, ascending by height; `cache-control: public, max-age=30, s-maxage=30`; 400 other chains; 405 anything but GET. No route clears a dispute.
- Accepted statement (all of section 6): parses strictly (version 2, canonical lengths, 33-byte keys, signatures of at most 72 bytes, no trailing bytes); chain id is mainnet's genesis `75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601` or, only where a test list is set, regtest's `521ad0951ed299e9c56aeb7db8188972772067560351b8e55adf71dbed532360`; replay context mainnet `32ad5c2e148149752a312561dc0b6879c9cc41fdf4bc09edcdd5e2bd09af7188`, regtest `9ed2add89d64a66015d6c4b2a746115c00c78503088fdde49e15ddcafed8a577`; shielded commitment on mainnet exactly `94343b766b39c0ea2d92d83323f77b5ccc5e775d99b34b01f5fa6400f2354541` (read from `git -C /Users/m2promende/repos/btx show 84b998b4:src/kernel/chainparams.cpp`, line 1278, the 219,000 entry; the 225,927 vector carries the same value), on regtest not zero (regtest's shielded pool is open, so its commitment moves with the chain); either a file of 1 to 67,108,864 bytes with chunk size 65,536 to 4,194,304 and `chunk_count = 1 + (size - 1) / chunk`, or all four file fields zero (file size 0, file hash 32 zero bytes, chunk size 0, chunk count 0), which makes it a **dissent**; height a positive multiple of **100** on both chains, and not a closed height; at least one signature; no key twice; **every** signature strict DER, low S, valid over the statement hash, from a key on that chain's list. Statement hash = double SHA-256 of `0x28 || "BTX_TRUSTED_UTXO_SNAPSHOT_ATTESTATION_V2" || statement`. **The website checks no depth.** Sending and signing only at 144 blocks deep is the producers' and confirmers' rule, on their own nodes; the website sees no chain.
- Confirmed = not a dissent, signed by at least **2 different operators under the list as it is at the moment of reading** (all of one operator's keys count once), at least one signature from a key in that chain's `mirror_pins`, and the file stored after its size and double SHA-256 matched the statement. `latest` names the highest confirmed height of the chain, and at one height with more than one confirmed statement (their files differ, their chain facts do not) the one confirmed first. A two-operator statement without a pinned signature is never served: every node would refuse it (design, section 7, step 1) and skip the older one it could load.
- Dispute (section 6a): the site holds two statements for the same grid height of one chain, each with at least one listed operator's signature, that differ in any of block hash, `hash_serialized_3`, coin count, chain transaction count. File fields alone never dispute. It is worked out whenever a statement or dissent is stored, and written once, with the owner's alert, as a dispute record; it stands until the owner clears it. `latest` also applies the rule to what it reads, so a dispute record that failed to write still turns it off.
- The owner's alert, written once into the dispute record when a height first becomes disputed (section 6a's text, filled in; a dissent's line says "Dissent from" instead of "Signed by"; names joined "Mende and jpp", "Mende, Aleksander and jpp"; heights and counts with thousands commas in prose, the plain number in the command; on regtest the command gains ` --chain regtest`):

  ````text
  easyNode snapshots: block 233,800 is disputed. Fast-forward is off for every node until you clear it.
  1: block <hash>, UTXO hash <hash>, 140,731 coins, 328,195 transactions. Signed by Mende and jpp.
  2: block <hash>, UTXO hash <hash>, 140,731 coins, 328,195 transactions. Dissent from Aleksander.
  They differ in: block hash.
  Check both nodes, then clear it: node scripts/clear-snapshot-dispute.mjs 233800
  ````

- The operator list is `site/src/lib/snapshot-operators.json`, byte for byte easyNode's `crates/btx-core/snapshot-operators.json` (928 bytes, SHA-256 `f91529f31dde7311d7970088187b3d39838cc8722b97a8bd3dda894e519fdf3d`, computed from the content in Task 2, Step 3): schema 1; operators Mende (`02d5efca…4675`, the 3060), Aleksander (`026da4e3…bbf1`, `03047189…8a24`, `02c9cfb7…72a0`, one operator), jpp (`02e0a965…cadb5`); `mirror_pins` the four keys every node pins (`BTX_TRUSTED_ATTESTATION_PUBKEYS`, which an app test keeps equal). Aleksander's entry is in it, so the PR that adds it must not merge until the owner confirms Aleksander agreed. `scripts/check-snapshot-operators.mjs` compares the copy with easyNode's file at one pinned commit of easynode's `main` (Task 2 says why a pin, not `main`), fetched from `https://raw.githubusercontent.com/MendeMatthias/easynode/<commit>/crates/btx-core/snapshot-operators.json` (easynode is the public repository, `scripts/check-public-sync.sh`). The regtest list comes from `SNAPSHOT_REGTEST_OPERATORS` (easyNode's `name=key[,key];name=key` format) and the regtest pins from `SNAPSHOT_REGTEST_PINS` (`key,key`); both are ignored when `VERCEL_ENV=production`.
- Storage (Vercel Blob, the project's existing `BLOB_READ_WRITE_TOKEN`): `snapshots/records/<statement_hash>.json` private, rewritten with `ifMatch` (up to 4 tries); `snapshots/uploads/<statement_hash>/<upload>/part-<nnnn>` private, deleted after `complete`; `snapshots/<height>/<statement_hash>.dat` and `snapshots/<height>/<statement_hash>-<signature count>.manifest` public, `cacheControlMaxAge` one year (content-addressed, never rewritten with other bytes); `snapshots/disputes/<chain>-<height>.json`, `snapshots/closed/<chain>-<height>.json` and `snapshots/archive/<chain>-<height>-<time>/...` private. Kept, per chain, after every write: the **5** newest confirmed pairs; files only for the **10** newest pending statements; a pending pair at or below the newest confirmed height loses its file (state `dropped`); every record first seen in the last 7 days; everything at a disputed height, until the owner clears it; dispute and closed records always. The rest is deleted.
- The owner's clear: `node scripts/clear-snapshot-dispute.mjs <height> [--chain main|regtest]` in this repository, run by the owner with `BLOB_READ_WRITE_TOKEN` (or `SNAPSHOT_STORE_DIR` for a local stand-in's folder). It copies every statement, dissent, file, manifest and the dispute record at that height into `snapshots/archive/`, closes the height, then deletes the originals. No HTTP endpoint clears a dispute.
- Why parts of the website's own: a Vercel function takes a request body of about 4.5 MB, and Blob's multipart API needs parts of at least 5 MB except the last (`uploadPart` docs in `@vercel/blob` 2.4.0), so a part that fits through a function cannot be a Blob multipart part. Parts are kept as private blobs and joined by the `complete` step, which writes the file with `put(..., { multipart: true })` above 8 MiB.
- Dependencies: add `@noble/curves` `~2.2.0` as a direct dependency (it is locked at 2.2.0 already, through `@noble/post-quantum` 0.6.1: `site/bun.lock` line 229, `site/package-lock.json` line 1792 on `3dcfa37a`); SHA-256 comes from `node:crypto`. No other new package.
- CI in this repository: `cd site && npx vitest run` (`.github/workflows/site-tests.yml`, which this plan also points at `scripts/clear-snapshot-dispute.mjs`); `bash scripts/check-download-links.sh`, which runs `python3 scripts/check-node-links.py` at its line 122, plus, since #548, a live check that `https://easybtx.com/updater/node-<type>.json` answers 404 (`site-links.yml`; these routes touch no `/updater` path and no `vercel.json` rule); and the new `node scripts/check-snapshot-operators.mjs` (`snapshot-operators.yml`). Also run `cd site && npx astro build` before the PR.
- Commits end with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (use `git commit -m "<subject>" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"`).
- Words anyone reads (errors returned to a node, the alert, the clear's output, comments that explain): friendly, simple, no hype, no guarantees, no em-dashes.
- Shared test vectors, the same bytes easyNode's tests read (`crates/btx-core/tests/fixtures/confirmed_snapshot/` in easynode). Source: `S=/private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad` (present and matching the sums below on 2026-09-29, night); if that folder is gone, copy them from easyNode's fixture folder once the app plan has put them there (`git -C /Users/m2promende/repos/easynode show <branch>:crates/btx-core/tests/fixtures/confirmed_snapshot/<name> > <name>`). SHA-256, exactly:

  ````text
  8adc90c2b4514334d0bc0e1dafa5f3bc85ed0cfcc55d051a586e117794e332ed  mainnet-225927.manifest
  b2c5c43c4fd931475c4769926564644b6cff744a229c54623f5eb03c16e4ce85  regtest-100.dat
  691e427d899ed18c0701a20422e908d35a4e29fdf9fa17f8eb484f256525cb6f  regtest-C.manifest
  3ce5d526dfe971d736b3d9ce3efc27ad7111c60e3ee42e9952bc61e1b89ac8fd  regtest-CD.manifest
  2a77b16882ea2ee988882e55b19cb20305f3202dd85ef10157bade072387fbc2  regtest-CP.manifest
  eac1e450d1aa22cdadb8472f7b1dec80d6af62fbb04387b6d49e726abdcc2d4a  regtest-P.manifest
  e8fa06d2f700feb488d7f494a532cd0e44799d20613b80532b2eca5cffaa746e  regtest-PC.manifest
  649ce13ffb22f57358253530d187a6d3dc3679d4bc5649acb77dc6a6d22e5b09  regtest-PCD.manifest
  ````

  The mainnet manifest: height 225,927, statement hash `d3ee93122fb062baa00bfe5d8c586f03c619427e8a9253f8fae0c980c9aa0482`, file hash `f234192dba8bb29875778620259fc870fa1e245175c61c89ed82cd0c1feceb2c`, shielded commitment `94343b76…4541` (the compiled one), one signature by the 3060 (`02d5efca…4675`), valid. The regtest ones: height 100, block `bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46`, UTXO hash `e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527`, 101 coins, 101 transactions, statement hash `11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194`, file 8,055 bytes with file hash `76cc3cebd9fc21d6107b11e196dffe3cf2672924ef82e5162882d9395f02058e`, signed by the spike's throwaway keys P `0343faeb…e464`, C `02c05d68…6c0f`, D `034694ab…6d0e` in the orders the file names say.

  **The dissent vector** (design, Tests): the 225,927 statement with its four file fields zeroed (bytes 181 to 228: file size, file hash, chunk size, chunk count), everything else unchanged. Its statement hash is `44ba673dbeb964d0e9e89d21597ccc5e590a216001f1fe69afbd863c6db0676b`; moved to height 226,000 (on the grid) it is `a4dae6bf1eb49595c133252661b9e3a28dd5a2507ddf5d6aad03e31329cf1fab` (both computed independently in Python from the 225,927 bytes, not by this plan's code). The network-app and core plans generate its bytes for easyNode's fixture folder. This plan does not copy them: its tests build the statement from the 225,927 vector and sign it with a throwaway test key made in the test, never a real operator's key.

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `site/package.json`, `site/package-lock.json`, `site/bun.lock` | modify | `@noble/curves` `~2.2.0` as a direct dependency (one line each) |
| `site/src/lib/snapshotManifest.mjs` | create | Read and write the engine's manifest, statement fields, statement hash, dissents, strict DER, low-S verification, file hashes |
| `site/src/lib/snapshot-operators.json` | create | Byte for byte easyNode's `crates/btx-core/snapshot-operators.json`: operators and `mirror_pins` |
| `site/src/lib/snapshotOperators.mjs` | create | Read the operator file (operators, `mirror_pins`), the regtest list and pins from the environment, key to operator, distinct operators |
| `scripts/check-snapshot-operators.mjs`, `.github/workflows/snapshot-operators.yml` | create | Fail CI when the copy differs from easyNode's file at the pinned commit; warn daily when easynode's `main` has moved on |
| `site/src/lib/snapshotRendezvous.mjs` | create | Pure rules: accept, merge, records, confirmed, `latest`, `pending`, disputes and the alert, storage, parts |
| `site/src/lib/snapshotStore.mjs` | create | The store: Vercel Blob or a folder, one shape |
| `site/src/lib/snapshotRoutes.mjs` | create | The five handlers from `Request` to `Response`, dispute records, closed heights, and `clearHeight` for the owner's script |
| `site/src/pages/api/snapshots/statement.ts`, `file.ts`, `pending.ts`, `latest.ts`, `disputes.ts` | create | Astro routes, one line each around the handlers |
| `site/scripts/snapshot-rendezvous-local.mjs` | create | Local stand-in serving the same handlers over `node:http`, for easyNode's rehearsal |
| `scripts/clear-snapshot-dispute.mjs` | create | The owner's clear: archive a disputed height, close it |
| `.github/workflows/site-tests.yml` | modify | Also run on changes to `scripts/clear-snapshot-dispute.mjs` (two lines) |
| `site/tests/fixtures/snapshots/*` | create | The shared vectors (8 files) |
| `site/tests/unit/snapshotManifest.test.ts`, `snapshotOperators.test.ts`, `snapshotRendezvous.test.ts`, `snapshotStore.test.ts`, `snapshotRoutes.test.ts`, `snapshotClearScript.test.ts` | create | vitest |

## Tasks

### Task 1: Read and verify a manifest (`snapshotManifest.mjs`), with the shared vectors

**Files:**
- Modify: `site/package.json`, `site/package-lock.json`, `site/bun.lock` (one line each)
- Create: `site/tests/fixtures/snapshots/` (8 files)
- Create: `site/src/lib/snapshotManifest.mjs`
- Test: `site/tests/unit/snapshotManifest.test.ts`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces (Tasks 2 to 6): `STATEMENT_VERSION`, `STATEMENT_LEN` (229), `FILE_FIELDS_AT` (181), `MAX_MANIFEST_BYTES` (65,536), `MAX_SIGNATURES` (64), `MAX_DER_LEN` (72), `MAX_FILE_BYTES` (67,108,864), `MIN_CHUNK`, `MAX_CHUNK`; `class ManifestError`; `displayHex(bytes) -> string`; `sha256(bytes)`, `sha256d(bytes)`; `statementFields(raw229) -> {version, chainId, blockHash, height, hashSerialized, coins, chainTx, shielded, replayContext, fileSize, fileHash, chunkSize, chunkCount}` (hashes in display hex); `isDissent(fields) -> boolean`; `statementDigest(raw229) -> Uint8Array(32)` (display it with `displayHex` for the statement hash); `parseManifest(bytes) -> {statement: Uint8Array, signatures: [{key: hex66, der: Uint8Array}]}` (throws `ManifestError`); `serializeManifest(m) -> Uint8Array`; `isStrictDer(der) -> boolean`; `isCompressedKey(hex) -> boolean`; `signatureIsValid(digest, keyHex, der) -> boolean`; `fileHashes(chunks) -> {size, sha256, fileHash}`.

- [ ] **Step 1: Create the worktree and add the dependency**

````bash
git -C /Users/m2promende/repos/EasyBTX fetch -q origin main
git -C /Users/m2promende/repos/EasyBTX worktree add ../EasyBTX-snapshot-rendezvous -b claude/snapshot-rendezvous origin/main
cd /Users/m2promende/repos/EasyBTX-snapshot-rendezvous/site
npm install --package-lock-only --no-audit --no-fund @noble/curves@~2.2.0
bun install --lockfile-only
git diff --stat
npm ci --no-audit --no-fund
````

Expected: `npm install` prints `up to date`; `bun install` prints `Saved bun.lock`; `git diff --stat` shows exactly `site/bun.lock | 1 +`, `site/package-lock.json | 1 +`, `site/package.json | 1 +`, each the line `"@noble/curves": "~2.2.0",` right before `"@noble/hashes"` (on `3dcfa37a`, `"@noble/hashes": "^2.3.0",` is `site/package.json` line 27, right after `"@fontsource/inter": "^5.2.8",`; `site/bun.lock` line 14, in `workspaces[""].dependencies`; `site/package-lock.json` line 18, in `packages[""].dependencies`). If anything else moved, `git checkout -- package.json package-lock.json bun.lock` and stop. If `bun` is not installed, add that same line by hand to `workspaces[""].dependencies` in `site/bun.lock`, right before `"@noble/hashes": "^2.3.0",`.

- [ ] **Step 2: Add the shared vectors**

````bash
S=/private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad
F=tests/fixtures/snapshots
mkdir -p $F
cp $S/mainnet/pair/snapshot-manifest-225927.json $F/mainnet-225927.manifest
for m in P PC PCD CD C CP; do cp $S/spike/m/$m.manifest $F/regtest-$m.manifest; done
cp $S/spike/m/snap.dat $F/regtest-100.dat
(cd $F && shasum -a 256 *)
````

Expected: the eight sums in Global Constraints, exactly (checked against those files on 2026-09-29, night). (The manifest is binary despite the `.json` name the engine's examples use.) No dissent file is copied: Task 1's test builds the dissent vector from the 225,927 statement.

- [ ] **Step 3: Write the failing test**

Create `site/tests/unit/snapshotManifest.test.ts`:

````ts
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { secp256k1 } from '@noble/curves/secp256k1.js';
import {
  parseManifest, serializeManifest, statementFields, statementDigest, displayHex, isDissent,
  isStrictDer, isCompressedKey, signatureIsValid, fileHashes, ManifestError, FILE_FIELDS_AT, STATEMENT_LEN,
} from '../../src/lib/snapshotManifest.mjs';

// The shared vectors: the same files easyNode's confirmed_snapshot.rs tests
// read (crates/btx-core/tests/fixtures/confirmed_snapshot/), byte for byte.
const fx = (n: string) => new Uint8Array(readFileSync(new URL(`../fixtures/snapshots/${n}`, import.meta.url)));
const MAINNET = fx('mainnet-225927.manifest');
const R = (n: string) => fx(`regtest-${n}.manifest`);
const R_DAT = fx('regtest-100.dat');

const MENDE = '02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675';
const P = '0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464';
const C = '02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f';
const REGTEST_STATEMENT = '11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194';

describe('the published 225,927 manifest', () => {
  it('reads as the independent Python check read it', () => {
    const m = parseManifest(MAINNET);
    const f = statementFields(m.statement);
    expect(f.version).toBe(2);
    expect(f.chainId).toBe('75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601');
    expect(f.blockHash).toBe('06780445dae193010e099e6425c5430f121416b067b8d68a8a5c3b52e8a4b932');
    expect(f.height).toBe(225_927);
    expect(f.hashSerialized).toBe('79435348a3ff8bc8c07bd58603d18439fe29f0b629f9d38695d0c824be9439a8');
    expect([f.coins, f.chainTx]).toEqual([140_731, 328_195]);
    expect(f.shielded).toBe('94343b766b39c0ea2d92d83323f77b5ccc5e775d99b34b01f5fa6400f2354541');
    expect(f.replayContext).toBe('32ad5c2e148149752a312561dc0b6879c9cc41fdf4bc09edcdd5e2bd09af7188');
    expect([f.fileSize, f.chunkSize, f.chunkCount]).toEqual([9_045_522, 1_048_576, 9]);
    expect(f.fileHash).toBe('f234192dba8bb29875778620259fc870fa1e245175c61c89ed82cd0c1feceb2c');
    expect(displayHex(statementDigest(m.statement))).toBe('d3ee93122fb062baa00bfe5d8c586f03c619427e8a9253f8fae0c980c9aa0482');
    expect(m.signatures.map((s) => s.key)).toEqual([MENDE]);
    expect(signatureIsValid(statementDigest(m.statement), MENDE, m.signatures[0].der)).toBe(true);
    expect(Buffer.from(serializeManifest(m)).equals(Buffer.from(MAINNET))).toBe(true);
  });
});

describe('the regtest vectors', () => {
  it('verify, reserialize byte for byte, and the file matches', () => {
    for (const n of ['P', 'PC', 'PCD', 'CD', 'C', 'CP']) {
      const bytes = R(n);
      const m = parseManifest(bytes);
      const d = statementDigest(m.statement);
      expect(displayHex(d), n).toBe(REGTEST_STATEMENT);
      for (const s of m.signatures) expect(signatureIsValid(d, s.key, s.der), `${n} ${s.key}`).toBe(true);
      expect(Buffer.from(serializeManifest(m)).equals(Buffer.from(bytes)), n).toBe(true);
    }
    const f = statementFields(parseManifest(R('P')).statement);
    const h = fileHashes([R_DAT.subarray(0, 1000), R_DAT.subarray(1000)]);
    expect(h.sha256).toBe('b2c5c43c4fd931475c4769926564644b6cff744a229c54623f5eb03c16e4ce85');
    expect([h.size, h.fileHash]).toEqual([f.fileSize, f.fileHash]);
    const changed = R_DAT.slice();
    changed[100] ^= 1;
    expect(fileHashes([changed]).fileHash).not.toBe(f.fileHash);
    expect(fileHashes([R_DAT.subarray(0, R_DAT.length - 1)]).size).not.toBe(f.fileSize);
  });
});

describe('the dissent vector', () => {
  // Built here, not copied: the 225,927 statement with its four file fields
  // zeroed (file size, file hash, chunk size, chunk count). easyNode's plans
  // generate the same statement for the app's tests. Any throwaway test key
  // signs it, never a real operator's key.
  const dissent = parseManifest(MAINNET).statement.slice();
  dissent.fill(0, FILE_FIELDS_AT, STATEMENT_LEN);
  it('keeps the 225,927 chain facts, zeroes every file field, and has its own statement hash', () => {
    const f = statementFields(dissent);
    const o = statementFields(parseManifest(MAINNET).statement);
    expect([f.chainId, f.blockHash, f.height, f.hashSerialized, f.coins, f.chainTx, f.shielded, f.replayContext])
      .toEqual([o.chainId, o.blockHash, o.height, o.hashSerialized, o.coins, o.chainTx, o.shielded, o.replayContext]);
    expect([f.fileSize, f.fileHash, f.chunkSize, f.chunkCount]).toEqual([0, '00'.repeat(32), 0, 0]);
    expect(isDissent(f)).toBe(true);
    expect(isDissent(o)).toBe(false);
    expect(displayHex(statementDigest(dissent))).toBe('44ba673dbeb964d0e9e89d21597ccc5e590a216001f1fe69afbd863c6db0676b');
    const sk = secp256k1.utils.randomSecretKey();
    const key = Buffer.from(secp256k1.getPublicKey(sk, true)).toString('hex');
    const der = secp256k1.sign(statementDigest(dissent), sk, { prehash: false, format: 'der', lowS: true });
    const back = parseManifest(serializeManifest({ statement: dissent, signatures: [{ key, der }] }));
    expect(signatureIsValid(statementDigest(back.statement), key, back.signatures[0].der)).toBe(true);
  });
  it('is a dissent only when all four file fields are zero', () => {
    for (const at of [181, 189, 221, 225]) {
      const partly = dissent.slice();
      partly[at] = 1;
      expect(isDissent(statementFields(partly)), `byte ${at}`).toBe(false);
    }
  });
});

describe('the parser is strict', () => {
  const PC = R('PC');
  it('refuses a truncated manifest, trailing bytes, a big count, version 1, a short key', () => {
    expect(() => parseManifest(PC.subarray(0, PC.length - 1))).toThrow(ManifestError);
    expect(() => parseManifest(Uint8Array.from([...PC, 0]))).toThrow('1 bytes after the last signature');
    const many = PC.slice(0, 230);
    many[229] = 65;
    expect(() => parseManifest(many)).toThrow('65 signatures is not a manifest');
    const v1 = PC.slice();
    v1[0] = 1;
    expect(() => parseManifest(v1)).toThrow('statement version 1, not 2');
    const shortKey = PC.slice();
    shortKey[230] = 32;
    expect(() => parseManifest(shortKey)).toThrow('a signer key is not 33 bytes');
    expect(() => parseManifest(new Uint8Array(65 * 1024 + 1))).toThrow('more than a manifest may be');
  });
  it('refuses a length written the long way', () => {
    // 0xfd 0x02 0x00 means 2, which has to be written as one byte.
    const bytes = Uint8Array.from([...PC.subarray(0, 229), 0xfd, 0x02, 0x00, ...PC.subarray(230)]);
    expect(() => parseManifest(bytes)).toThrow('a length is not written the one allowed way');
  });
});

describe('signatures', () => {
  const m = parseManifest(MAINNET);
  const d = statementDigest(m.statement);
  const der = m.signatures[0].der;
  it('refuses a high-S twin of a valid signature', () => {
    const sig = secp256k1.Signature.fromBytes(der, 'der');
    const high = new secp256k1.Signature(sig.r, secp256k1.Point.Fn.ORDER - sig.s).toBytes('der');
    expect(isStrictDer(high)).toBe(true);
    expect(signatureIsValid(d, MENDE, high)).toBe(false);
  });
  it('refuses DER that is not strict', () => {
    // An extra zero in front of R, and a length byte that lies.
    const lenR = der[3];
    const padded = Uint8Array.from([0x30, der[1] + 1, 0x02, lenR + 1, 0x00, ...der.subarray(4)]);
    expect(isStrictDer(padded)).toBe(false);
    expect(signatureIsValid(d, MENDE, padded)).toBe(false);
    const lying = der.slice();
    lying[1] += 1;
    expect(isStrictDer(lying)).toBe(false);
  });
  it('refuses the right signature under the wrong key, and a key off the curve', () => {
    expect(signatureIsValid(d, C, der)).toBe(false);
    expect(isCompressedKey(`02${'00'.repeat(32)}`)).toBe(false);
    expect(isCompressedKey(`04${MENDE.slice(2)}`)).toBe(false);
    expect(isCompressedKey(P)).toBe(true);
  });
  it('accepts a fresh low-S signature over the digest, not over its hash', () => {
    const sk = secp256k1.utils.randomSecretKey();
    const pk = Buffer.from(secp256k1.getPublicKey(sk, true)).toString('hex');
    const sig = secp256k1.sign(d, sk, { prehash: false, format: 'der', lowS: true });
    expect(signatureIsValid(d, pk, sig)).toBe(true);
    const hashedFirst = secp256k1.sign(d, sk, { prehash: true, format: 'der', lowS: true });
    expect(signatureIsValid(d, pk, hashedFirst)).toBe(false);
  });
});
````

- [ ] **Step 4: Run it to see it fail**

Run: `cd site && npx vitest run tests/unit/snapshotManifest.test.ts`
Expected: FAIL, `Error: Cannot find module '../../src/lib/snapshotManifest.mjs'`.

- [ ] **Step 5: Write the module**

Create `site/src/lib/snapshotManifest.mjs`:

````js
// Read and check a signed snapshot manifest, the engine's format, exactly as
// easyNode's crates/btx-core/src/confirmed_snapshot.rs reads it. The website
// uses this only to FILTER what it stores: the app checks everything again
// and trusts nothing the website says.
//
// A manifest (btxd v0.34.9, src/matmul/trusted_utxo_snapshot_attestation.h):
// a 229-byte version-2 statement, a compact-size count of signatures, each a
// compact-size length and a 33-byte compressed key, then a compact-size
// length and a strict-DER signature. 32-byte fields are little-endian and
// shown reversed ("display hex"), as btxd prints them.
//
// A dissent is a manifest like any other whose statement has all four file
// fields zero (the design, section 6a). The engine refuses to load one.
import { createHash } from 'node:crypto';
import { secp256k1 } from '@noble/curves/secp256k1.js';

export const STATEMENT_VERSION = 2;
export const STATEMENT_LEN = 229;
/** Where the four file fields start: file size, file hash, chunk size and chunk count run to the end. */
export const FILE_FIELDS_AT = 181;
export const MAX_MANIFEST_BYTES = 64 * 1024;
export const MAX_SIGNATURES = 64;
export const MAX_DER_LEN = 72;
export const MAX_FILE_BYTES = 64 * 1024 * 1024;
export const MIN_CHUNK = 64 * 1024;
export const MAX_CHUNK = 4 * 1024 * 1024;
const DOMAIN = new TextEncoder().encode('BTX_TRUSTED_UTXO_SNAPSHOT_ATTESTATION_V2');

export class ManifestError extends Error {}

const hex = (b) => Buffer.from(b).toString('hex');
export const displayHex = (b) => hex(Uint8Array.from(b).reverse());
const le64 = (b, at) => Number(Buffer.from(b.subarray(at, at + 8)).readBigUInt64LE(0));
const le32 = (b, at) => Buffer.from(b.subarray(at, at + 4)).readUInt32LE(0);

export function sha256(bytes) {
  return new Uint8Array(createHash('sha256').update(bytes).digest());
}
export function sha256d(bytes) {
  return sha256(sha256(bytes));
}

/** The statement's fields, read from its 229 bytes. */
export function statementFields(raw) {
  return {
    version: raw[0],
    chainId: displayHex(raw.subarray(1, 33)),
    blockHash: displayHex(raw.subarray(33, 65)),
    height: Buffer.from(raw.subarray(65, 69)).readInt32LE(0),
    hashSerialized: displayHex(raw.subarray(69, 101)),
    coins: le64(raw, 101),
    chainTx: le64(raw, 109),
    shielded: displayHex(raw.subarray(117, 149)),
    replayContext: displayHex(raw.subarray(149, 181)),
    fileSize: le64(raw, 181),
    fileHash: displayHex(raw.subarray(189, 221)),
    chunkSize: le32(raw, 221),
    chunkCount: le32(raw, 225),
  };
}

/**
 * A dissent: what a confirmer sends when a statement's chain facts disagree
 * with its own node's diary. All four file fields are zero, so it has no
 * file, the engine cannot load it, and the website never takes a file for it.
 */
export function isDissent(f) {
  return f.fileSize === 0 && /^0{64}$/.test(f.fileHash) && f.chunkSize === 0 && f.chunkCount === 0;
}

/** What every signature signs: double SHA-256 of the length-prefixed domain and the statement. */
export function statementDigest(raw) {
  const buf = new Uint8Array(1 + DOMAIN.length + raw.length);
  buf[0] = DOMAIN.length;
  buf.set(DOMAIN, 1);
  buf.set(raw, 1 + DOMAIN.length);
  return sha256d(buf);
}

function readCompact(bytes, pos) {
  if (pos.at >= bytes.length) throw new ManifestError('the manifest ends early');
  const first = bytes[pos.at++];
  if (first < 253) return first;
  const width = first === 253 ? 2 : first === 254 ? 4 : 8;
  if (pos.at + width > bytes.length) throw new ManifestError('the manifest ends early');
  let n = 0n;
  for (let i = width - 1; i >= 0; i--) n = (n << 8n) | BigInt(bytes[pos.at + i]);
  pos.at += width;
  const min = width === 2 ? 253n : width === 4 ? 0x10000n : 0x100000000n;
  if (n < min) throw new ManifestError('a length is not written the one allowed way');
  return Number(n);
}

function writeCompact(out, n) {
  if (n < 253) out.push(n);
  else out.push(253, n & 0xff, (n >> 8) & 0xff);
}

/** Read a manifest strictly: version 2, canonical lengths, 33-byte keys, at most 72-byte signatures, nothing after the last. */
export function parseManifest(input) {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  if (bytes.length > MAX_MANIFEST_BYTES) throw new ManifestError(`${bytes.length} bytes is more than a manifest may be`);
  if (bytes.length === 0) throw new ManifestError('the manifest ends early');
  if (bytes[0] !== STATEMENT_VERSION) throw new ManifestError(`statement version ${bytes[0]}, not 2`);
  if (bytes.length < STATEMENT_LEN) throw new ManifestError('the manifest ends early');
  const statement = bytes.slice(0, STATEMENT_LEN);
  const pos = { at: STATEMENT_LEN };
  const count = readCompact(bytes, pos);
  if (count > MAX_SIGNATURES) throw new ManifestError(`${count} signatures is not a manifest`);
  const signatures = [];
  for (let i = 0; i < count; i++) {
    if (readCompact(bytes, pos) !== 33) throw new ManifestError('a signer key is not 33 bytes');
    if (pos.at + 33 > bytes.length) throw new ManifestError('the manifest ends early');
    const key = bytes.slice(pos.at, pos.at + 33);
    pos.at += 33;
    const len = readCompact(bytes, pos);
    if (len > MAX_DER_LEN) throw new ManifestError('a signature is longer than 72 bytes');
    if (pos.at + len > bytes.length) throw new ManifestError('the manifest ends early');
    signatures.push({ key: hex(key), der: bytes.slice(pos.at, pos.at + len) });
    pos.at += len;
  }
  if (pos.at !== bytes.length) throw new ManifestError(`${bytes.length - pos.at} bytes after the last signature`);
  return { statement, signatures };
}

/** The engine's serialization, byte for byte. */
export function serializeManifest(m) {
  const out = Array.from(m.statement);
  writeCompact(out, m.signatures.length);
  for (const s of m.signatures) {
    const key = Buffer.from(s.key, 'hex');
    writeCompact(out, key.length);
    out.push(...key);
    writeCompact(out, s.der.length);
    out.push(...s.der);
  }
  return Uint8Array.from(out);
}

/** The engine's IsStrictDERSignature, without a sighash byte. */
export function isStrictDer(sig) {
  const n = sig.length;
  if (n < 8 || n > MAX_DER_LEN) return false;
  if (sig[0] !== 0x30 || sig[1] !== n - 2 || sig[2] !== 0x02) return false;
  const lenR = sig[3];
  if (lenR === 0 || 5 + lenR >= n) return false;
  if (sig[4] & 0x80) return false;
  if (lenR > 1 && sig[4] === 0 && !(sig[5] & 0x80)) return false;
  const sTag = 4 + lenR;
  if (sig[sTag] !== 0x02) return false;
  const lenS = sig[sTag + 1];
  const sValue = sTag + 2;
  if (lenS === 0 || sValue + lenS !== n) return false;
  if (sig[sValue] & 0x80) return false;
  if (lenS > 1 && sig[sValue] === 0 && !(sig[sValue + 1] & 0x80)) return false;
  return lenR + lenS + 6 === n;
}

/** A compressed secp256k1 key on the curve, as 66 lowercase hex. */
export function isCompressedKey(keyHex) {
  if (!/^0[23][0-9a-f]{64}$/.test(keyHex)) return false;
  try {
    secp256k1.Point.fromHex(keyHex);
    return true;
  } catch {
    return false;
  }
}

/** Strict DER, low S, a compressed key on the curve, and valid over the digest. */
export function signatureIsValid(digest, keyHex, der) {
  if (!isCompressedKey(keyHex) || !isStrictDer(der)) return false;
  try {
    return secp256k1.verify(der, digest, Buffer.from(keyHex, 'hex'), { prehash: false, lowS: true, format: 'der' });
  } catch {
    return false;
  }
}

/** Streaming file hash: [bytes, plain SHA-256 hex, statement-order file hash in display hex]. */
export function fileHashes(chunks) {
  const h = createHash('sha256');
  let len = 0;
  for (const c of chunks) {
    h.update(c);
    len += c.length;
  }
  const plain = new Uint8Array(h.digest());
  return { size: len, sha256: hex(plain), fileHash: displayHex(sha256(plain)) };
}
````

- [ ] **Step 6: Run the test to see it pass**

Run: `cd site && npx vitest run tests/unit/snapshotManifest.test.ts`
Expected: `Tests  10 passed (10)` (derived, not run). These reproduce the independent Python check: statement hash `d3ee9312…0482`, file hash `f234192d…eb2c`, the compiled shielded commitment, the 3060's signature valid, the regtest statement hash `11c5406e…f194`, the dissent vector's statement hash `44ba673d…676b`, and they prove that a high-S twin, non-strict DER and the right signature under the wrong key are refused.

- [ ] **Step 7: Commit**

````bash
cd /Users/m2promende/repos/EasyBTX-snapshot-rendezvous
git add site/package.json site/package-lock.json site/bun.lock site/tests/fixtures/snapshots site/src/lib/snapshotManifest.mjs site/tests/unit/snapshotManifest.test.ts
git commit -m "site: read and verify signed snapshot manifests and dissents, with easyNode's test vectors" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 2: The operator file and its check (`snapshotOperators.mjs`)

**Files:**
- Create: `site/src/lib/snapshot-operators.json`
- Create: `site/src/lib/snapshotOperators.mjs`
- Create: `scripts/check-snapshot-operators.mjs`, `.github/workflows/snapshot-operators.yml`
- Test: `site/tests/unit/snapshotOperators.test.ts`

**Interfaces:**
- Consumes: Task 1 (`isCompressedKey`).
- Produces (Tasks 3, 5, 6): `OPERATOR_FILE_SCHEMA` (1); `checkOperatorFile(file) -> {ok: true, operators, mirror_pins} | {ok: false, reason}` where `operators` is `[{name, keys: [hex66]}]` and `mirror_pins` is `[hex66]`; `checkOperatorList(ops) -> {ok: true, list} | {ok: false, reason}`; `parseRegtestOperators(raw) -> list | null`; `parseKeyList(raw) -> [hex66] | null`; `operatorOf(list, keyHex) -> name | null`; `distinctOperators(list, keys) -> names` (list order, each once); `listsFromEnv(env, operatorFile) -> {main: list, regtest: list | null, pins: {main: [hex66], regtest: [hex66]}}`. The module takes the parsed file as an argument instead of importing it, so the Vite-built routes and tests (`import operatorFile from '.../snapshot-operators.json'`) and the plain-Node scripts (`JSON.parse(readFileSync(...))`) share it without import attributes.

- [ ] **Step 1: Write the failing test**

Create `site/tests/unit/snapshotOperators.test.ts`:

````ts
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import operatorFile from '../../src/lib/snapshot-operators.json';
import {
  checkOperatorFile, checkOperatorList, parseRegtestOperators, parseKeyList, operatorOf, distinctOperators, listsFromEnv,
} from '../../src/lib/snapshotOperators.mjs';

const RAW = readFileSync(new URL('../../src/lib/snapshot-operators.json', import.meta.url), 'utf8');
const MENDE = '02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675';
const ALEKS_1 = '026da4e3a07676cada5488123033fb1057faeb7fe6a7d50b2ae3921431e0f4bbf1';
const ALEKS_2 = '03047189023913e1922c80c895ee2a9e2eff6df05438654749e1a4f95019578a24';
const ALEKS_3 = '02c9cfb77d7e4dce0cd6b7968fee1dd53d31ef06c764185e120c825fab8c0572a0';
const JPP = '02e0a9653b49ad74900dec86b86770735579e8e64859028a76129c71a70e1cadb5';
const PINS = [
  '0224e80df33697385b54b3c69bae1f097f533c0c43e93c29f73ee97319d4a5e04c',
  '028995b25c887ee03eb53a41312d33c8eccf48f261ecf9e91fe2b1e8e50373258a',
  MENDE,
  '03d90c148db37da28ce47ce15bade88a177728d663da4bc9ba765943b7d4e4f0aa',
];
const P = '0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464';
const C = '02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f';
const D = '034694ab29307fd4e46f3fc7a5115dd52b4143c473a5eb919e857eb5a12bbd6d0e';

describe('the checked-in operator file, the same bytes as easyNode\'s', () => {
  it('is written the one way both repositories write it', () => {
    // Two-space indent, keys in this order, one newline at the end: what
    // JSON.stringify(x, null, 2) gives, and what serde_json's pretty printer
    // gives on the app side.
    expect(RAW).toBe(`${JSON.stringify(JSON.parse(RAW), null, 2)}\n`);
    expect(Object.keys(JSON.parse(RAW))).toEqual(['schema', 'operators', 'mirror_pins']);
  });
  it('lists Mende, Aleksander with three keys, and jpp, and the keys every node pins', () => {
    const r: any = checkOperatorFile(operatorFile);
    expect(r.ok, r.reason).toBe(true);
    expect(r.operators).toEqual([
      { name: 'Mende', keys: [MENDE] },
      { name: 'Aleksander', keys: [ALEKS_1, ALEKS_2, ALEKS_3] },
      { name: 'jpp', keys: [JPP] },
    ]);
    expect(r.mirror_pins).toEqual(PINS);
    // Today only the 3060's key is both listed and pinned, so latest needs its signature.
    expect(r.operators.flatMap((o: any) => o.keys).filter((k: string) => r.mirror_pins.includes(k))).toEqual([MENDE]);
  });
  it('refuses a file of another schema, without pins, or with a pin that is not a key', () => {
    const file = JSON.parse(RAW);
    expect(checkOperatorFile({ ...file, schema: 2 }).ok).toBe(false);
    expect(checkOperatorFile({ ...file, mirror_pins: undefined }).ok).toBe(false);
    expect(checkOperatorFile({ ...file, mirror_pins: [] }).ok).toBe(false);
    expect(checkOperatorFile({ ...file, mirror_pins: [`02${'00'.repeat(32)}`] }).ok).toBe(false);
    expect(checkOperatorFile({ ...file, mirror_pins: [MENDE, MENDE] }).ok).toBe(false);
    expect(checkOperatorFile({ ...file, operators: [...file.operators, { name: 'Mende', keys: [P] }] }).ok).toBe(false);
    expect(checkOperatorFile('Mende').ok).toBe(false);
  });
});

describe('checkOperatorList', () => {
  it('counts the keys of one operator once', () => {
    const r: any = checkOperatorList((checkOperatorFile(operatorFile) as any).operators);
    expect(r.ok).toBe(true);
    expect(distinctOperators(r.list, [ALEKS_1, ALEKS_2, ALEKS_3])).toEqual(['Aleksander']);
    expect(distinctOperators(r.list, [JPP, ALEKS_3, MENDE, P])).toEqual(['Mende', 'Aleksander', 'jpp']);
    expect(operatorOf(r.list, P)).toBeNull();
  });
  it('refuses a malformed list', () => {
    expect(checkOperatorList([{ name: 'a', keys: [P] }, { name: 'a', keys: [C] }]).ok).toBe(false);
    expect(checkOperatorList([{ name: ' ', keys: [P] }]).ok).toBe(false);
    expect(checkOperatorList([{ name: 'a', keys: [P] }, { name: 'b', keys: [P] }]).ok).toBe(false);
    expect(checkOperatorList([{ name: 'a', keys: [] }]).ok).toBe(false);
    expect(checkOperatorList([{ name: 'a', keys: [`02${'00'.repeat(32)}`] }]).ok).toBe(false);
    expect(checkOperatorList('Mende').ok).toBe(false);
  });
});

describe('the test chain', () => {
  it('reads easyNode\'s operator format and refuses anything else', () => {
    const list = parseRegtestOperators(`producer=${P};confirmer=${C},${D}`)!;
    expect(list.map((o: any) => o.name)).toEqual(['producer', 'confirmer']);
    expect(operatorOf(list, D.toUpperCase())).toBe('confirmer');
    expect(parseRegtestOperators('producer')).toBeNull();
    expect(parseRegtestOperators(`a=${P};b=${P}`)).toBeNull();
    expect(parseRegtestOperators('')).toBeNull();
    expect(parseRegtestOperators(undefined)).toBeNull();
  });
  it('reads the test chain\'s pins', () => {
    expect(parseKeyList(` ${P}, ${C} `)).toEqual([P, C]);
    expect(parseKeyList(`${P},${P}`)).toBeNull();
    expect(parseKeyList('zz')).toBeNull();
    expect(parseKeyList('')).toBeNull();
    expect(parseKeyList(undefined)).toBeNull();
  });
  it('never reaches a production deployment', () => {
    const env = { SNAPSHOT_REGTEST_OPERATORS: `producer=${P};confirmer=${C}`, SNAPSHOT_REGTEST_PINS: P };
    const l = listsFromEnv(env, operatorFile);
    expect(l.regtest).toHaveLength(2);
    expect(l.pins.regtest).toEqual([P]);
    const prod = listsFromEnv({ ...env, VERCEL_ENV: 'production' }, operatorFile);
    expect(prod.regtest).toBeNull();
    expect(prod.pins.regtest).toEqual([]);
    expect(prod.main.map((o: any) => o.name)).toEqual(['Mende', 'Aleksander', 'jpp']);
    expect(prod.pins.main).toEqual(PINS);
    expect(listsFromEnv({}, operatorFile).regtest).toBeNull();
  });
});
````

- [ ] **Step 2: Run it to see it fail**

Run: `cd site && npx vitest run tests/unit/snapshotOperators.test.ts`
Expected: FAIL, `Error: Cannot find module '../../src/lib/snapshot-operators.json'` (or the same for `snapshotOperators.mjs`).

- [ ] **Step 3: Write the file and the module**

Create `site/src/lib/snapshot-operators.json` with exactly these 928 bytes (two-space indent, keys in this order, one newline at the end). It must stay byte for byte easyNode's `crates/btx-core/snapshot-operators.json` (core plan, Task 1):

````json
{
  "schema": 1,
  "operators": [
    {
      "name": "Mende",
      "keys": [
        "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675"
      ]
    },
    {
      "name": "Aleksander",
      "keys": [
        "026da4e3a07676cada5488123033fb1057faeb7fe6a7d50b2ae3921431e0f4bbf1",
        "03047189023913e1922c80c895ee2a9e2eff6df05438654749e1a4f95019578a24",
        "02c9cfb77d7e4dce0cd6b7968fee1dd53d31ef06c764185e120c825fab8c0572a0"
      ]
    },
    {
      "name": "jpp",
      "keys": [
        "02e0a9653b49ad74900dec86b86770735579e8e64859028a76129c71a70e1cadb5"
      ]
    }
  ],
  "mirror_pins": [
    "0224e80df33697385b54b3c69bae1f097f533c0c43e93c29f73ee97319d4a5e04c",
    "028995b25c887ee03eb53a41312d33c8eccf48f261ecf9e91fe2b1e8e50373258a",
    "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675",
    "03d90c148db37da28ce47ce15bade88a177728d663da4bc9ba765943b7d4e4f0aa"
  ]
}
````

Check it: `shasum -a 256 site/src/lib/snapshot-operators.json` gives `f91529f31dde7311d7970088187b3d39838cc8722b97a8bd3dda894e519fdf3d`, and `wc -c` gives `928`.

Create `site/src/lib/snapshotOperators.mjs`:

````js
// Who may sign a chain snapshot the website stores, how their keys group, and
// which keys every node pins.
//
// site/src/lib/snapshot-operators.json is byte for byte the file easyNode
// compiles into the app (crates/btx-core/snapshot-operators.json, read by
// crates/btx-core/src/operators.rs). scripts/check-snapshot-operators.mjs
// fails CI when the two differ by one byte. The website uses its copy only to
// filter: the app never takes the website's word for anything and checks
// every signature against its own compiled list again.
//
// An operator is a person. All of one person's keys count once, so one
// person with three machines can never confirm a snapshot alone.
//
// `mirror_pins` are the keys every node pins. A node loads only a manifest
// that carries a signature from one of them, so the website points only at
// such a manifest. It is not a second authority: it only keeps the website
// from pointing at a snapshot no node could load.
import { isCompressedKey } from './snapshotManifest.mjs';

export const OPERATOR_FILE_SCHEMA = 1;

/**
 * A checked list: names unique and not empty, every key a compressed point on
 * the curve, no key twice.
 * @param {unknown} ops
 * @returns {{ ok: true, list: Array<{name: string, keys: string[]}> } | { ok: false, reason: string }}
 */
export function checkOperatorList(ops) {
  if (!Array.isArray(ops)) return { ok: false, reason: 'the list is not a list' };
  const names = new Set();
  const keys = new Set();
  const list = [];
  for (const op of ops) {
    const name = typeof op?.name === 'string' ? op.name.trim() : '';
    if (!name) return { ok: false, reason: 'an operator has no name' };
    if (names.has(name)) return { ok: false, reason: `operator ${name} is listed twice` };
    names.add(name);
    if (!Array.isArray(op.keys) || op.keys.length === 0) return { ok: false, reason: `operator ${name} has no key` };
    const own = [];
    for (const k of op.keys) {
      const key = typeof k === 'string' ? k.trim().toLowerCase() : '';
      if (!isCompressedKey(key)) return { ok: false, reason: `operator ${name} has a key that is not a compressed secp256k1 point` };
      if (keys.has(key)) return { ok: false, reason: `key ${key} is listed twice` };
      keys.add(key);
      own.push(key);
    }
    list.push({ name, keys: own });
  }
  return { ok: true, list };
}

/** At least one key, each a compressed point on the curve, none twice. */
function checkKeys(raw) {
  if (!Array.isArray(raw) || raw.length === 0) return { ok: false, reason: 'no keys' };
  const seen = new Set();
  for (const k of raw) {
    const key = typeof k === 'string' ? k.trim().toLowerCase() : '';
    if (!isCompressedKey(key)) return { ok: false, reason: `${key || 'a key'} is not a compressed secp256k1 point` };
    if (seen.has(key)) return { ok: false, reason: `${key} is listed twice` };
    seen.add(key);
  }
  return { ok: true, keys: [...seen] };
}

/**
 * The operator file: `{schema: 1, operators: [...], mirror_pins: [...]}`.
 * @param {unknown} file the parsed site/src/lib/snapshot-operators.json
 * @returns {{ ok: true, operators: Array<{name: string, keys: string[]}>, mirror_pins: string[] } | { ok: false, reason: string }}
 */
export function checkOperatorFile(file) {
  if (!file || typeof file !== 'object') return { ok: false, reason: 'the operator file is not an object' };
  const f = /** @type {any} */ (file);
  if (f.schema !== OPERATOR_FILE_SCHEMA) return { ok: false, reason: `schema ${f.schema}, not ${OPERATOR_FILE_SCHEMA}` };
  const ops = checkOperatorList(f.operators);
  if (!ops.ok) return ops;
  const pins = checkKeys(f.mirror_pins);
  if (!pins.ok) return { ok: false, reason: `mirror_pins: ${pins.reason}` };
  return { ok: true, operators: ops.list, mirror_pins: pins.keys };
}

/**
 * The test-chain list, in easyNode's EASYNODE_REGTEST_OPERATORS format:
 * `name=key[,key];name=key`. Empty or malformed gives null: no regtest
 * statement is taken at all.
 * @param {string | undefined} raw
 */
export function parseRegtestOperators(raw) {
  if (!raw || !raw.trim()) return null;
  const ops = [];
  for (const part of raw.split(';').map((p) => p.trim()).filter(Boolean)) {
    const at = part.indexOf('=');
    if (at < 0) return null;
    ops.push({ name: part.slice(0, at).trim(), keys: part.slice(at + 1).split(',').map((k) => k.trim()) });
  }
  const r = checkOperatorList(ops);
  return r.ok ? r.list : null;
}

/**
 * The test chain's pins, `key,key`. Empty or malformed gives null.
 * @param {string | undefined} raw
 */
export function parseKeyList(raw) {
  if (!raw || !raw.trim()) return null;
  const r = checkKeys(raw.split(',').map((k) => k.trim()).filter(Boolean));
  return r.ok ? r.keys : null;
}

/** The operator a key belongs to, or null. */
export function operatorOf(list, keyHex) {
  const k = String(keyHex).toLowerCase();
  const op = list.find((o) => o.keys.includes(k));
  return op ? op.name : null;
}

/** The operators behind `keys`, each named once, in list order. Unlisted keys count for nobody. */
export function distinctOperators(list, keys) {
  const set = new Set(keys.map((k) => String(k).toLowerCase()));
  return list.filter((o) => o.keys.some((k) => set.has(k))).map((o) => o.name);
}

/**
 * The lists and pins this deployment checks against. Mainnet's come from the
 * checked-in file. The test chain's come from SNAPSHOT_REGTEST_OPERATORS and
 * SNAPSHOT_REGTEST_PINS and never on a production deployment, so a
 * production site stores no test-chain statement.
 * @param {Record<string, string | undefined>} env
 * @param {unknown} operatorFile the parsed site/src/lib/snapshot-operators.json
 */
export function listsFromEnv(env, operatorFile) {
  const file = checkOperatorFile(operatorFile);
  const production = env.VERCEL_ENV === 'production';
  return {
    main: file.ok ? file.operators : [],
    regtest: production ? null : parseRegtestOperators(env.SNAPSHOT_REGTEST_OPERATORS),
    pins: {
      main: file.ok ? file.mirror_pins : [],
      regtest: production ? [] : parseKeyList(env.SNAPSHOT_REGTEST_PINS) || [],
    },
  };
}
````

- [ ] **Step 4: Run the test to see it pass**

Run: `cd site && npx vitest run tests/unit/snapshotOperators.test.ts`
Expected: `Tests  8 passed (8)` (derived, not run).

- [ ] **Step 5: Write the cross-repository check**

How CI gets the app's copy, decided here: it fetches easyNode's file **at one pinned commit of easynode's `main`**, by full hash, from the public repository's raw files. Not `main` itself: the list changes only with a signed app release (design, section 1), and easynode's `main` can carry a new list days before that release ships; following `main` would make the website filter with a list no running app has, and would turn every unrelated website PR red the moment the app side edits its list. The pin moves in one PR here, together with the copy, once the release that carries the new list has shipped. A hash, not a tag or a branch, because a hash cannot move. Not this repository's own `crates/btx-core`: that is a downstream working copy that lags easynode (`scripts/check-public-sync.sh`), so it proves nothing about what the app ships. The daily run also looks at easynode's `main` and warns, without failing, when it has moved on.

Create `scripts/check-snapshot-operators.mjs`:

````js
#!/usr/bin/env node
// The website's copy of the snapshot operator list must be easyNode's, byte
// for byte (easyNode's design, section 1).
//
// easyNode compiles the list into the app (crates/btx-core/src/operators.rs
// reads crates/btx-core/snapshot-operators.json), and that file is the only
// place the list is written. The website keeps the same bytes in
// site/src/lib/snapshot-operators.json to decide which signatures it stores
// and which snapshot it points at. If the two differed, the website would
// take or refuse signatures differently from the app, so this fails on one
// changed byte.
//
// WHICH copy of easyNode's file: the one at EASYNODE_COMMIT, a full commit
// hash on easynode's main, fetched from the public repository.
//   * Not easynode's main as it is today: the list changes only with a signed
//     app release, and main can carry a new list days before that release
//     ships. Following main would make the website filter with a list no
//     running app has, and would turn every unrelated website PR red the
//     moment the app side edits its list. The pin moves here in one PR,
//     together with the copy, once the release that carries the new list
//     has shipped.
//   * Not a tag or a branch: a commit hash cannot move.
//   * Not this repository's own crates/btx-core: a downstream working copy
//     that lags easynode (scripts/check-public-sync.sh), so it proves
//     nothing about what the app ships.
// With --watch-main (the daily run) it also looks at easynode's main and
// warns, without failing, when that has another list, so a coming change is
// seen here before it ships.
//
//   node scripts/check-snapshot-operators.mjs
//   node scripts/check-snapshot-operators.mjs --watch-main
//   EASYNODE_REF=<commit or branch> node scripts/check-snapshot-operators.mjs   (a look by hand)
//   EASYNODE_OPERATORS_FILE=<path> node scripts/check-snapshot-operators.mjs    (offline)
import { readFileSync } from 'node:fs';

// The easynode main commit that carries the list this copy was taken from.
// Set in the PR that adds or changes the copy (the website plan, Task 2,
// Step 6, and Task 7, Step 2).
const EASYNODE_COMMIT = '';
const FILE = 'crates/btx-core/snapshot-operators.json';
const rawUrl = (ref) => `https://raw.githubusercontent.com/MendeMatthias/easynode/${ref}/${FILE}`;
const LOCAL = new URL('../site/src/lib/snapshot-operators.json', import.meta.url);

function fail(message) {
  console.error(`::error::${message}`);
  process.exit(1);
}

async function fetchBytes(url) {
  let res;
  try {
    res = await fetch(url, { signal: AbortSignal.timeout(15_000) });
  } catch (e) {
    fail(`could not reach ${url}: ${e?.message || e}`);
  }
  if (!res.ok) fail(`${url} answered HTTP ${res.status}`);
  // Bytes, not text: text() would drop a byte-order mark and hide a difference.
  return Buffer.from(await res.arrayBuffer());
}

const describe = (bytes) => {
  try {
    const f = JSON.parse(bytes.toString('utf8'));
    const ops = (f.operators || []).map((o) => `${o.name} (${o.keys.length} ${o.keys.length === 1 ? 'key' : 'keys'})`);
    return `${ops.join(', ')}; ${(f.mirror_pins || []).length} pinned keys`;
  } catch {
    return '<not JSON>';
  }
};

const local = readFileSync(LOCAL);
let theirs;
let from;
if (process.env.EASYNODE_OPERATORS_FILE) {
  from = process.env.EASYNODE_OPERATORS_FILE;
  theirs = readFileSync(from);
} else {
  const ref = process.env.EASYNODE_REF || EASYNODE_COMMIT;
  if (!process.env.EASYNODE_REF && !/^[0-9a-f]{40}$/.test(ref)) {
    fail('EASYNODE_COMMIT in scripts/check-snapshot-operators.mjs is not a full commit hash yet. Set it to the easynode main commit that carries the list.');
  }
  from = `easynode ${ref}`;
  theirs = await fetchBytes(rawUrl(ref));
}

if (!local.equals(theirs)) {
  console.error('::error::site/src/lib/snapshot-operators.json is not byte for byte easyNode\'s crates/btx-core/snapshot-operators.json');
  console.error(`  website: ${describe(local)}`);
  console.error(`  ${from}: ${describe(theirs)}`);
  console.error('  Copy easyNode\'s file over the website\'s, and move EASYNODE_COMMIT with it, once the app release that carries it has shipped.');
  process.exit(1);
}
console.log(`snapshot operators: the website's copy is ${from}, byte for byte: ${describe(local)}`);

if (process.argv.includes('--watch-main') && !process.env.EASYNODE_OPERATORS_FILE) {
  const main = await fetchBytes(rawUrl('main'));
  if (!main.equals(theirs)) {
    console.log(`::warning::easynode main has another operator list (${describe(main)}). Nothing to do until the app release that carries it ships; then copy it here and move EASYNODE_COMMIT.`);
  } else {
    console.log('easynode main has the same list.');
  }
}
````

Create `.github/workflows/snapshot-operators.yml` (the action pins are the ones `site-tests.yml` uses on `3dcfa37a`):

````yaml
name: Snapshot operators

# The website stores a snapshot statement only when every signature on it is
# from a key on its copy of the operator list (site/src/lib/snapshot-operators.json),
# and points at a snapshot only when a key every node pins signed it too.
# easyNode compiles the real list into the app from
# crates/btx-core/snapshot-operators.json. This fails when the website's copy
# is not that file, byte for byte, at the easynode commit pinned in
# scripts/check-snapshot-operators.mjs: on every PR that touches the copy or
# the check, and once a day, when it also warns if easynode's main has moved
# on to another list.
#
# Security: no github.event.* content is interpolated into any run: step.

on:
  pull_request:
    branches: [main]
    paths:
      - "site/src/lib/snapshot-operators.json"
      - "scripts/check-snapshot-operators.mjs"
      - ".github/workflows/snapshot-operators.yml"
  push:
    branches: [main]
    paths:
      - "site/src/lib/snapshot-operators.json"
      - "scripts/check-snapshot-operators.mjs"
  schedule:
    - cron: "23 6 * * *"
  workflow_dispatch:

permissions:
  contents: read

jobs:
  same-list:
    name: website copy equals easyNode's list
    runs-on: ubuntu-latest
    timeout-minutes: 5
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

      - uses: actions/setup-node@2028fbc5c25fe9cf00d9f06a71cc4710d4507903 # v5.0.0
        with:
          node-version: 22

      - name: Compare with the pinned easynode commit
        if: github.event_name != 'schedule'
        run: node scripts/check-snapshot-operators.mjs

      - name: Compare, and look at easynode main
        if: github.event_name == 'schedule'
        run: node scripts/check-snapshot-operators.mjs --watch-main
````

- [ ] **Step 6: Run the check both ways, and set the pin if easyNode's list is on its main**

````bash
cd /Users/m2promende/repos/EasyBTX-snapshot-rendezvous
X=$(mktemp -d)
EASYNODE_OPERATORS_FILE=site/src/lib/snapshot-operators.json node scripts/check-snapshot-operators.mjs; echo "exit $?"
sed 's/"jpp"/"someone"/' site/src/lib/snapshot-operators.json > $X/other-operators.json
EASYNODE_OPERATORS_FILE=$X/other-operators.json node scripts/check-snapshot-operators.mjs; echo "exit $?"
node scripts/check-snapshot-operators.mjs; echo "exit $?"
git -C /Users/m2promende/repos/easynode fetch -q origin main
PIN=$(git -C /Users/m2promende/repos/easynode log -1 --format=%H origin/main -- crates/btx-core/snapshot-operators.json); echo "pin: ${PIN:-none yet}"
[ -n "$PIN" ] && git -C /Users/m2promende/repos/easynode show "$PIN:crates/btx-core/snapshot-operators.json" | cmp - site/src/lib/snapshot-operators.json && echo "same bytes at $PIN"
````

Expected (derived, not run): first `snapshot operators: the website's copy is site/src/lib/snapshot-operators.json, byte for byte: Mende (1 key), Aleksander (3 keys), jpp (1 key); 4 pinned keys` and `exit 0`; second `::error::site/src/lib/snapshot-operators.json is not byte for byte easyNode's …` with both lists named, and `exit 1`; third `::error::EASYNODE_COMMIT in scripts/check-snapshot-operators.mjs is not a full commit hash yet. …` and `exit 1`. Then:

- If `pin:` shows a hash and `same bytes at <hash>` follows, set `const EASYNODE_COMMIT = '<that hash>';` in the script and run `node scripts/check-snapshot-operators.mjs; echo "exit $?"`: `snapshot operators: the website's copy is easynode <hash>, byte for byte: …` and `exit 0`.
- If it says `pin: none yet` (expected on 2026-09-29: the app plan's list task is not on easynode `main`, and it waits for the owner's word on Aleksander), leave `EASYNODE_COMMIT` empty and go on; Task 7, Step 2 sets it. The workflow fails on the PR until then, which is the gate: do not merge this PR while the live check fails.
- If the bytes differ, stop: one of the two files is not the INTERFACES content, and the owner decides which one is right.

- [ ] **Step 7: Commit**

````bash
git add site/src/lib/snapshot-operators.json site/src/lib/snapshotOperators.mjs site/tests/unit/snapshotOperators.test.ts scripts/check-snapshot-operators.mjs .github/workflows/snapshot-operators.yml
git commit -m "site: easyNode's snapshot operator file, and a check that it stays the same at a pinned easynode commit" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 3: The rules (`snapshotRendezvous.mjs`)

**Files:**
- Create: `site/src/lib/snapshotRendezvous.mjs`
- Test: `site/tests/unit/snapshotRendezvous.test.ts`

**Interfaces:**
- Consumes: Task 1 (`parseManifest`, `serializeManifest`, `statementFields`, `statementDigest`, `displayHex`, `isDissent`, `signatureIsValid`, `ManifestError`, `MAX_MANIFEST_BYTES`, `MAX_FILE_BYTES`, `MIN_CHUNK`, `MAX_CHUNK`), Task 2 (`operatorOf`, `distinctOperators`, the `lists` shape of `listsFromEnv`).
- Produces (Task 5): `GRID` (100), `MAIN_SHIELDED`, `CHAINS` (`{main, regtest}` each `{genesis, replayContext, shielded, grid}`), `PENDING_DAYS` (7), `KEEP_CONFIRMED` (5), `KEEP_PENDING_FILES` (10), `CLOSED_HEIGHT` (`'closed-height'`), `CHAIN_FIELDS`, `chainOf(chainIdHex)`, `acceptManifest(bytes, lists) -> {ok: true, chain, fields, hash, manifest, dissent} | {ok: false, reason}`, `mergeSignatures(stored, incoming) -> {manifest, added: [key]}`, `newRecord(accepted, nowIso)`, `withManifest(record, manifest, nowIso)`, `recordManifest(record)`, `recordOperators(record, lists)`, `hasPinnedSigner(record, lists)`, `isConfirmed(record, lists)`, `chooseLatest(records, chain, lists) -> record | null`, `latestView(record, lists)`, `disputeAt(records, chain, height, lists) -> {chain, height, statements, differ} | null`, `liveDisputes(records, chain, lists)`, `disputedHeights(records, chain, lists, recorded)`, `joinNames(names)`, `alertText(dispute, lists)`, `newDispute(dispute, lists, nowIso)`, `refreshDispute(record, dispute, lists)`, `disputesView(disputeRecords, chain)`, `pendingView(records, chain, nowMs, lists, recorded)`, `storagePlan(records, nowMs, lists, recorded) -> {dropRecords, dropFiles}`, `partPlan(fileSize, partBytes) -> {parts, sizeOf(n)}`.
- The statement record (private JSON, one per statement or dissent): `{version: 1, chain, statement_hash, height, block_hash, hash_serialized_3, coins, chain_tx, file_size, file_hash, dissent, manifest_hex, signers, first_seen, updated_at, file: {state: "missing"} | {state: "uploading", upload, started_at} | {state: "stored", url, sha256, stored_at} | {state: "dropped", dropped_at}, public_manifest: null | {url, sha256, size, signatures}, confirmed_at: null | iso}`. Operators are never stored: they are worked out from `signers` under the list at the moment of reading.
- The dispute record (private JSON, one per disputed height and chain): `{version: 1, chain, height, statements: [{statement_hash, block_hash, hash_serialized_3, coins, chain_tx, file_size, file_hash, signers: [names], dissent}], differ: [field names], alert, opened_at}`. `alert` and `opened_at` are written once; `statements` and `differ` follow what is stored at that height.

- [ ] **Step 1: Write the failing test**

Create `site/tests/unit/snapshotRendezvous.test.ts`. One sabotage per rule: an unknown chain, another replay context, an empty shielded commitment, another shielded commitment on mainnet, bad chunk geometry, a zero file size beside a file hash, a file over 64 MiB, off-grid heights on both chains (including the real 225,927 manifest), no signatures, a key twice, a signature that does not verify, a high-S twin, a key on no list; dissents taken without a file; for `latest`: one operator never, two operators without the file never, two keys of one operator once, two operators without a pinned signature never (and the older pinned one served instead), a key or a pin taken away, a dissent never, a file-only twin; for disputes: the exact alert on mainnet, a file-only difference not a dispute, a dissent that agrees and one that does not, unlisted signers disputing nothing, a recorded dispute standing until the owner clears it; for storage: five confirmed, ten pending files, nothing at a disputed height, and the file of a pending pair below the newest confirmed height.

````ts
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { secp256k1 } from '@noble/curves/secp256k1.js';
import operatorFile from '../../src/lib/snapshot-operators.json';
import { parseManifest, serializeManifest, statementDigest } from '../../src/lib/snapshotManifest.mjs';
import { parseRegtestOperators, listsFromEnv } from '../../src/lib/snapshotOperators.mjs';
import {
  acceptManifest, mergeSignatures, newRecord, withManifest, recordOperators, isConfirmed,
  chooseLatest, latestView, pendingView, storagePlan, partPlan, disputeAt, liveDisputes, disputedHeights,
  alertText, newDispute, refreshDispute, disputesView, joinNames, CHAINS, MAIN_SHIELDED,
} from '../../src/lib/snapshotRendezvous.mjs';

const fx = (n: string) => new Uint8Array(readFileSync(new URL(`../fixtures/snapshots/${n}`, import.meta.url)));
const MAINNET = fx('mainnet-225927.manifest');
const R = (n: string) => fx(`regtest-${n}.manifest`);
const P = '0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464';
const C = '02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f';
const D = '034694ab29307fd4e46f3fc7a5115dd52b4143c473a5eb919e857eb5a12bbd6d0e';
const MENDE = '02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675';
const ALEKS_2 = '03047189023913e1922c80c895ee2a9e2eff6df05438654749e1a4f95019578a24';
const JPP = '02e0a9653b49ad74900dec86b86770735579e8e64859028a76129c71a70e1cadb5';
const BLOCK = 'bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46';
const HS3 = 'e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527';
const NOW = Date.parse('2026-10-01T12:00:00Z');
const iso = (msAgo: number) => new Date(NOW - msAgo).toISOString();
const DAY = 24 * 3600 * 1000;
const num = (a: number, b: number) => a - b;

const mainLists = listsFromEnv({}, operatorFile);
const regtestLists = () => ({
  main: mainLists.main,
  regtest: parseRegtestOperators(`producer=${P};confirmer=${C};third=${D}`),
  pins: { main: mainLists.pins.main, regtest: [P] },
});

// Fresh keys for statements the vectors do not cover.
const fresh = () => {
  const sk = secp256k1.utils.randomSecretKey();
  return { sk, pub: Buffer.from(secp256k1.getPublicKey(sk, true)).toString('hex') };
};
const A = fresh();
const B = fresh();
const OUTSIDER = fresh();
const freshLists = () => ({
  main: [{ name: 'a', keys: [A.pub] }, { name: 'b', keys: [B.pub] }],
  regtest: [{ name: 'a', keys: [A.pub] }, { name: 'b', keys: [B.pub] }],
  pins: { main: [A.pub], regtest: [A.pub] },
});
const sign = (statement: Uint8Array, ...who: { sk: Uint8Array; pub: string }[]) =>
  serializeManifest({
    statement,
    signatures: who.map((w) => ({
      key: w.pub,
      der: secp256k1.sign(statementDigest(statement), w.sk, { prehash: false, format: 'der', lowS: true }),
    })),
  });
/** A vector's statement with `edit` applied to a copy of its 229 bytes. */
const edited = (edit: (st: Uint8Array) => void, base: Uint8Array = R('P')) => {
  const st = parseManifest(base).statement.slice();
  edit(st);
  return st;
};
const setU32 = (st: Uint8Array, at: number, v: number) => Buffer.from(st.buffer, st.byteOffset).writeUInt32LE(v, at);
const setI32 = (st: Uint8Array, at: number, v: number) => Buffer.from(st.buffer, st.byteOffset).writeInt32LE(v, at);
const setU64 = (st: Uint8Array, at: number, v: number) => Buffer.from(st.buffer, st.byteOffset).writeBigUInt64LE(BigInt(v), at);

describe('acceptManifest: what the website stores', () => {
  it('takes the spike\'s two-operator regtest statement', () => {
    const r: any = acceptManifest(R('PC'), regtestLists());
    expect(r.ok).toBe(true);
    expect([r.chain, r.dissent]).toEqual(['regtest', false]);
    expect(r.hash).toBe('11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194');
    expect(r.fields.height).toBe(100);
  });
  it('takes no test-chain statement where no test list is set', () => {
    expect(acceptManifest(R('PC'), { ...regtestLists(), regtest: null })).toEqual({
      ok: false, reason: 'chain 521ad0951ed299e9c56aeb7db8188972772067560351b8e55adf71dbed532360 is not one this website takes',
    });
  });
  it('refuses the real 225,927 manifest: its height is off the grid of 100', () => {
    expect(acceptManifest(MAINNET, regtestLists())).toEqual({ ok: false, reason: 'height 225927 is not a multiple of 100' });
  });
  it('takes a mainnet statement at a grid height signed by listed keys', () => {
    const st = edited((s) => setI32(s, 65, 226_000), MAINNET);
    const r: any = acceptManifest(sign(st, A, B), freshLists());
    expect(r.ok, r.reason).toBe(true);
    expect([r.chain, r.dissent, r.fields.shielded]).toEqual(['main', false, MAIN_SHIELDED]);
  });
  // One sabotage per rule. Each statement is otherwise a valid one.
  const cases: [string, (s: Uint8Array) => void, string, Uint8Array?][] = [
    ['an unknown chain', (s) => s.fill(7, 1, 33), 'is not one this website takes'],
    ['another replay context', (s) => { s[149] ^= 1; }, 'the replay context is not this engine\'s'],
    ['an empty shielded commitment', (s) => s.fill(0, 117, 149), 'the shielded commitment is empty'],
    ['another shielded commitment on mainnet', (s) => { setI32(s, 65, 226_000); s[117] ^= 1; }, 'the shielded commitment is not this chain\'s', MAINNET],
    ['chunks that do not add up', (s) => setU32(s, 225, 2), 'the file size and chunks do not add up'],
    ['a chunk size below the engine\'s', (s) => setU32(s, 221, 1024), 'the file size and chunks do not add up'],
    ['a zero file size beside a file hash', (s) => setU64(s, 181, 0), 'the file size and chunks do not add up'],
    ['a file over 64 MiB', (s) => { setU64(s, 181, 64 * 1024 * 1024 + 1); setU32(s, 221, 4 * 1024 * 1024); setU32(s, 225, 17); }, 'is more than this website stores'],
    ['an off-grid height', (s) => setI32(s, 65, 150), 'height 150 is not a multiple of 100'],
    ['height zero', (s) => setI32(s, 65, 0), 'height 0 is not a multiple of 100'],
    ['an off-grid mainnet height', (s) => setI32(s, 65, 226_050), 'height 226050 is not a multiple of 100', MAINNET],
  ];
  for (const [what, edit, reason, base] of cases) {
    it(`refuses ${what}`, () => {
      const r: any = acceptManifest(sign(edited(edit, base), A, B), freshLists());
      expect(r.ok).toBe(false);
      expect(r.reason).toContain(reason);
    });
  }
  it('takes a dissent: every file field zero, on the grid, signed by a listed key', () => {
    const r: any = acceptManifest(sign(edited((s) => s.fill(0, 181, 229)), A), freshLists());
    expect(r.ok, r.reason).toBe(true);
    expect([r.chain, r.dissent, r.fields.fileSize]).toEqual(['regtest', true, 0]);
  });
  it('takes the dissent vector moved onto the grid, and refuses it where it stands', () => {
    const st = edited((s) => { setI32(s, 65, 226_000); s.fill(0, 181, 229); }, MAINNET);
    const r: any = acceptManifest(sign(st, A), freshLists());
    expect(r.ok, r.reason).toBe(true);
    expect([r.chain, r.dissent, r.hash]).toEqual(['main', true, 'a4dae6bf1eb49595c133252661b9e3a28dd5a2507ddf5d6aad03e31329cf1fab']);
    expect((acceptManifest(sign(edited((s) => s.fill(0, 181, 229), MAINNET), A), freshLists()) as any).reason)
      .toBe('height 225927 is not a multiple of 100');
  });
  it('refuses a statement signed by a key on no list, even with listed keys beside it', () => {
    const st = edited(() => {});
    expect(acceptManifest(sign(st, A, OUTSIDER), freshLists())).toEqual({
      ok: false, reason: `key ${OUTSIDER.pub} is not on the operator list`,
    });
    expect((acceptManifest(sign(st, OUTSIDER), freshLists()) as any).ok).toBe(false);
  });
  it('refuses no signatures, a key twice, and a signature that does not verify', () => {
    const st = edited(() => {});
    expect(acceptManifest(serializeManifest({ statement: st, signatures: [] }), freshLists())).toEqual({ ok: false, reason: 'no signatures' });
    expect((acceptManifest(sign(st, A, A), freshLists()) as any).reason).toBe(`${A.pub} signed twice`);
    const m = parseManifest(sign(st, A, B));
    // B's signature over another statement, under B's key.
    m.signatures[1].der = parseManifest(sign(edited((s) => { s[100] ^= 1; }), B)).signatures[0].der;
    expect((acceptManifest(serializeManifest(m), freshLists()) as any).reason).toBe(`the signature by ${B.pub} is not valid`);
  });
  it('refuses a high-S twin of a valid signature', () => {
    const m = parseManifest(R('PC'));
    const sig = secp256k1.Signature.fromBytes(m.signatures[1].der, 'der');
    m.signatures[1].der = new secp256k1.Signature(sig.r, secp256k1.Point.Fn.ORDER - sig.s).toBytes('der');
    expect((acceptManifest(serializeManifest(m), regtestLists()) as any).reason).toBe(`the signature by ${C} is not valid`);
  });
  it('refuses trailing bytes and oversize bodies before anything else', () => {
    expect((acceptManifest(Uint8Array.from([...R('PC'), 0]), regtestLists()) as any).reason).toBe('1 bytes after the last signature');
    expect((acceptManifest(new Uint8Array(64 * 1024 + 1), regtestLists()) as any).reason).toBe('65537 bytes is more than a manifest may be');
  });
});

describe('merging co-signatures', () => {
  it('keeps one signature per key, stored ones first', () => {
    const merged = mergeSignatures(parseManifest(R('P')), parseManifest(R('CP')));
    expect(merged.added).toEqual([C]);
    expect(Buffer.from(serializeManifest(merged.manifest)).equals(Buffer.from(R('PC')))).toBe(true);
    expect(mergeSignatures(merged.manifest, parseManifest(R('P'))).added).toEqual([]);
  });
});

type RecOpts = {
  stored?: boolean; ago?: number; height?: number; hash?: string; confirmedAgo?: number;
  dissent?: boolean; coins?: number; hs3?: string; fileHash?: string;
};
/** A record as the routes keep it, from a regtest vector, with fields changed as asked. */
const rec = (vector: string, o: RecOpts = {}) => {
  const a: any = acceptManifest(R(vector), regtestLists());
  let r: any = newRecord(a, iso(o.ago ?? 0));
  r = {
    ...r,
    ...(o.height ? { height: o.height } : {}),
    ...(o.hash ? { statement_hash: o.hash } : {}),
    ...(o.dissent ? { dissent: true, file_size: 0, file_hash: '00'.repeat(32) } : {}),
    ...(o.coins !== undefined ? { coins: o.coins } : {}),
    ...(o.hs3 ? { hash_serialized_3: o.hs3 } : {}),
    ...(o.fileHash ? { file_hash: o.fileHash } : {}),
  };
  if (o.stored) {
    r = {
      ...r,
      file: { state: 'stored', url: 'https://x.public.blob.vercel-storage.com/f.dat', sha256: 'b2'.repeat(32), stored_at: iso(0) },
      public_manifest: { url: 'https://x.public.blob.vercel-storage.com/m.manifest', sha256: 'aa'.repeat(32), size: 440, signatures: r.signers.length },
      confirmed_at: iso(o.confirmedAgo ?? 0),
    };
  }
  return r;
};

describe('latest points only at what every node can load', () => {
  const lists = regtestLists();
  it('one operator, even with its file stored: nothing', () => {
    expect(recordOperators(rec('P', { stored: true }), lists)).toEqual(['producer']);
    expect(chooseLatest([rec('P', { stored: true })], 'regtest', lists)).toBeNull();
  });
  it('two operators without the file: nothing', () => {
    expect(chooseLatest([rec('PC')], 'regtest', lists)).toBeNull();
  });
  it('two keys of one operator count once', () => {
    const one = { main: [], regtest: parseRegtestOperators(`producer=${P},${C}`), pins: { main: [], regtest: [P] } };
    expect(isConfirmed(rec('PC', { stored: true }), one)).toBe(false);
  });
  it('two operators, one of them pinned, with the file stored: the pointer, in the app\'s contract', () => {
    const r = rec('PC', { stored: true });
    expect(chooseLatest([r, rec('P', { stored: true, height: 200, hash: 'ff'.repeat(32) })], 'regtest', lists)).toBe(r);
    expect(Object.keys(latestView(r, lists))).toEqual([
      'version', 'chain', 'height', 'block_hash', 'statement_hash', 'manifest_url', 'manifest_size',
      'manifest_sha256', 'file_url', 'file_size', 'file_sha256', 'file_hash', 'operators', 'confirmed_at',
    ]);
    expect(latestView(r, lists)).toMatchObject({ version: 1, chain: 'regtest', height: 100, file_size: 8055, operators: ['producer', 'confirmer'] });
  });
  it('a newer two-operator statement without a pinned signature is skipped for an older one with one', () => {
    const low = rec('PC', { stored: true });
    const high = rec('CD', { stored: true, height: 200, hash: '02'.repeat(32) });
    expect(recordOperators(high, lists)).toEqual(['confirmer', 'third']);
    expect(isConfirmed(high, lists)).toBe(false);
    expect(chooseLatest([low, high], 'regtest', lists)).toBe(low);
    expect(pendingView([low, high], 'regtest', NOW, lists).statements.map((s: any) => [s.height, s.confirmed])).toEqual([[200, false], [100, true]]);
  });
  it('a key taken off the list stops counting at once', () => {
    const shrunk = { main: [], regtest: parseRegtestOperators(`producer=${P}`), pins: { main: [], regtest: [P] } };
    expect(chooseLatest([rec('PC', { stored: true })], 'regtest', shrunk)).toBeNull();
  });
  it('a pin taken away stops counting at once', () => {
    const unpinned = { ...lists, pins: { main: lists.pins.main, regtest: [] } };
    expect(chooseLatest([rec('PC', { stored: true })], 'regtest', unpinned)).toBeNull();
  });
  it('a dissent is never pointed at, however many sign it', () => {
    const d = rec('PCD', { stored: true, dissent: true });
    expect(isConfirmed(d, lists)).toBe(false);
    expect(chooseLatest([d], 'regtest', lists)).toBeNull();
  });
  it('two confirmed statements that differ only in their files: the one confirmed first, and no dispute', () => {
    const first = rec('PC', { stored: true, confirmedAgo: 2000 });
    const second = rec('PCD', { stored: true, confirmedAgo: 1000, hash: '03'.repeat(32), fileHash: 'ee'.repeat(32) });
    expect(disputeAt([first, second], 'regtest', 100, lists)).toBeNull();
    expect(chooseLatest([second, first], 'regtest', lists)).toBe(first);
  });
  it('only the asked chain', () => {
    expect(chooseLatest([rec('PC', { stored: true })], 'main', lists)).toBeNull();
  });
});

describe('disputes', () => {
  const lists = regtestLists();
  it('writes the owner\'s alert as the design words it', () => {
    const onMain = (over: any = {}) => ({
      version: 1, chain: 'main', height: 233_800, statement_hash: '05'.repeat(32),
      block_hash: 'aa'.repeat(32), hash_serialized_3: 'bb'.repeat(32), coins: 140_731, chain_tx: 328_195,
      file_size: 9_045_522, file_hash: 'cc'.repeat(32), dissent: false, signers: [MENDE, JPP], first_seen: iso(2000), ...over,
    });
    const a = onMain();
    const b = onMain({
      statement_hash: '06'.repeat(32), block_hash: 'dd'.repeat(32), file_size: 0, file_hash: '00'.repeat(32),
      dissent: true, signers: [ALEKS_2], first_seen: iso(1000),
    });
    const d: any = disputeAt([b, a], 'main', 233_800, mainLists);
    expect(d.statements).toEqual([a, b]);
    expect(d.differ).toEqual(['block_hash']);
    expect(alertText(d, mainLists)).toBe([
      'easyNode snapshots: block 233,800 is disputed. Fast-forward is off for every node until you clear it.',
      `1: block ${'aa'.repeat(32)}, UTXO hash ${'bb'.repeat(32)}, 140,731 coins, 328,195 transactions. Signed by Mende and jpp.`,
      `2: block ${'dd'.repeat(32)}, UTXO hash ${'bb'.repeat(32)}, 140,731 coins, 328,195 transactions. Dissent from Aleksander.`,
      'They differ in: block hash.',
      'Check both nodes, then clear it: node scripts/clear-snapshot-dispute.mjs 233800',
    ].join('\n'));
  });
  it('two listed statements that differ in chain fields are a dispute; the command names the test chain', () => {
    const a = rec('P', { ago: 2000 });
    const b = rec('C', { ago: 1000, hash: '04'.repeat(32), coins: 102, hs3: 'ab'.repeat(32) });
    const d: any = disputeAt([a, b], 'regtest', 100, lists);
    expect(d.differ).toEqual(['hash_serialized_3', 'coins']);
    expect(alertText(d, lists).split('\n')).toEqual([
      'easyNode snapshots: block 100 is disputed. Fast-forward is off for every node until you clear it.',
      `1: block ${BLOCK}, UTXO hash ${HS3}, 101 coins, 101 transactions. Signed by producer.`,
      `2: block ${BLOCK}, UTXO hash ${'ab'.repeat(32)}, 102 coins, 101 transactions. Signed by confirmer.`,
      'They differ in: UTXO hash and coin count.',
      'Check both nodes, then clear it: node scripts/clear-snapshot-dispute.mjs 100 --chain regtest',
    ]);
    expect(disputedHeights([a, b], 'regtest', lists)).toEqual([100]);
    expect(liveDisputes([a, b], 'main', lists)).toEqual([]);
  });
  it('a difference in the file fields alone is not a dispute', () => {
    const a = rec('P');
    const b = rec('C', { hash: '07'.repeat(32), fileHash: 'ee'.repeat(32) });
    expect(disputeAt([a, b], 'regtest', 100, lists)).toBeNull();
    expect(disputedHeights([a, b], 'regtest', lists)).toEqual([]);
  });
  it('a dissent disputes a statement whose chain fields differ, and not one whose agree', () => {
    const s = rec('P');
    expect(disputeAt([s, rec('C', { hash: '08'.repeat(32), dissent: true })], 'regtest', 100, lists)).toBeNull();
    const d: any = disputeAt([s, rec('C', { hash: '09'.repeat(32), dissent: true, coins: 99 })], 'regtest', 100, lists);
    expect(d.differ).toEqual(['coins']);
    expect(alertText(d, lists)).toContain('99 coins, 101 transactions. Dissent from confirmer.');
  });
  it('a statement no listed operator signed disputes nothing', () => {
    const a = rec('P');
    const b = rec('C', { hash: '0a'.repeat(32), coins: 5 });
    const shrunk = { ...lists, regtest: parseRegtestOperators(`producer=${P}`) };
    expect(disputeAt([a, b], 'regtest', 100, lists)).not.toBeNull();
    expect(disputeAt([a, b], 'regtest', 100, shrunk)).toBeNull();
  });
  it('a recorded dispute stands until the owner clears it, beside the ones the rule finds', () => {
    const a = rec('P');
    const b = rec('C', { hash: '0b'.repeat(32), coins: 5 });
    const recorded = [{ chain: 'regtest', height: 700 }, { chain: 'main', height: 800 }];
    expect(disputedHeights([a, b], 'regtest', lists, recorded)).toEqual([100, 700]);
    expect(disputedHeights([], 'regtest', lists, recorded)).toEqual([700]);
    expect(disputedHeights([a, b], 'main', lists, recorded)).toEqual([800]);
  });
  it('the record keeps its alert and the time it opened; the answer has exactly the contract\'s keys', () => {
    const a = rec('P', { ago: 2000 });
    const b = rec('C', { ago: 1000, hash: '0c'.repeat(32), coins: 5 });
    const r0 = newDispute(disputeAt([a, b], 'regtest', 100, lists), lists, iso(5000));
    expect(r0).toMatchObject({ version: 1, chain: 'regtest', height: 100, differ: ['coins'], opened_at: iso(5000) });
    expect(r0.statements.map((s: any) => [s.signers, s.dissent])).toEqual([[['producer'], false], [['confirmer'], false]]);
    const r1 = refreshDispute(r0, disputeAt([a, { ...b, signers: [C, D] }], 'regtest', 100, lists), lists);
    expect(r1.statements[1].signers).toEqual(['confirmer', 'third']);
    expect([r1.alert, r1.opened_at]).toEqual([r0.alert, r0.opened_at]);
    const view = disputesView([r1, { ...r1, chain: 'main', height: 900 }], 'regtest');
    expect(Object.keys(view)).toEqual(['disputes']);
    expect(view.disputes).toHaveLength(1);
    expect(Object.keys(view.disputes[0])).toEqual(['height', 'statements', 'differ', 'alert', 'opened_at']);
    expect(Object.keys(view.disputes[0].statements[0])).toEqual([
      'statement_hash', 'block_hash', 'hash_serialized_3', 'coins', 'chain_tx', 'file_size', 'file_hash', 'signers', 'dissent',
    ]);
  });
  it('joins names the way the app does', () => {
    expect([joinNames([]), joinNames(['Mende']), joinNames(['Mende', 'jpp']), joinNames(['Mende', 'Aleksander', 'jpp'])])
      .toEqual(['', 'Mende', 'Mende and jpp', 'Mende, Aleksander and jpp']);
  });
});

describe('pending, storage and parts', () => {
  const lists = regtestLists();
  it('lists statements and dissents of the last seven days, newest height first, with who signed and what is disputed', () => {
    const view = pendingView(
      [rec('P', { ago: 8 * DAY }), rec('PC', { ago: DAY, height: 200 }), rec('C', { ago: 0 }), rec('C', { ago: 0, height: 300, hash: '0d'.repeat(32), dissent: true })],
      'regtest', NOW, lists, [{ chain: 'regtest', height: 200 }],
    );
    expect(view.statements.map((s: any) => [s.height, s.operators, s.dissent, s.disputed])).toEqual([
      [300, ['confirmer'], true, false], [200, ['producer', 'confirmer'], false, true], [100, ['confirmer'], false, false],
    ]);
    expect(Object.keys(view.statements[1])).toEqual([
      'statement_hash', 'height', 'block_hash', 'manifest_hex', 'signers', 'operators', 'file', 'first_seen', 'confirmed', 'disputed', 'dissent',
    ]);
    expect(view.statements[1]).toMatchObject({ file: 'missing', confirmed: false });
    expect(view.statements[1].manifest_hex).toBe(Buffer.from(R('PC')).toString('hex'));
  });
  it('keeps the five newest confirmed pairs and files for the ten newest pending statements', () => {
    const confirmed = [1, 2, 3, 4, 5, 6, 7].map((i) => rec('PC', { stored: true, ago: 30 * DAY, height: 100 * i, hash: `c${i}`.padStart(64, '0') }));
    const pending = Array.from({ length: 12 }, (_, i) => rec('C', { stored: true, ago: DAY, height: 1000 + 100 * i, hash: `p${i}`.padStart(64, '0') }));
    const stale = rec('C', { ago: 9 * DAY, height: 5000, hash: 'aa'.repeat(32) });
    const plan = storagePlan([...confirmed, ...pending, stale], NOW, lists);
    expect(plan.dropRecords.map((r: any) => r.height).sort(num)).toEqual([100, 200, 5000]);
    expect(plan.dropFiles.map((r: any) => r.height).sort(num)).toEqual([1000, 1100]);
  });
  it('lets go of the file of a pending pair at or below the newest confirmed height, but of nothing at a disputed height', () => {
    const top = rec('PC', { stored: true, height: 500, hash: 'e1'.padStart(64, '0') });
    const below = rec('C', { stored: true, height: 300, hash: 'e2'.padStart(64, '0') });
    const twin = rec('C', { stored: true, height: 500, hash: 'e3'.padStart(64, '0'), fileHash: 'ee'.repeat(32) });
    const a = rec('P', { stored: true, ago: 30 * DAY, height: 200, hash: 'e4'.padStart(64, '0') });
    const b = rec('C', { stored: true, ago: 30 * DAY, height: 200, hash: 'e5'.padStart(64, '0'), coins: 5 });
    const plan = storagePlan([top, below, twin, a, b], NOW, lists);
    expect(plan.dropFiles.map((r: any) => r.statement_hash).sort()).toEqual([below.statement_hash, twin.statement_hash].sort());
    expect(plan.dropRecords).toEqual([]);
    // The rule no longer sees the dispute (the facts now agree), but it is recorded: still kept.
    const plan2 = storagePlan([top, a, { ...b, coins: 101 }], NOW, lists, [{ chain: 'regtest', height: 200 }]);
    expect([plan2.dropRecords, plan2.dropFiles]).toEqual([[], []]);
  });
  it('splits a file into parts of the size the routes take', () => {
    const plan = partPlan(9_045_522, 4 * 1024 * 1024);
    expect(plan.parts).toBe(3);
    expect([plan.sizeOf(1), plan.sizeOf(2), plan.sizeOf(3), plan.sizeOf(4), plan.sizeOf(0)]).toEqual([4_194_304, 4_194_304, 656_914, -1, -1]);
    expect(partPlan(8055, 4096).parts).toBe(2);
    expect([CHAINS.main.grid, CHAINS.regtest.grid, CHAINS.main.shielded]).toEqual([100, 100, '94343b766b39c0ea2d92d83323f77b5ccc5e775d99b34b01f5fa6400f2354541']);
  });
  it('withManifest updates signers', () => {
    const r = withManifest(rec('P'), parseManifest(R('PC')), iso(0));
    expect(r.signers).toEqual([P, C]);
  });
});
````

- [ ] **Step 2: Run it to see it fail**

Run: `cd site && npx vitest run tests/unit/snapshotRendezvous.test.ts`
Expected: FAIL, `Cannot find module '../../src/lib/snapshotRendezvous.mjs'`.

- [ ] **Step 3: Write the module**

Create `site/src/lib/snapshotRendezvous.mjs`:

````js
// The meeting point for confirmed chain snapshots: the rules, with no I/O.
//
// docs/decisions/2026-09-29-every-node-starts-near-the-tip.md, sections 6 and
// 6a, in easyNode's repository. Producers send a signed statement and its
// file; confirmers fetch what is waiting, check it against their own node's
// diary, and send back one signature, or a signed dissent when their diary
// disagrees; every node reads `latest`. NOTHING here is trusted by the app: it
// re-checks every signature against its own compiled list, its own node and
// the file's hash. The website only filters, stores and points, so the worst
// it can do is point at nothing.
//
// How deep a block must be before anyone sends or signs it (144 blocks) is
// the producers' and confirmers' rule, checked on their own nodes. The
// website sees no chain and checks no depth.
import {
  parseManifest, serializeManifest, statementFields, statementDigest, displayHex, isDissent,
  signatureIsValid, ManifestError, MAX_MANIFEST_BYTES, MAX_FILE_BYTES, MIN_CHUNK, MAX_CHUNK,
} from './snapshotManifest.mjs';
import { operatorOf, distinctOperators } from './snapshotOperators.mjs';

/** Snapshots are taken only at multiples of 100 (the design, section 2). */
export const GRID = 100;
/**
 * Mainnet's shielded commitment near the tip. The pool closed at 199,300 and
 * every compiled snapshot since carries this value (btxd 84b998b4,
 * src/kernel/chainparams.cpp:1278, the 219,000 entry).
 */
export const MAIN_SHIELDED = '94343b766b39c0ea2d92d83323f77b5ccc5e775d99b34b01f5fa6400f2354541';

/**
 * Each chain's genesis hash, replay context (btxd v0.34.9, the same in
 * v0.34.12), shielded commitment and grid: the values easyNode compiles.
 * Regtest's shielded pool is open, so its commitment moves with the chain and
 * is only checked for being there.
 */
export const CHAINS = {
  main: {
    genesis: '75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601',
    replayContext: '32ad5c2e148149752a312561dc0b6879c9cc41fdf4bc09edcdd5e2bd09af7188',
    shielded: MAIN_SHIELDED,
    grid: GRID,
  },
  regtest: {
    genesis: '521ad0951ed299e9c56aeb7db8188972772067560351b8e55adf71dbed532360',
    replayContext: '9ed2add89d64a66015d6c4b2a746115c00c78503088fdde49e15ddcafed8a577',
    shielded: null,
    grid: GRID,
  },
};
/** Statements and dissents are listed for confirmers this long after they first arrive. */
export const PENDING_DAYS = 7;
/** Confirmed pairs kept per chain, newest first. */
export const KEEP_CONFIRMED = 5;
/** Pending statements per chain whose files are kept, newest first. */
export const KEEP_PENDING_FILES = 10;
/** The refusal for a height the owner cleared. A code the app matches, not a sentence. */
export const CLOSED_HEIGHT = 'closed-height';
/** The chain fields a dispute compares, with the words the owner's alert uses for them. */
export const CHAIN_FIELDS = [
  ['block_hash', 'block hash'],
  ['hash_serialized_3', 'UTXO hash'],
  ['coins', 'coin count'],
  ['chain_tx', 'transaction count'],
];
const FIELD_WORDS = Object.fromEntries(CHAIN_FIELDS);
const DAY_MS = 24 * 3600 * 1000;
const ZERO = /^0+$/;

/** @param {string} chainIdHex */
export function chainOf(chainIdHex) {
  for (const [name, c] of Object.entries(CHAINS)) if (c.genesis === chainIdHex) return name;
  return null;
}

const refuse = (reason) => ({ ok: false, reason });
const listOf = (lists, chain) => (chain === 'main' ? lists.main : chain === 'regtest' ? lists.regtest : null);
const pinsOf = (lists, chain) => lists.pins?.[chain] || [];
const ms = (iso) => Date.parse(iso || '') || 0;
const byHash = (a, b) => (a.statement_hash < b.statement_hash ? -1 : a.statement_hash > b.statement_hash ? 1 : 0);
const firstSeen = (a, b) => ms(a.first_seen) - ms(b.first_seen) || byHash(a, b);
/** Highest first; at one height, the one confirmed (or else seen) first. */
const newestFirst = (a, b) => b.height - a.height || ms(a.confirmed_at || a.first_seen) - ms(b.confirmed_at || b.first_seen) || byHash(a, b);
const grouped = (n) => String(n).replace(/\B(?=(\d{3})+(?!\d))/g, ',');

/**
 * Whether the website stores this manifest: section 6's rules, every one.
 * Every signature valid (strict DER, low S) and from a key on the chain's
 * list; no key twice; the chain's genesis, replay context and shielded
 * commitment; the height on the grid; either a file of at most 64 MiB in the
 * engine's chunk geometry, or all four file fields zero (a dissent). Whether
 * the height is closed needs the store, so the route checks that next.
 * @param {Uint8Array} bytes
 * @param {{ main: any[], regtest: any[] | null, pins?: { main: string[], regtest: string[] } }} lists
 */
export function acceptManifest(bytes, lists) {
  if (bytes.length > MAX_MANIFEST_BYTES) return refuse(`${bytes.length} bytes is more than a manifest may be`);
  let manifest;
  try {
    manifest = parseManifest(bytes);
  } catch (e) {
    if (e instanceof ManifestError) return refuse(e.message);
    throw e;
  }
  const f = statementFields(manifest.statement);
  const chain = chainOf(f.chainId);
  const list = chain ? listOf(lists, chain) : null;
  if (!chain || !list) return refuse(`chain ${f.chainId} is not one this website takes`);
  const rules = CHAINS[chain];
  if (f.replayContext !== rules.replayContext) return refuse('the replay context is not this engine\'s');
  if (ZERO.test(f.shielded)) return refuse('the shielded commitment is empty');
  if (rules.shielded && f.shielded !== rules.shielded) return refuse('the shielded commitment is not this chain\'s');
  const dissent = isDissent(f);
  if (!dissent) {
    if (
      f.fileSize <= 0 || ZERO.test(f.fileHash) || f.chunkSize < MIN_CHUNK || f.chunkSize > MAX_CHUNK ||
      f.chunkCount !== 1 + Math.floor((f.fileSize - 1) / f.chunkSize)
    ) return refuse('the file size and chunks do not add up');
    if (f.fileSize > MAX_FILE_BYTES) return refuse(`a ${f.fileSize}-byte file is more than this website stores`);
  }
  if (f.height <= 0 || f.height % rules.grid !== 0) return refuse(`height ${f.height} is not a multiple of ${rules.grid}`);
  if (manifest.signatures.length === 0) return refuse('no signatures');
  const digest = statementDigest(manifest.statement);
  const seen = new Set();
  for (const s of manifest.signatures) {
    if (seen.has(s.key)) return refuse(`${s.key} signed twice`);
    seen.add(s.key);
    if (!operatorOf(list, s.key)) return refuse(`key ${s.key} is not on the operator list`);
    if (!signatureIsValid(digest, s.key, s.der)) return refuse(`the signature by ${s.key} is not valid`);
  }
  return { ok: true, chain, fields: f, hash: displayHex(digest), manifest, dissent };
}

/**
 * One signature per key: the stored ones first, in their order, then the
 * new ones in the order they came.
 */
export function mergeSignatures(stored, incoming) {
  const have = new Set(stored.signatures.map((s) => s.key));
  const added = incoming.signatures.filter((s) => !have.has(s.key));
  return {
    manifest: { statement: stored.statement, signatures: [...stored.signatures, ...added] },
    added: added.map((s) => s.key),
  };
}

const hexOf = (b) => Buffer.from(b).toString('hex');

/** The record kept for a statement or dissent the website took for the first time. */
export function newRecord(accepted, nowIso) {
  const f = accepted.fields;
  return {
    version: 1,
    chain: accepted.chain,
    statement_hash: accepted.hash,
    height: f.height,
    block_hash: f.blockHash,
    hash_serialized_3: f.hashSerialized,
    coins: f.coins,
    chain_tx: f.chainTx,
    file_size: f.fileSize,
    file_hash: f.fileHash,
    dissent: accepted.dissent === true,
    manifest_hex: hexOf(serializeManifest(accepted.manifest)),
    signers: accepted.manifest.signatures.map((s) => s.key),
    first_seen: nowIso,
    updated_at: nowIso,
    file: { state: 'missing' },
    public_manifest: null,
    confirmed_at: null,
  };
}

/** The record with a merged manifest. */
export function withManifest(record, manifest, nowIso) {
  return {
    ...record,
    manifest_hex: hexOf(serializeManifest(manifest)),
    signers: manifest.signatures.map((s) => s.key),
    updated_at: nowIso,
  };
}

/** The stored manifest, parsed. */
export function recordManifest(record) {
  return parseManifest(new Uint8Array(Buffer.from(record.manifest_hex, 'hex')));
}

/** The operators behind a record's signatures, under TODAY's list: a key taken off the list stops counting at once. */
export function recordOperators(record, lists) {
  const list = listOf(lists, record.chain);
  return list ? distinctOperators(list, record.signers) : [];
}

/** At least one signature is from a key every node pins, so a node can load it (the design, section 7, step 1). */
export function hasPinnedSigner(record, lists) {
  const pins = pinsOf(lists, record.chain);
  return record.signers.some((k) => pins.includes(k));
}

/** Not a dissent, two different operators, a pinned key among them, and its file stored and checked. */
export function isConfirmed(record, lists) {
  return (
    record.dissent !== true &&
    record.file?.state === 'stored' &&
    recordOperators(record, lists).length >= 2 &&
    hasPinnedSigner(record, lists)
  );
}

/**
 * The newest confirmed statement of a chain whose public manifest is written.
 * Two confirmed statements at one height differ only in their files (a
 * difference in chain facts is a dispute, and then the route serves none):
 * the one confirmed first is served.
 */
export function chooseLatest(records, chain, lists) {
  const ready = records.filter((r) => r.chain === chain && isConfirmed(r, lists) && r.public_manifest).sort(newestFirst);
  return ready[0] || null;
}

/**
 * GET /api/snapshots/latest, the contract easyNode's
 * attested_snapshot::ConfirmedPointer reads (crates/btx-core/tests/fixtures/
 * confirmed_snapshot/latest.json).
 */
export function latestView(record, lists) {
  return {
    version: 1,
    chain: record.chain,
    height: record.height,
    block_hash: record.block_hash,
    statement_hash: record.statement_hash,
    manifest_url: record.public_manifest.url,
    manifest_size: record.public_manifest.size,
    manifest_sha256: record.public_manifest.sha256,
    file_url: record.file.url,
    file_size: record.file_size,
    file_sha256: record.file.sha256,
    file_hash: record.file_hash,
    operators: recordOperators(record, lists),
    confirmed_at: record.confirmed_at,
  };
}

const differingFields = (a, b) => CHAIN_FIELDS.filter(([k]) => a[k] !== b[k]).map(([k]) => k);

/**
 * Section 6a's rule at one height: the statements and dissents stored there,
 * each with at least one listed operator's signature, disagree in a chain
 * field (block hash, UTXO hash, coin count, transaction count). File fields
 * alone never dispute. Returns the statements in the order they arrived and
 * the fields that differ, or null.
 */
export function disputeAt(records, chain, height, lists) {
  const here = records
    .filter((r) => r.chain === chain && r.height === height && recordOperators(r, lists).length > 0)
    .sort(firstSeen);
  const differ = new Set();
  for (let i = 0; i < here.length; i++) {
    for (let j = i + 1; j < here.length; j++) differingFields(here[i], here[j]).forEach((k) => differ.add(k));
  }
  if (differ.size === 0) return null;
  return { chain, height, statements: here, differ: CHAIN_FIELDS.map(([k]) => k).filter((k) => differ.has(k)) };
}

/** Every height of a chain where the rule finds a dispute now, ascending. */
export function liveDisputes(records, chain, lists) {
  const heights = [...new Set(records.filter((r) => r.chain === chain).map((r) => r.height))].sort((a, b) => a - b);
  return heights.map((h) => disputeAt(records, chain, h, lists)).filter(Boolean);
}

/**
 * Every height of a chain where a dispute stands: the recorded ones, which
 * stand until the owner clears them, and any the rule finds now. Ascending.
 */
export function disputedHeights(records, chain, lists, recorded = []) {
  const set = new Set(recorded.filter((d) => d.chain === chain).map((d) => d.height));
  for (const d of liveDisputes(records, chain, lists)) set.add(d.height);
  return [...set].sort((a, b) => a - b);
}

/** "Mende", "Mende and jpp", "Mende, Aleksander and jpp". */
export function joinNames(names) {
  if (names.length <= 1) return names.join('');
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
}

/** The owner's alert, section 6a's text filled in. Orca delivers it; this only writes it. */
export function alertText(dispute, lists) {
  const lines = [
    `easyNode snapshots: block ${grouped(dispute.height)} is disputed. Fast-forward is off for every node until you clear it.`,
  ];
  dispute.statements.forEach((r, i) => {
    const who = joinNames(recordOperators(r, lists));
    lines.push(
      `${i + 1}: block ${r.block_hash}, UTXO hash ${r.hash_serialized_3}, ${grouped(r.coins)} coins, ` +
        `${grouped(r.chain_tx)} transactions. ${r.dissent ? 'Dissent from' : 'Signed by'} ${who}.`,
    );
  });
  lines.push(`They differ in: ${joinNames(dispute.differ.map((k) => FIELD_WORDS[k]))}.`);
  const chainArg = dispute.chain === 'main' ? '' : ` --chain ${dispute.chain}`;
  lines.push(`Check both nodes, then clear it: node scripts/clear-snapshot-dispute.mjs ${dispute.height}${chainArg}`);
  return lines.join('\n');
}

const disputeStatement = (r, lists) => ({
  statement_hash: r.statement_hash,
  block_hash: r.block_hash,
  hash_serialized_3: r.hash_serialized_3,
  coins: r.coins,
  chain_tx: r.chain_tx,
  file_size: r.file_size,
  file_hash: r.file_hash,
  signers: recordOperators(r, lists),
  dissent: r.dissent === true,
});

/** The dispute record, written the first time a height is disputed. */
export function newDispute(dispute, lists, nowIso) {
  return {
    version: 1,
    chain: dispute.chain,
    height: dispute.height,
    statements: dispute.statements.map((r) => disputeStatement(r, lists)),
    differ: dispute.differ,
    alert: alertText(dispute, lists),
    opened_at: nowIso,
  };
}

/** The record brought up to date: the statements and fields as they stand now. The alert and the time it opened never change. */
export function refreshDispute(record, dispute, lists) {
  return { ...record, statements: dispute.statements.map((r) => disputeStatement(r, lists)), differ: dispute.differ };
}

/** GET /api/snapshots/disputes. */
export function disputesView(disputeRecords, chain) {
  return {
    disputes: disputeRecords
      .filter((d) => d.chain === chain)
      .sort((a, b) => a.height - b.height)
      .map(({ height, statements, differ, alert, opened_at }) => ({ height, statements, differ, alert, opened_at })),
  };
}

const young = (r, nowMs) => nowMs - ms(r.first_seen) <= PENDING_DAYS * DAY_MS;

/** GET /api/snapshots/pending: what confirmers read, newest height first. */
export function pendingView(records, chain, nowMs, lists, recorded = []) {
  const disputed = new Set(disputedHeights(records, chain, lists, recorded));
  const statements = records
    .filter((r) => r.chain === chain && young(r, nowMs))
    .sort((a, b) => b.height - a.height || firstSeen(a, b))
    .map((r) => ({
      statement_hash: r.statement_hash,
      height: r.height,
      block_hash: r.block_hash,
      manifest_hex: r.manifest_hex,
      signers: r.signers,
      operators: recordOperators(r, lists),
      file: r.file?.state || 'missing',
      first_seen: r.first_seen,
      confirmed: isConfirmed(r, lists),
      disputed: disputed.has(r.height),
      dissent: r.dissent === true,
    }));
  return { version: 1, chain, statements };
}

/**
 * What storage lets go of, per chain (section 6). Kept: the five newest
 * confirmed pairs; the files of the ten newest pending statements above the
 * newest confirmed height (one at or below it can never be served); every
 * record first seen in the last seven days; everything at a disputed height,
 * until the owner clears it. `dropRecords` go with all their blobs;
 * `dropFiles` keep their record and lose their file.
 */
export function storagePlan(records, nowMs, lists, recorded = []) {
  const keepRecord = new Set();
  const keepFile = new Set();
  for (const chain of Object.keys(CHAINS)) {
    const mine = records.filter((r) => r.chain === chain);
    const held = new Set(disputedHeights(mine, chain, lists, recorded));
    const confirmed = mine.filter((r) => isConfirmed(r, lists)).sort(newestFirst);
    const top = confirmed.length ? confirmed[0].height : 0;
    for (const r of mine) {
      if (held.has(r.height)) {
        keepRecord.add(r);
        keepFile.add(r);
      }
      if (young(r, nowMs)) keepRecord.add(r);
    }
    for (const r of confirmed.slice(0, KEEP_CONFIRMED)) {
      keepRecord.add(r);
      keepFile.add(r);
    }
    mine
      .filter((r) => keepRecord.has(r) && r.dissent !== true && !isConfirmed(r, lists) && r.height > top)
      .sort(newestFirst)
      .slice(0, KEEP_PENDING_FILES)
      .forEach((r) => keepFile.add(r));
  }
  const hasFile = (r) => r.file?.state === 'stored' || r.file?.state === 'uploading';
  return {
    dropRecords: records.filter((r) => !keepRecord.has(r)),
    dropFiles: records.filter((r) => keepRecord.has(r) && !keepFile.has(r) && hasFile(r)),
  };
}

/** How a file of `fileSize` bytes goes up in parts of `partBytes`: part n (from 1) and its exact size. */
export function partPlan(fileSize, partBytes) {
  const parts = Math.max(1, Math.ceil(fileSize / partBytes));
  return {
    parts,
    sizeOf: (n) => (n < 1 || n > parts ? -1 : n < parts ? partBytes : fileSize - (parts - 1) * partBytes),
  };
}
````

- [ ] **Step 4: Run the test to see it pass**

Run: `cd site && npx vitest run tests/unit/snapshotRendezvous.test.ts`
Expected: `Tests  45 passed (45)` (derived, not run: 21 intake, 1 merge, 10 `latest`, 8 disputes, 5 pending, storage and parts).

- [ ] **Step 5: Commit**

````bash
git add site/src/lib/snapshotRendezvous.mjs site/tests/unit/snapshotRendezvous.test.ts
git commit -m "site: the snapshot meeting point's rules: what is stored, merged, confirmed, disputed and kept" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 4: The store (`snapshotStore.mjs`)

**Files:**
- Create: `site/src/lib/snapshotStore.mjs`
- Test: `site/tests/unit/snapshotStore.test.ts`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces (Tasks 5, 6): `class StoreConflict`; a store is `{kind, getPrivate(path) -> {bytes, etag} | null, putPrivate(path, bytes, {etag}) -> {etag}, putPublic(path, bytes, contentType?) -> {url}, readPublic(path) -> bytes | null, list(prefix) -> [path], del(paths)}` where `etag: string` replaces only an unchanged blob, `etag: null` creates only, `etag` absent overwrites, and a lost race throws `StoreConflict`; `folderStore(dir, publicBase)`; `blobStore(sdk)`; `storeFromEnv(env) -> store | null` (`SNAPSHOT_STORE_DIR` only when `VERCEL` is unset; Blob when `BLOB_READ_WRITE_TOKEN` or `BLOB_STORE_ID` is set; else `null`). `readPublic` is for the owner's clear, which copies public blobs into the private archive.

- [ ] **Step 1: Write the failing test**

Create `site/tests/unit/snapshotStore.test.ts`. The Blob half runs against a fake SDK and checks exactly what is asked of Vercel: private reads past the cache (`useCache: false`), conditional writes (`ifMatch`), create-only (`allowOverwrite: false`), public immutable files, public reads for the archive, every page of a listing.

````ts
import { describe, it, expect } from 'vitest';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { folderStore, blobStore, storeFromEnv, StoreConflict } from '../../src/lib/snapshotStore.mjs';

const bytes = (s: string) => new TextEncoder().encode(s);

describe('folderStore', () => {
  const store = () => folderStore(mkdtempSync(join(tmpdir(), 'snapstore-')), 'http://127.0.0.1:29650/blob');
  it('creates only once, replaces only with the etag it read, and overwrites when asked', async () => {
    const s = store();
    const first = await s.putPrivate('snapshots/records/a.json', bytes('1'), { etag: null });
    await expect(s.putPrivate('snapshots/records/a.json', bytes('x'), { etag: null })).rejects.toBeInstanceOf(StoreConflict);
    const second = await s.putPrivate('snapshots/records/a.json', bytes('2'), { etag: first.etag });
    await expect(s.putPrivate('snapshots/records/a.json', bytes('3'), { etag: first.etag })).rejects.toBeInstanceOf(StoreConflict);
    expect(new TextDecoder().decode((await s.getPrivate('snapshots/records/a.json'))!.bytes)).toBe('2');
    expect((await s.getPrivate('snapshots/records/a.json'))!.etag).toBe(second.etag);
    await s.putPrivate('snapshots/records/a.json', bytes('4'));
    expect(await s.getPrivate('snapshots/records/missing.json')).toBeNull();
  });
  it('serves public blobs under its base, lists by prefix and deletes', async () => {
    const s = store();
    const { url } = await s.putPublic('snapshots/100/h.dat', bytes('file'));
    expect(url).toBe('http://127.0.0.1:29650/blob/snapshots/100/h.dat');
    expect(new TextDecoder().decode((await s.readPublic('snapshots/100/h.dat'))!)).toBe('file');
    await s.putPrivate('snapshots/uploads/h/u/part-0001', bytes('p'));
    expect(await s.list('snapshots/')).toEqual(['snapshots/100/h.dat', 'snapshots/uploads/h/u/part-0001']);
    await s.del(['snapshots/100/h.dat']);
    expect(await s.list('snapshots/100/')).toEqual([]);
    expect(await s.readPublic('snapshots/100/h.dat')).toBeNull();
  });
});

describe('blobStore asks Vercel Blob for exactly this', () => {
  class BlobPreconditionFailedError extends Error {}
  const fake = () => {
    const calls: any[] = [];
    return {
      calls,
      BlobPreconditionFailedError,
      async put(path: string, body: Buffer, opts: any) {
        calls.push(['put', path, opts]);
        if (opts.ifMatch === 'stale') throw new BlobPreconditionFailedError('Precondition failed: ETag mismatch.');
        if (opts.allowOverwrite === false && path.endsWith('exists.json')) throw new Error('This blob already exists');
        return { url: `https://store.public.blob.vercel-storage.com/${path}`, etag: '"e2"' };
      },
      async get(path: string, opts: any) {
        calls.push(['get', path, opts]);
        if (path.endsWith('missing.json')) return null;
        return { statusCode: 200, stream: new Response('{"a":1}').body, blob: { etag: '"e1"' } };
      },
      async list(opts: any) {
        calls.push(['list', opts]);
        return opts.cursor ? { blobs: [{ pathname: 'b' }], hasMore: false } : { blobs: [{ pathname: 'a' }], hasMore: true, cursor: 'c1' };
      },
      async del(paths: string[]) {
        calls.push(['del', paths]);
      },
    };
  };
  it('reads private records past the cache and writes them conditionally', async () => {
    const sdk = fake();
    const s = blobStore(sdk);
    const got = await s.getPrivate('snapshots/records/h.json');
    expect([new TextDecoder().decode(got!.bytes), got!.etag]).toEqual(['{"a":1}', '"e1"']);
    expect(sdk.calls[0][2]).toEqual({ access: 'private', useCache: false });
    expect(await s.getPrivate('snapshots/records/missing.json')).toBeNull();
    await s.putPrivate('snapshots/records/h.json', bytes('{}'), { etag: '"e1"' });
    expect(sdk.calls.at(-1)[2]).toMatchObject({ access: 'private', addRandomSuffix: false, allowOverwrite: true, ifMatch: '"e1"' });
    await s.putPrivate('snapshots/records/new.json', bytes('{}'), { etag: null });
    expect(sdk.calls.at(-1)[2]).toMatchObject({ allowOverwrite: false });
    expect(sdk.calls.at(-1)[2].ifMatch).toBeUndefined();
    await expect(s.putPrivate('x', bytes('{}'), { etag: 'stale' })).rejects.toBeInstanceOf(StoreConflict);
    await expect(s.putPrivate('exists.json', bytes('{}'), { etag: null })).rejects.toBeInstanceOf(StoreConflict);
  });
  it('writes checked files public and immutable, lists every page, deletes', async () => {
    const sdk = fake();
    const s = blobStore(sdk);
    const { url } = await s.putPublic('snapshots/100/h.dat', new Uint8Array(9 * 1024 * 1024));
    expect(url).toBe('https://store.public.blob.vercel-storage.com/snapshots/100/h.dat');
    expect(sdk.calls[0][2]).toMatchObject({ access: 'public', addRandomSuffix: false, cacheControlMaxAge: 31_536_000, multipart: true });
    expect(await s.list('snapshots/')).toEqual(['a', 'b']);
    await s.del([]);
    await s.del(['a']);
    expect(sdk.calls.filter((c) => c[0] === 'del')).toEqual([['del', ['a']]]);
  });
  it('reads a public blob back, for the owner\'s archive', async () => {
    const sdk = fake();
    const s = blobStore(sdk);
    expect(new TextDecoder().decode((await s.readPublic('snapshots/100/h.dat'))!)).toBe('{"a":1}');
    expect(sdk.calls[0]).toEqual(['get', 'snapshots/100/h.dat', { access: 'public', useCache: false }]);
    expect(await s.readPublic('snapshots/100/missing.json')).toBeNull();
  });
});

describe('storeFromEnv', () => {
  it('uses a folder only off Vercel, Blob with a token, and nothing without one', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'snapstore-'));
    expect((await storeFromEnv({ SNAPSHOT_STORE_DIR: dir }))!.kind).toBe('folder');
    expect(await storeFromEnv({ SNAPSHOT_STORE_DIR: dir, VERCEL: '1' })).toBeNull();
    expect(await storeFromEnv({})).toBeNull();
    expect((await storeFromEnv({ BLOB_READ_WRITE_TOKEN: 'vercel_blob_rw_x_y' }))!.kind).toBe('blob');
  });
});
````

- [ ] **Step 2: Run it to see it fail**

Run: `cd site && npx vitest run tests/unit/snapshotStore.test.ts`
Expected: FAIL, `Cannot find module '../../src/lib/snapshotStore.mjs'`.

- [ ] **Step 3: Write the module**

Create `site/src/lib/snapshotStore.mjs`:

````js
// Where the snapshot meeting point keeps its records and files. Two stores
// with one shape:
//
// * blobStore: Vercel Blob, what easybtx.com runs on. Records, dispute
//   records, closed heights, the archive and upload parts are PRIVATE blobs
//   (read back with `get`, written with `ifMatch` so two confirmers signing
//   at once cannot lose a signature); the checked file and the merged
//   manifest are PUBLIC, immutable, content-addressed blobs, which is what
//   the app downloads.
// * folderStore: a folder on disk, for tests and for the local stand-in the
//   app's end-to-end rehearsal runs against (scripts/snapshot-rendezvous-local.mjs).
//   Never on Vercel.
//
// putPrivate's `etag` option: a string replaces only if the stored copy still
// has that etag; null creates only if nothing is there; undefined overwrites.
// A lost race throws StoreConflict, and the caller reads again and retries.
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile, rm, readdir, rename } from 'node:fs/promises';
import { dirname, join, relative, sep } from 'node:path';

export class StoreConflict extends Error {}

const etagOf = (bytes) => createHash('sha256').update(bytes).digest('hex');

/**
 * @param {string} dir the folder that holds everything
 * @param {string} publicBase where the folder's public/ subfolder is served, e.g. http://127.0.0.1:29650/blob
 */
export function folderStore(dir, publicBase) {
  const priv = (p) => join(dir, 'private', ...p.split('/'));
  const pub = (p) => join(dir, 'public', ...p.split('/'));
  let chain = Promise.resolve();
  // One writer at a time: the folder store serves one process, and this makes
  // its etag check exact.
  const serial = (fn) => {
    const run = chain.then(fn, fn);
    chain = run.catch(() => {});
    return run;
  };
  const write = async (file, bytes) => {
    await mkdir(dirname(file), { recursive: true });
    const tmp = `${file}.tmp-${process.pid}-${Math.random().toString(16).slice(2)}`;
    await writeFile(tmp, bytes);
    await rename(tmp, file);
  };
  const read = async (file) => {
    try {
      return new Uint8Array(await readFile(file));
    } catch (e) {
      if (e?.code === 'ENOENT') return null;
      throw e;
    }
  };
  const walk = async (root) => {
    const out = [];
    const go = async (d) => {
      let entries;
      try {
        entries = await readdir(d, { withFileTypes: true });
      } catch (e) {
        if (e?.code === 'ENOENT') return;
        throw e;
      }
      for (const e of entries) {
        const full = join(d, e.name);
        if (e.isDirectory()) await go(full);
        else if (!e.name.includes('.tmp-')) out.push(relative(root, full).split(sep).join('/'));
      }
    };
    await go(root);
    return out;
  };
  return {
    kind: 'folder',
    async getPrivate(path) {
      const bytes = await read(priv(path));
      return bytes ? { bytes, etag: etagOf(bytes) } : null;
    },
    putPrivate(path, bytes, { etag } = {}) {
      return serial(async () => {
        const now = await read(priv(path));
        if (etag === null && now) throw new StoreConflict(path);
        if (typeof etag === 'string' && (!now || etagOf(now) !== etag)) throw new StoreConflict(path);
        await write(priv(path), bytes);
        return { etag: etagOf(bytes) };
      });
    },
    async putPublic(path, bytes) {
      await write(pub(path), bytes);
      return { url: `${publicBase}/${path}` };
    },
    /** The bytes of a public blob: for the local stand-in's /blob/ route and the owner's archive. */
    readPublic: (path) => read(pub(path)),
    async list(prefix) {
      const all = [...(await walk(join(dir, 'private'))), ...(await walk(join(dir, 'public')))];
      return all.filter((p) => p.startsWith(prefix)).sort();
    },
    async del(paths) {
      for (const p of paths) {
        await rm(priv(p), { force: true });
        await rm(pub(p), { force: true });
      }
    },
  };
}

/**
 * Vercel Blob. `sdk` is `await import('@vercel/blob')`, passed in so tests can
 * check exactly what is asked of it.
 */
export function blobStore(sdk) {
  const conflict = (e) =>
    e instanceof sdk.BlobPreconditionFailedError || /already exists/i.test(String(e?.message || ''));
  const readAs = async (path, access) => {
    const r = await sdk.get(path, { access, useCache: false });
    if (!r || r.statusCode !== 200) return null;
    return { bytes: new Uint8Array(await new Response(r.stream).arrayBuffer()), etag: r.blob?.etag };
  };
  return {
    kind: 'blob',
    async getPrivate(path) {
      return readAs(path, 'private');
    },
    async putPrivate(path, bytes, { etag } = {}) {
      try {
        const r = await sdk.put(path, Buffer.from(bytes), {
          access: 'private',
          addRandomSuffix: false,
          allowOverwrite: etag !== null,
          ...(typeof etag === 'string' ? { ifMatch: etag } : {}),
          contentType: 'application/octet-stream',
        });
        return { etag: r.etag };
      } catch (e) {
        if (conflict(e)) throw new StoreConflict(path);
        throw e;
      }
    },
    async putPublic(path, bytes, contentType = 'application/octet-stream') {
      const r = await sdk.put(path, Buffer.from(bytes), {
        access: 'public',
        addRandomSuffix: false,
        allowOverwrite: true,
        contentType,
        // Content-addressed paths never change, so caches may keep them.
        cacheControlMaxAge: 365 * 24 * 3600,
        multipart: bytes.length > 8 * 1024 * 1024,
      });
      return { url: r.url };
    },
    /** The bytes of a public blob, for the owner's archive. */
    async readPublic(path) {
      const got = await readAs(path, 'public');
      return got ? got.bytes : null;
    },
    async list(prefix) {
      const out = [];
      let cursor;
      do {
        const page = await sdk.list({ prefix, cursor, limit: 1000 });
        out.push(...page.blobs.map((b) => b.pathname));
        cursor = page.hasMore ? page.cursor : undefined;
      } while (cursor);
      return out.sort();
    },
    async del(paths) {
      if (paths.length) await sdk.del(paths);
    },
  };
}

/**
 * The store this deployment uses, or null when none is provisioned (a
 * preview without a Blob token), which the routes answer with 503.
 * SNAPSHOT_STORE_DIR is honoured only off Vercel.
 */
export async function storeFromEnv(env) {
  if (env.SNAPSHOT_STORE_DIR && !env.VERCEL) {
    return folderStore(env.SNAPSHOT_STORE_DIR, env.SNAPSHOT_PUBLIC_BASE || 'http://127.0.0.1:29650/blob');
  }
  if (!env.BLOB_READ_WRITE_TOKEN && !env.BLOB_STORE_ID) return null;
  return blobStore(await import('@vercel/blob'));
}
````

- [ ] **Step 4: Run the test to see it pass**

Run: `cd site && npx vitest run tests/unit/snapshotStore.test.ts`
Expected: `Tests  6 passed (6)` (derived, not run).

- [ ] **Step 5: Commit**

````bash
git add site/src/lib/snapshotStore.mjs site/tests/unit/snapshotStore.test.ts
git commit -m "site: where snapshot records and files live: Vercel Blob, or a folder for tests" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 5: The handlers (`snapshotRoutes.mjs`)

**Files:**
- Create: `site/src/lib/snapshotRoutes.mjs`
- Test: `site/tests/unit/snapshotRoutes.test.ts`

**Interfaces:**
- Consumes: Tasks 1 to 4.
- Produces (Task 6, and easyNode's client in the app plan, Task 4): `PART_BYTES` (4,194,304), `NODE_HEADER` (`x-ebtx-node`), `NODE_HEADER_VALUE` (`ebtx-snapshot-v1`), `disputePath(chain, height)`, `closedPath(chain, height)`, `contextFromEnv(env, operatorFile) -> ctx` with `ctx = {store, lists, now: () => ms, partBytes}`; `handleStatement`, `handleFile`, `handlePending`, `handleLatest`, `handleDisputes`, each `(request: Request, ctx) -> Promise<Response>`, answering exactly the routes in Global Constraints; `clearHeight(ctx, {chain, height}) -> Promise<{chain, height, archive, statements, moved}>`, called only by `scripts/clear-snapshot-dispute.mjs`.

- [ ] **Step 1: Write the failing test**

Create `site/tests/unit/snapshotRoutes.test.ts`. It runs the whole life of a statement on a folder store with parts of 3,000 bytes (so the 8,055-byte regtest file goes up in three): refused without the header, over 64 KB, off the grid; one operator with its file stored and `latest` still 404; a tampered file refused and nothing kept; a confirmer's signature, and `latest` naming it with bytes that match the vectors; a third signature under a new address; a co-signature from an unlisted key refused with the record unchanged; two confirmers at the same moment both counted; parts of the wrong size refused; a newer two-operator statement without a pinned signature skipped; a pending file below the newest confirmed height let go; an outsider, a file difference and an agreeing dissent disputing nothing, and a differing dissent disputing; a listed operator's other account turning `latest` off at every height until the owner's clear, then that height closed; the clear refusing a height without a dispute.

````ts
import { describe, it, expect, beforeEach } from 'vitest';
import { readFileSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { secp256k1 } from '@noble/curves/secp256k1.js';
import { parseManifest, serializeManifest, statementDigest } from '../../src/lib/snapshotManifest.mjs';
import { parseRegtestOperators } from '../../src/lib/snapshotOperators.mjs';
import { folderStore } from '../../src/lib/snapshotStore.mjs';
import {
  handleStatement, handleFile, handlePending, handleLatest, handleDisputes, clearHeight, NODE_HEADER, NODE_HEADER_VALUE,
} from '../../src/lib/snapshotRoutes.mjs';

const fx = (n: string) => new Uint8Array(readFileSync(new URL(`../fixtures/snapshots/${n}`, import.meta.url)));
const R = (n: string) => fx(`regtest-${n}.manifest`);
const DAT = fx('regtest-100.dat');
const P = '0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464';
const C = '02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f';
const D = '034694ab29307fd4e46f3fc7a5115dd52b4143c473a5eb919e857eb5a12bbd6d0e';
const HASH = '11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194';
const BLOCK = 'bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46';
const HS3 = 'e611efee5d8466160be26e4ed23d2868d391d9fa7202b60312c5d04216c8d527';
const BASE = 'http://127.0.0.1:29650';
const T0 = Date.parse('2026-10-01T12:00:00Z');
const sha = (b: Uint8Array) => createHash('sha256').update(b).digest('hex');

// Keys this suite controls: listed as operators e and f. OUT is on no list.
const fresh = () => {
  const sk = secp256k1.utils.randomSecretKey();
  return { sk, pub: Buffer.from(secp256k1.getPublicKey(sk, true)).toString('hex') };
};
const E = fresh();
const F = fresh();
const OUT = fresh();
const signed = (st: Uint8Array, ...who: { sk: Uint8Array; pub: string }[]) =>
  serializeManifest({
    statement: st,
    signatures: who.map((w) => ({ key: w.pub, der: secp256k1.sign(statementDigest(st), w.sk, { prehash: false, format: 'der', lowS: true }) })),
  });
const view = (st: Uint8Array) => Buffer.from(st.buffer, st.byteOffset, st.byteLength);
/** The regtest vector's statement with `edit` applied to a copy. */
const statementAt = (edit: (st: Uint8Array) => void) => {
  const st = parseManifest(R('P')).statement.slice();
  edit(st);
  return st;
};
/** The same chain facts and file, at height 200. */
const at200 = statementAt((s) => view(s).writeInt32LE(200, 65));

let clock = T0;
let ctx: any;
beforeEach(() => {
  clock = T0;
  ctx = {
    store: folderStore(mkdtempSync(join(tmpdir(), 'snaproutes-')), `${BASE}/blob`),
    lists: {
      main: [],
      regtest: parseRegtestOperators(`producer=${P};confirmer=${C};third=${D};e=${E.pub};f=${F.pub}`),
      pins: { main: [], regtest: [P] },
    },
    now: () => clock,
    partBytes: 3000,
  };
});

const headers = { [NODE_HEADER]: NODE_HEADER_VALUE };
const post = (bytes: Uint8Array, extra: Record<string, string> = headers) =>
  handleStatement(new Request(`${BASE}/api/snapshots/statement`, { method: 'POST', headers: extra, body: bytes }), ctx);
const file = (method: string, query: string, body?: Uint8Array) =>
  handleFile(new Request(`${BASE}/api/snapshots/file?${query}`, { method, headers, body }), ctx);
const latest = () => handleLatest(new Request(`${BASE}/api/snapshots/latest?chain=regtest`), ctx);
const pending = () => handlePending(new Request(`${BASE}/api/snapshots/pending?chain=regtest`), ctx);
const disputes = () => handleDisputes(new Request(`${BASE}/api/snapshots/disputes?chain=regtest`), ctx);

async function upload(bytes: Uint8Array, hash = HASH) {
  const start = await (await file('POST', `statement=${hash}&action=start`)).json();
  for (let n = 1; n <= start.parts; n++) {
    const part = bytes.subarray((n - 1) * start.part_bytes, n * start.part_bytes);
    const r = await file('PUT', `statement=${hash}&upload=${start.upload}&part=${n}`, part);
    if (r.status !== 200) return r;
  }
  return file('POST', `statement=${hash}&upload=${start.upload}&action=complete`);
}

describe('who may write', () => {
  it('needs the node header, POST, and a provisioned store', async () => {
    expect((await post(R('P'), {})).status).toBe(403);
    expect((await handleStatement(new Request(`${BASE}/x`), ctx)).status).toBe(405);
    ctx.store = null;
    expect((await post(R('P'))).status).toBe(503);
    expect((await latest()).status).toBe(404);
  });
  it('refuses bodies over 64 KB and manifests the rules refuse, saying why', async () => {
    expect((await post(new Uint8Array(64 * 1024 + 1))).status).toBe(413);
    const r = await post(fx('mainnet-225927.manifest'));
    expect(r.status).toBe(422);
    expect(await r.json()).toEqual({ error: 'height 225927 is not a multiple of 100' });
  });
});

describe('a statement, its file and its co-signatures', () => {
  it('goes from one operator to confirmed, and latest points at it only then', async () => {
    const first = await post(R('P'));
    expect(first.status).toBe(200);
    expect(await first.json()).toEqual({
      statement_hash: HASH, chain: 'regtest', height: 100, signers: [P], operators: ['producer'], added: 1, file: 'missing',
    });
    expect((await latest()).status).toBe(404);

    // A file that is not the statement's: refused, nothing kept.
    const tampered = DAT.slice();
    tampered[4000] ^= 1;
    const bad = await upload(tampered);
    expect(bad.status).toBe(422);
    expect(await bad.json()).toEqual({ error: 'the file does not match the statement' });
    expect(await ctx.store.list('snapshots/uploads/')).toEqual([]);
    expect(await ctx.store.list('snapshots/100/')).toEqual([]);
    expect((await (await pending()).json()).statements[0].file).toBe('missing');

    // The right file, one operator: stored, still not pointed at.
    const good = await upload(DAT);
    expect(good.status).toBe(200);
    expect(await good.json()).toMatchObject({ stored: true, file_sha256: sha(DAT), confirmed: false });
    expect((await latest()).status).toBe(404);
    expect((await file('POST', `statement=${HASH}&action=start`)).status).toBe(409);

    // A confirmer's one signature: two operators, one of them pinned, and now latest names it.
    const co = await post(R('C'));
    expect(await co.json()).toMatchObject({ signers: [P, C], operators: ['producer', 'confirmer'], added: 1, file: 'stored' });
    const l = await latest();
    expect(l.status).toBe(200);
    expect(l.headers.get('cache-control')).toBe('public, max-age=60, s-maxage=60');
    const p = await l.json();
    expect(p).toMatchObject({
      version: 1, chain: 'regtest', height: 100, statement_hash: HASH, manifest_size: R('PC').length,
      manifest_sha256: sha(R('PC')), file_size: 8055, file_sha256: sha(DAT), operators: ['producer', 'confirmer'],
      confirmed_at: '2026-10-01T12:00:00.000Z',
    });
    expect(p.block_hash).toBe(BLOCK);
    expect(p.file_hash).toBe('76cc3cebd9fc21d6107b11e196dffe3cf2672924ef82e5162882d9395f02058e');
    const path = (u: string) => u.slice(`${BASE}/blob/`.length);
    expect(Buffer.from((await ctx.store.readPublic(path(p.manifest_url)))!).equals(Buffer.from(R('PC')))).toBe(true);
    expect(Buffer.from((await ctx.store.readPublic(path(p.file_url)))!).equals(Buffer.from(DAT))).toBe(true);

    // A third signature is merged and served under a new address.
    await post(R('PCD'));
    const p3 = await (await latest()).json();
    expect(p3.manifest_sha256).toBe(sha(R('PCD')));
    expect(p3.manifest_url).not.toBe(p.manifest_url);
    expect(p3.operators).toEqual(['producer', 'confirmer', 'third']);
  });

  it('refuses a co-signature from a key on no list and keeps what it had', async () => {
    await post(R('P'));
    const r = await post(signed(parseManifest(R('P')).statement, OUT));
    expect(r.status).toBe(422);
    expect((await r.json()).error).toContain('is not on the operator list');
    expect((await (await pending()).json()).statements[0].signers).toEqual([P]);
  });

  it('two confirmers signing at the same moment both count', async () => {
    await post(R('P'));
    const [c, d] = await Promise.all([post(R('C')), post(onlyD())]);
    expect([c.status, d.status]).toEqual([200, 200]);
    const s = (await (await pending()).json()).statements[0];
    expect(new Set(s.signers)).toEqual(new Set([P, C, D]));
  });

  it('takes a part only of the size its place in the file says', async () => {
    await post(R('P'));
    const start = await (await file('POST', `statement=${HASH}&action=start`)).json();
    expect(start).toMatchObject({ part_bytes: 3000, parts: 3 });
    const wrong = await file('PUT', `statement=${HASH}&upload=${start.upload}&part=1`, DAT.subarray(0, 2999));
    expect(await wrong.json()).toEqual({ error: 'part 1 must be 3000 bytes, not 2999' });
    expect((await file('PUT', `statement=${HASH}&upload=${start.upload}&part=4`, DAT.subarray(0, 55))).status).toBe(422);
    expect((await file('PUT', `statement=${HASH}&upload=${'0'.repeat(32)}&part=1`, DAT.subarray(0, 3000))).status).toBe(404);
    expect((await file('POST', `statement=${'ab'.repeat(32)}&action=start`)).status).toBe(404);
    expect((await file('POST', `statement=${HASH}&upload=${start.upload}&action=complete`)).status).toBe(422);
  });

  it('answers pending with who signed, and refuses an unknown chain', async () => {
    await post(R('P'));
    const r = await pending();
    expect(r.headers.get('cache-control')).toBe('public, max-age=30, s-maxage=30');
    expect(await r.json()).toMatchObject({
      version: 1, chain: 'regtest',
      statements: [{ statement_hash: HASH, operators: ['producer'], confirmed: false, disputed: false, dissent: false }],
    });
    expect((await handlePending(new Request(`${BASE}/api/snapshots/pending?chain=testnet`), ctx)).status).toBe(400);
    expect(await (await handleLatest(new Request(`${BASE}/api/snapshots/latest`), ctx)).json()).toEqual({ version: 1, confirmed: null });
  });
});

describe('what latest points at, and what storage keeps', () => {
  it('latest skips a newer two-operator statement without a pinned signature for an older one with one', async () => {
    await post(R('P'));
    await upload(DAT);
    await post(R('C'));
    const newer = await (await post(signed(at200, E, F))).json();
    expect((await upload(DAT, newer.statement_hash)).status).toBe(200);
    const p = (await (await pending()).json()).statements;
    expect(p.map((s: any) => [s.height, s.operators, s.file, s.confirmed])).toEqual([
      [200, ['e', 'f'], 'stored', false], [100, ['producer', 'confirmer'], 'stored', true],
    ]);
    expect((await (await latest()).json()).height).toBe(100);
  });

  it('a pending file below the newest confirmed height is let go, and not taken again', async () => {
    ctx.lists = { ...ctx.lists, pins: { main: [], regtest: [P, E.pub] } };
    await post(R('P'));
    await upload(DAT);
    const top = await (await post(signed(at200, E, F))).json();
    await upload(DAT, top.statement_hash);
    expect((await (await latest()).json()).height).toBe(200);
    const p = (await (await pending()).json()).statements;
    expect(p.map((s: any) => [s.height, s.file, s.confirmed])).toEqual([[200, 'stored', true], [100, 'dropped', false]]);
    expect(await ctx.store.list('snapshots/100/')).toEqual([]);
    expect((await file('POST', `statement=${HASH}&action=start`)).status).toBe(409);
  });
});

describe('disputes, and the owner\'s clear', () => {
  it('disputes only on chain facts from listed operators: a dissent can, a file difference and an outsider cannot', async () => {
    await post(R('P'));
    const noDispute = async () => expect((await (await disputes()).json()).disputes).toEqual([]);
    // An outsider's other account: refused, and nothing changes.
    expect((await post(signed(statementAt((s) => view(s).writeBigUInt64LE(102n, 101)), OUT))).status).toBe(422);
    await noDispute();
    // The same chain facts with another file: a statement of its own, not a dispute.
    expect((await post(signed(statementAt((s) => { s[189] ^= 1; }), E))).status).toBe(200);
    await noDispute();
    // A dissent that agrees on the chain facts: no dispute, and it never takes a file.
    const agree = await (await post(signed(statementAt((s) => s.fill(0, 181, 229)), E))).json();
    expect(agree.file).toBe('missing');
    expect(await (await file('POST', `statement=${agree.statement_hash}&action=start`)).json()).toEqual({ error: 'a dissent has no file' });
    await noDispute();
    // A dissent that does not agree: disputed, and latest serves nothing.
    const differ = await (await post(signed(statementAt((s) => { view(s).writeBigUInt64LE(102n, 101); s.fill(0, 181, 229); }), F))).json();
    const d = (await (await disputes()).json()).disputes;
    expect(d.map((x: any) => [x.height, x.differ])).toEqual([[100, ['coins']]]);
    expect(d[0].alert).toContain('Dissent from f.');
    const l = await latest();
    expect(l.status).toBe(200);
    expect(await l.json()).toEqual({ disputed: [100] });
    const p = (await (await pending()).json()).statements;
    expect(p.find((s: any) => s.statement_hash === differ.statement_hash)).toMatchObject({ dissent: true, disputed: true, file: 'missing' });
  });

  it('a listed operator\'s other account of a height turns latest off at every height until the owner clears it', async () => {
    await post(R('P'));
    await upload(DAT);
    await post(R('C'));
    expect((await (await latest()).json()).height).toBe(100);
    const x = await (await post(signed(at200, E))).json();
    clock += 1000;
    const y = await (await post(signed(statementAt((s) => { view(s).writeInt32LE(200, 65); view(s).writeBigUInt64LE(102n, 101); }), F))).json();
    expect(await (await latest()).json()).toEqual({ disputed: [200] });
    const d = (await (await disputes()).json()).disputes;
    expect(d).toHaveLength(1);
    expect(d[0]).toMatchObject({ height: 200, differ: ['coins'], opened_at: '2026-10-01T12:00:01.000Z' });
    expect(d[0].statements.map((s: any) => [s.statement_hash, s.signers, s.dissent])).toEqual([
      [x.statement_hash, ['e'], false], [y.statement_hash, ['f'], false],
    ]);
    expect(d[0].alert).toBe([
      'easyNode snapshots: block 200 is disputed. Fast-forward is off for every node until you clear it.',
      `1: block ${BLOCK}, UTXO hash ${HS3}, 101 coins, 101 transactions. Signed by e.`,
      `2: block ${BLOCK}, UTXO hash ${HS3}, 102 coins, 101 transactions. Signed by f.`,
      'They differ in: coin count.',
      'Check both nodes, then clear it: node scripts/clear-snapshot-dispute.mjs 200 --chain regtest',
    ].join('\n'));
    expect((await (await pending()).json()).statements.filter((s: any) => s.disputed).map((s: any) => s.height)).toEqual([200, 200]);

    clock += 1000;
    const cleared = await clearHeight(ctx, { chain: 'regtest', height: 200 });
    expect([...cleared.statements].sort()).toEqual([x.statement_hash, y.statement_hash].sort());
    expect((await (await latest()).json()).height).toBe(100);
    expect((await (await disputes()).json()).disputes).toEqual([]);
    expect(await ctx.store.list('snapshots/closed/')).toEqual(['snapshots/closed/regtest-200.json']);
    const archived = await ctx.store.list(cleared.archive);
    expect(archived.filter((p: string) => p.includes('/snapshots/records/'))).toHaveLength(2);
    expect(archived.filter((p: string) => p.endsWith('/snapshots/disputes/regtest-200.json'))).toHaveLength(1);
    const again = await post(signed(at200, E));
    expect(again.status).toBe(422);
    expect(await again.json()).toEqual({ error: 'closed-height' });
  });

  it('the clear refuses a height without a dispute, and no route clears one', async () => {
    await post(R('P'));
    await expect(clearHeight(ctx, { chain: 'regtest', height: 100 })).rejects.toThrow('block 100 on regtest has no open dispute, so there is nothing to clear');
    await expect(clearHeight(ctx, { chain: 'regtest', height: 150 })).rejects.toThrow('height 150 is not a multiple of 100');
    expect(await ctx.store.list('snapshots/closed/')).toEqual([]);
    const r = await disputes();
    expect(r.headers.get('cache-control')).toBe('public, max-age=30, s-maxage=30');
    expect(await r.json()).toEqual({ disputes: [] });
    expect((await handleDisputes(new Request(`${BASE}/api/snapshots/disputes`, { method: 'DELETE' }), ctx)).status).toBe(405);
  });
});

/** The statement with only D's signature, cut from the three-signature vector. */
function onlyD() {
  const m = parseManifest(R('PCD'));
  return serializeManifest({ statement: m.statement, signatures: m.signatures.filter((s: any) => s.key === D) });
}
````

- [ ] **Step 2: Run it to see it fail**

Run: `cd site && npx vitest run tests/unit/snapshotRoutes.test.ts`
Expected: FAIL, `Cannot find module '../../src/lib/snapshotRoutes.mjs'`.

- [ ] **Step 3: Write the module**

Create `site/src/lib/snapshotRoutes.mjs`:

````js
// The snapshot routes as plain functions from a Request to a Response, so the
// Astro routes (site/src/pages/api/snapshots/*.ts), the vitest suite and the
// local stand-in (scripts/snapshot-rendezvous-local.mjs) all run the very
// same code. The rules live in snapshotRendezvous.mjs; this file reads and
// writes the store around them.
//
//   POST /api/snapshots/statement            a manifest or a dissent, at most 64 KB
//   POST /api/snapshots/file?statement=H&action=start
//   PUT  /api/snapshots/file?statement=H&upload=U&part=N   one part, at most 4 MiB
//   POST /api/snapshots/file?statement=H&upload=U&action=complete
//   GET  /api/snapshots/pending?chain=main
//   GET  /api/snapshots/latest?chain=main
//   GET  /api/snapshots/disputes?chain=main
//
// No route clears a dispute. Only the owner does, by running
// scripts/clear-snapshot-dispute.mjs with the store's write token, which
// calls clearHeight below.
//
// Why the file goes up in parts of its own: a Vercel function takes a request
// body of about 4.5 MB, and Blob's multipart API wants parts of at least 5 MB
// (except the last), so a part that fits through a function cannot be a Blob
// multipart part. Each part is kept as a private blob; when the last is in,
// the function reads them back, hashes the whole file, and writes it as one
// public blob only if its size and double SHA-256 are the statement's.
//
// The store's layout:
//   snapshots/records/<statement hash>.json            private, one per statement or dissent
//   snapshots/uploads/<statement hash>/<upload>/...     private, parts on their way in
//   snapshots/<height>/<statement hash>.dat             public, a checked file
//   snapshots/<height>/<statement hash>-<n>.manifest    public, a confirmed manifest with n signatures
//   snapshots/disputes/<chain>-<height>.json            private, an open dispute and the owner's alert
//   snapshots/closed/<chain>-<height>.json              private, a height the owner cleared
//   snapshots/archive/<chain>-<height>-<time>/...       private, what a clear moved away
import { randomBytes, createHash } from 'node:crypto';
import { MAX_MANIFEST_BYTES, fileHashes } from './snapshotManifest.mjs';
import {
  CHAINS, GRID, CLOSED_HEIGHT, acceptManifest, mergeSignatures, newRecord, withManifest, recordManifest, recordOperators,
  isConfirmed, chooseLatest, latestView, pendingView, storagePlan, partPlan,
  disputeAt, liveDisputes, disputedHeights, newDispute, refreshDispute, disputesView,
} from './snapshotRendezvous.mjs';
import { listsFromEnv } from './snapshotOperators.mjs';
import { storeFromEnv, StoreConflict } from './snapshotStore.mjs';

export const PART_BYTES = 4 * 1024 * 1024;
export const NODE_HEADER = 'x-ebtx-node';
export const NODE_HEADER_VALUE = 'ebtx-snapshot-v1';

const recordPath = (hash) => `snapshots/records/${hash}.json`;
const uploadPrefix = (hash) => `snapshots/uploads/${hash}/`;
const partPath = (hash, upload, n) => `${uploadPrefix(hash)}${upload}/part-${String(n).padStart(4, '0')}`;
const filePath = (r) => `snapshots/${r.height}/${r.statement_hash}.dat`;
const manifestPath = (r) => `snapshots/${r.height}/${r.statement_hash}-${r.signers.length}.manifest`;
const publicPrefix = (r) => `snapshots/${r.height}/${r.statement_hash}`;
export const disputePath = (chain, height) => `snapshots/disputes/${chain}-${height}.json`;
export const closedPath = (chain, height) => `snapshots/closed/${chain}-${height}.json`;
const ARCHIVE = 'snapshots/archive/';
/** The only public paths: checked files and confirmed manifests. */
const isPublicPath = (p) => /^snapshots\/\d+\//.test(p);
const DROPPED = 'this file was let go to keep storage small: the website keeps files only for the newest statements';

class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

const json = (status, body, headers = {}) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } });
const fail = (status, error) => json(status, { error });
const iso = (ms) => new Date(ms).toISOString();
const enc = (value) => new TextEncoder().encode(JSON.stringify(value));

/**
 * What every handler needs.
 * @param {Record<string, string | undefined>} env
 * @param {unknown} operatorFile the parsed site/src/lib/snapshot-operators.json
 */
export async function contextFromEnv(env, operatorFile) {
  return { store: await storeFromEnv(env), lists: listsFromEnv(env, operatorFile), now: () => Date.now(), partBytes: PART_BYTES };
}

async function readJson(store, path) {
  const got = await store.getPrivate(path);
  return got ? { value: JSON.parse(new TextDecoder().decode(got.bytes)), etag: got.etag } : null;
}

async function readRecord(store, hash) {
  const got = await readJson(store, recordPath(hash));
  return got ? { record: got.value, etag: got.etag } : null;
}

async function readAllAt(store, prefix) {
  const out = [];
  for (const p of await store.list(prefix)) {
    try {
      const got = await readJson(store, p);
      if (got) out.push(got.value);
    } catch (e) {
      console.error('snapshot record unreadable', p, e);
    }
  }
  return out;
}
const readAll = (store) => readAllAt(store, 'snapshots/records/');
const readDisputes = (store) => readAllAt(store, 'snapshots/disputes/');

/**
 * Read, change, write back only if nobody wrote in between; three more tries
 * if somebody did. A change that returns what it was given, or null, writes
 * nothing.
 */
async function upsertAt(store, path, change) {
  for (let attempt = 0; attempt < 4; attempt++) {
    const cur = await readJson(store, path);
    const next = await change(cur ? cur.value : null);
    if (next == null || (cur && next === cur.value)) return next ?? null;
    try {
      await store.putPrivate(path, enc(next), { etag: cur ? cur.etag : null });
      return next;
    } catch (e) {
      if (!(e instanceof StoreConflict)) throw e;
    }
  }
  throw new HttpError(503, 'too many writers at once; try again');
}
const upsert = (store, hash, change) => upsertAt(store, recordPath(hash), change);

/** Once confirmed: the merged manifest as a public, content-addressed blob, and when it was first confirmed. */
async function publish(store, record, lists, nowIso) {
  if (!isConfirmed(record, lists)) return record;
  let r = record;
  if (!r.public_manifest || r.public_manifest.signatures !== r.signers.length) {
    const bytes = Buffer.from(r.manifest_hex, 'hex');
    const { url } = await store.putPublic(manifestPath(r), bytes);
    r = {
      ...r,
      public_manifest: {
        url,
        sha256: createHash('sha256').update(bytes).digest('hex'),
        size: bytes.length,
        signatures: r.signers.length,
      },
    };
  }
  return r.confirmed_at ? r : { ...r, confirmed_at: nowIso };
}

/**
 * Section 6a, after every statement or dissent is stored: open a dispute
 * record, with the owner's alert, the first time a height is disputed, and
 * keep its statements current after that. Never closes one: only the owner's
 * clear does. Best effort: `latest` applies the rule itself too, and the next
 * statement tries again.
 */
async function syncDisputes(ctx, records) {
  const nowIso = iso(ctx.now());
  for (const chain of Object.keys(CHAINS)) {
    for (const d of liveDisputes(records, chain, ctx.lists)) {
      try {
        await upsertAt(ctx.store, disputePath(chain, d.height), async (cur) => {
          if (!cur) return newDispute(d, ctx.lists, nowIso);
          const next = refreshDispute(cur, d, ctx.lists);
          return JSON.stringify(next) === JSON.stringify(cur) ? cur : next;
        });
      } catch (e) {
        console.error('snapshot dispute record not written', chain, d.height, e);
      }
    }
  }
}

/** Best effort: storage stays what storagePlan keeps (Global Constraints, Storage). */
async function prune(ctx, records) {
  try {
    const plan = storagePlan(records, ctx.now(), ctx.lists, await readDisputes(ctx.store));
    for (const r of plan.dropRecords) {
      await ctx.store.del([
        ...(await ctx.store.list(publicPrefix(r))),
        ...(await ctx.store.list(uploadPrefix(r.statement_hash))),
        recordPath(r.statement_hash),
      ]);
    }
    const nowIso = iso(ctx.now());
    for (const r of plan.dropFiles) {
      await ctx.store.del([...(await ctx.store.list(publicPrefix(r))), ...(await ctx.store.list(uploadPrefix(r.statement_hash)))]);
      await upsert(ctx.store, r.statement_hash, async (cur) =>
        cur ? { ...cur, file: { state: 'dropped', dropped_at: nowIso }, public_manifest: null } : null);
    }
  } catch (e) {
    console.error('snapshot prune failed', e);
  }
}

function guard(request, ctx, method) {
  if (request.method !== method) return fail(405, `use ${method}`);
  if (method !== 'GET' && request.headers.get(NODE_HEADER) !== NODE_HEADER_VALUE) return new Response(null, { status: 403 });
  if (!ctx.store) return new Response(null, { status: 503 });
  return null;
}

async function run(fn) {
  try {
    return await fn();
  } catch (e) {
    if (e instanceof HttpError) return fail(e.status, e.message);
    console.error('snapshot route failed', e);
    return new Response(null, { status: 503 });
  }
}

async function bodyUpTo(request, limit) {
  const declared = Number(request.headers.get('content-length') || '0');
  if (declared > limit) throw new HttpError(413, `at most ${limit} bytes`);
  const bytes = new Uint8Array(await request.arrayBuffer());
  if (bytes.length > limit) throw new HttpError(413, `at most ${limit} bytes`);
  return bytes;
}

/** POST /api/snapshots/statement: a new statement or dissent, or co-signatures for one already here. */
export async function handleStatement(request, ctx) {
  return guard(request, ctx, 'POST') ?? run(async () => {
    const bytes = await bodyUpTo(request, MAX_MANIFEST_BYTES);
    if (bytes.length === 0) throw new HttpError(400, 'no manifest');
    const accepted = acceptManifest(bytes, ctx.lists);
    if (!accepted.ok) throw new HttpError(422, accepted.reason);
    if (await ctx.store.getPrivate(closedPath(accepted.chain, accepted.fields.height))) throw new HttpError(422, CLOSED_HEIGHT);
    const nowIso = iso(ctx.now());
    let added = [];
    const record = await upsert(ctx.store, accepted.hash, async (cur) => {
      if (!cur) {
        added = accepted.manifest.signatures.map((s) => s.key);
        return newRecord(accepted, nowIso);
      }
      const merged = mergeSignatures(recordManifest(cur), accepted.manifest);
      added = merged.added;
      if (merged.added.length === 0) return cur;
      return publish(ctx.store, withManifest(cur, merged.manifest, nowIso), ctx.lists, nowIso);
    });
    const records = await readAll(ctx.store);
    await syncDisputes(ctx, records);
    await prune(ctx, records);
    return json(200, {
      statement_hash: record.statement_hash,
      chain: record.chain,
      height: record.height,
      signers: record.signers,
      operators: recordOperators(record, ctx.lists),
      added: added.length,
      file: record.file.state,
    });
  });
}

const HEX64 = /^[0-9a-f]{64}$/;
const HEX32 = /^[0-9a-f]{32}$/;

/** The file, in parts: start, one PUT per part, complete. Only for a statement already stored that is not a dissent. */
export async function handleFile(request, ctx) {
  const method = request.method === 'PUT' ? 'PUT' : 'POST';
  return guard(request, ctx, method) ?? run(async () => {
    const q = new URL(request.url).searchParams;
    const hash = q.get('statement') || '';
    if (!HEX64.test(hash)) throw new HttpError(400, 'statement must be 64 lowercase hex characters');
    const cur = await readRecord(ctx.store, hash);
    if (!cur) throw new HttpError(404, 'no such statement');
    const plan = partPlan(cur.record.file_size, ctx.partBytes);
    const upload = q.get('upload') || '';
    const current = (r) => r?.file?.state === 'uploading' && r.file.upload === upload && HEX32.test(upload);

    if (method === 'POST' && q.get('action') === 'start') {
      if (cur.record.dissent) throw new HttpError(422, 'a dissent has no file');
      if (cur.record.file.state === 'stored') throw new HttpError(409, 'the file is already stored');
      if (cur.record.file.state === 'dropped') throw new HttpError(409, DROPPED);
      if (recordOperators(cur.record, ctx.lists).length === 0) throw new HttpError(422, 'no signature on this statement is from a listed key');
      const id = randomBytes(16).toString('hex');
      await upsert(ctx.store, hash, async (r) => {
        if (!r) throw new HttpError(404, 'no such statement');
        if (r.file.state === 'stored') throw new HttpError(409, 'the file is already stored');
        if (r.file.state === 'dropped') throw new HttpError(409, DROPPED);
        return { ...r, file: { state: 'uploading', upload: id, started_at: iso(ctx.now()) } };
      });
      const stale = (await ctx.store.list(uploadPrefix(hash))).filter((p) => !p.startsWith(`${uploadPrefix(hash)}${id}/`));
      await ctx.store.del(stale);
      return json(200, { upload: id, part_bytes: ctx.partBytes, parts: plan.parts });
    }

    if (method === 'PUT') {
      if (!current(cur.record)) throw new HttpError(404, 'no such upload');
      const n = Number(q.get('part'));
      const want = Number.isInteger(n) ? plan.sizeOf(n) : -1;
      if (want < 0) throw new HttpError(422, `part must be 1 to ${plan.parts}`);
      const body = await bodyUpTo(request, ctx.partBytes);
      if (body.length !== want) throw new HttpError(422, `part ${n} must be ${want} bytes, not ${body.length}`);
      await ctx.store.putPrivate(partPath(hash, upload, n), body);
      return json(200, { part: n, bytes: body.length });
    }

    if (method === 'POST' && q.get('action') === 'complete') {
      if (!current(cur.record)) throw new HttpError(404, 'no such upload');
      const chunks = [];
      for (let n = 1; n <= plan.parts; n++) {
        const got = await ctx.store.getPrivate(partPath(hash, upload, n));
        if (!got || got.bytes.length !== plan.sizeOf(n)) throw new HttpError(422, `part ${n} is missing`);
        chunks.push(got.bytes);
      }
      const h = fileHashes(chunks);
      const parts = await ctx.store.list(uploadPrefix(hash));
      if (h.size !== cur.record.file_size || h.fileHash !== cur.record.file_hash) {
        await ctx.store.del(parts);
        await upsert(ctx.store, hash, async (r) => (current(r) ? { ...r, file: { state: 'missing' } } : r));
        throw new HttpError(422, 'the file does not match the statement');
      }
      const { url } = await ctx.store.putPublic(filePath(cur.record), Buffer.concat(chunks));
      const nowIso = iso(ctx.now());
      const record = await upsert(ctx.store, hash, async (r) =>
        r ? publish(ctx.store, { ...r, file: { state: 'stored', url, sha256: h.sha256, stored_at: nowIso } }, ctx.lists, nowIso) : null);
      await ctx.store.del(parts);
      await prune(ctx, await readAll(ctx.store));
      return json(200, { stored: true, file_url: url, file_sha256: h.sha256, confirmed: record ? isConfirmed(record, ctx.lists) : false });
    }
    throw new HttpError(400, 'action must be start or complete');
  });
}

function chainParam(request) {
  const chain = new URL(request.url).searchParams.get('chain') || 'main';
  if (chain !== 'main' && chain !== 'regtest') throw new HttpError(400, 'chain must be main or regtest');
  return chain;
}

/** GET /api/snapshots/pending: statements and dissents of the last seven days and who signed each. */
export async function handlePending(request, ctx) {
  return guard(request, ctx, 'GET') ?? run(async () => {
    const chain = chainParam(request);
    const view = pendingView(await readAll(ctx.store), chain, ctx.now(), ctx.lists, await readDisputes(ctx.store));
    return json(200, view, { 'cache-control': 'public, max-age=30, s-maxage=30' });
  });
}

const NONE = { version: 1, confirmed: null };
const LATEST_CACHE = { 'cache-control': 'public, max-age=60, s-maxage=60' };

/**
 * GET /api/snapshots/latest: while any dispute on the chain stands, only the
 * disputed heights; otherwise the newest statement two different operators
 * signed, a pinned key among them, with its checked file.
 */
export async function handleLatest(request, ctx) {
  if (request.method !== 'GET') return fail(405, 'use GET');
  if (!ctx.store) return json(404, NONE, LATEST_CACHE);
  return run(async () => {
    const chain = chainParam(request);
    const records = await readAll(ctx.store);
    const disputed = disputedHeights(records, chain, ctx.lists, await readDisputes(ctx.store));
    if (disputed.length) return json(200, { disputed }, LATEST_CACHE);
    const best = chooseLatest(records, chain, ctx.lists);
    return best ? json(200, latestView(best, ctx.lists), LATEST_CACHE) : json(404, NONE, LATEST_CACHE);
  });
}

/** GET /api/snapshots/disputes: open disputes with both accounts, who signed each, what differs and the owner's alert. */
export async function handleDisputes(request, ctx) {
  return guard(request, ctx, 'GET') ?? run(async () => {
    const chain = chainParam(request);
    return json(200, disputesView(await readDisputes(ctx.store), chain), { 'cache-control': 'public, max-age=30, s-maxage=30' });
  });
}

/**
 * The owner's clear (section 6a). Every statement, dissent, file, manifest
 * and upload at one height, and the dispute record, are copied into
 * snapshots/archive/; the height is closed, so a copy of an old statement or
 * dissent cannot bring the dispute back; then the originals go. Nothing is
 * deleted until everything is copied, so a failed copy changes nothing and
 * the clear can simply run again. Called by scripts/clear-snapshot-dispute.mjs
 * only: no route reaches it.
 */
export async function clearHeight(ctx, { chain, height }) {
  if (!CHAINS[chain]) throw new Error(`chain must be main or regtest, not ${chain}`);
  if (!Number.isInteger(height) || height <= 0 || height % GRID !== 0) throw new Error(`height ${height} is not a multiple of ${GRID}`);
  const here = async () => (await readAll(ctx.store)).filter((r) => r.chain === chain && r.height === height);
  const dPath = disputePath(chain, height);
  const first = await here();
  if (!(await ctx.store.getPrivate(dPath)) && !disputeAt(first, chain, height, ctx.lists)) {
    throw new Error(`block ${height} on ${chain} has no open dispute, so there is nothing to clear`);
  }
  const nowIso = iso(ctx.now());
  const archive = `${ARCHIVE}${chain}-${height}-${nowIso.replace(/[-:.]/g, '')}/`;
  const moved = new Set();
  const statements = new Set();
  const copy = async (records) => {
    const paths = [];
    for (const r of records) {
      statements.add(r.statement_hash);
      paths.push(
        recordPath(r.statement_hash),
        ...(await ctx.store.list(publicPrefix(r))),
        ...(await ctx.store.list(uploadPrefix(r.statement_hash))),
      );
    }
    if (await ctx.store.getPrivate(dPath)) paths.push(dPath);
    for (const p of paths) {
      if (moved.has(p)) continue;
      const bytes = isPublicPath(p) ? await ctx.store.readPublic(p) : (await ctx.store.getPrivate(p))?.bytes;
      if (!bytes) throw new Error(`could not read ${p}, so nothing was deleted; run the clear again`);
      await ctx.store.putPrivate(`${archive}${p}`, bytes);
      moved.add(p);
    }
  };
  await copy(first);
  // Closed before anything is deleted, so nothing new lands at this height in between.
  await ctx.store.putPrivate(
    closedPath(chain, height),
    enc({ version: 1, chain, height, closed_at: nowIso, archive, statements: [...statements] }),
  );
  // Whatever arrived while this ran goes into the archive too.
  await copy((await here()).filter((r) => !moved.has(recordPath(r.statement_hash))));
  await ctx.store.del([...moved]);
  return { chain, height, archive, statements: [...statements], moved: moved.size };
}
````

- [ ] **Step 4: Run the test, then the whole suite**

Run: `cd site && npx vitest run tests/unit/snapshotRoutes.test.ts`
Expected: `Tests  12 passed (12)` (derived, not run).

Run: `cd site && npx vitest run`
Expected: every file passes; the five new files add 81 tests to the 250 already there, `Tests  331 passed (331)` (derived, not run: the first version of this plan measured 303 on `e187d14c` with its own 53 new tests, so 250 before; #548 touched no file vitest reads).

- [ ] **Step 5: Commit**

````bash
git add site/src/lib/snapshotRoutes.mjs site/tests/unit/snapshotRoutes.test.ts
git commit -m "site: the snapshot routes as plain handlers: statement, file in parts, pending, latest, disputes, and the owner's clear" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 6: The Astro routes, the local stand-in, and the owner's clear

**Files:**
- Create: `site/src/pages/api/snapshots/statement.ts`, `file.ts`, `pending.ts`, `latest.ts`, `disputes.ts`
- Create: `site/scripts/snapshot-rendezvous-local.mjs`
- Create: `scripts/clear-snapshot-dispute.mjs`
- Modify: `.github/workflows/site-tests.yml` (two `paths:` lines)
- Test: `site/tests/unit/snapshotClearScript.test.ts`

**Interfaces:**
- Consumes: Task 5 (`contextFromEnv`, the five handlers, `clearHeight`, `PART_BYTES`), Task 4 (`folderStore`, `storeFromEnv`), Task 2 (`listsFromEnv`).
- Produces (easyNode's rehearsal, app plan Task 8): `node scripts/snapshot-rendezvous-local.mjs --port <p> --dir <folder>` run from `site/`, with `SNAPSHOT_REGTEST_OPERATORS` and `SNAPSHOT_REGTEST_PINS` set, serving the five routes at `http://127.0.0.1:<p>/api/snapshots/...` and the public blobs at `http://127.0.0.1:<p>/blob/<path>`; it prints `snapshot rendezvous stand-in on http://127.0.0.1:<p>, …` once listening and refuses bodies over 4,500,000 bytes with 413, as a Vercel function does. For the owner: `node scripts/clear-snapshot-dispute.mjs <height> [--chain main|regtest]` from the repository root, with `BLOB_READ_WRITE_TOKEN` or `SNAPSHOT_STORE_DIR`; exit 0 and one line saying what it moved, exit 1 with one line saying why nothing changed, exit 2 on a wrong command line.

- [ ] **Step 1: Write the five routes**

Create `site/src/pages/api/snapshots/statement.ts`:

````ts
// POST a signed snapshot statement, co-signatures for one already here, or a
// dissent. The rules and the reasons are in src/lib/snapshotRendezvous.mjs;
// the handler is in src/lib/snapshotRoutes.mjs, shared with the tests and the
// local stand-in easyNode's rehearsal runs against.
import type { APIRoute } from 'astro';
import operatorFile from '../../../lib/snapshot-operators.json';
import { contextFromEnv, handleStatement } from '../../../lib/snapshotRoutes.mjs';

export const prerender = false;

export const POST: APIRoute = async ({ request }) => handleStatement(request, await contextFromEnv(process.env, operatorFile));
````

Create `site/src/pages/api/snapshots/file.ts`:

````ts
// The file of a stored statement, in parts of at most 4 MiB: POST start, PUT
// each part, POST complete. Kept only if its size and double SHA-256 are the
// statement's; never for a dissent. See src/lib/snapshotRoutes.mjs for why it
// goes in parts.
import type { APIRoute } from 'astro';
import operatorFile from '../../../lib/snapshot-operators.json';
import { contextFromEnv, handleFile } from '../../../lib/snapshotRoutes.mjs';

export const prerender = false;

export const POST: APIRoute = async ({ request }) => handleFile(request, await contextFromEnv(process.env, operatorFile));
export const PUT: APIRoute = async ({ request }) => handleFile(request, await contextFromEnv(process.env, operatorFile));
````

Create `site/src/pages/api/snapshots/pending.ts`:

````ts
// What confirmers read: statements and dissents of the last seven days and who signed each.
import type { APIRoute } from 'astro';
import operatorFile from '../../../lib/snapshot-operators.json';
import { contextFromEnv, handlePending } from '../../../lib/snapshotRoutes.mjs';

export const prerender = false;

export const GET: APIRoute = async ({ request }) => handlePending(request, await contextFromEnv(process.env, operatorFile));
````

Create `site/src/pages/api/snapshots/latest.ts`:

````ts
// What every easyNode reads first: the newest snapshot two different
// operators signed, a key every node pins among them, with its checked file;
// or, while operators disagree about any height, only the disputed heights.
// The app trusts none of it and checks everything again; a 404 here, or a
// dispute, only means every node uses its fallbacks.
import type { APIRoute } from 'astro';
import operatorFile from '../../../lib/snapshot-operators.json';
import { contextFromEnv, handleLatest } from '../../../lib/snapshotRoutes.mjs';

export const prerender = false;

export const GET: APIRoute = async ({ request }) => handleLatest(request, await contextFromEnv(process.env, operatorFile));
````

Create `site/src/pages/api/snapshots/disputes.ts`:

````ts
// Open disputes, for the owner's alerting (Orca reads this): both accounts of
// the height, who signed each, the fields that differ and the alert text.
// Read-only. Only the owner clears a dispute, with
// scripts/clear-snapshot-dispute.mjs and the store's write token.
import type { APIRoute } from 'astro';
import operatorFile from '../../../lib/snapshot-operators.json';
import { contextFromEnv, handleDisputes } from '../../../lib/snapshotRoutes.mjs';

export const prerender = false;

export const GET: APIRoute = async ({ request }) => handleDisputes(request, await contextFromEnv(process.env, operatorFile));
````

- [ ] **Step 2: Build, and run the real routes under `astro dev`**

````bash
cd /Users/m2promende/repos/EasyBTX-snapshot-rendezvous/site
npx astro build 2>&1 | tail -3
T=$(mktemp -d)/rv-astro
SNAPSHOT_STORE_DIR=$T SNAPSHOT_PUBLIC_BASE=http://127.0.0.1:4329/blob \
SNAPSHOT_REGTEST_OPERATORS="producer=0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464;confirmer=02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f" \
SNAPSHOT_REGTEST_PINS=0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464 \
  npx astro dev --port 4329 --host 127.0.0.1 > $T.log 2>&1 &
for i in $(seq 1 60); do curl -s -o /dev/null 'http://127.0.0.1:4329/api/snapshots/latest?chain=regtest' && break; sleep 1; done
curl -s -w ' %{http_code}\n' 'http://127.0.0.1:4329/api/snapshots/latest?chain=regtest'
curl -s -w ' %{http_code}\n' 'http://127.0.0.1:4329/api/snapshots/disputes?chain=regtest'
curl -s -w ' %{http_code}\n' -H 'x-ebtx-node: ebtx-snapshot-v1' -H 'content-type: application/octet-stream' --data-binary @tests/fixtures/snapshots/regtest-PC.manifest http://127.0.0.1:4329/api/snapshots/statement
curl -s -w ' %{http_code}\n' -X POST -H 'x-ebtx-node: ebtx-snapshot-v1' 'http://127.0.0.1:4329/api/snapshots/file?statement=11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194&action=start'
pkill -f 'astro dev --port 4329'
````

Expected (derived, not run; the first version of these routes answered this way on `e187d14c`): the build ends `[build] Complete!`; then `{"version":1,"confirmed":null} 404`; then `{"disputes":[]} 200`; then `{"statement_hash":"11c5406e…f194","chain":"regtest","height":100,"signers":["0343faeb…e464","02c05d68…6c0f"],"operators":["producer","confirmer"],"added":2,"file":"missing"} 200`; then `{"upload":"<32 hex>","part_bytes":4194304,"parts":1} 200`. Every POST and PUT body goes as `application/octet-stream`: Astro's origin check refuses form content types from another origin, and the app sends octet-stream.

- [ ] **Step 3: Write the local stand-in**

Create `site/scripts/snapshot-rendezvous-local.mjs`:

````js
#!/usr/bin/env node
// A local stand-in for the snapshot routes on easybtx.com, for easyNode's
// end-to-end rehearsal on regtest (crates/btx-core/tests/snapshot_network_regtest.rs
// in the easynode repository). It runs the SAME handlers the Astro routes run
// (src/lib/snapshotRoutes.mjs) over node:http, with a folder store instead of
// Vercel Blob, and serves the folder's public blobs under /blob/. Like a
// Vercel function it refuses request bodies over 4.5 MB. The owner's clear
// (scripts/clear-snapshot-dispute.mjs, from the repository root) works on the
// same folder with SNAPSHOT_STORE_DIR set to it.
//
//   SNAPSHOT_REGTEST_OPERATORS="producer=<66 hex>;confirmer=<66 hex>" \
//   SNAPSHOT_REGTEST_PINS="<66 hex>" \
//     node scripts/snapshot-rendezvous-local.mjs --port 29650 --dir /tmp/rv
//
// Loopback only. Not a server for anything else.
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { folderStore } from '../src/lib/snapshotStore.mjs';
import { listsFromEnv } from '../src/lib/snapshotOperators.mjs';
import {
  handleStatement, handleFile, handlePending, handleLatest, handleDisputes, PART_BYTES,
} from '../src/lib/snapshotRoutes.mjs';

const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : fallback;
};
const port = Number(arg('port', '29650'));
const dir = arg('dir', null);
if (!dir) {
  console.error('usage: snapshot-rendezvous-local.mjs --port 29650 --dir <folder>');
  process.exit(2);
}
const MAX_BODY = 4_500_000;
const base = `http://127.0.0.1:${port}`;
const store = folderStore(dir, `${base}/blob`);
const operatorFile = JSON.parse(readFileSync(new URL('../src/lib/snapshot-operators.json', import.meta.url), 'utf8'));
const ctx = { store, lists: listsFromEnv(process.env, operatorFile), now: () => Date.now(), partBytes: PART_BYTES };
const routes = {
  '/api/snapshots/statement': handleStatement,
  '/api/snapshots/file': handleFile,
  '/api/snapshots/pending': handlePending,
  '/api/snapshots/latest': handleLatest,
  '/api/snapshots/disputes': handleDisputes,
};

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, base);
    if (url.pathname.startsWith('/blob/')) {
      const bytes = await store.readPublic(decodeURIComponent(url.pathname.slice('/blob/'.length)));
      res.writeHead(bytes ? 200 : 404, { 'content-type': 'application/octet-stream' });
      res.end(bytes ? Buffer.from(bytes) : undefined);
      return;
    }
    const handler = routes[url.pathname];
    if (!handler) {
      res.writeHead(404);
      res.end();
      return;
    }
    const chunks = [];
    let size = 0;
    for await (const c of req) {
      size += c.length;
      if (size > MAX_BODY) {
        res.writeHead(413);
        res.end();
        return;
      }
      chunks.push(c);
    }
    const hasBody = req.method !== 'GET' && req.method !== 'HEAD';
    const request = new Request(url, { method: req.method, headers: req.headers, body: hasBody ? Buffer.concat(chunks) : undefined });
    const response = await handler(request, ctx);
    res.writeHead(response.status, Object.fromEntries(response.headers));
    res.end(Buffer.from(await response.arrayBuffer()));
  } catch (e) {
    console.error(e);
    res.writeHead(500);
    res.end();
  }
});
server.listen(port, '127.0.0.1', () => {
  const ops = ctx.lists.regtest ? ctx.lists.regtest.map((o) => o.name).join(', ') : 'none';
  console.log(`snapshot rendezvous stand-in on ${base}, store ${dir}, regtest operators: ${ops}, regtest pins: ${ctx.lists.pins.regtest.length}`);
});
````

- [ ] **Step 4: Write the failing test for the owner's clear**

Create `site/tests/unit/snapshotClearScript.test.ts`. It runs the command line as the owner runs it, in a child process, against a folder in the layout the stand-in serves:

````ts
import { describe, it, expect } from 'vitest';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { secp256k1 } from '@noble/curves/secp256k1.js';
import { parseManifest, serializeManifest, statementDigest } from '../../src/lib/snapshotManifest.mjs';
import { parseRegtestOperators } from '../../src/lib/snapshotOperators.mjs';
import { folderStore } from '../../src/lib/snapshotStore.mjs';
import { handleStatement, handleLatest, NODE_HEADER, NODE_HEADER_VALUE } from '../../src/lib/snapshotRoutes.mjs';

// The owner's command, run as the owner runs it, on a folder in the layout
// the local stand-in (scripts/snapshot-rendezvous-local.mjs) serves.
const CLI = fileURLToPath(new URL('../../../scripts/clear-snapshot-dispute.mjs', import.meta.url));
const BASE = 'http://127.0.0.1:29650';
const P = '0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464';
const fresh = () => {
  const sk = secp256k1.utils.randomSecretKey();
  return { sk, pub: Buffer.from(secp256k1.getPublicKey(sk, true)).toString('hex') };
};
const E = fresh();
const F = fresh();
const OPERATORS = `producer=${P};e=${E.pub};f=${F.pub}`;
const BASE_STATEMENT = parseManifest(new Uint8Array(readFileSync(new URL('../fixtures/snapshots/regtest-P.manifest', import.meta.url)))).statement;
/** The regtest statement at `height` with `coins` coins. */
const statementAt = (height: number, coins: number) => {
  const st = BASE_STATEMENT.slice();
  const b = Buffer.from(st.buffer, st.byteOffset, st.byteLength);
  b.writeInt32LE(height, 65);
  b.writeBigUInt64LE(BigInt(coins), 101);
  return st;
};
const signed = (st: Uint8Array, w: { sk: Uint8Array; pub: string }) =>
  serializeManifest({ statement: st, signatures: [{ key: w.pub, der: secp256k1.sign(statementDigest(st), w.sk, { prehash: false, format: 'der', lowS: true }) }] });

const setup = () => {
  const dir = mkdtempSync(join(tmpdir(), 'snapclear-'));
  const ctx = {
    store: folderStore(dir, `${BASE}/blob`),
    lists: { main: [], regtest: parseRegtestOperators(OPERATORS), pins: { main: [], regtest: [P] } },
    now: () => Date.now(),
    partBytes: 3000,
  };
  const post = (bytes: Uint8Array) =>
    handleStatement(new Request(`${BASE}/api/snapshots/statement`, { method: 'POST', headers: { [NODE_HEADER]: NODE_HEADER_VALUE }, body: bytes }), ctx);
  const latest = () => handleLatest(new Request(`${BASE}/api/snapshots/latest?chain=regtest`), ctx);
  return { dir, ctx, post, latest };
};
const cli = (args: string[], env: Record<string, string>) =>
  spawnSync(process.execPath, [CLI, ...args], { env: { PATH: process.env.PATH ?? '', ...env }, encoding: 'utf8' });

describe('scripts/clear-snapshot-dispute.mjs', () => {
  it('clears a disputed height in a stand-in\'s folder, closes it, and says what it moved', async () => {
    const { dir, ctx, post, latest } = setup();
    await post(signed(statementAt(200, 101), E));
    await post(signed(statementAt(200, 102), F));
    expect(await (await latest()).json()).toEqual({ disputed: [200] });
    const run = cli(['200', '--chain', 'regtest'], { SNAPSHOT_STORE_DIR: dir, SNAPSHOT_REGTEST_OPERATORS: OPERATORS, SNAPSHOT_REGTEST_PINS: P });
    expect(run.status, run.stderr).toBe(0);
    expect(run.stdout).toMatch(
      /^Cleared block 200 on regtest\. Moved 2 statements and 3 items in all to snapshots\/archive\/regtest-200-\d{8}T\d{9}Z\/\. The height is closed, and the next grid height starts clean\.\n$/,
    );
    expect((await latest()).status).toBe(404);
    expect(await ctx.store.list('snapshots/disputes/')).toEqual([]);
    expect(await (await post(signed(statementAt(200, 101), E))).json()).toEqual({ error: 'closed-height' });
  });
  it('changes nothing without a store, without a height, or where nothing is disputed', async () => {
    const { dir, ctx, post } = setup();
    await post(signed(statementAt(300, 101), E));
    const noStore = cli(['300', '--chain', 'regtest'], {});
    expect(noStore.status).toBe(1);
    expect(noStore.stderr).toContain('set BLOB_READ_WRITE_TOKEN');
    expect(cli([], { SNAPSHOT_STORE_DIR: dir }).status).toBe(2);
    const calm = cli(['300', '--chain', 'regtest'], { SNAPSHOT_STORE_DIR: dir, SNAPSHOT_REGTEST_OPERATORS: OPERATORS });
    expect(calm.status).toBe(1);
    expect(calm.stderr).toContain('block 300 on regtest has no open dispute, so there is nothing to clear');
    expect(await ctx.store.list('snapshots/closed/')).toEqual([]);
    expect(await ctx.store.list('snapshots/records/')).toHaveLength(1);
  });
});
````

- [ ] **Step 5: Run it to see it fail**

Run: `cd site && npx vitest run tests/unit/snapshotClearScript.test.ts`
Expected: FAIL: the first case's `run.status` is `1`, with `Cannot find module '…/scripts/clear-snapshot-dispute.mjs'` in `run.stderr`; the second case's `noStore.stderr` does not contain `set BLOB_READ_WRITE_TOKEN`.

- [ ] **Step 6: Write the owner's command**

Create `scripts/clear-snapshot-dispute.mjs`:

````js
#!/usr/bin/env node
// The owner's clear for a disputed snapshot height (easyNode's design,
// section 6a). No count of operators settles a disagreement, so only the
// owner clears one, and only here: no endpoint of the website can.
//
// It copies every statement, dissent, file and manifest at that height, and
// the dispute record, into snapshots/archive/ in the site's store, closes the
// height so a copy of an old statement or dissent cannot bring the dispute
// back, and then deletes the originals. The next grid height, 100 blocks
// later, starts clean. If it stops half way, run it again.
//
//   node scripts/clear-snapshot-dispute.mjs <height> [--chain main|regtest]
//
// Run from the repository root after `cd site && npm ci`, with
// BLOB_READ_WRITE_TOKEN set to the site's storage write token (easybtx.com),
// or with SNAPSHOT_STORE_DIR set to a local stand-in's folder (tests and
// easyNode's rehearsal).
import { readFileSync } from 'node:fs';
import { storeFromEnv } from '../site/src/lib/snapshotStore.mjs';
import { listsFromEnv } from '../site/src/lib/snapshotOperators.mjs';
import { clearHeight } from '../site/src/lib/snapshotRoutes.mjs';

const args = process.argv.slice(2);
const at = args.indexOf('--chain');
const chain = at >= 0 ? args[at + 1] : 'main';
const heightArg = args.find((a, i) => !a.startsWith('--') && (at < 0 || i !== at + 1));
if (!heightArg || !/^\d+$/.test(heightArg)) {
  console.error('usage: node scripts/clear-snapshot-dispute.mjs <height> [--chain main|regtest]');
  process.exit(2);
}

const store = await storeFromEnv(process.env);
if (!store) {
  console.error('There is no store to clear: set BLOB_READ_WRITE_TOKEN (the site\'s storage write token), or SNAPSHOT_STORE_DIR for a local stand-in.');
  process.exit(1);
}
const operatorFile = JSON.parse(readFileSync(new URL('../site/src/lib/snapshot-operators.json', import.meta.url), 'utf8'));
const ctx = { store, lists: listsFromEnv(process.env, operatorFile), now: () => Date.now() };

try {
  const r = await clearHeight(ctx, { chain, height: Number(heightArg) });
  console.log(
    `Cleared block ${r.height} on ${r.chain}. Moved ${r.statements.length} statements and ${r.moved} items in all to ${r.archive}. ` +
      'The height is closed, and the next grid height starts clean.',
  );
} catch (e) {
  console.error(String(e?.message || e));
  process.exit(1);
}
````

Add the command to the unit-test workflow, so a change to it alone runs the suite. In `.github/workflows/site-tests.yml`, under both `pull_request:` → `paths:` and `push:` → `paths:`, add the line `      - "scripts/clear-snapshot-dispute.mjs"` right after `      - "site/**"` (on `3dcfa37a` those are lines 29 and 34).

- [ ] **Step 7: Run it to see it pass**

Run: `cd site && npx vitest run tests/unit/snapshotClearScript.test.ts`
Expected: `Tests  2 passed (2)` (derived, not run).

- [ ] **Step 8: Walk the stand-in through a statement, a dispute and a clear by hand**

````bash
cd /Users/m2promende/repos/EasyBTX-snapshot-rendezvous/site
T=$(mktemp -d)/rv-local
P=0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464; C=02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f
F=tests/fixtures/snapshots; H=11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194; N='x-ebtx-node: ebtx-snapshot-v1'; O='content-type: application/octet-stream'
# Two throwaway keys, e and f, each signing an account of block 200; the two differ in the coin count.
EF=$(T=$T node --input-type=module -e '
import { secp256k1 } from "@noble/curves/secp256k1.js";
import { readFileSync, writeFileSync } from "node:fs";
import { parseManifest, serializeManifest, statementDigest } from "./src/lib/snapshotManifest.mjs";
const base = parseManifest(new Uint8Array(readFileSync("tests/fixtures/snapshots/regtest-P.manifest"))).statement;
const out = [];
for (const [name, coins] of [["e", 101n], ["f", 102n]]) {
  const st = base.slice();
  const b = Buffer.from(st.buffer, st.byteOffset, st.byteLength);
  b.writeInt32LE(200, 65);
  b.writeBigUInt64LE(coins, 101);
  const sk = secp256k1.utils.randomSecretKey();
  const key = Buffer.from(secp256k1.getPublicKey(sk, true)).toString("hex");
  const der = secp256k1.sign(statementDigest(st), sk, { prehash: false, format: "der", lowS: true });
  writeFileSync(`${process.env.T}.${name}.manifest`, serializeManifest({ statement: st, signatures: [{ key, der }] }));
  out.push(`${name}=${key}`);
}
console.log(out.join(";"));
')
OPS="producer=$P;confirmer=$C;$EF"
SNAPSHOT_REGTEST_OPERATORS="$OPS" SNAPSHOT_REGTEST_PINS=$P \
  node scripts/snapshot-rendezvous-local.mjs --port 29650 --dir $T > $T.log 2>&1 &
sleep 1; cat $T.log
curl -s -H "$N" -H "$O" --data-binary @$F/regtest-P.manifest http://127.0.0.1:29650/api/snapshots/statement; echo
U=$(curl -s -X POST -H "$N" "http://127.0.0.1:29650/api/snapshots/file?statement=$H&action=start" | python3 -c 'import json,sys;print(json.load(sys.stdin)["upload"])')
curl -s -X PUT -H "$N" -H "$O" --data-binary @$F/regtest-100.dat "http://127.0.0.1:29650/api/snapshots/file?statement=$H&upload=$U&part=1"; echo
curl -s -X POST -H "$N" "http://127.0.0.1:29650/api/snapshots/file?statement=$H&upload=$U&action=complete"; echo
curl -s -o /dev/null -w '%{http_code}\n' 'http://127.0.0.1:29650/api/snapshots/latest?chain=regtest'
curl -s -H "$N" -H "$O" --data-binary @$F/regtest-C.manifest http://127.0.0.1:29650/api/snapshots/statement; echo
curl -s 'http://127.0.0.1:29650/api/snapshots/latest?chain=regtest'; echo
FU=$(curl -s 'http://127.0.0.1:29650/api/snapshots/latest?chain=regtest' | python3 -c 'import json,sys;print(json.load(sys.stdin)["file_url"])'); curl -s "$FU" | shasum -a 256
curl -s 'http://127.0.0.1:29650/api/snapshots/disputes?chain=regtest'; echo
curl -s -H "$N" -H "$O" --data-binary @$T.e.manifest http://127.0.0.1:29650/api/snapshots/statement; echo
curl -s -H "$N" -H "$O" --data-binary @$T.f.manifest http://127.0.0.1:29650/api/snapshots/statement; echo
curl -s -w ' %{http_code}\n' 'http://127.0.0.1:29650/api/snapshots/latest?chain=regtest'
curl -s 'http://127.0.0.1:29650/api/snapshots/disputes?chain=regtest' | python3 -c 'import json,sys;print(json.load(sys.stdin)["disputes"][0]["alert"])'
(cd .. && SNAPSHOT_STORE_DIR=$T SNAPSHOT_REGTEST_OPERATORS="$OPS" node scripts/clear-snapshot-dispute.mjs 200 --chain regtest; echo "exit $?")
curl -s 'http://127.0.0.1:29650/api/snapshots/latest?chain=regtest' | python3 -c 'import json,sys;print(json.load(sys.stdin)["height"])'
curl -s -w ' %{http_code}\n' -H "$N" -H "$O" --data-binary @$T.e.manifest http://127.0.0.1:29650/api/snapshots/statement
pkill -f 'snapshot-rendezvous-local.mjs --port 29650'
````

Expected, in order (derived, not run): `snapshot rendezvous stand-in on http://127.0.0.1:29650, store …, regtest operators: producer, confirmer, e, f, regtest pins: 1`; the statement with `"operators":["producer"]` and `"file":"missing"`; `{"part":1,"bytes":8055}`; `{"stored":true,"file_url":"http://127.0.0.1:29650/blob/snapshots/100/11c5406e…f194.dat","file_sha256":"b2c5c43c…ce85","confirmed":false}`; `404` (one operator); the statement with `"operators":["producer","confirmer"]` and `"file":"stored"`; the pointer with `"manifest_sha256":"e8fa06d2f700feb488d7f494a532cd0e44799d20613b80532b2eca5cffaa746e"` (the regtest-PC vector) and `"file_sha256":"b2c5c43c4fd931475c4769926564644b6cff744a229c54623f5eb03c16e4ce85"`; `b2c5c43c…ce85  -`; `{"disputes":[]}`; e's statement with `"operators":["e"]`; f's with `"operators":["f"]`; `{"disputed":[200]} 200`; the alert, five lines, ending `They differ in: coin count.` and `Check both nodes, then clear it: node scripts/clear-snapshot-dispute.mjs 200 --chain regtest`; `Cleared block 200 on regtest. Moved 2 statements and 3 items in all to snapshots/archive/regtest-200-<time>/. The height is closed, and the next grid height starts clean.` and `exit 0`; `100` (the confirmed snapshot is served again); `{"error":"closed-height"} 422`.

- [ ] **Step 9: All checks, then commit**

````bash
cd /Users/m2promende/repos/EasyBTX-snapshot-rendezvous
(cd site && npx vitest run 2>&1 | tail -4)
bash scripts/check-download-links.sh
git add site/src/pages/api/snapshots site/scripts/snapshot-rendezvous-local.mjs scripts/clear-snapshot-dispute.mjs site/tests/unit/snapshotClearScript.test.ts .github/workflows/site-tests.yml
git commit -m "site: /api/snapshots routes, a local stand-in for easyNode's rehearsal, and the owner's dispute clear" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

Expected: vitest `Tests  333 passed (333)` (derived, not run: 250 before, 83 new); `check-download-links.sh` exits 0 (these files touch no download link, no `/updater` path and no `vercel.json` rule, which is all it and `check-node-links.py` look at).

### Task 7: Checks, then the owner's go for the pull request, the merge and production

Nothing in Tasks 1 to 6 leaves this machine. Everything outward-facing in this task waits for the owner, in that session: pushing the branch and opening the PR (GitHub shows it, and Vercel builds a preview) only when the owner says so at that time; merging, which deploys to production, only on the owner's go. Do not push, open, merge or deploy on a guess, and not because an earlier session was told yes.

**Files:** `scripts/check-snapshot-operators.mjs` (the pin, if Task 2 could not set it).

**Interfaces:**
- Consumes: Tasks 1 to 6; easynode `main` carrying `crates/btx-core/snapshot-operators.json` (core plan, Task 1, which merges only once the owner confirms Aleksander agreed).
- Produces, after the owner's go: `https://easybtx.com/api/snapshots/latest` answering `404 {"version":1,"confirmed":null}` until a statement is confirmed (two listed operators, a pinned key among them, its file stored), which is what every 0.7.0 app expects until then (it uses its fallbacks); `https://easybtx.com/api/snapshots/disputes` answering `{"disputes":[]}`.

- [ ] **Step 1: Everything local passes**

````bash
cd /Users/m2promende/repos/EasyBTX-snapshot-rendezvous
(cd site && npx vitest run 2>&1 | tail -4)
(cd site && npx astro build 2>&1 | tail -3)
bash scripts/check-download-links.sh; echo "links exit $?"
grep -rn "$(printf '\342\200\224')" site/src/lib/snapshot*.mjs site/src/lib/snapshot-operators.json site/src/pages/api/snapshots site/scripts/snapshot-rendezvous-local.mjs scripts/check-snapshot-operators.mjs scripts/clear-snapshot-dispute.mjs; echo "em-dash grep exit $?"
````

Expected: `Tests  333 passed (333)`; `[build] Complete!`; `links exit 0`; `em-dash grep exit 1` (no em-dash found; the `printf` spells U+2014 in UTF-8 so this plan carries none).

- [ ] **Step 2: The pin, and the live operator check**

If Task 2 left `EASYNODE_COMMIT` empty:

````bash
git -C /Users/m2promende/repos/easynode fetch -q origin main
PIN=$(git -C /Users/m2promende/repos/easynode log -1 --format=%H origin/main -- crates/btx-core/snapshot-operators.json); echo "pin: ${PIN:-none yet}"
git -C /Users/m2promende/repos/easynode show "$PIN:crates/btx-core/snapshot-operators.json" | cmp - site/src/lib/snapshot-operators.json && echo "same bytes at $PIN"
````

If it says `pin: none yet`, the core plan's Task 1 is not on easynode `main` (it waits for the owner's word on Aleksander): stop here and tell the owner. Otherwise set `const EASYNODE_COMMIT = '<PIN>';`, then:

````bash
node scripts/check-snapshot-operators.mjs; echo "exit $?"
node scripts/check-snapshot-operators.mjs --watch-main; echo "exit $?"
git add scripts/check-snapshot-operators.mjs
git commit -m "site: pin the snapshot operator check to easynode ${PIN:0:12}" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

Expected: `snapshot operators: the website's copy is easynode <PIN>, byte for byte: Mende (1 key), Aleksander (3 keys), jpp (1 key); 4 pinned keys` and `exit 0`, twice, the second time followed by `easynode main has the same list.` If the bytes differ, stop and ask the owner which file is right.

- [ ] **Step 3: STOP. Ask the owner whether to open the PR**

Say in one short message what is ready and ask, for example: "The website half of the snapshot network is ready on claude/snapshot-rendezvous: 333 site tests pass, the build completes, and the operator check matches easynode <PIN>. Open the PR? It would push the branch and Vercel would build a preview." Wait for a clear yes in that session. Without one, stop here and report.

Only on that yes:

````bash
git push -u origin claude/snapshot-rendezvous
gh pr create --repo MendeMatthias/EasyBTX --base main --head claude/snapshot-rendezvous \
  --title "site: /api/snapshots, the meeting point for confirmed chain snapshots" \
  --body "$(printf '%s\n' \
  'The website half of docs/decisions/2026-09-29-every-node-starts-near-the-tip.md, sections 6 and 6a (easynode).' \
  '' \
  '- POST /api/snapshots/statement takes a manifest only when every signature is valid, strict DER, low S and from a key on the operator list; the chain id, replay context and shielded commitment are the expected ones; the height is a multiple of 100 and not closed; the file is at most 64 MiB, or all four file fields are zero (a dissent). Co-signatures merge, one per key.' \
  '- The file goes up in parts of 4 MiB through /api/snapshots/file, never for a dissent, and is kept only when its size and double SHA-256 are the statement'"'"'s.' \
  '- GET /api/snapshots/latest names the newest statement two different operators signed, a key every node pins among them, with its file stored and checked. While listed operators disagree about any height it answers only {"disputed": [...]}, and every node uses its fallbacks.' \
  '- GET /api/snapshots/disputes lists open disputes with both accounts, who signed each, the fields that differ and the alert text for the owner. Only the owner clears one, with scripts/clear-snapshot-dispute.mjs and the store token; no endpoint does.' \
  "- The operator list is byte for byte easyNode's crates/btx-core/snapshot-operators.json at easynode ${PIN:0:12}, checked in CI and daily." \
  '- Please do not merge until the owner confirms Aleksander agreed to be on the list.' \
  '- Nothing here is trusted by the app: it checks every signature, its own node and the file hash again.' \
  '' \
  'Tests: 83 new vitest cases on the same vectors the app tests read.' \
  '' \
  '🤖 Generated with [Claude Code](https://claude.com/claude-code)')"
````

- [ ] **Step 4: Check the preview deployment (read-only)**

Find the preview URL on the PR (the Vercel bot comment), then:

````bash
P=<the preview URL from the PR, https://…vercel.app>
curl -s -w ' %{http_code}\n' "$P/api/snapshots/latest"
curl -s -w ' %{http_code}\n' "$P/api/snapshots/pending"
curl -s -w ' %{http_code}\n' "$P/api/snapshots/disputes"
curl -s -w ' %{http_code}\n' -X POST "$P/api/snapshots/statement" --data-binary @site/tests/fixtures/snapshots/regtest-PC.manifest
curl -s -w ' %{http_code}\n' -H 'x-ebtx-node: ebtx-snapshot-v1' -H 'content-type: application/octet-stream' --data-binary @site/tests/fixtures/snapshots/mainnet-225927.manifest "$P/api/snapshots/statement"
````

Expected: `{"version":1,"confirmed":null} 404`; `{"version":1,"chain":"main","statements":[]} 200` (or `503` if the preview has no Blob token, which is fine for a preview); `{"disputes":[]} 200` (or `503`); ` 403` (no header); `{"error":"height 225927 is not a multiple of 100"} 422` (or `503` without a token). Both POSTs are refused by design and write nothing; post nothing a preview could accept, since a preview may share the production Blob store. If the preview is behind Vercel's deployment protection, open the URL in the browser once or use the bypass token the project already uses; do not change project settings.

- [ ] **Step 5: STOP. Merging and production need the owner's go**

Do not merge. Merging this PR deploys it to easybtx.com. Tell the owner the preview answers and what the merge needs: the owner's confirmation that Aleksander agreed; `Snapshot operators`, `Site unit tests` and `Site download links` green on the PR. Only when the owner says go (he merges, or tells you to merge, squash as the repository does), then check production, read-only:

````bash
curl -s -w ' %{http_code}\n' https://easybtx.com/api/snapshots/latest
curl -s -w ' %{http_code}\n' https://easybtx.com/api/snapshots/pending
curl -s -w ' %{http_code}\n' https://easybtx.com/api/snapshots/disputes
````

Expected: `{"version":1,"confirmed":null} 404`, `{"version":1,"chain":"main","statements":[]} 200` and `{"disputes":[]} 200`. The first real statement arrives when a 0.7.0 producer on the list has exported at a multiple of 100 and that block is 144 deep on its node; `latest` stays 404 until a second listed operator has signed it and one of the signatures is from a pinned key (today only the 3060's is both listed and pinned).

## Rollback

- `latest` answering 404 makes every node use its fallbacks (design section 9); to force it, delete `site/src/pages/api/snapshots/latest.ts` in a revert PR, or delete the `snapshots/` blobs in the Vercel Blob browser. A standing dispute does the same on its own. Nothing on a node depends on the other routes to start.
- A dispute: only `node scripts/clear-snapshot-dispute.mjs <height>` with `BLOB_READ_WRITE_TOKEN` clears it. The archive keeps what it moved; reopening a closed height means deleting `snapshots/closed/<chain>-<height>.json` by hand, which the owner decides.
- Changing the operator list: the change lands in easyNode's `crates/btx-core/snapshot-operators.json` and ships in an app release; then, in one PR here, copy that file over `site/src/lib/snapshot-operators.json` and move `EASYNODE_COMMIT` to the easynode commit that carries it. The operator check lets it through only when the two are the same bytes.

## Risks and open points (for the owner)

1. **The design's upload path does not fit Vercel.** Section 6 says "Vercel Blob multipart". A function takes about 4.5 MB per request and Blob multipart parts must be at least 5 MB except the last (`@vercel/blob` 2.4.0, `uploadPart`), so parts cannot pass through a function as multipart parts. This plan keeps each part as a private blob and joins them in the `complete` step (read back, hash, one `put`). Same effect, one more read of the file per upload (at most 64 MiB, within the 60-second function limit, `maxDuration: 60` in `site/astro.config.mjs`; about 9 MB today).
2. **Presigned direct uploads exist** (`issueSignedToken` and `presignUrl` in 2.4.0), which would skip the function for the bytes, but the wire format for a non-SDK client (the app is Rust) is not documented; not used.
3. **The file is read twice per upload** (parts in, whole file out) and `complete` holds the whole file in memory: at most 64 MiB against the function's memory, today 9 MB.
4. **The alert reaches the owner only through Orca.** The site writes it into the dispute record, and `GET /api/snapshots/disputes` serves it; nothing here sends it. Until Orca (another project) reads that route, the owner learns of a dispute by looking, and Fast-forward stays off for every node meanwhile.
5. **One listed operator can freeze Fast-forward on purpose** with a dissent. That is what "no majority" means (design, section 6a); the alert names who dissented. An outsider cannot: an unlisted key is refused at intake and disputes nothing.
6. **The clear's read of a public blob on Vercel** (`get` with `access: 'public'`, Task 4) was not run against the real store; the tests use a fake SDK and the folder store. The clear copies everything before it closes the height or deletes anything, so if that call is refused the clear stops and nothing changes.
7. **Junk filter, not authentication.** The header keeps scanners out; anyone can still read `pending`, `latest` and `disputes`, which only hold public data (statements, public keys, signatures, operator names that are published anyway). Writes need signatures from listed keys, so only operators can create records, and a file can only be as large as a stored statement says.
8. **Each statement POST reads every record** (the dispute check and storage), as the first version's pruning already did. At a week of statements, a few hundred small blobs, that fits the function's time; if it grows, keep an index.
9. **`closed-height` is a code, not a sentence**, so the app can match it exactly; the other refusals stay sentences. The app plan should treat `422 {"error":"closed-height"}` as final for that height.
10. **Regtest's shielded commitment is only checked for being there** (its pool is open, so it moves with the chain); mainnet's must be the compiled value.

## Self-review

- Spec coverage (sections 6 and 6a, and the website half of section 1): intake with every rule, including the shielded commitment, closed heights and dissents (Task 3 `acceptManifest`, Task 5 `handleStatement`); merging, one per key (Task 3 `mergeSignatures`, Task 5 concurrency test); the file in parts, kept only if size and hash match, only for a stored statement that is not a dissent and has a listed signer (Task 5 `handleFile`); `pending` for seven days with who signed, dissents and disputes marked (Tasks 3, 5); `latest` in the app's contract, two different operators, a pinned key, file stored and checked, `{"disputed": [...]}` under a dispute (Tasks 3, 5, 6); the dispute rule, the record, the alert text and `GET /api/snapshots/disputes` (Tasks 3, 5, 6); storage kept to five confirmed pairs, ten pending files and seven days, nothing at a disputed height (Task 3 `storagePlan`, Task 5 `prune`); the owner-only clear and closed heights (Task 5 `clearHeight`, Task 6 the command and its test on the stand-in's folder); the operator file and its byte check against easyNode at a pinned commit (Task 2); `@noble/curves` as a direct dependency (Task 1); the same vectors as the app, plus the dissent vector built from the 225,927 statement (Task 1). No depth logic: the website checks no confirmations.
- Sabotage tests, one per rule: a statement from an unlisted key, alone or beside listed ones; a merged co-signature from an unlisted key; a file whose hash does not match; `latest` never pointing at a one-operator statement, at a two-operator statement without a pinned signature (the older pinned one served instead), or at a dissent; off-grid heights on both chains (the real 225,927 manifest, 150 and 226,050); wrong chain, wrong replay context, empty or wrong shielded commitment; chunk geometry, a zero size beside a file hash, a file too large; duplicate key, invalid signature, high S, non-strict DER, trailing bytes, oversize body; a key or a pin taken off the list; a part of the wrong size; a file for a dissent; a file-only difference not disputing; an outsider's conflict refused and changing nothing; a dissent disputing only when the chain facts differ; a disputed height turning `latest` off at every height; a resubmitted statement at a closed height refused; the clear refusing a height without a dispute and changing nothing.
- Placeholders: two values are filled in when the tasks run: `EASYNODE_COMMIT` (Task 2, Step 6, or Task 7, Step 2), which exists only once the app plan's list task is on easynode `main`, and the preview URL, which exists only after the owner says to open the PR.
- Not run as written: this amendment's code was written against EasyBTX `origin/main` `3dcfa37a`, and vitest, npm, `astro build` and the stand-in were not run (the amending session could not install packages). What was checked: every `.mjs` and `.ts` block parses (`node --check`, types stripped for `.ts`); and the five library modules, joined in memory with the curve stubbed to accept every strict-DER signature and a Map in place of the folder store, passed 145 checks mirroring Tasks 1 to 3 and the Task 5 handler tests (intake, dissents, `latest` with pins, disputes and the exact alert, storage, the clear, closed heights). Not covered by that: real signature verification, `folderStore`, the Astro routes, the CLI child process. Test counts are derived by counting cases: 10, 8, 45, 6, 12 and 2, 83 new, 333 in all with the 250 there before. The two dissent statement hashes were computed independently in Python from the 225,927 bytes; the operator file's size and SHA-256 were computed from its content.
- Consistency: the handler names, `PART_BYTES`, `NODE_HEADER_VALUE`, the reply keys and the record shape are the ones the app plan's client (Task 4) reads, plus the new `dissent` in `pending`, `dropped` as a file state, `{"disputed": [...]}` from `latest`, `{"error":"closed-height"}` from intake, and `SNAPSHOT_REGTEST_PINS` for the rehearsal's stand-in; the network-app and core plans take those.
