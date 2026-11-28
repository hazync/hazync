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

## The fix on the GTX 1080 Ti: the whole header chain PROVED on Windows (28.11.2026, second Xesktop session)

Xesktop, a fresh 10x GTX 1080 Ti server (Windows 10 Pro, driver 582.28), ONE card (CUDA_VISIBLE_DEVICES=0,
HZ_SEG_MEM_GB=7, no VPMM setting). hzengine.exe from engine-windows run 46152650102 (request 17: stark-backend-msvc.patch +
stark-backend-pascal.patch with the race fix, 308f79a1; exe sha256 a1f0608f...). Input: the header chain 0..975,278
(stream.bin sha256 4bb7efca..., the 26.11 reference's).

- **Proved: 3,894.2 s** (707 segments at the 7 GiB cap; 15:47:30-16:52:30), rc 0; `hzengine verify` on Windows: OK (4.8 s).
- **Verified on Linux** (hazync-coord, hzengine at /root/engine-kit, keys-hdr29): **OK, journal sha256
  20bb754bde3542a970abd49053ba5aeec82fb91123c58910325e4a8c780058d5**, byte-identical to every Linux run (PRO 6000 266.2 /
  268.0 s, A6000, the A40 unfused 756 s). A copy with one digit of the proof changed: refused (rc 1).
- The first engine GPU proof on Windows, and the first on Pascal (sm_61). Peak GPU memory sampled: 11,011 MiB of 11,264
  once (a short spike), otherwise ~6.8-7.6 GiB.
- For scale (same input, measured): RTX PRO 6000 Workstation 268.0 s; RTX A6000 722.0 s; A40 (unfused forced) 756 s.

Files: xesktop2_28.11.2026/ (the journal, the Linux verify lines).

## A real block across the 1080 Tis: 976,379 (8,130 inputs) on Windows (28.11.2026, same session)

`hzengine prove-block --dir keys --export export/976379t` (bundle-def: keys-def image, the 24.11 export with 5 chunks +
the trie), all 10 GPUs visible (workers = GPUs x share; this block has 6 leaf jobs, so 6 workers on GPUs 0-5).

- **Run 1, the automatic segment cap (7 GiB on an 11 GiB card): 4 of the 6 workers died OUT OF MEMORY**
  (`vpmm_set_access failed ... out of memory`, d_buffer.rs:83; block976379_run1_oom_excerpt.txt); stopped by me after ~5
  min. The header chain fit at 7 GiB; chunk proofs need more per segment, and WDDM keeps part of each card.
- **Run 2, HZ_SEG_MEM_GB=4: PROVED in 405.6 s** (16:59:16-17:06:02), rc 0. The 6 leaf workers on GPUs 0-5 (~6.5-7.2 GiB
  each), then the aggregate on GPU 0 with its segments spread over the workers (segpar). Verified on Linux (hazync-coord,
  bundle-def keys): **OK, journal 5,284 bytes sha256 fb8e05582022bd67...**; against the export's RISC Zero journal
  (agg.r0journal): 32 bytes differ (2908-2939), the image's self_id, as on every Linux run of this block.
- For scale (measured, same block): one A40, Linux, 409 s (engine/results/gpu_a40_block976379); one RTX A6000 183-289 s
  by setting (gpu_a6000_tuning).
- To do: the automatic segment cap must leave room on small cards under Windows (7 GiB on 11 GiB fails for chunks; 4 GiB
  works): a lower default below ~16 GiB, or a WDDM margin (engine/host engine.rs segment cap).
