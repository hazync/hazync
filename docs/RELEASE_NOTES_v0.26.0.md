<!-- DRAFT for v0.26.0: not yet released. Facts only; every number is measured (sources: #717's body and its GPU comment). -->

**The board's image can now change without re-proving.** Until now a new Hazync image meant proving the chain again
from genesis, because every proof names the exact program that made it. v0.26.0 adds the **handover**: a new image
names the image it continues and the block where it takes over, proves that block on top of the old image's
genesis-anchored proof, and carries on from there. Every earlier proof stays valid.

This release is the first handover. Its mainnet image **`1ff5a129`** continues v0.24.0/v0.25.0's `8888d59c` at
**block 2000**: the board's blocks 1 to 1999 stay `8888d59c`'s proofs, and `1ff5a129` proves block 2000 on top of
`8888d59c`'s proof of 1 to 1999. The rules a block is checked against do not change, and RISC Zero stays at 3.1.0.

```
mainnet  1ff5a129d1ebc2022262f1249c1b41a0c096b8d5d16f985d7fc018138c67c66a
signet   28e396006e97a8075daeb9fb12ce3ef1b3a20e44eac935256a1c2e8c3d8d57f3
headers  2db34e12cc993dd1edb003f00f85c93d02e0e61208589b57fc5c969d39b3e632
```

Two reproducible builds of the same source (`reproduce/libzync.Dockerfile`, CI and a second machine) agree on these
ids (`reproduce/LIBZYNC_IDS`). Signet's image is a new chain from genesis (there is no signet board to continue). The
header-chain zyncAPP is unchanged.

## The rule: only upgrades that are not soundness fixes

A handover keeps everything below the handover block exactly as strong as the old image. So it is only for upgrades
that make proving faster or add features. If an upgrade exists to fix a soundness bug, in Hazync's guest or in RISC
Zero's proof system, the chain is proved again from genesis instead ([`HANDOVER.md`](HANDOVER.md) §1).

## Across a RISC Zero upgrade

A new RISC Zero version cannot check an old version's proofs: from 3.0.5 to 3.1.0 the recursion circuit's constraints
themselves changed. So the guest can carry a copy of the previous version's recursion circuit and check the old spine
with it, and the new image's lineage names that version. This release carries copies for RISC Zero 3.0.5 and 3.1.0.

**Not yet usable.** Inside the zkVM the check works: real 3.0.5 proofs and the board's real 3.1.0 proof of blocks 1
to 1466 are accepted in about 23 M cycles, and a changed seal, wrong image, wrong output or wrong version are each
refused. But proving that step on a GPU fails inside RISC Zero 3.1.0's CUDA witness generation (`Inconsistent set`),
though the same step executes correctly. It has to be fixed before a RISC Zero upgrade relies on it. This release's
own handover is between two images on the same RISC Zero and does not use it.

## Measured

- **A real handover on a GPU** (one RTX PRO 6000, 2026-11-14): a test image continuing `8888d59c` at block 1467 proved
  block 1467 on top of the board's real proof of blocks 1 to 1466 in **27 s**. The new proof of 1 to 1467 matches the
  board's own state at 1467; nine wrong inputs were refused (a flipped byte, the wrong leaf height, the wrong spine
  height, the wrong earlier image, and the verifier without the handover or with the wrong one).
- **A board rehearsal** (local board, no real proofs): the old image to block 10, the handover at 11, on to 20: 19 of
  19 checks; with state stamping switched off, 8 checks fail, as they should.

## What changes for you

- **Provers:** from the handover the board takes only `1ff5a129` proofs above block 2000. Take v0.26.0 and restart any
  worker loop: `run-workers.sh` checks the image only when it starts. The Windows app checks for `1ff5a129`.
- **Verifiers:** the verifier keeps `8888d59c`. A receipt by it is checked as that image with no extra option, so the
  board's proofs below block 2000 keep verifying; `--handover <id>:<height>` states a lineage for an image you
  rebuilt yourself.
- **Board operators:** `LZ_HANDOVER_AT` (docs/COORDINATOR_REFERENCE.md); the switch is [`HANDOVER.md`](HANDOVER.md) §5.
