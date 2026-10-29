# btx-sentinel

The always-on watcher on the Azure box, beside btxd and btxd2. Vercel runs 60
seconds at a time and the laptop only while it is awake; this runs all the
time and sends what is wrong to the same Orca chat and ntfy topic the box's
selfcheck uses.

It only reads. It never writes to a node, never calls a mutating RPC, and
listens on nothing.

## What it alarms on

| Alarm | When | How often |
|---|---|---|
| No snapshot confirmed yet | `/api/snapshots/latest` says `confirmed: null` | once a day, until the first one |
| Snapshot old | the newest confirmed snapshot is older than 6 h | hourly |
| Producer quiet | no new statement in `/pending` for 12 h | hourly |
| Confirmer quiet | the newest statement still has one operator after 6 h | hourly |
| Dispute | `/disputes` lists a height | hourly while open, a recovery when cleared |
| Few signers | fewer than 2 distinct keys over the last 100 blocks (btxd2) | once a day (a warning) |
| Signatures silent | no new signed block for 20 min (btxd2) | hourly |
| btxd2 RPC down | two failed reads in a row | hourly |
| Seed down | a shipped seed fails a version handshake twice in a row (probed every 30 min) | hourly |
| Census stale | `easybtx.com/api/nodes` `checkedAt` older than 90 min, or unreadable twice | hourly |
| Witness down or behind | two failed reads, or more than 6 blocks behind btxd2 | hourly |
| Fleet | a node on the current engine is further behind on two census runs in a row | hourly |

Each alarm says once when it clears, to the channels that heard it. The seeds
are read from the code (`BTX_BOOTSTRAP_PEERS` and `BTX_ARCHIVE_PEERS` in
`crates/btx-core/src/node.rs`), so a release that changes them changes what is
watched. A witness that has never answered (witness-2 at
`https://api.btxscan.io/witness` until it is deployed) is skipped, not an
alarm. Every threshold is a flag: `btx-sentinel --help`.

The handshake sends `version` and `verack` and nothing else: no `getdata`,
no `getaddr`, no attestation requests.

## Install

The Linux binary comes from the box-tools workflow. With it on the box at
`./btx-sentinel` and this folder beside it, one command:

```sh
sudo install -m755 btx-sentinel /usr/local/bin/ && sudo install -m644 btx-sentinel.service /etc/systemd/system/ && sudo systemctl daemon-reload && sudo systemctl enable --now btx-sentinel
```

Then check it:

```sh
sudo btx-sentinel --check          # one pass of every check, prints, sends nothing
sudo btx-sentinel --test-orca      # one test line to Orca
journalctl -u btx-sentinel -f      # what it is doing
```

`--check` and `--test-orca` run as root, so they read
`/etc/btxscan-selfcheck.conf` and the cookie directly. The service reads the
conf through systemd's `LoadCredential=` (see the unit for why: the conf
stays root 600 and is never copied or regranted).

## Stop

```sh
sudo systemctl disable --now btx-sentinel
```

State is in `/var/lib/btx-sentinel/state.json`; deleting it only means each
alarm that is still true is said once more.
