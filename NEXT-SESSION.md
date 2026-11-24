# Next easyNode session: the handover

State on 2026-10-14, after 0.7.15 (Linux and Windows). Verified against the
released bytes, not
just the local build, except where it says otherwise.

---

## Update 2026-11-25: the engine moves to BTX 0.34.16 (in pull requests, nothing released)

**The plan** (owner's planning session, 2026-11-25): move the engine from
v0.34.15 to upstream's final **v0.34.16 =
`074dac167c26c1e091198f691ed5727817dba88a`**, pinned by SHA because upstream
moved that tag once (from `d27530af`). Not the 0.34.17 prerelease. Step one is
parity: the same roles and behaviour on the newer engine, the model network
still compiled out (`-DWITH_MODELNET=OFF`), SwapNet off at run time. The
swarm and ModelNet are a later release. Three threads work in parallel and
each ends in an open pull request: the Linux engine build, the Windows engine
build, and the app side. Nobody merges; the planning session integrates and
releases.

**The app side is the draft pull request from `claude/engine-0.34.16-app`.**
It moves `NODE_RELEASE_TAG` and `NODE_RELEASE_COMMIT`, passes `-swapnet=0` to
0.34.16 and newer, adds the tests that hold both, and brings
`docs/btx-update-protocol.md` (v3), the recipe and
`scripts/btx-pin-census.py` up to date. It does not bump the app version and
touches no workflow and no staging script. `commands.rs` has everything that
was read at the SHA beside the pin.

**What is still owed before a release** (the pull request's description has
the same list as checkboxes):

1. The two engine pull requests: `PROVEN_BTXD_RUN_ID`, `PROVEN_BTXD_ARTIFACT`
   and `PROVEN_BTXD_COMMIT` in `node-linux-installer.yml`; `PROVEN_BTXD_RUN_ID`,
   `PROVEN_BTXD_ARTIFACT`, `PROVEN_BTXD_VERSION` and `PROVEN_BTXD_TAG` in
   `node-win-installer.yml`; the `btxchain_ref` default in `btxd-windows.yml`
   (two places); `VERSION` and the archive hashes in `stage-node-pkg.sh` and
   `stage-node-pkg-linux.sh`. Until they land, the installer workflows'
   identity checks are red on purpose: the app asks for `v0.34.16` and the
   proven engines are 0.34.15.
2. **The Mac.** Upstream publishes no macOS archive for 0.34.16, so
   `stage-node-pkg.sh` cannot stage this engine. The Mac needs a source build
   at the SHA (`btxd-macos.yml`, `stage-node-pkg-mac-source.sh`) before it can
   ship this release; until then it stays where it is.
3. **The engine build needs Python 3.11 or newer** with development files,
   `cryptography` and `sqlite3` (`cmake/SwapNet.cmake`), on every builder.
4. **The pinned start pair** (`attested_snapshot::pinned_pair()`) is still
   245,300, about 42,000 blocks back. Refreshing it needs a newer pair
   PUBLISHED as `utxo-snapshot-<height>` on EasyBTX-releases first; the newest
   one there is 245,300 itself. The website's confirmed pointer serves 287,000
   (Mende and Jarek, 2026-11-24), which is what new nodes actually start from;
   the pinned pair is only the fallback when that pointer fails. The 3060's
   keeper holds its last four signed pairs in `~/.easybtx/snapshots/`
   (287,300 to 287,600 on 11-25, about 32 MB each), which is where the file
   to publish comes from. `scripts/publish-attested-snapshot.sh` would
   publish one, but read its step 5 first: it deletes older
   `utxo-snapshot-<n>` pre-releases and protects only 225,927, while
   released apps pin 239,111 and 245,300 as well.
5. The real-engine `#[ignore]` suites against the built 0.34.16 btxd, an
   upgrade of a real mirror-era datadir (recipe, "Rebuilding the Linux
   release", step 5), and `ps` on one real install to see `-swapnet=0`.
6. Then the release itself: the version in the five files, the changelog
   heading, the site, and **hold the 3060's app auto-update before the feed
   goes live** (`docs/btx-update-protocol.md` B3.0). The signer's engine is
   v0.34.15 today (`/BTX:0.34.15(innovation3060)/`, tip 287,646 at 23:20 UTC
   on 2026-11-25).

**The confirmer cannot sign on 0.34.16.** Open-work item 0 below is no longer
a warning about a pull request: btxchain/btx#225 (`af9020cd`) is in this tag.
`signutxosnapshotmanifest` now rebuilds the statement from the node's own tip
and signs only if every field matches, so the confirmer's co-signature 144
blocks deep and its dissent (file fields zero) are both refused: "Refusing to
sign manifest: consensus fields do not match this node's active-chain
statement". Each attempt would make the engine write and delete a full UTXO
export under `<datadir>/utxo-snapshot-sign/` first, every ten minutes per
waiting statement. So the app does not ask: `snapshot_confirmer::why_not_on_engine`
reads `getnetworkinfo.version` and stands the confirmer down at 3416 (0.34.16)
and newer, before the website or the engine is asked anything. Copy
diagnostics shows "confirmer: off, this engine signs only at its own tip, so
this node cannot add its signature to another operator's statement", and the
`btx-confirmer` binary refuses to run. Nothing breaks for users: snapshots are
confirmed by two PRODUCERS (the 3060 and Jarek, the way item 0 describes), and
`dumptxoutsetattested` did not change. **But no node on this engine can
dissent from a wrong statement any more.** On the 3060 it has never signed anything (there is no
`snapshot-confirmer.json` in its data folder): every statement it sees
carries its own operator's signature already. The decision owed: retire the
confirmer and the dissent, or give them a form that works at the tip. `confirmed_snapshot_regtest` and
`snapshot_network_regtest` fail at their co-signing steps against 0.34.16
until that is settled.

## Update 2026-10-18: Jarek confirms snapshots

**Jarek (Byron Bay) is the second snapshot operator** (#218, owner's decision
of 2026-10-18, after btxchain/btx#239). His validator key `02d12c32…fe030`
is in `crates/btx-core/snapshot-operators.json` as an operator, NOT a pin:
it confirms snapshots, has no say over blocks, and `trim_to_pinned` keeps it
from the engine. He cuts at every multiple of 1,000 while his own tip is that
block and publishes the single-signature manifest at
`https://api-explorer.btxbyronbay.com/snapshots/`. At 251,000 his statement
and file were the 3060's, byte for byte
(`confirmed_snapshot::tests::mende_and_jarek_confirm_251000_on_mainnet`).

**The website picks his cuts up** (EasyBTX #599, `.github/workflows/byron-cuts.yml`,
hourly at :41, no secret). It posts a cut only onto a statement the website
already holds from the 3060, which posts at 144 deep. His tip-time cut never
opens a statement or a dispute on its own. A cut that differs fails the job
for a human to look at. First real run, 2026-10-17 22:5x UTC: his signature
merged into 251,000 (record confirmed, operators Mende and Jarek), but
writing the public manifest answered 503. `@vercel/blob`'s `useCache: false`
adds `?cache=0`, which the PUBLIC store refuses with 400 whether the blob
exists or not. Fixed in EasyBTX #601 (private reads only). After it deploys,
the next statement that adds a signature runs `republishStale` and
`/api/snapshots/latest` serves 251,000. Check that, then check that easyNode's
loader takes it.

**Older apps are unaffected.** They check a confirmed pointer against their own
compiled list, where Jarek is missing. `peek_confirmed` fails quietly (no
Fast-forward offer) and `prepare_start` falls back to the pinned 245,300 pair,
just as with the 404. Only 0.7.16 and later take the confirmed pair.

**0.7.16 is the version bump in #220.** It goes out for Linux and Windows with
#216, #217 and #218. The order above still holds: the Mac's 0.7.15 first, and
once #220 is on main, that Mac build must come from `e76315b`. Or skip it:
build 0.7.16 for all three platforms, and the Mac gap closes with it.

**The 3060 skipped 249,600 and 250,000** (no statement at either). Jarek's
cuts land only on multiples of 1,000, so a skipped one costs a day without a
new confirmed snapshot. Being investigated on the 3060 PC itself (its logs).

**Jarek has a brief** (Mende sent it in the nodes Telegram group,
2026-10-18): the pickup contract, "don't post to /api/snapshots/statement
yourself", and proposals that are not agreed yet (cut every 100, his key in
a future M=2 block set, explorers checking each other, a second AInode model
host).

## Where things stand (2026-10-14)

**Published: easyNode 0.7.15 for Linux and Windows** (`node-v0.7.15`,
release id 413015565, built by CI from main's `e76315b`, #212, runs
38607810262 and 38607825843): #211 on top of 0.7.14, so a card lost
mid-run is recovered even when easyNode itself restarted after the loss.
The site moved Linux and Windows to it (EasyBTX #590). Before it,
**easyNode 0.7.14 for Linux and Windows** (`node-v0.7.14`, built
by CI from `6ac2e60`, tree identical to main's `f637808`, #209), the audit
fixes of #208 on top of 0.7.13. Same checks as 0.7.13 below: draft first,
every asset re-downloaded identical, signatures verified against the released
bytes, Latest still v0.30.0. The Mac is next, below.

**On main, not released (2026-10-16): the 5-minute chain-state write**
(`btx_core::state_flush`). It becomes 0.7.16 for Linux and Windows only after
the Mac's 0.7.15 is in `node-v0.7.15` (it was not on 10-16: only the
AppImage, the `.deb`, the Windows setup, their `.sig`s and `SHA256SUMS`). The
Mac can take it straight into 0.7.16.
What 10-15 taught, so nobody re-derives it: btxd writes on its own every 50
to 70 min (validation.cpp:127), `gettxoutsetinfo` forces a write first
(rpc/blockchain.cpp:1488), and btxd retries its addnodes every ~15 s by
itself (net.cpp `ThreadOpenAddedConnections`), so an app-side re-dial adds
nothing. The 6 peerless minutes after the 10-15 restart were the 3060's
TP-Link Wi-Fi failing to associate until 10:53:46 (WLAN-AutoConfig 8002,
"The driver disconnected while associating"); on 10-14 it took 9 minutes.
Its Ethernet port is unplugged.

## Tomorrow on the Mac: add the Mac build to node-v0.7.15

The Linux and Windows assets of `node-v0.7.15` are published and signed. Do
NOT rebuild or re-sign them; the Mac build goes into the SAME release, the way
0.7.8 to 0.7.12 did. Until it ships, every Mac below 0.7.12 is offered nothing
(the feed names 0.7.15 with no Mac key), which is why this is first.

1. `git pull` on main (`e76315b` or later; `tauri.conf.json` says 0.7.15).
2. Prove the staged `apps/node/src-tauri/resources/node-pkg` is the shipped
   engine: byte for byte against 0.7.12's Mac bundle (btxd `a315d86e…`), as in
   recipe step 4. Never run `stage-node-pkg.sh`.
3. Build: `cd apps/node && TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/easybtx.key)"
   TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" npx tauri build --bundles app,updater`,
   then the `.dmg` with `hdiutil` (Applications symlink plus the `.app`, UDZO).
4. Rename the tarball to `BTX-Node_0.7.15_aarch64.app.tar.gz`, sign THAT name
   (`npx tauri signer sign -f ~/.tauri/easybtx.key -p "" <file>`), and verify it
   with `apps/node/scripts/verify-updater-sig.py`.
5. Upload the tarball, its `.sig` and `BTX-Node_0.7.15_aarch64.dmg` to
   `node-v0.7.15`. Replace `SHA256SUMS` with five lines: the three published
   lines unchanged, plus the tarball and the `.dmg`, as 0.7.12's had. Keep
   `make_latest` `"false"`, and check `/releases/latest` is still v0.30.0.
6. Feed with all three platforms WITHOUT re-signing Linux and Windows
   (`build-node-feed.sh` re-signs every file it is given). Download the
   published `.sig` files of the AppImage, the `.deb` and the setup `.exe`,
   then `python3 apps/node/scripts/gen-node-feed.py --version 0.7.15 --tag
   node-v0.7.15 --mac-sig <tgz>.sig --linux-sig <AppImage>.sig --deb-sig
   <deb>.sig --win-sig <exe>.sig --notes "easyNode 0.7.15. See
   https://easybtx.com/node/changelog" --pub-date <now, UTC> --out
   latest-node.json`.
7. Site PR on EasyBTX: copy both feeds into `site/public/updater/`; move
   `REL_MAC`, `relPageMac` and `dl.mac` in `site/src/pages/node.astro` to
   `node-v0.7.15`; rewrite every "the Mac stays on 0.7.12" sentence on `/node`
   and `/node/how-to` (grep `0.7.12`); change the 0.7.15 changelog heading's
   platform tag to `mac + linux + windows` in easynode and re-sync
   `apps/node/CHANGELOG.md`. Run `scripts/check-download-links.sh`. Merge only
   once the Mac assets answer 200.
8. Edit the `node-v0.7.15` release notes: add the Mac (the `.dmg`, and the
   updater tarball) to the first line, and drop both Mac sentences at the
   end (the Mac staying on 0.7.12, and a Mac on 0.7.11 or older being
   offered nothing). The notes of node-v0.7.13 and node-v0.7.14 already say
   they have no Mac build, and so do their changelog headings.
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

## The UI audit (shipped in 0.7.12)

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

00. **The Mac build of 0.7.15**: "Tomorrow on the Mac" above. Build 0.7.15,
    not 0.7.14: node-v0.7.14 stays Linux and Windows only.
01. **Done 2026-10-14: the WSL recovery, end to end, in the real app.** A
    real card reset on the 3060 under 0.7.14: the app restarted WSL at 15
    minutes and the node was checking blocks again 62 s later, no clicks
    (timeline in docs/incident-2026-10-11-3060-verifier-quarantine.md). It
    showed one gap, fixed by #211 and released in 0.7.15: an app started
    after the loss never acted (`engine_lost_its_card`). To repeat it: an
    admin `pnputil /restart-device` on the card while the app has verified
    blocks; a GPU-memory hog does NOT cause the loss (WDDM pages GPU memory
    out, measured 2026-10-14).

0. **Upstream btxchain/btx#225 ends co-signing as the confirmer does it,
   and it is in v0.34.16** (see the 2026-11-25 update at the top). Written
   on 2026-10-12, when it was open and "a later 0.34.16 can carry it" (its
   author, 2026-10-07): its `af9020cd` makes `signutxosnapshotmanifest` rebuild the
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

1. **A second operator confirms: done, waiting on EasyBTX #601** (see the
   2026-10-18 update above). Jarek co-signs every multiple of 1,000. Until
   then, every statement since 5 October carried only `02d5efca`.
   Aleksander (zbtx1 to zbtx3) and jpp would still make it sturdier. zbtx1 sits on a dead branch from
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
