# libzync on real GPUs: measured 2026-10-29

**What ran:** the round-13 libzync guest (METHOD_ID `5e460bf3…`, branch `libzync-gpu`) proving real blocks on RunPod
secure cloud.
- Every number below is a measured wall-clock time, and every block receipt **verified**.
- Total spend: **$10.35**.

⛔ This is an experiment. The SwiftSync path still lacks its handover check, and batch verification is unreviewed. These are
**speed** results, not a sound proof of the chain.

## Headline

| block | setup | proving wall (chunks + aggregate) | for comparison |
|---|---|---|---|
| 962,000 | 1× A40 | **1,167 s** (1,016 + 151) | CORE on the same card: **9,990 s** |
| 962,000 | 1× RTX PRO 6000 | **517 s** (452 + 65) | |
| 962,000 | 4× RTX PRO 6000, aggregate spread | **174 s** (141.2 + 32.6) | |
| **969,122** (09-29 tip hour) | 4× RTX PRO 6000 | **197 s** (162.6 + 34.0) | **558.2 s on 38 cards**, 09-29 |
| **969,118** (09-29 tip hour) | 4× RTX PRO 6000 | **220 s** (182.5 + 37.6) | **368.2 s on 38 cards**, 09-29 |

- **Tip blocks run 2–3× faster on about one tenth of the cards.**
- **Cost:**
  - 4 dedicated PRO 6000s cost about **$1.66 per 10-minute block**, against **$15.83 per tip block** on 09-29.
  - Card time alone is **$0.19 per block** on an A40 and **$0.36** on a PRO 6000. That is what a block costs when spare capacity
    does backlog work.
- **vs CORE, measured in seconds:**
  - Card time: **8.6×** (9,990 vs 1,167 s on the A40; the cycle count said 10.7×).
  - Critical path: **13.3×** (max chunk + aggregate = 2,922 vs 219 s).
- **No #119 faults:** 34 single-card proofs plus 48 fleet chunk proofs, all on the first attempt.
- Not included in the times above: the receipt hop via the laptop (16–19 s; pod to pod would be about 1 s) and hint collection
  (see below).

## Detail

**Single card, block 962,000**

| | A40 ($0.59/hr) | RTX PRO 6000 ($2.49/hr) |
|---|---|---|
| build (prebuilt guest, CUDA host) | 956 s on 96 cores | 381 s on 128 cores |
| execute check (one chunk) | journal `98e2652d`, 326,652,314 cycles, **= laptop** | same |
| 16 chunks (≈22.4 M cycles each, 12–14 segments at po2 21) | 61–68 s each, Σ 1,016 s | 27–31 s each, Σ 452 s |
| aggregate (57.6 M cycles, 32 segments) | 151 s | 65 s |
| s per M cycles, chunks / aggregate | 2.84 / 2.62 | 1.26 / 1.13 |

- **A40 per segment:** about 3.5 s per po2-21 segment, plus about 9 s of assembly per chunk.
- **Fixed cost:** small chunks pay a fixed per-process and assembly cost, so they run at 2.84 s/Mcycle against CORE's 2.24.

**CORE baseline:** A40, canonical v0.22.1 host, `37987b85`, the same block in 16 chunks.

| | measured |
|---|---|
| per-chunk cycles | 172–255 M (Σ 3,369.5 M, matching the laptop) |
| 16 chunks | 389–558 s each, **Σ 7,626 s** |
| aggregate (555 segments) | **2,364 s** |
| whole block, one card | **9,990 s** (2 h 46 m) |

**Fleet: 4× PRO 6000, 4 separate single-card pods**
- No 2- or 4-GPU PRO 6000 box was available on secure cloud.
- Chunks:
  - pod k proved chunks i ≡ k (mod 4);
  - receipts went to pod A;
  - pod A ran `seg-serve` (HAZYNC_AGG=1, HAZYNC_SEG_REMOTE=1);
  - each of the 4 pods ran one `seg-connect` worker; A's own connected over loopback.
- Every worker attached over A's public port. No hairpin problem with these four IPs.

| block | per-pod lane time (A / B / C / D) | aggregate: segments, segment phase, assembly | aggregate wall |
|---|---|---|---|
| 962,000 | 141.2 / 104.0 / 112.0 / 112.0 s | 32 seg, 22.9 s, 9.0 s | 32.6 s (one card: 65 s) |
| 969,122 | 162.6 / 123.2 / 131.6 / 127.5 s | 32 seg, 22.3 s, 10.1 s | 34.0 s |
| 969,118 | 182.5 / 139.4 / 149.2 / 143.7 s | 39 seg, 26.3 s, 10.1 s | 37.6 s |

⚠ **Pod A was about 25% slower than the others on all three blocks**, and it alone set the chunk-phase wall. That matches
the stable per-card effect found on 09-29 (memory: project_hazync_tip_hour_4_measured). With A at the others' speed, the chunk
phase is about 110–145 s and a tip block takes about 2.5–3 minutes.

**Tip blocks 969,118 and 969,122**
- **Block files:** `mkblock.py` builds each one from the tip bridge's node (`getblock <hash> 3`). As a positive control it
  regenerated `block_962000.json` **byte for byte**, before it was trusted for the tip blocks.
- **Hints:** collected on the pods with the same host.
- **Checks:** both passed the hinted full execute (`ok=true` ×16, aggregate exit 0).

| block | inputs | libzync chunk cycles | ECMH ops | hint collect + check |
|---|---|---|---|---|
| 969,122 | 9,047 | 425.1 M (max 29.4 M) | 5,744 | 314 + 132 s |
| 969,118 | 8,441 | 488.5 M (max 33.1 M) | 13,184 | 231 + 118 s |

## Improvements and next steps

Ranked by expected payoff per unit of work.

### Speed: the tip block's critical path

1. **Overlap the aggregate with chunk proving.**
   - Today the aggregate (33–38 s) waits for every chunk receipt.
   - It needs only the chunk *journals* to execute, and those exist as soon as each chunk is **executed**, seconds into
     the block. The receipts are needed only when its assumptions are resolved, at the very end.
   - So: execute chunks, start aggregate segments at once, prove chunks alongside, resolve last.
   - Expected: the block costs about the chunk phase + 10 s, i.e. about **−25 s per block**.
2. **Balance the chunk phase.**
   - Replace static round-robin with a pull queue (work stealing) over 32 smaller chunks, or weight the lanes by each card's
     measured speed.
   - The slow pod cost 30–40 s per block today.
   - Expected: the chunk phase falls from 141–183 s to about 110–145 s.
3. **Spread chunk segments as well,** not only the aggregate's.
   - One global segment pool for chunks and aggregate removes lanes and stragglers altogether. seg-serve already does
     this for one session.
   - Expected: the block approaches (total segments × per-segment time) / cards + serial floor, i.e. roughly
     **90–120 s on 4 cards**.
4. **Keep the prover resident.**
   - Each `prove-chunk` is a fresh process: it loads the block, builds the prover server, assembles alone (about 9 s on the A40).
   - A long-lived worker per card that takes jobs removes this per-chunk cost.
5. **Move receipts pod to pod,** or let workers upload them straight to the coordinator over the seg-serve connection.
   About −17 s per block today.

### Readiness: things a live tip run needs

6. **Fast native hints.**
   - Hint collection today re-executes every chunk *unbatched* in the zkVM (230–315 s), then runs a hinted check.
   - Both kinds of hint can be computed **natively** from the block + prevouts in seconds:
     - batch hints (R points, valid bits);
     - ECMH adds/subs.
   - Without this, hints cost more than proving.
7. **A block-JSON source at the tip.** `mkblock.py` from a node is enough. It was not timed today: it ran well inside a few minutes per block.
   Fold it into the bridge, which already holds the block and the coins.
8. **A live 1-hour tip run** on 4 cards with this pipeline, measuring end to end against real block arrival, including arming
   and staging overheads. On 09-29 those cost about 86 s and about 110 s per block.

### Coverage and soundness: before any claim beyond speed

9. **The SwiftSync handover check**: without it the SwiftSync path is not sound on its own.
   - Also handle the two BIP30-grandfathered blocks on the agg-lite path.
10. **A review of batch ECDSA/Schnorr verification.** Covers the Fiat–Shamir transcript, hint validity, and the
    fallback cases: 1–66 fallbacks per chunk were seen on the tip blocks.
11. **More blocks,** chosen to stress the cost model:
    - a taproot- and inscription-heavy block;
    - a block with many CHECKMULTISIG inputs (969,118 had up to 66 fallbacks in one chunk);
    - an old, small block.
    Refit the packer if the per-chunk spread grows.
12. **Card choice:** A40 is cheapest per block ($0.19) for backlog proving; PRO 6000 is the tip card.
    - Measure an H100 or B200 on one chunk and the aggregate before buying time on them. Their per-dollar speed is
      unmeasured on this guest.

## Files

- **Logs and receipts (laptop):** `~/libzync-gpu/`.
  - Single-card runs: `outA.tgz` (A40), `outP6K.tgz` (PRO 6000).
  - CORE baseline: `outC.tgz`.
  - Fleet runs: `run/r4`, `run/t969122`, `run/t969118`.
  - Tip-block files and hints: `run/block_96911{8,22}.json`, `run/th12{2,18}.tgz`.
- **Scripts:**
  - In this directory: `gpu-libzync.sh` (one card), `gpu-libzync-multi.sh` (one box, N GPUs; unused: no multi-GPU box was
    available).
  - Under `fleet/`: `run4.sh` with its pod scripts, and `mkblock.py`.
