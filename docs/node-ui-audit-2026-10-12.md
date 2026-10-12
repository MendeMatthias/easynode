# easyNode UI and UX audit, 12 October 2026

Every screen state of the node app (0.7.11), looked at in a browser through
the new preview (`npm run preview:states` in `apps/node`, see the end), with
the backend's real sentences. The question each time: does the screen tell a
person the truth at a glance, and can they do the obvious next thing without
scrolling or guessing?

Fixes that were safe to make are in
[easynode #189](https://github.com/MendeMatthias/easynode/pull/189). The
rest are below, ranked, with a proposal each. Items marked **needs a
decision** change product copy or behaviour on purpose and should not be
done without the owner.

## Fixed in #189

| # | What was wrong | What it does now |
|---|---|---|
| 1 | **Green LIVE over a stuck node.** The headline said "Your node is live and helping the network" on a node following signatures that had sat on block 245,498 for six hours, while the cards under it said "The node is not following the chain", BEHIND, and Node mode "Needs attention". The compact light and the Tools panel said the same LIVE. This was the real state of every follower on 12 October. | One tested function (`apps/node/src/headline.ts`) decides the badge, the line and the light, and only repeats what a card says: NOT FOLLOWING (the watchdog's 15 minute stall verdict, or a tip over 2 hours old), BEHIND (fork or behind-signers sentence), DIFFERENT BRANCH (no other operator holds this node's block), and a softer LIVE line ("other sources have newer blocks") when only the agreement row says so. The badge turns amber or red. |
| 2 | The agreement row could be about a source ("api.btxscan.io/witness is behind the others") or about this node, and nothing on the wire said which. | `btx_core::chain_agreement` adds `this_node: behind | differs`, read off the branch the decision took. The headline only reacts to rows about this node. |
| 3 | A stopped or failed node: the headline line was empty and the error and Start node sat at the bottom, below every card. | Error card and Start node directly under the headline; the line says "What happened is just below". |
| 4 | "Global network stats" opened btxprice.com/stats, which resolves and times out (measured 02:30 UTC). Its arrow wrapped onto its own line. | Opens easybtx.com/stats; arrow inline. |
| 5 | Panels: Escape did nothing, focus stayed behind the dimmed screen, Tab walked out of the dialog. | Escape closes the top panel through its own close button, focus moves in and back, Tab stays inside. The close dialog and welcome panel need an answer, so Escape leaves them. |
| 6 | **Bug:** with Ask, Tools or What's next open, the window's close button opened "Close BTX Node?" *behind* the panel, so the click seemed to do nothing (same z-index, earlier in the page). | The close dialog paints above every panel and focuses "Keep running". |
| 7 | About said "No wallet" (there is one) and "menu bar" on Windows; What's next listed Ask (shipped in v0.2) and "following the chain on any machine" (shipped as Quick start) as plans. | Both describe today's app. |
| 8 | "Node version v0.34.15" next to "v0.7.11" in the header read as two app versions. | The tile is "BTX engine". |
| 9 | Ask's "Start node" failed silently (console only). | Says why, in the panel. |
| 10 | Node mode showed the watchdog's sentence in lower case with no full stop. | Shown as a sentence. |
| 11 | Info and Settings buttons had no tooltip; em-dashes in visible sentences. | Fixed. |
| 12 | Compact mode is only a light, with no way to read why it is amber. | The light's tooltip is the headline. |
| 13 | The tray said "Node: running · block 245498" on a stuck node. | "Node: not following · block 245,498" (`tray::stuck_for_tray`, tested). |
| 14 | A slow status poll (btxd not answering) stacked up overlapping polls behind a screen that looked current. | One poll at a time; after 10 s the line says it is waiting and how old the screen is. |
| 15 | "Very low disk space: the node may stop" sat in the disk card, about 500 px below the fold of the 780 px window, under a green LIVE. | DISK ALMOST FULL in amber at the top, on the disk card's own threshold (`disk_critical_mb`, 2 GiB). Fixed in the follow-up to #189. |

Each row can be opened in the preview (end of this page);
`?s=follower-frozen-6h` is the first one.

## Still open, most important first

### 1. The role card talks to home users like operators. Needs a decision.

Generated from `btx_core::role` itself (`apps/node/preview/rolegen`), a Mac
that follows signatures behind a home router shows, under a headline that
says it is helping:

- **Validation: "Following ONE signer"**, not helping: "However many keys
  the config pins, that one machine being switched off ... stops this node
  ... Pin a second key that is actually signing." (`role.rs:439`). A
  follower shows this whenever one key carried all its recent blocks,
  which is what a follower whose only working signer is the 3060 sees (not
  checked on a real follower tonight). A home user cannot pin anything from
  the app, and "config pins" is operator language.
- **Serving history: "Off"**, not helping: "Not serving attestations."
- **Reachable: "No inbound connections"**, not helping, with a port
  forwarding instruction (`role.rs:644`).

Three of five lines carry the "does not help" mark for the most common
install (a hollow circle, read out as "does not help the network"), and two
of them are not things the person did wrong or can easily change. The mark
is quiet; the words are the problem.

Proposal: keep the facts, change who they are addressed to.

- Validation (follower, one signer): value "Following signatures (one
  signer)", amber; note "Your node follows blocks signed by other nodes.
  Right now only one of the signers it trusts is signing, so if that one
  stops, your node waits. Nothing to change on this computer."
- Serving history and Reachable on a follower: neutral (grey dot, not
  "not helping"), notes that say what it would take without implying a
  fault ("Other nodes cannot connect in, which is normal behind a home
  router. Forwarding port 19335 lets them.").
- Operator wording ("config pins", "attestations", "threshold one", "full
  authority") moves to Copy diagnostics, which already carries the facts.

Why this is a decision and not a fix: the "Pin a second key" line was
written for mirror operators (the 2026-09-17 btxscan case, where one of four
pinned keys carried 552 of 600 blocks), and its test says naming the remedy
is the point. The same card serves both audiences. The clean version keeps
the remedy where the pins are the operator's own (a `btx_rw.conf` the app
did not write, which the watchdog already detects) and gives home installs,
whose pins ship with the app, the "nothing to change on this computer"
wording.

### 2. The tray icon does not change colour

The tray's status line rendered `Node: running · block 245498` from the
phase alone, so someone who only looks at the menu bar saw nothing wrong.
**Fixed in #189:** it says `Node: not following · block 245,498` on the
watchdog's verdict or an old tip with nothing newer known, computed in Rust
because the window may be hidden. Still open: the icon is the static app
icon. Proposal: an amber and a grey variant, so the state is visible
without opening the menu.

### 3. A failed status poll leaves the last state on screen

`get_node_status` asks btxd twice with a 60 s timeout each, and the app
polled every 1.5 s with no guard, so a btxd that accepts connections and
never answers stacked up dozens of polls while the screen showed its last
state as current. **Fixed in #189:** one poll at a time (an action's
refresh runs right after the slow one lands), and after 10 s the line under
the badge says "Waiting for your node to answer. This screen is 12 s old".
Still open: a poll that *fails* (the command errors) after the first success
only goes to the console; the screen keeps its last state. Proposal: after
two failures in a row, the same line with "The app lost track of your
node".

### 4. Settings is one flat list of about 22 rows

Everyday choices (launch at login, keep awake, what the close button does,
colour, updates) sit between operator switches (sign confirmations, offer my
key, witness address, Esplora with build paths, serve a chain snapshot) and
the destructive Remove node data. Screen: `finding-settings-flat-list.jpg`.

Proposal, four groups with headings, the third collapsed by default:

1. **This computer:** Launch at login, Keep awake, When you close the
   window, Updates, Colour, Look, Data folder.
2. **Helping the network:** Keeper mode, Serve confirmations, Follow
   signatures instead of checking, Node nickname, Service report.
3. **For operators** (collapsed): Sign confirmations, public key, Offer my
   key, Help wallets check the chain, Serve a chain snapshot, Esplora.
4. **Danger zone:** Reclaim disk space, Remove node data.

Also: Esplora shows "Not installed yet: electrs
(deploy/esplora/build-electrs.sh)" (`main.ts:1442`), a developer path a
user cannot act on. Hide the row unless the programs are found, or link a
guide. Descriptions mix sentences with and without a full stop.

### 5. The setup screen lets you start an install that cannot fit

The disk check runs only after "Set up my node" is pressed, then shows an
error below the fold (it scrolls there), although both numbers were on
screen before the click, for example "Disk needed 140.0 GiB, Free on this
computer 12.4 GiB". Screen:
`finding-wizard-disk-error-after-click.jpg`. Proposal: colour the free space
red and say it next to the button before the click; when free space is
between 20 and 140 GiB, offer Keeper mode (20 GiB) right there.

### 6. Ask answers only fixed questions

Seven questions and a lookup box. The questions people actually ask when
something looks off are not there. Proposal, each answered from data the
app already has:

- "Is my node on the right chain?" (the agreement row, in plain words)
- "Why is my node not moving?" (stall verdict, stale tip, signer state)
- "Who is my node connected to?" (peer count, named peers, inbound)
- "What has my node done for the network?" (the Helping card)

The source chips name raw RPC calls ("via getblockchaininfo +
getchainstates", `ask.ts:110`): honest, but "from your node" with the call
in smaller type would read better.

### 7. Smaller things

- **Units:** "Helping the network" prints 1024-based bytes as "GB"
  (`contribution.ts:70`) two cards below "9.8 GiB". `fmtGB` was already
  fixed for the same 7%.
- **Times:** three formats on one screen: "6:12:19 AM" (Settings, key
  offer, `main.ts:852`), "today 05:37" (updates), "21:40 UTC" (agreement
  row). Proposal: local "HH:MM" everywhere, UTC only where a source's time
  is quoted.
- **"Helping the network"** is one long sentence ("4 nodes have connected to
  you, you have sent them 1.2 GB of the chain, you have passed ... running
  for 3 hours"). Three short rows would scan faster.
- **The welcome panel** is five paragraphs; the signing paragraph says
  "until now they all followed one key on one home computer", which will
  age. Proposal: one line per switch, the long reasons behind a "Why?".
- **Tools** repeats the headline at the top, then leaves a blank gap. When
  something is wrong it could put the useful action first (Copy
  diagnostics when stuck, Restart when the engine asks for it).
- **The header** is six icon-only buttons. Tooltips exist now; Compact and
  Info could move (window control, Settings footer) to leave four.
- **Rust sentences with em-dashes** that reach the screen: wallet errors
  (`wallet.rs:645, 721, 772, 794`), Ask (`ask.rs:280`), snapshot save
  (`snapshot.rs:480`), start errors (`commands.rs:1319, 1328, 2014, 5133`),
  node start (`node.rs:4267`), watchdog summaries (`watchdog.rs:426, 534`).
  Each has tests on its text, so this is a sweep of its own.
- **About's fine print** quotes the chain at 124 GiB measured 2026-09-04.
  Worth re-measuring before the next release.

## Ideas for the design

- **A heartbeat:** "New block 40 s ago" under the height, ticking. A
  healthy node shows it resetting every minute or two; a stuck one shows it
  growing, long before any verdict. It needs the newest block's own time,
  not the median time the stale check uses.
- **"Block 245,498 of 245,760"** in the height tile, from the network
  height the app already computes (`btx_core::network_height`).
- **One action under the headline when there is one:** "Copy diagnostics"
  when NOT FOLLOWING for an hour, "Start node" when stopped (done), "Free
  space" when the disk is low.
- **Tray icon in three colours** (see 2).

## Edge cases checked

| Case | Result |
|---|---|
| Follower frozen under 15 minutes | LIVE (by design, no verdict yet) |
| Follower frozen 15 min to 2 h | NOT FOLLOWING, "no signature it trusts is arriving" (was LIVE) |
| Follower frozen over 2 h | NOT FOLLOWING with the age (was LIVE) |
| Agreement row about a source behind or on another branch | Headline unchanged (tested) |
| Stopped node with chain facts left from the last run | STOPPED, no amber (tested) |
| Syncing node that stopped adding blocks | SYNCING x%, amber, "not keeping up" (tested) |
| Close button pressed with a panel open | Dialog now on top (was hidden) |
| Escape on the close dialog or welcome panel | Ignored on purpose |
| Windows wording | About now uses the tray word |
| Status poll slow (btxd not answering) | One poll at a time, "Waiting for your node to answer" after 10 s (checked in the preview with a 20 s status call) |
| Disk under 2 GiB free while live or syncing | DISK ALMOST FULL, amber (was LIVE, warning far below) |
| Disk under 10 GiB free | Amber note in the disk card only, as before |
| Status poll fails after start | Last state stays up (open, item 3) |

## Looking at every state yourself

```bash
npm --prefix apps/node run preview:states
```

Then open `http://localhost:1440/preview/index.html` for the list, or
`http://localhost:1440/?s=follower-frozen-6h&open=tools` for one state with
a panel open. Scenarios live in `apps/node/preview/scenarios.ts`; after a
change to `role.rs`, regenerate the role card lines with
`cargo run --manifest-path apps/node/preview/rolegen/Cargo.toml`.
