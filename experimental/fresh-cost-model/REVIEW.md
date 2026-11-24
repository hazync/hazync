# What an OpenVM build of Hazync would do for proving time

24.11.2026. Branch `experiment/fresh-cost-model` (trie work: `experiment/fresh-trie`). All runs on mainnet block
976,379 (4,394 txs, 8,130 inputs, 7,959 signatures), one RTX A6000 ($0.69/hr), OpenVM 2.1.0, total spend ~$3.60.

## Bottom line

Hazync's **own code**, unchanged at its core, ported to OpenVM, proves the same block in **~7x less GPU time**,
with every output checked identical to Hazync's. Measured, not estimated:

| part of a block | Hazync today (RISC Zero 3.1, v0.26.0 product) | Hazync's code on OpenVM | ratio | identical output? |
|---|---|---|---|---|
| scripts + signatures (Core v28 interpreter, sighash, libsecp, batch check) | 1,172 s (16 chunks) | **139 s** | 8.4x | all 8,130 inputs accepted |
| counting-trie update | 212 s | **36 s** (27 s new format) | 5.9x (7.8x) | journal byte-identical |
| aggregate (coin log, block rules, live-coin hash, BIP30, block forest) | 274 s | **49 s** | 5.6x | ChainState byte-identical |
| **block** | **1,658 s ≈ $0.32** | **224 s ≈ $0.04** | **7.4x** | |

OpenVM figures are each part wrapped to ONE STARK proof (630 KB) and verified. Host-side hint generation (~4 min CPU)
is the same for both and not included.

## Why it is faster

- **Signatures:** OpenVM's secp256k1 extension. With Hazync's batch hook and a sqrt hook in libsecp, Core's own libsecp
  costs less than the Rust k256 library (7.6B vs 8.8B cost units for the block's ECDSA). BIP340 goes through the same
  batch, so OpenVM's missing Schnorr support does not matter.
- **Plain RISC-V work** (interpreter, coin log, tree walks) proves ~2.5x cheaper per cost unit than accelerator-heavy
  code, and that is most of Hazync's per-block work.
- **SHA-256:** a native extension; the trie's update-list hash moved onto it cut the trie by a third.

## What "done properly" adds (measured levers, not yet combined)

The port carries RISC Zero-shaped adapters that a native OpenVM build drops:
- **SHA state in RISC Zero's byte-swapped layout** (Core patches 0002/0009/0010, libzync): native layout measured
  -22% on the trie; the same shim sits under Core's sighash and txids.
- **libsecp's own field arithmetic runs in software:** an OpenVM field backend (the counterpart of patch 0012) would
  accelerate key/signature parsing. Not built.
- **RISC Zero's serde word format** for inputs and journals: plain bytes instead. Not measured.
- **Parallelism:** the trie splits 16 ways linearly (4.1 s per part, measured); chunks already split. GPU utilisation
  was 64-69%. Tip latency is bounded by the aggregate (~49 s) plus the largest chunk slice.

So 7.4x is a floor for this block, not a ceiling. I would not quote a final number before those are built.

## What it costs to get there

- **A new chain from genesis.** A new image cannot continue RISC Zero proofs; the board keeps running RISC Zero
  until the OpenVM chain catches up. At ~7x, history is roughly a seventh of the GPU time (Hazync's own whole-chain
  estimate ~$280-295k on PRO 6000 -> ~$40k, if the ratio holds across older blocks; only one block measured).
- **Recursion rebuilt on OpenVM.** The port skips `env::verify` (17 calls per block: chunks + trie); OpenVM's
  aggregation does that job. Its wrap step cost +7-11% here; joining block proofs into the chain is NOT measured.
- **Verifiers and zAPPs** (CLI, browser/wasm, header chain, coin check) rebuilt around OpenVM proofs.
- **Trust:** one audited, maintained zkVM, and LESS custom crypto (drops the bigint2 field backend, patches
  0005/0006/0008/0012/0013 and the copied circuit blobs). New: two small libsecp hooks (batch, sqrt), the OpenVM
  batch module, and the port's adapters (removed in a native build). Each needs review like Hazync's own.

## Recommendation

Worth doing. Next, in order: (1) native-layout SHA + plain-byte inputs in the port, re-measure the block; (2) OpenVM
recursion for chunk/trie verification and block joins, measure a 2-block join; (3) a field backend for libsecp;
(4) then decide on a from-genesis OpenVM chain alongside the RISC Zero board.

## Found on the way (for Hazync today)

- v0.26.1 (and v0.27.0) Linux CUDA host aborts on real GPUs ("context is destroyed"); v0.26.0 / v0.27.1 are fine.
- `HAZYNC_LEGACY_SIGHASH` without `HAZYNC_AGG_LITE` reuses a stale legacy preimage (address-keyed cache). The product
  sets both; nothing enforces it — a one-line `#error` would.
- A guest that skips `__libc_init_array()` rejects every taproot spend (Core's static tagged-hash midstates).

Evidence: `bench/results/` (gpu_a6000_976379, run2, run3, run4, cpu_libsecp_ovm, cpu_core_ovm, cpu_trie_ovm;
trie_faster on experiment/fresh-trie).
