# Quick start or Full check, the history check's progress, and a calmer stale card

| | |
|---|---|
| Status | proposed 2026-09-29 for 0.7.0. The owner approved on 2026-09-29: the wizard offers "Quick start (follows signatures, ready in minutes)" or "Full check (validates every block, takes longer the first time)", switchable later; show the background history check's progress; the amber stale-tip card must not say "not following the chain" while the catch-up is converging; keep 560x780, keep the status screen as home, add no top-level screens. This document waits for the owner's review before any code |
| Date | 2026-09-29 |
| Supersedes | the wizard's single "Set up my node" with the hardware deciding alone; the stale-tip sentence on a node that is catching up |
| Leaves alone | the window size and the status screen as home; the Settings switch and the status-screen offer that change the choice later; the rule that the engine's refusal moves a Mac to following signatures; the fork and "behind the signers" messages |
| Origin | the owner's plan of 2026-09-29; btx2 and btx3 on 28 September, whose card said "The node is not following the chain" under "about 5 days to go"; the confirmed-snapshot decision of the same day, which starts every node near the tip |
| Code | `apps/node/index.html` (the wizard, one status line), `apps/node/src/main.ts` (`renderWizard`, `beginSetup`, `reflectFork`), `apps/node/src/catchup-trend.ts` (one pure function for the card), `apps/node/src-tauri/src/commands.rs` (`begin_setup` takes the choice; `NodeStatusInfo` gains the history check; the refresher projects `getchainstates`), `crates/btx-core/src/node_api.rs` (the history check, pure) |

## Context

Read on main at b008f98 and in the UI survey of 2026-09-29:

- **The wizard asks nothing.** One button, "Set up my node"
  (`index.html:94`), and `begin_setup` takes no arguments. Whether the node
  checks blocks or follows signatures is decided by `launches_as_mirror`
  (`crates/btx-core/src/node.rs:1864`): the owner's marker file
  `.follow-signatures` wins; otherwise a machine without a usable GPU follows
  signatures, an NVIDIA machine checks blocks, and a Mac checks blocks until the
  engine refuses its chip.
- **Whether a Mac's chip qualifies is known only after the engine's first
  start.** The owner's M2 Pro was refused on 29 September ("no qualified
  ExactReplay device ... local_accelerator_failure"). The app already moves a
  refused Mac to following signatures and remembers it.
- **Switching later already exists.** The Settings switch
  (`set_follow_signatures`, `commands.rs:3857`) and the status-screen offer
  (`main.ts:830`, two clicks) write or remove the marker and restart the node.
- **The history check is invisible.** The refresher reads `getchainstates`
  every 3 seconds (`commands.rs:1692`) but passes only the height on. The only
  trace is a wallet caveat ("still backfilling older history").
- **The stale card is decided in Rust and shown by the window.**
  `tip_stale_message` fires when the newest block is more than 2 hours old by
  the clock (`node_api.rs:239`) and ends "The node is not following the chain."
  (`commands.rs:3145`). The catch-up trend (`catchup-trend.ts`: converging,
  stalled or unknown, from samples over 10 minutes) exists only in the window.
- **The status screen is full above the fold.** It scrolls (`.screen`,
  `overflow-y: auto`), and a thin bar exists (`.progress-track`).

## Decision

### 1. The wizard asks one question

Above "Set up my node", two choices:

- **Quick start**: follows signatures, ready in minutes.
- **Full check**: validates every block, takes longer the first time.

One line under them says what the choice means. As the owner worded it on
30 September: "Both start from a recent signed snapshot and check the older
history in the background. Full check also checks every new block on this
computer's graphics card. You can switch later in Settings." The last sentence
shows only where Full check can be picked. (Proposed first as "Both start from
a snapshot confirmed by two node operators. ..."; changed because that is true
only once a second operator confirms, and because every node, Quick start
too, checks the older history.)

- **Which is selected first** (the owner's decision of 29 September). Full
  check on a machine with an NVIDIA card the bundled engine can use; Quick
  start on a Mac, where Full check can still be picked, because 0.34.9 does not
  move a Mac whose chip fails the engine's check to Quick start by itself;
  Quick start where the machine cannot check blocks. (Proposed first as "Full
  check where the machine may be able to check blocks, an NVIDIA GPU or a
  Mac".)
- **A machine that cannot check blocks** (no usable GPU) shows Full check
  greyed out, with the reason: "This computer has no graphics card the BTX
  engine can check blocks with."
- **A Mac whose chip the engine refuses** at first start moves to Quick start,
  as it does today, and the status screen says so once: "This Mac's graphics
  chip didn't pass the engine's check, so your node follows signatures. Nothing
  for you to do."
- `begin_setup` takes the choice and writes or removes `.follow-signatures`
  before the first start. Nothing else about setup changes shape; how the
  snapshot is found and loaded is the confirmed-snapshot decision.

### 2. The history check, one line on the status screen

While a snapshot's older history is still being checked, one line and a thin
bar under the status line: "Checking older history: 131,200 of 225,927 (58%)".
The base is the snapshot's own height, read once from its header, not the
compiled 219,000. The line disappears when the engine reports the check done.
The numbers come from `getchainstates`, which the refresher already reads, and
reach the window as one new field, `history_check`. There is no time estimate:
the check is paced by the engine and its speed says nothing reliable yet.

On a node that checks blocks and started from a signed snapshot, the role line
adds: "Checks every new block itself. Its older history is still being
checked." Once the check is done, that sentence goes.

### 3. The stale card, calm while the gap closes

One pure function in `catchup-trend.ts` decides what the card says, from the
trend and from what the node knows:

| The node | The card |
|---|---|
| is behind, and the gap is closing (trend converging) | no stale sentence; the catch-up line already says "still catching up, about N to go" |
| is behind, and the trend is not measured yet (the first minutes) | neutral, not amber: "Checking whether your node is catching up..." |
| is behind, and the gap is not closing (trend stalled) | amber, today's wording, with the pace: "...It adds about A blocks an hour while the network adds about 40." |
| does not know of any newer block, and its newest is over 2 hours old | amber, today's sentence unchanged: this is the real "not following the chain" |

The fork and "behind the signers" messages keep their place after the stale
sentence, as today.

As built, three rules sit under the table. A node that is still syncing is
judged by the blocks it adds, read over the last 20 minutes, not by the gap,
because its headers can run far ahead of its blocks: amber only when it adds
fewer than the chain's 40 an hour. The header fetch (height 0) always shows
the calm line, so a fresh install is never amber while it counts headers. And
"behind" means two or more headers beyond the tip, as on the role card: one
is a block in flight.

### 4. What stays

The window stays 560x780. The status screen stays home. No new top-level
screens: the only new overlay is Tools, decided separately. Every new line
fits the existing scrolling screen.

## Tests

- The card function: one test per row of the table, plus a node that crosses
  from converging to stalled, and one that restarts (the trend starts over).
- `history_check` from recorded `getchainstates` answers: no snapshot, a
  snapshot being checked, a finished check, and a signed snapshot's own base.
- The wizard: the choice reaches `begin_setup`; a machine with no usable GPU
  cannot pick Full check.
- By hand on a Mac, a Windows PC and a Linux machine: a fresh install with
  each choice, then a switch in Settings.

## What this does not do

- It does not add a screen or change the window size.
- It does not estimate how long the history check will take.
- It does not hide a real stall: a node whose gap is not closing still gets
  the amber card, with the numbers.

## Rollback

- The wizard: remove the two choices; `begin_setup` without a choice behaves
  as today.
- The stale card: have the function return today's sentence in every row.
- The history line: hide it; the field stays harmless.

## Choices for the owner

1. Full check selected first where the machine may check blocks.
2. No time estimate for the history check.
3. The neutral "Checking whether your node is catching up..." in the first
   minutes, instead of the amber sentence.
