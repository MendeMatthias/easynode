# Serving the Esplora API from an easyNode

Ported from the deployment behind `api.btxscan.io`, verified against its
source, and extended with the parts that deployment kept on one machine.
`PROVENANCE.md` records the source commit and every hand-made difference.
Read [docs/esplora-mode.md](../../docs/esplora-mode.md) first: it carries the
route contract, the acceptance gate, and the finding that decides whether this
is worth doing at all.

## What is here

| file | what it is |
|---|---|
| `electrs/` | the electrs fork that serves Esplora for BTX (vendored, see PROVENANCE.md) |
| `rust-btx/` | the BTX consensus decode crate it links (vendored) |
| `test-vectors/` | real blocks rust-btx's tests decode byte-exactly (vendored) |
| `build-electrs.sh` | builds `electrs` from the tree above and installs it |
| `build-caddy.sh` | builds a Caddy WITH the rate-limit plugin this front needs |
| `Caddyfile.template` | the TLS + CORS + freshness front. Reads `BTX_ESPLORA_HOST` and `BTX_ESPLORA_RUN` |
| `electrs.service.template` | the indexer as a systemd unit. Replace `USER` and the two data paths |
| `btxd.service.template` | btxd as a unit, for a server that does not run the easyNode app |
| `btx-staleness-check.sh` | the freshness guardian: judges the served tip against the chain census and writes the marker the front matches on |
| `btx-staleness.service` / `.timer` | run it every 30 s |
| `healthcheck.sh` | a cron health line: btxd vs the census, electrs liveness and lag, disk |
| `scan-chain.sh` | proves rust-btx decodes every block byte-exactly before electrs indexes |
| `sync-from-btx-esplora.sh` | refreshes the three vendored trees from a checkout |

## Two ways to run it

**With the easyNode app.** Settings → "Serve wallets (Esplora API)". The app
runs the prune gate (`crates/btx-core/src/esplora.rs`) and refuses with the
reason on a pruned datadir; finds `electrs` and `caddy` on PATH, in
`/usr/local/bin` or `~/.local/bin`, and names the build script for a missing
one; starts both beside btxd with the node; writes this directory's Caddyfile
next to the datadir; runs the guardian every 30 s
(`crates/btx-core/src/esplora_freshness.rs`, the same rules as the shell
guardian here); and shows the verdict beside the switch. The front listens on
`http://127.0.0.1:3080` until you give it a hostname in the next Settings row.
Everything lives under `<datadir>/esplora/`: `run/` (the markers),
`electrs-db/`, `Caddyfile`, and the two logs.

**Without the app, on a server.** The units and the timer, the way
api.btxscan.io runs:

1. **Check you can.** `prune=0` is not advice, it is a precondition: electrs
   indexes from btxd's block files and refuses a pruned node outright. A
   pruned datadir needs a resync; there is no shortcut.
2. `build-electrs.sh`, then `electrs.service.template` →
   `/etc/systemd/system/electrs.service` with the paths filled in.
3. `build-caddy.sh`. Stock Caddy refuses this Caddyfile (`rate_limit`).
4. Install `btx-staleness-check.sh` to `/usr/local/bin/`, enable the timer.
   Confirm it writes exactly one of `/run/btx-{fresh,stale,unverified}` and
   says why on stdout (`journalctl -u btx-staleness`).
5. `BTX_ESPLORA_HOST=… caddy run --config Caddyfile.template --adapter caddyfile`,
   or the same through a unit of your own.
6. **Run the gate before telling anyone the endpoint exists:**
   ```bash
   scripts/verify-esplora.sh https://your-host <mainnet-address-with-spend-history> …
   ```

## Four things that will bite

**Freshness is declared, never faked.** Exactly one marker exists at a time
and the proxy matches on its presence; with none at all it answers
`unverified`. `unverified` is not a failure state, it is the honest one. The
predecessor of the guardian treated a missing witness as proof of health and
served a four-day-old chain labelled `local`; that is the single most
expensive line in this directory's history.

**The witness is the census, not an explorer.** `easybtx.com/api/nodes`
publishes which chain carries the most work, measured from every reachable
node's headers. On 2026-09-05 the last explorer sat on a minority branch for
a day; as a witness it would have inverted every verdict. A served tip that
is not on the heaviest measured chain is `unverified` whatever its age.

**Never fail over to another chain.** Serve your own node and label it. An
overstated balance reaching a signing wallet is worse than a stale one, and
both former fallbacks demonstrate it: minebtx answers 503, and byronbay
follows the unattested branch *and* under-reports spends in the range both
chains share.

**CORS exactly once.** electrs emits its own headers and Caddy strips them
downstream before emitting one set. Duplicate `Access-Control-Allow-Origin`
is rejected by browsers outright and broke the web wallet once already.

## Changes made during the port

`PROVENANCE.md` has the table. In one paragraph: the guardian's witness moved
from two dead explorers to the census; the Caddyfile lost a bcrypt credential,
an unrelated site, a home-IP rate-limit exemption, the hostnames and the
request log, gained two environment placeholders, and answers `unverified`
when no marker exists; paths and the service user became placeholders;
electrs and rust-btx were vendored verbatim, minus a 1 MB Bitcoin-mainnet data
file; and the fact that the front needs a Caddy plugin is written down, which
the source never did.
