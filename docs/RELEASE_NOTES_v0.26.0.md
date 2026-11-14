**A new image, the board proved again from genesis, and each block's undo data committed in its proof.** v0.26.0 is a
re-baseline: its mainnet image **`46666b6f`** replaces v0.24.0/v0.25.0's `8888d59c`, and the board resets and proves the
chain again from block 1 under it. The rules a block is checked against do not change, and RISC Zero stays at 3.1.0.

```
mainnet  46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c
signet   01a80f17469dfd75869534ffdbf3105e55438afc77329e818eca7044b4129be7
headers  2db34e12cc993dd1edb003f00f85c93d02e0e61208589b57fc5c969d39b3e632
```

Two reproducible builds of the same source (`reproduce/libzync.Dockerfile`, CI and a second machine) agree on these
ids (`reproduce/LIBZYNC_IDS`). Both images start their chains at genesis (an empty lineage). The header-chain zyncAPP
is unchanged.

## The undo-data commitment

Each block's proof already checked its spends against the coin log. It now also **commits** them: one hash per block
over every coin the block spends, in the block's leaf of the block forest, so a proof of the tip commits the undo hash
of every block ([`libzync/UNDO.md`](libzync/UNDO.md)).

- **Format (U0):** per input of every non-coinbase transaction, in block order, SHA-256 of `code u32 (creation height
  * 2 + coinbase) ‖ amount u64 ‖ script length u32 ‖ script`; the block's undo hash is SHA-256 of those digests. These are
  the fields and the order of Bitcoin Core's own undo data, so a node computes a block's undo hash from its `rev*.dat`.
- **Where:** a new block leaf, tag `0x05`, `SHA-256(0x05 ‖ block hash ‖ height ‖ coin state ‖ undo hash)`, for every
  block from 1. `ChainState` is unchanged. The genesis leaf, and so each network's genesis anchor, is unchanged.
- **For:** SwiftSync hints files (#722), which need each block's undo hash, and designs that split chain validation into
  parts linked by per-block undo commitments.
- **Cost, measured** (2026-11-14, [#729](https://github.com/hazync/hazync/issues/729); execute only, 4 chunks, 11 real
  mainnet blocks, a prototype of this format against the previous image): **+0.35% to +2.46%** of a block's executed
  cycles (+0.00% to +2.68% padded to segments), the most at block 962,000 (8,006 inputs). The guest's undo hashes equal
  an independent Python implementation (`scripts/libzync/undo_ref.py`) on all 11, and CI checks them on every push.
- **Verifiers** select a block's leaf format by its height, from the image that proved it: `8888d59c`'s and signet
  `d4210dae`'s blocks have no undo hash, and every block of a v0.26.0 chain from 1 on has one. Coin and transaction facts
  that open a block's leaf carry its undo hash (`block_undo`, `undo`), and the verifier prints which blocks commit one.

## Handover support (built and tested, not used by this release)

A new image can now continue an earlier image's chain instead of proving it again: it names the image it continues and
the block where it takes over, proves that block on top of the old image's genesis-anchored proof, and carries on from
there ([`HANDOVER.md`](HANDOVER.md)). Only upgrades that are not soundness fixes may be handed over (§1). v0.26.0 does
not use it: its images have empty lineages, and the board starts again from genesis so that every block carries the
undo-data commitment. It is there for later images.

**Across a RISC Zero upgrade.** A new RISC Zero version cannot check an old version's proofs: from 3.0.5 to 3.1.0 the
recursion circuit's constraints themselves changed. So the guest can carry a copy of the previous version's recursion
circuit and check the old spine with it, and the new image's lineage names that version. This release carries copies
for RISC Zero 3.0.5 and 3.1.0. Inside the zkVM the check works: real 3.0.5 proofs and the board's real 3.1.0 proof of
blocks 1 to 1466 are accepted in about 23 M cycles, and a changed seal, wrong image, wrong output or wrong version are
each refused. Its fast form cannot be proved, on the CPU prover or the GPU, because of a RISC Zero 3.1.0 bug in the
Poseidon2 instruction it uses: the executor reads the instruction's addresses as word addresses and the proof circuit
as byte addresses ([risc0#3427](https://github.com/risc0/risc0/issues/3427); our analysis and a fix are posted there).
With plain software hashing instead, the same step **was proved on a GPU** (2026-11-14, one RTX PRO 6000): block 1467
handed over on the board's real proof of blocks 1 to 1466, across the vendored 3.1.0 checker, in 280 s (257 M cycles),
verified, and a changed receipt refused.

**Measured:**

- **A real handover on a GPU** (one RTX PRO 6000, 2026-11-14): a test image continuing `8888d59c` at block 1467 proved
  block 1467 on top of the board's real proof of blocks 1 to 1466 in **27 s**. The new proof of 1 to 1467 matches the
  board's own state at 1467; nine wrong inputs were refused (a flipped byte, the wrong leaf height, the wrong spine
  height, the wrong earlier image, and the verifier without the handover or with the wrong one).
- **A board rehearsal** (local board, no real proofs, in CI): an image to block 10, the handover to a second image at
  11, on to 20; with state stamping switched off, it fails, as it should.

## What changes for you

- **Provers:** the board resets and takes only `46666b6f` proofs. Take v0.26.0 and restart any worker loop:
  `run-workers.sh` checks the image only when it starts. The Windows app checks for `46666b6f`.
- **Verifiers:** the verifier keeps `8888d59c` (and signet `d4210dae`), so a receipt by it is still checked as that
  image with no extra option. Facts about blocks of a v0.26.0 chain need the block's undo hash
  ([`libzync/UNDO.md`](libzync/UNDO.md) §5).
- **Board operators:** the board restarts from genesis under the new host (no `LZ_HANDOVER_AT`). `history.bin` records
  now carry each block's undo hash; older files still read.

## Also

- **Hardening from the 2026-11-14 soundness review** (no critical, high or medium findings; these are its low ones):
  the legacy sighash cache starts over at every entry point that runs scripts, and the guest checks at start that its
  allocator never reuses an address (the caches keyed by address rely on it); a transaction or coinbase larger than a
  block is refused before any C++ call sizes a buffer from it; the live-coin total a state carries is checked to be a
  curve point; libsecp's illegal-argument and internal-error callbacks stop the guest, as Core's defaults do; the batch
  signature transcript binds the number of checks and their schemes; a trie fact with a pruned all-zero hash is refused
  by the verifier. The v0.25.0 board's prep carries over without re-running (`libzync-reforest`,
  [`libzync/UNDO.md`](libzync/UNDO.md) §8).

- **The sponsor bot** (#728) skips a run while a fresh release is still being signed (no signed manifest yet), instead
  of latching off.
- **The board's progress alarm** (#719) takes a lower proven count as a new board, not a stall. After the v2 switch it
  had compared the new board's count with v1's and reported "nobody is proving" on every run.
