# The GTX 1080 Ti (Pascal) on the engine (28.11.2026)

## Xesktop: the machine and what failed

10x NVIDIA GeForce GTX 1080 Ti (11,264 MiB each), driver 582.28, Windows 10 Pro, 2x Xeon E5-2620 v4 (32 threads),
256 GB RAM. hzengine.exe from engine-windows.yml (CUDA build, sm_61..sm_120). Input: the header chain 0..975,278
(bundle_hdr.tgz: keys-hdr29 + export/stream.bin, the 26.11 reference run's input). One card (CUDA_VISIBLE_DEVICES).

1. **VPMM's 8 TB address reservation refused** by the driver ("out of memory" before any work, even for `--help`;
   vm_pool.rs:195). With VPMM_VA_SIZE=64 GiB it runs. Fixed in engine/ci/stark-backend-pascal.patch: the reservation
   is halved until the driver accepts it (>= 16 GiB). Run 2 needed no setting.
2. **The fused bitrev + tree kernel needs 64 KB of shared memory per block** (gkr.cu `_bit_rev_frac_ext_build_k2`,
   cudaFuncSetAttribute); Pascal allows 48 KB: `cudaErrorInvalidValue` in LogupZerocheck(FractionalSumcheck(
   BitReversal)) at segment 0 (run 1, xesktop/hdr1_tail.log; 707 segments at HZ_SEG_MEM_GB=7). Gated in the same
   patch: the fused kernel only when the device offers 64 KB, else the unfused path (engine-windows run 46144629411).
3. **Then NonzeroRootSum at segment 0** (run 2, xesktop/hdr2_tail.log: LogupZerocheck(FractionalSumcheck(
   NonzeroRootSum))): the prover refuses its own sum, so no proof (fails safe).

## Test B: the unfused path on a modern GPU (Linux, RunPod)

Pod 60095ncqy99khy, 1x NVIDIA A40 (46,068 MiB, compute 8.6), SECURE, $0.59/hr; 13:04-13:17 deployed, measured
balance 5.52 -> 5.39 = **$0.13**; terminated, the pod list empty afterwards. hzengine built on the pod (197 s) against
stark-backend v2.1.0 + engine/ci/stark-backend-msvc.patch + stark-backend-pascal.patch (0e889d96), no MSVC build.
Same header-chain input (podB_a40/).

| run | HZ_FORCE_UNFUSED_BITREV | result |
|---|---|---|
| (a) unfused | 1 | **FAILED after 42 s: segment 0 LogupZerocheck(FractionalSumcheck(NonzeroRootSum))**, the same failure as the 1080 Ti |
| (b) fused (control) | 0 | stopped by me after 83 s (rc 143), not completed: the session's usage limit; the fused path's correctness on this input is the 26.11 reference (engine/results/zapp_headers: OK, journal 20bb754b) |

**So the NonzeroRootSum is not Pascal and not Windows: it is the unfused path at large N, on any card.** Upstream
takes that path only for <= 1024 leaves.

## The likely cause (read in source, not yet tested)

frac_build_tree_layer_kernel<revert=false, apply_alpha=true> (gkr.cu ~82-130), in the virtual-input first-layer
branch (`virtual_mode && layer_size == logical_len`), reads `layer[parent_start]` and `layer[parent_start + 1]`
(parent_start = bit-reversed idx * 2) and then writes `layer[idx]` (and a right leaf at idx + half) IN PLACE, ordered
only by `__syncthreads()`, which orders the threads of ONE block. For <= 1024 leaves (upstream's use) that is one
block; at the header chain's sizes it is many blocks, and one block can overwrite a slot another block has not read
yet: wrong layer values, a nonzero root sum.

A proposed fix, NOT applied and NOT tested: PROPOSED_fix_unfused_race.UNTESTED.diff (gkr.cu: a kernel reading every
input from a snapshot copy of the dense layer; the launcher takes it for exactly that branch). To test: rebuild with it,
rerun (a) on a modern card (expect OK, journal 20bb754b), then the 1080 Ti on Xesktop.

## Also running on Xesktop (not finished when this was written)

The CPU build (hzengine.exe, no CUDA) proving block 741,000 (bundle-def keys, export 741000t), started 12:24:09;
see xesktop_28.11.2026/ for its log when harvested.
