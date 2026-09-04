# docs

- **[always-on.md](always-on.md)**: why this project exists, what running a
  node actually costs you, and what it does not promise. Start here.
- **[fleet-proposal.md](fleet-proposal.md)**: the plan, the measurements behind
  it, and the parts we have not verified.
- **[node-release-recipe.md](node-release-recipe.md)**: how a release is cut,
  including the two engine gates that must both pass first.

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
