# BTX AI Node — feasibility research

> Research prompt. Run in a fresh session with Fable at max effort.
> Written 11 Oct 2026.

You are researching whether to build a new desktop app, working name AInode, on BTX's Native Model Network. The user needs a hard-nosed assessment, not encouragement. A well-evidenced "this does not work yet" is a successful outcome and is more valuable than optimism.

## Repos (READ-ONLY — do not modify anything in either repo except the single output document described at the end)

- `/Users/m2promende/repos/btx` — the BTX node (upstream btxchain/btx). Currently on `main` = commit 476f3f23 = tag v0.34.15. Model plane docs are in `doc/modelnet/`, plus `doc/pay-with-compute.md`, `doc/bounties.md`, `doc/compute-rpc.md`. Do not commit anything here.
- `/Users/m2promende/repos/easynode` — the user's existing Tauri desktop app that installs and supervises btxd. Shipping 0.7.8.

## The idea

A desktop app that makes open AI models easy to get: paste a `btx://` link or search, the app downloads, verifies every piece by hash, and seeds it back. No HuggingFace account, no login, no trusting a download link. Later: run models locally, and let people with good GPUs earn BTX.

It would run `btx-modeld` standalone. Per `doc/modelnet/researcher-quickstart.md` that path needs "no wallet, no coins, no mining, and no chain sync", so the app would NOT require syncing the blockchain. Verify that claim.

## Findings so far — verify these, do not trust them

1. `btx-modeld` runs standalone without `btxd`. (`doc/modelnet/researcher-quickstart.md`)
2. easyNode deliberately builds the engine with `-DWITH_MODELNET=OFF` and strips 7 helper binaries from upstream archives. (easynode `README.md`, `apps/node/scripts/stage-node-pkg.sh`, `apps/node/src-tauri/src/commands.rs` around line 281)
3. Hosting/seeding earns NO BTX. `doc/modelnet/economics.md` lists "Protocol emissions or token rewards for hosting or advertised capacity" under what does not exist.
4. Pay With Compute credits are "not transferable, not cash-redeemable, no cross-agreement credit, and no carryover". They buy access to one resource, not BTX. (`doc/pay-with-compute.md`)
5. Real BTX does flow for: sponsored mirroring of scarce content, bounty awards, and release campaigns settled with SHA-256 HTLCs.
6. Hosting requires OpenSSL 3.5+ with MLKEM768 and mldsa44; OpenSSL 3.0 cannot host.

## Priority question

**Can a user get PAID without running a full node?** `btx-modeld` "never holds wallet keys" (`doc/modelnet/model-economy.md`), and HTLC claim/refund use `buildhtlcclaim` / `buildhtlcrefund`, which look like wallet RPCs on `btxd`. If earning requires a synced chain and a wallet, say so plainly and describe exactly what the app would need. Check whether any lighter path exists (watch-only, external signing, an address someone else pays out to, anything). This single answer decides whether the app is light for everyone, or light only for consumers and heavy for earners.

## Other questions

**A. IS IT REAL, OR ONLY SPECIFIED?** Do not re-establish that code exists — it does, in volume: `src/modelnet/` has 225 source files (4.5 MB), `test/functional/` has 27 `feature_modelnet_*` / `feature_pay_with_compute*` / `wallet_modelnet_funding` tests, and `contrib/modelnet/` holds roughly 90 `e2e-*` scripts. The real question is narrower: **what has actually been executed, and under what conditions?**

- Most of those e2e scripts look like regtest or isolated-lab harnesses. Determine which, if any, exercise a real network.
- `doc/modelnet/cuda-not-run.md` and `audit/hcp-not-run.md` sound alarming but may not be. Establish precisely what each one gates. (`cuda-not-run.md` appears to describe a default-off runtime check whose status code is literally `NOT_RUN_CUDA_ISOLATION`, not an unbuilt feature. Confirm.)
- Work out what the acceptance matrices actually assert and which rows are unmet: `audit/acceptance-matrix-0.34.8.csv`, `audit/acceptance-matrix-notes.md`, `audit/hcp-native-acceptance.csv`, `audit/cr11-native-acceptance.csv`, `contrib/modelnet/bounty/tests/acceptance-matrix.csv`, `contrib/modelnet/crl12/tests/native-acceptance-v1.2.csv`.
- `contrib/modelnet/evidence/` contains only two JSON files. Decide whether that is meaningful.

For each capability the app would depend on (import/host, retrieve, search, seeding, economy RPCs), classify it: shipped and tested on a real network / tested only in regtest / implemented but untested / specified only.

**B. IS ANYONE USING IT?** This is the biggest commercial risk and deserves the most effort. Is there any evidence of real mainnet model-plane activity: models hosted, campaigns funded, bounties awarded, peers seeding, any `btx://` link published anywhere? Look outside the repo too — GitHub issues and releases, the BTX site, block explorers, social channels. If the network is empty, a beautiful client has nothing to show on first launch, and the whole product depends on solving a cold-start problem. Say so plainly if you cannot find evidence either way, and if it is empty, estimate what seeding the network with worthwhile content would take.

**C. DOES THE ECONOMIC LOOP CLOSE?** Walk through each paying path end to end and name who pays, who receives, what triggers it, and what could realistically be earned. Is sponsored mirroring something a home machine on a domestic connection can win against a datacenter? Be concrete.

**D. PACKAGING.** What exactly ships: which binaries, what size, what dependencies per platform (macOS arm64, Linux x86-64, Windows). How hard is bundling OpenSSL 3.5 on each. What ports open and what that means for a home user behind NAT. Check whether Windows is even supported for the model plane.

**E. STORAGE CONSENT.** A packaged install defaults to automatic bounded storage (`-modelstorage=auto`). Find the actual default size and what a user must be told before it starts using their disk.

**F. COMPETITION.** Honestly compare against how people get models today: HuggingFace, Ollama, direct torrents, IPFS/Filecoin. What is genuinely better here, and what is worse? If the answer is "nothing is better", say that.

**G. RISKS.** Legal, reputational, and technical. Include what happens if the app seeds a model that turns out to be illegal or malicious where the user lives, and whether the design gives the user any control over that.

## Evidence standard

Cite file paths and line numbers for repo claims, URLs for external ones. Distinguish clearly between: shipped and tested / implemented but unproven / specified only / your own inference. Never present a doc's intention as a working feature. Flag anything you could not verify. Do not build or execute anything from the repos; reading and static inspection only.

## Output

A single markdown document with a one-page verdict at the top: **BUILD**, **BUILD BUT NARROWER**, or **DO NOT BUILD YET**, with the three strongest reasons and the single biggest unknown. Then the detail, organised by the questions above.

Write it to `docs/ainode-feasibility-research.md` in the easynode repo, and also copy it to `~/Desktop/AInode-feasibility-research.md` (the user looks on the Desktop; this copy is required, not optional).

Do not commit. Report back a concise summary of the verdict and the biggest unknown.
