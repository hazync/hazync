# The board's host commands on the engine: CPU run of mainnet blocks 1..4 (27.11.2026)

engine/STAGE3.md 3.2. The script is `b32.sh` here, run on hazync-coord (16 cores, CPU only, `nice`, RAYON_NUM_THREADS=8)
at commit 69e7e0ae, on keys-ch: the image of the engine chain 1..200 (c7fe0eb85d2d4c10532742191b059a2adcc3e24e22aa81c1e632e2ee718d2fc9,
key pin b5b94f9c0eb4e4e90ee3653da7b1f521b056a994b6d486b90e1e19622a16da7a, 96,425-byte key from `hzengine pack --key`).
The board's side: HAZYNC_ENGINE_VK + HAZYNC_ENGINE_VK_PIN; the prover's side: HAZYNC_ENGINE + HAZYNC_ENGINE_KEYS.

## Measured

| step | result | time |
|---|---|---|
| `host method-id` (board env, and prover env) | `METHOD_ID c7fe0eb8...` both | |
| `host seg-po2` (prover env) | `engine` | |
| prep 1..4 with leaf bundles | leaf_1..4.bin | 0 s |
| `prove-range-bridge 1..4` (leaf: trie + aggregate, the aggregate verifying the trie proof in-guest) | range_<h>.bin, HZE1, 274,327 / 274,463 / 274,471 / 274,479 bytes | 451 / 448 / 448 / 447 s (trie ~116 s, aggregate ~264 s, rest the engine's key load) |
| `verify-any range_<h>.bin <h> --check-end` | RANGE-OK, end=board, all 4; block 1 anchored=1 | 41 ms (range 1..4) |
| board chain check | out_tip(h) == in_tip(h+1) for h = 1..3, genesis tip 852c5c7e (the RISC Zero board's genesis tip: same state layout) | |
| `fold-range range_1.bin range_2.bin fold_1_2.bin` (HZE1 in, HZE1 out) | RANGE-OK lo=1 hi=2 anchored=1 end=board | 282 s |
| `fold-range range_3.bin range_4.bin fold_3_4.bin` | RANGE-OK lo=3 hi=4 end=board | 290 s |
| `fold-range fold_1_2.bin fold_3_4.bin range_1_4.bin` (folds of folds) | RANGE-OK lo=1 hi=4 anchored=1 end=board; no proof pairs left in the work dir | 291 s |

Refused, each REJECT with exit 1:

| input | reason printed |
|---|---|
| range_2.bin, a journal byte changed | the proof's public values are not SHA-256 of this journal |
| range_2.bin, a proof byte changed | the STARK does not verify against the image's key |
| range_3.bin claimed from block 2 | not anchored on the state before block 2 |
| fold_3_4.bin claimed from genesis | not anchored on the state before block 1 |
| a wrong pin | HAZYNC_ENGINE_VK is not the pinned key |
| another image's key (the header zAPP's, pinned correctly) | does not verify against this coordinator's engine image 163d229d... (MISMATCH): the proof does not decode |
| an engine file on a RISC Zero board (no HAZYNC_ENGINE_VK) | an engine (OpenVM) proof file, but this coordinator verifies RISC Zero receipts (METHOD_ID MISMATCH) |

## Found by this run, fixed (69e7e0ae)

The first run (faea28d2) proved the 4 leaves, but verify-any --check-end refused every one ("not the board's", the
tips equal) and the first fold's guest exited 1. Cause: `stamp()` gave prep and leaf states the RISC Zero METHOD_ID as
self_id, so each leaf was anchored on a state the engine chain never commits. An engine host now stamps the engine
image id and a zero lineage. The rerun above is on the fix.

## Not run here

The coordinator itself (server.py) with a worker: the worker's part is covered by coordinator/test_worker_engine.py
(one attempt on an engine host) and the 13 worker tests; server.py's config (the engine image id as /api/meta's
method_id, verify-any with the engine env) is the dry run's (Gate 3).
