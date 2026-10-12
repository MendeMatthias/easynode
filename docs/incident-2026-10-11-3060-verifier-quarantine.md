# 2026-10-11: the 3060 signer's GPU verifier was quarantined for 13+ hours

The project's RTX 3060 signer (`02d5efca`, nickname `innovation3060`, a
Windows PC) stopped checking blocks at block 245,498 and signed nothing
after it. Every easyNode that follows signatures stopped with it, except
where zbtx3's key (`02c9cfb7`) carried them further. Not resolved when this
was written (2026-10-12 12:00 UTC).

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

## Not known

- Why the card failed at 19:48. A sleep/wake or a graphics driver reset
  would explain both the verifier and the network drop; nothing measured
  says which.
- Whether restarting the engine brings the card back, or the PC needs a
  reboot (a driver reset).

## What to look at on the 3060

- The engine's `debug.log` (node folder: Settings > Data folder > Open)
  around 19:40 to 20:00 UTC on 10-11: lines starting "MatMul Profile-1
  provider quarantined" and what precedes them.
- Windows Event Viewer, System log, same window: 23:40 to 00:00 local time
  on 10-11 to 10-12 (the PC is in Dubai, UTC+4; it shares its public address
  with this project's Mac). Kernel-Power (sleep, resume), `nvlddmkm`
  (driver reset, Xid), Display.
- `nvidia-smi`: driver version, temperatures, whether another process holds
  the GPU (the easyBTX miner shares the folder `~/.easybtx`).
- Power settings: anything that lets the PC sleep despite easyNode's
  keep-awake.

## What changed

- easyNode 0.7.12 (released 10-12) still shows NOT FOLLOWING for this,
  and its tray says "not following" too.
- easynode #199 (merged 10-12, not released): when the engine reports it
  cannot verify blocks for 15 minutes in a run that had been verifying,
  easyNode restarts the engine itself, at most once every 3 hours
  (`btx_core::engine_warnings::restart_for_lost_verifier`). Release it
  (0.7.13) only if a restart is shown to bring such a card back; if only a
  reboot does, rework it first.
