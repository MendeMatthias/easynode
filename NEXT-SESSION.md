# Next easyNode session: the handover

State on 2026-10-14, after 0.7.14 (Linux and Windows). Verified against the
released bytes, not
just the local build, except where it says otherwise.

---

## Where things stand (2026-10-14)

**Published: easyNode 0.7.14 for Linux and Windows** (`node-v0.7.14`, built
by CI from `6ac2e60`, tree identical to main's `f637808`, #209), the audit
fixes of #208 on top of 0.7.13. Same checks as 0.7.13 below: draft first,
every asset re-downloaded identical, signatures verified against the released
bytes, Latest still v0.30.0. The Mac is next, below.

## Tomorrow on the Mac: add the Mac build to node-v0.7.14

The Linux and Windows assets of `node-v0.7.14` are published and signed. Do
NOT rebuild or re-sign them; the Mac build goes into the SAME release, the way
0.7.8 to 0.7.12 did. Until it ships, every Mac below 0.7.12 is offered nothing
(the feed names 0.7.14 with no Mac key), which is why this is first.

1. `git pull` on main (`f637808` or later; `tauri.conf.json` says 0.7.14).
2. Prove the staged `apps/node/src-tauri/resources/node-pkg` is the shipped
   engine: byte for byte against 0.7.12's Mac bundle (btxd `a315d86e…`), as in
   recipe step 4. Never run `stage-node-pkg.sh`.
3. Build: `cd apps/node && TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/easybtx.key)"
   TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" npx tauri build --bundles app,updater`,
   then the `.dmg` with `hdiutil` (Applications symlink plus the `.app`, UDZO).
4. Rename the tarball to `BTX-Node_0.7.14_aarch64.app.tar.gz`, sign THAT name
   (`npx tauri signer sign -f ~/.tauri/easybtx.key -p "" <file>`), and verify it
   with `apps/node/scripts/verify-updater-sig.py`.
5. Upload the tarball, its `.sig` and `BTX-Node_0.7.14_aarch64.dmg` to
   `node-v0.7.14`. Replace `SHA256SUMS` with five lines: the three published
   lines unchanged, plus the tarball and the `.dmg`, as 0.7.12's had. Keep
   `make_latest` `"false"`, and check `/releases/latest` is still v0.30.0.
6. Feed with all three platforms WITHOUT re-signing Linux and Windows
   (`build-node-feed.sh` re-signs every file it is given). Download the
   published `.sig` files of the AppImage, the `.deb` and the setup `.exe`,
   then `python3 apps/node/scripts/gen-node-feed.py --version 0.7.14 --tag
   node-v0.7.14 --mac-sig <tgz>.sig --linux-sig <AppImage>.sig --deb-sig
   <deb>.sig --win-sig <exe>.sig --notes "easyNode 0.7.14. See
   https://easybtx.com/node/changelog" --pub-date <now, UTC> --out
   latest-node.json`.
7. Site PR on EasyBTX: copy both feeds into `site/public/updater/`; move
   `REL_MAC`, `relPageMac` and `dl.mac` in `site/src/pages/node.astro` to
   `node-v0.7.14`; rewrite every "the Mac stays on 0.7.12" sentence on `/node`
   and `/node/how-to` (grep `0.7.12`); change the 0.7.14 changelog heading's
   platform tag to `mac + linux + windows` in easynode and re-sync
   `apps/node/CHANGELOG.md`. Run `scripts/check-download-links.sh`. Merge only
   once the Mac assets answer 200.
8. Edit the `node-v0.7.14` release notes: drop the sentence about the Mac
   staying on 0.7.12.
9. Watch a Mac on 0.7.12 or older update itself, and one below 0.7.8.

## 0.7.13, the release 0.7.14 hardens

**Published: easyNode 0.7.13 for Linux and Windows** (`node-v0.7.13`, seven
assets re-downloaded identical, every updater signature verified against the
released bytes, Latest still v0.30.0, release notes say the Mac stays on
0.7.12; site PR EasyBTX #585 merged as `d8f08bec` and deployed, the live feed
reads 0.7.13 with `linux-x86_64` and `windows-x86_64` only). Built by CI from
`3abafff`, whose tree is identical to main's `26eb878` (#206). Engine
unchanged: BTX v0.34.15. The 3060 updated itself to it through the website at
22:02 UTC on 10-13 (AppImage `12eff640…`), btxd running throughout.

What it carries (easynode #199, #205, #206): a node that loses its GPU block
verifier mid-run gets it back by itself. Inside WSL it restarts WSL (an
engine restart does not help there, measured with a deliberate NVIDIA install
on 10-13); elsewhere it restarts the engine. The cause of the 10-11 freeze was
Windows Update installing a driver under the running engine
(`docs/incident-2026-10-11-3060-verifier-quarantine.md`).

**The audit fixes** (#208, shipped in 0.7.14). Two read-only
audits of #204 to #206 found a WSL restart that ignored a node owned by
another app, a 3-hour gap lost across the restart it causes (a possible
restart every ~20 min), no safety net when Windows failed to restart WSL, the
observer's lost exec bit, and wording. All fixed, and shipped as 0.7.14.

**The 3060's own setup changed** (not in the repo): a boot task starts WSL
with nobody logged in; `.wslconfig` has `instanceIdleTimeout=-1`; zbtx3's key
`02c9cfb7` is pinned in `btx_rw.conf`; the observer runs #204 from this clone.
The AInode project's WSL distros run in the same VM, and `AInode-build` boots
`systemd-binfmt`, which wipes the WSLInterop handler for every distro: the
recovery's `/init` fallback exists for that.

## Before that: 0.7.12 (2026-10-12)

**Published: easyNode 0.7.12 on Mac, Linux and Windows** (`node-v0.7.12`,
ten assets re-downloaded identical, every updater signature verified
against the released bytes, Latest still v0.30.0; site PR EasyBTX #577
merged as `0e3d1221`). Engine unchanged: BTX v0.34.15, the Mac node package
byte-identical to 0.7.11's (btxd `a315d86e`). What it carries (easynode
#189 to #197):

- **Every feature on by default, running only where it can, all
  switchable** (the owner's rule of 2026-10-12): wallet panel, Explorer mode,
  snapshot serving, Esplora and launch at login joined the services already
  on. Existing installs get them once (`migrate_features_on`) and the
  welcome panel lists them. Explorer mode is applied only where no pruning is
  in effect (`btx_core::setup::explorer_mode_possible_here`), which also
  fixed a Keeper that turned it on from Ask and then could not start (BTX
  refuses pruning plus txindex, `src/init.cpp:2077`). The whole chain held
  349,792 transactions (07:52 UTC), so the index is tens of MB. Snapshot
  loads and Fast-forward pass on regtest with `-txindex=1` on the shipped
  engine.
- **The headline follows the chain cards** (`apps/node/src/headline.ts`):
  NOT FOLLOWING / BEHIND / DIFFERENT BRANCH / DISK ALMOST FULL instead of a
  green LIVE over a stuck node; the tray (`tray::stuck_for_tray`) and the
  compact light follow.
- Settings in four groups, Escape and focus for panels, the close dialog no
  longer opens behind a panel, one status poll at a time, and the rest of
  `docs/node-ui-audit-2026-10-12.md`.

**Before that: easyNode 0.7.11 on Mac, Linux and Windows** (`node-v0.7.11` on
MendeMatthias/EasyBTX-releases, ten assets, every one re-downloaded identical
and every updater signature verified against the released bytes; repo Latest
still v0.30.0). Engine unchanged: BTX v0.34.15 (`476f3f23`).

**The website serves 0.7.11** (EasyBTX #576, `0c3f08b`, deployed by the owner
at about 07:36 UTC on 10-12; Vercel had made no deployment for the merge on
its own). Checked from the live feed the way the app updates: all four
updates (Mac, AppImage, .deb, Windows) download, verify against the app's
updater key, and match the release's SHA256SUMS; the /node page's four
download links answer 200.

**Followers are stuck, and that is a signer problem, not a code one**
(read-only on btxscan's node, 07:35 UTC, tip 245,972):

- `02d5efca` (the 3060) has signed nothing since 245,498. It stays connected
  to btxscan with a starting height of 245,498.
- `02c9cfb7` (zbtx3, pinned since 0.7.5) signed 245,500 to 245,764 and then
  stopped (block 245,764 is from 02:12 UTC), so followers moved up overnight
  and stand at 245,764 again.
- Only `037db271` signs the tip. It is deliberately not pinned (owner
  unrecorded, see the pin list's doc and
  `the_pin_carries_the_three_zbtx_keys_and_none_of_the_unrecorded_ones`).

To see who signs a block: `getmatmulattestations <hash>` returns raw hex;
the signer is the 33-byte key after `21`
(`grep -o '210[23][0-9a-f]\{64\}'`).

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
(easynode #189, #190, #191). The one that matters: the headline said a green LIVE
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

00. **The Mac build of 0.7.14**: "Tomorrow on the Mac" above.
01. **An end-to-end run of the WSL recovery in the real app.** The pieces are
    proven one by one, but the app has not yet restarted WSL by itself after
    a real GPU loss. It needs an admin `pnputil /restart-device` on the 3060
    while 0.7.14 runs and has verified blocks; a GPU-memory hog does NOT
    cause the loss (WDDM pages GPU memory out, measured 2026-10-14).

0. **Upstream btxchain/btx#225 would end co-signing as the confirmer does
   it.** Open, not merged, "a later 0.34.16 can carry it" (its author,
   2026-10-07). Its `af9020cd` makes `signutxosnapshotmanifest` rebuild the
   statement from the signing node's own active tip and refuse unless every
   field matches. The confirmer (`snapshot_confirmer.rs`) co-signs a
   statement 144 blocks deep that it checked against its diary, relying on
   the RPC signing blindly (its header says so), so under #225 every
   co-signature and every dissent is refused. `confirmed_snapshot_regtest`
   already catches this: its confirmer co-signs block 100 from a node with
   no blocks. Run it against any new engine before bumping.
   The way that still works under #225: two operators agree by each
   PRODUCING the statement at the same grid height (the snapshot keeper
   exports at every multiple of 100 when its tip reaches it, and the website
   merges identical statements by hash). Measured on regtest with the shipped
   v0.34.15, 2026-10-12: two validating nodes on one chain exported
   byte-identical files (8,055 bytes) and identical statements at block 100.
   Since 0.7.12 snapshot serving is on by default on every node that
   validates and signs, so an operator-listed easyNode (zbtx's) now produces
   too. Check that the website's merge and the producer's upload really do
   this with two producers before relying on it, and redesign the confirmer
   before taking an engine with #225.

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
