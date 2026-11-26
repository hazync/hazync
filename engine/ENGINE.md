# Hazync engine on OpenVM

Branch `engine/openvm`. The next proving engine for Hazync: the same consensus code (Bitcoin Core v28's
interpreter, libsecp256k1, Hazync's block rules, coin log, counting trie, live-coin hash, BIP30 tree, block forest),
proved on OpenVM 2.1 instead of RISC Zero 3.1. Measured on block 976,379 with the bench port
(`experimental/fresh-cost-model/REVIEW.md` on `experiment/fresh-cost-model`): 1,658 s -> 224 s on one A6000, every
output identical. This directory is that port rebuilt natively, without RISC Zero-shaped adapters.

## Shape

One guest image, one VM config, one SDK. Modes, as in the product guest:

| mode | what | verifies (in-guest, via deferral) | public values |
|---|---|---|---|
| chunk | Core's `verify_inputs_batch_once` on a slice of a block's transactions, batch signature check | nothing | sha256(ChunkOut) |
| trie | the block's counting-trie update (one part of 16, or whole) | nothing | sha256(TrieOut) |
| block | aggregate: coin log, block rules, live-coin hash, BIP30, block forest | every chunk + trie proof of the block, and the previous block proof (or none at an anchor) | sha256(ChainState) |
| join | two adjacent chain proofs -> one | both | sha256(ChainState) |

Recursion is OpenVM's deferral framework (`openvm-verify-stark-guest`, `DeferralAggProver::verify_stark`): a guest
call returns `(app_exe_commit, app_vm_commit, user_public_values)` of a proof that exists and verifies; the proof is
checked by the deferral circuit during aggregation, not executed in the VM. The self-recursive setup verifies proofs
from the same SDK, including ones that carry deferrals, so a block proof verifies chunk proofs and the previous block
proof, and a join verifies block proofs. This replaces every `env::verify`.

Image binding (the product's `self_id` rule, S1): the guest takes its own `app_exe_commit` as input, requires every
verified child to carry the same exe and vm commits, and commits it in its journal. The verifier checks the final
proof's exe commit equals the published one AND the journal's copy.

Public values: OpenVM's are 32 bytes; each mode commits sha256 of its journal, and the journal itself travels next to
the proof (the child journal is an input to the parent, checked against the 32 bytes).

## What "native" means here (the adapters the bench port had, removed)

1. SHA-256 in OpenVM's native state layout throughout (Core patches 0002/0009/0010 and libzync's W layout were
   RISC Zero's byte-swapped `sys_sha_compress`). Measured -22% on the trie.
2. libsecp's field arithmetic on OpenVM's modular extension (the counterpart of patch 0012), not software.
3. Plain input chunks (`openvm::io::read_vec`) instead of RISC Zero's serde word stream.
4. No compat crates (`risc0-zkvm`, `risc0-crypto` shims), no stubbed `xver`/handover code: a new chain from
   genesis has no RISC Zero predecessor.
5. The trie in its new format (node hashes take child state words directly): -22% again, measured 27.2 s vs 36.1 s.

Each lands only with its measurement, and with outputs checked against the product on the same block.

## Milestones

- **M1 recursion** — host crate on `openvm-sdk` 2.1 (CPU + CUDA), self-recursive deferral config with the
  extensions Hazync needs (rv32im, io, sha2, secp256k1 ecc + modular), a guest with `selftest` leaf/node modes;
  prove a node verifying two leaves on CPU; measure the deferral cost on a GPU.
- **M2 chunk** native: Core + libsecp, native SHA, plain inputs. Block 976,379: all 8,130 inputs accepted, ChunkOut
  equal to the product's.
- **M3 trie** native, new format, 16-part split + combine.
- **M4 block**: aggregate verifying chunks + trie + previous block. ChainState equal to the product's (modulo the
  fields that name the image).
- **M5 join** + chain from genesis on a short range; verifier CLI.
- **M6** host pipeline from Hazync's prep: inputs for every mode from a block, GPU end-to-end, tip latency on N cards.

## Status (24.11.2026)

- M1 recursion: DONE on CPU (results/M1.md): leaf, node over two leaves, node over a node (nested deferral); negative
  controls (tampered journal, wrong image) refused in-guest.
- The engine guest = the product guest's own source (engine/guest), built on the platform crate (engine/platform):
  BUILDS (31 MB exe). Block-level run: `hzengine prove-block` over the product host's `engine-export`.
- Product-host backend (prover/host/src/engine_backend.rs): `HAZYNC_ENGINE=<hzengine> HAZYNC_ENGINE_KEYS=<dir>`
  makes `libzync-prove-range` and `libzync-join` run every execution and proof on the engine (`hzengine serve`).

## Defaults (from results/gpu_a6000_tuning, 24.11.2026)

| setting | default | override | measured on block 976,379, one A6000 |
|---|---|---|---|
| chunks | sized by predicted cost, ~0.9B each, at most 16 (engine runs and `engine-export` only) | `HAZYNC_CHUNK_TARGET`, `HAZYNC_CHUNKS` | 5 chunks 240 s vs 16 chunks 289 s (one prover) |
| provers per GPU | 3 on a card of 40 GiB or more, else 1 | `prove-block --workers`, `HAZYNC_ENGINE_PER_GPU` | 1: 240 s, 2: 194 s, 3: 183 s |
| segment cap | round(GPU GiB / provers) − 4, at most 24 (A6000: 24 / 20 / 12); smaller cards and CPU: OpenVM's 15 | `HZ_SEG_MEM_GB` | 1 prover: 15 GiB 240 s, 24 GiB 221 s; 32 GiB refused by the prover |

`hzengine gpu-share` prints the provers-per-GPU default for the card it sees. Cards under 40 GiB keep OpenVM's
settings, because nothing has been measured on them yet. A `prove-block` aggregate runs alone on the card, so it gets
the 1-prover cap.

## Guest memory: a design limit (26.11.2026, results/guest_memory)

OpenVM's guest address space is **512 MiB at most** (pointer_max_bits <= F::bits() - 2 = 29): a hard limit of the VM,
not a setting; more GPU memory or more cards do not raise it. **One chunk must fit in it.** RISC Zero's guest had 3 GiB.

What makes that safe: #794's C-heap recycling (prover/methods/guest/src/c_heap.rs: per-size-class free lists for the
C/C++ consensus code; Rust stays on the bump allocator). A chunk's heap then follows its live data, not everything the
script interpreter ever allocated. Measured high-water marks per chunk (HZHEAP, metering images; an upper bound):
**62.9 MiB worst** (mem_k1140, the rig's memory attack), 36.5 MiB tap_full, 4.6 MiB tap_small (> 512 MiB without
recycling), 25.7 MiB block 962,000 in one chunk. A chunk's heap grows with its input size, so if a block ever came
near the ceiling the lever is more chunks; the fallback, if a SINGLE input ever needed more than 512 MiB, is splitting
that input's script check across proofs (not built: nothing measured needs it).

Known gap (roadmap 5.1): the chunker's cost model misses heavy scripts by about four orders of magnitude, so on
adversarial blocks the chunks are badly unbalanced in time (not in memory or correctness).

## Layout

| path | what |
|---|---|
| engine/host | `hzengine`: keygen, build, prove / execute (RISC Zero input streams, child proofs), prove-block, serve, verify |
| engine/platform | the RISC Zero guest API on OpenVM (risc0-zkvm, risc0-crypto, sha2 slices) |
| engine/guest | assemble.sh (product guest -> engine guest), base.sh (C/C++ tree), edits.py, batch_ecdsa_ovm.rs, libsecp patch |
| engine/selftest-guest | M1's recursion self-test image |
| engine/scripts | m1.sh, mkbundle.sh, gpu_block.sh + pod/ (one-GPU block run) |
| prover/host/src/{sink,engine_backend}.rs | the product host's side: input streams, the engine as its prover |

## Not changed

Consensus code and its patches' meaning; ChainState's content (apart from image ids and the trie format);
zAPP-facing facts. Anything that changes a commitment's meaning is listed in CHANGES.md with the reason.
