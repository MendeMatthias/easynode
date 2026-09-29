# Snapshot Network, website side: the meeting point on easybtx.com (Implementation Plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** easybtx.com takes signed snapshot statements and their files from producers, co-signatures from confirmers, and points every easyNode at the newest statement two different operators signed, while trusting nothing and storing little.

**Architecture:** Four Astro API routes (`site/src/pages/api/snapshots/{statement,file,pending,latest}.ts`) are one-line wrappers around handlers in `site/src/lib/snapshotRoutes.mjs`, which read and write a store (`snapshotStore.mjs`: Vercel Blob in production, a folder in tests and in the local stand-in) around pure rules (`snapshotRendezvous.mjs`: what is accepted, how signatures merge, what `latest` names, what is pruned). `snapshotManifest.mjs` reads the engine's manifest exactly as easyNode's `confirmed_snapshot.rs` does and verifies strict-DER low-S secp256k1 signatures with `@noble/curves`; `snapshotOperators.mjs` holds the rules for the checked-in copy of easyNode's operator list, and `scripts/check-snapshot-operators.mjs` fails CI when that copy differs from easyNode's. `site/scripts/snapshot-rendezvous-local.mjs` serves the same handlers over `node:http` for easyNode's end-to-end rehearsal.

**Tech Stack:** Astro 5 on Vercel (Node-runtime functions, `export const prerender = false`), `@vercel/blob` 2.4 (`get`, `put` with `ifMatch`, `list`, `del`), `@noble/curves` 2.2 (secp256k1), `node:crypto` (SHA-256), vitest 4.

**This is the website half of the snapshot network.** The app half is `2026-09-29-snapshot-network-app.md` (same folder). Its Task 1 publishes `crates/btx-core/snapshot-operators.json` on easynode's `main`; this plan's Task 2 checks against that file, so that app task lands on easynode `main` before this plan's PR merges (it is a data file and a test, safe to merge on its own). The app's end-to-end rehearsal (app plan, Task 8) runs this plan's `site/scripts/snapshot-rendezvous-local.mjs`, so this plan's Tasks 1 to 6 come before it.

## Global Constraints

- Design: `docs/decisions/2026-09-29-every-node-starts-near-the-tip.md` on easynode's `origin/claude/cosigned-snapshots` (read it with `git -C /Users/m2promende/repos/easynode show origin/claude/cosigned-snapshots:docs/decisions/2026-09-29-every-node-starts-near-the-tip.md`), approved 2026-09-29 with its choices as proposed. Implemented here: section 6, and the website half of section 1 (the copy of the list and its check). "Nothing here is trusted by the app. It filters, stores and points."
- Repository: `MendeMatthias/EasyBTX` (private). Work in a new worktree, never in `/Users/m2promende/repos/EasyBTX` itself (the owner's working tree): `git -C /Users/m2promende/repos/EasyBTX fetch -q origin main && git -C /Users/m2promende/repos/EasyBTX worktree add ../EasyBTX-snapshot-rendezvous -b claude/snapshot-rendezvous origin/main` (base `e187d14c` or later). All paths below are relative to that worktree.
- The pointer contract is easyNode's `attested_snapshot::ConfirmedPointer` (confirmed-snapshots core plan, Task 3; fixture `crates/btx-core/tests/fixtures/confirmed_snapshot/latest.json`): `GET /api/snapshots/latest` answers HTTP 200, `content-type: application/json`, the object `{version, chain, height, block_hash, statement_hash, manifest_url, manifest_size, manifest_sha256, file_url, file_size, file_sha256, file_hash, operators, confirmed_at}` in that key order; HTTP 404 with `{"version":1,"confirmed":null}` when nothing is confirmed; `cache-control: public, max-age=60, s-maxage=60`. `manifest_url` and `file_url` are `https://<store>.public.blob.vercel-storage.com/...` in production. `block_hash`, `statement_hash` and `file_hash` are display hex (byte-reversed, as btxd prints them); `manifest_sha256` and `file_sha256` are plain SHA-256 of the bytes.
- The routes, exactly:
  - `POST /api/snapshots/statement`, body the raw manifest bytes (at most 65,536), header `x-ebtx-node: ebtx-snapshot-v1`. 200 `{statement_hash, chain, height, signers, operators, added, file}` (`file` is `missing`, `uploading` or `stored`); 400 empty; 403 no header; 405; 413 too big; 422 `{error}`; 503 no store.
  - `POST /api/snapshots/file?statement=<hash>&action=start` → 200 `{upload, part_bytes, parts}`; 404 unknown statement; 409 file already stored; 422 no listed signer left.
  - `PUT /api/snapshots/file?statement=<hash>&upload=<id>&part=<n>`, body exactly the bytes of part `n` (parts of 4,194,304 bytes, the last one the rest) → 200 `{part, bytes}`; 404 no such upload; 422 wrong size or number; 413 over 4 MiB.
  - `POST /api/snapshots/file?statement=<hash>&upload=<id>&action=complete` → 200 `{stored: true, file_url, file_sha256, confirmed}`; 422 `{"error":"the file does not match the statement"}` (parts deleted, nothing kept) or `part <n> is missing`.
  - `GET /api/snapshots/pending?chain=main|regtest` (default `main`) → 200 `{version: 1, chain, statements: [{statement_hash, height, block_hash, manifest_hex, signers, operators, file, first_seen, confirmed, disputed}]}`, statements first seen in the last 7 days, highest first; `cache-control: public, max-age=30, s-maxage=30`; 400 other chains.
  - `GET /api/snapshots/latest?chain=main|regtest` (default `main`), the contract above.
- Accepted statement (all of section 6): parses strictly (version 2, canonical lengths, 33-byte keys, signatures of at most 72 bytes, no trailing bytes); chain id is mainnet's genesis `75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601` or, only where a test list is set, regtest's `521ad0951ed299e9c56aeb7db8188972772067560351b8e55adf71dbed532360`; replay context mainnet `32ad5c2e148149752a312561dc0b6879c9cc41fdf4bc09edcdd5e2bd09af7188`, regtest `9ed2add89d64a66015d6c4b2a746115c00c78503088fdde49e15ddcafed8a577`; shielded commitment not zero; file 1 to 67,108,864 bytes with chunk size 65,536 to 4,194,304 and `chunk_count = 1 + (size - 1) / chunk`; height a positive multiple of **200** (mainnet) or **100** (regtest); at least one signature; no key twice; **every** signature strict DER, low S, valid over the statement hash, from a key on that chain's list. Statement hash = double SHA-256 of `0x28 || "BTX_TRUSTED_UTXO_SNAPSHOT_ATTESTATION_V2" || statement`.
- Confirmed = signed by at least **2 different operators under the list as it is at the moment of reading** (all of one operator's keys count once) and the file stored after its size and double SHA-256 matched the statement. `latest` names the highest confirmed height of the chain; a height with two different confirmed statements is skipped (and flagged `disputed` in `pending`). `latest` never names a statement one operator signed.
- The mainnet operator list is a checked-in copy, `site/src/data/snapshot-operators.json`, **Mende only** (`02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675`) until the owner confirms Aleksander and jpp agreed. It must equal easyNode's `crates/btx-core/snapshot-operators.json` byte for byte; `scripts/check-snapshot-operators.mjs` checks that against `https://raw.githubusercontent.com/MendeMatthias/easynode/main/crates/btx-core/snapshot-operators.json` (easynode is public). The regtest list comes from `SNAPSHOT_REGTEST_OPERATORS` (easyNode's `name=key[,key];name=key` format) and is ignored when `VERCEL_ENV=production`.
- Storage (Vercel Blob, the project's existing `BLOB_READ_WRITE_TOKEN`): `snapshots/records/<statement_hash>.json` private, rewritten with `ifMatch` (up to 4 tries); `snapshots/uploads/<statement_hash>/<upload>/part-<nnnn>` private, deleted after `complete`; `snapshots/<height>/<statement_hash>.dat` and `snapshots/<height>/<statement_hash>-<signature count>.manifest` public, `cacheControlMaxAge` one year (content-addressed, never rewritten with other bytes). Kept: the 5 newest confirmed statements per chain and every statement first seen in the last 7 days; the rest is deleted on every write.
- Why parts of the website's own: a Vercel function takes a request body of about 4.5 MB, and Blob's multipart API needs parts of at least 5 MB except the last (`uploadPart` docs in `@vercel/blob` 2.4.0), so a part that fits through a function cannot be a Blob multipart part. Parts are kept as private blobs and joined by the `complete` step, which writes the file with `put(..., { multipart: true })` above 8 MiB.
- Dependencies: add `@noble/curves` `~2.2.0` as a direct dependency (it is locked at 2.2.0 already, through `@noble/post-quantum`); SHA-256 comes from `node:crypto`. No other new package.
- CI in this repository: `cd site && npx vitest run` (`.github/workflows/site-tests.yml`), `python3 scripts/check-node-links.py` (`site-links.yml`), and the new `node scripts/check-snapshot-operators.mjs` (`snapshot-operators.yml`). Also run `cd site && npx astro build` before the PR.
- Commits end with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (use `git commit -m "<subject>" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"`).
- Words anyone reads (errors returned to a node, comments that explain): friendly, simple, no hype, no guarantees, no em-dashes.
- Shared test vectors, the same bytes easyNode's tests read (`crates/btx-core/tests/fixtures/confirmed_snapshot/` in easynode). Source: `S=/private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad`; if that folder is gone, copy them from the easynode branch `claude/confirmed-snapshots` (`git -C /Users/m2promende/repos/easynode show claude/confirmed-snapshots:crates/btx-core/tests/fixtures/confirmed_snapshot/<name> > <name>`). SHA-256, exactly:

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

  The mainnet manifest: height 225,927, statement hash `d3ee93122fb062baa00bfe5d8c586f03c619427e8a9253f8fae0c980c9aa0482`, file hash `f234192dba8bb29875778620259fc870fa1e245175c61c89ed82cd0c1feceb2c`, one signature by the 3060 (`02d5efca…4675`), valid. The regtest ones: height 100, block `bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46`, statement hash `11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194`, file 8,055 bytes with file hash `76cc3cebd9fc21d6107b11e196dffe3cf2672924ef82e5162882d9395f02058e`, signed by the spike's throwaway keys P `0343faeb…e464`, C `02c05d68…6c0f`, D `034694ab…6d0e` in the orders the file names say.

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `site/package.json`, `site/package-lock.json`, `site/bun.lock` | modify | `@noble/curves` `~2.2.0` as a direct dependency (one line each) |
| `site/src/lib/snapshotManifest.mjs` | create | Read and write the engine's manifest, statement fields, statement hash, strict DER, low-S verification, file hashes |
| `site/src/data/snapshot-operators.json` | create | The website's copy of easyNode's mainnet operator list |
| `site/src/lib/snapshotOperators.mjs` | create | Check a list, the regtest list from the environment, key to operator, distinct operators |
| `scripts/check-snapshot-operators.mjs`, `.github/workflows/snapshot-operators.yml` | create | Fail CI when the copy differs from easyNode's published list |
| `site/src/lib/snapshotRendezvous.mjs` | create | Pure rules: accept, merge, records, confirmed, `latest`, `pending`, pruning, parts |
| `site/src/lib/snapshotStore.mjs` | create | The store: Vercel Blob or a folder, one shape |
| `site/src/lib/snapshotRoutes.mjs` | create | The four handlers from `Request` to `Response` |
| `site/src/pages/api/snapshots/statement.ts`, `file.ts`, `pending.ts`, `latest.ts` | create | Astro routes, one line each around the handlers |
| `site/scripts/snapshot-rendezvous-local.mjs` | create | Local stand-in serving the same handlers over `node:http`, for easyNode's rehearsal |
| `site/tests/fixtures/snapshots/*` | create | The shared vectors (8 files) |
| `site/tests/unit/snapshotManifest.test.ts`, `snapshotOperators.test.ts`, `snapshotRendezvous.test.ts`, `snapshotStore.test.ts`, `snapshotRoutes.test.ts` | create | vitest |

## Tasks

### Task 1: Read and verify a manifest (`snapshotManifest.mjs`), with the shared vectors

**Files:**
- Modify: `site/package.json`, `site/package-lock.json`, `site/bun.lock` (one line each)
- Create: `site/tests/fixtures/snapshots/` (8 files)
- Create: `site/src/lib/snapshotManifest.mjs`
- Test: `site/tests/unit/snapshotManifest.test.ts`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces (Tasks 2 to 6): `STATEMENT_VERSION`, `STATEMENT_LEN` (229), `MAX_MANIFEST_BYTES` (65,536), `MAX_SIGNATURES` (64), `MAX_DER_LEN` (72), `MAX_FILE_BYTES` (67,108,864), `MIN_CHUNK`, `MAX_CHUNK`; `class ManifestError`; `displayHex(bytes) -> string`; `sha256(bytes)`, `sha256d(bytes)`; `statementFields(raw229) -> {version, chainId, blockHash, height, hashSerialized, coins, chainTx, shielded, replayContext, fileSize, fileHash, chunkSize, chunkCount}` (hashes in display hex); `statementDigest(raw229) -> Uint8Array(32)` (display it with `displayHex` for the statement hash); `parseManifest(bytes) -> {statement: Uint8Array, signatures: [{key: hex66, der: Uint8Array}]}` (throws `ManifestError`); `serializeManifest(m) -> Uint8Array`; `isStrictDer(der) -> boolean`; `isCompressedKey(hex) -> boolean`; `signatureIsValid(digest, keyHex, der) -> boolean`; `fileHashes(chunks) -> {size, sha256, fileHash}`.

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

Expected: `npm install` prints `up to date`; `bun install` prints `Saved bun.lock`; `git diff --stat` shows exactly `site/bun.lock | 1 +`, `site/package-lock.json | 1 +`, `site/package.json | 1 +`, each the line `"@noble/curves": "~2.2.0",` right before `"@noble/hashes"`. If anything else moved, `git checkout -- package.json package-lock.json bun.lock` and stop. If `bun` is not installed, add that same line by hand to `workspaces[""].dependencies` in `site/bun.lock`, right before `"@noble/hashes": "^2.3.0",`.

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

Expected: the eight sums in Global Constraints, exactly. (The manifest is binary despite the `.json` name the engine's examples use.)

- [ ] **Step 3: Write the failing test**

Create `site/tests/unit/snapshotManifest.test.ts`:

````ts
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { secp256k1 } from '@noble/curves/secp256k1.js';
import {
  parseManifest, serializeManifest, statementFields, statementDigest, displayHex,
  isStrictDer, isCompressedKey, signatureIsValid, fileHashes, ManifestError,
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
import { createHash } from 'node:crypto';
import { secp256k1 } from '@noble/curves/secp256k1.js';

export const STATEMENT_VERSION = 2;
export const STATEMENT_LEN = 229;
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
Expected: `Tests  8 passed (8)`. These reproduce the independent Python check: statement hash `d3ee9312…0482`, file hash `f234192d…eb2c`, the 3060's signature valid, the regtest statement hash `11c5406e…f194`, and they prove that a high-S twin, non-strict DER and the right signature under the wrong key are refused.

- [ ] **Step 7: Commit**

````bash
cd /Users/m2promende/repos/EasyBTX-snapshot-rendezvous
git add site/package.json site/package-lock.json site/bun.lock site/tests/fixtures/snapshots site/src/lib/snapshotManifest.mjs site/tests/unit/snapshotManifest.test.ts
git commit -m "site: read and verify signed snapshot manifests, with easyNode's test vectors" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 2: The operator list copy and its check (`snapshotOperators.mjs`)

**Files:**
- Create: `site/src/data/snapshot-operators.json`
- Create: `site/src/lib/snapshotOperators.mjs`
- Create: `scripts/check-snapshot-operators.mjs`, `.github/workflows/snapshot-operators.yml`
- Test: `site/tests/unit/snapshotOperators.test.ts`

**Interfaces:**
- Consumes: Task 1 (`isCompressedKey`).
- Produces (Tasks 3, 5, 6): `checkOperatorList(ops) -> {ok: true, list} | {ok: false, reason}` where `list` is `[{name, keys: [hex66]}]`; `parseRegtestOperators(raw) -> list | null`; `operatorOf(list, keyHex) -> name | null`; `distinctOperators(list, keys) -> names` (list order, each once); `listsFromEnv(env, mainFile) -> {main: list, regtest: list | null}`.

- [ ] **Step 1: Write the failing test**

Create `site/tests/unit/snapshotOperators.test.ts`:

````ts
import { describe, it, expect } from 'vitest';
import mainFile from '../../src/data/snapshot-operators.json';
import {
  checkOperatorList, parseRegtestOperators, operatorOf, distinctOperators, listsFromEnv,
} from '../../src/lib/snapshotOperators.mjs';

const MENDE = '02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675';
const ALEKS_2 = '03047189023913e1922c80c895ee2a9e2eff6df05438654749e1a4f95019578a24';
const ALEKS_3 = '02c9cfb77d7e4dce0cd6b7968fee1dd53d31ef06c764185e120c825fab8c0572a0';
const P = '0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464';
const C = '02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f';
const D = '034694ab29307fd4e46f3fc7a5115dd52b4143c473a5eb919e857eb5a12bbd6d0e';

describe('the checked-in mainnet list', () => {
  it('is Mende alone with the 3060, until the owner confirms another operator agreed', () => {
    const r = checkOperatorList(mainFile.main);
    expect(r.ok).toBe(true);
    const list = (r as any).list;
    expect(list).toEqual([{ name: 'Mende', keys: [MENDE] }]);
    expect(operatorOf(list, ALEKS_2)).toBeNull();
  });
});

describe('checkOperatorList', () => {
  it('counts two keys of one operator once', () => {
    const r: any = checkOperatorList([{ name: 'Mende', keys: [MENDE] }, { name: 'Aleksander', keys: [ALEKS_2, ALEKS_3] }]);
    expect(r.ok).toBe(true);
    expect(distinctOperators(r.list, [ALEKS_2, ALEKS_3])).toEqual(['Aleksander']);
    expect(distinctOperators(r.list, [ALEKS_3, MENDE, P])).toEqual(['Mende', 'Aleksander']);
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

describe('the test-chain list', () => {
  it('reads easyNode\'s format and refuses anything else', () => {
    const list = parseRegtestOperators(`producer=${P};confirmer=${C},${D}`)!;
    expect(list.map((o: any) => o.name)).toEqual(['producer', 'confirmer']);
    expect(operatorOf(list, D.toUpperCase())).toBe('confirmer');
    expect(parseRegtestOperators('producer')).toBeNull();
    expect(parseRegtestOperators(`a=${P};b=${P}`)).toBeNull();
    expect(parseRegtestOperators('')).toBeNull();
    expect(parseRegtestOperators(undefined)).toBeNull();
  });
  it('never reaches a production deployment', () => {
    const env = { SNAPSHOT_REGTEST_OPERATORS: `producer=${P};confirmer=${C}` };
    expect(listsFromEnv(env, mainFile).regtest).toHaveLength(2);
    expect(listsFromEnv({ ...env, VERCEL_ENV: 'production' }, mainFile).regtest).toBeNull();
    expect(listsFromEnv({}, mainFile).regtest).toBeNull();
    expect(listsFromEnv({ ...env, VERCEL_ENV: 'production' }, mainFile).main).toEqual([{ name: 'Mende', keys: [MENDE] }]);
  });
});
````

- [ ] **Step 2: Run it to see it fail**

Run: `cd site && npx vitest run tests/unit/snapshotOperators.test.ts`
Expected: FAIL, `Error: Cannot find module '../../src/data/snapshot-operators.json'`.

- [ ] **Step 3: Write the list and the module**

Create `site/src/data/snapshot-operators.json`. It is a copy of easyNode's `crates/btx-core/snapshot-operators.json` (app plan, Task 1) and must stay byte for byte the same, two-space indent, one newline at the end:

````json
{
  "version": 1,
  "main": [
    {
      "name": "Mende",
      "keys": [
        "02d5efca78b53c89e7e1672feda8a9b70937bba40b001413495e86e05f196c4675"
      ]
    }
  ]
}
````

Create `site/src/lib/snapshotOperators.mjs`:

````js
// Who may sign a chain snapshot the website stores, and how their keys group.
//
// The website keeps a COPY of the list easyNode compiles in
// (crates/btx-core/src/operators.rs, published as
// crates/btx-core/snapshot-operators.json), only to filter what it stores.
// The app never takes the website's word for anything: it checks every
// signature against its own compiled list again. scripts/check-snapshot-operators.mjs
// fails CI when the two copies differ.
//
// An operator is a person. All of one person's keys count once, so one
// machine with two keys can never confirm a snapshot alone.
import { isCompressedKey } from './snapshotManifest.mjs';

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
 * The lists this deployment checks against. Mainnet's is the checked-in copy.
 * The regtest list comes from SNAPSHOT_REGTEST_OPERATORS and never on a
 * production deployment, so a production site stores no test-chain statement.
 * @param {Record<string, string | undefined>} env
 * @param {unknown} mainFile the parsed site/src/data/snapshot-operators.json
 */
export function listsFromEnv(env, mainFile) {
  const main = checkOperatorList(/** @type {any} */ (mainFile)?.main);
  const production = env.VERCEL_ENV === 'production';
  return {
    main: main.ok ? main.list : [],
    regtest: production ? null : parseRegtestOperators(env.SNAPSHOT_REGTEST_OPERATORS),
  };
}
````

- [ ] **Step 4: Run the test to see it pass**

Run: `cd site && npx vitest run tests/unit/snapshotOperators.test.ts`
Expected: `Tests  5 passed (5)`.

- [ ] **Step 5: Write the cross-repository check**

Create `scripts/check-snapshot-operators.mjs`:

````js
#!/usr/bin/env node
// The website's copy of the snapshot operator list must be easyNode's, byte
// for byte.
//
// easyNode compiles the list into the app (crates/btx-core/src/operators.rs)
// and publishes it as crates/btx-core/snapshot-operators.json, which a test
// there keeps equal to the compiled one. The website keeps a copy
// (site/src/data/snapshot-operators.json) to decide which signatures it
// stores. If the website lists someone the app does not, the app simply
// ignores that signature. If the app lists someone the website does not,
// that operator's confirmations are refused here and nothing gets
// confirmed. Either way the two should never differ for long, so this
// fails CI until they are the same again.
//
//   node scripts/check-snapshot-operators.mjs
//   EASYNODE_REF=<branch or tag> node scripts/check-snapshot-operators.mjs
//   EASYNODE_OPERATORS_FILE=<path> node scripts/check-snapshot-operators.mjs   (offline)
import { readFileSync } from 'node:fs';

const REF = process.env.EASYNODE_REF || 'main';
const SOURCE = `https://raw.githubusercontent.com/MendeMatthias/easynode/${REF}/crates/btx-core/snapshot-operators.json`;
const LOCAL = new URL('../site/src/data/snapshot-operators.json', import.meta.url);

const local = readFileSync(LOCAL, 'utf8');
let theirs;
if (process.env.EASYNODE_OPERATORS_FILE) {
  theirs = readFileSync(process.env.EASYNODE_OPERATORS_FILE, 'utf8');
} else {
  let res;
  try {
    res = await fetch(SOURCE, { signal: AbortSignal.timeout(15_000) });
  } catch (e) {
    console.error(`::error::could not reach ${SOURCE}: ${e?.message || e}`);
    process.exit(1);
  }
  if (!res.ok) {
    console.error(`::error::${SOURCE} answered HTTP ${res.status}`);
    process.exit(1);
  }
  theirs = await res.text();
}

const names = (text) => {
  try {
    return (JSON.parse(text).main || []).map((o) => `${o.name} (${o.keys.join(', ')})`);
  } catch {
    return ['<not JSON>'];
  }
};
if (theirs !== local) {
  console.error('::error::site/src/data/snapshot-operators.json differs from easyNode\'s crates/btx-core/snapshot-operators.json');
  console.error(`  website: ${names(local).join('; ')}`);
  console.error(`  easyNode (${REF}): ${names(theirs).join('; ')}`);
  console.error('  Copy easyNode\'s file over the website\'s once the app that carries it has shipped.');
  process.exit(1);
}
console.log(`snapshot operators: the website's copy is easyNode's (${REF}): ${names(local).join('; ')}`);
````

Create `.github/workflows/snapshot-operators.yml` (the action pins are the ones `site-tests.yml` uses):

````yaml
name: Snapshot operators

# The website stores a snapshot statement only when every signature on it is
# from a key on its copy of the operator list (site/src/data/snapshot-operators.json).
# easyNode compiles the real list into the app and publishes it as
# crates/btx-core/snapshot-operators.json. This fails when the two differ, on
# every PR that touches the copy or the check, and once a day, so a list
# changed on the app side does not sit unnoticed here.
#
# Security: no github.event.* content is interpolated into any run: step.

on:
  pull_request:
    branches: [main]
    paths:
      - "site/src/data/snapshot-operators.json"
      - "scripts/check-snapshot-operators.mjs"
      - ".github/workflows/snapshot-operators.yml"
  push:
    branches: [main]
    paths:
      - "site/src/data/snapshot-operators.json"
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

      - name: Compare
        run: node scripts/check-snapshot-operators.mjs
````

- [ ] **Step 6: Run the check both ways**

````bash
cd /Users/m2promende/repos/EasyBTX-snapshot-rendezvous
EASYNODE_OPERATORS_FILE=site/src/data/snapshot-operators.json node scripts/check-snapshot-operators.mjs; echo "exit $?"
sed 's/Mende/Someone/' site/src/data/snapshot-operators.json > /private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad/other-operators.json
EASYNODE_OPERATORS_FILE=/private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad/other-operators.json node scripts/check-snapshot-operators.mjs; echo "exit $?"
node scripts/check-snapshot-operators.mjs; echo "exit $?"
````

Expected: first `snapshot operators: the website's copy is easyNode's (main): Mende (02d5efca…4675)` and `exit 0`; second `::error::site/src/data/snapshot-operators.json differs from easyNode's …` with both lists named, and `exit 1`; third, the live fetch: `exit 0` once the app plan's Task 1 is on easynode `main`, and before that `::error::https://raw.githubusercontent.com/MendeMatthias/easynode/main/crates/btx-core/snapshot-operators.json answered HTTP 404` and `exit 1` (measured 2026-09-29: 404). That 404 is why the app task lands first; do not merge this PR while the live check fails.

- [ ] **Step 7: Commit**

````bash
git add site/src/data/snapshot-operators.json site/src/lib/snapshotOperators.mjs site/tests/unit/snapshotOperators.test.ts scripts/check-snapshot-operators.mjs .github/workflows/snapshot-operators.yml
git commit -m "site: a copy of easyNode's snapshot operator list, and a check that it stays the same" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 3: The rules (`snapshotRendezvous.mjs`)

**Files:**
- Create: `site/src/lib/snapshotRendezvous.mjs`
- Test: `site/tests/unit/snapshotRendezvous.test.ts`

**Interfaces:**
- Consumes: Task 1 (`parseManifest`, `serializeManifest`, `statementFields`, `statementDigest`, `displayHex`, `signatureIsValid`, `ManifestError`, `MAX_MANIFEST_BYTES`, `MAX_FILE_BYTES`, `MIN_CHUNK`, `MAX_CHUNK`), Task 2 (`operatorOf`, `distinctOperators`).
- Produces (Task 5): `CHAINS` (`{main, regtest}` each `{genesis, replayContext, grid}`), `PENDING_DAYS` (7), `KEEP_CONFIRMED` (5), `chainOf(chainIdHex)`, `acceptManifest(bytes, lists) -> {ok: true, chain, fields, hash, manifest} | {ok: false, reason}`, `mergeSignatures(stored, incoming) -> {manifest, added: [key]}`, `newRecord(accepted, nowIso)`, `withManifest(record, manifest, nowIso)`, `recordManifest(record)`, `recordOperators(record, lists)`, `isConfirmed(record, lists)`, `chooseLatest(records, chain, lists) -> record | null`, `disputedHeights(records, chain, lists)`, `latestView(record, lists)`, `pendingView(records, chain, nowMs, lists)`, `recordsToPrune(records, nowMs, lists) -> records`, `partPlan(fileSize, partBytes) -> {parts, sizeOf(n)}`.
- The record (private JSON, one per statement): `{version: 1, chain, statement_hash, height, block_hash, file_size, file_hash, manifest_hex, signers, first_seen, updated_at, file: {state: "missing"} | {state: "uploading", upload, started_at} | {state: "stored", url, sha256, stored_at}, public_manifest: null | {url, sha256, size, signatures}, confirmed_at: null | iso}`. Operators are never stored: they are worked out from `signers` under the list at the moment of reading.

- [ ] **Step 1: Write the failing test**

Create `site/tests/unit/snapshotRendezvous.test.ts`. One sabotage per rule: an unknown chain, another replay context, an empty shielded commitment, bad chunk geometry, a file over 64 MiB, off-grid heights (including the real 225,927 manifest), no signatures, a key twice, a signature that does not verify, a high-S twin, a key on no list; and for `latest`: one operator never, two operators without the file never, two keys of one operator once, a key taken off the list, a disputed height.

````ts
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { secp256k1 } from '@noble/curves/secp256k1.js';
import mainFile from '../../src/data/snapshot-operators.json';
import { parseManifest, serializeManifest, statementDigest } from '../../src/lib/snapshotManifest.mjs';
import { parseRegtestOperators, listsFromEnv } from '../../src/lib/snapshotOperators.mjs';
import {
  acceptManifest, mergeSignatures, newRecord, withManifest, recordOperators, isConfirmed,
  chooseLatest, latestView, pendingView, recordsToPrune, partPlan, CHAINS,
} from '../../src/lib/snapshotRendezvous.mjs';

const fx = (n: string) => new Uint8Array(readFileSync(new URL(`../fixtures/snapshots/${n}`, import.meta.url)));
const MAINNET = fx('mainnet-225927.manifest');
const R = (n: string) => fx(`regtest-${n}.manifest`);
const P = '0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464';
const C = '02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f';
const D = '034694ab29307fd4e46f3fc7a5115dd52b4143c473a5eb919e857eb5a12bbd6d0e';
const NOW = Date.parse('2026-10-01T12:00:00Z');
const iso = (msAgo: number) => new Date(NOW - msAgo).toISOString();
const DAY = 24 * 3600 * 1000;

const regtestLists = () => ({
  main: listsFromEnv({}, mainFile).main,
  regtest: parseRegtestOperators(`producer=${P};confirmer=${C};third=${D}`),
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
});
const sign = (statement: Uint8Array, ...who: { sk: Uint8Array; pub: string }[]) =>
  serializeManifest({
    statement,
    signatures: who.map((w) => ({
      key: w.pub,
      der: secp256k1.sign(statementDigest(statement), w.sk, { prehash: false, format: 'der', lowS: true }),
    })),
  });
/** The regtest statement with `edit` applied to a copy of its 229 bytes. */
const edited = (edit: (st: Uint8Array) => void, base = R('P')) => {
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
    expect(r.chain).toBe('regtest');
    expect(r.hash).toBe('11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194');
    expect(r.fields.height).toBe(100);
  });
  it('takes no test-chain statement where no test list is set', () => {
    expect(acceptManifest(R('PC'), { main: regtestLists().main, regtest: null })).toEqual({
      ok: false, reason: 'chain 521ad0951ed299e9c56aeb7db8188972772067560351b8e55adf71dbed532360 is not one this website takes',
    });
  });
  it('refuses the real 225,927 manifest: its height is off the grid of 200', () => {
    expect(acceptManifest(MAINNET, regtestLists())).toEqual({ ok: false, reason: 'height 225927 is not a multiple of 200' });
  });
  it('takes a mainnet statement at a grid height signed by listed keys', () => {
    const st = edited((s) => setI32(s, 65, 226_000), MAINNET);
    const r: any = acceptManifest(sign(st, A, B), freshLists());
    expect(r.ok, r.reason).toBe(true);
    expect(r.chain).toBe('main');
  });
  // One sabotage per rule. Each statement is otherwise the valid one above.
  const cases: [string, (s: Uint8Array) => void, string][] = [
    ['an unknown chain', (s) => s.fill(7, 1, 33), 'is not one this website takes'],
    ['another replay context', (s) => { s[149] ^= 1; }, 'the replay context is not this engine\'s'],
    ['an empty shielded commitment', (s) => s.fill(0, 117, 149), 'the shielded commitment is empty'],
    ['chunks that do not add up', (s) => setU32(s, 225, 2), 'the file size and chunks do not add up'],
    ['a chunk size below the engine\'s', (s) => setU32(s, 221, 1024), 'the file size and chunks do not add up'],
    ['a file over 64 MiB', (s) => { setU64(s, 181, 64 * 1024 * 1024 + 1); setU32(s, 221, 4 * 1024 * 1024); setU32(s, 225, 17); }, 'is more than this website stores'],
    ['an off-grid height', (s) => setI32(s, 65, 150), 'height 150 is not a multiple of 100'],
    ['height zero', (s) => setI32(s, 65, 0), 'height 0 is not a multiple of 100'],
  ];
  for (const [what, edit, reason] of cases) {
    it(`refuses ${what}`, () => {
      const r: any = acceptManifest(sign(edited(edit), A, B), freshLists());
      expect(r.ok).toBe(false);
      expect(r.reason).toContain(reason);
    });
  }
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

/** A record as the routes keep it, from a regtest vector. */
const rec = (vector: string, opts: { stored?: boolean; ago?: number; height?: number; hash?: string } = {}) => {
  const a: any = acceptManifest(R(vector), regtestLists());
  let r: any = newRecord(a, iso(opts.ago ?? 0));
  if (opts.height) r = { ...r, height: opts.height };
  if (opts.hash) r = { ...r, statement_hash: opts.hash };
  if (opts.stored) {
    r = {
      ...r,
      file: { state: 'stored', url: 'https://x.public.blob.vercel-storage.com/f.dat', sha256: 'b2'.repeat(32), stored_at: iso(0) },
      public_manifest: { url: 'https://x.public.blob.vercel-storage.com/m.manifest', sha256: 'aa'.repeat(32), size: 440, signatures: r.signers.length },
      confirmed_at: iso(0),
    };
  }
  return r;
};

describe('latest never points at a statement one operator signed', () => {
  const lists = regtestLists();
  it('one operator, even with its file stored: nothing', () => {
    expect(recordOperators(rec('P', { stored: true }), lists)).toEqual(['producer']);
    expect(chooseLatest([rec('P', { stored: true })], 'regtest', lists)).toBeNull();
  });
  it('two operators without the file: nothing', () => {
    expect(chooseLatest([rec('PC')], 'regtest', lists)).toBeNull();
  });
  it('two keys of one operator count once', () => {
    const one = { main: [], regtest: parseRegtestOperators(`producer=${P},${C}`) };
    expect(isConfirmed(rec('PC', { stored: true }), one)).toBe(false);
  });
  it('two operators with the file stored: the pointer, in the app\'s contract', () => {
    const r = rec('PC', { stored: true });
    expect(chooseLatest([r, rec('P', { stored: true, height: 200, hash: 'ff'.repeat(32) })], 'regtest', lists)).toBe(r);
    expect(Object.keys(latestView(r, lists))).toEqual([
      'version', 'chain', 'height', 'block_hash', 'statement_hash', 'manifest_url', 'manifest_size',
      'manifest_sha256', 'file_url', 'file_size', 'file_sha256', 'file_hash', 'operators', 'confirmed_at',
    ]);
    expect(latestView(r, lists)).toMatchObject({ version: 1, chain: 'regtest', height: 100, file_size: 8055, operators: ['producer', 'confirmer'] });
  });
  it('a key taken off the list stops counting at once', () => {
    const shrunk = { main: [], regtest: parseRegtestOperators(`producer=${P}`) };
    expect(chooseLatest([rec('PC', { stored: true })], 'regtest', shrunk)).toBeNull();
  });
  it('a height where two statements are confirmed is skipped, the one below is served', () => {
    const low = rec('PC', { stored: true });
    const high1 = rec('PC', { stored: true, height: 200, hash: '01'.repeat(32) });
    const high2 = rec('CD', { stored: true, height: 200, hash: '02'.repeat(32) });
    expect(chooseLatest([low, high1, high2], 'regtest', lists)).toBe(low);
    expect(pendingView([low, high1, high2], 'regtest', NOW, lists).statements.filter((s: any) => s.disputed)).toHaveLength(2);
  });
  it('only the asked chain', () => {
    expect(chooseLatest([rec('PC', { stored: true })], 'main', lists)).toBeNull();
  });
});

describe('pending, pruning and parts', () => {
  const lists = regtestLists();
  it('lists statements of the last seven days, newest height first, with who signed', () => {
    const view = pendingView([rec('P', { ago: 8 * DAY }), rec('PC', { ago: DAY, height: 200 }), rec('C', { ago: 0 })], 'regtest', NOW, lists);
    expect(view.statements.map((s: any) => [s.height, s.operators])).toEqual([[200, ['producer', 'confirmer']], [100, ['confirmer']]]);
    expect(view.statements[0]).toMatchObject({ file: 'missing', confirmed: false, disputed: false });
    expect(view.statements[0].manifest_hex).toBe(Buffer.from(R('PC')).toString('hex'));
  });
  it('keeps the five newest confirmed pairs and everything younger than seven days', () => {
    const old = [1, 2, 3, 4, 5, 6, 7].map((i) => rec('PC', { stored: true, ago: 30 * DAY, height: 100 * i, hash: String(i).padStart(64, '0') }));
    const young = rec('P', { ago: DAY, height: 50 * 100, hash: 'aa'.repeat(32) });
    const stale = rec('P', { ago: 9 * DAY, height: 60 * 100, hash: 'bb'.repeat(32) });
    const gone = recordsToPrune([...old, young, stale], NOW, lists).map((r: any) => r.height).sort((a: number, b: number) => a - b);
    expect(gone).toEqual([100, 200, 6000]);
  });
  it('splits a file into parts of the size the routes take', () => {
    const plan = partPlan(9_045_522, 4 * 1024 * 1024);
    expect(plan.parts).toBe(3);
    expect([plan.sizeOf(1), plan.sizeOf(2), plan.sizeOf(3), plan.sizeOf(4), plan.sizeOf(0)]).toEqual([4_194_304, 4_194_304, 656_914, -1, -1]);
    expect(partPlan(8055, 4096).parts).toBe(2);
    expect(CHAINS.main.grid).toBe(200);
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
// docs/decisions/2026-09-29-every-node-starts-near-the-tip.md, section 6, in
// easyNode's repository. Producers send a signed statement and its file;
// confirmers fetch what is waiting, check it against their own node's diary,
// and send back one signature; mirrors read `latest`. NOTHING here is trusted
// by the app: it re-checks every signature against its own compiled list,
// its own node and the file's hash. The website only filters, stores and
// points, so that the one thing it can get wrong is to point at nothing.
import {
  parseManifest, serializeManifest, statementFields, statementDigest, displayHex,
  signatureIsValid, ManifestError, MAX_MANIFEST_BYTES, MAX_FILE_BYTES, MIN_CHUNK, MAX_CHUNK,
} from './snapshotManifest.mjs';
import { operatorOf, distinctOperators } from './snapshotOperators.mjs';

/** Each chain's genesis hash, replay context (btxd v0.34.9) and grid. The same values easyNode compiles. */
export const CHAINS = {
  main: {
    genesis: '75a998a39d2d6e25a9ca7de2cc659309c4105839c06cd435ba2b1aabf0fa4601',
    replayContext: '32ad5c2e148149752a312561dc0b6879c9cc41fdf4bc09edcdd5e2bd09af7188',
    grid: 200,
  },
  regtest: {
    genesis: '521ad0951ed299e9c56aeb7db8188972772067560351b8e55adf71dbed532360',
    replayContext: '9ed2add89d64a66015d6c4b2a746115c00c78503088fdde49e15ddcafed8a577',
    grid: 100,
  },
};
/** Statements are listed for confirmers this long after they first arrive. */
export const PENDING_DAYS = 7;
/** Confirmed pairs kept per chain, newest first. */
export const KEEP_CONFIRMED = 5;
const DAY_MS = 24 * 3600 * 1000;

/** @param {string} chainIdHex */
export function chainOf(chainIdHex) {
  for (const [name, c] of Object.entries(CHAINS)) if (c.genesis === chainIdHex) return name;
  return null;
}

const refuse = (reason) => ({ ok: false, reason });

/**
 * Whether the website stores this manifest: section 6's rules, every one.
 * Every signature valid (strict DER, low S) and from a key on the chain's
 * list; no key twice; the chain's genesis and replay context; the height on
 * the grid; a file of at most 64 MiB in the engine's chunk geometry.
 * @param {Uint8Array} bytes
 * @param {{ main: any[], regtest: any[] | null }} lists
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
  const list = chain === 'main' ? lists.main : chain === 'regtest' ? lists.regtest : null;
  if (!chain || !list) return refuse(`chain ${f.chainId} is not one this website takes`);
  const rules = CHAINS[chain];
  if (f.replayContext !== rules.replayContext) return refuse('the replay context is not this engine\'s');
  if (/^0+$/.test(f.shielded)) return refuse('the shielded commitment is empty');
  if (
    f.fileSize <= 0 || /^0+$/.test(f.fileHash) || f.chunkSize < MIN_CHUNK || f.chunkSize > MAX_CHUNK ||
    f.chunkCount !== 1 + Math.floor((f.fileSize - 1) / f.chunkSize)
  ) return refuse('the file size and chunks do not add up');
  if (f.fileSize > MAX_FILE_BYTES) return refuse(`a ${f.fileSize}-byte file is more than this website stores`);
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
  return { ok: true, chain, fields: f, hash: displayHex(digest), manifest };
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

/** The record kept for a statement the website took for the first time. */
export function newRecord(accepted, nowIso) {
  const f = accepted.fields;
  return {
    version: 1,
    chain: accepted.chain,
    statement_hash: accepted.hash,
    height: f.height,
    block_hash: f.blockHash,
    file_size: f.fileSize,
    file_hash: f.fileHash,
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
  const list = record.chain === 'main' ? lists.main : lists.regtest;
  return list ? distinctOperators(list, record.signers) : [];
}

/** Two different operators signed it and its file is stored and checked. */
export function isConfirmed(record, lists) {
  return recordOperators(record, lists).length >= 2 && record.file?.state === 'stored';
}

/**
 * The newest confirmed statement of a chain whose public manifest is written.
 * A height where two different statements are both confirmed means operators
 * disagree about the chain; it is skipped, not settled here.
 */
export function chooseLatest(records, chain, lists) {
  const byHeight = new Map();
  for (const r of records) {
    if (r.chain !== chain || !isConfirmed(r, lists)) continue;
    byHeight.set(r.height, [...(byHeight.get(r.height) || []), r]);
  }
  const heights = [...byHeight.keys()].sort((a, b) => b - a);
  for (const h of heights) {
    const at = byHeight.get(h);
    if (at.length === 1 && at[0].public_manifest) return at[0];
  }
  return null;
}

/** Heights of a chain where more than one statement is confirmed. */
export function disputedHeights(records, chain, lists) {
  const count = new Map();
  for (const r of records) if (r.chain === chain && isConfirmed(r, lists)) count.set(r.height, (count.get(r.height) || 0) + 1);
  return [...count].filter(([, n]) => n > 1).map(([h]) => h);
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

const ageMs = (record, nowMs) => nowMs - Date.parse(record.first_seen);

/** GET /api/snapshots/pending: what confirmers read, newest height first. */
export function pendingView(records, chain, nowMs, lists) {
  const disputed = new Set(disputedHeights(records, chain, lists));
  const statements = records
    .filter((r) => r.chain === chain && ageMs(r, nowMs) <= PENDING_DAYS * DAY_MS)
    .sort((a, b) => b.height - a.height || Date.parse(a.first_seen) - Date.parse(b.first_seen))
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
    }));
  return { version: 1, chain, statements };
}

/** Records to delete: storage stays the newest five confirmed pairs per chain and whatever is younger than seven days. */
export function recordsToPrune(records, nowMs, lists) {
  const keep = new Set();
  for (const chain of Object.keys(CHAINS)) {
    records
      .filter((r) => r.chain === chain && isConfirmed(r, lists))
      .sort((a, b) => b.height - a.height)
      .slice(0, KEEP_CONFIRMED)
      .forEach((r) => keep.add(r.statement_hash));
  }
  return records.filter((r) => !keep.has(r.statement_hash) && ageMs(r, nowMs) > PENDING_DAYS * DAY_MS);
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
Expected: `Tests  28 passed (28)`.

- [ ] **Step 5: Commit**

````bash
git add site/src/lib/snapshotRendezvous.mjs site/tests/unit/snapshotRendezvous.test.ts
git commit -m "site: the snapshot meeting point's rules: what is stored, merged, confirmed and pointed at" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 4: The store (`snapshotStore.mjs`)

**Files:**
- Create: `site/src/lib/snapshotStore.mjs`
- Test: `site/tests/unit/snapshotStore.test.ts`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces (Tasks 5, 6): `class StoreConflict`; a store is `{kind, getPrivate(path) -> {bytes, etag} | null, putPrivate(path, bytes, {etag}) -> {etag}, putPublic(path, bytes, contentType?) -> {url}, list(prefix) -> [path], del(paths)}` where `etag: string` replaces only an unchanged blob, `etag: null` creates only, `etag` absent overwrites, and a lost race throws `StoreConflict`; `folderStore(dir, publicBase)` (also `readPublic(path)`); `blobStore(sdk)`; `storeFromEnv(env) -> store | null` (`SNAPSHOT_STORE_DIR` only when `VERCEL` is unset; Blob when `BLOB_READ_WRITE_TOKEN` or `BLOB_STORE_ID` is set; else `null`).

- [ ] **Step 1: Write the failing test**

Create `site/tests/unit/snapshotStore.test.ts`. The Blob half runs against a fake SDK and checks exactly what is asked of Vercel: private reads past the cache (`useCache: false`), conditional writes (`ifMatch`), create-only (`allowOverwrite: false`), public immutable files, every page of a listing.

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
// * blobStore: Vercel Blob, what easybtx.com runs on. Records and upload
//   parts are PRIVATE blobs (read back with `get`, written with `ifMatch` so
//   two confirmers signing at once cannot lose a signature); the checked file
//   and the merged manifest are PUBLIC, immutable, content-addressed blobs,
//   which is what the app downloads.
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
    /** For the local stand-in: the bytes of a public blob. */
    readPublic: (path) => read(pub(path)),
  };
}

/**
 * Vercel Blob. `sdk` is `await import('@vercel/blob')`, passed in so tests can
 * check exactly what is asked of it.
 */
export function blobStore(sdk) {
  const conflict = (e) =>
    e instanceof sdk.BlobPreconditionFailedError || /already exists/i.test(String(e?.message || ''));
  return {
    kind: 'blob',
    async getPrivate(path) {
      const r = await sdk.get(path, { access: 'private', useCache: false });
      if (!r || r.statusCode !== 200) return null;
      const bytes = new Uint8Array(await new Response(r.stream).arrayBuffer());
      return { bytes, etag: r.blob.etag };
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
Expected: `Tests  5 passed (5)`.

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
- Produces (Task 6, and easyNode's client in the app plan, Task 4): `PART_BYTES` (4,194,304), `NODE_HEADER` (`x-ebtx-node`), `NODE_HEADER_VALUE` (`ebtx-snapshot-v1`), `contextFromEnv(env, mainFile) -> ctx` with `ctx = {store, lists, now: () => ms, partBytes}`, and `handleStatement`, `handleFile`, `handlePending`, `handleLatest`, each `(request: Request, ctx) -> Promise<Response>`, answering exactly the routes in Global Constraints.

- [ ] **Step 1: Write the failing test**

Create `site/tests/unit/snapshotRoutes.test.ts`. It runs the whole life of a statement on a folder store with parts of 3,000 bytes (so the 8,055-byte regtest file goes up in three): refused without the header, over 64 KB, off the grid; one operator with its file stored and `latest` still 404; a tampered file refused and nothing kept; a confirmer's signature, and `latest` naming it with bytes that match the vectors; a third signature under a new address; a co-signature from an unlisted key refused with the record unchanged; two confirmers at the same moment both counted; parts of the wrong size refused.

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
  handleStatement, handleFile, handlePending, handleLatest, NODE_HEADER, NODE_HEADER_VALUE,
} from '../../src/lib/snapshotRoutes.mjs';

const fx = (n: string) => new Uint8Array(readFileSync(new URL(`../fixtures/snapshots/${n}`, import.meta.url)));
const R = (n: string) => fx(`regtest-${n}.manifest`);
const DAT = fx('regtest-100.dat');
const P = '0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464';
const C = '02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f';
const D = '034694ab29307fd4e46f3fc7a5115dd52b4143c473a5eb919e857eb5a12bbd6d0e';
const HASH = '11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194';
const BASE = 'http://127.0.0.1:29650';
const sha = (b: Uint8Array) => createHash('sha256').update(b).digest('hex');

let ctx: any;
beforeEach(() => {
  ctx = {
    store: folderStore(mkdtempSync(join(tmpdir(), 'snaproutes-')), `${BASE}/blob`),
    lists: { main: [], regtest: parseRegtestOperators(`producer=${P};confirmer=${C};third=${D}`) },
    now: () => Date.parse('2026-10-01T12:00:00Z'),
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

async function upload(bytes: Uint8Array) {
  const start = await (await file('POST', `statement=${HASH}&action=start`)).json();
  for (let n = 1; n <= start.parts; n++) {
    const part = bytes.subarray((n - 1) * start.part_bytes, n * start.part_bytes);
    const r = await file('PUT', `statement=${HASH}&upload=${start.upload}&part=${n}`, part);
    if (r.status !== 200) return r;
  }
  return file('POST', `statement=${HASH}&upload=${start.upload}&action=complete`);
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
    expect(await r.json()).toEqual({ error: 'height 225927 is not a multiple of 200' });
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

    // A confirmer's one signature: two operators, and now latest names it.
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
    expect(p.block_hash).toBe('bd23c642be34c3a1f1a637d6352b8cfb390c801f2b873605b64986a1bc962c46');
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
    const sk = secp256k1.utils.randomSecretKey();
    const st = parseManifest(R('P')).statement;
    const outsider = serializeManifest({
      statement: st,
      signatures: [{
        key: Buffer.from(secp256k1.getPublicKey(sk, true)).toString('hex'),
        der: secp256k1.sign(statementDigest(st), sk, { prehash: false, format: 'der', lowS: true }),
      }],
    });
    const r = await post(outsider);
    expect(r.status).toBe(422);
    expect((await r.json()).error).toContain('is not on the operator list');
    expect((await (await pending()).json()).statements[0].signers).toEqual([P]);
  });

  it('two confirmers signing at the same moment both count', async () => {
    await post(R('P'));
    const [c, d] = await Promise.all([post(R('C')), post(parseManifestOnly('D'))]);
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
    expect(await r.json()).toMatchObject({ version: 1, chain: 'regtest', statements: [{ statement_hash: HASH, operators: ['producer'], confirmed: false }] });
    expect((await handlePending(new Request(`${BASE}/api/snapshots/pending?chain=testnet`), ctx)).status).toBe(400);
    expect(await (await handleLatest(new Request(`${BASE}/api/snapshots/latest`), ctx)).json()).toEqual({ version: 1, confirmed: null });
  });
});

/** The statement with only D's signature, cut from the three-signature vector. */
function parseManifestOnly(who: 'D') {
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
// The four snapshot routes as plain functions from a Request to a Response,
// so the Astro routes (site/src/pages/api/snapshots/*.ts), the vitest suite
// and the local stand-in (scripts/snapshot-rendezvous-local.mjs) all run the
// very same code. The rules live in snapshotRendezvous.mjs; this file reads
// and writes the store around them.
//
//   POST /api/snapshots/statement            a manifest, at most 64 KB
//   POST /api/snapshots/file?statement=H&action=start
//   PUT  /api/snapshots/file?statement=H&upload=U&part=N   one part, at most 4 MiB
//   POST /api/snapshots/file?statement=H&upload=U&action=complete
//   GET  /api/snapshots/pending?chain=main
//   GET  /api/snapshots/latest?chain=main
//
// Why the file goes up in parts of its own: a Vercel function takes a request
// body of about 4.5 MB, and Blob's multipart API wants parts of at least 5 MB
// (except the last), so a part that fits through a function cannot be a Blob
// multipart part. Each part is kept as a private blob; when the last is in,
// the function reads them back, hashes the whole file, and writes it as one
// public blob only if its size and double SHA-256 are the statement's.
import { randomBytes, createHash } from 'node:crypto';
import { MAX_MANIFEST_BYTES, fileHashes } from './snapshotManifest.mjs';
import {
  acceptManifest, mergeSignatures, newRecord, withManifest, recordManifest, recordOperators,
  isConfirmed, chooseLatest, latestView, pendingView, recordsToPrune, partPlan,
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
const enc = (record) => new TextEncoder().encode(JSON.stringify(record));

/**
 * What every handler needs.
 * @param {Record<string, string | undefined>} env
 * @param {unknown} mainFile the parsed site/src/data/snapshot-operators.json
 */
export async function contextFromEnv(env, mainFile) {
  return { store: await storeFromEnv(env), lists: listsFromEnv(env, mainFile), now: () => Date.now(), partBytes: PART_BYTES };
}

async function readRecord(store, hash) {
  const got = await store.getPrivate(recordPath(hash));
  if (!got) return null;
  return { record: JSON.parse(new TextDecoder().decode(got.bytes)), etag: got.etag };
}

async function readAll(store) {
  const out = [];
  for (const p of await store.list('snapshots/records/')) {
    try {
      const got = await store.getPrivate(p);
      if (got) out.push(JSON.parse(new TextDecoder().decode(got.bytes)));
    } catch (e) {
      console.error('snapshot record unreadable', p, e);
    }
  }
  return out;
}

/** Read, change, write back only if nobody wrote in between; three more tries if somebody did. */
async function upsert(store, hash, change) {
  for (let attempt = 0; attempt < 4; attempt++) {
    const cur = await readRecord(store, hash);
    const next = await change(cur ? cur.record : null);
    try {
      await store.putPrivate(recordPath(hash), enc(next), { etag: cur ? cur.etag : null });
      return next;
    } catch (e) {
      if (!(e instanceof StoreConflict)) throw e;
    }
  }
  throw new HttpError(503, 'too many writers at once; try again');
}

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

/** Best effort: storage stays five confirmed pairs per chain plus the last seven days. */
async function prune(ctx) {
  try {
    for (const r of recordsToPrune(await readAll(ctx.store), ctx.now(), ctx.lists)) {
      await ctx.store.del([
        ...(await ctx.store.list(`snapshots/${r.height}/${r.statement_hash}`)),
        ...(await ctx.store.list(uploadPrefix(r.statement_hash))),
        recordPath(r.statement_hash),
      ]);
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

/** POST /api/snapshots/statement: a new statement, or co-signatures for one already here. */
export async function handleStatement(request, ctx) {
  return guard(request, ctx, 'POST') ?? run(async () => {
    const bytes = await bodyUpTo(request, MAX_MANIFEST_BYTES);
    if (bytes.length === 0) throw new HttpError(400, 'no manifest');
    const accepted = acceptManifest(bytes, ctx.lists);
    if (!accepted.ok) throw new HttpError(422, accepted.reason);
    const nowIso = iso(ctx.now());
    let added = accepted.manifest.signatures.map((s) => s.key);
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
    await prune(ctx);
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

/** The file, in parts: start, one PUT per part, complete. Only for a statement already stored. */
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
    const current = (r) => r.file?.state === 'uploading' && r.file.upload === upload && HEX32.test(upload);

    if (method === 'POST' && q.get('action') === 'start') {
      if (cur.record.file.state === 'stored') throw new HttpError(409, 'the file is already stored');
      if (recordOperators(cur.record, ctx.lists).length === 0) throw new HttpError(422, 'no signature on this statement is from a listed key');
      const id = randomBytes(16).toString('hex');
      await upsert(ctx.store, hash, async (r) => {
        if (r.file.state === 'stored') throw new HttpError(409, 'the file is already stored');
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
      const parts = (await ctx.store.list(uploadPrefix(hash)));
      if (h.size !== cur.record.file_size || h.fileHash !== cur.record.file_hash) {
        await ctx.store.del(parts);
        await upsert(ctx.store, hash, async (r) => (current(r) ? { ...r, file: { state: 'missing' } } : r));
        throw new HttpError(422, 'the file does not match the statement');
      }
      const { url } = await ctx.store.putPublic(filePath(cur.record), Buffer.concat(chunks));
      const nowIso = iso(ctx.now());
      const record = await upsert(ctx.store, hash, async (r) =>
        publish(ctx.store, { ...r, file: { state: 'stored', url, sha256: h.sha256, stored_at: nowIso } }, ctx.lists, nowIso));
      await ctx.store.del(parts);
      await prune(ctx);
      return json(200, { stored: true, file_url: url, file_sha256: h.sha256, confirmed: isConfirmed(record, ctx.lists) });
    }
    throw new HttpError(400, 'action must be start or complete');
  });
}

function chainParam(request) {
  const chain = new URL(request.url).searchParams.get('chain') || 'main';
  if (chain !== 'main' && chain !== 'regtest') throw new HttpError(400, 'chain must be main or regtest');
  return chain;
}

/** GET /api/snapshots/pending: statements of the last seven days and who signed each. */
export async function handlePending(request, ctx) {
  return guard(request, ctx, 'GET') ?? run(async () => {
    const view = pendingView(await readAll(ctx.store), chainParam(request), ctx.now(), ctx.lists);
    return json(200, view, { 'cache-control': 'public, max-age=30, s-maxage=30' });
  });
}

const NONE = { version: 1, confirmed: null };
const LATEST_CACHE = { 'cache-control': 'public, max-age=60, s-maxage=60' };

/** GET /api/snapshots/latest: the newest statement two operators signed, with its checked file. */
export async function handleLatest(request, ctx) {
  if (request.method !== 'GET') return fail(405, 'use GET');
  if (!ctx.store) return json(404, NONE, LATEST_CACHE);
  return run(async () => {
    const chain = chainParam(request);
    const best = chooseLatest(await readAll(ctx.store), chain, ctx.lists);
    return best ? json(200, latestView(best, ctx.lists), LATEST_CACHE) : json(404, NONE, LATEST_CACHE);
  });
}
````

- [ ] **Step 4: Run the test, then the whole suite**

Run: `cd site && npx vitest run tests/unit/snapshotRoutes.test.ts`
Expected: `Tests  7 passed (7)`.

Run: `cd site && npx vitest run`
Expected: every file passes; the five new files add 53 tests (measured on `e187d14c`: `Tests  303 passed (303)`).

- [ ] **Step 5: Commit**

````bash
git add site/src/lib/snapshotRoutes.mjs site/tests/unit/snapshotRoutes.test.ts
git commit -m "site: the snapshot routes as plain handlers: statement, file in parts, pending, latest" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

### Task 6: The Astro routes and the local stand-in

**Files:**
- Create: `site/src/pages/api/snapshots/statement.ts`, `file.ts`, `pending.ts`, `latest.ts`
- Create: `site/scripts/snapshot-rendezvous-local.mjs`

**Interfaces:**
- Consumes: Task 5 (`contextFromEnv`, the four handlers, `PART_BYTES`), Task 4 (`folderStore`), Task 2 (`listsFromEnv`).
- Produces (easyNode's rehearsal, app plan Task 8): `node scripts/snapshot-rendezvous-local.mjs --port <p> --dir <folder>` run from `site/`, with `SNAPSHOT_REGTEST_OPERATORS` set, serving the four routes at `http://127.0.0.1:<p>/api/snapshots/...` and the public blobs at `http://127.0.0.1:<p>/blob/<path>`; it prints `snapshot rendezvous stand-in on http://127.0.0.1:<p>, …` once listening and refuses bodies over 4,500,000 bytes with 413, as a Vercel function does.

- [ ] **Step 1: Write the four routes**

Create `site/src/pages/api/snapshots/statement.ts`:

````ts
// POST a signed snapshot statement, or co-signatures for one already here.
// The rules and the reasons are in src/lib/snapshotRendezvous.mjs; the
// handler is in src/lib/snapshotRoutes.mjs, shared with the tests and the
// local stand-in easyNode's rehearsal runs against.
import type { APIRoute } from 'astro';
import mainFile from '../../../data/snapshot-operators.json';
import { contextFromEnv, handleStatement } from '../../../lib/snapshotRoutes.mjs';

export const prerender = false;

export const POST: APIRoute = async ({ request }) => handleStatement(request, await contextFromEnv(process.env, mainFile));
````

Create `site/src/pages/api/snapshots/file.ts`:

````ts
// The file of a stored statement, in parts of at most 4 MiB: POST start, PUT
// each part, POST complete. Kept only if its size and double SHA-256 are the
// statement's. See src/lib/snapshotRoutes.mjs for why it goes in parts.
import type { APIRoute } from 'astro';
import mainFile from '../../../data/snapshot-operators.json';
import { contextFromEnv, handleFile } from '../../../lib/snapshotRoutes.mjs';

export const prerender = false;

export const POST: APIRoute = async ({ request }) => handleFile(request, await contextFromEnv(process.env, mainFile));
export const PUT: APIRoute = async ({ request }) => handleFile(request, await contextFromEnv(process.env, mainFile));
````

Create `site/src/pages/api/snapshots/pending.ts`:

````ts
// What confirmers read: statements of the last seven days and who signed each.
import type { APIRoute } from 'astro';
import mainFile from '../../../data/snapshot-operators.json';
import { contextFromEnv, handlePending } from '../../../lib/snapshotRoutes.mjs';

export const prerender = false;

export const GET: APIRoute = async ({ request }) => handlePending(request, await contextFromEnv(process.env, mainFile));
````

Create `site/src/pages/api/snapshots/latest.ts`:

````ts
// What every easyNode reads first: the newest snapshot two different
// operators signed, with its checked file. The app trusts none of it and
// checks everything again; a 404 here only means every node uses its
// fallbacks.
import type { APIRoute } from 'astro';
import mainFile from '../../../data/snapshot-operators.json';
import { contextFromEnv, handleLatest } from '../../../lib/snapshotRoutes.mjs';

export const prerender = false;

export const GET: APIRoute = async ({ request }) => handleLatest(request, await contextFromEnv(process.env, mainFile));
````

- [ ] **Step 2: Build, and run the real routes under `astro dev`**

````bash
cd /Users/m2promende/repos/EasyBTX-snapshot-rendezvous/site
npx astro build 2>&1 | tail -3
T=/private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad/rv-astro
rm -rf $T
SNAPSHOT_STORE_DIR=$T SNAPSHOT_PUBLIC_BASE=http://127.0.0.1:4329/blob \
SNAPSHOT_REGTEST_OPERATORS="producer=0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464;confirmer=02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f" \
  npx astro dev --port 4329 --host 127.0.0.1 > $T.log 2>&1 &
for i in $(seq 1 60); do curl -s -o /dev/null 'http://127.0.0.1:4329/api/snapshots/latest?chain=regtest' && break; sleep 1; done
curl -s -w ' %{http_code}\n' 'http://127.0.0.1:4329/api/snapshots/latest?chain=regtest'
curl -s -w ' %{http_code}\n' -H 'x-ebtx-node: ebtx-snapshot-v1' -H 'content-type: application/octet-stream' --data-binary @tests/fixtures/snapshots/regtest-PC.manifest http://127.0.0.1:4329/api/snapshots/statement
curl -s -w ' %{http_code}\n' -X POST -H 'x-ebtx-node: ebtx-snapshot-v1' 'http://127.0.0.1:4329/api/snapshots/file?statement=11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194&action=start'
pkill -f 'astro dev --port 4329'
````

Expected: the build ends `[build] Complete!`; then `{"version":1,"confirmed":null} 404`; then `{"statement_hash":"11c5406e…f194","chain":"regtest","height":100,"signers":["0343faeb…e464","02c05d68…6c0f"],"operators":["producer","confirmer"],"added":2,"file":"missing"} 200`; then `{"upload":"<32 hex>","part_bytes":4194304,"parts":1} 200` (measured on `e187d14c` with these files, 2026-09-29). Every POST and PUT body goes as `application/octet-stream`: Astro's origin check refuses form content types from another origin, and the app sends octet-stream.

- [ ] **Step 3: Write the local stand-in**

Create `site/scripts/snapshot-rendezvous-local.mjs`:

````js
#!/usr/bin/env node
// A local stand-in for the four snapshot routes on easybtx.com, for easyNode's
// end-to-end rehearsal on regtest (crates/btx-core/tests/snapshot_network_regtest.rs
// in the easynode repository). It runs the SAME handlers the Astro routes run
// (src/lib/snapshotRoutes.mjs) over node:http, with a folder store instead of
// Vercel Blob, and serves the folder's public blobs under /blob/. Like a
// Vercel function it refuses request bodies over 4.5 MB.
//
//   SNAPSHOT_REGTEST_OPERATORS="producer=<66 hex>;confirmer=<66 hex>" \
//     node scripts/snapshot-rendezvous-local.mjs --port 29650 --dir /tmp/rv
//
// Loopback only. Not a server for anything else.
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { folderStore } from '../src/lib/snapshotStore.mjs';
import { listsFromEnv } from '../src/lib/snapshotOperators.mjs';
import { handleStatement, handleFile, handlePending, handleLatest, PART_BYTES } from '../src/lib/snapshotRoutes.mjs';

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
const mainFile = JSON.parse(readFileSync(new URL('../src/data/snapshot-operators.json', import.meta.url), 'utf8'));
const ctx = { store, lists: listsFromEnv(process.env, mainFile), now: () => Date.now(), partBytes: PART_BYTES };
const routes = {
  '/api/snapshots/statement': handleStatement,
  '/api/snapshots/file': handleFile,
  '/api/snapshots/pending': handlePending,
  '/api/snapshots/latest': handleLatest,
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
  console.log(`snapshot rendezvous stand-in on ${base}, store ${dir}, regtest operators: ${ctx.lists.regtest ? ctx.lists.regtest.map((o) => o.name).join(', ') : 'none'}`);
});
````

- [ ] **Step 4: Walk the stand-in through a statement by hand**

````bash
cd /Users/m2promende/repos/EasyBTX-snapshot-rendezvous/site
T=/private/tmp/claude-501/-Users-m2promende-repos-easynode--claude-worktrees-easynode-0-7-0-release-65b687/ccaaa761-fc6e-4fe3-ad52-eb25130739ea/scratchpad/rv-local
rm -rf $T
SNAPSHOT_REGTEST_OPERATORS="producer=0343faebbc3a28f2e452132477192cb5455f0c0f2cfdab01c9217c43c2cbc3e464;confirmer=02c05d68daeabe9e5f0556fcdca6c5a4011eca1d46ee34826d444d1d95b15e6c0f" \
  node scripts/snapshot-rendezvous-local.mjs --port 29650 --dir $T > $T.log 2>&1 &
sleep 1; cat $T.log
F=tests/fixtures/snapshots; H=11c5406e51423d5817e3fd62b2a8c5e18b4f7ba079fdce1087cf732453bbf194; N='x-ebtx-node: ebtx-snapshot-v1'; O='content-type: application/octet-stream'
curl -s -H "$N" -H "$O" --data-binary @$F/regtest-P.manifest http://127.0.0.1:29650/api/snapshots/statement; echo
U=$(curl -s -X POST -H "$N" "http://127.0.0.1:29650/api/snapshots/file?statement=$H&action=start" | python3 -c 'import json,sys;print(json.load(sys.stdin)["upload"])')
curl -s -X PUT -H "$N" -H "$O" --data-binary @$F/regtest-100.dat "http://127.0.0.1:29650/api/snapshots/file?statement=$H&upload=$U&part=1"; echo
curl -s -X POST -H "$N" "http://127.0.0.1:29650/api/snapshots/file?statement=$H&upload=$U&action=complete"; echo
curl -s -o /dev/null -w '%{http_code}\n' 'http://127.0.0.1:29650/api/snapshots/latest?chain=regtest'
curl -s -H "$N" -H "$O" --data-binary @$F/regtest-C.manifest http://127.0.0.1:29650/api/snapshots/statement; echo
curl -s 'http://127.0.0.1:29650/api/snapshots/latest?chain=regtest'; echo
FU=$(curl -s 'http://127.0.0.1:29650/api/snapshots/latest?chain=regtest' | python3 -c 'import json,sys;print(json.load(sys.stdin)["file_url"])'); curl -s "$FU" | shasum -a 256
pkill -f 'snapshot-rendezvous-local.mjs --port 29650'
````

Expected, in order: `snapshot rendezvous stand-in on http://127.0.0.1:29650, store …, regtest operators: producer, confirmer`; the statement with `"operators":["producer"]` and `"file":"missing"`; `{"part":1,"bytes":8055}`; `{"stored":true,"file_url":"http://127.0.0.1:29650/blob/snapshots/100/11c5406e…f194.dat","file_sha256":"b2c5c43c…ce85","confirmed":false}`; `404` (one operator); the statement with `"operators":["producer","confirmer"]` and `"file":"stored"`; the pointer with `"manifest_sha256":"e8fa06d2f700feb488d7f494a532cd0e44799d20613b80532b2eca5cffaa746e"` (the regtest-PC vector) and `"file_sha256":"b2c5c43c4fd931475c4769926564644b6cff744a229c54623f5eb03c16e4ce85"`; `b2c5c43c…ce85  -` (measured 2026-09-29).

- [ ] **Step 5: All checks, then commit**

````bash
cd /Users/m2promende/repos/EasyBTX-snapshot-rendezvous
(cd site && npx vitest run 2>&1 | tail -4)
python3 scripts/check-node-links.py
git add site/src/pages/api/snapshots site/scripts/snapshot-rendezvous-local.mjs
git commit -m "site: /api/snapshots routes, and a local stand-in for easyNode's rehearsal" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
````

Expected: vitest all passed; `check-node-links.py` exits 0 (these files touch no link).

### Task 7: Preview check, pull request, production

**Files:** none new.

**Interfaces:**
- Consumes: Tasks 1 to 6; easynode `main` carrying `crates/btx-core/snapshot-operators.json` (app plan, Task 1).
- Produces: `https://easybtx.com/api/snapshots/latest` answering `404 {"version":1,"confirmed":null}` until a second operator is on the list, which is what every 0.7.0 app expects today (it then uses its fallbacks).

- [ ] **Step 1: The live operator check passes**

Run: `node scripts/check-snapshot-operators.mjs; echo "exit $?"`
Expected: `snapshot operators: the website's copy is easyNode's (main): Mende (02d5efca…4675)` and `exit 0`. If it says HTTP 404, the app plan's Task 1 is not on easynode `main` yet: stop here until it is.

- [ ] **Step 2: Push and open the PR (the preview deploys on its own)**

````bash
git push -u origin claude/snapshot-rendezvous
gh pr create --repo MendeMatthias/EasyBTX --base main --head claude/snapshot-rendezvous \
  --title "site: /api/snapshots, the meeting point for confirmed chain snapshots" \
  --body "$(printf '%s\n' \
  'The website half of docs/decisions/2026-09-29-every-node-starts-near-the-tip.md, section 6 (easynode).' \
  '' \
  '- POST /api/snapshots/statement takes a manifest only when every signature is valid, strict DER, low S and from a key on the operator list; right chain, replay context, grid height, file size. Co-signatures merge, one per key.' \
  '- The file goes up in parts of 4 MiB through /api/snapshots/file and is kept only when its size and double SHA-256 are the statement'"'"'s.' \
  '- GET /api/snapshots/pending lists the last seven days for confirmers; GET /api/snapshots/latest names the newest statement two different operators signed, in the pointer contract easyNode reads.' \
  '- The operator list is a copy of easyNode'"'"'s, checked against it in CI and daily. Today it is Mende alone, so latest answers 404 and every node uses its fallbacks.' \
  '- Nothing here is trusted by the app: it checks every signature, its own node and the file hash again.' \
  '' \
  'Tests: 53 new vitest cases on the same vectors the app tests read.' \
  '' \
  '🤖 Generated with [Claude Code](https://claude.com/claude-code)')"
````

- [ ] **Step 3: Check the preview deployment**

Find the preview URL on the PR (the Vercel bot comment), then:

````bash
P=<the preview URL from the PR, https://…vercel.app>
curl -s -w ' %{http_code}\n' "$P/api/snapshots/latest"
curl -s -w ' %{http_code}\n' "$P/api/snapshots/pending"
curl -s -w ' %{http_code}\n' -X POST "$P/api/snapshots/statement" --data-binary @site/tests/fixtures/snapshots/regtest-PC.manifest
curl -s -w ' %{http_code}\n' -H 'x-ebtx-node: ebtx-snapshot-v1' -H 'content-type: application/octet-stream' --data-binary @site/tests/fixtures/snapshots/mainnet-225927.manifest "$P/api/snapshots/statement"
````

Expected: `{"version":1,"confirmed":null} 404`; `{"version":1,"chain":"main","statements":[]} 200` (or `503` if the preview has no Blob token, which is fine for a preview); ` 403` (no header); `{"error":"height 225927 is not a multiple of 200"} 422` (or `503` without a token). If the preview is behind Vercel's deployment protection, open the URL in the browser once or use the bypass token the project already uses; do not change project settings.

- [ ] **Step 4: Merge, then check production**

After review, merge the PR on GitHub (squash, as the repository does). Then:

````bash
curl -s -w ' %{http_code}\n' https://easybtx.com/api/snapshots/latest
curl -s -w ' %{http_code}\n' https://easybtx.com/api/snapshots/pending
````

Expected: `{"version":1,"confirmed":null} 404` and `{"version":1,"chain":"main","statements":[]} 200`. The first real statement arrives when a 0.7.0 producer on the list reaches a multiple of 200; `latest` stays 404 until a second operator is added to both lists.

## Rollback

- `latest` answering 404 makes every node use its fallbacks (design section 9); to force it, delete `site/src/pages/api/snapshots/latest.ts` in a revert PR, or delete the `snapshots/` blobs in the Vercel Blob browser. Nothing on a node depends on the other three routes to start.
- Adding an operator: the same line in easyNode's `operators.rs` and `snapshot-operators.json`, shipped in an app release; then copy `crates/btx-core/snapshot-operators.json` over `site/src/data/snapshot-operators.json` in a PR here, which the operator check lets through only when the two are equal.

## Risks and open points (for the owner)

1. **The design's upload path does not fit Vercel.** Section 6 says "Vercel Blob multipart". A function takes about 4.5 MB per request and Blob multipart parts must be at least 5 MB except the last (`@vercel/blob` 2.4.0, `uploadPart`), so parts cannot pass through a function as multipart parts. This plan keeps each part as a private blob and joins them in the `complete` step (read back, hash, one `put`). Same effect, one more read of the file per upload (at most 64 MiB, within the 60-second function limit; about 9 MB today).
2. **Presigned direct uploads exist** (`issueSignedToken` and `presignUrl` in 2.4.0), which would skip the function for the bytes, but the wire format for a non-SDK client (the app is Rust) is not documented; not used.
3. **The file is read twice per upload** (parts in, whole file out) and `complete` holds the whole file in memory: at most 64 MiB against the function's memory, today 9 MB.
4. **Two operators disagreeing** at one height make that height `disputed` and `latest` falls back to the height below; nobody is told except through `pending`. An alert (the node-health workflow pattern) would be a small follow-up.
5. **Junk filter, not authentication.** The header keeps scanners out; anyone can still read `pending` and `latest`, which only hold public data (statements, public keys, signatures). Writes need signatures from listed keys, so only operators can create records, and a file can only be as large as a stored statement says.

## Self-review

- Spec coverage (section 6 and the website half of section 1): statement intake with every rule (Task 3 `acceptManifest`, Task 5 `handleStatement`); merging, one per key (Task 3 `mergeSignatures`, Task 5 concurrency test); the file in parts, kept only if size and hash match, only for a stored statement with a listed signer (Task 5 `handleFile`); `pending` for seven days with who signed (Tasks 3, 5); `latest` in the app's contract, two different operators, file stored and checked (Tasks 3, 5, 6); storage kept to five confirmed plus seven days (Task 3 `recordsToPrune`, Task 5 `prune`); the list copy and its check against easyNode (Task 2); `@noble/curves` as a direct dependency (Task 1); the same vectors as the app (Task 1).
- Sabotage tests, one per rule: a statement from an unlisted key (`refuses a statement signed by a key on no list…`), a merged co-signature from an unlisted key (`refuses a co-signature from a key on no list and keeps what it had`), a file whose hash does not match (`goes from one operator to confirmed…`, the tampered upload), `latest` never pointing at a one-operator statement (`one operator, even with its file stored: nothing`, and the routes test before the confirmer signs), off-grid (the real 225,927 manifest, and height 150 on regtest), wrong chain, wrong replay context, empty shielded commitment, chunk geometry, file too large, duplicate key, invalid signature, high S, non-strict DER, trailing bytes, oversize body, a key taken off the list, a disputed height, a part of the wrong size.
- Placeholders: none. Every code step is the file as it passed on 2026-09-29 in a copy of `e187d14c` (303 vitest cases passed, `astro build` completed, `astro dev` and the stand-in answered as shown). The only value the engineer fills in is the preview URL, which exists only after the push.
- Consistency: the handler names, `PART_BYTES`, `NODE_HEADER_VALUE`, the reply keys and the record shape are the ones the app plan's client (Task 4) reads.
