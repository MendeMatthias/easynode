# Next easyNode session: the handover

State on 2026-10-12, after 0.7.11. Verified against the released bytes, not
just the local build, except where it says otherwise.

---

## Where things stand

**Published: easyNode 0.7.11 on Mac, Linux and Windows** (`node-v0.7.11` on
MendeMatthias/EasyBTX-releases, ten assets, every one re-downloaded identical
and every updater signature verified against the released bytes; repo Latest
still v0.30.0). Engine unchanged: BTX v0.34.15 (`476f3f23`).

**The website PR is merged (EasyBTX #576, `0c3f08b`) but was NOT deployed**:
Vercel made no deployment for that merge commit (GitHub and Vercel both
reported all systems operational), and a manual `vercel deploy --prod` from
the session was blocked by the permission classifier. Until it deploys,
easybtx.com serves the 0.7.10 feed and pages (safe: those assets exist). Check
`curl -s https://easybtx.com/updater/latest-node.json | jq .version` first.

What 0.7.11 did (easynode #187):

- **The built-in start moved from 239,111 to 245,300**
  ([utxo-snapshot-245300](https://github.com/MendeMatthias/EasyBTX-releases/releases/tag/utxo-snapshot-245300)).
  Every check 0.7.6 used, except a load on a fresh mainnet node (none runs on
  the release Mac); instead the UTXO set hash and coin count were recomputed
  from the file by a script that reproduces the 239,111 pair exactly.
- **One network height for every "blocks behind" reader**
  (`btx_core::network_height`, `docs/decisions/2026-10-12-network-height.md`):
  the highest of blocks, the best header, and the signed frontier when it is
  on the node's own chain. After block 244,000 `headers` alone stays near
  `blocks` while a node catches up.

What the website fix did (EasyBTX #575, deployed 21:10 UTC on 10-11): the
snapshot routes take a write's etag from `head()`, not `get()`. Before, every
conditional update of a stored record failed, so no snapshot file was ever
stored and no co-signature could ever merge. Verified live: the 3060's
producer stored block 245,300's file six minutes after the deploy.

## Since 0.7.11 on main, unreleased: the UI audit

An audit of every screen (`docs/node-ui-audit-2026-10-12.md`) and its fixes
(easynode #189, #190). The one that matters: the headline said a green LIVE
over a node following signatures that had been stuck for hours, while the
cards below said it was not following. `apps/node/src/headline.ts` now
derives the badge, the line and the light from what the cards say; the tray
says "not following" too (`tray::stuck_for_tray`). The next release (0.7.12)
ships them. Two items in the audit are decisions, not fixes: the role card's
wording for home users (it tells them to "pin a second key") and grouping
Settings.

To look at any screen without a node: `npm --prefix apps/node run
preview:states`, then `http://localhost:1440/preview/index.html`.

## Open work, in priority order

1. **A second operator must confirm.** `/api/snapshots/latest` stays 404
   until Aleksander (zbtx1 to zbtx3) or jpp co-signs; every statement since
   5 October carries only `02d5efca`. zbtx1 sits on a dead branch from
   244,089 (first block `24528d1139a71aa694e65bf54dc47a3e2652d4716f94fa43c3d1a3738fb16ba7`).
   No co-signature attempt reached the website at all. The overnight report
   (`~/Desktop/easyNode-overnight-report.md`) has the commands for each of
   them.
2. **A validating node on a lost one-block twin still under-counts.** Its
   signed frontier is off its own chain (`on_active_chain` false), so the
   network height leaves it out; on a node that follows signatures `headers`
   carries it anyway. Counting a frontier whose branch leaves ours within 6
   blocks needs the fork point. See the decision's "What this does not do".
3. **Only the newest statement gets a file.** The producer uploads its offered
   pair; older statements stay `missing`. Fine for `latest`, which wants the
   newest, but a confirmer's co-signature on an older one never confirms.

## Release facts that cost time to rediscover

- The recipe is docs/node-release-recipe.md; memory notes in
  `~/.claude/projects/-Users-bonuz-repos-easynode/memory/` say which steps the
  classifier has blocked and which ran.
- `apps/node/src-tauri/resources/node-pkg` is git-ignored and can be stale in
  a fresh worktree (it was v0.34.6). Restage it from the previous release's
  Mac bundle and diff it byte for byte (btxd `a315d86e…` for v0.34.15).
- No `minisign` on the release Mac. `build-node-feed.sh` verifies with
  `verify-updater-sig.py`; for the "verify like the app does" step a tiny Rust
  bin on the cached `minisign-verify = "=0.2.5"` crate works offline.
- easybtx.com's request logs are readable with `npx vercel@59.26.0 logs
  --project easy-btx --scope mendematthias-projects --since 24h --query <path>
  --json` (about 50 rows a query, about a day of retention). That is how the
  snapshot 503s were found.
- **A test install runs a mainnet node on the release Mac**
  (`~/btx-test/app2`, btxd on `~/.easybtx`), stalled on a stale block at
  242,639 on 10-11. Not started by the release work; the owner decides.
- No node runs on the release Mac by the owner's decision, so the feed and
  publish scripts need `OBSERVER_OVERRIDE=1` after proving "no split, live
  chain" from easybtx.com/api/nodes, both pools, witness-1 and api.btxscan.io.
- Never re-cut a published version.
