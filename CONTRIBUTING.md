# Contributing to easyNode

Thank you for looking. The most useful thing you can do is run it and tell us
what went wrong on hardware we do not own.

## What is most wanted right now

- **Windows.** There is no native Windows build yet. This is the biggest gap.
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

## Before you open a pull request

```bash
cd apps/node
npm install
npm run tauri dev
```

Rust and the Tauri prerequisites are needed. `crates/btx-core` builds as a path
dependency, so there is no separate step for it.

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
