# AInode feasibility research: a desktop app on BTX's Native Model Network

Date: 2026-10-11. Evidence base: `btxchain/btx` at `main` = `476f3f23` = tag `v0.34.15` (read-only static inspection, nothing built or executed), the easyNode worktree at 0.7.8 (`00abdad`), GitHub metadata via `gh`, and web search. Line numbers refer to those exact trees.

Evidence labels used throughout:

- **SHIPPED+TESTED**: code exists and a Boost or functional test in the tree exercises it (on regtest or loopback unless stated).
- **IMPLEMENTED, UNPROVEN**: code exists, no test or only a self-reported script result; never seen on a real network.
- **SPECIFIED ONLY**: written in docs, no code path, or the code refuses.
- **INFERENCE**: my conclusion from the above, not something the repo states.

---

## One-page verdict

**DO NOT BUILD YET.** Not as "AInode" with an earning story, and not yet even as a pure download client. A narrower free-only client becomes defensible on the day one condition below is met; today it is not.

### The three strongest reasons

**1. There is nothing to earn, and the two things that do pay need a full wallet node.**
The protocol pays nobody for hosting, seeding, or GPU capacity. `doc/modelnet/economics.md:17-24` lists "protocol emissions or token rewards for hosting or advertised capacity" under *what does not exist*, and no code contradicts it (a search for sponsor/mirror payment in `src/modelnet/` finds only HCP string fields). "Paid delivery" is not functional: `getmodel` in any paid mode returns `APPROVAL_REQUIRED` (`src/modelnet/helper.cpp:7284,7299`), the helper advertises `paid_chain_verify=false` (`src/modelnet/catalog.cpp:154`), the peer-facing `/quotes` and `/payment` endpoints answer `403 OWNER_ONLY` (`helper.cpp:2928-2936`), and the only gate on serving a piece is a signed *free* grant (`helper.cpp:2071-2136`). BTX actually moves in exactly two places: release campaigns (funders pay the **publisher** who reveals a secret) and bounties (contributors pay the **winning model creator**). Both receive through `buildhtlcclaim` / `preparebountyaward`, which are wallet RPCs on `btxd`; `buildhtlcclaim` says "The wallet must hold the claimer's PQ private key" and blocks until the wallet is synced to the chain (`src/wallet/shielded_rpc.cpp:16487-16500`, `+41`). So the app is light for consumers and heavy for earners, and the earners are model publishers and ML researchers, not people with a good GPU.

**2. The network is empty and has no way to find itself.**
No compiled bootstrap peers, no DNS seed for the model plane (the three DNS seeds in `src/kernel/chainparams.cpp:1013-1015` are monetary only), every `-modelpeer` example in docs and e2e scripts is `127.0.0.1`. The monetary P2P `MDPEERS` hints that could introduce model peers are drained and logged "hint only; not connecting" and never handed to the helper (`src/net_processing.cpp:20312-20333`, the #172 fix). The one real model URI in the tree (granite-4.0-h-tiny) was served from the developer's own box at `/opt/btx-0347-rc` (`contrib/modelnet/granite_loopback_retrieve.py:13-17`). Both visible community operators switch the plane off: easyNode strips the seven helpers and builds `WITH_MODELNET=OFF` on all three platforms; the btxscan mirror ops script writes `modelnet=0` into its conf. Web search finds no mention of the model network outside the repo. The model plane merged to `main` on 2026-09-16 (PR #156), under four weeks before this tag.

**3. The platform story does not reach Windows, and the Linux and storage defaults need work upstream has not done.**
Upstream has never published a Windows binary (release asset lists for v0.34.6 through v0.34.15; easyNode `docs/node-release-recipe.md:915-916`). The depends tree has no OpenSSL package at all (`depends/packages/`), and 20 of the helper's source files include POSIX-only headers (unix sockets, `popen`, `posix_spawn`, `statvfs`; `<arpa/inet.h>` is unguarded at `helper.cpp:67`). Windows is a port, not a build. On Linux the shipped archive needs glibc 2.38 (issue #169) and OpenSSL 3.5 with ML-KEM-768 (issue #202: Ubuntu 24.04 fails to load, 25.04 fails closed, 25.10 works). The packaged default is `-modelstorage=auto` = min(512 GiB, 10% of the disk) with a 32 GiB floor, plus `-modelfollowpeers=1`, which downloads and seeds models the user never asked for; the helper never reads the consent file that exists for this purpose.

### The single biggest unknown

**Whether any reachable mainnet host with a real, redistributable model will exist, and who will run it.** Every other gap (Windows port, bundling OpenSSL 3.5 and llama.cpp, consent UI, bootstrap list) is engineering. Supply is not. Nothing in the repo, on GitHub, or on the web shows a single public `btx://` model being seeded by anyone other than the author's test machine, and I could not find a way to prove the negative either: there is no public census of model-plane peers.

### If the owner still wants a narrower build

The only honest shape is a **macOS/Linux free-download-and-seed client with zero earning claims**, gated on three preconditions: (a) at least two independent, reachable mainnet hosts seeding real models (the app ships their endpoints as `-modelpeer`), (b) upstream fixes the open helper deadlock (#225) and the two-tier unix-socket-only control surface is wrapped, (c) an explicit storage consent screen that defaults peer-follow **off**. Even then, upstream already ships its own Qt desktop with a Models dock on macOS since 0.34.12 (`src/qt/bitcoingui.cpp:119-124`, `src/qt/modelnetpage.cpp`, 2552 lines), so the differentiator would be packaging and onboarding only.

---

## Verification of the six prior findings

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | `btx-modeld` runs standalone without `btxd` | **True, but incomplete** | `src/modeld.cpp:27-67` is a standalone entrypoint with its own args and no chain code; `test/functional/feature_modelnet_firstrun.py:27` starts it beside a `btxd -nomodelnet`. What it cannot do alone: discover peers (see B), observe chain funding (`src/rpc/modelnet.cpp:177-209` feeds observations from a `btxd` wallet), or run Pay With Compute qualification (`src/rpc/compute.cpp:32-49` anchors to `btxd`'s active chain). |
| 2 | easyNode builds `WITH_MODELNET=OFF` and strips 7 helpers | **True** | `README.md:268-277`; `apps/node/scripts/stage-node-pkg.sh:86-98` removes `btx-modeld btx-modelcheck btx-open btx-capability btx-capabilityd btx-hcpd btx-hosted`; `commands.rs` (around line 281) "Built with -DWITH_MODELNET=OFF on all three platforms"; `.github/workflows/btxd-windows.yml:272-285` passes `-DWITH_MODELNET=OFF` because the depends tree has no OpenSSL 3.5. |
| 3 | Hosting/seeding earns no BTX | **True** | `doc/modelnet/economics.md:17-24`; `doc/modelnet/README.md:145-148`; no payout code for hosts anywhere in `src/modelnet/`. |
| 4 | PWC credits are closed, not BTX | **True** | `doc/pay-with-compute.md:122-123` "not transferable, not cash-redeemable, no cross-agreement credit, and no carryover"; `:133` "There is no global P1E balance and no `sendcompute`". PR #223 (merged into 0.34.15) repeats "does not turn Pay With Compute into a spend path". |
| 5 | Real BTX flows for sponsored mirroring, bounties, release HTLCs | **Two of three** | Release HTLCs: `wallet_modelnet_funding.py` (regtest, prepare/sign/submit). Bounties: `feature_modelnet_bounty_lifecycle.py` (regtest, CLTV award + refund). Sponsored mirroring: **SPECIFIED ONLY**. `economics.md:17-18` says "A sponsor may pay a mirror"; there is no RPC, record, or wire path for it. It means an ordinary BTX transfer arranged off-protocol. |
| 6 | Hosting needs OpenSSL 3.5+ with MLKEM768 and mldsa44 | **True, and stricter than stated** | `src/modelnet/transport_pq.cpp:44-56` fails closed if either algorithm is missing. The helper also shells out to an `openssl` CLI that lists MLKEM768 to mint its ML-DSA-44 certificate (`src/modelnet/pq1_runtime.cpp:67-98`, `doc/build-osx.md:187-189`), so the **CLI** must ship too, not only the library. |

---

## Priority question: can a user get paid without a full node?

### Short answer

**No path exists in which a home user earns BTX by running this app, with or without a full node.** The two paths that do pay are for model publishers and model creators, and both require a `btxd` with a loaded wallet that holds the recipient's post-quantum private key and is synced to the chain tip. A pruned or snapshot-started `btxd` is probably enough (see "lighter paths"), but the helper alone is never enough.

### What the code says, path by path

**Release campaigns (the only HTLC money).**
- Funding side: `preparemodelfunding` is compiled only under `ENABLE_WALLET`, calls `WalletForModelFunding(request)` and `wallet::CreateUnsignedFunding(*pwallet, …)` (`src/rpc/modelnet.cpp:1114-1151`); `signmodelfunding` signs "through wallet policy"; `submitmodelfunding` broadcasts through `btxd` (`:1164-1300`). The helper's own `submitmodelfunding` only journals a txid and reports `broadcast: false`, `paid_chain_verify: false` (`src/modelnet/funding.cpp:612-640`).
- Receiving side: the payee is the `claimant` PQ pubkey the publisher hands to funders (`src/rpc/modelnet.cpp:1092`; `src/modelnet/release.cpp:150` "htlc_sha256(key_hash, claimant)"). To collect, the publisher reveals the 32-byte secret and spends with `buildhtlcclaim`, a wallet RPC (`src/wallet/rpc/wallet.cpp:1694`) whose help states "The wallet must hold the claimer's PQ private key to produce a transaction-bound claim signature" and which calls `pwallet->BlockUntilSyncedToCurrentChain()` and refuses to reveal the preimage until the funding output has `min_confirmations` (`src/wallet/shielded_rpc.cpp:16487-16500`, body at `+41`, `+106-143`). The helper's `buildmodelhtlcclaim` only emits an unsigned template, "complete=false until the tx is signed" (`src/rpc/modelnet.cpp:1419-1422`); `claimmodelrelease` "updates model-plane unlock state only" (`helper.cpp:7854-7860`).
- Seeing that you were funded: `getmodeleconomyentry` and friends join chain facts only when `btxd` has a wallet (`MaybeWalletForObservation`, `src/rpc/modelnet.cpp:177-209`), and `wallet::ObserveReleaseFunding` scans `wallet.mapWallet` (`src/wallet/model_funding.cpp`), so the wallet must own or watch the HTLC script. A standalone helper shows `pledged_atoms` (non-binding) and whatever it is told.
- Timing: claim before `refund_height`; after it, claim and refund race (`doc/modelnet/htlc-reuse.md:29-36`). A publisher offline at the wrong time loses the money to refunds.
- Mainnet status: the 32-byte-preimage HTLC rule became consensus at `BTX_SECURITY_ACTIVATION_HEIGHT` = 244,000 (`src/kernel/chainparams.cpp:181, 812-814`), which the chain passed before 2026-10-10 (easyNode `commands.rs:366-376`). So the primitive is live; I found no evidence any campaign has used it.

**Bounties.** Contributors fund lots into a two-leaf escrow; the winner is paid by an M-of-N council CLTV spend after `award_height` (`preparebountyaward`), unawarded lots refund to the contributor key after `refund_height` (`doc/bounties.md:25-34, 67-87`). All wallet RPCs on `btxd` (`src/rpc/modelnet.cpp:1641-1984`). Helper `approvebountyaward` "is policy only and is not a transaction signature". Readiness: `contrib/modelnet/bounty/tests/acceptance-matrix.csv` has 245 mandatory rows: **88 PASS, 140 NOT_RUN, 17 UNSUPPORTED_ENVIRONMENT**, executed 2026-09-15 on isolated regtest.

**Paid delivery.** Not a path. Besides the `APPROVAL_REQUIRED` / `paid_chain_verify=false` / `403 OWNER_ONLY` facts above, `doc/modelnet/architecture.md:78` says of quotes/payment/releases on the wire: "Not implemented in this tree; capabilities stay false", and `doc/modelnet/http.md:22-23` marks `/payment` as "**no chain verify**". A seller has no way to know a buyer paid, so no seller can price anything.

**Pay With Compute.** A GPU owner can earn *access credits* to one developer's resource by running P1E jobs. Credits are non-transferable (above). Qualification challenges live on `btxd` and are accepted only "while the anchor block is still on this node's active chain" (`src/rpc/compute.cpp:32-49`; `doc/pay-with-compute.md:36-39`), so this path also needs a synced `btxd`. Production solve is off unless `-enablecomputeproductionwork=1`. No BTX changes hands.

**Mining** is the actual GPU-to-BTX path and is a different product (EasyBTX).

### Lighter paths examined

| Idea | Finding |
|---|---|
| Watch-only wallet | Can **observe** (an existing lock "can still be imported watch-only and spent", `htlc-reuse.md:27-28`; `ObserveReleaseFunding` works on anything in `mapWallet`) but cannot **claim**: the claim signature needs the PQ private key in the wallet. |
| External signer / PSBT | No PSBT or external-signer path for the P2MR HTLC claim found; `buildhtlcclaim` builds, signs and finalizes in one wallet call. **INFERENCE**: a hardware-signer flow would be new upstream work. |
| Pruned node | `btxd` keeps Bitcoin Core's `-prune` (`src/init.cpp:762`). easyNode's Keeper role is a pruned (`prune=5000`), snapshot-started node at about 10 GB (`README.md:112`, `crates/btx-core/src/node.rs:187`, `docs/snapshot-serve.md`). **INFERENCE**: wallet claim/refund needs the tip and the funding tx, not archival blocks, so a Keeper-class node should suffice. Not verified against BTX-specific restrictions. Note a non-GPU node follows other nodes' attestations rather than validating (`README.md:120-124`). |
| An address someone else pays to | Works trivially and has nothing to do with the model plane: it is a donation, with no escrow, no refund path and no link to the model. The campaign's whole point (trust-minimised reveal-for-payment) disappears. |

### What the app would need to let someone earn

A `btxd` (snapshot start, pruned, roughly 10 GB plus sync time), a loaded wallet with an ML-DSA/SLH-DSA key the user controls and backs up, a UI that exports the claimant pubkey to funders before the campaign, a watcher for funding confirmations, and a claim flow that runs before `refund_height`. That is easyNode plus a wallet, for an audience of model publishers. It has nothing to offer a seeder.

---

## A. Is it real, or only specified?

### Implemented and tested (regtest / loopback)

All of these run in `test/functional/test_runner.py` (lines 531-558):

| Capability | Code | Test | Label |
|---|---|---|---|
| Import / host / pin / list / manifest / export path | `helper.cpp` (10,794 lines) | `feature_modelnet_helper.py` (one regtest node + spawned helper, unix RPC only), `feature_modelnet_firstrun.py` (quota 0 refuses import, 8 MiB quota stores) | SHIPPED+TESTED |
| FREE_ONLY retrieve, multi-seeder swarm, seeder failover | `swarm.cpp`, `piece_picker.cpp`, `transfer_session.cpp` | `feature_modelnet_swarm_live.py` (3 seeders + 2 buyers on loopback, 8 MiB synthetic safetensors) | SHIPPED+TESTED (loopback) |
| Demand-seed after retrieve | `direct_seed.cpp` | `granite_second_process_retrieve.py` asserts `seeded=true` | SHIPPED+TESTED (loopback) |
| Search (LOCAL and NETWORK fan-out) | `search.cpp` (1,631 lines) | `feature_modelnet_search.py` (regtest; "coverage is never complete") | SHIPPED+TESTED |
| PQ1 TLS (pure ML-KEM-768, ML-DSA-44, TOFU SPKI pin) | `transport_pq.cpp`, `pq1_runtime.cpp` | unit tests `modelnet_b0_pq_rest_tests.cpp` etc.; needs OpenSSL 3.5 host | SHIPPED+TESTED |
| Release funding prepare/sign/submit | `src/rpc/modelnet.cpp`, `src/wallet/model_funding.cpp` | `wallet_modelnet_funding.py --descriptors` (regtest) | SHIPPED+TESTED |
| Bounty lifecycle incl. on-chain CLTV award and refund | `bounty.cpp`, wallet RPCs | `feature_modelnet_bounty_lifecycle.py --descriptors` | SHIPPED+TESTED (regtest); readiness matrix 88/245 PASS |
| Pay With Compute (toy profile) | `compute_economy.cpp`, `src/rpc/compute.cpp` | `feature_pay_with_compute.py`, `rpc_compute_qualification.py` | SHIPPED+TESTED (regtest toy profile only) |
| Structure check (SafeTensors/GGUF headers, refuses pickle) | `qualification.cpp`, `btx-modelcheck` | unit tests | SHIPPED+TESTED |

### Implemented, unproven on any real network

- **WAN transfer.** `contrib/modelnet/e2e-two-wan.sh` is named WAN but runs "Two seeders + fetcher on one dedicated host" with all binds on `127.0.0.1` (`e2e-two-wan-remote.sh:16-18`). The only genuine WAN datapoint in the tree is a hard-coded heredoc in `e2e-production-evidence.sh:30-33`: 13,888,336,427 bytes in 6,562 s (about 2.1 MB/s). It cannot be re-verified.
- **NAT traversal, relay, hole punch.** Code exists (`model_nat.cpp`, `relay_reserve.cpp`, `reachability.cpp`); tests are unit-level and a namespace lab. The relay forwards at most 32 MiB per reservation (`relay_reserve.h:18`), so relayed delivery of a model is not a thing; relay is for rendezvous.
- **Peer follow / preserve-rare propagation.** `propagation.md` describes it; `helper.cpp:3793` defaults it on; no multi-host evidence.
- **Local generate.** `generatemodel` `execv`s an operator-supplied `llama-cli` or Python adapter (`helper.cpp:501-556`, `doc/modelnet/generate.md:17-58`). Nothing is bundled. CUDA runtime qualification is `NOT_RUN` by default (`doc/modelnet/cuda-not-run.md`).

### Specified only, or refusing

- Paid delivery over the wire (above).
- Sponsored mirroring (above).
- Live Hugging Face / Xet / torrent origins: `HuggingFaceByteSource::Read` returns "not wired to live network" (`source_huggingface.cpp:43`) unless the operator opts into `live_wan` (`registry-independence.md:59-67`); `rpc.md:856` lists live HF HTTP, `btx-torrentd`, live R2 WAN, SCALE huge and GUI as NOT_RUN / operator-gated.
- Cloud (S3/R2) backends: FakeS3 unit-tested, live R2 "HONEST_NOT_RUN" (`storage-backends.md:4-7`).

### The acceptance matrix is not what it looks like

`planning/acceptance-matrix.csv` reports 469 PASS and 1 NOT_RUN, but its header says "Packaged **0.34.7** production acceptance matrix" and "PASS only where this tree has a Boost test or an executed e2e script". It is a self-reported, local-test tally frozen eight releases ago, not a network acceptance.

### Docs drift that matters for an app

- `doc/modelnet/README.md:80` "Fresh-install defaults: payload storage **0**" vs `src/init.cpp:4242` default `"auto"` and `HUMANS.md:290` (issue #209 updated HUMANS but not this README).
- `researcher-quickstart.md:9-11` says Qt Models pages are "out of scope for this tree" and "Desktop btx-qt is not a documented shipped surface"; `src/qt/modelnetpage.cpp` is 2,552 lines and every archive since 0.34.12 includes `btx-qt` (release notes v0.34.12).
- Most model docs are frozen at "0.34.7 / 0.34.8 / 0.34.9-dev" headers while the tree is 0.34.15.

### Open defect at this tag

PR #225 (open): a 64-hex id passed to `getmodelreleaseeconomics` or `cacheencryptedmodel` deadlocks the helper; "This is the one defect from the v0.34.15 re-verification that wedges a process."

---

## B. Is anyone using it?

**No evidence of any mainnet model-plane activity; the negative cannot be proven.**

- No bootstrap: no compiled model peers, no DNS seed, no public endpoint anywhere in `doc/`, `contrib/` or `src/` (every `-modelpeer` is `127.0.0.1` or `[::1]`). `bootstrap.md:7-13` lists "compiled defaults where present"; none are present.
- No introduction from the money network: `MDPEERS` hints are consumed and logged "(hint only; not connecting, not spending)" (`net_processing.cpp:20323-20333`). The helper never learns a peer from `btxd`.
- No public models: the one real URI in the tree is granite-4.0-h-tiny on the author's `/opt/btx-0347-rc` host (`granite_loopback_retrieve.py:13-17`); the search record schema has no public index anywhere.
- Operators turn it off: easyNode (OFF on all platforms, helpers stripped); the btxscan mirror (`btx-apps/.claude/skills/btx-ops/recovery/UPGRADE-btxscan-engine.sh:17-18` adds `modelnet=0`).
- Upstream footprint: 51 stars, 22 forks, 1 open issue; 224 of roughly 300 commits by one author (`gh api repos/btxchain/btx/contributors`); the model plane landed on `main` 2026-09-16 (PR #156). The 29 issues matching "modelnet" are review findings against release candidates, none reports a hosted model.
- Web: searches for the Native Model Network, `btx-modeld`, `btx://` return nothing beyond a [Trendshift listing of the repo](https://trendshift.io/repositories/38084); "btxscan"/"btx.best" searches return unrelated results.

If the network has peers, no document, explorer, or census shows them. A client built today would open to an empty catalog unless the owner runs the seeds himself.

---

## C. Does the economic loop close?

| Path | Who pays | Who receives | Trigger | Realistic take for a home user | Status |
|---|---|---|---|---|---|
| Release campaign | Funders with a `btxd` wallet (prepare/sign/submit) | The **publisher** (claimant PQ key) | Publisher reveals the secret and claims before `refund_height` | 0 unless you authored a private model people will pay to open | SHIPPED+TESTED (regtest); unused on mainnet as far as evidence shows |
| Bounty | Contributors (lots) | The **winning creator**, via council M-of-N | Council signs after `award_height`; evaluator reports are policy, not payment | 0 unless you do the ML work and win | Regtest-tested; 140/245 readiness rows NOT_RUN |
| Paid delivery / quotes | (nobody) | (nobody) | Returns `APPROVAL_REQUIRED`; wire endpoints 403; no chain verify; serving gated by free grants only | 0 | SPECIFIED ONLY |
| Sponsored mirroring | A sponsor, by ordinary transfer | A mirror operator | Off-protocol agreement | 0 unless negotiated by hand | SPECIFIED ONLY |
| Pay With Compute | Nobody pays BTX | GPU owner gets closed access credits | Receipts for P1E jobs | 0 BTX | Regtest toy profile only |
| Seeding | Nobody | Nobody | Reciprocity is "a local convenience target, not a debt" (`free-first-policy.md:45`) | 0 | By design |

**Home machine vs datacenter**, in case sponsorship ever becomes real: to be advertised as a host you need `PUBLIC_DIRECT` or `PUBLIC_MAPPED`, i.e. two successful dial-backs from distinct netgroups (`reachability.md:16-18, 32-38`). PCP/NAT-PMP is attempted once at start, one try, 3600 s lifetime (`model_nat.cpp:200-222`). Behind a symmetric NAT you stay `RELAY_REACHABLE`, and relays cap at 32 MiB per reservation, so you cannot serve a model through one. Inbound is capped at 16 connections, 8 per netgroup, outbound 8 (`pq1_runtime.h:24-27`). A datacenter mirror with a public IPv4 and symmetric bandwidth wins on every axis; the "metrics that count" are "bytes actually served" (`economics.md:43-47`), which a domestic uplink cannot compete on. The design explicitly says mirrors get "no influence over free ranking" (`README.md:78`), so there is also no reputation to accumulate.

The loop closes only for publishers and creators who already have an audience, and only if that audience runs wallet nodes. Nothing in it closes for the AInode user.

---

## D. Packaging

**What ships (minimum):** `btx-modeld` (the helper), `btx-modelcheck` (structure check), `btx-open` (preview dispatcher), optionally `btx-capability`/`btx-capabilityd` (`src/modelnet/CMakeLists.txt:169-204`). Plus, for the headline features: an OpenSSL 3.5+ **CLI** (certificate minting via `popen`, `pq1_runtime.cpp:67-98`), and a `llama-cli` or Python adapter for "run models locally" (`generate.md`). None of the runtime pieces are in the repo.

**Sizes:** per-binary sizes could not be measured without downloading (not done). Whole upstream archives for v0.34.15: macOS arm64 72.4 MB, Linux x86-64 116 MB, Linux CUDA13 858 MB (each also contains `btxd`, `btx-qt`, and on Linux bundled Qt 6).

| Platform | Build/dependency reality | Label |
|---|---|---|
| macOS arm64 | CMake prefers static Homebrew `openssl@3` (`CMakeLists.txt:175-182`); Homebrew `openssl@3` is 3.6.5 today ([formulae.brew.sh](https://formulae.brew.sh/api/formula/openssl@3.json)), so the helper is self-contained. Apple's `/usr/bin/openssl` is LibreSSL and cannot mint ML-DSA-44 (`build-osx.md:187-189`), so an OpenSSL CLI must be bundled or `BTX_OPENSSL` set. Feasible. | IMPLEMENTED |
| Linux x86-64 | Dynamic libssl; the release wrapper adds `lib/libssl.so.3` to `LD_LIBRARY_PATH` when present and otherwise fails with a message (`scripts/release/package_release_archive.py:103-135, 207-219`). Needs glibc 2.38+ (issue #169): Ubuntu 22.04 and 24.04 LTS cannot run the upstream archive; 24.04 can run a self-built one if OpenSSL 3.5 is vendored. Ubuntu/Debian system OpenSSL is 3.0 except Debian 13 / Ubuntu 25.10 (issue #202). An AppImage with vendored OpenSSL 3.5 is feasible but a new pipeline. | IMPLEMENTED, fragile |
| Windows | Upstream has never shipped a Windows binary. The depends tree has no OpenSSL package. easyNode's own Windows CI builds `btxd` with `-DWITH_MODELNET=OFF` for exactly this reason (`btxd-windows.yml:272-285`). The helper uses unix sockets for its only control surface (`helper.cpp:112-120`), `popen`, `posix_spawn`, `statvfs`, and unguarded POSIX headers in 20 files; only 9 `#ifndef WIN32` lines exist across 3 files. | NOT SUPPORTED; a port |

**Ports and NAT.** PQ1 TLS over TCP 29447; under `btxd` the default bind is `0.0.0.0:29447` (`init.cpp:645`), standalone the helper is unix-only unless `-modelbind` is given (`architecture.md:34`). Control is a unix JSON-RPC socket with a 108-byte path limit that the tests already work around (`helper.cpp:112-120`). A home user behind NAT retrieves fine (outbound) and serves only to peers who can reach them; without a working PCP/NAT-PMP mapping or manual port forward they are `nat_limited=true` and never advertised as a host (`architecture.md:52-67`). "LAN discovery" is string classification of endpoints as LAN-looking (`lan_discovery.cpp:21-30`), not mDNS.

**Supervision.** Standalone helper means the app owns lifecycle, restarts, TLS material under `tls/` (0600 after issue #161), `catalog.json`, and the piece store.

---

## E. Storage consent

**Actual default:** `-modelstorage=auto` in both `btxd` (`src/init.cpp:4242`) and `btx-modeld` (`src/modeld.cpp:51-52`). AUTO sizing (`src/modelnet/auto_storage.h:31-34`, `auto_storage.cpp:75-100`):

- target = min(512 GiB, 10% of the filesystem holding the model store);
- if that is under 32 GiB, use 32 GiB when the safe free space allows, else whatever is safely available;
- a free-space reserve of max(32 GiB, 10% of capacity) is never consumed;
- grows on intentional imports up to the 512 GiB cap (`-modelstorageautocap`).

Examples: 256 GB laptop with 100 GB free: quota 32 GiB. 1 TB desktop: about 100 GiB. 8 TB NAS: 512 GiB.

**What else happens by default:** `-modelfollowpeers=1` (`modeld.cpp:60`, `helper.cpp:3793`): FREE models announced by any configured or PEX-learned contact are downloaded into spare quota and re-advertised, polled every 5 s (`propagation.md:33-38, 57`). `-modelseed=auto` seeds everything retrieved. `-modeluploadlimit=0` means no byte cap, only the 16-inbound ceiling. `-modelpreserverare` and `-modelallowencrypted` are off.

**Consent machinery exists but is not enforced:** `modelnet/firstrun.json` and `AllowPayloadStorage()` (`src/modelnet/firstrun.cpp:16-30`) are consulted only by `btx-open`, which prints `storage_consent_required=true` (`src/btx-open.cpp:166`). The helper allocates AUTO quota without it. `doc/modelnet/README.md:80` still claims payload storage defaults to 0.

**What a user must be told before first start:** up to 10% of this disk (32-512 GiB) will fill with model files, including models you did not ask for if peer-follow stays on; your machine will upload them to strangers without a cap; TCP 29447 will listen if hosting is enabled; models are verified for structure, not for safety or legality; and the app will run external inference binaries if you enable "run locally". **INFERENCE**: an app should ship peer-follow off and a fixed, user-chosen quota, which means not using upstream defaults.

---

## F. Competition

| | Hugging Face | Ollama | BitTorrent | IPFS / Filecoin | BTX model plane today |
|---|---|---|---|---|---|
| Catalog | Millions of repos | Curated registry | Whatever is seeded | Whatever is pinned | Zero public models found |
| Login needed | No for public models; yes for gated | No | No | No | No |
| Integrity | SHA-256 in LFS pointers, Xet chunk hashes | Digest-addressed layers | SHA-1 (v1) / SHA-256 (v2) pieces | CID content addressing | SHA-384 pieces + ML-DSA-44 publisher signatures + PQ TLS |
| Runtime | None (libraries) | **Bundled llama.cpp**, one command | None | None | Operator-supplied llama.cpp / Python adapter |
| Peers / speed | CDN | CDN | Large swarms for popular files | Variable | One measured 2.1 MB/s WAN retrieve, unverifiable |
| Pays creators | No (paid inference endpoints separately) | No | No | Filecoin pays storage providers | HTLC campaigns and bounties exist, unused |
| Platforms | All | macOS/Linux/Windows | All | All | macOS/Linux; Windows unsupported |

What is genuinely better: publisher-signed, hash-pinned identity that is independent of where bytes come from (`registry-independence.md`); refusal of pickle/`.pt`; post-quantum transport; a specified, trust-minimised way to pay a creator to open a model. What is worse: no content, no peers, no bundled runtime, a NAT story that makes home seeding mostly symbolic, OpenSSL 3.5 as a hard runtime dependency, no Windows, a 10.8k-line helper with an open deadlock, and search whose coverage is "never complete" by design.

For the consumer promise in the brief ("paste a link, download, verify every piece, no account"), HF plus `huggingface-cli`/Xet already does this without an account for public models, and Ollama does it with a runtime attached. The honest answer is that **nothing is better today for a consumer**; the PQ-and-signature story matters to a niche that does not yet have anything to download.

---

## G. Risks

**Legal.** Seeding is redistribution. There is no license gate on import or peer-follow: `license` is only a search filter tag (`search.cpp:687, 1141-1143`), not a field in the signed record (`search.md:75-99`). With defaults, a node downloads and re-serves models announced by its contacts without the user choosing them. If one is non-redistributable (many open-weights licenses restrict redistribution), unlawful, or malicious where the user lives, the user is the distributor. Tombstones are local: `guaranteed_global_delete: false` (`rpc.md:572`). Running an "earn BTX" app also sits adjacent to securities and payments regulation in several jurisdictions; that is a matter for counsel, not this document.

**User control that does exist.** `-modelfollowpeers=0`, `-modelpreserverare` off, `-modelallowencrypted` off; per-publisher / model / artifact / collection / endpoint deny lists (`src/modelnet/acl.h:35-43`); `hidesearchmodel`, `unseedmodel`, `unhostmodel`, `unpinmodel`; signed safety advisories (`safety_advisory_v1`, `catalog.cpp:174`). An app could default to "seed only what I explicitly downloaded" by turning peer-follow off. Nothing lets a user see a model's license before the helper fetches it under peer-follow.

**Malicious models.** `btx-modelcheck` reads SafeTensors/GGUF headers and refuses pickle, `.pt`, `.py`, `.so`; the docs are explicit that `STRUCTURE_VERIFIED` "is not a claim that the model is useful or safe" (`researcher-quickstart.md:56-57`). Local generate runs operator binaries with `trust_remote_code=false`. Reasonable, but a safetensors file can still carry a model that is illegal to possess or distribute.

**Reputational.** The product brief's "let people with good GPUs earn BTX" is not something this protocol does. Shipping it would mean promising income the code cannot deliver. The owner's existing credibility with easyNode rests on saying what a machine can honestly do (`README.md:103-107`); an AInode that implied seeding income would contradict that.

**Technical.** Single-maintainer upstream with eight releases in five weeks and a consensus flag day at 244,000 that forced an emergency easyNode engine bump; the model plane is under four weeks on `main`; one open process-wedging bug (#225); PQ TLS on OpenSSL 3.5's brand-new ML-KEM/ML-DSA providers; the helper's only control surface is a unix socket; peer hints from the money network are dropped; docs lag code by several releases. Any app would be chasing a moving, thinly reviewed target.

---

## Things I could not verify

- Per-binary sizes of `btx-modeld` and friends (no download performed).
- Whether a pruned `btxd` wallet can complete `buildhtlcclaim` end to end on BTX mainnet (Bitcoin Core heritage says yes; BTX-specific tests all run unpruned regtest).
- Whether any model-plane peer is reachable on mainnet right now (no census tool, no public endpoint to probe; probing was out of scope for a static review).
- The provenance of the one WAN throughput figure (hard-coded in `e2e-production-evidence.sh`).
- Anything about `btx-qt`'s Models dock behaviour; I read that it exists and ships, not how well it works.

## Sources outside the two repos

- GitHub `btxchain/btx` via `gh`: release assets for v0.34.6/9/12/15; issues #161, #169, #172, #202, #209, #225; PR #156, #223; contributor counts.
- [Homebrew openssl@3 formula API](https://formulae.brew.sh/api/formula/openssl@3.json) (3.6.5).
- [Trendshift listing for btxchain/btx](https://trendshift.io/repositories/38084) (the only web mention of the project found; no model-network content).
