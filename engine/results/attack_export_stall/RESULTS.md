# The engine-export "ECMH collect" stall on mem_k1140 (28.11.2026)

The 5.1 run (engine/results/cost_model_scripts, exec_attack.sh) saw `host engine-export` of the regtest attack block
mem_k1140 (height 126) write all 16 chunk inputs, print `libzync: ECMH COLLECT (no bundle)`, and then spend over 17 h on
one thread without finishing.

## Cause: an endless loop in the coin-log MODEL, not ECMH work

- `libzync: ECMH COLLECT (no bundle)` is printed by `libzync_agg_tail` (prover/host/src/lz.rs:760) whenever it writes an
  aggregate stream with no hint bundle. `cl_synth_around` (lz.rs:~597) does that deliberately: it removes
  HAZYNC_LZ_HINTS and runs the aggregate as a COLLECT run to learn which coins the block spends. In collect mode the
  guest maps nothing: `swiftsync::point` (prover/methods/guest/src/swiftsync.rs) only logs each leaf (`ECMHREC <hex>`)
  and returns. So the line was the last output before the stall, not its cause.
- After the collect run, `cl_synth` (lz.rs:2529) builds a MODELLED coin log around the spends: its size
  n = 1.64 x (chain transactions before this height), from HAZYNC_COINLOG_TXCOUNTS, and each spent coin is placed at a
  position drawn from its creation height's range, with linear probing on a collision:
  `while !taken.insert(p) { p = (p + 1) % n; }`.
- Measured on the box's real inputs (/root/fresh-bench/hz_txcounts.txt, the mainnet getchaintxstats pairs the 5.1 run
  gave this regtest block; /root/worst-case/run/blocks/mem_k1140/block_126.json): **n = 859 positions for 24,374
  earlier-coin spends, created at 13 distinct heights**. After 859 placements every position is taken and the probe for
  the 860th never ends: an infinite loop on one thread, which is the 17 h "not finished".
- Even with enough positions the probing is quadratic here (every spend starts in the same narrow range): a replica of
  the loop with n = 2 x spends takes 296,972,692 probes (21.7 s in Python).

## Who is affected

- Only the measurement tools that model the coin log: `engine-export`, `agg-exec-fake`, `agg-chunks`, and seg-serve's
  aggregate (seg.rs:1732), each through `cl_synth_around`.
- **Not** real proving: the board, the tip pipeline and the rig prove through `libzync-prove-range` /
  `prove-range-bridge` on prep's REAL coin log (replayed from genesis), which never calls `cl_synth`. The 26.11
  attack-block run (engine/results/q7_attack_blocks, pod/run_rig.sh) proved mem_k1140's pieces that way. So the
  worst-case proving claim (paper §9.1) is not touched by this; what limits mem_k1140 there is guest memory and the
  piece count, as recorded in q7_attack_blocks.
- It does stop cost MEASUREMENT of an attack block through engine-export, whenever the counts file models fewer
  coins than the block spends (any regtest attack block measured with mainnet counts, or a block that spends more
  old coins than 1.64 x the chain's transactions before it).

## Proposed fix (not applied: prover/host is being rewritten on c8-step4)

PROPOSED_cl_synth.diff, two changes in `cl_synth`:
1. The modelled log holds at least twice the block's spends (`n = max(modelled, 2 x spends)`, printed when it grows).
2. Positions by "next free slot" (union-find with path compression) instead of linear probing: the same rule (the
   first free position at or after the drawn one, wrapping), so ordinary blocks get the SAME positions, witness and
   roots as today; only the cost changes, from quadratic to near-linear.

Measured on a replica of the placement loop (Python, the same draws: SHA-256("rank" || i)):

| placement | n | result |
|---|---|---|
| today (linear probing) | 859 | never finishes (stopped after 50M probes) |
| linear probing | 48,748 (2 x spends) | 24,374 positions, 296,972,692 probes, 21.7 s |
| next free slot (union-find) | 48,748 | 24,374 distinct positions, 0.04 s |

Not done: the patch is not compiled or run in the host (the host is mid-rewrite); it should go in on c8-step4 or after
it merges, then engine-export of mem_k1140 rerun to measure the whole export. An alternative that removes the guesswork:
measure attack blocks from prep's real state (libzync-prove-range's export path) instead of a model.
