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
same CPU probes, on an otherwise idle Mac, plain and under `taskpolicy -b`
(raw output, `engE/res-ecore-cpu.txt` in the session scratchpad):

```
2026-10-01T18:24:14Z, load averages 3.63 2.97 3.96
run 1 plain cpu1=1545 cpu10=2783 | bg cpu1=4827 cpu10=12432
run 2 plain cpu1=1594 cpu10=2668 | bg cpu1=3656 cpu10=12112
run 3 plain cpu1=2534 cpu10=4833 | bg cpu1=4042 cpu10=13318
run 4 plain cpu1=1542 cpu10=2300 | bg cpu1=4240 cpu10=12342
run 5 plain cpu1=1536 cpu10=2181 | bg cpu1=4291 cpu10=12918
```

An earlier single run had bg 4833 / 18836 ms against plain 1551 / 2169 ms
(not saved as a file), and the review's rerun had bg 3976-4046 / 11687-12531
ms. So: about 2.4 to 3.1 times as long on one thread and 4.4 to 8.7 times on
ten threads, and it varies from run to run. Under `taskpolicy -c utility` the
same probes took 1544 and 2193 ms, no change.

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
   gives way. One log line says which policy was applied, or why not. The
   controller remembers the policy, because macOS does not report another
   process's background state.
2. **The GPU-check extension runs at normal priority.** 0.7.1 sized the
   ten-minute budget at about 2.75 times the slowest normal-priority check
   (218 s). The background policy makes the check 1.5 to 2.1 times slower, so
   on a slower Mac it could use up most of that margin, and running out moves a
   validating (signing) Mac to following signatures. So when the ordinary
   180 s wait runs out and the app shows "checking this machine's chip", the
   engine is held at normal priority until its RPC is up, then put back under
   the background policy. The first three minutes still give way to the person
   using the Mac; a slow Mac is never failed over because of the policy.
3. **A snapshot load runs at normal priority.** Before the load task starts
   (the plain load, the signed load, and Fast-forward's, which all go through
   `ensure_snapshot_loaded_with`), the engine is held at normal priority; the
   load's watch releases it when the task ends, for the same run only. A
   multi-GB load is not queued behind other programs' disk I/O, and it does not
   depend on whether a retune tick has seen the gap yet.
4. The status refresher re-decides every 30 s from the headers/blocks gap:
   more than 500 blocks behind (about 12 hours of chain), the engine runs at
   normal priority so a long catch-up is not confined to the efficiency cores;
   within 10 blocks of the tip it goes back into the background policy.
   Between the two lines nothing changes, and a hold (2, 3) is left alone. The
   slot stays locked across the `setpriority` call, so the child cannot be
   reaped and its pid reused in between. It logs when it changes.
5. **A node adopted after a self-update** (the previous app's btxd, kept
   running across the update) is managed too: the refresher finds it through
   its `btxd.pid`, used only when that process is alive and named btxd (the
   same guard as the force-kill), and APPLIES the chosen policy to it rather
   than assuming one, since the previous app may have left it either way.
6. Before a stop, the engine is taken out of the background policy, so the
   shutdown flush is not queued behind other programs' disk I/O and killed when
   the grace runs out. That covers the spawned node (`NodeController::stop`)
   and the adopted one (`stop_unmanaged_node`).

The update from 0.7.1: 0.7.1 started its node at normal priority and the
updated app adopts it. From 0.7.2 on, the refresher applies the policy to that
adopted node too, so it goes into the background policy within about 30 s of
the new app's first status poll once it is at the tip, without a restart.

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
not measured, and neither was a foreground that keeps every CPU core busy
(see below). The one way the policy could stop a Mac signing, a GPU check that
runs past the ten-minute budget, is closed by running the extension at normal
priority (point 2 above).

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
  priority, so its writes are not throttled, and so does a snapshot load. The
  shutdown flush is taken out of the policy first. An assumeutxo node validating its history in the
  background while at the tip does that work in the background policy, slower
  when the Mac is busy; that is the intended direction and was not measured.
- **Validation and signing of a live block** at the tip, in the background
  policy, with the Mac busy. Reasoned above, not measured. Nor a foreground
  that keeps every CPU core busy.
- **Network traffic.** macOS also marks a background process's sockets as
  background traffic, which it sends with a yielding congestion control
  (LEDBAT) behind other traffic on the same link. While a foreground app uses
  the link heavily, that could add seconds to relaying a block or an
  attestation (a signature). On an idle link it changes nothing. Not measured.
- **A fresh node's header sync** runs in the background policy: before the
  headers climb, headers and blocks are both near zero, so the node reads as at
  the tip. Header sync is light work (download and check of 80-byte-class
  headers, no MatMul replay), it reached the snapshot anchor in about a minute
  in earlier measurements, and the app only ends the header bootstrap as
  stalled after 5 minutes with no movement at all (`HEADER_BOOTSTRAP_STALL`).
  A 2-3 times slower climb still moves well inside that limit, and any
  movement resets it. Once headers lead blocks by more than 500, the refresher
  lifts the engine anyway. Low risk, not measured.
- **Other Macs.** One M2 Pro. The reports are from M4 and M5 machines, with
  the miner running; the miner was not part of this measurement (the hog runs
  only stand in for a GPU-heavy program).
- **Screen smoothness itself.** The probes are a fixed Metal job and a fixed
  CPU job, not frame times of the window server.
