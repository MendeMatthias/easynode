# docs

- **[always-on.md](always-on.md)**: why this project exists, what running a
  node actually costs you, and what it does not promise. Start here.
- **[fleet-proposal.md](fleet-proposal.md)**: the plan, the measurements behind
  it, and the parts we have not verified.
- **[node-release-recipe.md](node-release-recipe.md)**: how a release is cut,
  including the two engine gates that must both pass first.
- **[archival-capacity.md](archival-capacity.md)**: how many peers can still
  serve a block body (measured: one), and what an unpruned node actually costs
  on disk (measured: 123.8 GiB of blocks on 2026-09-04, which is more than the
  install gate used to allow). Answers issue #14. Re-measure it with
  `scripts/measure-chain-size.py` and `scripts/blockstore-census.py` rather than
  quoting the number; CI does so monthly and fails the gate check if it drifts.
- **[esplora-mode.md](esplora-mode.md)**: the opt-in Esplora API front, the
  route contract a wallet actually requires, and the gate that decides whether an
  endpoint may be advertised.
- **[appimage-size.md](appimage-size.md)**: what the ~466 MB (445 MiB) Linux download is
  made of, and why the per-architecture split that was planned does not pay.
- **[gpu-qualification-rtx3060.md](gpu-qualification-rtx3060.md)**: a full
  transcript from an RTX 3060 validating and signing on mainnet, including the
  part that does not flatter us.

## About the citations in code comments

Some comments in `crates/btx-core` cite dated measurement notes, for example
`docs/2026-08-14-mac-0.16.0-release-and-metal-rc-findings.md` or
`LEARNINGS-mac-mining.md`. Those files are not in this repository and you are
not missing anything you need.

They are the working notes behind measurements that were mostly taken while
building the **miner**, which is a separate closed product. The comments cite
them the way a paper cites a source: so the number has a provenance and is not
a thing somebody made up. The number itself, and the reason it matters, is
always written out in the comment.

If a specific measurement matters to a change you are making, open an issue and
ask. We would rather publish the relevant note than have you guess.
- **[incident-2026-09-05-fork.md](incident-2026-09-05-fork.md)**: the longer header chain from height 210496 that no peer served on release day, raw facts from the validator and what settles it (tracking: issue #35)
