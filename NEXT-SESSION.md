# Next easyNode session — paste this as the first message

Everything below is done and live. This is the handoff.

---

## Paste this to start

> Pick up easyNode. 0.6.21 is live on Mac, Linux and Windows and the network is
> not split. Two things are queued and one is a real piece of work:
>
> 1. **Merge PR #81** (README status note comes down — #35 is closed). I have to
>    tick the bypass box myself; GitHub will not let me approve my own PR.
> 2. **0.6.22 whenever convenient.** Two fixes sit unreleased on main: #79 (a port
>    collision is no longer reported as an unrecognised error) and #80 (the
>    watchdog's frozen-node recovery was dialling three peers that cannot serve a
>    block). Neither is urgent. Follow docs/node-release-recipe.md; I must run
>    the signing and feed steps myself because your sandbox blocks the key.
> 3. **The service tier — this is the actual next feature.** Start by showing me a
>    table mapping every capability the app already measures (GPU qualification,
>    disk, prune posture, uptime, reachability) against every job the network
>    needs (full validator, fork witness, archive, Esplora, seed). Do not write
>    code until I have seen that table. The point: a machine with no GPU is NOT
>    useless, and the app currently implies it is.

---

## Context the next session needs

**The vision, in Mende's words:** *different computer hardware = different
services for the network.* One install, and the machine contributes whatever it
is actually capable of.

**Why the service tier matters.** A GPU-less machine stalls below the fork
height — that needs a golden manifest row from upstream and is not fixable here.
But it can still witness forks for wallets (0.6.20, works on any node including
a pruned 10 GB one), serve history, and seed fresh nodes. The app never tells
anyone that. `service_report.rs` already calls itself "the opt-in seed of the
Keepers idea". The pieces exist; the join does not.

**Open, not ours:** btxchain/btx#142 — the engine refuses a live-chain node as a
body source and then bans it for asking about its own chain. Root cause of the
5 September fork. Tracked in #38.

**Unanswered product question:** 0.6.21 shipped a native Windows build, which
reverses the site's stated *"we are not building a newer native Windows app"*.
Copy was rewritten honestly (WSL2 is the route measured against a GPU; the
native build is not). Pulling `windows-x86_64` from a future feed is small if
Mende wants it out.

**Two working habits that paid off, worth keeping:**
- Verify claims against the engine source at `~/btx-ship` (v0.34.6 @ 9eb4e005)
  before acting on them. An audit finding this month said to use
  `-matmulvalidation=relay`; the source says relay refuses to start with a
  wallet present. It would have shipped broken.
- Measure peers from a *running* node, and read the census **late**. A reading
  taken two minutes in showed a good peer as dead and nearly retired it; four
  minutes later it was a fully handshaked attestation archive.

## Release facts that cost time to rediscover

- Version lives in FIVE files: package.json, package-lock.json (×2),
  tauri.conf.json, Cargo.toml, Cargo.lock. Plus CHANGELOG.
- Rename the mac tarball to `BTX-Node_<ver>_aarch64.app.tar.gz` before the feed
  sees it, and rename its `.sig` too (signature is over content, stays valid).
- Build the `.dmg` by hand with `hdiutil` — the recipe's build command skips it,
  but the website's Mac button points at it.
- Append the mac sums to `SHA256SUMS` manually; there is no mac node CI.
- Publish the release BEFORE the site feed, never the reverse.
- The site renders `apps/node/CHANGELOG.md` from **the EasyBTX repo's own copy**.
  They drift. Sync them.
- `MendeMatthias/EasyBTX` has no review requirement — those PRs merge from the
  CLI. `MendeMatthias/easynode` does, and Mende must use the UI bypass.
