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
| `install-systemd.sh` | installs the units, `--mode witness` (default) or `--mode esplora` |
| `btx-witness.service.template` | the witness server as a unit; runs on any node, pruned or not |
| `test-witness.sh` | proves the node here can serve as a fork witness |
| `test-front.sh` | starts the real Caddyfile against stubs and checks every claim in it |
| `test-guardian.sh` | runs the freshness guardian against stubs and pins that it agrees with the Rust |
| `test-stack.sh` | the whole stack against a REAL btxd on a throwaway regtest chain |
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

**Without the app, on a server.** Two tiers, and most operators want the
smaller one.

A **witness** serves the two routes a wallet needs to settle a fork. It runs on
any node, including a pruned one, because pruning discards block data and not
the block index. It is the tier the network is short of:

```bash
(cd crates/btx-core && cargo build --release --bin btx-witness)
sudo install -m755 crates/btx-core/target/release/btx-witness /usr/local/bin/
deploy/esplora/build-caddy.sh          # stock Caddy refuses this Caddyfile
deploy/esplora/install-systemd.sh --host esplora-1.example.com          # prints the plan
deploy/esplora/install-systemd.sh --host esplora-1.example.com --yes    # does it
```

**Esplora** serves the whole API, balances included, and needs `prune=0`, the
full ~124 GiB chain and an index on top:

```bash
deploy/esplora/build-electrs.sh
deploy/esplora/install-systemd.sh --mode esplora --host esplora-2.example.com --yes
```

`install-systemd.sh` changes nothing without `--yes`, and before it changes
anything it asks **btxd itself** whether the datadir is pruned. It does not
read the conf for that: a datadir's own `btx_rw.conf` outranks the conf file,
and a node on this project ran pruned for weeks against a conf that said
`prune=0`. It also refuses when either binary is missing, or when the caddy on
PATH has no `rate_limit` module.

Then, in order:

1. `journalctl -fu electrs` — the first index takes hours on a full chain.
2. `curl -sI https://your-host/blocks/tip/height | grep -i x-btx-freshness`.
   It says `unverified` until the guardian has judged it against the census,
   which is the honest state on the way up.
3. **Run the gate before telling anyone the endpoint exists**, against a
   reference that is *not* this host — the gate refuses a self-comparison,
   because comparing an endpoint with itself agrees every time:
   ```bash
   scripts/verify-esplora.sh https://your-host <mainnet-address-with-spend-history> …
   ```

Both moving parts have their own tests, which need no node and no network:

```bash
deploy/esplora/test-front.sh      # the Caddyfile, started, against stubs
deploy/esplora/test-guardian.sh   # the freshness rules, executed
```

And one that needs only the node you already have, no chain rebuild and no
network — it runs the witness server against it and checks every answer against
that node's own `getblockhash`, including at heights below its prune height:

```bash
deploy/esplora/test-witness.sh
```

And one that needs a btxd binary but no chain, no GPU and no network — it mines
its own regtest chain in a temporary directory and throws it away:

```bash
deploy/esplora/test-stack.sh      # btxd -> electrs -> the front, end to end
```

That is the one that proves electrs actually works for BTX: it checks that the
hash electrs serves at a height is the hash btxd reports there, which is
`rust-btx` decoding real 182-byte headers with their trailing MatMul payloads
and agreeing with the node that made them.

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
