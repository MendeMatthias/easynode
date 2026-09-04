# Qualification transcript: RTX 3060 (sm_86) as a BTX consensus validator

A published transcript from a machine that is doing the job, captured
2026-09-04 14:30 UTC. It is here because "an RTX 3060 qualifies" has been
asserted several times without evidence anyone else can check, and because one
detail in it is worth upstream's attention.

Everything below is `btx-cli` output and `debug.log` lines from a node that was
at tip, signing, and serving attestations at the moment of capture.

## Host

```
os        Ubuntu 22.04.2 LTS  (WSL2, kernel 6.18.33.2-microsoft-standard-WSL2)
glibc     2.35
gpu       NVIDIA GeForce RTX 3060, compute capability 8.6, 12288 MiB
driver    610.74
cuda      13.3.73
```

## Engine

```
subversion       /BTX:0.34.6/
protocolversion  800002
services         WITNESS, SHIELDED, NETWORK_LIMITED, P2P_V2,
                 MATMUL_CONSENSUS, MATMUL_ATTESTATION_ARCHIVE
connections      19 (in 0, out 19)
```

Source build of `origin/release/0.34.6` at `9eb4e005`, installed over the
shipped v0.34.5 binaries. The install directory keeps the v0.34.5 name because
the app resolves its engine from `NODE_RELEASE_TAG` and re-provisions its
bundled package if that name changes — so **the app UI displays v0.34.5 while
running v0.34.6**. Trust `getnetworkinfo.subversion`, never the UI or the
directory name.

## Chain position

```
chain                 main
blocks                210130
headers               210132
bestblockhash         ca6ccf57a0f126cde593b07a94536b10bd72bc12e8c835daf3f47372415d3109
verificationprogress  0.9999792877116979
initialblockdownload  false
pruned                true   (pruneheight 184942, size_on_disk 224.9 MB)
```

## Consensus role — `getmatmultrustedstatus`

```
matmul_validation_mode    consensus
local_signer              true
serves_attestations       true
chain_oracle              true
trusted_mirror            false
threshold                 1
trusted_signers           2
pin_quorum_reachable      true
log_tree_size             3073
stored_blocks             3008
stored_attestations       3008
blocks_with_quorum        3008
accepted                  3073
duplicates                1532
rejected                  1735
capacity_rejections       0
```

## Signed frontier — `getmatmulattestedtip`

```
active_tip_height        210130
active_tip_has_quorum    true
on_active_chain          true
signed_frontier.height   210130
signed_frontier.blocks_behind  0
```

Worth noting for anyone wiring `crates/btx-core/src/frontier.rs`: this RPC
answers usefully on a **healthy** node, not only on a frozen mirror. The
existing caller sits three conditions deep inside the stall watchdog, but the
data itself is available for the price of one `getmatmulattestedtip` per status
tick.

## GPU execution policy, from `debug.log`

```
MatMul RC execution policy: strict-device
    provider=cuda_rc_exact_fused_extract ready=1
    reason=generic_exactgemm_and_rc_self_qualified:canary=missing_golden
    workspace_required=5164972400 workspace_capacity=9663283200
    allow_unverifiable_catchup=0

MatMul RC production canary: outcome=missing_golden
    admission=self_qualification provider=cuda_rc_exact_fused_extract
    family=cuda arch=sm_86 driver=13030 runtime=13030
    epoch_height=185000 profile=1 transcript=1 matmul_dim=4096
    manifest= wall=0.000s device_macs=0 cpu_fallbacks=0

MatMul-v4 mining backend: cuda
    (requested=auto, auto_selected_cuda:imma_s8s8s32_tensor_path:sm_86)

MatMul RC production canary: build provenance is advisory
    (matches=0 dirty=0 fingerprint=a7bf4bd7...)
```

`ready=1`, `cpu_fallbacks=0`, `dirty=0`, `arch=sm_86`. The card qualifies and
the node validates on it.

## The part upstream should read

The 3060 is admitted by **`admission=self_qualification`**, with
`outcome=missing_golden` and an empty `manifest=`. It is not matched against a
golden manifest row, because there is no row for `cuda/sm_86` — the sealed
manifest ships two, `cuda/sm_120` and `metal/m4_class`. This is the same gap
`.github/workflows/engine-fleet-ready-guard.yml` was built to catch, seen from
the other side.

Two honest consequences:

* `wall=0.000s device_macs=0` — the canary did not perform a device measurement,
  because with no golden to compare against there was nothing to compare. So
  this transcript shows the card **admitted and working in production**, not the
  card passing a sealed conformance vector.
* Every non-sm_120 CUDA machine on the network is in the same position. A golden
  row for the common consumer architectures would convert a fleet of
  self-qualified nodes into a fleet of verified ones.

We would rather publish that caveat than a cleaner claim.

## Reproducing it

```bash
btx-cli -datadir=<datadir> getnetworkinfo
btx-cli -datadir=<datadir> getmatmultrustedstatus
btx-cli -datadir=<datadir> getmatmulattestedtip
grep -aE "execution policy|production canary|mining backend" <datadir>/debug.log | tail
```
