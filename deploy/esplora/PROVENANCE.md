# Where deploy/esplora came from

Everything under this directory is a **port** of the deployment behind
`api.btxscan.io`, whose source lives in the private repository
`MendeMatthias/btx-esplora`. This file records what was copied verbatim, what
was ported by hand, and what was left behind and why, so that the next person
can tell a deliberate difference from drift.

Vendored commit: c77fa4011863a2cd8a8d8c9cee45e910d25fa0c8
Vendored on: 2026-09-06, via `git archive` (bytes from the repository objects, not a checkout)
Upstream base of the electrs fork: Blockstream/electrs `new-index` at `ee7add1259b42133e0300926993794faf7660585` (`electrs/FORK_BASE_COMMIT.txt`)

## Copied verbatim

| tree | what it is | changes made here |
|---|---|---|
| `electrs/` | the electrs fork that serves the Esplora REST API for BTX | none. One file excluded: `contrib/popular-scripts.txt`, 1 MB of Bitcoin-mainnet script hashes for upstream's precache tool, which mean nothing on BTX. `src/bin/popular-scripts.rs` reads such a file at runtime and still compiles. |
| `rust-btx/` | the BTX consensus decode crate (182-byte header, trailing matmul payloads, shielded bundles, the BTX txid rule) that electrs links in place of rust-bitcoin's block types | none |
| `test-vectors/` | real mainnet blocks and an address oracle that rust-btx's tests decode byte-exactly | none |

`electrs/Cargo.toml` refers to `../rust-btx` by path; the layout is preserved
so that works unchanged. Neither crate is in a cargo workspace here (this
repository deliberately has none) and neither is covered by the repository's
`cargo fmt` gate; `.github/workflows/esplora.yml` tests rust-btx and checks
electrs whenever these trees change.

Why vendor rather than link: the source repository is private, so a submodule
or a build-time clone would work for exactly one person. A copy with a
recorded commit works for everyone and is refreshed with
`sync-from-btx-esplora.sh`.

Before copying, the trees were scanned for credentials, keys and addresses.
Nothing was found beyond upstream's author line in `electrs/Cargo.toml`.

## Ported by hand, from `deploy/` in the source repository

| there | here | what changed, and why |
|---|---|---|
| `Caddyfile` | `Caddyfile.template` | Verified against the source on 2026-09-06 by diffing the `esplora_api` snippet with comments stripped: 92 directives there, 89 here, and the only difference is the removed rate-limit exemption for one home IP that the original marked "REMOVE THIS AFTER THE EVENT". Also removed: the `basic_auth` block (it carries a bcrypt credential, which has no business in a public repository even hashed), the second site for an unrelated internal service, the two concrete hostnames, and the anonymised request log. Added: `{$BTX_ESPLORA_HOST}` for the site address and `{$BTX_ESPLORA_RUN:/run}` for the marker directory. **One deliberate behaviour change:** with no marker file present the front answers `X-Btx-Freshness: unverified`, where the original answered `fresh`. The original could afford that default because its guardian always writes a marker; here the front is also started by the app, and a front that says `fresh` before anything has checked is the exact failure the guardian exists to prevent. |
| `electrs.service` | `electrs.service.template` | paths and the service user are placeholders |
| `btx-staleness-check.sh` | `btx-staleness-check.sh` | The witness is now the chain census at `easybtx.com/api/nodes` (which chain carries the most work, measured from every reachable node's headers), never an explorer. The original's witnesses were `esplora.btxbyronbay.com` and `explorer.minebtx.com`; both are gone, and on 2026-09-05 `api.btxscan.io` itself sat on a minority branch for a day, so an explorer as the reference would have called a correct node stale and a wrong one fresh. The rules are in the script header and are implemented identically in `crates/btx-core/src/esplora_freshness.rs`, which the app uses. They also distinguish a deep branch from a mining race, which the original had no reason to: measured 2026-09-06, the census's heaviest tip was a one-block orphan and a first version of these rules called a correct endpoint "on another chain" for not holding it. |
| `btx-staleness.service`, `.timer` | same names | the description only |
| `btxd.service` | `btxd.service.template` | placeholders; for a server that serves Esplora without the app |
| `healthcheck.sh` | `healthcheck.sh` | the network-tip reference is the census, for the reason above; the rest as it was |
| `scan-chain.sh` | `scan-chain.sh` | paths come from the environment instead of the Azure host's layout |
| `deploy-to-vm.sh` | `build-electrs.sh` | the build half; the rsync-to-Azure half was not ported |
| `verify-esplora.sh` | not ported | superseded by `scripts/verify-esplora.sh`, which compares UTXO sets rather than fields and separates the routes the wallet needs today from the witness routes |
| `README.md` | not ported | the runbook of one Azure VM: subscription ids, disk names, an admin IP. `README.md` here is the port's own |
| `docs/BTX-Esplora-API-Contract.md` | `docs/esplora-api-contract.md` | verbatim, with a provenance header |
| `docs/EPOCH-A-*.md`, `docs/ESPLORA-INDEX-BUG-REPORT-*.md` | not ported | incident records of that host; what a port needs from them is in `docs/esplora-mode.md` |

## One thing this port writes down that the source never did

The `Caddyfile` uses `rate_limit`. That directive is **not in stock Caddy**: it
is the `github.com/mholt/caddy-ratelimit` plugin, and the source repository's
runbook mentions it once, under "Hardening (2026-07-17)". A stock `caddy`
refuses the whole configuration at `caddy validate` with
`unrecognized directive: rate_limit`. `build-caddy.sh` builds the right binary
and refuses to install one whose module list lacks `http.handlers.rate_limit`.

## Refreshing the copy

    deploy/esplora/sync-from-btx-esplora.sh /path/to/btx-esplora

copies the three trees again from that checkout's HEAD with the same exclusion
and rewrites the "Vendored commit" line above. Review the diff. The
hand-ported files are not touched.
