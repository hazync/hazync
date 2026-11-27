# The chunker's cost model on the engine: executed scripts, transaction bytes, legacy position (ROADMAP 5.1; 27.11.2026)

Host-side scheduling only (prover/host/src/chunks.rs `input_costs`): what a chunk's inputs are predicted to cost
decides the chunk bounds and, with HAZYNC_CHUNK_TARGET (the engine's 0.9 G per chunk), the chunk count. Nothing of
it reaches a guest input or a journal except through those bounds (its callers: pack_chunks, nchunks_for, the OOM
re-split, bench.rs and engine-export's printed total).

## What was wrong (measured)

1. **No executed-script term.** tap_small (one input) was predicted at 1,507,256 and costs 26,084,647,308 on the
   engine; tap_full 8,477,256 against 207,857,104,851. Their leaves (500,001 and 3,985,001 bytes: 200,000 /
   1,594,000 OP_3DUP and 300,001 / 2,391,001 OP_2DROP on three 520-byte items) cost 52,165 engine per byte, both.
2. **Every input charged its WHOLE transaction's bytes.** The guest parses a transaction once per chunk, so in a
   one-transaction block every input was charged the transaction: mem_k1140 (one transaction, 24,374 inputs, 12.4 MB
   with its prevouts) priced each 1-byte OP_TRUE spend ~25 M, ~10x its measured cost, and its 1,140 heavy inputs
   (bare 9,818-byte scripts: 18 pushes of 520 bytes, 201 OP_CHECKSIG) only ~4x a light one where they cost ~255x.
   Measured split: chunks of 409 / 409 / 696 inputs at 283.6 / 280.7 / 222.0 G, then 13 chunks of 1,759 inputs at
   ~4.75 G: straggler 5.35.
3. **No legacy position.** A legacy input's sighash hashes 41 x (inputs from it on) + the outputs after the guest's
   shared prefix (the fitted model's LEGACY feature); the hand model priced 364,292's 16 chunks equal and they
   measured 12.5 G falling to 1.4 G: straggler 1.83.

## The change

`cost_features`: per input, its share of the transaction's bytes (tx + prevouts / inputs), the legacy position for
an input with a scriptSig and no witness, and the bytes past SCRIPT_ALLOWANCE (520) of a script it executes that is
not a template the signature terms already price (`executed_script`: a tapscript leaf; a P2WSH / P2SH-P2WSH witness
script or a P2SH redeem script that is not OP_m..OP_n CHECKMULTISIG; a bare output script that is not P2PKH, P2PK,
multisig or OP_RETURN; nothing for P2WPKH, P2SH-P2WPKH, a taproot key path or anyone-can-spend programs). Standard
scripts are shorter than the allowance, so their predictions do not move from it. Rates (in input_costs' units):
COST_PER_LEGACY_POS 37, COST_PER_SCRIPT_BYTE 11,600; both overridable (HAZYNC_COST_LEGACY_POS, HAZYNC_COST_SCRIPT_BYTE),
and HAZYNC_COST_MODEL=v1 gives the old model exactly (checked: v1's bounds and predictions are byte-identical to the
model before, report_v1.txt).

**Fit.** Least squares over all 47 measured chunks (each row weighted 1 / measured; fit.py) of engine cost on: the
model's other terms (base + signatures + 2 x share), legacy position, script bytes past the allowance, and one per
chunk. On the 29 ordinary chunks alone: 4.22 engine per unit, 155.5 per legacy-position byte (= 36.8 units; 30.9
jointly with the attacks), 4.79 M per chunk. On the attack chunks alone: 49,147 per script byte (52,165 directly from
the two tap blocks), = 11,600 units at the ordinary 4.22 (the units HAZYNC_CHUNK_TARGET is in).

Measured chunk costs: the engine's `hzengine execute` cost of every chunk as the v1 split exported it, on the build
box (edge blocks: edgef/<h>/chunk_NN.exec.log; attack blocks: results/guest_memory heap.out; measured.txt). Block
JSONs: prover/testdata (edge, boundaries) and the attack blocks of results/q7_attack_blocks. Predictions: the ignored
test `cost_report` in chunks.rs (it runs the chunker itself and reproduces every measured chunk count: 16, 4, 4, 2, 1).

## Before / after

| block | inputs | chunks (v1 / new) | predicted v1 | predicted new | measured engine | engine / v1 | engine / new |
|---|---|---|---|---|---|---|---|
| 185956 | 619 | 1 / 1 | 298,762,561 | 315,366,559 | 1,282,171,230 | 4.29 | 4.07 |
| 227931 | 742 | 1 / 1 | 342,183,628 | 346,484,684 | 1,524,425,736 | 4.45 | 4.40 |
| 364292 | 5,569 | 16 / 16 | 15,800,350,093 | 26,094,741,266 | 109,417,851,990 | 6.93 | 4.19 |
| 388381 | 1,685 | 2 / 2 | 1,039,852,115 | 1,056,568,777 | 4,724,552,979 | 4.54 | 4.47 |
| 419328 | 4,951 | 4 / 4 | 3,019,402,463 | 3,518,078,271 | 14,138,741,607 | 4.68 | 4.02 |
| 481824 | 5,192 | 4 / 4 | 2,862,672,302 | 3,100,790,011 | 12,538,159,784 | 4.38 | 4.04 |
| 91880 | 1 | 1 / 1 | 459,771 | 463,804 | 6,739,959 | 14.66 | 14.53 |
| mem_k1140 | 24,374 | 16 / 16 | 712,717,246,922 | 229,431,732,670 | 848,167,089,969 | 1.19 | 3.70 |
| tap_small | 1 | 1 / 1 | 1,507,256 | 5,795,486,856 | 26,084,647,308 | 17,306.05 | 4.50 |
| tap_full | 1 | 1 / 1 | 8,477,256 | 46,228,456,856 | 207,857,104,851 | 24,519.39 | 4.50 |

On ordinary chunks with the measured (v1) bounds, each model with its own best scale: v1 median error 4.1%, worst
13.4% (without 364,292) and 26% / 140% with it; the new model 4.1% / 10.2% and 6.5% / 25%.

**The new splits** (report_new.txt): the ordinary blocks keep their chunk counts; 364,292's early chunks shrink (186
inputs where v1 put 349; the last 1,192); mem_k1140 puts 75 heavy inputs in each of 15 chunks and the last 15 heavy
with all 23,234 light ones. Their engine cost is NOT measured yet (executing them needs the regtest RISC Zero host on
the build box, whose CPU another run was using): computed from the measured per-input costs (0.693 G heavy, 2.70 M
light), mem_k1140's new split has a straggler of ~1.38, against 5.35 measured on v1's. The estimator (estimate.py)
reproduces the measured v1 stragglers (364,292 1.85 vs 1.83; mem_k1140 5.47 vs 5.35; 419,328 1.13 vs 1.10), but on
the new splits it is circular (same features as the model), so it is not evidence there.

To measure the new splits (on the box): `HAZYNC_NETWORK=regtest HAZYNC_PROFILE_LAYOUT=cost <host> libzync-hints` then
`engine-export` for each attack block (as results/guest_memory/heap_per_chunk.sh), and `hzengine execute` per chunk.

## Left (5.1)

- Execute the new splits (above) and the ~50 more blocks across eras and transaction types; refit on them.
- A bare OP_TRUE spend is still priced one ECDSA verify (predicted_sig_ops' max(1)); mem_k1140's remaining imbalance
  comes from it (light inputs ~28% under, heavy ~17% over in engine terms).
- The fitted model (HAZYNC_COST_MODEL=fit) is unchanged: it already splits the transaction across its inputs and has
  the legacy position; it has no executed-script term, and its coefficients are RISC Zero's.
