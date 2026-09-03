# easyNode

**Run a real BTX node from home, on the machine you already own.**

easyNode is a desktop app that installs, configures and supervises `btxd`, the
BTX full node, for someone who does not want to run a daemon by hand. It is MIT
licensed and this repository is the whole of it.

Downloads and the auto update feed live in
[EasyBTX-releases](https://github.com/MendeMatthias/EasyBTX-releases).

## Why this exists

BTX needs nodes that are simply on. Not fast, not powerful, on. A trusted mirror
serving an explorer or a wallet can only be as honest as the nodes around it, and
right now there are not many of them.

The uncomfortable version, measured 2026-09-03: exactly one source on the whole
network publishes a block hash at a given height. When our own explorer spent six
hours following an orphaned block that morning, the only reason anybody could
tell was that that one source happened to still be running.

More independent nodes is the fix. That is what this app is for.

## What your machine can actually do

Roles are cumulative. A machine takes the highest one it qualifies for and keeps
everything below it. **Being useful does not require a good computer**, but being
an independent validator does, and we would rather say so than flatter you.

| role | what it needs | what it gives BTX |
|---|---|---|
| **Relay** | a connection and an open port | peer introduction and address gossip |
| **Keeper** | about 10 GB disk, an inbound port, uptime | passes signed confirmations to other nodes |
| **Archive** | full chain disk, upload bandwidth, uptime | serves block history to people setting up |
| **Witness** | a qualifying GPU or Apple Silicon | an **independent opinion** about which chain is real |
| **Signer** | a qualifying GPU, and above all always on | the attestations mirrors depend on |

Two things worth knowing, because they are not obvious:

**Only a GPU or Apple Silicon machine can be a real witness.** btxd only
advertises `NODE_MATMUL_CONSENSUS` in `strict-device` mode with a qualified
provider. A node without one follows the chain through other people's signed
attestations, which makes it useful but makes its agreement an echo rather than
evidence.

**For signing, staying on beats being fast.** One reliable machine that never
goes quiet is worth more than many powerful ones that sleep.

**For most home machines the limit is a router setting and a power setting, not
the hardware.** Forwarding a port and disabling sleep is usually the difference
between one role and the next.

## What this app will never do

- It will never ask for your seed phrase or private key in a web page.
- It will never promise you a reward. There is no token, no payment, no earnings
  here. Contribution is recognised and recorded, and nothing is owed to you.
- It will never install an update that was not signed by the release key.

## Build it yourself

```bash
cd apps/node
npm install
npm run tauri dev          # run it
npm run tauri build        # package it
```

Rust and the Tauri prerequisites are required. `crates/btx-core` is the shared
library that talks to `btxd`; it is built as a path dependency and needs no
separate step.

## What is in here, and what is not

```
apps/node/          the desktop app, UI and Tauri shell
crates/btx-core/    node lifecycle, RPC, health, disk, installer, service report
```

The easyBTX **miner** is a separate product and is **not** open source. It lives
in a private repository and always will. This repository is the node, complete.

## Three levers, and only one of them is open

Opening the source opens exactly one thing, and it is worth being precise
because "open source" is often read as "no control anywhere".

- **The licence governs the code.** MIT. Read it, change it, ship your change.
- **The trademark governs the name.** Forks are welcome and must ship under a
  different name.
- **The signing key governs updates.** Only builds signed by the release key are
  installed by the in-app updater. A fork cannot push an update to anybody
  running this app.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Security issues go through
[SECURITY.md](SECURITY.md), never a public issue.

The most useful contributions right now are Windows support, packaging for more
Linux distributions, and anything that makes the "what can my machine do" answer
more accurate on hardware we do not own.

## Talking to upstream

We asked the BTX maintainers directly what a home node client must do, and must
never do, to be worth recommending:
[btxchain/btx#139](https://github.com/btxchain/btx/issues/139). If you have an
opinion about that, it belongs there rather than here.

## Licence

MIT. See [LICENSE](LICENSE). Fonts under `apps/node/src/assets/fonts` are
JetBrains Mono, SIL Open Font License, see the `OFL.txt` beside them.
