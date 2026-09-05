# easyNode for BTX changelog

All notable changes to BTX Node are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versions track the app version
in `apps/node/package.json` / `apps/node/src-tauri/tauri.conf.json`. BTX Node
versions independently of the easyBTX miner (that changelog lives at the repo
root).

## [Unreleased]

**CI can build the untagged engine.** `engine-pin.sh` gained `engine_pin_ref`:
the commit named by `NODE_RELEASE_COMMIT` when the pin is untagged, else the
tag. `btxd-linux.yml` checks that out instead of a tag upstream does not have,
which is what it was about to do for 0.34.6. README and the release recipe say
what 0.6.18 actually shipped and how, including the two build traps and the two
gate-script traps hit on the way.

## [0.6.18] - 2026-09-05 · linux

**The node engine can keep up again.** Every 0.6.17 install that was not hand-
patched has been running an engine (v0.34.5) that connects 0.68 blocks a minute
against a chain producing 0.95: it falls behind for good, downloads blocks the
whole time, and reports itself healthy. Measured on one machine, same data,
only the binary changed, the 0.34.6 engine does 3.80. 0.6.18 bundles it.

Being honest about what that is: upstream has not tagged 0.34.6. This is the
`release/0.34.6` branch at commit `9eb4e005`, built from a pristine tree on
Ubuntu 22.04, the same build that has held the tip on the project's own
validator since 2 September and that at least one other operator on the network
runs. Both engine gates verify that exact commit. If upstream later tags 0.34.6
at a different commit, the pin is re-checked rather than renamed.

Linux only in this release; the Mac and Windows builds follow and their update
feeds are untouched until they do.

**The install now asks for enough disk to finish.** The free-space check was
set at 120 GiB from a chain that measured about 105 GB in July. The chain has
grown since, and nothing re-checked, so the gate had quietly fallen **below the
chain it exists to gate** — it stopped refusing installs that could never finish
and started waving through installs that fill the disk halfway, which is the
failure it was raised to prevent. The chain is now measured at **123.8 GiB of
blocks** (2026-09-04) and the gate is 140 GiB. If you were going to run out of
space, you now find out before the download instead of during it.

The measurement is in `docs/archival-capacity.md` with its method, and
`scripts/measure-chain-size.py` re-runs it in minutes, because this number has
been wrong in the repository in both directions and the only defence is that
re-measuring is cheap. A test now fails the build if the gate is ever set below
the measured chain again. The disk message also says GiB where it was dividing
by 1024 and printing "GB", which was making the app understate what it wanted by
about 7%.

Growth is not what it was, either: blocks left the ~1 MB MatMul mode around
height 185,000, and since 2026-08-10 the measured average is 8.4 kB a block —
about 8 MB a day, where the old comments claimed 1 GB a day.

**Your node can have a name.** Settings has a nickname field. Set one and the
nodes you connect to see `/BTX:0.34.5(yourname)/` instead of an anonymous
`/BTX:0.34.5/` — the same idea as worker names in the miner, and the first
actual mechanism behind the recognition this project offers instead of payment.
The status screen lists the names of peers you are connected to, when they have
them; today nobody on the network does, so easyNode's will be the first.

It is off by default and it is public: a name follows your node across restarts
and IP changes, which is what makes it recognisable and why it is a choice. The
box takes letters, numbers, spaces, dots, dashes and underscores, up to 24
characters, and refuses anything else rather than writing it — a name the engine
dislikes does not get ignored, it stops the node starting. Settings shows the
real user agent your peers are seeing, so you can tell the difference between a
name you have saved and a name that is live: it applies at the next node start.

**A wallet import can no longer point you at the wrong wallet.** If the default
name is already taken by a different wallet, your import goes in beside it — and
the code only adopted that new name when the call *succeeded*. But a wallet.dat
rescan takes hours and the call gives up after a minute, so failing is the normal
outcome; the app then checked whether the **old** wallet existed, found that it
did (of course it did), and showed you that one. You would have been looking at
somebody else's empty balance while your keys sat safely in the wallet next to
it. Fixed on both import paths.

**The update banner can admit it failed.** A Linux `.deb` cannot be replaced by
the built-in updater at all, and the failure was swallowed: the banner said
"Update available — downloading…" at every launch, forever. It now tells you
plainly that the automatic update could not install and where to get the build.
A failed check still stays quiet, because being offline is normal.

**Your balance stops going stale after a send.** One successful send latched the
wallet into keeping the last screen it had, so a stopped node kept showing the
old balance under "verified by your node" for the rest of the session.

**The send form prints a number it will actually accept.** "Ready to spend" was
rounded up, so typing exactly the figure the wallet showed you could be rejected
as spending too much — and the rejection quoted the same rounded number back.

**Keeper mode is installable again on a normal laptop.** The free-space check
ignored which profile you had chosen and demanded room for the whole chain even
when it was about to install a ~10 GB pruned node. Choosing Keeper on the setup
screen and being refused for 140 GiB was the single most confusing way to meet
the tier the network is shortest of. The setup screen's "Disk needed" now shows
the figure for the profile you picked, too: 20 GiB for a keeper, instead of
quoting the full node's 140 to everyone.

**The app stops calling it a menu bar on machines that do not have one.** The
first-run pitch, the close dialog, the close-behaviour setting and a button all
said "menu bar" — macOS's word — while asking Windows and Linux users to choose
a place they do not have. It now uses whatever your system calls it.

**Switching to a full node checks your disk first.** Going from Keeper to Full
means holding the whole chain, and the app used to accept the switch and then
write it into place silently at the next engine update — so a keeper who had
25 GB free could find their node quietly trying to download 124 GiB. Settings
now refuses the switch on the spot if the disk cannot hold it, and an engine
update never changes your node's prune posture unless there is room for it.

**Reclaim and Remove data now check whether something else is using your data
folder** — the miner, or a second copy of the app — and refuse instead of
deleting out from under a running node.

**"Keep computer awake" no longer pretends.** Off macOS it never did anything,
while being shown, switched on, and silently doing nothing when you toggled it.
It now says so, and points at your system's own sleep settings.

**Settings shows what your node is really providing.** A node can advertise that
it serves history while quietly having stopped, and nothing said so. That verdict
was computed but never displayed; it now sits next to the switch that controls
it, in amber when it needs you.

Under the hood: the config file that launches your node, and the file holding
your settings, are now written so they cannot end up half-written; a failed
restart can no longer leave both Start buttons disabled; and the dashboard no
longer freezes when the node accepts a connection and then stops answering.

**The node now says what it is actually providing to the network.**
`crates/btx-core/src/frontier.rs` could always answer that question and nothing
called it: the signed frontier was read only from inside the stall watchdog, on
a mirror that had already frozen. A healthy node could advertise the archive bit,
sit far enough behind the frontier that btxd had quietly narrowed it to the live
window, look completely fine, and tell nobody. Settings now shows the honest
answer, and the cost was measured rather than assumed — one
`getmatmulattestedtip` at ~10 ms against a 3-second refresher, and only on nodes
that serve attestations at all.

**The local service report can be switched on.** It was read on every tick and
written by nothing, so it could not be enabled. It writes `service-report.json`
into your data folder every few minutes and does nothing else — no network, no
upload — it records only what this node has served, plus the public nickname
if you set one. **Nothing phones home** remains true with it on, which is
why it is worded that way in Settings.

**A prune posture that survives an old datadir.** btxd loads the datadir's
`btx_rw.conf` on every start regardless of `-conf`, and a read-write setting
outranks a config-file one — so on any install carrying a remembered prune value,
neither the full nor the keeper posture actually applied. Measured on a live
validator: the conf said `prune=0`, the datadir said `prune=4096`, and 4096 won
for weeks with nothing on screen saying so. The app now re-asserts the conf's own
value on the command line, which keeps the keeper profile pruned on purpose and
the full profile un-pruned as intended.

**An opt-in Esplora front, gated before it can waste your time.** An easyNode can
serve the REST API that wallets read, but only from an unpruned archive — electrs
indexes from block files that a pruned node deletes. The app refuses the mode up
front and says which of the three cases you are in, rather than letting an index
fail hours later. `scripts/verify-esplora.sh` decides whether an endpoint is fit
to advertise, and `deploy/esplora/` carries the proxy, indexer and freshness
guardian.

**Also:** the seed list gained a consented operator node (`/BTX:0.34.6/`, at the
tip, verified before it was added); `scripts/node-observer.sh` publishes the
unattended recovery that previously existed on one machine; and the two Tauri
plugins moved forward with their JS and Rust halves kept in lockstep.

**Known and unchanged:** the pinned engine is still v0.34.5, which cannot keep up
with the chain — measured 0.68 blocks/min against a 0.95 blocks/min chain, where
0.34.6 does 3.80. The pin does not move because upstream has not tagged 0.34.6;
`release/0.34.6` exists only as a branch. The reasoning sits next to
`NODE_RELEASE_TAG` for whoever moves it.

## [0.6.17] - 2026-09-01 · mac AND linux LIVE

**One number that contains everything, so bytes and versions stay one to
one.** 0.6.16 was published minutes before the corrected seed census landed
on main, so its binaries still carry the bootstrap seed that three
independent confirmations caught serving stale branch headers, which can
wedge a fresh install's first header sync. 0.6.17 is 0.6.16 plus that
census: the stale seed is out, a consented operator node at the tip is in,
nothing else moved. The updater feed points here. 0.6.15 and 0.6.16 stay
published as history with a warning, and no feed ever offered 0.6.16 to
anyone.

**Live since 1 September, on both platforms.** The Linux release published
first, the update feed serves it, and installed Linux and WSL2 copies from
0.6.0 onward update themselves on their regular checks; the first machines
moved within hours. The engine claims here were verified again on the day of
release, on real hardware, including one full rescue of a deliberately broken
install through the live update feed alone.

**The Mac binaries landed later the same day, from this same commit.** Mac had
been on 0.6.12 since 25 August, an engine carrying the withdrawn 199,299 rule,
and the updater feed carried no darwin key at all, so no Mac copy could be
offered anything even when a build existed. Both are fixed here: the feed now
carries darwin-aarch64 and linux-x86_64 at 0.6.17, and the bundled mac engine
was verified byte identical to the official BTX v0.34.5 macOS release before it
was signed. A Mac updating from 0.6.12 refreshes the pinned snapshot to the
height 203,000 one and then backfills; expect hours, set mostly by peer
quality, and keep the wallet closed while it works.

## [0.6.16] - 2026-09-01 · never offered by any feed

**0.6.15 could not start its node on machines that ran the August mirror, and
this fixes it.** Caught within hours of 0.6.15 going live, by upgrading the
real 0.6.5 era install on our own rig rather than a clean test directory.

The engine keeps its own settings file in the data directory
(`btx_rw.conf`), and it loads that file on every start no matter what the
app passes. The mirror era app left `matmulvalidation=trusted` with one
signer in there, and BTX v0.34.5 refuses a one key mirror at startup. So on
exactly the machines that ran a node in August, which is the whole fleet,
0.6.15 provisioned the new engine and then watched it exit within five
seconds, three times, and gave up.

The fix is one honest line: the app now passes
`-matmulvalidation=consensus` explicitly instead of relying on the engine's
default. A command line value outranks the persisted file, the node starts,
and the engine itself logs that the leftover signer pin degrades to
telemetry that a stolen key cannot abuse. Fresh installs never noticed any
of this and are unaffected.

Verified on the poisoned install itself: with the flag the same engine
starts, self qualifies on the GPU, and syncs.

## [0.6.15] - 2026-08-31 · linux only, superseded by 0.6.17 within hours

**A Linux node now validates on its own GPU instead of waiting on a dead
quorum, and this is the release that reaches Linux.** Neither 0.6.13 nor
0.6.14 was published anywhere, so for every user this release carries their
changes too: the v0.34.5 engine, the 9.3 MB height 203,000 snapshot, v2
transport, and the measured peer census.

### Consensus mode instead of the single key mirror, on engines that allow it

Since the MatMul fork, every non Mac host launched its node as a 1 of 1
trusted mirror: one operator signature stood in for the GPU proof, because on
older engines that was the only mode that started at all. Two things changed
under that decision. BTX v0.34.5 starts every host and admits a capable card
by measuring it at startup, and the attestation supply the mirror depends on
went quiet in mid August, so a mirror node parks no matter how good its
hardware is.

Measured on an RTX 3060 with the exact shipped Linux package, both ways, the
same evening: in consensus mode the engine qualifies the card at startup,
reports ready with zero CPU fallbacks, and advertises the MatMul consensus
service bit; under the old mirror pin the same machine ran with its GPU idle
while btxd warned that a single stolen key could poison the node.

So on engines 0.34.5 and newer, non Mac hosts now stay in consensus mode. A
machine with a suitable NVIDIA card is a full independent validator. A
machine without one starts, follows headers, and stalls in plain sight where
the app's stall detection can name it, which is also everything the dead
quorum had to offer it. Mac routing is unchanged and stays a decision made
with Mac measurements.

### The train reaches Linux

First Linux release since 0.6.5, whose engine stopped following the chain in
mid August. The Linux build carries the same v0.34.5 engine, built from the
official source tag on Ubuntu 22.04 because the official Linux binaries need
glibc 2.38 and LTS machines do not have it. The engine, its GPU math library,
and every other library it needs travel inside the package, so nothing has to
be installed; that makes the AppImage about 445 MB. Kernels ship for GPU
generations from Turing through Blackwell (sm_75, sm_86, sm_89, sm_100,
sm_120). Linux copies from 0.6.0 onward self update to this build once the
feed carries the linux key.

## [0.6.14] - 2026-08-31 · mac, not published

**A fresh install can actually find the network now.** 0.6.13 shipped the right
engine and still could not sync: our own test install looped header presync for
four hours because every compiled peer was pruned or unreachable. Two changes,
confirmed with the BTX maintainers the same evening, close that hole.

### v2 transport is on

Every archive node on the BTX network now prefers BIP324 v2 transport. A v1
connection to one opens TCP and then dies silently during the handshake, with
no log line and no error, which looks exactly like a dead host. The bundled
node now starts with `v2transport=1`, so those peers accept us.

### The compiled peer list points at nodes that serve

The old bootstrap list was measured on 2026-08-31: of eleven peers five
connected and every one was pruned, serving no history at all. The new list
carries the maintainer vetted archive nodes plus the hosts we measured actually
serving block bytes that day. Several seeds on purpose: after the first good
connection the node learns the rest of the network by itself.

The engine stays v0.34.5 and the 9.3 MB fast start snapshot stays at height
203,000, both unchanged from 0.6.13.

Late correction before this build shipped: one seed was removed again after
three independent confirmations that it sits on a stale branch and wedges a
fresh install's header sync (valid bodies, poisoned headers, which is why it
looked healthy in our first measurement). A community operator's node at the
tip joined the list with his consent in its place. That corrected seed census
is what 0.6.15 above carries.

## [0.6.13] - 2026-08-31 · mac, not published

**This release follows the chain again, and it starts in minutes instead of
days.** Two changes, and they only work together.

### The engine no longer stops at block 199,299

0.6.12 bundled BTX v0.33.4.1, which carries the difficulty rule BTX published on
2026-08-25 and withdrew on 2026-08-27. A node running it stops at block 199,299
and cannot follow the rest of the network. This release bundles **v0.34.5**,
which sets that rule to disabled on mainnet.

We verified this rather than trusting the tag name. A fresh node on v0.34.5
synced past 199,299 to header 204,385 and began downloading blocks.

### First run downloads 9 MB instead of 452 MB

The fast start snapshot moves from height 179,000 to **height 203,000**, and the
file upstream publishes for it is **9.3 MB** rather than 452 MB. That leaves
about 2,860 blocks to catch up after loading instead of about 26,860.

Verified by downloading the asset, checking its SHA-256 against both the release
manifest and the release `SHA256SUMS`, and running a real `loadtxoutset` on a
v0.34.5 node: it returned base height 203,000 and a tip hash matching the
manifest exactly, and the node left initial block download.

### If you are already stuck

If your node sits at or just past 199,299, updating may not be enough on its own.
BTX's own note on the withdrawal says the binary alone does not reorganise a
chainstate that has already split. If your node does not start moving within an
hour of updating, use Reset in Settings and let fast start rebuild. That is now a
9 MB download, so it costs minutes.

### Honest limits, and one of them is important

**This release does not promise you reach the tip.** It closes most of the gap and
it does not close the last part for you.

Measured on an Apple Silicon Mac on 2026-08-31, after loading the snapshot:

- The network produced blocks at about 61.8 per hour over the preceding 47 hours,
  which is about one block every 58 seconds.
- That Mac validated the recent chain at about 60 blocks per hour.

Those two numbers are close enough that catching up the last few thousand blocks
can take days, and on a badly peered machine it may not finish at all. What
decides it is your peers and your machine, not this release. If you are not
gaining, attach a peer that serves block bodies.

For contrast, an operator running the same engine on an RTX 5080 measured much
higher rates. This is a real difference between platforms and we are not going to
paper over it.

Two smaller things:

- Three of the peer addresses built into this app are themselves still stuck at
  the old height. Your node will find others.
- Only Apple Silicon macOS is published on this version.

## [0.6.12] - 2026-08-25 · mac

> **Correction, 2026-08-28. Read this before the rest of the entry.**
>
> The notice below told you to update before block 199,299 because BTX changed a
> difficulty rule there. **BTX has since withdrawn that rule.** It was published
> on 2026-08-25 and withdrawn on 2026-08-27, and the engine bundled in this
> release still carries it.
>
> So this version does not carry you past block 199,299. It stops there, and it
> cannot follow the rest of the network. Some nodes stop just below that block.
> Others followed the withdrawn rule a short way past it, onto a branch that has
> since died, and those need a one-time rollback as well as a new engine.
>
> **Your coins are not affected.** A node reads the chain, it does not hold your
> keys. What is stale is the node's view, not your balance.
>
> The fix needs a BTX release that is not published yet. We are not shipping an
> engine until there is one that is clean, because every currently published
> version has a defect of its own that BTX has documented. We will ship it as
> soon as that release exists and say so here.
>
> The original text is left below exactly as it was, so the record is honest.

> Rolling out. If the Mac download on [the node page](https://easybtx.com/node)
> is still on an earlier build, this one has not reached it yet.

**Update before block 199,299.** BTX changes a difficulty rule at that height.
A node still running the old engine reads every block after it as invalid and
stops there, the same way Linux nodes stopped at 191,713 in August. The chain
is currently sitting still just below that line, so there is time, but not a
lot of it.

- **The bundled engine is BTX v0.33.4.1**, built here from the official release
  tag. It carries the EncDr stall recovery baked at 199,299. We build it
  ourselves, as we have since 0.6.2, because BTX's own Mac download expects
  libraries from a developer tool most Macs do not have. The build in here
  needs nothing that is not already on your Mac.
- **We use the .1 tag on purpose.** The plain v0.33.4 tag builds a node that
  starts, syncs, and then quietly refuses to check anything, because it fails
  its own build integrity test. The .1 release exists to fix exactly that, and
  the build shipped here passes that test.

**The newest Macs could not start the node, and now they can.** This is the
part worth reading twice.

BTX ships a reference measurement that a Mac has to match before it will act as
a full independent validator. That reference covers the M1 through the M4. It
does not cover the M5, because the M5 arrived after it was published. So on an
M5 the node refused to start at all, and the app could only report that the
node never became ready. No amount of waiting or reinstalling fixed it. This
was true of the previous release too, not something new.

An M5 now follows the chain through the signed confirmation quorum instead,
which is the same route a PC without a suitable graphics card already takes. It
still checks blocks, transactions and balances itself. It delegates one
specific proof to the operators who published a signature for it.

Two things about how that decision is made. It is measured, never guessed: your
Mac is asked to be a full validator on every start, and only the engine's own
refusal moves it. And it is reconsidered on every engine update, so the day BTX
publishes an M5 reference, your Mac goes back to being a full validator on its
own.

**A missing signing key is fixed.** BTX confirms blocks with two independent
signers, and a node needs both keys because they sign different blocks. We were
shipping one of them plus an older one, and missing the second key BTX
published on 20 August. A node missing a key rejects roughly half of what it
receives. All three keys ship now, which is safe because the threshold is one:
an extra key can only widen what the node accepts and can never cause a
rejection.

**A note on confirmation counts.** This engine restarts the network's
confirmation records under a new cryptographic context, so a node rebuilds
those records after updating. Block height and balances are unaffected.

**The screen may sit still for a few minutes after the update** while the node
reads the chain back in. It is working even when the number has not moved yet.

## [0.6.11] - 2026-08-18 · linux

> Rolling out. If the Linux download on [the node page](https://easybtx.com/node)
> is still on an earlier build, this one has not reached it yet.

**The engine train reaches Linux.** This is the Linux half of the train 0.6.8
promised, and it brings Linux up to everything Mac received the day before.

BTX changed one of its difficulty rules at block 191,714. A node built before
that change reads every block after it as invalid, so it stops at 191,713 and
stays there, however long you leave it. The node inside 0.6.5 was built before
the change, so every Linux copy stopped at the same block on the same
afternoon.

- **The bundled engine is BTX v0.33.3**, built from the official release tag on
  a Linux machine. It crosses the line and follows the recovered chain.
- **The app improvements from 0.6.7 through 0.6.10 arrive with it**: a stall
  now gets a name instead of a frozen number, the archive peer list and the
  permission lines a mirror cannot sync without ship with the app and are
  asserted on every start, and the stall watchdog can actually fire.
- **Keeper mode is available**, since this engine supports it. Note the
  standalone Keeper installer is still macOS only; on Linux, Keeper is the
  profile switch inside the app.

**One thing Linux does not get yet.** The roughly 100x first-start header fix
in 0.6.8 is our own patch, and it rides on the older engine that patch was cut
against, not on the official v0.33.3 tag this release ships. Choosing the
newest official engine was the right call for a release whose whole job is to
cross block 191,714. So a **brand new** Linux setup still has the slow header
phase from 0.6.6, where the count climbs and drops back for a while before it
settles. Updating an existing node is unaffected, and the fix reaches Linux in
the next train.

**Nothing is lost and nothing needs downloading again.** The chain already on
your disk is still good. Your node picks up where it stopped and catches up on
its own.

**The screen may sit still for a few minutes after the update** while the node
reads the chain back in. It is working even when the number has not moved yet.

**One honest note about confirmations.** This engine restarted the network's
confirmation records under a new cryptographic context, so any "confirmations
served" count begins again from zero, and a node still catching up may wait a
while for records that no one is publishing yet. The chain itself is
unaffected, and this resolves as the network republishes them.

Linux only in this release. Mac is already on 0.6.10 and Windows follows.

## [0.6.10] - 2026-08-17 · mac, URGENT: the official v0.33.3 engine

**Update now: without this engine your node stops at block 191,713 and cannot
continue.** BTX shipped an emergency consensus release today after a nine-hour
network halt; it draws a line at block 191,714 that older engines cannot cross.

- The bundled engine is now BTX **v0.33.3** (the official release commit,
  built reproducibly for Apple silicon). It crosses the line, follows the
  recovered chain, and carries every fix from today's incident.
- Heads-up shown once after updating: the network's confirmation records
  restarted under a new cryptographic context in this release, so the
  "confirmations served" numbers begin again from zero. Nothing is lost;
  the chain itself is unaffected.
- Keeper mode works on this engine, as it did on 0.6.8/0.6.9.

## [0.6.9] - 2026-08-17 · mac

**The hardening train, hardened: a 10-angle review of 0.6.7/0.6.8's new
machinery found 15 real faults; this round fixes them all.** Same engine as
0.6.8, this is the app around it, corrected. If you downloaded 0.6.8 today,
take this one instead.

- **The stall watchdog can now actually fire.** It counted an arriving
  header as "progress", and BTX mints one every ~90 seconds, so on a live
  network the 15-minute freeze window reset forever and the watchdog could
  never trigger, precisely while a mirror starved. It now keys on block
  movement while blocks lag headers. A frozen frontier with zero authority
  peers (total isolation) also classifies now, instead of being invisible.
- **The watchdog arms itself.** It used to depend on the UI poll to learn
  the node is a trusted mirror; with the window closed to the tray, nothing
  polled and the watchdog silently stood down on exactly the unattended
  nodes it guards.
- **Serving survives your config.** The app deleted a hand-added
  `matmulattestationserve=1` from the conf on every start. It now adopts a
  hand-set flag into Settings instead, and a new "Serve confirmations"
  switch controls it, independent of Keeper mode (which still implies it):
  a FULL node can serve its history too, and a full-history node that
  serves is the most valuable archive the network has.
- **The noban whitelist is no longer forever.** Archive whitelist lines
  were append-only, an address that left the census kept ban-immunity and
  download authority for life. The list now lives in a managed conf block
  rewritten each start from the shipped pins + a live DNS resolution of the
  hostname archives; your own whitelist lines are untouched.
- Archive peers are detected by service bit 31, not a name substring; a
  failed archive redial is logged as failed (and retried in 1 minute
  instead of silently burning the 10-minute budget); a stopped node no
  longer shows the previous run's stall verdict; one peer census per
  refresh now feeds the status card, the watchdog and the service report
  (the UI poll ran its own full `getpeerinfo` every 1.5 s on top).
- **Keeper (the standalone Mac installer):** the watchdog's fail-quiet stop
  now sticks (a pause marker the run wrapper honors, it used to restart
  the node within 2 minutes); uninstall refuses to delete the data folder
  under a still-flushing node; the conf carries BOTH trusted signer keys
  (one key measurably rejects ~half of all blocks, a btx-core test now
  cross-checks the installer against the app's constants); a half-downloaded
  snapshot is checksum-checked instead of trusted; the spin detector no
  longer resets its counters when a busy node fails to answer RPC; the
  installer works when invoked by relative path; reinstalls keep your
  previous conf at `btx.conf.prev`.
- Site: `/virustotal` points at the current macOS artifact again (it still
  pointed at a 0.6.9-era scan).

## [0.6.8] - 2026-08-17 · mac, the new engine, and Keeper mode switches ON

**First start in minutes instead of hours, and your node can now give the
network the thing it is shortest of.**

- **A new node engine** (btxd `1932613f`, the newest sealed state of the
  0.33.3 branch, source-built for Apple silicon). It carries a week of
  upstream stability fixes, and one of ours: we found and fixed the reason
  first start took hours. The headers phase that used to run all evening
  ("headers climbing then dropping back", 0.6.6) now completes in about a
  minute, measured at roughly 100× on the same machine.
- **Keeper mode is live on this engine.** The switch in Settings now actually
  switches: a small pruned node (~10 GB instead of ~105 GB) that serves
  signed block confirmations, verified end to end on real hardware before
  shipping, including serving records for blocks it never held.
- The engine also makes serving possible at all (older engines advertised
  and answered nothing) and makes pruned nodes safe across unclean
  shutdowns, the two fixes the Keeper switch was gated on.
- Mac only in this release. Windows and Linux stay on their last build and
  get the same engine in the next train, their updater feeds are untouched,
  so nothing breaks; they simply wait.

## [0.6.7] - 2026-08-17 · trusted-mirror hardening

**Your node now knows why a stuck chain is stuck, fixes the one cause that is
cheap to fix, and tells you the truth about the rest.**

- The app ships the archive peer list and the permission lines a trusted
  mirror cannot sync without, asserted on every start. This is the single
  most likely cause of a silent post-upgrade stall, removed.
- A stall now gets a NAME: the app reads the node's own signals and says
  whether a frozen height means missing blocks, missing signed
  confirmations, or no peer allowed to hand them over, and for that last
  one it re-dials the known archive peers itself (never a restart).
- The Mirror card warns BEFORE the height freezes when no connected peer is
  allowed to serve confirmations, and the "Helping the network" card now
  credits confirmations you served, the scarcest thing a node can give.
- New opt-ins (both off by default): serve historical confirmations to the
  network; write a local service-report.json a future dashboard can read.
  Nothing phones home.
- A footnote that changed everything upstream: the hours-long first-start
  header sync ("headers climbing then dropping back is normal", 0.6.6,
  below) turned out to be a measurable node bug, found and fixed on this
  project's own hardware at ~100×. It ships when the bundled node advances
  past `0ece8ef4`+fix; until then the 8-hour warmup patience stays.
- New alongside the app: **`keeper/`**, a one-command installer that turns
  any Apple-silicon Mac into a small pruned node that serves signed
  confirmations. Recognition-only, no keys, one-command uninstall.

## [0.6.6] - 2026-08-13 · mac, windows

**Setting up works properly again, and Windows updates now arrive on their own.**

### Setting up no longer gives up on the snapshot

A new node starts from a verified snapshot of the chain, which saves you days of
waiting. Before the app can hand that snapshot over, the node has to find its own
place in the chain first, and that can take the better part of an hour.

While it runs you will see a count of chain headers going up. **That number
climbing to a big value and then dropping back to a small one is normal**, the
node is starting over with a different computer to ask, and it may do that
several times before it settles. It is not stuck.

The app used to read those restarts as a failure. It stopped waiting, left the
snapshot sitting there unused, and let the node build the chain from the very
beginning instead. That is the difference between being ready after an evening
and not being ready by tomorrow.

It now waits for as long as that step genuinely takes, and uses the snapshot the
moment the node is ready for it. A node that really has stopped still trips the
check, and the app writes it to its log.

**If a setup already fell into this, updating repairs it.** The snapshot was
never deleted, so it is still on your disk and there is nothing to download
again, the next start picks it up.

**Otherwise this only affects setting up for the first time.** A node that is
already running found its place in the chain long ago and was never at risk.

Windows machines check blocks on the processor, the same as the machine where we
measured this, so Windows setups are the ones exposed to it. An Apple Silicon Mac
does that work itself and settles sooner, so most Mac setups never ran into it.
The fix is in both.

### Windows: updates now come to you

No Windows copy of BTX Node has ever been offered an update in the app. Older
builds asked an address that was never published; we fixed the address in 0.6.0,
but every update we published after that listed only the Mac, and then only
Linux, so there was still never an answer for Windows. Every Windows update so
far has meant coming back to easybtx.com and fetching it by hand.

Windows is on the update list from now on, the same as the Mac.

**And an update now hands over to your node properly.** Updating leaves your
running node behind for a moment, and Windows had no way to tell that node from
one another app was looking after, so it left it running. That could leave you
on the old node while the app showed you the new version. The app now recognises
its own node and hands over to the new one, so an update actually takes effect.

**If you are on 0.6.0 or 0.6.4, this one reaches you in the app**, your copy was
already asking the right place, there was simply nothing there for it.
**If you are on anything older than 0.6.0, please fetch this one by hand** from
easybtx.com. Those builds ask an address that does not answer, and nothing can
reach them until you have moved off one.

This is the first time the Windows update path carries a real release, so if
anything about it misbehaves we would rather hear about it than not.

**Block 185,000 is unchanged.** Checking blocks past that height still needs an
Apple Silicon Mac or one of the very newest graphics cards. That is the node
software's rule, not ours. A Windows node checking on the processor still stops
at 184,999, and this update does not change that. We would rather say it again
than have you read "update" and expect it to move.

Windows installers are unsigned, so Windows may show a SmartScreen warning on
first run. That is unchanged from previous versions.

### Mac

The setup fix above, plus clearer wording on the **Block checking** line: a node
on the simpler path still keeps the whole chain and shares it with other people,
and the readout now says so instead of leaving you to guess.

Your bundled node is the same one 0.6.2 introduced, so there is nothing to
re-download and your copy of the chain stays exactly where it is.

## [0.6.5] - 2026-08-12 · linux

**Linux catches up with everything Mac and Windows got, and now tells you the
truth about block 185,000.** This release brings 0.6.1 through 0.6.4 to Linux in
one go, and adds a readout that explains something we would rather you heard
from us than worked out on your own.

**Read this part before you update.** BTX changed how blocks are proven at height
185,000. Checking those new proofs needs specific hardware: an Apple Silicon Mac,
or one of the very newest graphics cards. That is the node software's own rule,
not ours. **Most Linux machines are outside it**, including every AMD card and
every NVIDIA card older than the current generation.

If yours is one of them, **your node will stop at block 185,000 and this update
does not change that.** We ran this exact build on exactly such a machine before
shipping and watched it happen: it reached block 184,999, asked for 185,000 over
and over, and stayed there. We would rather say so plainly than sell you an
update that promises to fix it.

What this version does change is the silence around it. Your node now knows where
the real end of the chain is, rather than believing 184,999 is the end of the
world, and a **Block checking** line on the status screen says which mode your
machine is in. A node that cannot go further tells you so instead of looking
perfectly healthy and going nowhere.

If your machine does have qualifying hardware, the node follows the chain past
185,000 normally.

**We are not leaving it there.** Following the chain on ordinary hardware needs a
different mode, which the BTX team is still building out. When there is a way to
do it that is safe to leave running unattended, it ships.

**What actually arrives for everyone in this release**, carried over from the Mac
and Windows versions:

- **The node recovers on its own** instead of quietly giving up on downloading
  some blocks and needing a restart to unstick.
- **Updating the app hands over to the new node cleanly.** The previous node is
  asked to stop and given time to finish writing to disk before the new one
  starts, with a retry if it is still busy, so an update never leaves you with no
  node running.
- **Quitting lets your node finish shutting down** rather than cutting it off
  after ten seconds, so it starts fast next time instead of rebuilding.
- **The Block checking readout**, described above.

**Setting up is fast again.** Finding the start of the chain can take the node
the better part of an hour, during which it looks like nothing is happening. The
app used to conclude something had gone wrong, give up on the verified snapshot
it had just downloaded, and start building the chain from the very beginning
instead. That is the difference between being ready after a coffee and not being
ready by tomorrow. It now waits properly, and uses the snapshot the moment the
node is ready for it.

**First sync still takes a while** after that. The snapshot puts you most of the
way there and the rest fills in behind it. A block height that creeps up slowly
is it working, not it stuck.

**Other rough edges we would rather name here than leave you to find:**

- **A node can stop advancing after several hours**, even with peers connected
  and ahead of it. Restarting it gets it moving again. There is a fix for this in
  the BTX team's newer work, but taking it today would stop the node starting at
  all on machines without qualifying hardware, so we have left it.
- **The background fill of older blocks can stall.** If it does, your node keeps
  following the chain normally but never becomes a complete archive, so older
  blocks stay missing. This matters only if you point a wallet or block explorer
  at your own node and ask it about history.

Also in this release:

- **Nothing to re-download.** Updating swaps the node program and leaves your
  copy of the chain where it is.
- The bundled node is the same one Mac and Windows 0.6.4 carry, built from the
  BTX team's in-progress 0.33.3 branch (commit `1e51f0d1`), because the network
  needed these fixes before a tagged release existed. We move to the official tag
  the moment it lands. It reports itself as `v0.33.2` because that branch has not
  bumped its own version string yet; that is expected.

## [0.6.4] - 2026-08-12 · mac, windows

### Windows

**Windows catches up, and your node stops being stuck at block 184,999.** This
is the first Windows update since the proof-of-work change at block 185,000,
and it is the one that gets a Windows node moving again. The node inside the
app could score the continuing chain as "not trustworthy yet" and simply never
ask other computers for those blocks, so it sat there looking perfectly
healthy, peers connected, height frozen. If your Windows node has been showing
184,999 for days, this is why, and this update is the fix.

Coming from 0.6.0, you also get everything Mac users received in between:

- **Your node tells you how it checks blocks.** A "Block checking" line on the
  status screen says whether this computer checks every block itself or leans
  on a simpler path. On Windows it will normally say the processor is doing the
  work, which keeps your node running and useful, though it can fall behind the
  newest blocks. That is expected, not a fault.
- **The node recovers on its own** instead of quietly giving up on downloading
  some blocks and needing a restart to unstick.
- **Updating the app hands over to the new node cleanly.** The previous node is
  asked to stop and given time to finish writing to disk before the new one
  starts, with a retry if it is still busy, so an update never leaves you with
  no node running.
- **Quitting lets your node finish shutting down** rather than cutting it off
  after ten seconds, so it starts fast the next time instead of rebuilding.

**Nothing to re-download.** Your copy of the chain stays exactly where it is.

Windows installers are unsigned, so Windows may show a SmartScreen warning on
first run. That is unchanged from previous versions.

### Mac

**Quitting no longer cuts the node's shutdown short.** When the app stops your
node it asks it to shut down and then waits, because a node needs up to a
minute or two to finish writing everything to disk. That wait was being applied
in one case and not the other: if the app had *adopted* an already-running node
(which is exactly what happens right after an update installs itself), quitting
gave it only ten seconds before forcing it closed. A node cut off mid-write has
to rebuild part of its state the next time it starts, which is the slow
"Verifying blocks…" wait some people saw after quitting.

Both cases now get the same full budget, so a node you quit shuts down cleanly
and starts fast next time.

## [0.6.3] - 2026-08-12 · mac

**Updating the app no longer risks leaving your node stopped.** When an app
update also carries a new bundled node (like 0.6.2 did), the freshly updated
app could try to launch the new node while the previous one was still holding
the data folder. The new node would exit immediately ("cannot obtain a lock"),
the old one wound down anyway, and the app sat on an error with no node
running until you restarted it by hand. We hit this ourselves on our own Mac
while watching the 0.6.2 rollout.

The app now does the handover properly:

- **It stops the previous node first and waits for it to finish**, including
  the disk flush at the end of a node shutdown, which can take a minute or
  two, and only then starts the new one. While that happens you see
  "Waiting for the previous node to finish shutting down…" instead of a
  silent hang.
- **It retries.** If the launch still loses the race, the app notices within
  seconds and tries again instead of giving up with nothing running.
- **It never touches a node another app is managing.** If the easyBTX miner
  (or a second copy of this app) is running the node on this machine, the
  update leaves that node alone and applies on its next natural restart.
- If your node was already running the 0.6.2 binaries, nothing is restarted,
  the app simply attaches like before.

The bundled node itself is unchanged from 0.6.2 (the BTX team's 0.33.3 work at
commit `1e51f0d1`), so there is nothing to re-download and your copy of the
chain stays exactly where it is.

## [0.6.2] - 2026-08-12 · mac

**The network moved past block 185,000, this update makes sure your node moves
with it.** Most nodes on the old code sat at height 184,999 looking synced while
the chain carried on without them: the node scored the continuing chain's
headers as "not yet trustworthy" and never asked peers for those blocks. The
bundled node advances to the BTX team's newest 0.33.3 work (commit `1e51f0d1`),
which fixes that ranking, downloads blocks in the right order from peers that
can actually serve them, and repairs a crash in the snapshot loader.

Verified on our own Mac before shipping: headers jumped from 184,999 to the
live network tip within minutes of the swap, a fresh node using the built-in
snapshot start caught up through the fork at several hundred blocks per
minute, and the first proof-of-work blocks of the new era validated on the
GPU with the canonical chain confirmed at the fork's two checkpoint heights.
One honest note: blocks after 185,000 carry the new heavier proofs, so the
final stretch of catch-up validates at whatever pace your Mac's GPU can
check them, expect that part to take a while on older machines.

- **Nothing to re-download.** Updating swaps roughly 25 MB of node program and
  leaves your copy of the chain exactly where it is.
- Like 0.6.1, the bundled node is built from the BTX team's in-progress 0.33.3
  branch, because the network needs these fixes before a tagged release exists.
  We re-pin to the official tag the moment it lands.

## [0.6.1] - 2026-08-11 · mac

**Nodes were quietly wedging, and this stops it.** The node inside the app had a
bug where it could permanently give up on downloading some blocks: an internal
marker was set while a block was being checked and never cleared if that check
was abandoned. The node then sat there looking healthy, peers connected, height
frozen, and the only cure was quitting and starting it again. If you restarted
your node to "unstick" it in the last day, this was why.

The bundled node moves to **v0.33.3**, which expires those stale markers so the
node recovers on its own, and fixes several related stalls: it no longer
schedules work past the first gap, no longer busy-loops while deferring blocks
during catch-up, and no longer deadlocks from a lock-order problem under load.

- **Nodes without a supported graphics chip are no longer boxed in.** The
  consensus tier used to be able to deadlock a processor-only node outright. It
  is now a preference rather than a hard gate, so those nodes keep moving.
- **Nothing to re-download.** Updating swaps roughly 25 MB of node program and
  leaves your copy of the chain exactly where it is.

⚠️ **About this version:** it is built from the BTX team's in-progress 0.33.3
work rather than a finished release, because the network needed the stall fix
now. It has been verified on Apple Silicon here: the node passes BTX's own
production self-check and reports itself as a full validator. We will move to
the finished release as soon as it is published.

## [0.6.0] - 2026-08-10 · mac, windows, linux

**BTX changed how blocks are proven at block 185,000, and this update carries
your node across.** The node inside the app moves from BTX v0.33.1 to v0.33.2.
A v0.33.1 node cannot check the new blocks at all: it does not go wrong loudly,
it simply stops following the chain while still looking healthy. If your node is
sitting at block 184,999, this is why, and this update is the fix. Nothing you
have already downloaded is lost.

- **Your node now tells you how it checks blocks.** A new "Block checking" line
  on the status screen says whether this machine checks every block itself, or
  checks them on the processor and may drift behind the newest ones. It reads
  the answer from the node itself rather than guessing from your hardware.
- **And it explains the busy first few minutes.** To find out what your machine
  can do, your node runs the new proof of work once at startup. That takes a
  couple of minutes and works the graphics chip hard, so the fans may spin up.
  The status now says "Checking…" while it happens, instead of leaving you to
  wonder. It runs once per start.
- **A node that stops following the chain no longer says LIVE.** If your machine
  cannot check the new proof of work, the status turns amber and says so,
  instead of showing a confident green while nothing moves.
- **Machines without a supported graphics chip keep running.** On Windows and
  Linux the node is told to check blocks on the processor. It stays useful and
  keeps serving the network, though it can fall behind the newest blocks. On
  Apple Silicon the graphics chip does the work and the node checks everything
  itself.
- **Faster first-time setup.** The bundled starting snapshot moves from block
  155,700 to 179,000, so a fresh install has 23,300 fewer blocks to catch up on.
  (Existing installs keep the chain they already have.)
- **Explorer mode stops switching itself off.** Updating the node used to
  quietly clear the Explorer setting while the app still showed it as on, so
  transaction lookups answered "not found" for transactions that existed.
- **Automatic updates now work on Windows and Linux.** Both were pointed at an
  update file that was never published, so they could never update themselves.
  They now use the same working feed as the Mac. This one release still has to
  be installed by hand on Windows and Linux; after it, they keep themselves
  current like the Mac does.

## [0.5.3] - 2026-07-15 · mac, windows, linux

**Nothing hides below the window edge anymore, especially on Windows.** The
setup progress and any setup error used to render below the visible area of the
fixed-size window, with no way to scroll: on Windows, where text runs taller,
clicking "Set up my node" could look completely frozen while the download ran
(or failed) out of sight. The screen now scrolls, the progress card slides into
view on click, and an error jumps into view the moment it happens.

- **Updates are loud now.** When a new version is found, an accent-framed
  banner appears under the header, "Update available: v0.5.x, downloading…",
  instead of a silent swap. Same automatic install, now visible.
- **Check for updates yourself.** Settings → Updates → "Check now". It answers
  either way: update found, you're on the latest, or couldn't reach the feed.
- **A setup log.** First-run setup now writes every step and any error to
  `setup.log` in the data folder, so "it seems stuck" is diagnosable instead of
  a mystery.

## [0.5.2] - 2026-07-15 · mac, windows, linux

**A first-run screen that clearly does something.** Pressing "Set up my node"
used to feel like nothing happened. Now the button turns into a live "Setting up
your node…" with a spinner and a plain-language readout of each step
("Downloading the snapshot… 34%", "Starting your node…"), a moving progress bar
that never sits dead at zero, and a clear note: this takes a few minutes, you can
leave and come back, it's ready when the screen turns green. The welcome copy is
trimmed down too, so the one thing to do is obvious. (If you're already set up,
this screen never shows.)

## [0.5.1] - 2026-07-15 · mac, windows, linux

**The app updates itself now.** BTX Node checks for a new version on launch and
every few hours after, and when one is out it downloads it, verifies the
signature, and swaps itself in on the next relaunch, no more hunting the
website for a fresh build. It's the same signed-update mechanism the easyBTX
miner uses. This is the first version that carries it, so this one you install by
hand; from here on it keeps itself current. (Everything in 0.5.0 below is
included.)

## [0.5.0] - 2026-07-15 · mac, windows, linux

Wallet polish, coins that go *ding*, and a red X that finally behaves.

- **Live updates.** The open wallet now re-asks your node every 20 seconds, so a
  confirmation ticks up while you watch instead of only when you reopen the panel.
  That's the fix for "it arrived but still says unconfirmed."
- **Transaction sounds.** A short 16-bit coin when a payment lands, a brighter
  chime when it gets its first confirmation, and a little rising blip when you
  send. Kept low (about a third volume) and decent, never annoying. There's a
  Sound toggle right in the wallet if you'd rather have quiet, and it's built
  from a tone generator, so nothing extra is bundled and nothing leaves your box.
- **Buttons where you'd reach for them.** The sent-transaction screen and your
  own address now have clear *Open in explorer* and *Copy* buttons, and every
  activity row shows a ↗ so it's obvious a click opens it on btxscan.io.
- **Closing a wallet asks first.** "Close this wallet" now confirms before it
  stops watching, and reminds you the .btxwallet file stays saved, no more
  one-click surprise. (Your keys were never deleted; now it's clear.)
- **The red X, your way.** Closing the window used to silently keep the node
  running in the menu bar, which is why quitting felt like it needed Force Quit.
  Now the X asks, keep it running in the menu bar, or quit, and can remember
  your choice (also changeable in Settings). Either way, quitting stops the node
  *without freezing*: it shows a quick "stopping safely" and exits on its own.
- **Clearer at a glance.** A running node now says **LIVE** instead of "Ready"
  (ready read like it was still waiting on something). In the wallet, money
  coming in is **green** and money going out is **red**. And the ✕ that closes
  any panel is now red, so it's obvious how to get out.

## [0.4.0] - 2026-07-15 · mac, windows, linux

The wallet grew up. It was a window you could look through; now it's a wallet
you can use.

- **Send.** Pay any BTX address straight from your node. A review step shows the
  amount and the destination before anything leaves, and **Max** sends the whole
  spendable balance (the network fee comes out of the amount, so it actually
  goes through). Your node checks the address is real before it signs anything.
- **Receive.** A fresh address whenever you want one, with a QR code to scan and
  a one-click copy. Every address you've ever been given keeps working. Handing
  out a new one per payment is what stops your payments from being tied together
  in public.
- **Activity.** Fifty transactions instead of eight, and every row is now
  clickable: it opens that transaction on the public explorer (btxscan.io) if you
  want a second pair of eyes. Your own address opens there too.
- **Still your node's answer.** Balances and history come from the full copy of
  the chain on this computer, same as before. The explorer link is the one thing
  that reaches out, it never happens on its own, and it sends nothing but the id
  you clicked.

Under the hood: sending was never a new capability. The wallet file the node
restores carries the post-quantum master seed, so the node has held full spending
keys since the wallet feature shipped, v0.3.x simply had no button. Nothing about
your keys changed here.

## [0.3.0] - 2026-07-14 · mac, windows, linux

One app, three platforms. BTX Node now runs on Windows and Linux too.

- **Windows (x64).** An installer with the full one-click experience: verified
  snapshot fast-start, "Ask your node", the optional wallet, and automatic disk
  housekeeping. The bundled BTX node binaries are built from the official
  v0.33.1 source and boot-tested on real Windows, including mining a regtest
  block, before they are allowed into the installer. Node data lives under
  `%APPDATA%\easyBTX`.
- **Linux (x86_64).** AppImage and deb builds bundling the official static
  v0.33.1 binaries. Tested on a clean Ubuntu machine.
- **Honest numbers everywhere.** The "what it costs" panel now measures memory
  on every platform; CPU shows a dash where the OS has no cheap per-process
  number (Windows).
- Small copy pass: the app says "this computer" instead of "this Mac" where it
  matters, because it might be neither.

## [0.2.3] - 2026-07-12 · mac

- The header links to network-wide stats (btxprice.com/stats) next to your
  node's own numbers.

## [0.2.2] - 2026-07-12 · mac

- **Create a wallet.** The optional wallet view can now create a fresh
  post-quantum BTX wallet inside your node and save its `.btxwallet` file, the
  same format the official BTX browser wallet uses. Still off by default.

## [0.2.1] - 2026-07-12 · mac

- Honest disk numbers: the chain measures about 105 GB today and grows roughly
  1 GB a day; the app now says so up front and checks free space before setup.
- **Remove node data** in Settings: gracefully stops the node, removes the
  chain data, and returns the app to the setup screen. Wallets are never
  touched.

## [0.2.0] - 2026-07-12 · mac

- **Ask your node.** Tap the ? and your node answers the questions people
  usually ask an explorer website: chain progress, supply so far, the next
  halving, fees, mining difficulty, any block. Every answer names its source,
  and the green dot means it came from your own verified copy of the chain.
- **Explorer mode** (optional): builds a transaction index in the background
  so you can look up old transactions.
- **Optional wallet view** (off by default): import your `.btxwallet` file and
  read balances from your own node instead of a public explorer.
- A calm **Warming** phase while the node checks its data after a restart, and
  automatic disk housekeeping on every start.
- BTX Node look: the green status core, dark calm theme, and the BTX Node
  wordmark.

## [0.1.0] - 2026-07-11 · mac

- First release: a one-click BTX full node for Apple Silicon Macs. Verified
  snapshot fast-start, live status (block height, peers, uptime, disk), menu
  bar tray, launch at login, keep awake, graceful shutdowns.
