# Why the Linux AppImage is 445 MB, and what actually shrinks it

Measured on the WSL Ubuntu 22.04 box (RTX 3060, sm_86) on 2026-09-04 against
the released `BTX-Node_0.6.15_amd64.AppImage`
(sha256 `b65bcb2344b479c443a65c553acd888047c8cbea58a0f7460cd981444ee2f5b3`;
the filename says 0.6.15, the bytes are 0.6.17).

This document exists because the obvious plan — "ship one AppImage per GPU
architecture instead of five" — turns out to be worth almost nothing, and
somebody was going to spend a week discovering that.

## What the 445 MB actually is

The shipped image is squashfs, zstd, 128 KB blocks, 444.55 MB. Rebuilding the
extracted tree with those same settings reproduces it at 446.5 MB, so the
per-component figures below are measured under the real compressor, not guessed.

| component | compressed | share of download |
|---|---:|---:|
| `libcublasLt.so.13` | 336.2 MB | **75.3 %** |
| `libwebkit2gtk-4.1.so.0` | 31.1 MB | 7.0 % |
| `btxd` | 27.8 MB | 6.2 % |
| `easybtx-node` (the app) | 6.2 MB | 1.4 % |
| everything else | ~45 MB | ~10 % |

Uncompressed the tree is 1.4 GB, and cuBLASLt appears in it **twice** — once at
`usr/lib/` and once at `usr/lib/easyBTX Node/resources/node-pkg/bin/lib/`. The
two are byte-identical (same sha256). This costs nothing: the squashfs
superblock says `Duplicates are removed`, and the rebuild above confirms it.
Not a bug, and not worth a change.

## The per-architecture split does not pay

`btxd` carries our kernels for five architectures. Extracted and measured:

| arch | cubin bytes in `btxd` |
|---|---:|
| sm_75 | 5.42 MB |
| sm_86 | 5.43 MB |
| sm_89 | 5.43 MB |
| sm_120 | 5.33 MB |
| sm_100 | ~0 MB |
| **total** | **21.68 MB** raw, inside a 71.02 MB binary |

So dropping four of the five architectures removes about 16 MB of raw bytes
from a component that is 6 % of the download. Under 2 % of what a user
downloads. The claim that "four fifths is dead weight" is true only of that
6 % slice, and even there sm_100 is already empty.

## The 75 % cannot be pruned the easy way

`libcublasLt.so.13` holds 5,309 cubins across nine architectures. By decompressed
bytes:

| arch | cubins | bytes |
|---|---:|---:|
| sm_120 | 1601 | 212.2 MB |
| sm_80 | 488 | 192.3 MB |
| sm_100 | 1026 | 148.0 MB |
| sm_75 | 166 | 147.9 MB |
| sm_90 | 225 | 77.1 MB |
| **sm_86** | **111** | **54.0 MB** |
| sm_89 | 247 | 25.9 MB |
| sm_103 | 64 | 0.1 MB |
| sm_121 | 4 | 0.0 MB |
| total | 5309 | 1141.8 MB |

An sm_86 machine needs 4.7 % of that by bytes, which looks like an enormous win.
It is not available:

**`nvprune` refuses linked shared libraries.** On the shipped `.so` it exits with
`Input file ... not relocatable`. nvprune only rewrites relocatable objects and
static archives, so the `.so` NVIDIA ships cannot be trimmed after the fact.

Pruning the *static* archive works but disappoints. `libcublasLt_static.a`,
766.7 MB, pruned with `nvprune --arch sm_86`:

* seven of nine architectures are gone — 5,309 cubins down to 488
* sm_80 is **kept**, because sm_86 dispatches through it; sm_80 and sm_86 are
  the two that must stay, and they are among the largest payloads
* the file only falls 766.7 MB → 642.5 MB, about 16 %
* compressed at shipped settings: 287.9 MB, against 336.2 MB for the `.so` today

So the whole per-architecture exercise, and it would mean switching btxd from
dynamic to static cuBLASLt — an upstream build change — buys roughly **48 MB of
445 MB, about 11 %**. That is not nothing, and it is nowhere near the 3× the
plan assumed.

## What would actually work

**Stop bundling cuBLASLt.** Fetch it on first run into `~/.easybtx/`, exactly as
the app already provisions engines — `~/.easybtx/engines/` holds
`matador-0.9.26` and friends next to their `.sha256` siblings, so the
mechanism, the integrity check and the disk location all exist already.

That takes the download from 445 MB to roughly **110 MB for everyone**, and the
~336 MB fetch happens once, only on machines that will actually use the GPU. A
CPU-only machine cannot be a witness anyway and would never pay it.

This has not been implemented or measured end to end. It is a design change to
the app rather than to packaging, so it needs a decision before it needs code.

## Reproducing any of this

```bash
BTX-Node_<ver>_amd64.AppImage --appimage-extract
cuobjdump --list-elf squashfs-root/usr/lib/easyBTX\ Node/resources/node-pkg/bin/btxd \
  | grep -oE 'sm_[0-9]+' | sort | uniq -c
mksquashfs squashfs-root out.sqfs -comp zstd -b 131072   # matches the shipped image
```

`nvprune` is not in a CUDA install that only carries `nvcc`. It can be unpacked
without touching the system:

```bash
apt-get download cuda-nvprune-13-3 && dpkg -x ./cuda-nvprune-13-3_*.deb ./unpacked
```
