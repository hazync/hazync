#!/usr/bin/env python3
"""estimate.py <stats_base.tsv> <stats_a.tsv> <stats_b.tsv>: the GPU batch's time and cost, an ESTIMATE, from the
dev-mode segment counts execute.sh measured (po2 21, the CUDA host's) and these MEASURED rates:

  T_EMPTY  15.1 s  a coinbase-only block, linear prove-range, one RTX PRO 6000, risc0 3.1.0 (mainnet 2..200 under
                   8888d59c, experimental/libzync-bench/RESULT_risc0_3_1_gpu.txt; block 1, the base step, 21.0 s)
  T_SEG    2.2 s   per po2-21 segment beyond an empty block's: block 962,000's chunks, 12-14 segments in 27-31 s each on
                   one RTX PRO 6000 (experimental/libzync-bench/gpu/GPU_RESULTS.md, "Single card, block 962,000")
  T_PROOF  3 s     per proof beyond an empty block's (a chunk receipt's lift and resolution): NOT measured on its own,
                   an allowance
  BRING_UP 25 min  pod setup (CUDA toolkit, rustup) + the CUDA host build (381 s on 128 cores, GPU_RESULTS.md) + a
                   possible first-use kernel JIT on sm_120 (~10 min, docs/libzync/ROADMAP.md O5): an allowance
  PRICE    $2.49/h RunPod RTX PRO 6000, as billed for the 11-11 batch (RESULT_risc0_3_1_gpu.txt)
"""
import sys

T_EMPTY, T_BASE, T_SEG, T_PROOF, BRING_UP, PRICE = 15.1, 21.0, 2.2, 3.0, 25 * 60, 2.49


def rows(p):
    return [dict(zip(("h", "proofs", "segments", "total", "user"), map(int, l.split("\t"))))
            for l in open(p).read().splitlines()[1:]]


base, a, b = (rows(p) for p in sys.argv[1:4])
empty = [r for r in base if r["h"] > 1]
e_seg = min(r["segments"] for r in empty); e_pr = min(r["proofs"] for r in empty)


def t(r):
    if r["h"] == 1:
        return T_BASE
    return T_EMPTY + max(0, r["segments"] - e_seg) * T_SEG + max(0, r["proofs"] - e_pr) * T_PROOF


blocks = base + [r for r in a if r["h"] > 101] + [r for r in b if r["h"] > 110]
prove = sum(t(r) for r in blocks)
print(f"empty block: {e_pr} proofs, {e_seg} segments; {len(blocks)} block proofs (base 1..101, A 102..112, B 111..113)")
print(f"segments {sum(r['segments'] for r in blocks)}, total cycles {sum(r['total'] for r in blocks):,}, "
      f"user cycles {sum(r['user'] for r in blocks):,}")
for r in blocks:
    if r["segments"] > e_seg or r["proofs"] > e_pr:
        print(f"  block {r['h']:>3}: {r['proofs']} proofs, {r['segments']:>3} segments, {r['total']:>11,} cycles -> {t(r):6.1f} s")
for f in (1.0, 1.5):
    s = prove * f + BRING_UP
    print(f"ESTIMATE x{f}: proving {prove * f / 60:.0f} min + bring-up {BRING_UP / 60:.0f} min = {s / 60:.0f} min, "
          f"${s / 3600 * PRICE:.2f} at ${PRICE}/h")
