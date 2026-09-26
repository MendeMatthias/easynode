# Forking easyNode

Forks are welcome. More independent node apps make BTX harder to break, and
that is what the MIT licence is for: as [docs/always-on.md](docs/always-on.md)
puts it, we would rather five node projects existed than one.

This page is the short list: the four things a fork must change so it does not
collide with this app on a user's computer, one decision about the data folder,
and what a fork can keep as it is.

## Change these four

**1. The name and icon.** `productName` and the window title in
`apps/node/src-tauri/tauri.conf.json`, and the files in
`apps/node/src-tauri/icons/`. The licence covers the code, not the easyNode or
easyBTX name or logo ([CONTRIBUTING.md](CONTRIBUTING.md#a-note-on-the-name)).
A user should always be able to tell whose build they are running.

**2. The app identifier.** `identifier` in the same file, today
`com.m2promende.easybtx-node`. The operating system keeps an app's settings
and storage under its identifier, so two apps with the same one share them.

**3. The update feed and its key.** `plugins.updater.endpoints` and
`plugins.updater.pubkey` in the same file. If you keep both, your users are
offered our next release, and installing it replaces your app with ours. If
you keep our feed with your own key, each of our releases shows your users an
update that then fails. So:

- Generate your own key pair, from `apps/node`:
  `npx tauri signer generate -w ~/.tauri/<your-app>.key`.
- Give it a passphrase, and keep the private half out of the repository.
  `*.key` is ignored here, and CI fails on key material.
- Publish your own feed and point the endpoint at it, or turn the updater off.

The endpoint is compiled into every build, so settle it before your first
release: changing it later strands every copy already installed.
[docs/node-release-recipe.md](docs/node-release-recipe.md#the-feed) explains
how ours works. `tauri.linux.conf.json` and `tauri.windows.conf.json` are empty
today; anything added to them overrides the main file on that platform.

**4. The check-in name.** A node that signs confirmations for mirrors sends
its public key to `easybtx.com`, so mirror operators can find it
(`crates/btx-core/src/checkin.rs`). Nothing else sends a check-in, and it only
happens once the operator turns signing on. Keeping this helps your signers
get found, but change `agent` in `apps/node/src-tauri/src/commands.rs`, today
`easynode/<version>`, so the directory can tell your nodes from ours.

## Decide about the data folder

The chain lives in `~/.easybtx`, or `%APPDATA%\easyBTX` on Windows (set in
`crates/btx-core/src/platform/`). The easyBTX miner shares that folder on
purpose, so the chain is stored once. Sharing only works between apps that
agree on the node: this app treats the node recorded in
`<datadir>/easybtx-node.pid` as its own, and stops any other node running on
that folder so it can start its own.

If your fork pins a different engine or different node settings, give it its
own folder: change the `data_dir()` functions in `crates/btx-core/src/platform/`
and the `.easybtx-location` file name in `crates/btx-core/src/datadir.rs`.
Otherwise the two apps will take over each other's node. Two nodes running at
once on one computer also need different ports; btxd's defaults are 19335 for
peers and 19334 for RPC.

## Keep these, if you like

Most forks should keep all of it:

- **The engine pin,** `NODE_RELEASE_TAG` in
  `apps/node/src-tauri/src/commands.rs`, and the two workflows that guard it,
  `engine-tag-guard.yml` and `engine-fleet-ready-guard.yml`.
- **The signed snapshots** the app downloads from
  `MendeMatthias/EasyBTX-releases`. The node refuses a snapshot that is not
  signed by a key it pins, so the download host never has to be trusted.
- **The pinned signer keys,** `BTX_TRUSTED_ATTESTATION_PUBKEYS` in
  `crates/btx-core/src/node.rs`. A node that follows signatures trusts these
  keys to tell it which blocks are valid. Keeping them means trusting the same
  signers we do. Changing them is a trust decision, so make it on purpose.
- **Links and reads:** transaction and address links open btxscan.io, the
  stats button opens btxprice.com, and Esplora mode reads the chain census at
  `easybtx.com/api/nodes`.

## Stay in touch

If you fix something that matters for every node, a pull request here helps
your users and ours. The node software itself is developed upstream at
[btxchain/btx](https://github.com/btxchain/btx).
