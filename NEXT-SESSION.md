# Next easyNode session: the handover

State on 2026-10-11, after 0.7.10. Everything below is live and was verified
against the released bytes, not just the local build.

---

## Where things stand

**Live on easybtx.com: easyNode 0.7.10 on Mac, Linux and Windows**
(`node-v0.7.10` on MendeMatthias/EasyBTX-releases). Engine BTX v0.34.15
(`476f3f23`). The repo-global Latest pointer stays on the miner (v0.30.0).

What the last three releases did, all on 2026-10-10:

- **0.7.8**: engine v0.34.12 to v0.34.15, because mainnet block 244,000
  switched on three consensus checks 0.34.12 lacks (32-byte HTLC preimage,
  recovery proofs in the shielded block budget, a Phase-1-only MatMul header
  can no longer be the best header). A failed engine upgrade no longer falls
  back to an engine without those rules: `btx_core::node::engine_follows_network_rules`
  and `after_failed_engine_upgrade` in commands.rs. The pin reasoning sits
  beside `NODE_RELEASE_TAG`.
- **0.7.9**: Jarek's full-history archive `134.199.150.193:19335`
  (archive.btxbyronbay.com) in `BTX_BOOTSTRAP_PEERS`. Measured that night:
  both earlier block sources (`109.199.124.187` and `20.86.181.203:19338`)
  answer NOTFOUND for block 150,000, so before 0.7.9 no shipped peer could serve
  the old blocks a catching-up node asks for.
- **0.7.10**: btxscan's full node `20.86.181.203:19335` in `BTX_ARCHIVE_PEERS`,
  after `:19338`. Whole chain plus `02d5efca`'s signatures. Same box as 19338,
  so a second node, not a second site.

The network when this was written: one chain, no split, 0.34.15 the most common
engine, `02d5efca` (the 3060) signing 100 of the last 100 blocks.

## Open work, in priority order

1. **New nodes start far behind.** A fresh install logs
   `[attested] no confirmed snapshot (HTTP 404); trying the pinned pair` and
   starts at 239,111, a gap that grows about 1,400 blocks a day. The 0.7.7
   snapshot network should be serving a confirmed snapshot near the tip and is
   not. Find out why before changing anything (app, btx-confirmer, the
   easybtx.com rendezvous, or operators not running producers), and consider
   refreshing the pinned pair as a safety net, the way 0.7.6 (#173) did.
2. **"Blocks behind" under-counts after block 244,000.** v0.34.15 does not let a
   Phase-1-only header become the best header (`AddToBlockIndex`,
   `phase1_must_not_lead`), so `getblockchaininfo.headers` sits near `blocks`
   while a node catches up. Measured: the app said 4,888 behind when the gap was
   about 5,260. Readers: watchdog.rs gap rule, catchup_assist.rs
   (`t.headers < t.blocks + MIN_BEHIND`), engine_warnings.rs, commands.rs,
   wallet.rs. Harmless (nothing restarts), but the catch-up help and stall
   diagnosis go quiet. Do not "fix" it by reading `getchaintips` unfiltered:
   that brings back the false headers-only tower of the 227,313 incident.

## Release facts that cost time to rediscover

- The recipe is docs/node-release-recipe.md. Version lives in five files:
  package.json, package-lock.json (x2), tauri.conf.json, Cargo.toml,
  Cargo.lock. Feature work goes under `## [Unreleased]` in the changelog and the
  release renames it.
- Linux and Windows are built by CI (`node-linux-installer.yml`,
  `node-win-installer.yml`, dispatched on the branch; engine pins already prove
  v0.34.15). The Mac is built on the Mac:
  `TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/easybtx.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" npx tauri build --bundles app,updater`,
  then the `.dmg` by hand with `hdiutil`. The staged engine in
  `src-tauri/resources/node-pkg` is the CI macOS btxd, re-signed ad hoc by the
  staging script (only the code signature differs from the artifact).
- **No node runs on the release Mac** (owner's decision, disk). So
  `build-node-feed.sh` and `publish-node-release.sh` need `OBSERVER_OVERRIDE=1`;
  before using it, prove "no split, live chain" from easybtx.com/api/nodes,
  both pools, witness-1.easybtx.com/signers/recent and api.btxscan.io.
- Never re-cut a published version: the app only updates to a higher one, and
  the scripts refuse an existing tag. Versions compare as semver everywhere, so
  0.7.10 sorts after 0.7.9.
- The site (MendeMatthias/EasyBTX) moves after the release is live and every
  asset answers 200: pins in `site/src/pages/node.astro` and
  `node/how-to.astro`, the two feed files, the changelog copy, and
  `site/src/lib/node-peers.json`, which must equal node.rs's lists at an
  easynode main commit (`node scripts/check-node-peers.mjs`). Work from a fresh
  worktree off origin/main; the local ~/repos/EasyBTX checkout is months stale.
- easynode `main` needs a review; merge with `gh pr merge --squash --admin`.

## Testing habits that paid off

- Measure a peer before shipping it: `btx-sentinel --probe host:port` for the
  handshake, a `getdata` for an old block (150,000) with a parent-hash check,
  and the btx-ops `pull_attest.py` for signatures. A pruned peer answers
  NOTFOUND for old blocks; a mirror without the attestation-archive bit gives no
  signatures.
- The real-engine suites run on a Mac against the CI macOS btxd:
  `EASYNODE_TEST_BTXD=... cargo test --locked --tests -- --ignored --test-threads=1`.
  The Metal canary runs before RPC (about 100 to 115 s on an M-series Mac), and
  `snapshot_network_regtest` has a macOS branch for the CUDA-only log line.
- Verify the update path like a user's app does: read the live feed from
  easybtx.com, download the asset from GitHub, `minisign -V` with the pubkey
  from tauri.conf.json, and check the signed `file:` name.
