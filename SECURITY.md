# Security policy

## Reporting a vulnerability

**Do not open a public issue for a security problem.**

Use GitHub's private vulnerability reporting on this repository: the **Security**
tab, then **Report a vulnerability**. That opens a private thread visible only to
the maintainer, and it works without either of us publishing an email address.

If that is unavailable to you, open a public issue saying only that you have
something to report and asking for a private channel. Do not include details.

## What to expect

easyNode is maintained by one person. These are deliberately numbers that can be
met rather than numbers that sound good:

- **Acknowledgement within 5 days.** Often much sooner.
- **An assessment within 14 days** of the acknowledgement: whether we agree it is
  a vulnerability, and roughly what we intend to do.
- **No fixed fix deadline.** A single maintainer cannot honestly promise one. We
  will tell you what we are doing and when we expect it.

There is no bug bounty. We will credit you in the release notes unless you would
rather we did not.

## What is in scope

- The easyNode app in this repository, and `crates/btx-core`.
- The update path: anything that would let a package the release key did not sign
  reach a user, or point the updater at a host we do not control.
- The wallet path: key handling, the send flow, backup and restore.
- Anything that would cause a user's node to harm the BTX network, for example by
  getting a large number of honest peers banned.

## What is out of scope

- The BTX node itself. `btxd` is upstream: report those at
  [btxchain/btx](https://github.com/btxchain/btx). If you are unsure which side a
  problem is on, report it here and we will route it.
- The easyBTX miner, which is a separate and private product.
- Anything requiring physical access to an already unlocked machine.

## The trust anchors, so you know what to look at

The in-app updater installs only packages signed by the easyNode release key. The
public half of that key and the update endpoint are both in
`apps/node/src-tauri/tauri.conf.json`, and both are protected by CODEOWNERS. If
you find a way to make the app accept an unsigned or differently signed package,
that is the most serious class of bug in this repository and we want to hear
about it first.
