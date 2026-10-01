# The node engine's priority on a Mac (easyNode 0.7.2)

Measured 2026-10-01 on an M2 Pro (6 performance + 4 efficiency cores,
macOS 26.6.2, build 25G83), engine v0.34.12
(`btx-0.34.12-arm64-apple-darwin.tar.gz`, sha256
`d90d1adf2ae1d9a29decc97423db258459674a08c35e0bef60dc82467983d72e`).
Code: `crates/btx-core/src/engine_priority.rs`.

## Why

Owners on an M4 MacBook Air and an M4 MacBook Pro, with easyNode and the
easyBTX miner open, found the Mac almost unusable; the miner at 30% did not fix
it. The miner is a separate product and not covered here. Up to 0.7.1 easyNode
started btxd at normal priority with no nice, taskpolicy or QoS, and on Apple
Silicon btxd validates MatMul proofs on the GPU (Metal), which also draws the
screen.

## Method

- The engine started the way easyNode starts it on a validating Mac
  (`build_node_command`, Metal arm): `BTX_MATMUL_BACKEND=metal`, no
  `-matmulvalidation` (the engine's own strict-device default), `-server=1
  -v2transport=1 -parkdeepreorg=0 -reorgpolicy=legacy -autoupdate=0`, plus
  `-connect=0 -listen=0 -dnsseed=0`, its own rpc/p2p ports and an empty
  throwaway datadir on mainnet params. Each start runs the RC production
  canary (one full episode on the GPU) before RPC binds, which is the GPU
  phase measured here. Every run was stopped afterwards; no btxd was left
  running (`ps` checked after each batch).
- Variants: (a) normal, today; (b) `nice -n 10`; (c) `taskpolicy -b`, i.e.
  `setpriority(PRIO_DARWIN_PROCESS, PRIO_DARWIN_BG)`; (d) `taskpolicy -c
  utility`.
- Foreground probes, run back to back for the whole GPU phase: a fixed Metal
  compute job (60 command buffers of a 1M-thread sin/fma kernel, Swift) and a
  fixed integer CPU job on 1 and on 10 threads (C). Idle times: Metal 757-767
  ms, CPU 1 thread 1532-1596 ms, 10 threads 2078-2393 ms (12 runs, before and
  after the batch).
- GPU utilisation from `ioreg` (`Device Utilization %`), once a second. No sudo,
  so no powermetrics.
- "Loaded" runs (L1, L2) had the probes running; "quiet" runs (Q1) had nothing
  in the foreground, for the start time alone; "hog" runs (H1-H4) had the Metal
  probe looping without a pause for the whole start, a stand-in for a miner at
  normal priority.

## Numbers

Time to the RPC cookie (seconds; the GPU check is all but the last 1-3 s):

| variant | loaded L1 | loaded L2 | quiet Q1 | continuous GPU load |
|---|---|---|---|---|
| (a) normal | 110.2 | 105.6 | 83.5 | 125.8, 153.4 |
| (b) nice 10 | 106.2 | 100.8 | 107.8 | not run |
| (c) background | 168.9 | 219.0 | 148.3 | 139.2, 173.7 |
| (d) utility clamp | 153.5 | 123.4 | 108.7 | not run |

The app gives the GPU check ten minutes (`GPU_CHECK_EXTRA_POLLS`); the slowest
background start here was 219 s.

Foreground cost during the GPU check (loaded runs; Metal job, ×idle):

| variant | run | Metal median | Metal mean | samples > 1.5× idle | CPU 1 thread mean | CPU 10 threads mean | engine GPU util |
|---|---|---|---|---|---|---|---|
| (a) normal | L1 | 3100 ms | 3.16× | 9/14 | 1.07× | 1.31× | 87% |
| (a) normal | L2 | 2787 ms | 3.32× | 10/13 | 1.11× | 1.32× | 89% |
| (b) nice 10 | L1 | 2522 ms | 2.81× | 9/15 | 1.04× | 1.16× | 90% |
| (b) nice 10 | L2 | 3407 ms | 3.43× | 10/13 | 1.06× | 1.18× | 91% |
| (c) background | L1 | 828 ms | 2.65× | 12/26 | 1.03× | 1.08× | 71% |
| (c) background | L2 | 785 ms | 1.98× | 12/35 | 1.08× | 1.24× | 63% |
| (d) utility | L1 | 928 ms | 2.57× | 9/21 | 1.13× | 1.34× | 77% |
| (d) utility | L2 | 3318 ms | 3.12× | 9/17 | 1.07× | 1.15× | 80% |

Continuous GPU load (the Metal job looping for the whole start, including the
seconds after the cookie): median 943 / 1071 ms and mean 1642 / 1602 ms at
normal priority, median 776 / 890 ms and mean 1462 / 1368 ms in the background
policy.

The background policy confines plain CPU work to the efficiency cores. The
probes themselves under `taskpolicy -b` on an idle Mac: 1 thread 4833 ms (3.1×
the plain 1551 ms), 10 threads 18836 ms (8.7× the plain 2169 ms). Under
`taskpolicy -c utility`: 1544 and 2193 ms, no change.

## What the numbers say

- The engine's GPU check costs the foreground's GPU work about three times
  its idle time at normal priority, most of the time. That is the part of the
  report this app can own. Foreground CPU work was barely touched by the GPU
  check (at most 1.3× on 10 threads): the check is GPU work.
- `nice` changes nothing for the GPU (its Metal numbers are the same as
  normal's). It cannot be undone without root either.
- The utility clamp was inconsistent: one run as good as background, one as
  bad as normal.
- The background policy is the only variant that moved the GPU number
  consistently: the median foreground sample is back at idle and the mean
  drops to 2 to 2.7×. It does not remove the cost: the engine's GPU jobs still
  run once started, so about a third to a half of the samples stayed slow, in
  bursts. Because the check takes longer, the total number of slow samples per
  start is about the same (12 against 9-10); they are spread thinner.
- Its price: the start-up check takes 1.5 to 2.1 times as long (well inside the
  ten-minute budget, also under a continuous GPU load), and CPU work goes to
  the efficiency cores, about 3× slower per thread.

## What was chosen

On macOS:

1. At spawn (`NodeController::start`, so also every `restart`), btxd goes into
   the background policy, so the GPU check that runs on every start already
   gives way. One log line says which policy was applied, or why not.
2. The status refresher re-decides every 30 s from the headers/blocks gap: more
   than 500 blocks behind (about 12 hours of chain), the engine is taken back
   out to normal priority so a long catch-up is not confined to the efficiency
   cores; within 10 blocks of the tip it goes back into the background policy.
   Between the two lines nothing changes. A node the app did not spawn is left
   alone. It logs when it changes.
3. Before a stop, the engine is taken out of the background policy, so the
   shutdown flush is not queued behind other programs' disk I/O and killed when
   the grace runs out.

Linux and Windows: unchanged (nothing measured there, no reports from there).

No setting in Settings: the policy is on for every Mac. A switch would only be
worth adding if someone reports a node that needs the old behaviour.

### Signers

A node that signs blocks (on by default on every validating Mac) gets the same
policy. An exception would leave exactly the reported Macs unchanged. The
policy delays the engine while the foreground wants the same chip; it does not
stop it: under a continuous foreground GPU load the start-up check finished in
139 and 174 s, against 126 and 153 s at normal priority. A block arrives about
every 90 s, so a signature made somewhat later should still land long before
the next block. That is reasoning from the start-up check. Signing itself was
not measured (see below).

## What this measurement cannot show

- **Catch-up of real blocks.** With `-connect=0` no block arrived; only the
  start-up GPU check was exercised. How much slower catch-up would be in the
  background policy is not measured. The efficiency-core number (3.1× per
  thread for plain CPU work) and the start-up check (1.5-2.1×) are the only
  evidence, which is why a node more than 500 blocks behind runs at normal
  priority.
- **Disk throttling.** The background policy sets the process's disk I/O to
  the throttled tier: its reads and writes (block files, LevelDB chainstate
  and index writes) are delayed while other, unthrottled I/O is going to the
  same disk, and run at full speed when the disk is otherwise quiet. At the tip
  that is one block's writes every 90 s or so. A long catch-up runs at normal
  priority, so its writes are not throttled. The shutdown flush is taken out
  of the policy first. An assumeutxo node validating its history in the
  background while at the tip does that work in the background policy, slower
  when the Mac is busy; that is the intended direction and was not measured.
- **Validation and signing of a live block** at the tip, in the background
  policy, with the Mac busy. Reasoned above, not measured.
- **Other Macs.** One M2 Pro. The reports are from M4 and M5 machines, with
  the miner running; the miner was not part of this measurement (the hog runs
  only stand in for a GPU-heavy program).
- **Screen smoothness itself.** The probes are a fixed Metal job and a fixed
  CPU job, not frame times of the window server.
