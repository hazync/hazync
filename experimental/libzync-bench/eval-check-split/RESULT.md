# eval_check split: CPU equivalence, GPU re-profile, fixed cost (23.11.2026)

Follow-up to the local study `risc0-underuse-23.11.2026.md` (levers K0, K2, and "fixed term per proof"). Branch
`perf/eval-check-split`. **MEASURED** = read from a log or file in `raw/`. **ESTIMATED** = my arithmetic on
measured numbers, with the basis given. Prover-only: METHOD_ID `46666b6f…` is unchanged (`raw/pod/method_id_branch2.txt`),
and every receipt was checked with the unmodified v0.27.1 verifier.

## Answer in brief

- **The split works and is bit-identical.** On CPU it matched stock on 2 × 16,384 random rows and on all 262,144 rows of a real
  eval_check input. On the GPU it matched on all 84 eval_check calls of blocks 300,000 and 200,000. All MEASURED.
- **It makes the rv32im `eval_check` kernel 10.5% faster** (median 395.4 ms down to 353.8 ms per po2-21 call; 12.29 s down to
  10.96 s per 300,000 block, MEASURED). **Wall time barely moves: −0.5% on 300,000 and −1.0% on 200,000**, which is within run-to-run
  noise. The reason is the second finding.
- **On this card the GPU is idle about 72% of a block's wall time.** For 300,000 the kernels add up to 28.5 s of a 117 s run, and
  the first CUDA call comes 18.4 s after the process starts (nsys, MEASURED). rv32im eval_check is 43% of kernel time but only
  10.5% of wall time. So kernel levers (K2, K3, K4) are capped at a few percent of wall time until the CPU side overlaps with the
  GPU. That CPU-side gap is the bigger lever.
- **Per-process fixed cost is about 8 s per proof** (MEASURED): blocks 1 and 2 in one process took 45.4 s, against 27.1 + 26.6 =
  53.7 s as two processes. That is ~30% of a tiny block and ~7% of 300,000. **ESTIMATED ~1.9% of whole-chain card time**
  (8 s × 976,724 blocks ≈ 7.8 M s of the 417 M s projection). This is only part of the 23.77 s fitted fixed term.
- **Spend: $3.99 in total.** RunPod balance went from 7.46 to 3.47. Both pods are terminated, and RunPod lists 0 pods.

## 1. The split (design)

`vendor/risc0-circuit-rv32im-sys/split/gen_split.py` is a source-to-source tool. It reads the zirgen output, either
`kernels/cuda/eval_check_[0-3].cu` or the C++ twin `kernels/cxx/rust_poly_fp_[0-3].cpp`, and writes one translation unit:

- The 21-function tail-call chain (poly_fp → rv32im_v2_19 → … → 0) is cut after **v2_18, v2_14, v2_10 and v2_4**. That gives
  5 pieces of 3, 4, 4, 6 and 4 functions. Each piece is one `__global__` kernel, and its functions are `static __forceinline__`,
  so no ABI frames remain inside a piece.
- Two kinds of value cross a cut through a global SoA **carry** (one column per carried Fp, one row per domain row):
  - the cut call's by-value FpExt arguments;
  - every pool slot (`x33`/`x34`) written in an earlier piece and read in a later one. The reads become carry loads, and the
    writer also stores the value.

  Columns are reused by interval allocation: **416 columns = 1,664 B/row**. The launcher runs all 5 pieces over a 2^20-row
  chunk, then moves to the next chunk, so the carry is 1.74 GB however big the segment.
- Before writing anything, the tool checks the chain shape: write-once slots, a read always after its write, the tail-call
  targets, and the pointer roles. It refuses otherwise. Re-run it whenever risc0 regenerates the files.
- Selection happens at run time in `ffi_supra.cu`:
  - `HAZYNC_EVAL_CHECK` unset or `stock`: the stock kernel, so default behaviour is unchanged;
  - `split`: the split kernels;
  - `compare:split`: runs both and fails the prove if any row of `check[]` differs.

  `HAZYNC_EVAL_CHECK_LOG=1` logs the ms of every call, and `HAZYNC_EVAL_CHECK_DUMP=<dir>` writes one call's real inputs for the
  CPU test.
- **Dropped variant (MEASURED failure):** `--inline-mul` undefined `__CUDACC_RDC__` in its TU. The aim was to make sppark's
  FpExt multiply inline: it is a `__noinline__` call under `-rdc` (`vendor/sppark/sppark/ff/baby_bear.hpp:398`). A binary with
  those TUs failed the stock `cudaMemcpyToSymbol(poly_mix)` with "invalid argument":
  - nsys `cuda_api_trace`, Result=1;
  - the sticky error surfaced in `hash_rows` and in the split launcher;
  - without them the same copy succeeds and stock check[] matches the CPU.

  Inline FpExt multiplication therefore needs a source rewrite, not the macro. That branch state is recorded in commit
  7a8079ff, and the logs from the failed first build are in `raw/pod/old1/`.

## 2. CPU equivalence test

`split/test_split_cpu.sh` compiles the stock C++ twin together with the split C++ that the tool emits from those same files.
It runs stock poly_fp per row, then the pieces chunk by chunk with the carry, and compares the raw Montgomery words.

| input | rows | split == stock | extra check |
|---|---|---|---|
| random, seeds 1 and 2 (laptop and pod) | 16,384 each | all rows | — |
| random, 21 pieces (a cut after every function) / 3 pieces (laptop) | 8,192 each | all rows | — |
| **real**: block 170's first eval_check call, dumped by the branch's CUDA host (po2 16, domain 262,144) | 262,144 | **all rows** | CPU stock check[] = GPU stock check[] on **all** rows |
| negative control: one carry load index changed by 1 (laptop) | 8,192 | **0 rows** (fails as it should) | — |

Logs: `raw/pod/cpu_test.log` and `raw/local/cpu_test_laptop_random.log`. The first pod attempt (`raw/pod/old1/cpu_test.log`)
also had split == stock on all 262,144 rows. There, however, the GPU-check cross-check differed on every row. That was the
poly_mix copy failure above: the GPU's stock check[] had been computed with a missing poly_mix. It is fixed in the rebuild.

## 3. GPU numbers (pod r8nxblvfvt4afk)

The pod:
- RunPod SECURE, NVIDIA RTX PRO 6000 Blackwell Server Edition, driver 595.112.07, AMD EPYC 9554, $2.69/h.
- Stock = the v0.27.1 release CUDA host (sha OK against SHA256SUMS.txt, whose hash matched the PGP-checked one; method id 46666b6f).
- Branch = 7a8079ff, built on the pod with CUDA 12.8 and `-arch=native` (sm_120), embedding the canonical guest, which was cut
  out of the release CPU host and checked against its pinned sha256 f7b87bf1….

### Wall time (MEASURED, `raw/pod/steps.tsv`, phases in `*.times.tsv`)

| block | release (A) | release again (B) | branch stock | branch split | split vs branch stock |
|---|---|---|---|---|---|
| 170 | 37.2 s | — | 37.3 s (warm-up) | — | — |
| 300,000 (32 segments) | 116.0 s | 116.2 s | 117.2 s | 116.6 s | −0.6 s (−0.5%) |
| 200,000 (52 segments) | 166.3 s | — | 166.0 s | 164.3 s | −1.7 s (−1.0%) |

Run-to-run spread on the same binary is ~±1 s (release 300,000: 116.0 and 116.2 s). This pod is ~13-17% slower than d7's
(102.5 / 142.6 s on an EPYC 9555 host), which fits the CPU-bound picture in §4.

### Kernel time (MEASURED)

| | branch stock | branch split |
|---|---|---|
| rv32im eval_check per po2-21 call, median (host-timed, `HAZYNC_EVAL_CHECK_LOG`) | 395.4 ms | **353.8 ms (−10.5%)** |
| rv32im eval_check per block 300,000 / 200,000 (32 / 52 calls) | 12.29 / 19.67 s | **10.96 / 17.53 s** |
| nsys 300,000: all kernels | 28.55 s | 27.22 s |
| nsys 300,000: rv32im eval_check (stock 12.886 s "eval_check" minus 0.598 s of recursion eval_check / sum of the 5 pieces) | 12.29 s | 10.97 s |
| nsys 300,000: eval_check share of kernel time | 43% | 40% |

Split pieces (nsys, 300,000): 1.35 / 2.19 / 2.36 / 2.41 / 2.66 s. Other kernels are unchanged: `par_stepExec` 2.02 s
(7.1%), `parStepExec` 2.11 s, `stepAccum` 2.88 s, `_poseidon2_rows` 2.96 s. The release binary's profile is the same as the
branch's stock profile (28.54 s kernels, eval_check 45.2%).

The compare-mode `variant_ms` is not kernel time: it includes a tmp buffer, memset and two 134 MB copies. Do not read the
compare logs as timings.

### Registers and spill (MEASURED static, `cuobjdump --dump-resource-usage`, sm_120; `raw/pod/prof/resusage_*.txt`)

| kernel | REG | STACK (B/thread) |
|---|---|---|
| stock rv32im `eval_check` (release and branch) | 255 | 21,392 (the whole call chain's frames) |
| split piece 0 / 1 / 2 / 3 / 4 | 255 each | 1,288 / 1,536 / 2,176 / 2,640 / 2,840 |
| `par_stepExec` (unchanged) | 255 | 3,424 |

ptxas's own spill bytes, from NVRTC 12.8 for sm_120 on the laptop (`raw/local/nvrtc_*.log`, same front end):
- **stock**: each of the 21 chain functions has 536-976 B of spill stores, plus a matching amount of loads, about 13 KB in all;
- **split pieces**: 1.5-5.4 KB of stores and 2.5-8.1 KB of loads each, 16.7 KB and 26.4 KB in all.

So the call-chain ABI traffic did go away: the stack fell from 21.4 KB to at most 2.8 KB per thread. Within each piece the
inlined live set now spills instead, and the net is the 10.5% above. **Dynamic local traffic and occupancy were not measured:**
ncu got `ERR_NVGPUCTRPERM` on this pod (no GPU performance-counter access), so `raw/pod/prof/ncu_*.csv` are empty.

### Receipts

All 19 receipts the pod made pass the v0.27.1 verifier: `SWIFTSYNC_VERIFY_DEMO` (`raw/pod/verify.tsv`). That run's rc column
is not meaningful: a command substitution reset `$?`.

On the laptop, with the v0.27.1 release verifier (sha 9c2183b9…), `--network mainnet --product --anchor … --expect h:hash`,
11 receipts were checked: every split receipt (b_split, cmp_split, nssplit; 300,000 and 200,000) and the stock and release
ones. Each gave rc=2 DEMO, the same as d7/d8. In each, the anchor equals prep's state_{h-1} and the committed state equals
prep's state_h (`STATE_MATCH`) (`raw/pod/laptop_verify.tsv`). The two b_split receipts are in `raw/pod/receipts/`.

## 4. Fixed cost per process

| measurement | result | label |
|---|---|---|
| mainnet blocks 1..2 in one process (`libzync-prove-range 1 2 … genesis`) | 45.4 s: block 1 26.5 s, block 2 18.7 s | MEASURED |
| block 1 alone / block 2 alone (anchor = block 1's state), each its own process | 27.1 s / 26.6 s (sum 53.7 s) | MEASURED |
| 1..4 in one process | 83.1 s: 26.8, then 18.7, 18.7, 18.8 s | MEASURED |
| ⇒ cost paid once per process | **~8 s** (26.6 → 18.7 s); the whole difference sits in the trie/ECMH/aggregate column (15.4-16.0 s → 7.6-7.7 s) | MEASURED |
| block 170 (nsys): first CUDA call at 16.5 s, first kernel 16.6 s, kernels 2.5 s of 37.7 s | GPU busy 12% of the first-to-last-kernel window | MEASURED |
| block 300,000 (nsys): first CUDA call at 18.4 s, kernels 28.5 s of 119.8 s | GPU busy 28% of that window | MEASURED |
| release `method-id` (process start + embedded image) | 0.0 s | MEASURED |

The in-process block 2 is a *recursive* step: it also verifies block 1's receipt, so it does more work than block 2 alone. The
~8 s saving is therefore a lower bound.

What a long-lived prover could save (ESTIMATED):
- ~8 s per proof: ~7% of a 300,000-sized block and ~1.9% of whole-chain card time.
- The 7.8-11.7 s "hints" phase and the 16-18 s before the first CUDA call are CPU work done on every proof. They run serially
  before the GPU starts, and while they run the card is idle. Overlapping them with the previous block's GPU work, by
  pipelining blocks in one process, is the larger lever. This run did not measure that.

## 5. What this means for the levers

- K2 (split) as built: **−10.5% on eval_check, ≤1% on wall**. Worth keeping only alongside work that makes the run GPU-bound.
- K4 (`par_stepExec` buckets): `par_stepExec` is 7% of kernel time, ≈2% of wall. Expect less than 1% of wall.
- Fixed cost and CPU/GPU overlap: the GPU sits idle ~72% of a 300,000 block. This is now the first lever to look at.
- Inline FpExt multiply needs a source rewrite; the macro route fails (§1).

## Files

- `raw/scripts/`: control_es.sh, pod_run_es.sh, pod_run_es2.sh, rpm_es.py, extract_elf.py, resusage.py, nsys_facts.py.
  pod_run_es2.sh is the hand restart after the first build's poly_mix failure, and it ran CPUREAL → DONE.
- `raw/pod/`: steps, logs, times, verify (pod and laptop), compare summary, CPU test, resource usage, nsys kernel CSVs and
  facts, control.log; `old1/` holds the failed first-build logs.
- `raw/local/`: NVRTC ptxas stats and the laptop CPU test.
- Spend: community pod g9g15p72ndwbtt ($1.69/h listing) never published SSH in 12 min and was terminated. The Max-Q $0.50
  listing had no capacity. Secure pod r8nxblvfvt4afk ran 21:51-23:08. Balance 7.4594 → 3.4712 = **$3.99**; 0 pods after.
