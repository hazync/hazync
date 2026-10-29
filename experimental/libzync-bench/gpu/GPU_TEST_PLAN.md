# libzync GPU test — block 962,000, round-13 guest (plan for 2026-10-29)

**Goal:** turn the execute-mode cycle counts in `BLOCK_962000.md` into measured proving seconds and
dollars.
- Today's derived estimate: ~8–11 B200-minutes of card time per block, and ~2–4 min wall clock on
  ~17 cards.
- That estimate rests on ONE old data point (948 M cycles ≈ 915 s on a B200) plus a 14–36% optimism
  warning from `docs/BUILDS.md`. This test replaces it.

**What is NOT measured:** a fleet run (orchestration, joins across boxes, network). Phase A gives every
chunk's and the aggregate's proving time on one card. Chunks are independent, so the fleet critical
path is max(chunk) + aggregate plus orchestration overhead. Phase B, if wanted, measures the overhead.

## Spend (needs the user's OK on the day)

| phase | what | cards | est. time | est. cost |
|---|---|---|---|---|
| A | build + execute check + prove 16 chunks + prove aggregate | 1 | ~2–3 h (build ~1 h of it) | ~$1–2 at A40 $0.49/hr |
| C (optional) | same card: prove 2 chunks with the CANONICAL release host (CORE) for a seconds-for-seconds baseline | 1 | ~0.5–1 h | ~$0.50 |
| B (later, optional) | 16 + 1 cards in parallel, real wall clock with orchestration | 17 | ~0.5 h after binaries exist | ~$4–7 |

Prices are 2026-09 RunPod figures (A40 $0.49/hr, RTX 4090 $0.74/hr) and must be rechecked at rent
time.

**Card: A40 (48 GB).**
- The CUDA prover has OOMed near ~22 GB (memory: gotcha_cuda_oom_is_build_dependent), which a 24 GB
  4090 makes tight.
- The 966,256 milestone fleet was A40s, so the numbers compare to history.
- Fallback: a $0.74 4090.
- ⛔ `cloudType: SECURE` only. Community hosts mostly never start (memory:
  gotcha_runpod_community_hosts_mostly_never_start).

## What ships to the box (built on the laptop tonight)

The guest is NOT rebuilt on the GPU box. `methods/build.rs` already supports `HAZYNC_GUEST_ELF`:
- The host embeds a prebuilt guest and REFUSES it unless its sha256 and image id equal
  `reproduce/GUEST_ELF_SHA256` and `reproduce/METHOD_ID`.
- Branch **`libzync-gpu`** = `libzync` with those two pins set to the round-13 guest.
- Result: the box needs no Core / secp trees and no riscv toolchain, and its cycle counts must equal
  the laptop's exactly.

Bundle (`libzync-bundle.tgz`, ~80 MB, scp from the laptop or via B2):

| file | what |
|---|---|
| `method.bin` | the round-13 guest, exactly as measured (id and sha in the pins) |
| `block_962000.json` | the block |
| `h1/` | batch hints for the whole block as ONE chunk (execute check) |
| `hg/` | batch hints for the 16-chunk fitted packing |
| `ecmh_g.bin` | SwiftSync ECMH hints for the aggregate |
| `fit_env.txt` | the round-13 packer fit (decides the 16 chunk boundaries) |

## Run (on the pod)

1. RunPod pytorch image (has CUDA; nvcc is off PATH — memory: gotcha_runpod_nvcc_is_a_path_problem).
   Check that the pod really started: `runtime.uptimeInSeconds` and published ports, not
   `desiredStatus`.
2. `git clone -b libzync-gpu …hazync && cd hazync && GPU=1 HAZYNC_PROVISION=deps ./provision-vps.sh`
   (the proven toolchain path: rust, protoc, CUDA env).
   ⛔ `cargo` is not on a non-interactive ssh PATH: run inside a login shell / tmux.
3. Unpack the bundle to `/workspace/libzync-bundle`.
4. `BUNDLE=/workspace/libzync-bundle ./experimental/libzync-bench/gpu/gpu-libzync.sh` inside tmux. It:
   1. builds the CUDA host around the prebuilt guest; asserts libcuda is linked, METHOD_ID = pin, and
      the agg-lite guest strings are present;
   2. **execute check:** whole block as one chunk; must give journal `98e2652d…` AND exactly
      **326,652,314** cycles (= laptop). Anything else stops the run before any proving;
   3. proves the 16 chunks one by one (#119 retry; times only the successful attempt; logs VRAM and
      power every 2 s);
   4. runs the aggregate from the REAL receipts in execute mode first (stream check), then proves it
      and verifies the block receipt;
   5. prints the sums, the one-card wall clock, the fleet critical path (max chunk + aggregate) and
      the card-seconds per block.
5. ⛔ **Harvest before releasing the pod:** tar the output dir and scp it back. (Memory:
   gotcha_tip_runs_destroyed_their_own_evidence.)

## What we learn

- **Seconds per million cycles** for chunks and for the aggregate on this card, measured. This
  replaces the 948 M ≈ 915 s data point.
- **Card-time per block** (Σ chunks + aggregate) → $ per block at this card's price.
- **Fleet critical path** (max chunk + aggregate) before orchestration overhead.
- With phase C: the same seconds measured for CORE on the same card, so "N x faster" is in seconds,
  not cycles.

## Known risks

- **CUDA build time** (~1 h of paid idle GPU). Acceptable at A40 prices. Alternative: build on a
  cheap CPU box and copy the binary.
- **#119** (transient invalid receipt): retried and recorded.
- **The aggregate stream on the real-receipt path (`agg-chunks`)** was refactored tonight to share
  `agg-exec-fake`'s code. Laptop-checked only via `agg-exec-fake`, so step 4a's execute-first run is
  the guard.
- **The SwiftSync path still lacks its handover check**, and the two BIP30 blocks are unhandled.
  Irrelevant to timing; stated so nobody cites this as a sound proof of the chain.
