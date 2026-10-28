# BTX update protocol (all of Mende's BTX apps)

v2, 2026-10-29. Aimed at **BTX 0.35.0** and reusable for every later release.
v2 folds in an independent review (29 claims spot-checked against `origin/main`) and
adds the signer box, the rc-phase pre-staging, a shadow canary and a stronger rollback.
Research only: nothing was bumped, built, committed or deployed while writing this.

- **Part A**: what's new in BTX and in 0.35, and the network state.
- **Part B**: the protocol. Phase 1 runs during the rc and does most of the work. Phase 2 runs
  on release day and mostly swaps a hash. Then per-app steps, checks and rollback.
- **Part C**: the lessons it is built from.

The core idea: **release day should only swap a hash.** Every risky step (reading the diff,
building engines, patching scripts, soaking a canary, regenerating fixtures) happens on the
rc, while the network is calm.

---

## Part A. What's new in BTX, and what 0.35 brings

### Where things stand (2026-10-29)

| | |
|---|---|
| Shipping release | **v0.34.15** (2026-10-06, GitHub "Latest"). `origin/main` == `v0.34.15` == `476f3f23`. |
| Release candidate | **v0.35.0-rc1** (2026-10-13, pre-release, `849fb2d8`, *not on main*, on `btx-035/lab`). |
| 0.35 work | `btx-035/lab` (+42 past rc1), `btx-035/swarm` (+815), `btx-035/atlas-native` (+836), `btx-035/model-security-v2` (+811). Merge direction: atlas-native + model-security → swarm (#273, #275) → lab (#245) → 0.35 final. The last step is inferred: no v0.35.0 tag exists yet. |
| 0.34.16 | **Will not exist** (numair, 10-25, Telegram #1924). All fixes go to 0.35. |
| 0.35 ETA | Jpp, 10-25: "0.35 should be ready and stable in 2 weeks max", so about **Nov 8**. The website relaunch is waiting on it. |
| Stale doc | The btx repo's CLAUDE.md says "0.34.9-dev, last tag v0.34.8". Wrong; CMake on main says 0.34.15. |

### Must-know before touching anything

1. **Never ship the `v0.35.0-rc1` tag binaries on mainnet.** In rc1, commit `68f2bf3e`
   narrowed `P2MRClaimLeafPinsPreimageLength` (`src/script/pqm.cpp:560`) to one HTLC leaf
   type. v0.34.15 applies the 32-byte preimage rule (consensus from height 244,000) to all three.
   An rc1 node would accept a claim that 0.34.15 rejects, which can split the chain. The rule was
   restored in `6b94704b` (10-14), and every `btx-035/*` branch has it. Checked in the code. Only
   ship 0.35 final, or a build whose history contains `6b94704b`:
   `git merge-base --is-ancestor 6b94704b <sha>`.
2. **btxd updates itself by default on mainnet**, and has since **v0.31.0**: `-autoupdate`
   defaults to 1 and polls `https://btx.dev/version.txt` every 30 min (`src/node/autoupdate.h`,
   `src/init.cpp:772`).
   - easyNode already passes `-autoupdate=0` (`crates/btx-core/src/node.rs:1002`), **but only when the
     install key parses as `vX.Y.Z…`**. A key that doesn't parse silently drops the flag. The same
     parse also gates `engine_follows_network_rules` and `-matmulrcexecution`.
     **Rule: the install key always starts with `v<major>.<minor>.<patch>`.**
   - **Live today: both btxscan nodes run with btxd's self-update ON.** Neither `/data/btx/bitcoin.conf` nor
     `/data/btx2/btx.conf` sets `autoupdate` (checked read-only on 2026-10-29 by the btxscan session). The
     patched switch in btx-apps PR #289 writes `autoupdate=0`, but only at the next engine switch. Until
     then, adding `autoupdate=0` to both confs and restarting is Mende's call.
   - In 0.35 the automatic off with `-proxy`/`-onlynet` exists only on swarm/atlas/model-security, not on lab or rc1.
3. **A sync from genesis on 0.34.15 stops at 126,978.** Block 126,979 fails
   `shielded-unshield-velocity-exceeded`. The bug is in every 0.34.x and in rc1. The fix PR #272 is
   still OPEN and is a consensus-replay change. Nodes that start from a snapshot or are already past
   that height are fine. On a fixed build, run `reconsiderblock 358a3d27…`; no reindex.
   #272 may also bump `BLOCK_VALIDATION_EPOCH`, which changes what the first start of 0.35 does.
   Put that in the verdict table.
4. **The checksum and archive format changes in 0.35.**
   - SHA256SUMS is missing for v0.34.13, .14 and .15. rc1 uses **SHA384SUMS**.
   - The archives are renamed to `btx-035-{linux-x86_64,macos-arm64,windows-x86_64}.tar.xz`. These
     names carry no patch number, so take the version from the tag, never from the file name.
   - Take digests from `gh api repos/btxchain/btx/releases/tags/<tag> --jq '.assets[]|.name+" "+.digest'`.
     Strip the `sha256:` prefix where a script wants bare hex.

### 0.34.8 → 0.34.15 (already shipped; every app must have absorbed it)

- **Flag day at 244,000** (`BTX_SECURITY_ACTIVATION_HEIGHT`, 0.34.14+): the HTLC 32-byte
  preimage rule, the shielded recovery-proof cost and the MatMul phase-1 header rule. Anything below 0.34.14 is off-chain.
  App floors: `MIN_ENGINE=0.34.14`.
- **assumeutxo 228000** (0.34.12), release asset `assumeutxo-228000`.
- **The `-reorgpolicy` default flipped twice**: bounded in 0.34.12, then legacy again when unset in 0.34.14. Always pass it explicitly.
- **45 new RPCs, none removed**:
  - about 38 Pay-With-Compute `*compute*` RPCs
  - `preparereorg` / `executereorg` / `getreorgrecoverystatus`
  - `loadmodel` / `unloadmodel` / `generatemodel` / `getmodelhostprofile` / `preparebountyaward`
- **HTLC**: `deriveaddresses` no longer makes legacy `htlc()`/`htlc_tx()`. `buildhtlcclaim` takes a
  confirmation depth. A lock whose refund time has already passed is refused.
- **P2P**: `PROTOCOL_VERSION` 800002 and min 800001 are unchanged, including in 0.35.
- **Datadir**: `BLOCK_VALIDATION_EPOCH` is 1 (no reindex). A one-time "epoch" log line on the first start is expected.
- **Archives (0.34.x)**: `btx-<ver>-{x86_64-linux-gnu,x86_64-linux-gnu-cuda13,arm64-apple-darwin}.tar.gz`.
  - `bin/btxd` is a wrapper around `libexec/btxd.real`.
  - Linux needs glibc 2.38, plus OpenSSL ≥ 3.5 for the model plane.
  - CUDA is `sm_120` only, and cuda12 was dropped after 0.34.10.
  - `-modelnet` defaults to 1.

### What 0.35 brings

- **Money and models in one node.** The "swarm" runs inside btxd whenever ModelNet is on
  (`-modelnet=0` turns it off). Models are shared like torrents (signed lineage, `btx://` links) and
  run locally. `btx-modeld` is **still a separate binary** (`src/modelnet/CMakeLists.txt:2597` on swarm).
- **BTX Atlas**: a Tauri desktop wallet and operator client next to Qt (#273, not the default).
- **Model security**: `btx-guardd` and the `modelsecurity` RPC (#275, #282). Windows and macOS confinement are not done yet.
- **Sync and mirror fixes**, all found on Lars's archive run:
  - #274 (IBD downloaded each block 3 times; now about 2.7× faster) and #261 (FindMostWorkChain 380 → 23 ms)
    are on **swarm** (via #245), not yet on lab.
  - Still open:
    - #272: the 126,978 stop
    - #280, #285: GPU-less mirror freezes after signed one-block twins. Today's fix is a
      reversible `invalidateblock <losing twin>`.
    - #281: `getblockstats` breaks on about 4,264 shielded and penalized blocks
    - #283, #284: CI silently skipped 164 test suites
- **Wallet**: macOS ML-DSA keys did not derive from the seed, so wallets made by macOS btxd 0.34.x
  hold cached keys the seed can't reproduce. 0.35 loads them (#253; we tested on an M5).
  Dandelion++ stem transactions stay private.
- **New RPCs**:
  - rc1: `getctvtemplatehash`, `lab035`, `verifycomputereceipt`
  - lab: `getbalancewatch`, `getbalanceatheight`, `removemodel`
  - swarm: `getmodelfundingsettlement`
- **New flags**: `-lightwatchdir`, `-offerattestedutxosnapshot[manifest]`, `-modellandiscovery`.
- **No consensus height changes so far.** `src/primitives` is untouched, so esplora decoding should be safe. Re-check on the final tag.
- **Packaging** (rc1, see `doc/release/0.35.0-rc1-checklist.md` on lab; there are no 0.35 release notes yet):
  - **CUDA is off in the official Linux and Windows archives.** This does not block easyNode, which builds its
    own CUDA engines from source in CI. It does block anyone running upstream's archive on a GPU box.
  - **rc1 has no `bin/btxd` wrapper.** `bin/btxd` is the bare binary with no RUNPATH, so nothing points it at
    the bundled OpenSSL. A read-only dry run on the btxscan box predicts that rc1 btxd **will not start
    there**: the system libssl lacks `OPENSSL_3.2.0`. Upstream's RUNPATH fix `d2eafbbf` landed after rc1, so
    check 0.35 final for it.
  - Linux btxd needs OpenSSL 3.2 from `lib/` (it had no RUNPATH) and links the system
    `libzmq.so.5`. `btx-qt` needs glibc 2.43. `verify_release_btxd.py --archive` now fails on any ELF above the glibc floor.
  - On macOS, Metal kernels moved to `bin/metal/`, and the build is **ad-hoc signed only**. This matters for
    easyNode's Mac bundle, which embeds the official arm64 tarball. Windows is not codesigned.
  - **The rc1 Mac `btxd` does not run as shipped.** Its code signature is invalid ("code or signature have
    been modified"), so macOS kills it with SIGKILL (exit 137). `btx-cli` is fine. Re-signing it ad hoc
    (`codesign -f -s -`) makes it print `v0.35.0rc1`. Measured 2026-10-29 on an M-series Mac. Worth telling
    Jpp/numair. Our staging must re-sign `bin/btxd` itself in the 0.35 layout (it re-signs only `libexec/*.real` today).
  - **The archive layout changed**: no `btx-<ver>/` top folder, `bin/btxd` is the real binary (no `libexec/`),
    and Metal kernels live in `bin/metal/`. easyNode's staging refuses this layout on purpose until it is adapted.
  - A btxd built without ModelNet (like easyNode's engine) **refuses `-modelnet=0`**.

### Network state from the BTX Unicorns group (Oct 24–29, all 318 messages)

- **Only two live signer keys**: ours, `02d5efca` (the RTX 3060 box), and `037db271` (owner unknown).
  zbtx1 has been stuck since 10-10 and zbtx3 quiet since 10-15. Mirrors on 2-of-3 freeze if either key stops.
  **That makes our signer box the most critical machine in this protocol.** Lars plans a GPU box as a 4th signer.
- **btx-pool has about 55–58 % of the hashrate.** That causes frequent 1–2 block twin races. These trigger
  #280/#285 and the "Deep reorg detected 4–6 blocks" warnings, which are expected, not an attack.
- **The official seeds** (node.btx.dev, node.btx.tools, node.btxchain.org) report 228,000, about 30k
  behind. New nodes need our own addnodes.
- **Public archives** (Lars, 24/7, full history): `85.215.122.100:19335`, `217.154.143.140:19335`.
  Lars is also taking over the hourly snapshots.
- Guides pinned in Nodes & Ops: "GPU-less archive sheet v5" (includes the freeze fix) and "Signer + relay guide v0.34.15".
- Our rc1 test (Mende, 10-24, M5): a clone of the easyNode datadir matched rc1 ↔ 0.34.15 in both
  directions (#237). Junior ran rc1 on Windows and it passed, on CPU only since it has no CUDA.

---

## Part B. The protocol

### B0. Triggers

- **A new rc or lab prerelease** (`gh release list -R btxchain/btx --limit 5`) starts **Phase 1**.
- **A final tag** starts **Phase 2**.
- `btx-doctor.sh` (btx-apps btx-ops) warns when upstream is newer than the box.
- Jpp or numair announce it in BTX Unicorns. Read it through ORCA, read-only, never post:
  `ssh mende@20.74.155.31 '~/.local/bin/orca-group inbox btx-unicorns -n 500 --hours 168'`.
  The default inbox returns only 30 messages, so always pass `-n`.
- **Ask Jpp or numair which commit the final tag will be cut from.** If final equals a commit we already
  built in Phase 1, Phase 2 really is just a hash swap.

### B1. Phase 1: during the rc (days before final)

1. **Pin census**: `python3 scripts/btx-pin-census.py --target <tag>` in easynode (branch
   `claude/btx-pin-census`, `9a424c8`, until merged). It is read-only and reads every repo at `origin/main`
   plus ainode through the GitHub API. It names every engine hash and commit against upstream's real release
   digests and exits 1 when a code or config pin is on another release. Today `--target v0.34.15` passes
   (118 pin lines) and `--target v0.35.0-rc1` lists the 13 pins a bump has to move. Run it at the start
   (the to-do list) and at the end (proof nothing was missed).
2. **Read the dangerous diff** against the last shipped tag, once, into a verdict table
   (format: `pq-wallet/docs/ROUND-0.34.15-send-in-parts-2026-10-11.md`):
   - `src/kernel/chainparams.cpp`, `src/consensus/params.h`: heights, assumeutxo rows, checkpoints, seeds.
     Also check that no assumeutxo base we use was withdrawn (recipe :523).
   - `src/script/pqm.*`, `script.h`, `pq_keyderivation.cpp`, `pqkey.cpp`, `key_io.cpp`: addresses and HTLC rules (qid, pq-wallet).
   - `src/primitives/*`: header and tx layout (the rust-btx decoder in btx-esplora).
   - `src/rpc/*.cpp`: added, removed or changed RPCs and fields.
   - `src/init.cpp` and `config.cpp`: flags and defaults. **Unknown conf keys are fatal**, so check that every key
     in every conf we run still exists.
   - `src/validation.cpp`, `src/net_processing.cpp`: reorg, mirror and IBD behaviour. Check `BLOCK_VALIDATION_EPOCH`.
   - `CMakeLists.txt`, `scripts/release/*`, the release checklist: version, archive names, layout, glibc floor, signing, CUDA.
3. **Build the engines now** (easyNode CI): dispatch `btxd-linux.yml`, `btxd-windows.yml` and
   `btxd-macos.yml` on the **commit SHA** of the newest 0.35 build that contains `6b94704b`.
   Record the run ids. A rebuild on final then only happens if the final commit differs.
4. **Patch the tooling for the new release format.** For 0.35, `btx-apps/.claude/skills/btx-ops/recovery/UPGRADE-btxscan-engine.sh`
   **cannot stage it as written**:
   - `ASSET="btx-$VER-x86_64-linux-gnu.tar.gz"` is hard-coded (:82)
   - it unpacks with `tar -xzf` (:121)
   - it checks `SHA256SUMS` with `sha256sum -c` (:119)
   - it wants bare hex in `SHA256=`
   - the version comes from the file name

   Status 2026-10-29:
   - **btxscan script**: fixed in btx-apps PR #289 (open).
   - **easyNode staging**: the download side is fixed in easynode PR #222. Adapting to the 0.35 layout is still open.
   - **ainode** `remote-setup.sh` / `stage-engine*.sh` and **btx-mine-hq** `vast-host.sh`: still to do. They hard-code
     the 0.34 name, `tar -xzf` and the `btx-0.34.15/libexec/` member path.
5. **Shadow canary** (instead of using the production explorer node as the canary). Run a third btxd on a
   **clone of a real datadir** (Mac: `cp -ac`, an APFS clone of 135 GB in 0.2 s; box: an LVM or rsync copy), on other ports,
   with its own conf copy, connected to live peers. Soak it for **24 h or more** on the rc build. Also run:
   - a **genuinely empty datadir** start, from the snapshot (recipe :722)
   - a **from-genesis** sync, or at least past 126,978, once #272 lands. Lars has offered to do this on his test box.
   - the **real conf** of every box (persisted `btx_rw.conf` settings killed easyNode 0.6.15)
6. **Draft PRs everywhere**, with pins moved to the rc SHA and hashes left as `TODO-final`: easynode, EasyBTX site,
   ainode, btx-mine-hq, btx-apps (copy and script), pq-wallet (fixtures), qid (if needed).
   Regenerate the PQ-wallet fixture and run the qid and pq-wallet regtest suites on the rc build now.
7. **Signer box plan** (see B3.0): decide whether the signer moves on release day or after a soak. **Never move it
   while it is one of only two signers unless a second signer is confirmed live.**

### B2. Phase 2: release day (final tag)

1. **Gate.** Every one of these must pass, or nothing ships:
   - The tag is upstream's final/"Latest", and `git merge-base --is-ancestor 6b94704b <sha>` passes.
   - The guards pass on the **literal tag**. Without an argument they can exit 0 on a known-bad pin.
     `check-engine-tag.sh` also refuses a tag that narrows the 32-byte HTLC rule (the rc1 bug) on
     branch `claude/btx-035-engine-guards`, `221693d`, until merged.
     ```bash
     cd ~/repos/easynode && scripts/check-engine-tag.sh <tag> && scripts/check-engine-fleet-ready.sh <tag>
     ```
   - The recipe's two-step golden-manifest seal check (`node-release-recipe.md` :639+) passes.
   - We downloaded every asset ourselves, and our hash matches the GitHub digest and SHA384SUMS. Upstream has
     regenerated assets in place before (v0.32.11), so hash again on publish day.
   - **The network is not split**: easybtx.com/api/nodes, both pools, `witness-1.easybtx.com/signers/recent`,
     api.btxscan.io and both Lars archives agree on the block hash at a settled height.
     Run `scripts/observer-ok.sh` (recipe :870). Never trust btxscan alone.
2. **Diff the final against the rc we soaked.** If only the version string changed, swap the hashes in the draft PRs.
   If code changed, re-run the B1.2 diff and B1.5 soak on that delta only.
3. **Roll out in the order of B2.4.** Pause all watchers first (B3.1 step 1).
4. Order:

| # | App | Repo | Note |
|---|---|---|---|
| 1 | Shadow canary on the final build (short soak, ≥ 1 h) | — | Already soaked on rc in Phase 1 |
| 2 | btxscan / esplora box: archive `btxd`, then serving `btxd2` | `btx-apps` btx-ops + `btx-esplora` | The wallet and bonuz read balances here, so keep the swap boring |
| 3 | AI node (`btx-modeld`) | `MendeMatthias/ainode` (GitHub only) | Independent of btxd |
| 4 | btx-mine-hq ainode copy | `btx-mine-hq` | Same PR wave as 3 |
| 5 | **EasyNode release** | `easynode` | **Reaches the signer box by auto-update.** Hold the signer first (B3.0) |
| 6 | EasyBTX site: node feed and version copy | `EasyBTX` `site/` | After 5 is published |
| 7 | **Signer box (RTX 3060)** | — | Last, after the network is calm on the new version (B3.0) |
| 8 | qid / PQ Wallet releases (if anything changed) | `qid`, `pq-wallet` | Fixtures already regenerated in Phase 1 |
| 9 | EasyBTX miner (only if MatMul or consensus changed) | `EasyBTX` | Usually nothing |
| 10 | Version copy: btxscan.io, easybtx.com, pq-wallet.com | | |

Steps 2–4 and 8 can run in parallel. minebtx (dexbtx, dormant since August) is skipped unless dexbtx asks.

### B3. Per-app steps

#### 0. The signer box (RTX 3060, Windows 11 + WSL2, easyNode Linux AppImage)

This box runs our signer key `02d5efca` (one of **two** live keys), `witness-1.easybtx.com` (behind a
Cloudflare tunnel; PQ Wallet, btxscan forkwatch and btx-esplora health checks read it), an esplora/electrs index,
CUDA btxd, and snapshot co-signing with Jarek. The pin `02c9cfb7` lives in its `btx_rw.conf`. AInode WSL distros run in the same VM.
See `orca-orchestrator/skills/btx-edge/SKILL.md` and `easynode/NEXT-SESSION.md`.

1. **It updates itself from the easyNode feed** ("the 3060 updated itself through the website",
   NEXT-SESSION 10-13). An easyNode release therefore upgrades our signer, usually within minutes. Before you
   publish the feed (B2.4 #5), turn off app auto-update on this box, or plan the signer move on purpose.
2. Move it only when all of these hold: the canary and btxscan are healthy on the new version; the other live signer is
   signing (`witness-1/signers/recent`); and, ideally, a second signer is already on the new version.
3. Clean stop, wait for "Shutdown: done", update, start. It takes about 40 s to sign again. Watch
   `witness-1/signers/recent` until `02d5efca` signs new blocks.
4. Check the CUDA engine, and that the WSL recovery and the boot task still work. Windows Update driver installs froze it before
   (`docs/incident-2026-10-11-3060-verifier-quarantine.md`).
5. **Rollback**: reinstall the previous easyNode AppImage. Its install key points at the old engine.
   Never roll back below a flag day the chain has passed.

#### 1. btxscan / esplora box (Azure VM `btx-esplora-1`, `20.86.181.203`, serves api.btxscan.io)

`btxd` is the archive (P2P 19335). `btxd2` is the serving node (RPC 8434) that electrs reads. **The box is the
truth** (`systemctl cat`). The `btx-esplora/deploy/*.service` files still say v0.34.5/6, so never redeploy them as they are.

The script steps below assume btx-apps PR #289 (merged before release day). That PR:
- reads the asset name, compression and sums file from the release
- checks both SHA*SUMS and GitHub's digest
- records the conf backup and restores it on `--rollback`
- writes `autoupdate=0`
- refuses to switch while `btxscan-forkwatch@` runs
- never restages a tree a unit runs from

1. Run `btx-doctor.sh` and make sure it says healthy. **Pause every watcher**:
   - on the Mac: `launchctl bootout …com.btx.ops.forkwatch`, and set `AZ_CONFLICT_TRIES=120`
   - on the box: stop both `btxscan-forkwatch@` units (btxd and btxd2; they can `invalidateblock`). The
     script refuses while they run and prints the stop/start lines. Also pause the selfcheck cron.
2. Extra safety: copy both confs and both units by hand as well (`cp -a … /var/lib/btxscan-upgrade/manual-<ts>/`).
   Without PR #289, `--rollback` only swaps ExecStart back and keeps the new conf.
3. Check that every conf key exists in the new source.
4. `recovery/UPGRADE-btxscan-engine.sh --stage <ver>` until it prints `STAGED_OK`. It refuses a `SHA256=` that
   disagrees with GitHub's digest. Update `MIN_ENGINE` on a flag day. For 0.35, confirm the final archive
   starts on this box (OpenSSL: see the rc1 RUNPATH note in Part A).
5. `--switch btxd` (archive), watch it to tip, then `--switch btxd2`.
6. esplora/electrs: if `src/primitives` or the tx families changed, update `rust-btx`, run its tests and
   `deploy/scan-chain.sh` over the new blocks, then `deploy/deploy-to-vm.sh`. Before any electrs restart, the
   journal must show `sentinel 'F' found`; otherwise expect a compaction outage of over an hour.
7. Explorer: once #281 ships, `getblockstats` works again. Until then take fees from `getblock <hash> 2`.
8. **Verify**:
   - `btx-doctor.sh --quick`
   - `UPGRADE-btxscan-engine.sh --status` (subversion, height, modeld = 0)
   - the same hash at a settled height on api.btxscan.io and esplora.btxbyronbay.com
   - `deploy/healthcheck.sh`
   - `REF_API=… deploy/verify-esplora.sh https://api.btxscan.io`
   - `FIX-btxscan-attestation-gap.sh` if needed

   Then turn the watchers back on.
9. **Never delete old `/data/src/btx-v*-official` trees** (v0.34.5/6/9/12/15 are on the box today); rollback
   needs them. `btx-supervisor.service` is only in the btx-esplora repo and is **not installed on the box**.
   Its v0.34.5 `BTX_CLI` default is a stale repo file, not a live dependency.
10. **Website copy**: `btx-apps/apps/web/src/lib/config.ts` (`version`, `minimumVersion`), `portal/nodes.ts`
    (`NODE_ENGINE`), `portal/stats.ts`, `portal/config.ts`, `sitemap.ts`, and a news post. Vercel ships it on merge.
11. **Rollback**: `--rollback <unit>`. With PR #289 it restores the recorded conf too, and keeps the current one as
    `.bak-rollback-<ts>`. Then restart.

#### 2. AI node (`MendeMatthias/ainode`, Azure `ainode-host-1`, UAE North, `20.196.8.98:29447`)

It runs stock `btx-modeld` only, with no btxd. Flags: `-modelfollowpeers=0 -modelhost=1 -modelbind=0.0.0.0:29447`.

1. Pins, each a version plus sha:
   - `host/remote-setup.sh:15-17,29`
   - `host/btx-modeld.service:2,5,22`
   - `host/RUNBOOK.md:85-89`
   - `host/vast/vast-host.sh`
   - `app/scripts/stage-engine.sh:24-27`
   - `app/scripts/stage-engine-linux.sh:34-37`
   - `app/ENGINE-CONTRACT.md`
2. For 0.35: check the `-model*` flags (they still exist on swarm), the new archive name and layout, and the
   OpenSSL 3.2 and glibc needs.
3. Run `host/setup-vps.sh ainode-host` from the Mac. It keeps the TLS/ML-DSA identity. **Never regenerate the
   identity**, because users pin it.
4. Restage the app engine, then ship the app release and its updater feed (`app/scripts/release-*.sh`).
5. **Verify**:
   - `journalctl -u btx-modeld` shows `PQ1 ready … bind=0.0.0.0:29447`
   - `host/fetch-test.sh 20.196.8.98:29447 3b` passes from another network
   - `/api/ainode-online` shows the counts
6. **Rollback**: each version lives in `/usr/local/lib/btx-<ver>`. Restore the previous service file and restart.

#### 3. btx-mine-hq (Vast fleet kit and dashboard)

1. Copy ainode's `vast-host.sh` into `kit/ainode/vast-host.sh:29-31` (`BTX_TAG`, `ARCHIVE`, `ARCHIVE_SHA`)
   **in the same wave**. Update `tests/test_ainode.py`. Check `kit/ainode/prefix.sha256` (the glibc prefix)
   if the glibc floor moved.
2. Miners (`kit/btx_vast_fleet.py:30-49`) change only if MatMul or consensus changed. Keep a tested fallback pinned.
3. Economics constants live in `kit/btx_vast_fleet.py:61` (`BLOCKS_PER_DAY`) and `btxdash/chain.py` (`subsidy()`).
   Change them only if issuance changed.
4. `tools/sync-azure.sh`, then `python3 kit/btx_vast_fleet.py ainode`.
5. **Verify**: "PQ1 ready" on a box, `tools/fleet-check`, and pool-credited ep/s. Check what shipped before raising an alarm.
6. **Rollback**: restore the old pins, then `sync-azure`.

#### 4. EasyNode (`~/repos/easynode`, lead app)

Full recipe: `docs/node-release-recipe.md`. Release state: `NEXT-SESSION.md`.
(The recipe's step 1 still says "do not move past `1e51f0d1`". That is stale; fix it in the same PR.)

1. Engines were built in Phase 1. If the final SHA differs, re-dispatch at that **commit SHA**. Paste the run ids,
   artifact names, commit and version **together** into:
   - `node-linux-installer.yml:157-159`
   - `node-win-installer.yml:113-116`
   - the `btxd-windows.yml:75` default
2. Pins in `apps/node/src-tauri/src/commands.rs`:
   - `NODE_RELEASE_TAG` (:404). This is the install key: users re-provision only when it changes, so
     **change it whenever the binary changes**. It must start with `vX.Y.Z` (see A.2).
   - `NODE_RELEASE_COMMIT` (:413).
3. Snapshot:
   - Add the test row in `the_snapshot_pin_is_the_newest_base_the_pinned_engine_carries`.
   - Move `snapshot_spec()` if a new mirrored base exists.
   - Refresh `crates/btx-core/src/attested_snapshot.rs pinned_pair()` **every release**.
   - Check the replay context and shielded commitment in `confirmed_snapshot.rs`.
4. Network: `crates/btx-core/src/node.rs`:
   - `engine_follows_network_rules` (:1595): raise it on a flag day.
   - `BTX_ARCHIVE_PEERS`: add Lars's `85.215.122.100:19335` and `217.154.143.140:19335`.
   - `BTX_BOOTSTRAP_PEERS`: the official seeds are stale.
   - `engine_supports_keeper_profile` is a denylist, so there's nothing to add there.
5. Flags: keep `-reorgpolicy=legacy`. The test `the_pinned_engine_launches_with_its_self_updater_off`
   (branch `claude/btx-035-engine-guards`) fails the build if the install key stops yielding `-autoupdate=0`,
   `-matmulrcexecution` or the network-rules floor. Also check `ps` on a real install once. Check that `-allowsinglekeytrustedmirror`, `strict-device` and matmul validation still exist.
   Never pass `-modelnet=0` to this Modelnet-off build.
6. Staging scripts in `apps/node/scripts/`: `stage-node-pkg.sh:34,40,45`, `stage-node-pkg-linux.sh:24-30`,
   the `-source` variants and `stage-node-pkg-windows.ps1`. Update VERSION, the URL pattern (`btx-035-*.tar.xz`)
   and the sum type. `apps/node/scripts/lib/engine-pin.sh` refuses a mismatch.
   `lib/release-archive.sh` (branch `claude/btx-035-engine-guards`) already handles the download side for both
   formats: sha256 or sha384, `.tar.gz` or `.tar.xz`, finding the root folder, and an exact version match, so
   `0.35.0` no longer accepts a `v0.35.0rc1` binary. It **refuses the 0.35 layout** until the copy, rpath and
   codesign steps are adapted. That adaptation is the remaining Phase 1 job, and it must also re-sign
   `bin/btxd` (see the rc1 signature note in Part A).
   - Never run `stage-node-pkg.sh` for a source build.
   - The Mac bundle embeds the official arm64 tarball, which is ad-hoc signed only in 0.35. Re-sign it as today.
7. Tests: the real-engine `#[ignore]` suites against the new btxd, with long RPC waits (10 min).
   Run the gates on the extracted bundles as well.
8. 0.35 specifics: check for a datadir/DB migration (if any, an engine rollback is impossible; say so in the notes),
   the new assumeutxo rows, the wallet and descriptor RPC shape, `BLOCK_VALIDATION_EPOCH`, and
   `getblockchaininfo.headers` (the "blocks behind" under-count is still open).
9. App version in 5 places: `apps/node/package.json:5`, package-lock (×2), `tauri.conf.json:4`, `Cargo.toml:3`,
   Cargo.lock. Then changelog, build, and `gh release` on `MendeMatthias/EasyBTX-releases` as `node-vX.Y.Z`
   **with `--latest=false`**. **Hold the signer box (B3.0) before the feed goes live.**
   Feed: `build-node-feed.sh` / `publish-node-release.sh`, with `OBSERVER_OVERRIDE=1` only after the B2.1
   network check. Never hand-write feeds.
10. **Verify**: CI logs show the btxd version, SMOKE PASSED, a pristine tree and the sm_* archs. Re-download every asset
    and check for 200 OK, the sha, and `minisign -V`.
11. **Rollback**: never re-cut a version. Ship a higher one with the old pin and a new install key.
    Deleting `node-deb.json` rolls back the .deb. Reverting the site PR rolls back the feeds.

#### 5. EasyBTX site (`EasyBTX/site`, easybtx.com on Vercel)

Work from a fresh worktree on `origin/main`. `EasyBTX/apps/node` is an old copy; the node ships from easynode.
1. Feeds: `site/public/updater/latest-node.json` and `node-deb.json`, both from the scripts.
2. Pages and data:
   - `site/src/pages/node.astro` (`REL_*`, `dl.*`)
   - `node/how-to.astro` (the .deb name)
   - `site/src/lib/node-peers.json`, which must equal `node.rs` (checked by `check-node-peers.mjs`)
3. The version copy "run ≥ v0.34.14, v0.34.15 current", in 15 locales plus `llms.txt`.
4. **Verify**:
   - `curl -s easybtx.com/updater/latest-node.json | jq .version` shows the new version
   - `scripts/check-node-links.py` and `check-download-links.sh` pass
   - `/releases/latest` is still the miner
5. **Rollback**: Vercel rollback or revert the PR. If needed, republish the old feed under a higher version.

#### 6. EasyBTX miner (only if MatMul or consensus changed)

The miner bundles no btxd (pool mode).
- Solver pins: `release.yml` `SOLVER_TAG`/`SHA_GBT`, `linux-app-installer.yml:93-106`, `win-app-installer.yml:38`.
- Matador floor: `src-tauri/src/matador_fetch.rs`. Fork height: `src-tauri/src/fork.rs:49`.
- Never use matador v0.9.7.

#### 7. qid (`~/repos/qid`)

Only consensus constants: `src/qid.js:31-48` and `:1045`.
1. Any change in the PQ, script or key files is **critical**, because derived addresses could move. Review it line by line.
2. Run all five regtest suites on the new btxd with `BTX_CLI=<path>` set explicitly.
3. `bun run test` (with the timeout). `dist/qid.bundle.js` must rebuild byte for byte. Land dependency bumps together with bun.lock and dist.
4. **Rollback**: consumers re-pin the previous bundle sha.

#### 8. PQ Wallet (`~/repos/pq-wallet`, remote MendeMatthias/btx-wallet, a light wallet over Esplora)

1. Diff for standardness (weight, sigops, dust, relay fee, RBF), script/pqm, descriptors, the wallet.dat format, reorg
   defaults and error strings.
2. Regenerate `tools/fixtures/node-<ver>-wallet.dat` from the **real** new btxd (done in Phase 1). For 0.35, also test
   a wallet made by **macOS** btxd 0.34.x (#253).
3. Run `tools/regtest/prove-send-in-parts.mjs` and the lab. Update the version comments in `ui/sendparts.js`,
   `tipwatch.js`, `walletdat.js` and `recoverymove.js`.
4. If qid changed: re-vendor `ui/qid.bundle.js`, run `tools/qid-equivalence.mjs` (expect 0 diffs), and update the sha in `ui/bundles.test.js`.
5. Bump the version in package.json, lock (×2), Cargo.toml plus the Cargo.lock entry, tauri.conf.json, index.html (×2), and
   the bundles.test.js pin. Then:
   - build with `build.yml` and `scripts/release-macos.sh`
   - release on `BTX-Wallet-releases` with SHA256SUMS.txt
   - update the site with `pq-wallet-web/scripts/bump-version.sh` (version.json drives the update banner)
   - update the stores
6. **Verify**: `node --test 'ui/*.test.js'`, `cargo test --locked`, then re-download the assets and `shasum -c`.
7. **Rollback**: point version.json and the site at the previous release.

#### 9. minebtx (dexbtx, dormant since August)

This is not our repo. If it comes back, its pins disagree with each other. Fix them together:
`.solver-channel.json`, `install.sh:55-63`, `pyproject.toml:27-28` and `release-tooling/SHA256SUMS`.

### B4. Done checklist (paste into the release PR)

```
Phase 1 (rc)
[ ] Verdict table written (diff, flags, conf keys, epoch, platform, per-app impact)
[ ] easyNode engines built at the 0.35 SHA (contains 6b94704b), run ids recorded
[ ] Staging scripts handle the new archive name / .tar.xz / SHA384 (btxscan, easynode, ainode, mine-hq)
[ ] Shadow canary soaked >= 24 h on a real datadir clone + empty datadir + real confs
[ ] Draft PRs open in every affected repo; PQ-wallet fixture regenerated; qid + wallet regtest green
[ ] Signer-box plan agreed (when it moves, how auto-update is held)

Phase 2 (final)
[ ] Gate: final/Latest, has 6b94704b, guards on literal tag, seal check, our hashes == digests, observer-ok, no split
[ ] Final-vs-rc diff reviewed; hashes swapped in drafts
[ ] btxscan: watchers paused, confs backed up, btxd -> btxd2, doctor healthy, autoupdate=0, watchers back on
[ ] AI node PQ1 ready + fetch-test; mine-hq copy synced
[ ] Signer box auto-update held BEFORE the easyNode feed goes live
[ ] EasyNode released (--latest=false), feed verified; EasyBTX site + copy (15 locales + llms.txt)
[ ] Signer box moved last; 02d5efca signing again on witness-1
[ ] qid / PQ Wallet released if changed; version copy on btxscan.io / easybtx.com / pq-wallet.com
[ ] Short update drafted for Mende to post (never auto-posted)
```

### B5. Rollback rules

- **There is no rollback below an activation height the chain has passed.** You fall off the chain (244,000 lesson).
- Never re-cut a published app version. Roll forward with a higher version that carries the old pin and a new install key.
- Keep the previous engine side by side (`/usr/local/lib/btx-<ver>`, `/data/src/btx-v*-official`), together with
  **backups of both the unit and the conf**. Rollback is then a file swap and a restart.
- If a release migrates the datadir or bumps `BLOCK_VALIDATION_EPOCH`, rolling back means restoring a datadir backup
  or syncing from a snapshot. Take the backup before the switch (`cp -ac` on a Mac).
- Roll back the signer box last and only on purpose. If it goes down, a 2-of-3 mirror network freezes.

---

## Part C. Lessons this protocol is built from

| Lesson | Where it came from |
|---|---|
| Upstream tags can be bad. Ship only what upstream calls clean, and run guards on the literal tag | v0.33.4–v0.34.1 `nMatMulStallRecoveryHeight=199'299` fork; the grep guard was vacuous |
| An rc can be consensus-different from the release before it | v0.35.0-rc1 HTLC narrowing (`68f2bf3e`, fixed in `6b94704b`) |
| Do the risky work on the rc; make release day a hash swap | This review (v2) |
| An easyNode release also upgrades our own signer | NEXT-SESSION 10-13 ("the 3060 updated itself through the website") |
| Change the install key whenever the binary changes, and keep it `vX.Y.Z…` or `-autoupdate=0` silently drops | easynode 0.6.18/0.6.23; `node.rs:995-1003` |
| Fail closed; never silently fall back to an older engine | easynode `82a5aaf`; btx-apps `497a809` |
| Rolling back below a flag day strands you | btxscan "244,000 split", `7fad386` (9 h frozen at 244,089) |
| Pass defaults explicitly (`-reorgpolicy`, `-autoupdate`) | easynode `48d1d5c`, btx-apps `5b6dd3b`, pq-wallet `3044ead` |
| Unknown conf keys are fatal, also when rolling back to an older engine | btx-apps `1d76c1a`; `--rollback` did not restore the conf (fixed in btx-apps PR #289) |
| Test on a real datadir with real confs and on an empty one | easyNode 0.6.15 (persisted `btx_rw.conf`), recipe :722, :1286 |
| Edit the file that holds the last ExecStart (drop-ins) | btx-apps `e6dd77f` |
| Refresh the attested snapshot pin every release | easynode `3e5a380`, `b01c4e5` |
| Test against fixtures made by the real new btxd | pq-wallet `3292a7a` |
| Set tool paths explicitly in regtest harnesses | qid `87e1b7c` (78 days untested) |
| No SHA256SUMS since 0.34.13, and 0.35 uses SHA384 and `.tar.xz`: scripts must read the format from the release | btx-apps `e66a3fe`, EasyBTX #574, UPGRADE script :82/:119/:121 |
| Never hand-write updater feeds; publish node releases with `--latest=false` | EasyBTX feed traps |
| Dispatch CI with a commit SHA and move all `PROVEN_BTXD_*` pins together | `btxd-windows.yml` default sat three engines stale |
| The box is the truth; repo units and docs go stale | btx-esplora `4c6831e`, `49b686d` |
| Pause watchers on the Mac **and** on the box before an upgrade | btxscan topology notes; `btxscan-forkwatch@` can `invalidateblock` |
| Website version copy is part of every bump | btx-apps `c00eb8d`, `15d7c1b` |
| A GPU-less box needs `matmulvalidation=trusted` | btx-esplora `23241c5` (20 days at 199,311) |
| Platform floors move (glibc 2.38 → 2.43 for Qt, OpenSSL 3.2, system libzmq, CUDA off in 0.35 official archives) | rc1 checklist, #237 |
| Keep a tested fallback miner pinned | btx-mine-hq `cf49022` |
| Check what actually shipped before raising alarms | btx-mine-hq `ac02fef` |
| Reintroduce the bug to prove a test can fail | EasyBTX `aa70f391` |
