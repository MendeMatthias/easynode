# easyNode

**Run a real BTX node from home, on the machine you already own.**

easyNode is a desktop app that installs, configures and supervises `btxd`, the
BTX full node, for someone who does not want to run a daemon by hand.

**It is open source, as of September 2026, and this repository is the whole of
it.** MIT licensed: the app, the shared engine library, the release recipe, the
CI that builds it, and the gates that must pass before any release changes which
`btxd` your machine runs. The only thing deliberately absent is the signing key,
which is not in any repository and never will be.

It grew inside a private monorepo next to the easyBTX miner, and it was moved out
and opened on purpose. Software that asks people to help decide which chain is
real has no business being unauditable. The miner is a separate product and stays
closed; the test for whether a file belongs here is whether it is part of running
a node.

## Get it

**[easybtx.com/node](https://easybtx.com/node)** — the download page always
points at the current release for macOS (Apple Silicon), Linux (x86-64
`.AppImage` and `.deb`) and Windows. Verified 2026-09-04: every link on it
returns 200.

The app updates itself from there afterwards, and will only ever install a build
signed by the release key.

> **On Linux, pick the `.AppImage` if you want automatic updates.** The updater
> can replace an AppImage in place; it cannot replace a `.deb`, because the
> installed files live under `/usr` and belong to root. A `.deb` install is a
> perfectly good node — it just tells you when a new version exists and leaves
> the install to you and `dpkg`. That is a real difference between the two
> downloads, so it is said here rather than discovered later.

> ⚠ **Do not use the "Latest" button on the releases repo.**
> [EasyBTX-releases](https://github.com/MendeMatthias/EasyBTX-releases) hosts
> both this app and the easyBTX *miner*, which is a different product, and the
> repo-global Latest pointer belongs to the miner on purpose — a node release
> that captured it would break the miner's updater. Node releases are tagged
> `node-vX.Y.Z`. Pick one of those, or just use the download page.

If you want to know what the app itself will fetch, without installing anything,
the update feed is public:
[`easybtx.com/updater/latest-node.json`](https://easybtx.com/updater/latest-node.json).

**Windows lags.** macOS and Linux are on the current release; the last Windows
build is 0.6.6 and there is no update path from it yet. The code compiles on
Windows — what is missing is a release. See
[docs/node-release-recipe.md](docs/node-release-recipe.md).

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
| **Full node** | about 150 GB on an SSD (the chain measured 124 GiB of blocks on 2026-09-04 — [method](docs/archival-capacity.md)) | validates everything itself |
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

## How nodes help each other

A node is not a passive copy of the chain. It talks to the other nodes
continuously, and most of what makes one worth running is what it hands out.

**Blocks and headers.** The ordinary business of a peer-to-peer network: a new
node asks its peers for history, an established one serves it. Everybody knows
about this part. On BTX it is scarcer than it sounds — see below.

**Attestations, which are the interesting part.** Since the MatMul v4.7 fork,
validating a block means recomputing an RC episode on a GPU. A machine that can
do that forms an *independent opinion* about which chain is real, signs it, and
passes the signature to its peers. Nodes that cannot do the maths follow those
signatures instead. So a witness is not only checking its own copy — it is
producing the evidence other people's nodes depend on, and a signer that is
always on is where that evidence comes from.

Measured on one home RTX 3060 on 2026-09-04, while it held the tip:

```
stored attestations    3,008
blocks with quorum     3,008
signed frontier        at the tip, 0 blocks behind
advertises             MATMUL_CONSENSUS, MATMUL_ATTESTATION_ARCHIVE
```

The full transcript, including the part that does not flatter us, is in
[docs/gpu-qualification-rtx3060.md](docs/gpu-qualification-rtx3060.md).

**Serving history, which almost nobody does.** A node that has fallen behind can
only be rescued by a peer that is both archival *and* current. Measured the same
day from a node with 19 peers: 12 advertised `NETWORK`, and exactly **one** was
archival and current. Ten of the twelve were older versions faithfully archiving
a chain nobody is on any more. The count, and what an unpruned node actually
costs on disk, is in [docs/archival-capacity.md](docs/archival-capacity.md).

## Run one

BTX is small right now. That is not a reason to wait — it is the whole reason to
start, because every number on this page is one a single additional machine
moves measurably. One archival-and-current peer becomes two. One independent
opinion about the chain becomes two.

You do not need a good computer to help, and the app will tell you honestly what
yours can do rather than flattering it. If it turns out your machine can be a
witness, the thing that matters after that is not speed. It is staying on.

**[Get it at easybtx.com/node](https://easybtx.com/node)** — macOS, Linux and
Windows. It installs and supervises the node for you, updates itself, and will
never install a build that was not signed by the release key.

If you would rather read the argument before installing anything, that is
[Always on](docs/always-on.md).

### Keeping it up, on a machine you do not sit in front of

`scripts/node-observer.sh` samples the node every couple of minutes, records
what it saw, and recovers the two things that actually take a home node off the
network: a btxd that is not running, and a tip that has stopped advancing
because no connected peer will serve a block body. The second one matters more
than it sounds — measured 2026-09-04, of 19 peers twelve advertised `NETWORK`
and exactly **one** was archival *and* current, so a node that falls behind can
run out of anybody to ask. It dials a peer rather than restarting anything, and
it never touches a live node.

```bash
BTX_START_CMD="/path/to/start-my-node.sh" scripts/node-observer.sh &
```

It is a shell script rather than part of the app on purpose.
`crates/btx-core/src/watchdog.rs` already *diagnoses* a stall, and does it
well — it can tell "the body never arrived" from "the body is banked and the
attestation is missing". What the app does not do is *act*, because restarting
somebody's node unasked is a consent decision. This is the stopgap, and it is in
the open so the behaviour can be argued with before any of it moves inside.

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
cd apps/node && npm test              # the web suite, under a second
cd crates/btx-core && cargo test      # the core suite, about 5 seconds
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

**The Linux path that does validate is also in this repository.** Releases are
staged with `scripts/stage-node-pkg-linux-source.sh`, which takes a btxd you
compiled yourself rather than a downloaded tarball, so the GPU maths ends up
inside the package. The full procedure — the cmake invocation, the CUDA
version, the gates — is "Rebuilding the Linux release from nothing" in
[docs/node-release-recipe.md](docs/node-release-recipe.md), and it was executed
on an Ubuntu 22.04 box for 0.6.15 through 0.6.17.

```bash
# 1. build btxd from a PRISTINE checkout of btxchain/btx at the tag
#    (see the recipe for the exact cmake flags and the CUDA requirement)
# 2. stage what you built, instead of stage-node-pkg.sh:
./scripts/stage-node-pkg-linux-source.sh ~/btx/build v0.34.5
# 3. then the usual npm ci && npm run tauri build
```

Three things bite in that order, and all three are silent:

- **Build from a pristine tree.** btxd records whether its source tree was
  dirty and fails its own provenance canary if it was. That node runs, syncs
  and holds peers while not validating, and nothing on screen says so.
- **Build on the oldest glibc you intend to support.** The official binaries
  need 2.38; building on 22.04 (glibc 2.35) is why the AppImage runs anywhere.
- **Check what you got.** `getnetworkinfo.subversion` is the truth about which
  engine is running — not the app UI, and not the install directory name.

Installing the released app is still the easy answer if you only want to run a
node. Building is for people who want to verify what they run, which on a
consensus validator is a reasonable thing to want.

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
