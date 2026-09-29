# One Tools button: a command window, diagnostics and quick fixes

| | |
|---|---|
| Status | approved by the owner on 2026-09-29 for 0.7.0, with the four choices at the end as proposed ("ok please continue"). The design itself was approved the same day ("keep it simple: one Tools entry point instead of new screens everywhere"). Amended the same day to match the confirmed-snapshot decision: Fast-forward is for every node the app owns, and the automatic catch-up help lives there |
| Date | 2026-09-29 |
| Supersedes | "open a terminal and grep debug.log" as the support path; the watchdog's note that the fix for a gated block fetch is `getblockfrompeer`, "which nothing in this app does yet" |
| Leaves alone | the status screen as home; Settings; Ask your node; the Wallet panel; the fixed 560x780 window; every RPC the app already makes on its own; the four warnings the status screen hides on purpose (they stay hidden there) |
| Origin | the owner's plan of 2026-09-29, phase 1 (command window, Copy diagnostics, quick actions); the 28 September support session in which Nikola had to be walked through `grep "Cadence burst hold" ~/.easybtx/debug.log` to answer why btx2 and btx3 sat at about 40 blocks an hour |
| Code | new: `crates/btx-core/src/console_policy.rs` (the one function that decides), `crates/btx-core/src/diagnostics.rs` (the report and its redaction), `apps/node/src-tauri/src/tools.rs` (the commands), `apps/node/src/tools.ts` (the overlay). Changed: `apps/node/index.html` (one header button, one overlay), `apps/node/src-tauri/src/lib.rs` (registration), `crates/btx-core/src/engine_warnings.rs` (a second view that includes the hidden warnings) |

## Context

What is true today, read on main at b008f98:

- **The window cannot reach the node itself.** The content security policy
  allows connections only to the app and its IPC bridge
  (`apps/node/src-tauri/tauri.conf.json:26`), and the window's capabilities
  grant no HTTP access (`capabilities/default.json`). Every action goes through
  a Rust command, and none of them passes an arbitrary RPC through. That is
  the property this change must keep: the decision about what may run lives in
  Rust, where a bug in the window cannot skip it.
- **The node's own RPC client** authenticates with the cookie and times out
  after 60 seconds (`crates/btx-core/src/rpc.rs:90`).
- **The app may be attached to a node it did not start**, usually the easyBTX
  miner's. `NodeOwnership` and `destructive_allowed`
  (`apps/node/src-tauri/src/commands.rs:4581`, `:4604`) decide whether an
  action that stops the node is ours to take.
- **Four kinds of engine warning are hidden from the status screen on
  purpose** (`crates/btx-core/src/engine_warnings.rs`, module doc): the cadence
  hold, a deep reorg, a pre-release build, and unknown rules being signalled.
  That is right for the home screen and wrong for someone working out why a
  node is slow: the cadence hold is exactly what Nikola needed to see.
- **The watchdog knows a fix nobody calls.** For a node that has headers above
  its tip and is asking no peer for the blocks (`StallClass::BlockFetchGated`,
  `crates/btx-core/src/watchdog.rs:246`), redialling peers never helps. On
  2026-08-20 asking named peers for named blocks with `getblockfrompeer` moved
  btxscan's wedged mirror in 20 seconds and recovered all 51 blocks
  (`watchdog.rs:120`).
- **Patterns to copy**: the Ask overlay renders the node's answers as text
  only; the two-click arm with a timed disarm (`apps/node/src/main.ts:829`);
  copying with `navigator.clipboard.writeText` and saying "Couldn't copy" when
  it fails (`main.ts:904`).
- **The engine has 299 commands** (captured from v0.34.9's `help` on
  2026-09-29): 94 wallet, 45 bridge, 23 shielded, and many that change the
  node's settings, peers, chain choice or files. Among them
  `addmatmulattestationblocklist`, which changes which signers count, and
  `withdrawattestedutxosnapshot`. A list of what is blocked can never be
  complete; a list of what is allowed can.

## Decision

### 1. One button, one overlay

A wrench button in the header, right after "Ask your node", opens one overlay
called **Tools**. Nothing else in the window moves; the status screen stays
home. Inside, top to bottom:

1. **Now**: one line, the status line exactly as the home screen shows it,
   with the height, the headers and the peer count.
2. **Quick actions**: four buttons (section 2).
3. **Fast-forward**, shown only on a node that has fallen far behind a
   confirmed snapshot (section 3).
4. **Copy diagnostics** (section 4).
5. **Command window**, folded under "Command window (advanced)" (section 5).

The overlay scrolls, as every overlay does in this window.

### 2. Quick actions

- **Restart node.** Two clicks: the first turns the button into "Click again
  to restart the node", which disarms after five seconds as the follow switch
  does. It calls the same `restart_node_projected` the app already uses, so the
  node comes back with the same arguments. Only when the app owns the node
  (`destructive_allowed`); otherwise the button is off and shows the refusal
  sentence that function already writes ("Another app is running the node in
  this data folder...").
- **Fetch a stuck block.** On only when the node knows of blocks it does not
  have (headers above blocks). One click. The app picks the next missing
  blocks on the best header chain above the tip, at most 16, and for each asks
  one connected peer that has announced it: peers the app itself dials first,
  then outbound, then inbound. It then says what it asked ("Asked 2 peers for
  16 blocks") and, 20 seconds later, whether the tip moved. It never adds,
  bans or disconnects a peer. A bigger gap gets its first 16 blocks only: this
  is for a stuck tip, not for catching up. With nothing missing, or no peer
  that has the block, it says so in a sentence. Catching up from far behind
  gets automatic help of the same kind, decided in the confirmed-snapshot
  decision; this button is for the case that help does not cover.
- **Open data folder.** The existing `open_data_folder`, moved here as well as
  staying in Settings.
- **Engine notices.** Opens a list of every warning the engine reports right
  now, each in plain words, including the four the home screen hides. A hidden
  one carries the reason it is hidden ("Local pacing while catching up. The
  engine says this is not a problem with the chain."). The engine's own text
  is under each, for anyone who wants it.

### 3. Fast-forward: where it sits and how it asks

The loading itself belongs to the co-signed snapshots decision. This document
fixes only the button:

- Shown only when all of these hold: the app owns the node; a confirmed
  snapshot is available (checked as that decision requires); and it is more
  than 1,000 blocks ahead of the node's tip. This holds for a node that checks
  blocks as much as for one that follows signatures: the confirmed-snapshot
  decision loads the snapshot for both.
- The button reads "Fast-forward to block 232,000". The first click shows
  what will happen, in these words or close to them: "Your node stops, sets
  its chain data aside, loads the confirmed snapshot at block 232,000 and
  starts again. Your wallets and keys stay as they are. This takes a few
  minutes, and afterwards the node checks the older history in the
  background." The second click, within ten seconds, runs it.

### 4. Copy diagnostics

One button. Rust builds a plain-text report; the overlay shows it in a box and
copies it. Showing it first means the person sees what they are about to
share, and if copying fails the box can still be selected by hand. Nothing is
uploaded anywhere.

The report, in this order:

- the time in UTC; the app version; the engine version (`getnetworkinfo`
  subversion and the staged `.btxd-version`); the platform and how the app
  was installed (dmg, msi, AppImage, deb);
- the role: checks blocks or follows signatures, and why; signing on or off
  and, if on, the public key (it is public by design);
- the status line as the home screen shows it, and the app's phase;
- heights: blocks, headers, the best header's hash, the snapshot and
  background check heights with whether the background check is done
  (`getchainstates`), verification progress;
- chain tips: how many, and every branch other than the active one that is
  longer than one block (height, length, status), with whether each held
  branch in `known_invalid.rs` is refused on this node;
- peers: inbound and outbound counts, and per peer its version, how far it
  has synced, whether it serves history, and its connection type;
- the signed frontier (`getmatmulattestedtip`) on a node that follows
  signatures;
- every engine warning, raw and recognised, hidden ones included;
- the watchdog's current verdict and the catch-up trend, if any;
- the last 20 warning or error lines of `debug.log`, read from its last 2 MB,
  each cut at 240 characters.

Never in the report, whatever the logs contain:

- the RPC cookie; the signing key file or anything shaped like a private key
  (a WIF); wallet names, addresses, balances or descriptors;
- peer IP addresses, except peers the app itself publishes in its source
  (btxscan's mirror, the archive peers). Other peers appear as "peer 12";
- the person's home folder, written as `~` instead.

One pure function does the redaction, and its tests feed it a fake
`debug.log` carrying a WIF, the cookie line, the signing key's own text, a
home path and a stranger's IP, and check that none of them comes out.

### 5. The command window

- **What it looks like.** An input line and a Run button. Each command and its
  answer go into a history below it: the last 50 of this session, kept in the
  window's memory and never written to disk. Each entry has a Copy button;
  there is also Copy all. Up and Down recall earlier commands. Answers are
  pretty-printed JSON set as text, never as HTML, and shown up to 256 KB with a
  note that Copy takes the whole answer.
- **What it talks to.** The node the app is connected to, including a node it
  is attached to. Everything that runs freely only reads, and the two actions
  behind a confirm click change nothing that survives a restart, so neither
  harms another app's node.
- **How a line is read.** `method arg arg`, split on spaces, with quotes for an
  argument that has spaces. Every allowed method has a fixed argument shape in
  Rust (the table below). An argument that does not fit is refused with the
  shape it expects. No raw JSON parameters, no named parameters, no batches.
- **The one function that decides.**
  `console_policy::decide(line) -> Run(call) | Confirm(call, sentence) | Refuse(sentence)`,
  pure and in `btx-core`. It refuses anything it does not recognise. The method
  name is lowercased first, and anything that is not plain letters (`a-z` and
  `_`) is refused before any lookup.
- **The confirm click cannot be skipped.** For a confirm-class call, Rust
  keeps the parsed call and returns a one-time token that lives 30 seconds.
  The second click sends back only the token. A reused, expired or unknown
  token is refused. So even a bug in the window cannot turn a confirm-class
  call into a free one.
- **`help`** on its own lists what this window can run. `help <command>`
  returns the engine's own help text for any command, which is harmless text.

| Class | Commands and their argument shape | Why |
|---|---|---|
| Runs freely | `getblockchaininfo`, `getchaintips`, `getpeerinfo`, `getnetworkinfo`, `getmempoolinfo`, `getmatmulattestedtip`, `getmatmultrustedstatus`, `uptime` (no arguments); `getblockhash <height>`; `getblockheader <hash> [verbose]`; `getblock <hash> [verbosity 0-3]`; `getrawtransaction <txid> [verbosity 0-2] [blockhash]`; `gettxout <txid> <n> [include_mempool]`; `help [command]` | Read only, and cheap |
| Runs freely, proposed additions | `getchainstates`, `getblockcount`, `getbestblockhash`, `getconnectioncount` (no arguments) | Read only; `getchainstates` is the one way to see a snapshot node's two heights |
| Confirm click | `addnode <ip[:port]> onetry` (an IP literal, not a hostname; `onetry` only) | One connection attempt, forgotten at restart. "Try one connection to 1.2.3.4? Nothing is saved." |
| Confirm click | `getblockfrompeer <hash> <peer_id>` | Asks one peer for one block. "Ask peer 7 (/BTX:0.34.9/) for block 8240c62e...?" |
| Refused: `stop` | | "Use Stop node, which shuts down in the right order." |
| Refused: `invalidateblock`, `reconsiderblock`, `preciousblock` | | "The app decides which branches this node holds. Changing that by hand fights the app and can strand the node." |
| Refused: `setban`, `clearbanned`, `disconnectnode`, `setnetworkactive`, `addnode` other than `onetry` | | "The app manages this node's peers." |
| Refused: `pruneblockchain`, `setprunelock` | | "Pruning is set in Settings." |
| Refused: `loadtxoutset`, `loadtxoutsetattested`, `dumptxoutset`, `dumptxoutsetattested`, `signutxosnapshotmanifest`, `offerattestedutxosnapshot`, `withdrawattestedutxosnapshot`, `fetchattestedutxosnapshot` | | "Snapshots go through Fast-forward and Serve a chain snapshot, which check them first." |
| Refused: `addmatmulattestationblocklist`, `clearmintedattestation`, `submitmatmulattestations`, `submitmatmulrefutation` | | "This changes which signatures the node trusts. That only changes with an app update." |
| Refused: every wallet, shielded and bridge command (`dumpprivkey`, `dumpwallet`, `backupwallet`, `exportpqkey`, `dumpmasterprivkey`, `sendtoaddress`, `send`, `sendall`, `z_*`, `bridge_*` and the rest) | | "Wallet keys and payments stay in the Wallet panel." |
| Refused: anything that reads or writes a file (`savemempool`, `importmempool`, `savefeeestimates`, `importwallet`, `restorewallet` and the rest) | | "This window does not read or write files." |
| Refused: everything else | | "This window runs a short list of read-only commands. `<name>` isn't one of them. Type help to see them." |

The named refusals exist only to give a better sentence. Safety comes from the
default: a command is refused unless it is in the first four rows.

## Tests

- **The policy.** One test per allowed command with a valid line; one per
  confirm-class command showing it returns Confirm and never Run; one per named
  refusal. Sabotage cases: `InvalidateBlock` and `STOP` (case), a zero-width
  space inside a name, `stop;getblockcount`, `getblock <hash> 2 extra`,
  `addnode 1.2.3.4 add`, `addnode evil.example onetry`,
  `addnode 1.2.3.4 onetry true archive`, `getblockhash -1`, JSON pasted as an
  argument.
- **The fixture.** The 299-command list captured from v0.34.9 is checked in.
  A test runs every name through the policy and asserts that only the allowed
  and confirm rows come out as anything but Refuse, and that every named
  refusal is a real command. An engine bump that adds a command then needs a
  new fixture, and the test shows what changed.
- **The confirm token.** Used twice: the second is refused. Expired: refused.
  Made for one call and sent with another: impossible by construction, because
  the token carries no call; the test checks the stored call runs unchanged.
- **Redaction**, as in section 4, with a sabotage case for each item on the
  never-list.
- **Fetch a stuck block**: peer choice as a pure function over a recorded
  `getpeerinfo`, including no qualifying peer and a gap larger than 16.
- **Against a real engine**: behind the same opt-in as the existing
  shipped-engine tests, a regtest btxd answers every row of the table through
  the real command path.
- **By hand**, on a Mac, a Windows PC and a Linux machine: open Tools, run
  each allowed command, try each refused one, copy diagnostics into a text
  editor and read it for anything private, restart the node from Tools.

## What this does not do

- It does not pass arbitrary commands through, and it does not add a way
  around the policy for "advanced users". Someone who wants every command
  already has `btx-cli`.
- It does not show or copy the RPC cookie, the signing key, wallet keys or
  wallet contents.
- It does not upload diagnostics. Copy only; the person chooses where to paste.
- It does not become a log viewer. Twenty lines, for the question at hand.
- It does not add a top-level screen or change the window size.
- It does not decide what the app does on its own. The automatic catch-up
  help, which also uses `getblockfrompeer`, is decided in the
  confirmed-snapshot decision.

## Rollback

- Remove the header button in `index.html`. The overlay and commands then
  have no way in, and nothing else depends on them.
- Any single command: move it out of the allowed rows in `console_policy.rs`.
  The fixture test shows the change.

## Choices for the owner

These are mine, not the approved design's. Say if any should go the other way.

1. The four read-only additions (`getchainstates`, `getblockcount`,
   `getbestblockhash`, `getconnectioncount`).
2. **Fetch a stuck block** takes one click, not two: the app chooses the
   blocks and peers, it asks for at most 16 blocks, and it changes nothing
   lasting. In the command window, `getblockfrompeer` still asks first.
3. Peer addresses stay out of diagnostics except the app's own public peers.
4. The command history lives only in memory and is gone when the app closes.
