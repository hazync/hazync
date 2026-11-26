# The guest's memory at OpenVM's 512 MiB ceiling (roadmap 1.8; 26.11.2026 evening)

OpenVM's guest address space is 512 MiB at most (pointer_max_bits <= F::bits() - 2 = 29; specs/openvm/isa.mdx): a hard
design limit, not a setting. What the guest uses of it, per chunk, on the final image's code (node format 2 + #794's
C-heap recycling), measured on CPU by execution (heap_per_chunk.sh, heap.out).

**Measure.** Metering images (`bench-noverify`: keys-rtchb = keys-rtch + the feature, regtest; keys-chb, mainnet) print
HZHEAP = the bump allocator's position at the end minus its position before the input is read. The bump never moves
back: Rust allocations are never reused and the C heap's freed blocks are recycled from their size class, so the bump
only advances when the live set outgrows everything allocated before. **HZHEAP is therefore the heap's high-water mark
(an upper bound on the heap in use at any moment), including the input's own copy.** Not included: code, static data and
the stack (fixed by the image, not by the block).

| block | chunks (the proof's own chunking) | worst chunk heap | its input | of 512 MiB |
|---|---|---|---|---|
| mem_k1140 (regtest attack: memory) | 16 | **62.9 MiB** | 11.9 MiB | 12.3% |
| tap_full (block 127, attack: one tapscript) | 1 | 36.5 MiB | 3.8 MiB | 7.1% |
| light_only (attack) | 16 | 20.0 MiB | 1.2 MiB | 3.9% |
| tap_small (block 127's one input: ran OUT of 512 MiB on the bump allocator) | 1 | 4.6 MiB | 0.5 MiB | 0.9% |
| mainnet 962,000 (honest, 1 chunk in this export) | 1 | 25.7 MiB | 2.5 MiB | 5.0% |
| mainnet 364,292 (edge set, 16 chunks) | 16 | 11.8 MiB | 1.2 MiB | 2.3% |
| mainnet 419,328 / 481,824 / 388,381 / 227,931 / 185,956 | 4 / 4 / 2 / 1 / 1 | 6.1 / 4.9 / 4.0 / 2.3 / 2.2 MiB | | |

**Reading.** With the C heap, a chunk's memory follows its LIVE data (the input stream and the script interpreter's
working set), no longer the sum of everything the interpreter ever allocated: tap_small went from > 512 MiB to 4.6 MiB.
The worst block measured (mem_k1140, built to exhaust memory) leaves ~8x headroom. The heap scales with the chunk's
input size (about 5x the input on the attack blocks, 10x on 962,000's single chunk), and a chunk's input is bounded by
the chunking, so the lever if a block ever approaches the ceiling is more chunks (finer cost-based splitting), with the
documented fallback of splitting one input's script check across proofs (decision 26.11).

**Design statement (proposed for the docs):** the guest must fit one chunk in 512 MiB; with C-heap recycling the measured
worst case is 62.9 MiB (mem_k1140), and every adversarial block in the rig's set fits in one chunk-split proof.

## A second finding: the cost model misjudges heavy scripts (roadmap 5.1, user item 10)

The chunker balances PREDICTED cost (chunks::input_costs). On these blocks the prediction is far from the measured cost:

| block | predicted | measured (engine cost, this run) |
|---|---|---|
| tap_small (1 input) | 1,507,256 | 26,084,647,308 (17,300x) |
| tap_full (1 input) | 8,477,256 | 207,857,104,851 (24,500x) |
| mem_k1140 (16 chunks of EQUAL predicted cost) | 712.7 G in all | chunks 0-2: 283.6 / 280.7 / 222.0 G; chunks 3-15: ~4.75 G each |

So the model does not see a script's execution cost (tapscript loops, memory-heavy scripts). Correctness is unaffected
(every chunk proves the same rules; memory is shown above), but the split is badly unbalanced on adversarial blocks: one
chunk then dominates the proving time. To look at with the ~50 extra blocks of 5.1: a cost term from the script's
opcode count / executed size, or a measured re-split.

**Why (prover/host/src/chunks.rs `input_costs`).** The hand-set model is base + signature operations
(`predicted_sig_ops`) + bytes x COST_PER_INPUT_BYTE + legacy-sighash bytes. Nothing prices EXECUTING the script: a
3.95 MB tapscript leaf of OP_3DUP / OP_2DROP (each copying 520-byte stack items) costs only its bytes. Script has no
loops, so the executed opcode count is at most the script's length; the per-opcode cost depends on the items it moves.
Proposal (for 5.1, host-side scheduling only, no soundness effect): charge the bytes of NON-standard executed scripts
(tapscript leaves, P2WSH / P2SH redeem scripts) at a measured rate, about 26.1 G / 3.95 MB ~ 6,600 per byte from
tap_small (to fit on more blocks: tap_full gives 208 G for its leaf); standard spends carry scripts of a few dozen bytes,
so their predictions barely move. To check with the fitted model (`HAZYNC_COST_MODEL=fit`, chunks.rs) on the 5.1 blocks.
