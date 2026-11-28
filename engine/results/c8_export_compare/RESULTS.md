# CUTOVER C8 (C): libzync-hints and engine-export on the engine vs RISC Zero (28.11.2026)

C8 step (C) (commit 9d8c02a2): with HAZYNC_ENGINE set, `host libzync-hints` and `host engine-export` run every guest
execution on the engine (the chunks, the dry collect run, the trie update and its split parts), none on RISC Zero.
Checked here: the same commit's RISC Zero host (its guest built, `host_r0`) and the engine-only host (`host_e`,
RISC0_SKIP_BUILD) on the repo's five real blocks (prover/block_{130000,140000,741000,962000,965500}.json), every exported
file compared. hazync-coord, in a private mount namespace (the kit at /root/engine-kit), 4 cores.

## Run 1: core image e3729469 (kit 17a685e2, keys built at 818d978b's guest; c1.sh, c2.sh)

c1.sh ran the RISC Zero hints with RISC Zero's default chunking, the engine with its own (the host sets
HAZYNC_CHUNKS=16, HAZYNC_CHUNK_TARGET=900000000 when it runs on the engine or exports): different layouts (block
130,000: 2 chunks vs 1), so every hints-dependent file differed (c1.out). Not a comparison: c2.sh gives the RISC Zero
side the engine's chunking (as era.sh did) and compares again.

c2.out, per block:

| block | chunks | hints bundle | chunk_NN.in, chunk_NN.r0journal | trie.in, trie.r0journal | predicted_cost | agg.in, agg_collect.in |
|---|---|---|---|---|---|---|
| 130,000 | 1 | same | same | same | same | same but the image-id words |
| 140,000 | 1 | same | same | same | same | same but the image-id words |
| 741,000 | 1 | same | same | same | same | same but the image-id words |
| 962,000 | 4 | same | same | same | same | same but the image-id words |
| 965,500 | 5 | same | same | same | same | same but the image-id words |

"The image-id words": agg.in and agg_collect.in differ in exactly 64 bytes, every one inside the two 32-byte places
the host writes its own image id (the stream's bytes 4..36 and the anchor state's self_id): RISC Zero's METHOD_ID in
host_r0, zeros in the engine-only host (no RISC Zero guest; step 4b gives it the engine's id). Checked byte by byte
(every differing byte inside an occurrence of host_r0's METHOD_ID, the engine host's own id at each). The engine
puts its image at 4..36 itself (`self_id`), and a base step does not check the anchor's self_id (R2-2 L1).

Not compared: chunk_NN.cycles (RISC Zero cycles vs the engine's cost, by design) and agg.r0journal (the RISC Zero host
executes the aggregate; the engine export leaves it to prove-block, which verifies its children in-guest).

## Run 2: core image 755f5c5a (HEAD 9f32757a, after the round 3 guest fix 5cb6724e)

(s2.sh: pending; recorded below when it ends.)
