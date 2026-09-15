# Keyless CPU hosts launch as trusted mirrors

| | |
|---|---|
| Status | proposed; merges only on the owner's explicit decision |
| Date | 2026-09-15 |
| Supersedes | the 2026-08-31 "consensus everywhere off Metal" rule, for hosts with no NVIDIA driver only (0.6.15 changelog; the 08-31 paragraph in `crates/btx-core/src/node.rs`) |
| Leaves alone | Metal routing (0.6.19), CUDA routing (08-31), every engine before 0.34.5 |
| Origin | btxscan issue #90, closed 2026-09-14 without recording a decision; the owner's mandate today is that every easyNode should be able to support the explorer |
| Code | `crates/btx-core/src/node.rs` (`build_node_command`, the SPLIT paragraph), `crates/btx-core/src/backend.rs` (`node_host_backend`), `apps/node/src-tauri/src/commands.rs` (`node_backend`) |

This is the first file in `docs/decisions/`. The convention it starts:
`YYYY-MM-DD-<slug>.md`, one decision per file, every number measured and its
source named, and a rollback section that says exactly what to change. A
decision that is later reversed gets a new file that names this one; this one
is not edited to agree with it.

## Context

BTX replaced its proof of work at mainnet block 185,000 with an RC episode
that block validation must recompute on a qualifying GPU. Since the 0.34.5
engine (0.6.15, 2026-08-31) the app has launched every non-Mac host with
`-matmulvalidation=consensus`. The reasoning, recorded in `node.rs` on
2026-08-31, was two measurements taken the same evening on an RTX 3060:

1. In consensus mode the engine admits the card by runtime measurement
   (`admission=self_qualification`, `ready=1`, `cpu_fallbacks=0`) and the
   node advertises `NODE_MATMUL_CONSENSUS`. That still holds. Re-read from
   this box's live signer on 2026-09-15: its `debug.log` says
   `provider=cuda_rc_exact_fused_extract ready=1`, and `/proc/<pid>/environ`
   shows the app launched it with `BTX_MATMUL_BACKEND=cpu`, so the env the app
   sets never decided the device. `docs/gpu-qualification-rtx3060.md` is the
   transcript.
2. Under the trusted-mirror pin the same card sat idle behind a single key,
   "against an attestation supply that measured dead in mid August" (fleet
   mirrors last advanced 2026-08-15 and 08-19).

The second premise has changed. This project's Linux validator has signed
since 2026-09-01, and LuckyPool's node carries attestations too (0.6.21
changelog, measured 2026-09-09 while re-measuring the seed list).

With a live supply, what a host without a capable card gets from consensus
mode is nothing. Upstream `src/init.cpp:2662-2673` (verified identical on
2026-09-15 at our pin `9eb4e005`, at tag `v0.34.6` = `3013c2c2`, and on the
unreleased 0.34.7 branch) grants `NODE_MATMUL_ATTESTATION_ARCHIVE` (service bit
31) only to a node that serves attestations AND is either a trusted mirror or a
local signer whose strict-device provider is ready. A GPU-less host in
consensus mode is neither. It starts degraded (`MatMul RC DEGRADED START`),
follows headers, stalls below the Epoch-A height, and can never advertise the
archive bit. Its operator believes it helps.

Meanwhile the role btxscan asked for needs no GPU and no key.
`src/node/matmul_trusted_attestations.h:317` opens
`TrustedSignerMayServeGetMmAttest` with `if (!has_local_signer) return true;`:
a KEYLESS node serves `GETMMATTEST` history with no window limit, where a node
holding a key is clamped to the last 16 blocks (0.6.22 recorded this about our
own signer). On 2026-09-13 the explorer sat frozen for 21 hours for want of one
historical signature that no peer it asked would serve. A keyless mirror would
have answered.

`docs/fleet-proposal.md` names the tiers. Keeper and Archive are the ones the
network is shortest of, and both say "No GPU". Today the app cannot produce
either on a PC without a card.

## The two options from #90

**Option A, the status quo.** Every non-Mac host in consensus mode. A capable
card validates independently, which is the only posture that produces evidence
(bit 27) rather than an echo. A host without one stalls in plain sight where
`rc_stalled` names it, and "loses nothing" because the quorum it would follow
was dead anyway. The cost, now that the quorum is alive: such a host follows
nothing past 185,000, serves no history, and can never hold the archive bit.

**Option B, this decision.** A host with no usable GPU launches as a keyless
trusted mirror on the three pinned signer keys at threshold 1, with the
`-allowsinglekeytrustedmirror=1` override the 0.34 engine requires. It follows
the signed chain, serves history at any height when serving is on, and
advertises bits 25 and 31. The cost is the one in the next section.

A third end exists and is not ours to take: a golden manifest row for the
common consumer device classes, which would turn self-qualified nodes into
verified ones and needs upstream. It is the only end that produces more
independent validators, and this decision does not move it either way.

## Decision

A non-Metal host on a degraded-start engine (0.34.5 and newer) is split on
what its backend says:

| backend | meaning in the node app | launch mode |
|---|---|---|
| `Metal` | macOS on Apple Silicon (compile time) | unchanged: no `-matmulvalidation` flag unless the `.matmul-consensus-refused` marker is set, then the mirror with the override |
| `Cuda` | the NVIDIA driver's CUDA library is present: `%SystemRoot%\System32\nvcuda.dll` on Windows, `libcuda.so.1` on Linux (known paths, then `ldconfig -p`) | unchanged: `-matmulvalidation=consensus`, explicitly; `-matmulrcexecution=strict-device` |
| `Cpu` | no NVIDIA driver, so no GPU btxd could ever qualify | **new**: `-matmulvalidation=trusted`, the three `-matmultrustedpubkey=` pins, `-matmultrustedthreshold=1`, `-allowsinglekeytrustedmirror=1`; `-matmulrcexecution=strict-device` |

On engines before 0.34.5 nothing changes: both PC classes keep the mirror
without the override, because those engines exit at init in consensus mode on
an off-manifest host and do not know the override flag.

The mode is stated on the command line for every non-Metal host, never left
to the engine's default. That is the 2026-09-01 lesson: btxd persists
`matmulvalidation` in the datadir's `btx_rw.conf` and loads it on every start,
a command-line value outranks it, and 0.6.15 died within five seconds on every
install that had run the mirror era because it passed nothing.

The rule in code, `crates/btx-core/src/node.rs`, `build_node_command`:

```rust
let degraded_start = node_allows_degraded_matmul_start(btxd);
let cuda_validates_here = degraded_start
    && matches!(backend, Backend::Cuda)
    && trusted_mirror_override() != Some(true);
let mirror_here = trusted_mirror_required(backend, datadir) && !cuda_validates_here;
let consensus_here = !matches!(backend, Backend::Metal) && degraded_start && !mirror_here;
```

`trusted_mirror_required` is unchanged (`trusted_mirror_enabled(backend) ||
matmul_consensus_was_refused(datadir)`), so the Metal marker path and the
older-engine paths are exactly what they were. The only thing the split adds is
that a `Cuda` host on a degraded-start engine is exempted from the mirror, and
therefore states consensus.

Before this change the app could not make the split at all:
`apps/node/src-tauri/src/commands.rs::node_backend()` returned `Backend::Cpu`
on every non-Mac host at compile time, and `node.rs` said so in a comment
("the node app never selects Cuda today"). Implementing the decision at the
`Backend` level without changing that would have put every PC on the mirror,
this project's RTX 3060 signer included. So `node_backend()` now calls
`btx_core::backend::node_host_backend()`, which asks for the driver library.

Why the driver library and not the miner's probe: `detect_backend` asks
`btx-matmul-backend-info`, which applies the matmul library's own device floor
and would read a Pascal card as CPU. The node packages do not ship that tool.
`BUNDLED_NODE_BINARIES` lists it as best-effort and
`apps/node/scripts/stage-node-pkg-linux-source.sh:97` copies `btxd` and
`btx-cli` only (the Mac script likewise). The driver library is the signal
every install already has, with no new binary and no new dependency. Both
files were measured present on this project's 3060 box on 2026-09-15:
`C:\Windows\System32\nvcuda.dll` (4,714,728 bytes) and, in WSL2,
`/usr/lib/wsl/lib/libcuda.so.1` listed by `ldconfig -p`.

## Risks traded

**At threshold 1 every pinned key is a full authority alone, so one stolen
signing key could make these nodes accept MatMul-invalid blocks; btxd itself
warns about this at init.** Verbatim from the shipped engine, on stderr and in
`debug.log`, at every start of a CPU host under this decision:

```
[warning] This node is a single-key trusted MatMul mirror on mainnet (3 signer(s), threshold 1)
started only because -allowsinglekeytrustedmirror=1. Above the Profile-1 activation height
the attestation quorum REPLACES ExactReplay. Anyone who steals that key can make this node
accept MatMul-invalid blocks. Configure a second independent signer with
-matmultrustedthreshold=2 and drop the override.
```

Threshold 2 is not available to us: measured 2026-08-12, the published signers
attest different blocks, so M=1 is a union that rejects nothing while M=2
demands both signatures on one block and rejects most of the chain
(`BTX_TRUSTED_ATTESTATION_PUBKEYS` in `node.rs` has the table). Upstream's
own guidance in issue 139 is to keep home signers at threshold 1 until the
restart double-attest is fixed.

**Mirrors also freeze when signers go quiet (2026-09-05/06 incidents) rather
than validating on their own.** `docs/incident-2026-09-05-fork.md` and
`docs/incident-2026-09-06-bodyless-tower.md` are the two days. A mirror has
nothing outside itself to compare a block against; `docs/fleet-proposal.md`
calls this the trap and says several mirrors following one source are one
source wearing several hats. A CPU host under this decision is such a node,
and the status card's "Mirror" copy says so in plain words: for the one check
it is trusting the signers, not verifying.

**So the operator trades this:** a node that stalls for certain below 185,000
and serves nobody, while looking like it helps, for a node that follows the
signed chain, can serve history at any height, and freezes when the signers go
quiet. The first failure is silent and permanent; the second is loud
(`getmatmulattestedtip`, the stall watchdog, `archive_authority == 0` on the
card) and clears when a signer returns.

**What it does not trade.** No CUDA host loses independent validation; no Mac
changes; no engine before 0.34.5 changes. The 08-31 measurement on the 3060
is the reason the CUDA arm exists.

## The gap this does not close

`Backend::Cuda` in the node app means "the NVIDIA driver is installed", not
"the card qualifies". A Pascal, Turing or weak Ampere card carries the same
driver, fails the startup canary, and its host takes the consensus arm and
stalls exactly as it does today, with the card reading "Stopped". The signal
that would route it lands only after start: `node::node_rc_status` reads
btxd's own policy line, `strict-device ... ready=0
reason=no_rc_self_qualified_device_backend`. Turning that into a sticky
per-datadir marker and a restart as a mirror, the way
`record_matmul_consensus_refused` already does for a Mac the engine refuses,
is the refinement. It costs one degraded start (the mainnet canary runs
minutes) per datadir, once, and it is not in this change.

Meanwhile `EASYBTX_NODE_TRUSTED_MIRROR=1` in the app's environment puts such a
host on the mirror by hand: `build_node_command` reads the override for both
arms since this change (before it, the override was ignored on 0.34.5+ for
every non-Metal host, which was the one documented behaviour of that variable
this decision alters).

A second, smaller gap: the Linux path list plus `ldconfig -p` covers Debian,
Ubuntu, Fedora, Arch, WSL2, NixOS and the NVIDIA container layout. A distro
outside that set with a driver would read as `Cpu` and run as a mirror; the
node still works, the card says "Mirror", and the app's stderr says
`[node] host backend: cpu (no NVIDIA driver library found)`.

## How to verify on a real CPU-only host

A PC with no NVIDIA driver (an AMD or Intel GPU, or none). On Linux the
datadir follows `HOME`, so `HOME=$(mktemp -d)` gives a fresh install to watch.
On a GPU box the CPU arm can be exercised with `EASYBTX_NODE_TRUSTED_MIRROR=1`;
the command line is then identical to a driver-less host's except for the
`BTX_MATMUL_BACKEND` env, which steers nothing on a node that never mines.

1. The app's stderr, at start: `[node] host backend: cpu (no NVIDIA driver
   library found), trusted mirror on a degraded-start engine`.
2. `<datadir>/debug.log`, in this order and all present:
   - `Command-line arg: matmulvalidation="trusted"`
   - `Command-line arg: matmultrustedthreshold="1"`
   - `Command-line arg: allowsinglekeytrustedmirror="1"`
   - the `[warning] This node is a single-key trusted MatMul mirror on mainnet (3 signer(s), threshold 1) ...` line quoted above
   - `WARNING: trusted MatMul mirror mode: exact replay authority is delegated to configured signed attestations; NODE_MATMUL_CONSENSUS is disabled.`
   - `MatMul RC execution policy: strict-device provider=not-probed ready=0 reason=non-strict-mode` (this is how a mirror looks; `node_rc_status` reads the `non-strict-mode` token and does not call it a stall)
   - `init message: Done loading`
   - and NOT `MatMul RC DEGRADED START`, which is the consensus arm's line.
3. `btx-cli -datadir=<datadir> getmatmultrustedstatus`:
   `matmul_validation_mode "trusted"`, `trusted_mirror true`, `local_signer
   false`, `threshold 1`, `trusted_signers 3`, `pin_quorum_reachable true`,
   `serves_attestations true` unless the operator turned serving off, and a
   `warning` field carrying the stolen-key sentence.
4. `btx-cli -datadir=<datadir> getnetworkinfo`: `localservicesnames` contains
   `MATMUL_TRUSTED_MIRROR` (bit 25), contains `MATMUL_ATTESTATION_ARCHIVE`
   (bit 31) exactly when serving is on, and never contains `MATMUL_CONSENSUS`
   (bit 27). Measured at genesis on this box: `0x82000d09` with serving on,
   `0x02000d09` with `-matmulattestationserve=0`.
5. The status card reads "Mirror" (not "Stopped"), and `blocks` advances past
   185,000 at whatever rate the signers cover; the 2026-08-12 measurement on a
   parked datadir was 1.59 blocks a minute with both signers pinned.
6. `easybtx.com/api/nodes` lists the node as a mirror, not as an independent
   validator (bit 27 is the discriminator the census decodes).
7. Negative control on a PC with the driver, same build: stderr says
   `[node] host backend: cuda (...)`, `debug.log` says `Command-line arg:
   matmulvalidation="consensus"`, and nothing else differs from 0.6.22.

The Windows classification is a file-existence check on
`%SystemRoot%\System32\nvcuda.dll`. It is compiled in this change and first
executes in the CI Windows build; the file's presence on an NVIDIA machine was
measured here, its absence on a machine without the driver follows from what
installs it.

## Rollback

One flag, at two levels.

- **Operator:** `EASYBTX_NODE_TRUSTED_MIRROR=0` in the app's environment. A
  CPU host then states `-matmulvalidation=consensus`, which is the 0.6.15
  through 0.6.22 posture exactly, leftover `btx_rw.conf` and all.
- **Code:** in `build_node_command`, change `matches!(backend, Backend::Cuda)`
  to `!matches!(backend, Backend::Metal)` in `cuda_validates_here`. That
  restores the 08-31 rule for every non-Metal host. The matrix test
  `the_launch_mode_matrix_is_metal_silent_cuda_consensus_cpu_mirror` then
  fails on its `Cpu` / `v0.34.6` row, which is the point: the test change is
  the whole of the revert, and this file gets a successor that says why.

Either direction is safe on an existing datadir, because the mode is on the
command line and outranks the persisted `btx_rw.conf` value. Moving a datadir
between the arms costs nothing in chain state: a mirror re-acquires
attestations into an archive namespaced by `(chain_id,
replay_authority_context, threshold, signer_set)`, and none of those four move
here (`replay_authority_context` read as
`32ad5c2e148149752a312561dc0b6879c9cc41fdf4bc09edcdd5e2bd09af7188` on the
shipped engine).

## Measurement appendix

Offline, 2026-09-15, this box, the shipped `v0.34.6` engine
(`~/.local/btx/v0.34.6/linux-x86_64/bin/btxd`, the binary the app runs),
scratch datadir on ext4, `-listen=0 -connect=0 -dnsseed=0`, alternate ports,
`BTX_MATMUL_BACKEND=cpu`, and the exact MatMul flags the `Cpu` arm produces.

| run | `Done loading` | `trusted_mirror` | `local_signer` | `serves_attestations` | `localservices` |
|---|---|---|---|---|---|
| default | 2 s | true | false | true | `0x82000d09` (bits 25, 31) |
| `-matmulattestationserve=0` | 2 s | true | false | false | `0x02000d09` (bit 25) |

Bit 27 (`MATMUL_CONSENSUS`, `0x08000000`) was clear in both. Both runs logged
the single-key warning, the mirror banner, and the `non-strict-mode` policy
line, and neither logged `DEGRADED START`. `btxd --help` on the same binary
says `-matmulattestationserve` defaults to 1 under `-matmulvalidation=trusted`,
which the first row confirms.

The live signer on the same box, for the CUDA arm: pid 1669, launched by the
app (`-matmulvalidation=consensus -matmulrcexecution=strict-device`,
`BTX_MATMUL_BACKEND=cpu` in its environment), `provider=cuda_rc_exact_fused_extract
ready=1 cpu_fallbacks=0`, services carrying `MATMUL_CONSENSUS` and
`MATMUL_ATTESTATION_ARCHIVE` (`docs/gpu-qualification-rtx3060.md`).
