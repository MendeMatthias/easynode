# Handoff: easyNode fleet and btxscan statistics, 2026-10-14

Started on the Windows RTX 3060 box, to be continued on the Mac, where `az`, `gh` and every repo are
signed in. The goal is the table Jarek posted for his archive (data sent, blocks served, header requests,
last 24 hours, nodes syncing from us), for our own nodes: the easyNode fleet and the two btxscan nodes.

## Done

- **EasyBTX#584 merged and live** (squash `af7e894c`, Vercel "Deployment has completed" 2026-10-14
  00:04Z). The check-in intake now sums each node's `bytes_sent` delta between check-ins. Published at
  `https://easybtx.com/api/nodes` under `selfReported.served` (`bytesLast24h`, `bytesLast7d`,
  `bytesTotal`, `since`, `daily`, `nodesLast24h`) and `selfReported.reportingInbound` /
  `acceptingInbound` / `inboundPeers`. Verified live at 00:05Z: the new fields are there, `served` is
  `null` until the first check-in after the deploy. The two signers already checking in (schema 2) feed
  it from their next check-in, with no app release.
- **easynode#207 open, not merged**: Settings switch "Count my node in the network's totals"
  (`stats_share_enabled`), check-in schema 3 (`inbound_peers`, `engine_uptime_secs`). It is now safe to
  ship, because #584 accepts schema 3. **Owner decision pending: default OFF (as built) or ON with a
  first-run notice like `signer_publish_enabled`.** Off means very few non-signers will be counted.
- **The 3060 samples its own counters every 5 minutes** since 2026-10-13 20:32Z: systemd `--user`
  `btx-service-stats.timer` → `~/bin/btx-service-stats-sample.sh` → `~/btx-service-stats.jsonl`.
  Report: `python3 ~/bin/btx-service-stats.py --cli "<btx-cli> -datadir=/home/bonuz/.easybtx
  -rpcport=19334" --log ~/btx-service-stats.jsonl report --windows 1h,24h,all`. A full 24 h window
  exists from 2026-10-14 20:32Z.

## What was measured (2026-10-13/14, Windows box)

- **The 3060** (pruned, 0 inbound, behind NAT): 86.8 MB sent in its first 3 h after a restart, ~0.5 GB a
  day, mostly headers. Its real contribution is signing: key `02d5efca` signed 94 of the last 100 blocks
  per `https://api.btxscan.io/witness/signers/recent`.
- **btxscan's two nodes**, from outside (both `/BTX:0.34.15/`, both at the tip, `status.json` `ok`):
  - `btxd` archive `:19335`: NETWORK + ATTESTATION_ARCHIVE. A `getdata` for block 120,000 (1,048,948 B)
    came back in 3.2 s, the same speed as Jarek's `134.199.150.193`, so it was serving history and was
    not at an upload cap at that moment.
  - `btxd2` `:19338`: NETWORK_LIMITED, `notfound` and then a disconnect on old blocks. Not a block source,
    by design.
  - Since easyNode 0.7.10 every easyNode dials both, so `btxd`'s upload is largely the fleet's catch-up
    load. That is the number worth having.

## Next, on the Mac

1. **btxscan's own counters (read-only).** `scripts/btxscan-stats-box.sh` on this branch runs uptime,
   getnettotals (including the `uploadtarget` block), getnetworkinfo and getpeerinfo for `btxd` and
   `btxd2`, finds each unit's btx-cli the way `btx-ops/box-resolve.sh` does, and prints one JSON line per
   node (under run-command's 4 KB). Run it through btx-ops' `az_box` so it queues behind forkwatch for
   Azure's single run-command slot, or directly:
   `az vm run-command invoke -g BTX-ESPLORA-RG -n btx-esplora-1 --command-id RunShellScript --scripts
   @scripts/btxscan-stats-box.sh --query 'value[0].message' -o tsv`.
   `btxd` has run since its 0.34.15 switch on 2026-10-10 (~18:30Z) unless restarted since, so its
   since-start totals cover about the same span as Jarek's four days.
2. **Check the upload cap.** The July runbook set `maxuploadtarget=300` (MiB a day) on the archive; the
   current btx-ops tooling never mentions it. The script prints the conf line and `uploadtarget`. If it is
   still 300, the archive refuses old blocks to non-whitelisted peers after 300 MiB a day, which matters
   now that the whole fleet leans on it.
3. **Windows over time (a write, owner's go).** Install the sampler on the box for `btxd`:
   `scripts/btx-service-stats.py ... sample` every 5 min. For exact block and header counts, `btxd` (not
   easyNode-managed) can run with the `net` debug category (`sample --ensure-net-logging`); watch the disk,
   it is one log line per P2P message.
4. **Never `--ensure-net-logging` on an easyNode-managed node.** The app reads the last 64 KB of
   debug.log for stall and verifier markers; net lines push them out of that window.
5. Decide #207's default, merge it, release.

## Paste this into the Mac session

> Continue the fleet statistics from the Windows box. Read `docs/handoff-fleet-stats-2026-10-14.md` on
> the `handoff/fleet-stats-2026-10-14` branch of MendeMatthias/easynode. First run
> `scripts/btxscan-stats-box.sh` read-only on the btxscan Azure box through btx-ops' `az_box`, and turn
> both nodes' output into a table like Jarek's (data sent, by kind, connections in/out, nodes syncing from
> us, uptime, upload cap). Then tell me whether the archive's `maxuploadtarget` is still set.
