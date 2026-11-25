# Next easyNode session: the handover

State on 2026-10-14, after 0.7.15 (Linux and Windows). Verified against the
released bytes, not
just the local build, except where it says otherwise.

---


## The swarm switch (draft PR 239, 2026-11-25, nothing released)

Owner's rules (2026-11-25): Linux only; two engines, the second one built
with the model network and fetched only when the swarm is switched on; off by
default. A node that signs refuses the switch until the cost per block is
measured.

Where it is:

- `crates/btx-core/src/swarm.rs`: the launch flags, the engine probe (the
  bytes `-modelnetrequired` in the btxd file), the reader for
  `getmodelnetworkinfo` + `listmodels` + the guard's file.
- `crates/btx-core/src/swarm_engine.rs`: download, sha256, signature,
  unpack beside the final folder, hold to the pin, one rename into
  `~/.local/btx/<tag>-modelnet/<platform>/`. Never the default engine's
  folder.
- `apps/node/src-tauri/src/commands.rs`: `SWARM_ENGINE_PIN` (compiled in from `apps/node/engine-modelnet-linux.pin`, written by the Linux engine workflow since PR 240) and `SWARM_ENGINE_RELEASE` (the published release's address), `swarm_engine_for_launch`, `set_swarm`, `apply_swarm_switch`,
  `swarm_status`, `swarm_card`. The start path picks the engine and writes
  the marker `easynode-swarm-on` together.
- `apps/node/src/swarm-card.ts`: every sentence of the row and card.

When the swarm engine is broken, and the other rules (2026-11-25, same PR):

- Signer rule: `swarm_signs_in` is the one test, asked by the launch, the
  switch and the card. It is the app's signer switch on a host that checks
  blocks, OR `signing_key_the_app_does_not_manage` (a key in `btx_rw.conf`,
  or in faststart.conf under the other option names).
- A launch of the swarm engine that does not come up holds the engine back
  and the same start begins again on the default engine, once. It is decided
  from the kind of failure, which the launch writes down where the failure
  is made (`EngineFailure`: could not be run, exited before answering, never
  answered, a file not loaded), never from the sentence. A Quit, a Stop, the
  set-aside, a full disk, the graphics check, warming through the whole wait
  and the read-block recovery write nothing down. `StartPasses` counts the
  passes; if the default engine then fails too, the hold is withdrawn
  (`withdraw_swarm_hold`), because what stops both is not the swarm engine's.
- The hold is the file `easynode-swarm-engine-held-back` in the node folder
  (`btx_core::swarm::hold_engine_back`, `engine_held_back`,
  `clear_engine_hold`): `app <version>`, then the reason. While it is there
  `swarm_engine_for_launch` answers the default engine. Only `set_swarm` (on
  or off) and a fresh install clear it. Card state `engine_start_failed`.
- A swarm engine that ends badly while running (signal, exit code other than
  0): one such end within an hour of its start is written to
  `easynode-swarm-engine-bad-end` and the node is left exactly as the
  default engine's is ("stopped responding", Start launches the swarm engine
  again). A second within 24 hours holds the engine back and the node starts
  again on the default engine (`swarm_crash_step`,
  `btx_core::swarm::judge_bad_end`). A run that lasted an hour or more is
  not counted. Nothing here starts the swarm engine again by itself.
- A damaged install (the list an install wrote is there, files are missing or
  the wrong size, or `bin/btxd` is gone) is `swarm_engine::damage`. The start
  writes which files to setup.log and runs the default engine; card state
  `engine_damaged`; off and on fetches again and `install_from_archive`
  replaces the folder.
- An install records its archive's sha256 in `.easynode-engine-archive` in
  the engine folder, and `swarm_engine::installed(pin, root)` wants it to be
  the pin's. A release that publishes another archive under the same tag is
  therefore fetched; the card reads `engine_missing` until then.
- The switch claims the download before its task is spawned
  (`claim_swarm_switch`); a request while one runs is refused. It restarts a
  running node only to change its engine (`swarm_switch_needs_restart`).

Open, on purpose:

- After an update that moves `NODE_RELEASE_TAG`, a switched-on swarm reads
  `engine_missing` ("the swarm engine for this version of the app is not
  installed", off and on fetches it). Nothing fetches it by itself: that is
  half a gigabyte nobody asked for at that moment, again at every start if
  it fails. The owner decides.
- Old `<tag>-modelnet` folders in `~/.local/btx/` are not removed (about
  830 MB each). The default engine's old folders are not removed either.
- Neither engine is started again by the app after one outside kill; the
  swarm engine is treated as the default engine is.
- Not covered: a swarm engine that stays up and misbehaves (helper down, not
  syncing); one that ends badly only after an hour or more, every time; a
  folder whose file list was deleted reads as never installed.
- Not tested: any of this with a real engine or inside a running app. The
  rules, the pass loop, the crash steps, the files and the replacement are
  run by tests; that the start path and the refresher call them at the right
  place is held by source checks.

To make it live, a release has to:

1. Publish the ModelNet engine as a `.tar.gz` in the layout of
   `resources/node-pkg` (`bin/btxd`, `bin/btx-cli`, `bin/btx-modeld`,
   `bin/btx-guardd`, `bin/lib/...`, `.btxd-version`), regular files and
   folders only.
2. Sign it with the app's update key (`tauri signer sign`), publish the
   signature beside it as `<archive>.sig`.
3. The pin is now read from `apps/node/engine-modelnet-linux.pin` (PR 240
   came with it; this branch does not carry its own copy) and the release address
   is `SWARM_ENGINE_RELEASE` in `commands.rs`. A new engine build means a
   new release tag, a new pin, and that constant changed with it; a
   published archive is never replaced. Done for v0.34.16 on 2026-11-25:
   release `engine-modelnet-v0.34.16`, id 458120480.

Run on the real engine on 2026-11-25 (CI run 45598950201, artifact
15015833815, repackaged by hand; `crates/btx-core/tests/fixtures/swarm`):

- The probe finds the option name twice in the ModelNet btxd, never in ten
  engines without it.
- The archive (489,913,031 bytes, 779 files, 828,804,903 bytes unpacked)
  installs through `install_from_archive` in 8.5 s, real `btxd --version`
  check included.
- Two regtest nodes with the app's observing flags, pointed at each other
  with `-modelpeer`: each saw the other 5 s after start. With `-modelnet=0`:
  no child process, no listening port. Everything stopped with `btx-cli stop`.
- A btxd given `-modelrpcsocket=<a stand-alone helper's socket>` attaches to
  that helper (`helper_managed_by_btxd: false`), starts none of its own, and
  `listmodels` answers from the helper's store. It still starts its own
  guard and writes `swarm/` and `security/` in its own folder, and it leaves
  the helper running when it stops.

Not tested: the download and signature against a published archive (there is
none), the whole switch inside a running app, a restart of the node onto the
swarm engine on mainnet, a model held on both nodes ("models both hold"),
LAN discovery between two computers, the cost to a signer.

## Update 2026-11-25 (later): fixes from the audit of 0.8.0, for a 0.8.1

Not released. One pull request (#236) from `claude/0.8.0-audit-fixes`, no
version bump. What it does is in the changelog under Unreleased; the rules
are in the module doc of `crates/btx-core/src/engine_files.rs`.

**The rule that matters most, from the review of the first version:** two
builds of one engine tag are not the same files. linuxdeploy rewrites the
run path of every ELF it bundles, an engine artifact can be pinned again
under the same tag, and the easyBTX miner installs its own build into the
same `~/.local/btx/<tag>/`. So the start-time check never treats a size
difference against the app's package as damage. It copies a file back only
when the file is missing or not the size the folder's own list says, the
package's copy has exactly the listed size, and nothing is running from the
folder (no holder of the node folder, no RPC answering). Everything else is
written to `setup.log` and left alone. A whole engine is replaced only by an
install on a tag change and by the answer to a loader failure.

**Whether a new AppImage's engine equals an installed 0.8.0 one is not
known and no longer matters.** The 3060's installed engine equals the 0.8.0
package it was copied from (776 files, all sizes), which says nothing about
the next build. If the next build differs in size anywhere, an install with
no list is adopted as "another build": its list is written from the folder
and nothing is copied.

**Open, for the owner: old engine folders are never removed.** Every engine
the app ever installed stays under `~/.local/btx/<tag>/`. On the project's
3060 that was ten folders and 4.4 GB on 2026-11-25 (0.33.3 to 0.34.16; each
0.34 one is about 580 to 710 MB). Proposed rule, not implemented: when an
engine update has finished and the new engine is running (the place the
saved tag moves in `start_node_once`), keep the folder of the pinned tag and
the folder of the tag the node ran before, and remove the rest. Why it is
not in the pull request: the folder is shared with the easyBTX miner, so a
folder easyNode does not use may be the one the miner runs. A safe version
needs a record of which folders easyNode itself installed (it has none for
past installs), or a check that no running process was started from the
folder.

**Open, for the Mac session:** on macOS the start-time check reports and
copies nothing (`NotCopied::MacOs`). A copy there signs the file again, and
a cut-off Mach-O is not seen by a size check. Run it on a Mac, then decide.
The `dyld` line is matched on its documented shape, not on real output. The
whole-engine copy after a loader failure is the same code as an install and
is not gated.

**What was run against the real 0.34.16 engine, 2026-11-25:**

- Linux: with `libpython3.14.so.1.0` removed the engine exits 127 with the
  loader's line; the app's code names the file, puts it back and the engine
  runs again (`engine_files::tests::a_real_engine_with_a_library_removed_is_put_back`).
  Removing the `encodings` folder of the Python library does not stop
  `btxd --version`, so only the start-time check finds that kind of damage.
- Windows: with `python314.dll` removed and the error mode set, btxd.exe
  exits with -1073741515 in under a second and prints nothing. With error
  mode 0, which is what an app started from the desktop has, it had not
  exited after six seconds: the system's message box. The app's own call was
  run too (`a_real_windows_engine_with_a_dll_removed_exits_instead_of_waiting`).
- What the Windows engine loads from outside its own folder, read from the
  import tables of btxd.exe, btx-cli.exe and python314.dll: only Windows
  system libraries (kernel32, advapi32, ws2_32, iphlpapi, shell32, msvcrt,
  bcrypt, version, and the Universal C Runtime's `api-ms-win-crt-*`). The
  Visual C++ runtime, the GCC runtime and Python are in the package. That is
  what the "a file of Windows itself" sentence rests on.

**Not tested:** the check and the launch-loop answer inside a running app,
on any system. Their rules are tested in btx-core, the app's decisions
(`engine_check_mode`, `engine_check_wanted`) as functions, the wiring by
reading the source.

---

## Update 2026-11-25: easyNode 0.8.0 carries BTX 0.34.16 (Linux and Windows)

**The plan** (owner's planning session, 2026-11-25): move the engine from
v0.34.15 to upstream's final **v0.34.16 =
`074dac167c26c1e091198f691ed5727817dba88a`**, pinned by SHA because upstream
moved that tag once (from `d27530af`). Not the 0.34.17 prerelease. Step one is
parity: the same roles and behaviour on the newer engine, the model network
still compiled out (`-DWITH_MODELNET=OFF`), SwapNet off at run time. The
swarm and ModelNet are a later release. Three threads worked in parallel: the
Linux engine build (#235, merged), the Windows engine build (#234, merged)
and the app side (#233). The planning session merges and publishes: Linux
first, the 3060 signer is updated from it, Windows follows.

**#233 is the release pull request for 0.8.0.** It moves `NODE_RELEASE_TAG`
and `NODE_RELEASE_COMMIT`, passes `-swapnet=0` to 0.34.16 and newer, stands
the snapshot confirmer down on this engine, pins the Linux installer to the
proven engine build (run 45523343898, artifact 14981349727), moves the two
contributor staging scripts, and sets the app version to 0.8.0 in the six
places the recipe names. The Windows installer pins came with #234 (run
45516298111). `commands.rs` has everything that was read at the SHA beside
the pin. When this was written nothing was published: no release, no feed,
no site change.

**What is still open after 0.8.0:**

1. **The Mac.** Upstream publishes no macOS archive for 0.34.16, so
   `stage-node-pkg.sh` refuses for this engine and says so. The Mac needs a
   source build at the SHA (`btxd-macos.yml`, `stage-node-pkg-mac-source.sh`)
   before it can ship this release; until then it stays on the version it has.
2. **SwapNet cannot be switched on inside an AppImage as the app starts btxd
   today.** Inside an AppImage btxd inherits `LD_LIBRARY_PATH`, `PYTHONHOME`
   and `PYTHONPATH` from AppRun, and so loads the AppImage's own libpython,
   which has no standard library beside it. Measured on the 3060 by the
   planning session on 2026-11-25, under that environment: the bundled engine
   starts and qualifies its GPU with `-swapnet=0`, and refuses to start with
   `-swapnet=1`. 0.8.0 always passes `-swapnet=0`, so it is not affected. But
   before any release switches SwapNet on, the child's environment has to be
   cleaned where btxd is spawned (`build_node_command` in
   `crates/btx-core/src/node.rs`, which today never clears it, on purpose, so
   power users can override), and the engine tested with `-swapnet=1` from
   inside the real AppImage.
3. **The pinned start pair** (`attested_snapshot::pinned_pair()`) stays on
   245,300 in 0.8.0, about 42,000 blocks back. The refresh waits for a
   two-operator pair PUBLISHED as `utxo-snapshot-<height>` on
   EasyBTX-releases; the newest one there is 245,300 itself. The website's
   confirmed pointer serves 287,000 (Mende and Jarek, 2026-11-24), which is
   what new nodes actually start from; the pinned pair is only the fallback
   when that pointer fails. `scripts/publish-attested-snapshot.sh` would
   publish one, but read its step 5 first: it deletes older
   `utxo-snapshot-<n>` pre-releases and protects only 225,927, while
   released apps pin 239,111 and 245,300 as well. #233's description has the
   steps and the checks for the refresh.
4. The real-engine `#[ignore]` suites against the built 0.34.16 btxd, an
   upgrade of a real mirror-era datadir (recipe, "Rebuilding the Linux
   release", step 5), and `ps` on one real install to see `-swapnet=0`.
5. The release steps themselves are the recipe's: the site, the feed, and
   **hold the 3060's app auto-update before the feed goes live**
   (`docs/btx-update-protocol.md` B3.0). The signer's engine was v0.34.15
   when this was written (`/BTX:0.34.15(innovation3060)/`, tip 287,646 at
   23:20 UTC on 2026-11-25).

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

Run end to end on 2026-11-25 against the published release (isolated HOME,
regtest): the app's download from the public address, sha256 and the real
update key's signature, install, in 124 s; a doctored signature and a
doctored archive (with a pin made to match it) served locally were both
refused by the signature check, nothing installed. One node folder started
on the default engine (no model arguments, `getmodelnetworkinfo` unknown, no
child, no port), on the downloaded swarm engine with the switch on (the
observing set, helper READY, follow/seed/relay off), and on the default
engine again; the swarm engine with the switch off got `-modelnet=0` and ran
silent. What was NOT run: the app itself switching, restarting and falling
back (the start path and the refresher), and mainnet.
