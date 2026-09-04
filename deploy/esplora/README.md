# Serving the Esplora API from an easyNode

Ported from the deployment behind `api.btxscan.io`. Read
[docs/esplora-mode.md](../../docs/esplora-mode.md) first — it carries the route
contract, the acceptance gate, and one finding that decides whether this is
worth doing at all.

## What is here

| file | what it is |
|---|---|
| `Caddyfile.template` | the TLS + CORS + freshness front. Set `BTX_ESPLORA_HOST`. |
| `electrs.service.template` | the indexer. Replace `USER` and the two data paths. |
| `btx-staleness-check.sh` | the guardian that writes the freshness markers |
| `btx-staleness.service` / `.timer` | run it every 30s |

## What is NOT here, and where to get it

**electrs itself, and the BTX consensus decode crate.** They live in the
`btx-esplora` repository — a fork of Blockstream's electrs `new-index` adapted
for BTX, plus a companion of rust-bitcoin 0.32.x that decodes P2MR outputs
(witness-v2 programs that upstream rust-bitcoin reports as "unknown"). Build
electrs from there and install it at `/usr/local/bin/electrs`.

Those are not vendored here on purpose: they are a fork of somebody else's
project with their own history, and copying a snapshot into this repository
would strand it.

## Order of operations

1. **Check you can.** `prune=0` is not advice, it is a precondition — electrs
   indexes from btxd's block files and a pruned datadir deletes them. easyNode
   refuses the mode outright rather than letting the index fail hours in.
2. Build and install `electrs`, then `electrs.service.template` → `electrs.service`.
3. Install `btx-staleness-check.sh` to `/usr/local/bin/`, enable the timer.
   Confirm it writes exactly one of `/run/btx-{fresh,stale,unverified}`.
4. `BTX_ESPLORA_HOST=… caddy run` with the template.
5. **Run the gate before telling anyone the endpoint exists:**
   ```bash
   scripts/verify-esplora.sh https://your-host <mainnet-address-with-spend-history> …
   ```

## Three things that will bite

**Freshness is declared, never faked.** Exactly one marker exists at a time and
the proxy matches on its presence. `unverified` means no witness was reachable —
it is not a failure state, it is the honest one. The predecessor of this script
treated a missing witness as proof of health and served a four-day-old chain
labelled `local`; that is the single most expensive line in this directory's
history.

**Never fail over to another chain.** Serve your own node and label it. An
overstated balance reaching a signing wallet is worse than a stale one, and both
former fallbacks demonstrate it: minebtx answers 503, and byronbay follows the
unattested branch *and* under-reports spends in the range both chains share.

**CORS exactly once.** electrs emits its own headers and Caddy strips them
downstream before emitting one set. Duplicate `Access-Control-Allow-Origin` is
rejected by browsers outright and broke the web wallet once already.

## Changes made during the port

- The witness list in the guardian was **repointed**. It defaulted to byronbay
  and minebtx; both are now gone, so those defaults would have recreated the
  exact bug the script's own header documents. It now defaults to
  `api.btxscan.io` and says why.
- Removed from the Caddyfile: a `basic_auth` block carrying a bcrypt credential,
  a second site for an unrelated internal service, a rate-limit exemption for one
  fixed home IP that the original marked "REMOVE THIS AFTER THE EVENT", and the
  two concrete hostnames.
- Request logging was left out rather than copied. The original logs only
  wallet-User-Agent requests and strips `remote_ip`, `remote_port` and `Cookie` —
  anonymous by construction. Copy it if you want metrics, and copy it *exactly*,
  because the filter is what makes it anonymous.
- Paths and the service user are placeholders instead of one host's Azure layout.
