# easyNode

**Run a real BTX node from home, on the machine you already own.**

easyNode is a desktop app that installs, configures and supervises `btxd`, the
BTX full node, for someone who does not want to run a daemon by hand. It is MIT
licensed and this repository is the whole of it.

Downloads and the auto update feed live in
[EasyBTX-releases](https://github.com/MendeMatthias/EasyBTX-releases).

**If you read one thing here, read [Always on](docs/always-on.md).** It is the
argument for the whole project: what BTX is actually short of, what running a
node really costs you, and what it does not promise in return.

## This repository is the app

Not a mirror of one. easyNode is developed here, released from here, and this is
where a change belongs. If you are looking at a copy of these files somewhere
else, that copy is downstream of this one.

That means the whole of it is here: the app, the shared engine library, the
release recipe in [docs/node-release-recipe.md](docs/node-release-recipe.md),
and the two engine gates in `scripts/` that must both pass before any release
changes which `btxd` users run. The only thing deliberately absent is the
signing key, which is not in any repository and never will be.

If you want BTX to have a home client that anyone can audit and anyone can
improve, this is the one to send patches to.

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
| **Full node** | about 150 GB on an SSD (the chain is ~138 GB of blocks, 2026-09-04) | validates everything itself |
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

## See it in ten seconds

No Rust, no staging, no package manager. This serves the real UI:

```bash
cd apps/node
npm install
npm run dev        # http://localhost:1430
```

Measured at under a second to a live page. Anything that has to talk to the node
will error in a plain browser, because there is no Tauri shell behind it, but
every screen, state and piece of copy is there to read and change.

## Run the tests

Both suites pass on a fresh clone and neither needs the staging step:

```bash
cd apps/node && npm test              # 52 tests, under a second
cd crates/btx-core && cargo test      # 234 tests, about 5 seconds
```

CI runs exactly these on every pull request, plus `tsc --noEmit` and a
production `vite build`.

## Build the real app

```bash
cd apps/node
npm install
./scripts/stage-node-pkg.sh   # REQUIRED FIRST, see below
npm run tauri dev             # run it
npm run tauri build           # package it
```

**The staging step is not optional.** The app bundles the BTX node package
itself, so a user does not need Homebrew or a package manager, and that tree is
about 25 MB of binaries which do not belong in a source repository. The script
fetches them from the public upstream release, verifies the SHA-256 against the
signed sums, and stages them into `src-tauri/resources/node-pkg/`. Without it
`tauri build` fails with `glob pattern resources/node-pkg/**/* path not found`,
because `tauri.conf.json` declares that directory as a bundle resource.

The staged engine is BTX v0.34.5, matching `NODE_RELEASE_TAG` in
`src-tauri/src/commands.rs`. Those two must agree: the app installs the bundled
package into a directory named after the tag and then checks that the binary
reports that version, so a mismatch fails first-run setup rather than quietly
running the wrong engine. The script writes a `.btxd-version` marker recording
what it actually staged.

Verified on a Mac with no Homebrew installed: v0.34.5 links no Homebrew
libraries, so staging needs nothing beyond the standard toolchain. Earlier
releases did, which is why the script still contains the dylib vendoring code.

**This is the contributor path, not the release path.** `stage-node-pkg.sh`
stages the official upstream release tarball, which is what you want in order to
build and run the app yourself. Shipped releases are built from a different
tree, and the two are not interchangeable, so do not assume a local
`tauri build` reproduces a published artifact byte for byte.

**On Linux it also does not produce a node that can validate.** The upstream
plain tarball carries no CUDA backend, and since the MatMul v4.7 fork
validation is exactly what needs the GPU, a node staged that way can never be
an independent validator whatever card is in the machine. It also needs glibc
2.38, which Ubuntu LTS does not have. The shipped Linux app is built from the
official source tag on Ubuntu 22.04 with its own GPU maths and kernels inside
the package, which is why that download is around 445 MB.

If you want a Linux node that actually validates, install the released app
rather than building one. Building from here is for working on the app itself.

Rust and the Tauri prerequisites are required for this path. `crates/btx-core`
is the shared library that talks to `btxd`; it builds as a path dependency and
needs no separate step. You can check just that crate quickly with
`cd crates/btx-core && cargo check`, which needs no staging.

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

The most useful contributions right now are resuming Windows releases, packaging for more
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
