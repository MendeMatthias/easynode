# 2026-10-11: the 3060 signer's GPU verifier was quarantined for 13+ hours

The project's RTX 3060 signer (`02d5efca`, nickname `innovation3060`, a
Windows PC) stopped checking blocks at block 245,498 and signed nothing
after it. Every easyNode that follows signatures stopped with it, except
where zbtx3's key (`02c9cfb7`) carried them further. The PC was rebooted on
10-12 at about 13:00 UTC and signs again (below).

## Measured

From the 3060's stats export (2026-10-12 08:53 UTC) and read-only RPC on
btxscan's node:

- The 3060's tip is block 245,498, whose time is 19:48:05 UTC on 10-11.
  `blocks` and `headers` are both 245,498. Its peers announce headers up to
  246,027 that it cannot use: since block 244,000 a header counts only once
  checked or signed by a pinned key, and its pin is itself plus `028995b2`
  (which no longer signs).
- `getblockchaininfo.warnings`: "CRITICAL: MatMul RC strict-device validation
  is unavailable ... provider=cuda_rc_exact_fused_extract
  reason=strict_device_provider_quarantined ... Restart with a provider that
  passes the full self-qualification and production canary".
- `getmininginfo.backend_runtime.rc_exact_replay`: startup qualification had
  passed (`self_qualified`, `startup_canary_passed`); now
  `provider_health.quarantined: true`, `validator_readiness_lost: true`,
  `quarantine_events: 18`, `recovery_self_checks: 0`; last validation
  `provider_health_reason: device_seeded_phase1_declined_or_wrong_size`.
- In v0.34.15 (`src/matmul/matmul_v4_rc_gkr.cpp`) each failed attempt calls
  `QuarantineProvider` again and counts one event, the re-test waits 60 s
  doubling to a 1 h cap, and `recovery_self_checks` counts only successes.
  So the engine re-tested about 18 times over 13 hours and the card failed
  every one, inside the same process.
- All 14 of the 3060's peer connections were opened between 19:51:06 and
  19:56 UTC on 10-11 (two later ones aside): its network dropped in the
  same minutes the verifier was lost.
- btxd uptime 122,061 s at the export (started 23:00 UTC on 10-10, after the
  0.7.9 update), so the app's 0.7.10 update adopted it and did not restart it.
- The app was 0.7.10. It already showed NOT FOLLOWING for this warning
  (`rc_stalled` from `CannotVerifyBlocks`); its observer log said "ok"
  because blocks equalled headers.
- 12:00 UTC on 10-12: btxscan still held the same connection from the 3060
  (opened 19:51:08 on 10-11), so its engine had not been restarted yet.

## After the reboot (10-12, read-only on btxscan's node)

- The owner quit easyNode and rebooted the PC at about 13:00 UTC. By 13:06
  `02d5efca` was on blocks 245,499, 245,500 and 245,510, which it had never
  signed: the card passes the engine's check again and the node signs each
  block as it checks it.
- Pace, 13:08:39 to 13:12:42 UTC: 245,520 to 245,534, about 207 blocks an
  hour against the chain's about 40. 664 behind the tip (246,198) then, so
  at the tip in about 4 hours if the pace holds.
- zbtx3's `02c9cfb7` signed up to about 245,975, so followers could already
  reach that far; past it they move with the 3060 (about 2 hours from 13:12).
- The 3060 was not directly connected to btxscan at 13:06; its signatures
  reached btxscan through other peers.
- A reboot is not a node restart; see "Would restarting only the engine
  have brought the card back?" below.
- It reached the network tip at 17:33 UTC on 10-12 (246,379, the same hash
  as btxscan), every block signed by `02d5efca`, verifier not quarantined.

## Cause: Windows Update replaced the GPU driver under the running engine

Measured on the 3060 on 10-12, read-only.

- The 3060's easyNode is the Linux AppImage inside WSL2 on that Windows
  PC (datadir `~/.easybtx`), with btxd reaching the card through WSL's
  CUDA passthrough. It is not the Windows build.
- System log, 10-11: Windows Update installed "NVIDIA Display Driver
  Update (32.0.16.1088)" (driver 610.88) from 23:49:25 to 23:50:46 local,
  which is 19:49:25 to 19:50:46 UTC. The engine kept running through it.
- The app's observer log (one row every two minutes) has block 245,498
  connected between 19:48:53 and 19:51:03 UTC and nothing after. The peer
  reconnections at 19:51 to 19:56 fall in the same minutes.
- Ruled out: no `nvlddmkm`, Display or Dxgkrnl events, no sleep, resume or
  unexpected power loss in the window. Sleep and hibernate are set to
  never. The easyBTX miner did not run: its launcher exits on an owner flag,
  and its logs were last written on 10-05.
- The engine's `debug.log` was trimmed at the restart and starts at 10-12
  02:25 UTC, so the 19:49 lines are gone; the app's own log rotates per
  launch. What survives (02:25 to 12:55 UTC) shows the in-process re-test
  about hourly ("provider auto-un-quarantined ... backoff_elapsed") failing
  at once with `device_seeded_phase1_declined_or_wrong_size` every time.
  That reason is logged when the CUDA call `rc_phase1_seeded` returns false
  or throws (v0.34.15 `src/matmul/matmul_v4_rc.cpp`, around line 840).
- Every 30 minutes the engine logged "MatMul RC runtime re-probe: ...
  present=1 reason=runtime_identity_present_but_full_qualification_requires_restart".
  That text is fixed (`src/init.cpp`, around line 4165): it means the
  device's identity was readable from inside the old process. The driver
  and the card answered; only the computation failed.
- The engine never resets its CUDA context in a running process (no
  `cudaDeviceReset` in the v0.34.15 sources).
- Windows also restarted the PC on its own on 10-12: the owner's reboot at
  12:54 UTC (from the update notification), then two automatic restarts
  at 13:29 and 13:32 UTC as an update finished. The engine came back after
  each, and its startup qualification passed every time.
- Nothing starts the node until someone logs in on that PC: WSL is started
  by a Startup-folder script and a logon task, and auto-logon is off.

## Would restarting only the engine have brought the card back? No.

Measured on 2026-10-13: the owner installed an NVIDIA driver on purpose
while the node ran (610.88 to 617.42).

- Windows registered the new driver at 10:09:28 UTC. At 10:09:29 the engine
  quarantined its verifier with the same reason as on 10-11
  (`device_seeded_phase1_declined_or_wrong_size`), withdrew
  `NODE_MATMUL_CONSENSUS` and disconnected all 13 peers. That is the 10-11
  "network drop": the engine's own doing, not the network's.
- It re-tested inside the process six times in 51 minutes and failed every
  time, as on 10-11. The installer ran until 10:55:42 UTC.
- 10:58: a throwaway btxd (empty datadir, no network, same flags) passed the
  full startup qualification on the new driver (`driver=13040`).
- 11:01: easyNode and btxd were restarted, WSL and the PC were not. The new
  btxd failed startup qualification: "CUDA driver version is insufficient for
  CUDA runtime version", DEGRADED START with provider `cpu`. A throwaway btxd
  at 11:02 failed the same way. New processes inside the old WSL VM worked
  only for a moment after the install; `nvidia-smi` inside it crashed
  throughout.
- 11:02: `wsl --shutdown`, then a fresh start. btxd passed startup
  qualification at 11:03:21 and signed 246,970 onwards (the block that had
  failed at 10:09).

So on a node inside WSL2, a host driver swap needs a WSL restart, not an
engine restart. #199's engine-only restart would not have helped. It costs
one restart and then stops (a run whose card fails startup qualification is
not restarted again), but it does not heal the node.

The owner keeps Windows Update and driver installs on, on purpose: the node
has to survive a driver swap, a restart or a power cut on its own.

## The recovery, end to end (2026-10-14)

easyNode 0.7.14 on the 3060, started at 09:23 UTC by the boot chain after a
Windows restart and verifying blocks since. The owner reset the card with
`pnputil /restart-device` (admin); a watcher on the Windows side logged each
step and never touched WSL while the restart was due. Times UTC.

| | |
|---|---|
| 16:45:53 | Card lost while checking block 248,134: `device_seeded_phase1_declined_or_wrong_size`, `NODE_MATMUL_CONSENSUS` withdrawn, as with the driver install of 10-13 |
| 16:46 to 17:01 | Frozen at 248,133 by design: the 15 minutes the engine's own re-test gets first. The app (pid 575) survived the reset |
| 17:01:26 | The app's Windows side: "easyNode lost its GPU block verifier; restarting WSL" (`%TEMP%\easynode-wsl-restart.log`) |
| 17:01:34 | `wsl --shutdown` exited 0 |
| 17:01:43 | Relaunch, attempt 1; Ubuntu running again at 17:01:44 |
| 17:02:23 | easyNode started by the relaunch (its 40 s wait); the boot hook stood aside |
| 17:02:28 | The new engine qualified the card (`ready=1`) and signed from 248,134 on as `02d5efca` |
| 17:25:25 (block 248,158) | At btxscan's tip again |

The restart record was written (`gpu_wsl_restart_last_unix`) and the streak
went back to 0 once the new run verified blocks. The witness, the tunnel, the
Esplora front, the observer and the keepalive came back with WSL by
themselves. The node came back 13 blocks behind (the network mined 248,134
to 248,146 during the outage) and connected 25 blocks in 22 and a half
minutes, about 54 s each, while the network added 12 more. That pace was
the engine's own ("MatMul pending verification cap reached
(budget-deferred)", bodies re-queued every 60 s), not easyNode's.

Costs, both expected: every other WSL distribution on the PC stopped with
WSL (here AInode's engines and build distributions), and the node signed
nothing for about 16 minutes.

**The gap it showed.** The app judged "the card had been working" from its
own run (`chain.blocks > first` in the refresher). It survived this reset,
but on 10-13 a device reset closed its window through WSLg, and the
keepalive started a fresh app. A fresh app adopts the same frozen engine,
never connects a block in its own run, and so never acts: the node stays
frozen until someone notices. Fixed after this run: the app also asks the
engine (`btx_core::engine_warnings::engine_lost_its_card`). In v0.34.15
`quarantine_events` rises only when a card in use fails while checking a
block (`QuarantineProvider`, `ExtendProviderQuarantine`,
src/matmul/matmul_v4_rc_gkr.cpp:84-169), so "quarantined, at least one
event" means this engine had the card and lost it, while a card refused at
startup counts none. Such a run is treated like one that verified, under
the same 3-hour spacing, but only before our first WSL restart for the
loss (on the engine path, before the app's first restart), so two
restarts per loss stays the cap; the one follow-up rule is unchanged.
Released in 0.7.15 (#211). That path is tested against the engine's
recorded answers; the reset above ran under 0.7.14, whose app survived it.

## What changed

- easyNode 0.7.12 (released 10-12) shows NOT FOLLOWING for this, and its
  tray says "not following" too.
- easynode #199 (merged 10-12, released in 0.7.13, hardened in 0.7.14 by
  #208, extended in 0.7.15 by #211): when the engine reports it
  cannot verify blocks for 15 minutes in a run that had been verifying,
  easyNode restarts the engine itself, at most once every 3 hours
  (`btx_core::engine_warnings::restart_for_lost_verifier`). Reworked after
  the 10-13 measurement (`lost_verifier_remedy`, `btx_core::wsl`): inside
  WSL the app stops the node and restarts WSL from the Windows side, behind
  a Settings switch that is on by default; elsewhere it restarts the
  engine; and the message says to restart the computer when the check
  still fails. The native Windows engine has no GPU verifier, so the case
  does not arise there. Whether an engine restart is enough on native Linux
  or macOS after a driver update is not measured.
- On the 3060 itself (not in the repo): a boot task starts WSL with nobody
  logged in, and `instanceIdleTimeout=-1` stops WSL from shutting an idle
  distro down. On 10-13 WSL had stopped the node's distro about 15 s after
  the logon launcher's calls ended, killing the running node.
