# Contributing to easyNode

Thank you for looking. The most useful thing you can do is run it and tell us
what went wrong on hardware we do not own.

## What is most wanted right now

- **Windows.** Not that there is no build: there is one, and that is the
  problem. `BTX-Node_0.6.6_x64-setup.exe` shipped signed on 2026-08-13 and was
  the last one. The platform code, the NSIS packaging and the Win32 module are
  all still here and still compile. What stopped is releasing. The update feed
  carries no Windows key, so anyone who installed then receives nothing and has
  no route forward. Picking that back up is real work and a real contribution.
- **Linux packaging** beyond the AppImage.
- **Hardware truth.** The app tries to tell a person what their machine can
  usefully do for BTX. On machines we have never seen, it may be wrong. A report
  saying "it said X, my machine is actually Y" is a genuinely valuable issue.
- **Plain language.** If a screen confused you, that is a bug. Say so.

## Sign your commits off

We use the Developer Certificate of Origin. Add `-s` to your commit:

```bash
git commit -s -m "your message"
```

That appends a `Signed-off-by` line, which is you saying you have the right to
submit the code under the MIT licence. There is no CLA and you keep your
copyright.

## Where releases come from

This repository. easyNode is built and released from here, and the process is
written down in [docs/node-release-recipe.md](docs/node-release-recipe.md) so
that it is auditable rather than folklore.

Two gates in `scripts/` must both pass before a release changes which `btxd`
users run:

```bash
./scripts/check-engine-tag.sh          # does this engine follow the majority chain
./scripts/check-engine-fleet-ready.sh  # can the fleet actually start on it
```

Neither needs a secret and both are worth running if you touch the engine pin.
They exist because an engine that passes one and fails the other is exactly the
release that strands people, and that has happened.

## Before you open a pull request

Run the tests. CI runs exactly these on your pull request, and both pass on a
fresh clone without the staging step:

```bash
cd apps/node && npm test              # the web suite, under a second
cd crates/btx-core && cargo test      # the core suite, about 5 seconds
```

To see the UI without building anything native, `cd apps/node && npm run dev`
serves it at http://localhost:1430. Node actions error there because there is no
Tauri shell behind the page, but every screen and string is editable.

For the full app:

```bash
cd apps/node
npm install
./scripts/stage-node-pkg.sh   # fetches and verifies the bundled node, required
npm run tauri dev
```

Rust and the Tauri prerequisites are needed. `crates/btx-core` builds as a path
dependency, so there is no separate step for it.

If you skip the staging script the build fails with `glob pattern
resources/node-pkg/**/* path not found`. That is expected: the bundled node
binaries are fetched from the public upstream release rather than committed, and
`tauri.conf.json` declares that directory as a bundle resource. To touch only
the Rust library you can run `cd crates/btx-core && cargo check`, which needs no
staging at all.

Please keep the diff to one subject. A pull request that fixes a bug and also
reformats a file is much harder to review than two pull requests.

## Things that will be sent back

**Anything touching the trust anchors.** `pubkey` and `endpoints` in
`apps/node/src-tauri/tauri.conf.json` decide which packages an installed app will
accept and where it looks for them. The engine pin in
`apps/node/src-tauri/src/commands.rs` decides which `btxd` every user runs. These
are protected by CODEOWNERS and there is no legitimate contribution that changes
them.

**Anything that makes the app less polite to the network.** BTX nodes ban a peer
after a number of consecutive ignored attestation requests, and free probes count
toward it. A change that increases how often easyNode asks its peers for things
can get a lot of honest people banned at once. If your change touches request
frequency, say so explicitly in the pull request and expect questions.

**Anything that promises a user a reward.** There is no token and no payment
here. Contribution is recorded, and nothing is owed to anybody.

**Anything that puts a private key in the UI layer.** Seed phrases and private
keys do not travel to the web view. Ever.

## Security problems

Do not open a public issue. See [SECURITY.md](SECURITY.md).

## Where the node itself is developed

`btxd` is upstream at [btxchain/btx](https://github.com/btxchain/btx). Consensus
behaviour, attestation logic and anything about how the chain works belongs
there, not here. easyNode installs and supervises that node; it does not modify
it.

## A note on the name

The MIT licence covers the code. It does not grant the easyNode or easyBTX name
or logo. Forks are welcome and should ship under a different name, so that a user
can always tell whose build they are running.
