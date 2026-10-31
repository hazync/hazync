# libzync proof bundle (`.sspf`), version 1

One file that travels next to a SwiftSync hints file and carries the proof that the file is correct
(`SWIFTSYNC_PROOFS.md`). Writer and reader: `swiftsync-verify` (`bundle` to write; any verify command accepts a
bundle where it accepts a receipt).

## Layout

All integers little-endian. Hashes in internal byte order, as the proof commits them.

| offset | size | field |
|---|---|---|
| 0 | 4 | magic `SSPF` |
| 4 | 1 | version `0x01` |
| 5 | 1 | network: `0` mainnet, `1` signet |
| 6 | 32 | image id M the receipt was made by |
| 38 | 4 | stop height H |
| 42 | 32 | tip hash at H (internal order) |
| 74 | 32 | cumulative work at H (32 bytes LE) |
| 106 | 32 | `ss_hints` (proposal §3) |
| 138 | 32 | SHA-256 of the hints file's bytes |
| 170 | 4 | receipt length L |
| 174 | L | the receipt: a risc0 3.0.5 `Receipt`, bincode 1 encoded: a succinct STARK (about 227 KB) or its Groth16 wrap (`host libzync-snark-wrap`) |

The file ends at `174 + L`; anything else is an error.

## The header is never trusted

The header exists so people and indexes can see what a bundle is about without verifying it: which network and
image, which height and tip, which hints file. A verifier:
1. takes the network and image id M from **its own** configuration (or a rebuilt M the user supplies), never from
   the header;
2. verifies the receipt exactly as for a bare receipt (proposal §5);
3. then requires every header field to equal the verified result: network and image are the verifier's, the
   SHA-256 of the hints file it was given equals the header's, and height, tip, work and `ss_hints` equal the
   verified journal's.

A bundle whose header disagrees with its receipt is a failure even when the receipt is valid: a header that lies
is a bundle that lies.

## Not in the bundle

- **The hints file.** It can be 122 MB and is published separately; the bundle names it by SHA-256.
- **The UTXO set.** The proof commits it as Utreexo roots (`UTREEXO_FORMAT.md`); the roots are in the receipt's
  journal.

## Versioning

A reader rejects any version it does not know. A later version that adds fields keeps the same rule: every field
is checked against the verified result or the given files.
