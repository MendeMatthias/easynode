# A .deb install updates itself

| | |
|---|---|
| Status | approved by the owner on 2026-09-28, the first of three options discussed: the app installs its own .deb after one password prompt. To ship in 0.6.33. Ships as 0.7.0: read 0.7.0 wherever this record says 0.6.33 |
| Date | 2026-09-28 |
| Supersedes | the manual-only rule for .deb installs, "told a new version exists and is updated by hand" on the site's node page |
| Leaves alone | `latest-node.json` and every install that reads it; the update binding's rule (#148) that every feed entry is signed under its release name; the AppImage, Mac and Windows update paths |
| Origin | the owner, 2026-09-28: "so linux node has no autoupdate? ... lets improve that it can auto-update" |
| Code | `apps/node/src-tauri/tauri.conf.json` (the endpoints), `apps/node/src-tauri/src/update_binding.rs` (the platform table, the .deb guard), `apps/node/src/main.ts` (the manual path, no repeat downloads), `apps/node/scripts/{build-node-feed.sh,gen-node-feed.py,publish-node-release.sh}`, and in the website repository `site/public/updater/node-deb.json` and `scripts/check-node-links.py` |

## Context

Linux ships two ways. The AppImage has updated itself since 0.6.0. The .deb
never has. It is also the common Linux path: node-v0.6.30's .deb was
downloaded 19 times, its AppImage 11.

What a .deb install does today, read from tauri-plugin-updater 2.11.0 (the
version in `Cargo.lock`) and checked against the published 0.6.32 .deb. The
flow has not been watched on a machine; the source leaves it no other path.

- The bundler stamps the package type into the binary it packs.
  `usr/bin/easybtx-node` inside `BTX-Node_0.6.32_amd64.deb` carries
  `__TAURI_BUNDLE_TYPE_VAR_DEB`, so `tauri::utils::platform::bundle_type()`
  returns `Deb` there.
- `get_urls` looks for the feed key `linux-x86_64-deb` first, then
  `linux-x86_64`. The feed has only the second, which is the AppImage.
- So the app downloads the ~467 MB AppImage, verifies its signature, and
  `install_deb` rejects it: `infer::archive::is_deb` is false, the error is
  `InvalidUpdaterFormat`. The app records `install-failed` and does it all
  again at the next launch and every six hours.
- .deb users therefore never update without going to the site, and every
  attempt inflates the AppImage download counter that rollouts are measured by.

What the plugin already does when it is handed a real .deb
(`install_deb`, `try_install_with_privileges`): check the bytes are a .deb,
write them to a temporary file, then run `pkexec dpkg -i`; failing that, ask
for the password through zenity or kdialog and run `sudo -S`; failing that,
plain `sudo`. On a Linux desktop that is one password dialog. On a headless
server, or WSL without a polkit agent, every route fails and the plugin returns
an error.

The constraint that shapes the design: 0.6.32's update binding
(`update_binding.rs`, #148) refuses the whole release when the feed lists a
platform key it does not know, "it lists X, which no release of this app has".
A `linux-x86_64-deb` entry in `latest-node.json` would stop updates on every
0.6.32 install, on every platform. The .deb entry has to live where 0.6.32
never reads.

## Decision

### A second feed, read only by .deb installs

`https://easybtx.com/updater/node-deb.json`, the same schema as
`latest-node.json`, with exactly one platform key, `linux-x86_64-deb`. Its url
is `…/node-v<V>/BTX-Node_<V>_amd64.deb` and its signature is that file's
`.sig`, signed under that name. `latest-node.json` keeps its three keys and
never gains this one.

### The app finds its feed by install type

The updater endpoints become, in this order:

1. `https://easybtx.com/updater/node-{{bundle_type}}.json`
2. `https://easybtx.com/updater/latest-node.json`

The plugin substitutes `{{bundle_type}}` with `deb`, `appimage`, `nsis`,
`app`, `rpm`, `msi` or `unknown`. Only `node-deb.json` exists. easybtx.com
answers the others with 404 (measured 2026-09-28 for all four names the fleet
can send), and the plugin treats a non-success status as "try the next
endpoint". Every other install therefore reads `latest-node.json` exactly as
it does today, for one extra request per check.

Two rules keep this safe, and the site check enforces both:

- The typed endpoint never answers 204. On a 204 the plugin returns "no
  update" without trying the next endpoint.
- No `node-*.json` other than `node-deb.json` is ever published.

### The binding learns the .deb

`PLATFORM_SUFFIXES` gains `("linux-x86_64-deb", "_amd64.deb")`, and
`gen-node-feed.py`'s `ASSET` table gains the same row; the existing test keeps
the two equal. A .deb entry must then be signed under `BTX-Node_<V>_amd64.deb`,
like every other entry.

### No more downloads that cannot install

On a .deb install, `bundle_type() == Some(BundleType::Deb)`, the comparator
declines a release that lists `linux-x86_64` but not `linux-x86_64-deb`. Such a
feed can only offer the AppImage, which cannot install there. The check is
recorded with a reason that names the manual command, `sudo apt install
./BTX-Node_<V>_amd64.deb` from easybtx.com/node, and nothing is downloaded.
This is what happens whenever `node-deb.json` is missing, including on a 0.6.33
.deb install before the first `node-deb.json` is published.

On any install, an automatic check does not download a version again once its
install has failed on this machine; the failed version is kept in the node app
settings. "Check now" still tries. So a headless .deb install that cannot
answer the password prompt does not pull 404 MB every six hours. It shows the
manual command instead.

### The release signs the .deb

- `build-node-feed.sh --deb <BTX-Node_V_amd64.deb>` checks the name, signs,
  verifies against the app's key, and writes `node-deb.json` beside
  `latest-node.json`.
- `gen-node-feed.py --deb-sig` writes the separate file and refuses to put
  `linux-x86_64-deb` into `latest-node.json`. Its self-test covers both.
- `publish-node-release.sh`: a `.deb` in the asset folder must carry a `.sig`,
  like the tarball, the AppImage and the installer.
- `docs/node-release-recipe.md`: the feed step deploys both files.

### The website serves and checks it

- `site/public/updater/node-deb.json`, deployed in the same site PR as
  `latest-node.json` whenever a release ships Linux.
- `scripts/check-node-links.py`: `node-deb.json` has exactly the one key; its
  URL is under `node-v<its version>`; it is signed under its release name; its
  version equals the site's Linux pin, the .deb download link; `latest-node.json`
  never lists `linux-x86_64-deb`; and no other `node-*.json` exists. Each rule
  gets a sabotage case.

## Rollout

This ships as 0.7.0, not 0.6.33; read 0.7.0 for 0.6.33 below.

- 0.6.33 ships all of the above, and its site PR publishes `node-deb.json` for
  0.6.33 itself. That changes nothing on the day, since no .deb install runs
  0.6.33 yet, and it makes the path live from the start.
- A .deb install on 0.6.32 or older reads only `latest-node.json`, so it moves
  to 0.6.33 by hand one last time. The 0.6.33 release note and the group post
  say so.
- From the first Linux release after 0.6.33, a .deb install updates after one
  password prompt.

## What this does not do

- It does not make .deb updates silent. Installing a system package needs
  root, and the prompt is the price of the .deb. Silent updates for .deb users
  need an apt repository, the second option discussed, which can come later on
  top of this. The AppImage already updates silently today.
- It does not touch `latest-node.json` or the AppImage, Mac and Windows paths.
- No .rpm and no Linux ARM.
- Headless servers and WSL still update by hand. They now say so plainly
  instead of downloading the AppImage on every check.
- It does not change how the updater signing key is kept or used. What the key
  guards does change: a .deb install runs the installer as root after the
  password prompt, so on a .deb machine the key now guards root, where through
  the AppImage it only ever reached the user's own account.

## Rollback

Remove `node-deb.json` from the site. .deb installs on 0.6.33 or later then
fall back to `latest-node.json`, the guard declines it, and they show the
manual command: today's behaviour without the wasted downloads. Nothing else
reads the file.

## Acceptance

- Unit tests: the binding offers a .deb feed signed under its name and refuses
  one signed under the AppImage name; the guard declines an AppImage-only feed
  on a .deb install and never on an AppImage install; the failed-version rule
  skips an automatic re-download and allows a manual one; `gen-node-feed.py`
  refuses the .deb key in the main feed; `check-node-links.py` catches every
  sabotage case.
- End to end on an Ubuntu desktop: two signed test builds whose updater
  endpoints point at a local server. Install the older from its .deb, offer
  the newer, see the password prompt, accept, and see the app relaunch on the
  new version. Cancel the prompt once and confirm the next automatic check
  does not download again.
- On a headless Ubuntu machine: the same offer fails at the prompt, is
  recorded with the manual command, and is not downloaded again
  automatically.
