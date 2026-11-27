# libzync proof bundle (`.sspf`), version 2

One file carrying a libzync chain proof behind a short header naming what it proves. Writer and reader:
`swiftsync-verify` (`bundle` to write; any verify command, the C ABI and WASM accept a bundle where they accept a
proof file). Version 1 also carried the SHA-256 of a SwiftSync hints file; E2 retired the hints file (no proof takes
one), and a version-2 reader refuses version 1.

## Layout

All integers little-endian. Hashes in internal byte order, as the proof commits them.

| offset | size | field |
|---|---|---|
| 0 | 4 | magic `SSPF` |
| 4 | 1 | version `0x02` |
| 5 | 1 | network: `0` mainnet, `1` signet |
| 6 | 32 | image id M the proof was made by |
| 38 | 4 | height H of the proven tip |
| 42 | 32 | tip hash at H (internal order) |
| 74 | 32 | cumulative work at H (32 bytes LE) |
| 106 | 32 | the counting trie's root at H (`trie_root`; zero for an image without the trie) |
| 138 | 4 | proof length L |
| 142 | L | the proof: an engine proof file as `swiftsync-verify` takes it bare (`HZE1`, engine/VERIFIER.md; 275,183 bytes for the engine chain's proof of mainnet 1..200). Until 27.11.2026 a bincode RISC Zero 3.1.0 receipt; v0.27.1 reads those bundles |

The file ends at `142 + L`; anything else is an error.

## The header is never trusted

The header exists so people and indexes can see what a bundle is about without verifying it: which network and
image, which height and tip, which trie root. A verifier:
1. takes the network and image id M from **its own** configuration (or a rebuilt M the user supplies), never from
   the header;
2. verifies the proof exactly as for a bare proof file, under the image's pinned key;
3. then requires every header field to equal the verified result: network and image are the verifier's, and
   height, tip, work and trie root equal the verified journal's.

A bundle whose header disagrees with its proof is a failure even when the proof is valid: a header that lies
is a bundle that lies.

A bundle is written only for an OK verdict, or DEMO for low work on a chain from genesis (e.g. a signet test range);
never for a proof from a supplied anchor, whose reader would have to trust the same anchor.

## Not in the bundle

- **The UTXO set.** The proof commits its history as the coin log's roots (`COINLOG.md`) and, with the product image,
  the live coins' multiset hash; both are in the proof's journal.
- **Facts.** Coin and trie facts travel separately (`ZYNCAPPS.md`, `swiftsync-verify --coin / --trie`) and are checked
  against the verified state.

## Versioning

A reader rejects any version it does not know. A later version that adds fields keeps the same rule: every field
is checked against the verified result.
