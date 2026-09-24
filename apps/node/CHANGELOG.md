# easyNode for BTX changelog

All notable changes to BTX Node are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versions track the app version
in `apps/node/package.json` / `apps/node/src-tauri/tauri.conf.json`. BTX Node
versions independently of the easyBTX miner (that changelog lives at the repo
root).

## [Unreleased]

**Every node refuses the invalid branch of the 23 September split.** The app now
asks the node to mark block 227,313 `b28c3e84…` invalid, the one instruction
BTX's developers gave every node operator that night, as soon as the node has
seen its header. A node that checks blocks on 0.34.9 already rules that branch
out, but two kinds of node did not. A mirror, which is an M5 or a PC without an
NVIDIA driver, follows whatever its pinned signers sign, and the mirrors the
census reached were on that branch. A mirror that is on it rolls back to
227,312 and stops there until a signer it trusts signs the valid chain, which
is safer than confirming blocks the network's validators reject. And a node
that had the branch's headers but not its blocks showed "a longer chain exists
that this node cannot obtain", pointing at the invalid branch; once the branch
is marked invalid that warning stops. The mark is kept in the node's own data,
so it holds across restarts, and `EASYBTX_NODE_REFUSE_KNOWN_INVALID=0` turns it
off. The list of refused blocks holds this one block and is meant to be
emptied again once BTX ships an engine that refuses it by itself.

**A new node gets its block headers from the app's own peers first, and has
them in about a minute.** Before it keeps a single header from a peer outside
its trusted list, the v0.34.9 engine re-checks that peer's chain in a pre-check
that gets slower with every block. Measured on an M2 Pro on 23 September, one
healthy peer took 32 minutes to hand a new node its headers this way. Every
other node a new install met ran its own pre-check on the same thread, and the
network has many nodes stuck on old forks that answer. In five first runs with
the app's settings, headers reached the fast-start point about a minute after
they began arriving twice; once it took 18 minutes, and twice they were still
short of it after 20 and 30 minutes. A new node now connects only to the app's
four block-serving peers, each trusted for that one start, until its headers
pass the fast-start point: about a minute in three runs, one of them with the
first of those peers unreachable. Then it restarts once and connects to the
whole network as before; on a Mac the restart repeats the startup check, one to
two minutes. If those peers deliver nothing for five minutes, it gives up on
them and starts the usual way. Not yet run in a built app.

**A native Windows PC with an NVIDIA card follows the signed chain instead of
stalling.** The Windows engine is cross-compiled without any GPU code, so it
cannot check blocks on any card. Since 0.6.23 the app nevertheless started
every Windows PC that has the NVIDIA driver in the mode meant for checking
blocks, because it looked only for the driver, and going by the code such a
node started degraded and stalled. The app now asks whether this platform's
engine can use the card as well, and on Windows the answer is no, so such a PC
runs as a trusted mirror like a PC without the driver: it follows the chain
the pinned signers have signed and can serve history, trusting those keys
instead of checking the maths itself, and it is no longer offered the signing
role it could never use. The Linux build under WSL2 is unchanged and still
checks blocks on a capable card, which makes it the route for a Windows
machine that should validate. Not yet run on a Windows PC.

**The fast start file now comes from easyBTX's own releases.** New nodes
download the height 219,000 snapshot from a byte-for-byte copy on easyBTX's
release page instead of from the pre-release where BTX's developers publish it,
which they can change or remove at any time. A missing file would have stopped
every first run at the download step. The file is the same one, checked against
the same size and SHA-256 as in 0.6.28.

## [0.6.28] - 2026-09-23

**Corrected on 24 September: the longer branch is the invalid one.** The first
version of this entry called the other side of the 23 September split "the
losing side" and said an upgraded node would cross to the heavier branch. BTX's
developers said the opposite that night. The valid chain is the one whose block
227,313 is `d5f0e92f…`. The longer branch starts from `b28c3e84…` at 227,313,
and btxscan.io, luckypool.io and the signed-confirmation mirrors the census
reached were all on it. By their account it fails ExactReplay on both the
processor and the graphics chip, and upstream's next release, 0.34.10, will
refuse it (upstream PR #203, still open on the 23rd). The census agreed in the
one way it can: that evening every reachable node on 0.34.9 that checks blocks
itself was on the `d5f0e92f…` chain, and none was on the longer one. Two nodes
calling themselves 0.34.10, a version upstream has not released, were on the
longer branch. We checked the two hashes on a node of our own that night: the
header chain served by `109.199.124.187` and `194.93.48.158` carries
`d5f0e92f…` at 227,313, on top of the block both branches share at 227,312
(`8c36f9a6…`), where btxscan.io carried `b28c3e84…`. That it fails ExactReplay
is BTX's developers' finding, not ours. An upgraded node that checks blocks
should stay where it is, and does.

**Until confirmations are real again, do not send or accept BTX.** Anything
before block 227,313 is on both branches and is safe. A payment confirmed on
the longer branch only is on a chain the network's validating nodes reject.

**The engine moves to upstream's v0.34.9, which decides a disputed block
instead of leaving it undecided.** At 00:13 UTC on 23 September the network
split at height 227,313. What 0.34.6 does wrong, read in its source: when a
node that checks blocks on its graphics chip computes a different digest from
the one a block's header claims, it files the block as unconfirmed and leaves
it to be retried by a second qualified device, which a machine with one
graphics chip does not have, so nothing ever settles it. 0.34.9 (upstream
`2bfc9716`) recomputes such a block on the processor with the same algorithm
and rules it valid or invalid, which is how its nodes rule the longer branch
out.

**What holds the valid chain back is not the engine.** Late on the 23rd every
reachable node on the valid chain stood at 227,374. Headers for it ran past
227,440, but the blocks behind them had been mined on rented machines with no
inbound port, so no public node had them, and no node accepts a block it cannot
download. Until those blocks reach a public node, the valid chain stands still,
whatever engine a node runs.

**How fast a machine checks blocks still decides how soon it reaches the tip.**
Every block costs one full MatMul replay. A Linux box with an NVIDIA card was
measured at 3.8 blocks a minute on 0.34.6. An M2 Pro's own startup check timed
one replay at 80 to 125 seconds on 0.34.6 and on 0.34.9 alike, and in the app
it connected 0.45 blocks a minute. That is less than the chain produces, so a
Mac of that class that checks blocks falls behind on either engine. This update
does not change that; which Macs are fast enough is measured nowhere yet.

**A node that follows signatures follows whichever chain its signers sign.** An
M5 and a PC without an NVIDIA driver do not check blocks; they trust the three
signing keys pinned in this app. The mirrors the census reached on the 23rd
were on the longer branch. The signer BTX's developers name as on the valid
chain all along, `02d5efca…`, is not one of the three keys this app pins. Until
that changes, treat a mirror's confirmations after 227,313 as unproven.

The peer BTX's developers point nodes at for the valid chain, `89.85.40.184`,
is the first one this app dials. On the afternoon of the 23rd it and
`109.199.124.187` both answered on the valid chain, at 227,355.

**0.34.9 refuses to start a node that signs, so this release pins the node's
own key.** Since 0.6.26 every node that checks blocks also signs them, with a
key the app keeps in the node's folder and pins nowhere. 0.34.9 refuses exactly
that at startup, with an error about an attestation blocklist that is in fact
empty (upstream `235d39be`): its new count of usable signers includes a local
post-quantum key but not a local secp256k1 one. The app now also pins the
node's own key, which puts the engine in the state 0.34.6 reached by itself,
measured side by side on the same Mac: the same signer status in every field,
plus one extra warning. Without it, this update would have stopped every node
with signing on, which is every Mac with an M1 to M4 and every NVIDIA machine
that checks blocks, unless its owner switched signing off. The fleet guard now
checks this pairing and fails the bump without the fix.

**Tested as an upgrade on an M2 Pro, and what that did not prove.** A keeper
install shaped like 0.6.27 (engine 0.34.6, signing on) was opened with this
build. The first attempt found the refusal above. With the fix, the app
installed the new engine over the old one, btxd passed its Metal self-check
(`m4_class`, strict-device, ready), reported `/BTX:0.34.9/`, kept its signing
key and reported the same signer state as 0.34.6, started no model helper and
opened no model port. It then loaded the 203,000 snapshot itself, checked
blocks above it on the Metal GPU and signed each one with its own key. What
it cannot show is the chain it ends up on. It learned both branches' headers
and ranks the heavier one first, which is the invalid one (see the correction
at the top of this entry), but at 0.45 blocks a minute this Mac would
need about five weeks to reach 227,313 from 203,000, the base the tested build
started from. New nodes now start from 219,000 instead, about 8,300 blocks
below the split, which on the same Mac is about two weeks; the fast start
entry below says what that does and does not change.

**Built without upstream's model network.** 0.34.7 switched on a "Native Model
Network" by default: a helper, `btx-modeld`, that btxd starts by itself on port
29447 on every network interface, to fetch and serve AI models. It also needs
OpenSSL 3.5, which neither the Linux build machine (Ubuntu 22.04) nor the
Windows cross-compiler has. This app runs a node for money, so all three
engines are built with the model network compiled out: no helper, no port, no
model downloads. One trace stays: btxd writes a small resource-governor file
into a `modelnet` folder inside the node's data every two seconds, model
network or not.

**What else changes in the engine, read from the diff.** No consensus rule
moves at a height, the 203,000 bootstrap snapshot and the golden manifest are
byte-identical, a second bootstrap snapshot at 219,000 is added, and no option
the app passes was removed. A node that signs no
longer rolls its own checked chain back onto a lighter sibling because the
sibling carries a signature (`2ac77d56`); since 0.6.26 every node that
validates signs, so that fix is ours too. The block-wide count of signature
operations now includes `OP_CHECKSIGFROMSTACK` and can no longer be zeroed by
an annex (`477b4e63`, `cd4301fc`), which only matters for blocks near the
limit. Signers can now pin post-quantum ML-DSA-44 keys beside secp256k1 ones;
the app pins and counts secp256k1 keys only, so a signer that moved to ML-DSA
alone would be missing from its signer card.

**One wallet route closes.** 0.34.9 refuses `importwallet` outright
("importwallet is disabled (legacy WIF)"), so the text file `dumpwallet` writes
can no longer be imported. How the app now answers such a file is the dumpwallet
entry further down.

**Which machines check blocks themselves is unchanged, and it is still two
kinds.** The sealed golden manifest lists NVIDIA `sm_120` and Apple
`m4_class`, which btxd assigns to M1 through M4. Every other machine starts
degraded. A PC whose NVIDIA card does not reproduce the golden digest starts in
consensus mode without the consensus service bit and cannot check blocks, so it
stalls; a PC without an NVIDIA driver, and an M5, follow the signers as a
trusted mirror. The Linux engine no longer carries native code for NVIDIA's
data-centre `sm_100` cards, because upstream made that build path opt-in;
`sm_75`, `sm_86`, `sm_89` and `sm_120` are all there.

On native Windows the first of those cases is every PC with an NVIDIA driver,
whatever its card. The Windows engine carries no GPU code at all: the
cross-compiler finds no CUDA compiler, and `btxd.exe` imports no CUDA library.
The app still sends a PC that has `nvcuda.dll` down the consensus path, which
that engine cannot take, so going by the build and the app's code such a node
starts degraded and stalls. This was read from the binary and the code, not
run on a Windows PC, and it is not new in this release. On such a PC, run the
Linux build under WSL2, whose engine has the kernels, or set
`EASYBTX_NODE_TRUSTED_MIRROR=1` to make the native app a mirror.

**Every install moves.** The install key is plain `v0.34.9`, replacing
`v0.34.6-3013c2c2`, so every existing install re-provisions onto the new engine
at its first start after the update. CI built the tag commit on all three
platforms. macOS and Linux were built from a pristine tree with
`BUILD_GIT_DIRTY 0`: macOS passes upstream's release gate, links Metal and
passed a regtest smoke; Linux passed a regtest smoke with its kernels present.
Windows is the mingw cross-compile, and two of its three patches edit the
source tree before the build, so it is not built from a pristine tree, and by
upstream's own rule its build info records it as dirty. It passed a regtest
smoke on a real Windows runner. Upstream published 0.34.9's
`SHA256SUMS` unsigned this time. Our engines are built from source, so that
touches only the contributor staging scripts, which pin the archive bytes.

**New nodes start from block 219,000 instead of 203,000.** v0.34.9 carries a
second fast start base at height 219,000 beside the 203,000 one, and a new node
now loads that. It sits about 8,300 blocks below the 227,313 split, on the
stretch both branches share. Measured against the chain on 23 September, it
leaves 8,761 blocks to catch up after loading instead of 24,761, and the
download stays about 9 MB. It is fewer blocks to catch up, not less work: the
node still re-checks everything below the base in the background, so what
changes is how soon it reaches the tip. The gain is biggest on fast machines,
and a machine that cannot keep up with the chain gains nothing from it.
Upstream publishes this file on a pre-release, `assumeutxo-219000`, not with
v0.34.9 itself, so we checked it rather than trusting the name: its size and
SHA-256 match upstream's manifest and sums, and the file names block 219,000 as
`dc51220b…3fdb87c3`, the block v0.34.9 compiles in and btxscan reports at that
height. Its contents check out too: recomputed from the file itself, its coins
hash to the value v0.34.9 compiles in for that height, the check a node makes
before it accepts the file. A node that already loaded the 203,000 snapshot
keeps it and downloads nothing, because v0.34.9 carries that base unchanged.
One that downloaded the 203,000 file but never loaded it fetches the new one on
its next start. A node that loaded 203,000 and is still below 219,000 reads
Syncing after the update instead of Live until it passes 219,000. Nothing
changed on the node; it had thousands of blocks to go either way.

**A node can now serve a snapshot of the chain to new nodes.** Every new
node still bootstraps from a file compiled into the engine, and the newest
one lags the chain by thousands of blocks. BTX has had the mechanism to do
better since 0.34, four commands that export the chain state, sign it, offer
it over the network and let another node fetch and load it, and measured on
20 September across 62 reachable peers nobody used it. On 21 September one
home RTX 3060 produced, served and round-tripped the first one, with four
shell scripts and a person watching them. This release makes that a switch:
"Serve a chain snapshot" in Settings. A node that checks blocks itself and
signs exports the chain state at the tip (0.15 s, the node is not disturbed:
its lock was held for 7 ms), waits ten confirmations because the export is
taken at the unconfirmed tip and the first one ever taken was orphaned in 40
seconds, offers the matured file, refreshes it every 500 blocks, keeps the
last two, and re-offers after every node start because the offer lives in
the running process only and was lost four times in a day before that was
understood. It also re-makes the links to the mirrors after every offer: the
engine sends its service bits once, in the connection handshake, so a peer
connected before the offer never learns of it, which is why the explorer
lost sight of the first snapshot for two hours. Off by default, refused with
the reason on a node that follows attestations instead of checking blocks or
holds no signing key. What this does not do: change what any node LOADS.
Loading an attested snapshot means trusting the signers for the chain state,
and that stays a separate decision. See docs/snapshot-serve.md.

**A dumpwallet file gets a plain answer instead of an engine error.** v0.34.9,
the engine this release moves to, switches `importwallet` off under BTX's
post-quantum policy: it refuses every file with "BTX PQ policy: importwallet
is disabled (legacy WIF); use importdescriptors with P2MR". easyNode still sent
a dumpwallet text there. On the 0.34.6 engine it already failed for almost
everyone: on the descriptor wallets easyNode creates, btxd answered "Only
legacy wallets are supported by this command", shown raw, and only after the
plaintext private keys had been staged on disk. The exception was a legacy
wallet.dat restored on Windows, whose engine is built with Berkeley DB, and
v0.34.9 closes that route anyway. The file is now recognised and answered
before anything is staged or sent to the node: what the file is, that easyNode
no longer imports it, that nothing was changed, and that a .btxwallet file or
a wallet.dat from a current BTX node brings the wallet across instead. The
advice for a file the app does not recognise stops naming the dumpwallet
format.

**A legacy wallet.dat gets a plain answer instead of an engine error.**
easyNode sent a Berkeley DB wallet.dat, the legacy kind Bitcoin Core wrote
before descriptor wallets, to `restorewallet` and let the engine decide.
Measured on 23 September against the v0.34.9 macOS engine, which is built
without Berkeley DB like the Linux one: it answered "Wallet file verification
failed. Failed to open database path '...'. Build does not support Berkeley DB
database format.", shown raw. It left nothing behind, so a later import under
the same name still worked. The Windows engine is built with Berkeley DB and,
going by its source, loads the file, into a legacy wallet that cannot hold
BTX: mainnet has taken only P2MR outputs since its first block, and a legacy
wallet's keys are secp256k1 and cannot give a P2MR address. `migratewallet`,
which reads the file without Berkeley DB, is no route either. It keeps the old
keys as secp256k1 descriptors, and on v0.34.9 it fails partway through for a
wallet with an HD seed and leaves it half migrated. The file is now recognised
and answered before anything is staged or sent to the node, the same on every
engine, including one easyNode is attached to and did not provision: what the
file is, why no BTX can be in it, that nothing was changed, and that a
.btxwallet file or a wallet.dat from a current BTX node brings a BTX wallet
across.

## [0.6.27] - 2026-09-20

**Your node now tells you when it is standing on a single signer, which until
this release it could not, however many keys it pinned.** A node that follows
signed confirmations accepts a block once ONE key it trusts has signed it, so
what decides whether it keeps working is not how many keys the config lists but
how many are actually delivering. Those are different numbers and nothing here
measured the second one. btxscan.io is the worked example: four keys pinned,
552 of its last 600 blocks carried by exactly one of them, nothing at all from
the other three, and it had stopped twice that way with every other indicator
on the machine green.

The line that says so has existed since 0.6.26 and has never been shown to
anybody, because nothing in the app ever set the number behind it. Underneath
that was the real gap: the window that counts signers was only kept by a node
holding a signing key of its own. A plain mirror, which is every machine
without a capable graphics card and the node most exposed to this, measured
nothing at all. It now keeps that window whether or not it holds a key. A node
that checks proofs itself still does not, because it depends on no signer.

Two guards, because a warning that cries wolf is one nobody reads by the time
it is true. It says nothing until it has read twenty blocks, so a node that
started forty seconds ago is not told it stands on one signer, and nothing at
all until the node has caught up, because a syncing node reads a window from
below the height confirmations begin at and would report none arriving.

**Signature counts no longer freeze while confirmations are still arriving.**
Measured on 2026-09-20 during a deliberate test, with a signer switched off at
the wall to see whether the explorer depended on it: four blocks carried no
signature from a key that, ten minutes later, had signed all four. Same
heights, same hashes, no reorg. The window re-read only the newest block, so
every height below it kept whatever had arrived by the time it was first read,
and a signer's own count stayed permanently short. It now re-reads the newest
six.

Also in this release: a peer that refused every connection for five hours after
being put at the head of the list is no longer left there (#86).

Nothing about how the node validates, signs or serves changed. The engine is
unchanged from 0.6.26.

## [0.6.26] - 2026-09-17 · linux + windows (mac follows when built)

**Every node with a capable graphics card now signs confirmations for the
mirrors, so the explorer no longer depends on one home computer.** The
explorer at btxscan.io, the wallets behind it and every GPU-less node follow the
chain through signed confirmations, because they cannot check the proof of work
themselves. Four keys are pinned there and one of them signed, from an RTX 3060
in a home. On 16 September at 15:23Z that machine was switched off and the
explorer stopped at 221,448 while the network went on to 221,464 and past; a
test transaction sent through the frozen explorer was relayed and mined five
minutes later, so money moved and nobody could see it there. Seventeen archive
nodes helped nothing, because they pass signatures on and cannot make them, and
the engine has said so on every start: "single-key trusted mirror ... configure
a second independent signer". 0.6.23 gave the archive half to every GPU-less PC.
This release gives the signer half to every node that checks blocks itself.
From this release such a node keeps one signing key in its data folder, hands
it to the engine, and signs a confirmation for every block it checks. The key
is made on the machine, once, and never leaves it; the engine refuses to start
without one (measured against the shipped 0.34.6: "Cannot read MatMul
attestation signing key file"), so the app makes it rather than asking. A node
that follows signatures instead of checking blocks, which is every machine
without an NVIDIA driver and every refused Mac, cannot sign, and the setting
says so there rather than pretending. **On by default for new installs, and
turned on once for an existing node that was never asked**, the same rule as
0.6.25, with the welcome panel saying so on the next launch; a switch somebody
turned off stays off.

**Your key finds the people who pin it, on its own.** A signing key that
nobody pins signs into the void, and the only way to change that used to be a
person copying 66 characters out of a settings panel and pasting them into a
chat. That works for the handful of people who already know each other and for
nobody else, which is the opposite of what a network short of signers needs. So
a node that signs now offers its **public** key to easybtx.com every fifteen
minutes, and mirror operators read the list and choose from it. Nothing else
goes with it: the public key, a random local id, the app and engine versions,
this run's block height and peer count, and the service bits your node already
broadcasts to every peer. No wallet, no address, no private key, and no node
that isn't signing ever sends anything at all. Settings shows exactly what was
sent and when it arrived, and the switch beside it stops the offer while
leaving your node signing.

**What you are being asked for is trust, not bandwidth, and the app says so.**
Settings shows your public signing key with a copy button and this sentence
under it: a mirror that pins this key takes your node's word for the proof of
work, with no second check. Offering is not pinning. Whoever runs a mirror
decides, on their own machine, and nothing in this app can make that decision
for them. Your node already dials the explorer's mirror when signing is on, so
no port needs forwarding: a signature only helps a mirror that hears it,
signatures travel to connected peers, and a node relays only the keys it pins,
which is why this project's own signer has kept that link by hand since
2 September.

**The role card says whether the key is actually doing anything.** "Signing
key: Signing, on N of the last 100 blocks", read from the node's own stored
confirmations, not from the setting. A configured key that is on none of the
recent blocks reads as exactly that, in amber. This is the surface that was
missing on 3 September, when a key was pinned in good faith and produced
nothing for eleven days without anybody being told.

**A signer is only useful while it is up.** Closing the window while the node
is signing says, in the close dialog, that a mirror trusting only this key
stops at its last block while the node is off. Keep-awake was already on by
default. Launch at login is turned on when the welcome panel that announces
signing is closed; both switches stay in Settings.

**The witness answers one more question, for the census.** `GET
/signers/recent` on the witness (the same server behind witness-1.easybtx.com)
says which keys signed the last hundred blocks, as JSON, from a window it keeps
current at two calls per new block. easybtx.com's node census has no other way
to count live signers, and on 16 September that count was one. The two block
routes are unchanged; every address, transaction and mempool route stays a 404.

## [0.6.25] - 2026-09-16 · mac + linux + windows

**A node you already run starts helping too.** 0.6.24 turned on serving
confirmations, answering wallets and the local service report, but only for
installs made after it. Every node that already existed updated and then
contributed exactly what it did the day before. The measurement that showed the
cost: on 16 September the network census saw 59 nodes and 21 of them serving
confirmations. Thirty-eight were not, including this project's own mining
machine, which had updated itself that morning. From this release an existing
node turns on the services **it was never asked about**, once, and the app says
which on the next launch so nobody finds out later. A setting somebody switched
off stays off: the file records the difference between a choice and a silence,
and only silence is filled in. Serving the Esplora API and Keeper mode are
untouched either way, because one needs the whole 124 GiB chain and two
programs the app does not install, and the other prunes.


## [0.6.24] - 2026-09-16 · mac + linux + windows

**Your node starts helping BTX the minute it finishes installing.** A new node
used to do nothing for the network until somebody found three switches in
Settings, and almost nobody does. From this release a fresh install serves
confirmations, answers wallets and keeps a service report, and the app says so
on first run instead of doing it quietly. Serving confirmations hands other
nodes the signatures that prove old blocks, and it costs about 208 bytes a
block. It has been the network's scarcest service: the census found a single
reachable full-history archive on 17 August. Eighteen nodes have served it
since 0.6.21, which is what stopped the signing link being a single point of
failure, and is why this is now on from the start rather than waiting to be
found. Answering wallets binds
127.0.0.1 only, so no port is opened and only this computer is served, which
is enough for a wallet here to settle a fork against a node its owner runs
rather than someone else's server. The service report is a file in your data
folder and is uploaded nowhere. Every one of them is in Settings and can be
turned off. Serving the Esplora API stays off, because it needs the whole
124 GiB chain and two programs the app does not install, and Keeper mode stays
off, because it prunes and a pruned node cannot rebuild after an unclean
shutdown. **Nothing changes for a node that already exists.** The new defaults
reach a machine with no settings file, so no update starts a service on
anybody's computer, and a switch somebody turned off stays off. There is a
test for that specifically.

**The app stops telling you your peers are the problem when they are not.** A
node whose blocks fall behind its headers showed an amber card reading "the gap
is not closing: no connected peer is serving them". It had no way to know that.
Measured here on 16 September: 41 blocks behind for 574 minutes with 16 peers
connected, 7 of them tracked, the best 35 blocks ahead and delivering, while
the node's own log showed it already holding blocks it had not connected. The
card now says that only when btxd itself reports an unserved body, and
otherwise says the hold-up is on this machine rather than the network. The
sentence mattered: it sent two earlier investigations, on 6 and 13 September,
looking at routers and peer lists for a problem that was neither.

**The status card says whether the gap is actually closing.** "Still catching
up" is a promise about the future, and a node more than three blocks behind is
paced by the engine to one block per block interval, which is the one speed at
which a gap never closes. Measured on 15 September: a Mac held between 84 and
99 blocks behind for hours while that line said it was catching up. The card
now reads the trend rather than the number and says "not catching up" when the
gap is not moving, while a node genuinely grinding through a large deficit
still reads as catching up. The release gate learned the same lesson:
`observer-ok.sh` refused only a forked or silent node, and the observer calls a
node that is behind but closing healthy on purpose, so the gate would have
passed a release cut from a node 4,827 blocks behind. Its own log holds 8,576
such readings. It now also refuses on the gap itself.

**A btxd restart no longer locks the witness out of its own node.** btxd
writes a new RPC cookie on every start, and a witness that read it once kept
authenticating with the dead one. Measured 2026-09-15 on
`witness-1.easybtx.com`: btxd restarted at 11:45:44Z for the 0.6.23 engine,
from 11:46:06Z every witness request logged `HTTP 401 Unauthorized`, and the
public endpoint, which the wallet's fork check and btxscan's health check
now depend on, answered `the node did not answer` with `x-btx-freshness:
unverified` until the unit was restarted at 11:47:37Z. The binary there
predated the 8 September reload; the reload itself now compares the cookie
on disk with the credentials each request actually sent rather than with
the shared state, so eight requests refused together all recover instead of
one, it logs one line per reload with the file's mtime and never its
contents, and tests pin the 401-then-200, 401-then-401 and unchanged-cookie
sequences. The systemd template restarts the witness always, after five
seconds.

## [0.6.23] - 2026-09-15 · mac + linux + windows

**Your node says which role it actually fills, and whether that helps.** The
status card could say a node was running, at the frontier, and advertising the
archive service. It never said what the machine was actually doing for the
network: checking blocks itself or following other people's signatures,
whether a key it holds produces anything, whether any other node can reach it.
Three things went wrong in that silence. On 3 September an operator offered a
signing key; it was pinned and produced zero signatures for eleven days,
because his node runs as a trusted mirror and a mirror consumes attestations
rather than producing them. Nothing was broken, nothing was logged, and no
screen could have said "this key signs nothing here". This project's own
signer advertises the archive service while holding a key, which the engine
clamps to the last sixteen blocks; on 13 September the explorer needed one
signature from about 800 blocks back, every peer it asked refused, and it sat
frozen for twenty-one hours. And a machine without a graphics card the engine
accepts is launched in consensus mode, starts degraded, follows headers,
advertises neither the consensus nor the archive service, and stalls below the
Epoch A height while its operator believes it is helping.

A small card now reads the engine's own answers (`getmatmultrustedstatus` and
`getnetworkinfo`, both calls the app was already making) and says, one line
each: how this node validates, whether it holds a signing key and whether that
key can sign anything here, what history it serves, whether anyone has
connected to it, and where it stands on the chain, each with one plain
sentence on whether that helps other nodes. An engine that does not answer is
reported as unknown, never as "no": absence of an answer is exactly how the
pinned key went unnoticed. A node that has been up under half an hour with no
inbound connection is not yet told it is unreachable, the same grace the
"Helping the network" card already gives. The lines are decided and tested in
Rust; the screen only lays them out.

**The engine moves to upstream's tagged v0.34.6.** Upstream tagged 0.34.6 on
13 September at commit `3013c2c2`, exactly one commit past the `9eb4e005`
branch build that 0.6.18 through 0.6.22 have shipped. That restores the rule
this project set for itself when it shipped an untagged engine: ship no engine
bump until a tag exists that upstream itself calls clean AND both guards pass
on the box cutting the release. Both do, on the literal commit: the fork guard
finds the withdrawn stall-recovery height disabled on all five networks, and
the fleet guard finds a degraded consensus start allowed, the 1-of-1 mirror
refused, and the same two device classes in the sealed golden manifest as
before (NVIDIA `sm_120`, Apple M4-class).

What the one commit changes, read from the diff rather than from the release
notes: a peer that has only ever sent headers no longer counts as a source to
fetch a competing chain's block bodies from, the mining guard's deferred-reorg
window is kept armed while a reorg is pending, and SHA-256 becomes the only new
HTLC lock the wallet creates. No consensus rule moves; `chainparams.cpp` is
byte-identical to the previous pin, so the 203,000 bootstrap base and the
golden manifest are unchanged, and the manifest's seal re-verifies. CI built
the tag commit from a pristine tree on both platforms: Linux with
`BUILD_GIT_DIRTY 0` and kernels for `sm_75/86/89/100/120`, Windows through the
mingw cross-compile and its regtest smoke on a real Windows runner. Both report
`BTX daemon version v0.34.6`.

The app installs it under the key `v0.34.6-3013c2c2` rather than `v0.34.6`, and
that is the part a returning user actually receives. The install directory is
named after the key, and the app re-provisions an existing install only when
the key changes; every install since 0.6.18 already sits in a directory called
`v0.34.6` holding the old build, so keeping the name would have shipped the
tag to fresh installs only, silently. The suffix is the commit's short hash, so
the directory says which engine is inside without running it. The binary still
reports `v0.34.6`, the staged package declares that in `.btxd-version`, and
provisioning verifies the binary against the declaration, exactly as the
`v0.33.3-pr105b` key did in 0.6.1. The chain in `~/.easybtx` is untouched by
the swap.

**A PC with no graphics card now follows the chain instead of stopping at it,
and can serve the explorer's history.** Since 0.6.15 every Windows and Linux
node has started in consensus mode. On a machine with a capable NVIDIA card
that is right: the engine measures the card at startup and the node validates
every block itself, as this project's RTX 3060 has since 1 September. On a
machine with no card it was a node that helped nobody. The engine starts it
degraded, it follows headers, it stops below block 185,000 where the new proof
of work begins, and it can never advertise the archive service, because the
engine (`init.cpp:2662-2673`, identical at our pinned commit `9eb4e005`, at
the `v0.34.6` tag and on the unreleased 0.34.7 branch) grants that service only
to a trusted mirror or to a signer whose card is ready. Its owner believed it
was helping.

The role the network is short of needs neither a card nor a key. A node with
NO signing key serves attestation history with no window limit at all, where a
signer is clamped to the last sixteen blocks (0.6.22 said this about our own
signer). On 13 September the explorer sat frozen for twenty-one hours for want
of one historical signature that no peer it asked would serve. A keyless mirror
would have answered.

So the app now looks for the NVIDIA driver before it starts the node. A machine
that has it (`nvcuda.dll` on Windows, `libcuda.so.1` on Linux, both measured on
this project's 3060 box) is launched in consensus mode exactly as before. A
machine that does not is launched as a trusted mirror: the same three pinned
signer keys the Mac fallback already uses, threshold one, the
`-allowsinglekeytrustedmirror=1` the 0.34 engine requires, and the mode stated
on the command line rather than left to the engine's default, for the reason
0.6.16 learned in five seconds. It follows the signed chain, and when "Serve
signed confirmations" is on, which the engine turns on by default in this
mode, it advertises the archive service and serves history at any height. The
status card calls it "Mirror" and says what is being trusted.

The trade, in the words of the decision record
(`docs/decisions/2026-09-15-keyless-cpu-hosts-are-trusted-mirrors.md`): at
threshold 1 every pinned key is a full authority alone, so one stolen signing
key could make these nodes accept MatMul-invalid blocks; btxd itself warns
about this at init. Mirrors also freeze when signers go quiet (2026-09-05/06
incidents) rather than validating on their own. A node that stalled for certain
and served nobody becomes one that follows on signatures, can serve, and
freezes if the signers do. The 31 August reasoning that put every PC in
consensus mode rested on an attestation supply that had measured dead in
mid-August; this project's validator has signed since 1 September and
LuckyPool's node carries attestations too, so that premise no longer holds.

What this does not fix, said so nobody expects it to: a PC whose NVIDIA card
has the driver but fails the engine's startup check (an older or weaker card)
still reads as a CUDA machine, still starts in consensus mode, and still stops
below block 185,000, with the card reading "Stopped" as before. The engine's
own verdict after start is the signal that would route such a machine to the
mirror, and using it is the next step, not this one. `EASYBTX_NODE_TRUSTED_MIRROR=1`
in the environment puts it on the mirror by hand meanwhile, and `=0` on a
machine with no card is the one-flag return to the 0.6.15 through 0.6.22
posture.

Verified offline on this box against the shipped 0.34.6 engine, with exactly
the flags a card-less PC receives and no peers: the engine reaches "Done
loading" in two seconds, `getmatmultrustedstatus` reads `trusted_mirror true`,
`local_signer false`, `serves_attestations true`, three signers at threshold
one, and the node advertises the trusted-mirror and attestation-archive
service bits with the consensus bit clear; switching serving off drops the
archive bit and nothing else. Macs are untouched: an Apple Silicon Mac still
passes no mode flag and validates for itself, and the M5 fallback path is
unchanged. A PC with a capable card is untouched.

**Every update check now says what it did, on screen and on disk.** The app
checks for a new version on launch and every six hours, and until now the
result of that check went to exactly one place: the sentence beside the "Check
now" button, and only when somebody had pressed it. An automatic check that
found nothing said nothing. An automatic check that failed said nothing
either, by design, because a six-hourly banner about being offline would have
been noise. Measured 2026-09-15: this project's own signer box ran 0.6.21 for
eight hours after the feed served 0.6.22, with two six-hourly checks due in
that window, and nothing on the machine could say whether those checks ran,
failed, or ran and found nothing. The release recipe has a step called
"observe a real upgrade", and there was nothing to observe it with.

Two things now hold the answer. Every check writes one line to
`update-check.log` in the data folder, beside `setup.log` and through the same
private file helper: when, which of five outcomes (`no-update`,
`check-failed`, `found`, `install-failed`, `installed`), and a short detail
that says whether the check was automatic or pressed, the version offered, and
the error text when there was one. The file is kept under 64 KiB, so a node
that runs for years does not grow it forever. And the last of those lines is
kept in the app's settings file, from which Settings renders a permanent line
under the button: "Last check: today 14:03 — you're on the latest version",
or "couldn't check", or "v0.6.23 installed". It is rendered from the persisted
value, so it is there at launch before any check has run this session, and it
reads the same after the relaunch an install causes. The five words are one
list on both sides of the app, and a test keeps them equal; the Rust side
refuses a sixth unwritten. Writing the line can never change what the updater
does: a failure to record is a console warning, and the one place the app is
about to end its own process, the restart after an install, waits at most two
seconds for the line to land first.

The six-hourly check itself has also moved, out of the window and into the
app's Rust side. Until now the recheck was a JavaScript timer inside the
webview, and the hypothesis behind those eight silent hours, held as a
hypothesis because the only measurement is the two checks that fell due and
left no trace while the launch-time check on the same box did work right
after a relaunch, is that WebKitGTK slows or suspends the timers of a hidden
window, and this app spends its life hidden in the menu bar. The recheck now
runs on a timer nothing throttles, two minutes after launch and every six
hours after, through the same updater and into the same log with the same
five words; the check at launch and the "Check now" button are unchanged. The
log is what settles the question on the next release: six-hourly lines from a
node left in the tray confirm it, and their absence points elsewhere.

## [0.6.22] - 2026-09-15 · linux + windows (mac follows when built)

**Your node will tell you when its own view of the chain has gone stale.**
Every chain signal on the status card was derived from the peers this node
happens to have: blocks, headers, `getchaintips`, and therefore the fork
verdict too. When a node and its peers are stuck together, all of them agree
that nothing is wrong. That is not a thought experiment. On 13 September the
network's explorer sat 878 blocks behind for twenty-one hours while its own
health check reported green the entire time, because every threshold it owned
compared that node against references that were stuck with it.

A block's timestamp cannot agree with a stuck peer set: it comes from the
chain. The card now says so when the newest block this node holds is more than
two hours old, and that sentence outranks a fork warning, because a fork says
there is a better chain we cannot reach while a stale tip says we are not
following any chain at all. The check itself was already written and tested in
this repository; it had simply only ever been wired into the wallet panel.

**A node holding a signing key is no longer described as an archive serving
history.** The engine clamps a node that holds a key to the last sixteen
blocks and refuses everything older, while a node with NO key answers the full
range. The app did not know the difference, so the machine least able to serve
history was reported as "doing its job" — including this project's own signer,
which is one of the peers the explorer asked on 13 September and was refused
by. The status now names that state, and says plainly that what the network is
short of is keyless archives.

**The Esplora front's site address is a `Host` matcher**, which is documented
now rather than rediscovered: a tunnel that forwards the public hostname
unchanged matches no site block, and Caddy answers 200 with an empty body that
reads exactly like a request that never arrived. `deploy/esplora/README.md`
carries the one-line curl that settles it in seconds.

**The electrs index is no longer an unmeasured cost.** A megabyte-sized BTX
block carries one transaction of about 200 bytes and the rest is MatMul
payload, so the whole chain holds roughly 43 MB of transaction data and the
index that sits on top of it is under a gigabyte, not a fraction of 124 GiB.
That makes keeping block files on a slow disk a correct design rather than a
compromise.

## [0.6.21] - 2026-09-09 · mac + linux + windows

**Your node dials the peers it was given.** The app shipped eleven bootstrap
and archive peers, and named each of them twice — once on the command line and
once in the generated config. btxd grants exactly eight manual peer slots
(`MAX_ADDNODE_CONNECTIONS`), works through the list in order, and does not
remove duplicates from it: the code comment saying it did was simply wrong.
Three peers were therefore never dialled at all, and while a node was still
connecting — the moment a fresh start or a reconnect matters most — the repeats
consumed slots of their own. On 5 September that is why a node could sit for
thirteen minutes without ever calling the one peer that had the live chain. The
list is now built once, each peer appears once, it is capped at eight, and the
peers measured serving the live chain come first. If a peer this build ships
does not fit, the log says which, rather than letting the engine drop it
quietly.

The list itself was re-measured for this release by starting a node against it
and reading what actually connected. Two seeds that had not answered since 5
September were retired — under a cap a seat that fails is a live peer evicted,
not just a wasted dial — and LuckyPool's node joins as a live source carrying
attestations. All eight peers this build ships were measured completing a
handshake and moving real bytes: six of them serving blocks, three of those
also serving attestations, and the remaining two the network's discovery
relays, which introduce peers rather than carry chain.

**The fork warning stops calling an ordinary traffic jam a fork.** On 6
September the app reported a competing chain when what was actually happening
is that no peer had yet sent the next block's contents — it cleared itself in
47 minutes. The two look identical in the numbers, and the earlier check read
only the numbers. btxd knows the difference and says so in its log, so the app
now reads btxd's own line: when the engine reports that no connected peer is
serving the block it is waiting for, the card says the node is waiting on the
network, not that the chain has split. A genuine split is still named a split,
in the same words as before.

**Two parts of the app can no longer overwrite each other's settings.** The
node's config file is edited by several places at once — the start sequence,
the Settings screen, and the miner, which shares the same folder. Each of them
read the file, changed their copy and wrote it back, so when two overlapped one
of the edits vanished. Nothing was ever corrupted; something was simply
missing, and what went missing could be the peer list that reaches the live
chain, or the setting that keeps every block. The whole read-and-write cycle is
now held under a lock.

**The node will not start onto a disk that has no room left.** Below 2 GB free
— the point the app already paints red — starting btxd risks it running out of
space mid-write and leaving the chain database needing hours of repair. It now
declines to start and says to free some space, instead of starting and
corrupting. A disk it cannot measure never blocks anything.

**The app reconnects after btxd restarts instead of going quiet.** btxd writes
a new authentication cookie every time it starts, and the app read it once and
kept the old one, so a connection that outlived a node restart failed from then
on and looked exactly like a node that was down. It now re-reads the cookie and
retries once.

**The bootstrap snapshot is kept until the node has actually used it.** The
450 MB download was deleted as soon as btxd accepted it, which is earlier than
knowing the node can build on it. It now stays until the node has connected a
thousand blocks past the snapshot's height, and is kept regardless if btxd has
failed to rewind a block.

**A recovery restart can no longer kill a node that is working.** The restart
path stopped btxd outright and tried again, with no limit and no check that
anything was wrong — and a node replaying blocks after an unclean shutdown
looks unresponsive for ten minutes while being perfectly healthy. It now
restarts only a node that has actually exited, waits longer between attempts,
and stops after three rather than looping.

**The wallet file you export is no longer readable by other accounts on the
machine.** It is written to the Desktop with your key material in it and
inherited whatever permissions btxd's defaults gave it; it is now readable only
by you.

**"Check now" stops blaming your connection for a release that has no build for
your platform.** Single-platform releases are the normal cadence here, and a
copy on a platform the release skips is meant to stay exactly where it is. The
updater does not report that as "no update" though — it reports an error,
because it looks for your platform's download before it compares versions — and
one catch answered every check failure with "Couldn't check right now, are you
online?". On 6 September 0.6.20 went out for Linux alone and every Mac and
Windows copy that pressed the button was told to check its network. It now says
what is actually true: this release has no build for your platform, this copy
stays where it is, and downloads for every platform are on the site. A real
network failure still reads as one. (The automatic six-hourly check stays
silent either way; there is nothing to act on.)

**One release for Mac, Linux and Windows.** That is unusual here — single
platform releases are the normal cadence — and it means two long gaps close at
once. Macs were last built at 0.6.19 and sat out 0.6.20 entirely. Windows was
last built at 0.6.6, whose engine predates the 17 August consensus release and
parks at block 191,713, or 184,999 on a machine that checks blocks on the
processor; a Windows copy is offered this one directly. The route we have
measured against a GPU on Windows is still the Linux build under WSL2, and the
native build has not been measured that way.

**What has not changed: the engine, and which machines validate for
themselves.** It stays v0.34.6 at commit `9eb4e005`, as in 0.6.18 through
0.6.20. Two device classes are rows in its sealed golden manifest, NVIDIA
`sm_120` and Apple M4-class; on this engine a capable NVIDIA card also
qualifies itself by measurement at startup. A machine with no capable card
still starts, follows headers and serves history, but a processor cannot keep
up with this network's block validation, so it will not hold at the tip. None
of the fixes above change that, and the peering fix in particular does not:
dialling the right peers is what lets a node obtain the chain, never what lets
it validate the fork.

Internal: `withGlobalTauri` is off — the interface never used it.

## [0.6.20] - 2026-09-06 · linux

**Your node can settle a fork for a wallet, whatever its size.** There is a
switch for it in Settings: "Help wallets check the chain". It is off until you
turn it on, it installs nothing, and it works on every node this app can run,
including a keeper on 10 GB.

**Why it matters.** The BTX wallet checks whether it is on the right chain by
asking an independent source for the block hash at a height they both hold.
It has had exactly one source for that, and it has been stuck since before
4 September, so the check has not run at all.
Replacing it looked like it needed a full archival node: the whole 124 GB chain,
a search index on top and a graphics card to build it.

It does not. Those costs are for balances and history. Comparing block hashes
needs only the list of blocks, which every node keeps even when it stores none
of them — so a keeper on 10 GB can do this as well as an archive can.

`btx-witness` is that server. It answers two read-only routes and refuses
everything else, on purpose: a node offering to settle forks has said nothing
about whether its balances are right, and the source this replaces was wrong
about balances in exactly that way. Proven on a pruned node on 6 September,
including block hashes from heights it has not stored a byte of for months.
`deploy/esplora/install-systemd.sh` installs it, and it is now the default.

The switch answers only from this machine until you change "Who can ask" to
`0.0.0.0`, and the row says which of those it is doing rather than leaving you
to work it out. Two honest limits, both on screen: a wallet asks a witness only
when its address is in the wallet's own built-in list, so turning this on does
not by itself put your node to work; and it answers block hashes and refuses
every other question.

**A node folder that had already saved space could refuse to start, and the
app said to delete it.** Since 0.6.18 the app has stated the disk posture on
the command line, so that a folder which remembered a different one could not
silently override the app. That was a real fix. It had a worse edge: a folder
that has already deleted old blocks cannot be told to keep them all. The engine
refuses to start against it — "you need to rebuild the database using
-reindex" — and it exits before the app can reach it, so the node simply never
came up, and the only advice on screen was to remove the node data and start
again from a snapshot. The reachable case is not unusual: any folder set up by
the older installer, which saved space by default, and then carried into this
one. An app update is what restarts them all at once, which is what made this
worth finding before this release went out rather than after.

The app now reads what the folder IS rather than what the app once wrote about
it, and never asks a folder that has already saved space to keep everything. It
starts as what it is. The "Keep less on disk" row says so plainly instead of
offering a choice that was already made, and it says the part that matters for
this release: a node that saved space can still help wallets check the chain,
because that needs the list of blocks rather than the blocks. Nothing changes
for a folder that has kept everything — it is still held to keeping everything,
which is what 0.6.18 was for.

**What has not changed: the engine.** It stays v0.34.6 at commit `9eb4e005`,
as in 0.6.18 and 0.6.19 — byte for byte the same twenty-one files, checked by
extracting both published AppImages and comparing every one.

Two device classes are rows in the engine's sealed golden manifest, NVIDIA
`sm_120` and Apple M4-class. Those are the classes reproduced independently,
not the only ones that work: on this engine a capable NVIDIA card qualifies
itself by measurement at startup instead, and this project's own RTX 3060 has
run as a full consensus validator at the tip since 4 September
(`docs/gpu-qualification-rtx3060.md` is the transcript, taken while it was
signing and serving attestations). A machine with no capable card still starts
and follows headers, but a processor cannot keep up with this network's block
validation, so it will not hold at the tip.

Worth saying beside a witness feature: a node that follows reports the chain it
was handed, so several witnesses following one source are one source wearing
several hats. Comparing block hashes tells a wallet that two sources agree. It
never tells it either is right, which is why the wallet asks more than one and
why this answers the question it can answer instead of the ones it cannot.

**Esplora mode: serve wallets from your own node.** A "Serve wallets
(Esplora API)" switch in Settings runs electrs and a Caddy front beside btxd,
so the BTX wallet can read balances from a full node you run instead of from
the one explorer left. It is a gate before it is a switch: a pruned datadir is
refused with the reason (a conf that asks for pruning, a datadir that has
already pruned, or a keeper, which is pruned on purpose) and a missing
`electrs` or `caddy` is refused naming the script that builds it
(`deploy/esplora/`). Freshness is judged every 30 seconds against the chain
census on easybtx.com, never against an explorer, and the front labels every
answer `fresh`, `stale` or `unverified`; a node that is not on the heaviest
measured chain is `unverified` whatever its age. Nothing is bundled and
nothing is downloaded: the two binaries are built from the source vendored in
this repository. The front listens on localhost until you give it a name.
`docs/esplora-mode.md` has the design and the acceptance gate.

**It says which block proves it.** Freshness used to be able to say only that
nothing looked wrong. The chain census published one hash per chain, its tip,
and on this chain a tip is regularly a one-block orphan: on 6 September the
heaviest chain's published tip was a block this project's own validator held as
a side tip while its active chain ran twelve blocks past it. So a node could be
placed on a chain only by elimination. The site now publishes settled block
hashes, six or more blocks below each chain's tip where a race has resolved,
and your node is placed on a chain by asking it for one of those blocks. The
verdict names the height that proved it, so it can be checked rather than
believed.

**A first index no longer looks like a broken node.** electrs answers nothing
until its index is built, which on a full chain is hours, and Settings used to
report that as an endpoint that "did not answer". It now says the index is
building, that the node keeps working throughout, and where the log is — and
does not colour it as a problem, because it is not one.

**The preflight is asked before the switch, not after the refusal.** Opening
Settings now tells you whether this datadir can serve Esplora at all, what the
disk will cost, and whether `electrs` and `caddy` are installed, naming the
script that builds a missing one. All of that was computed before and shown to
nobody.

**For operators running a server without this app:**
`deploy/esplora/install-systemd.sh` installs the units, and refuses first — it
asks btxd itself whether the datadir is pruned rather than trusting the config
file, because a datadir's own settings outrank it. It changes nothing without
`--yes`.

**Things that were only comments are now tests.** The Caddy front had never
been started; running it found that the per-IP rate limit had never been in
force, because the directive was ordered after one that terminates routing. A
300-request burst used to be served in full. The freshness guardian had never
been executed either, though its rules were documented as identical to the
app's. Both now run against local stubs in CI, and the acceptance gate refuses
to compare an endpoint with itself, which had produced a pass that meant
nothing.

## [0.6.19] - 2026-09-06 · mac + linux

**The Mac build ships, from the same tree and the same engine commit.**
Linux 0.6.19 went live on 5 September at 22:49Z and Macs were offered nothing
after it, on an engine that parks a deep reorganisation. The Mac build is that
same release: BTX 0.34.6 at commit `9eb4e005`, built from a pristine worktree
on an Apple M5 (`BUILD_GIT_DIRTY 0`, source fingerprint
`a7bf4bd74375b2690eda5a3b13c800e52e2f853ec53983c8f27ac06b2094236b`, the value
the Linux engine was built from), passing upstream's own
`scripts/release/verify_release_btxd.py` and carrying no Homebrew load
commands. It replaces the v0.34.5 engine that 0.6.17 shipped.

Two things were measured on that Mac rather than assumed, and both correct
notes this repository carried. An Apple M5 is **not** refused by the 0.34.6
engine in consensus mode: the startup canary reports
`admission=self_qualification`, `cpu_fallbacks=0` and `ready=1`, the node
advertises `MATMUL_CONSENSUS`, and `getmatmultrustedstatus` reads
`matmul_validation_mode consensus`, `trusted_mirror false`. So the app passes
no `-matmulvalidation` and no `-allowsinglekeytrustedmirror=1` on a Mac, the
sticky `.matmul-consensus-refused` marker is never written, and the
trusted-mirror path this changelog once said an M5 would take is not the path
an M5 takes on a 0.34.5-or-newer engine. And `scripts/node-observer.sh` runs
on macOS: its only Linux assumption is the default `BTX_CLI` path, and the one
`ss` call sits in a branch that only runs when btxd is already gone. Set
`BTX_CLI` and the release gate works on a Mac like anywhere else.

The fresh-install path and the upgrade path were both walked on that Mac.
A genuinely empty datadir completed setup in seven seconds (snapshot
downloaded and SHA verified, engine provisioned, RPC up), which is the trap
the release recipe names. The real 0.6.17 datadir, untouched since
2 September, took the upgrade: the app provisioned v0.34.6 and launched it
with `-parkdeepreorg=0` and both live-chain seeds on the command line, and the
node's header chain then contained luckypool.io's own published block hashes
for heights 211,410 and 211,421, with every headers-only branch in
`getchaintips` forking at exactly its active tip. That is the shape of a node
that is behind rather than forked, which is why the fork card stayed dark:
`btx_core::fork` treats a branch that extends our own tip as lag, not a fork.
It was 4,853 blocks behind at 00:15Z on 6 September, catching up at roughly
two blocks a minute, and it was not at the tip when this shipped.

**The node follows the chain with the most work again.** Since 0.6.7 the
shipped conf parked any reorganisation deeper than six blocks
(`parkdeepreorg=1`, `maxreorgdepthpark=6`), added after the 11 August split
kept nodes off a dead branch. On 5 September the same setting would have done
the opposite: with every reachable node on the minority branch, a parked node
could never rejoin the live chain 383 blocks away without an operator running
`invalidateblock`. The app now passes `-parkdeepreorg=0` on the command line,
which outranks every conf and the datadir's remembered settings, so an updated
node that finds a live-chain peer reorganises onto it by itself. The warn
depth stays. The fork detector below is what tells you when a longer chain
exists that your node cannot get, which is the honest replacement for a node
that silently stops.

**What the split taught about the engine, recorded so the seed list is not
blamed for it.** The shipped seed `89.85.40.184` was on the live chain all
along and served the validator's entire 383-block reorganisation once it
was dialled as a manual peer at 20:23Z on 5 September. It could not help
before because the validator had banned it at 16:23Z for "aggressive
getmmattest": a consensus node asking for attestations of blocks this
node did not have, counted as hostile after 32 ignored requests
(btxchain/btx#142). Four live-chain peers were banned that way in one
afternoon. A fresh install with this engine does the same, so the fork
detector above is the honest protection until upstream changes the rule.
`docs/incident-2026-09-05-fork.md` has the log lines.

**A node on a minority branch stops looking healthy.** On 5 September the
project's own validator sat on a minority fork for hours with a green status:
`headers` 300 ahead of `blocks`, a headers-only branch of 671 blocks in
`getchaintips`, and nothing in the app said so
(`docs/incident-2026-09-05-fork.md`, easynode#35). The status now carries a
fork verdict (`btx_core::fork`), read from the node's own `getchaintips` every
30 seconds and from the headers/blocks gap every tick. A headers-only branch
more than six blocks longer than ours since the two split, or headers more than
twenty ahead of blocks for ten minutes without the gap closing, shows in amber
beside the height: "A longer chain exists (height …) that your node cannot
obtain blocks for. Your view of the chain may be behind." No guess about which
chain is right. A node catching up after time offline does not alarm, because
its gap closes. (#36)

**The observer alarms, and the release scripts listen.**
`scripts/node-observer.sh` writes `FORK` in its state column, and says so on
stderr, under the same two conditions. The new `scripts/observer-ok.sh` exits 0
only when the observer's last row is younger than five minutes and `ok`;
`build-node-feed.sh` and `publish-node-release.sh` run it first, so nothing is
signed or published from a box whose own node is on a fork, down, or
unobserved. `OBSERVER_OVERRIDE=1` skips it and is echoed loudly. (#37)

**A fresh install can reach the live chain.** Measured 5 September 19:49Z
against 1,090 known addresses: every shipped seed that answered was on the
minority branch or parked below the split, so a fresh 0.6.18 had no route to
the live chain. The one reachable node found carrying the live chain and
serving its blocks, `13.140.141.180`, joins the seed list; the two 0.32.12
fallbacks parked at 185,109 and the operator node parked at 209,447 leave it.
The engine only fetches a competing branch's bodies from manual peers, and the
seeds are manual, which is why this is the fix and not merely a hint. (#38)

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
Ubuntu 22.04, the same build that has run on the project's own validator since
2 September (it held the tip until 5 September, when a longer header chain
forking at height 210,496 appeared that no reachable peer serves blocks for;
`docs/incident-2026-09-05-fork.md` has the facts, and which chain prevails was
not settled when this shipped) and that at least one other operator on the
network runs. Both engine gates verify that exact commit. If upstream later tags 0.34.6
at a different commit, the pin is re-checked rather than renamed.

Linux only in this release. The Mac and Windows builds follow; until they do
the update feed carries no key for those platforms, so a Mac or Windows copy is
offered nothing rather than something wrong (a Mac still on 0.6.12 takes 0.6.17
from the download page by hand meanwhile).

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
nodes you connect to see `/BTX:0.34.6(yourname)/` instead of an anonymous
`/BTX:0.34.6/` — the same idea as worker names in the miner, and the first
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

**Known:** because upstream has no 0.34.6 tag there is no upstream tarball, so
the download-based contributor scripts (`stage-node-pkg.sh`,
`stage-node-pkg-linux.sh`) refuse to stage this engine; contributors build it
from source at the pinned commit (README, "Building it yourself"). The pin's
reasoning sits next to `NODE_RELEASE_TAG` for whoever moves it next.

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
