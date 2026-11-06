# libzync proof statement and verifier procedure

What a libzync chain proof commits (§3) and every check a verifier makes on it (§5). Normative sources:
`swiftsync-verify/src/lib.rs` (`ChainState`, `verify`, `genesis_anchor`) and `host libzync-verify`.

The file name is historical: it began as a proposal for proven SwiftSync hints files (design B), retired in E2
(2026-11-05). Section and step numbers are kept because code cites them (the frozen guest cites step 3b). Design B's
proposal, measurements and questions are in the full original, `experimental/libzync-bench/retired-docs/SWIFTSYNC_PROOFS.md`.

## 2. What is proven

A receipt of a libzync image says: every block from genesis to `tip_hash` is valid under Bitcoin Core's consensus
rules as compiled into the image (`CONSENSUS_RULES.md`), and `ChainState` (§3) is the state after the last one: the
coin log (`COINLOG.md`), the counting trie's root, the live-coin hash, the block forest (`HANDOVER.md` §3), work and
time. It holds only for a receipt anchored at the network's genesis state (§5 step 3), and it rests on the soundness
of risc0's recursion. Which chain it is, is a separate check (step 7).

## 3. Objects

### The proof's public output: `ChainState`

In field order. Types are exact: risc0's serde writes a u32 as one word, a u64 as two (low first), every byte of
`[u8; 32]` as its own word (32 words), `[u32; 8]` as 8 words, a `Vec` as its length then its elements, an `Option` as
0/1 then the value. A wrong width changes `anchor_id`.

| field | type | meaning |
|---|---|---|
| `kind` | u32 | `0xC4A10012` (domain tag) |
| `version` | u32 | `2`, the layout. Fields are only ever appended, never reordered or removed, so a reader of version n decodes the prefix it knows from any later version (HANDOVER.md §2) |
| `tip_hash` | [u8; 32] | the chain proven: its last block's hash, internal byte order |
| `utxo_roots` | Vec<Option<[u8; 32]>> | the coin log's forest roots, index = tree level, trailing empties removed (except the genesis state's `[None]`, §5 step 3): every coin ever appended (COINLOG.md; leaf and `parent`: UTREEXO_FORMAT.md §2, §5) |
| `utxo_leaves` | u64 | leaves in that forest |
| `utxo_value` | u64 | satoshis of the live coins |
| `cum_work` | [u8; 32] | cumulative work, little-endian |
| `height` | u32 | height of the tip |
| `prev_nbits`, `prev_time`, `epoch_start` | u32 each | difficulty and time context, so the chain can be extended |
| `recent_times` | Vec<u32> | timestamps of the last ≤ 11 blocks |
| `anchor_id` | [u8; 32] | double-SHA-256 of the state the chain started from (genesis for a full proof; pinned per network) |
| `self_id` | [u32; 8] | the image id every step since the last handover ran (risc0's digest words) |
| `live_set` | u32 | profile bit 6: the multiset hash of the LIVE coins is the identity iff 0 (instant IBD: a downloaded UTXO set is checked against it). 0 without bit 6 |
| `live_hash` | [u32; 16] | profile bit 6: that hash's point. Zero without bit 6 |
| `trie_root` | [u8; 32] | profile bit 5: the counting trie's root (absence, history and balance facts, `ZYNCAPPS.md`). Zero without bit 5 |
| `smt_root` | [u8; 32] | root of the BIP30 tree (coinbase txids and tracked transactions) after the tip |
| `max_time` | u32 | the largest block timestamp in the chain (for `time-too-new`, §5 step 11) |
| `profile` | u32 | what the image checked, set from its compiled code, never carried from input: bit 0 libzync, bit 1 scripts and signatures **skipped** (assumevalid), bit 2 signet, bit 3 regtest, bit 4 coin log, bit 5 counting trie, bit 6 live-coin hash. The product images are `0x71` (mainnet), `0x75` (signet) |
| `block_roots` | Vec<Option<[u8; 32]>> | the block forest: one leaf per block, genesis first, each leaf committing the block hash, height and the coin state after it (HANDOVER.md §3) |
| `block_leaves` | u64 | `height + 1` |
| `lineage` | [u8; 32] | 32 zero bytes for a chain proved by one image since genesis; otherwise the digest of the images it was handed over from (HANDOVER.md §5) |

The serde aliases `ss_set`, `ss_acc`, `ss_hints` (design B's names for `live_set`, `live_hash`, `trie_root`) still
read a JSON written before E2.

## 5. Verifier procedure: all of it

Given a receipt R and the network. Both verifiers (`swiftsync-verify`, `host libzync-verify`) make every check
below; step 7 needs a source you name (without one they print the tip hash to compare).

1. `R.verify(M)`, where M is the published libzync image id for that network. ⚠ Until the freeze publishes the
   product ids (ROADMAP H7), the built-in table holds round-8 images whose journals do not decode as today's
   `ChainState`: pass the id with `--image <hex>`.
2. Decode the journal as `ChainState` J and check `J.kind == 0xC4A10012`, `J.version >= 2` and `J.self_id == M`. M is
   the **newest** image: after a handover the receipt verifies against the image that continued the chain.
3. `J.anchor_id == dsha256(genesis ChainState journal for the network)`.
   **This is essential:** the base step of the recursion is not verified, so a proof anchored anywhere else proves
   nothing about history.
   - The genesis ChainState, in field order: `kind = 0xC4A10012`; `version = 2`; `tip_hash` = the genesis hash
     (internal byte order); `utxo_roots` = a list of **one** empty entry (`[None]`, not an empty list);
     `utxo_leaves = 0` (u64); `utxo_value = 0` (u64); `cum_work` = `GetBlockProof(genesis nBits)` as 32 bytes
     little-endian; `height = 0`; `prev_nbits` = genesis nBits; `prev_time = epoch_start` = genesis time;
     `recent_times = [genesis time]`; `anchor_id` = 32 zero bytes; `self_id` = eight zero words (the genesis journal,
     and its digest, are the same for every image, so a chain keeps its anchor across a handover); `live_set = 0`;
     `live_hash` = 16 zero words; `trie_root` = 32 zero bytes; `smt_root` = the empty coinbase-SMT root, `e_256` where
     `e_0` = 32 zero bytes and `e_{d+1} = SHA-256(0x11 ‖ e_d ‖ e_d)`
     (`507d86c51845c467583449a973898eff681f18e74aa6d62b206da023cbcaec32`); `max_time` = genesis time; `profile = 0`;
     `block_roots` = one entry, `Some(SHA-256(0x02 ‖ genesis hash, internal byte order ‖ 0u32 LE ‖
     SHA-256(0x04 ‖ 0u64 LE ‖ 0u64 LE)))` (HANDOVER.md §3: the empty coin state); `block_leaves = 1` (u64);
     `lineage` = 32 zero bytes.
   - The journal is risc0's serde word encoding (risc0-zkvm 3.0.5 `serde::to_vec`, each word little-endian), as in §3.
   - `host libzync-verify` computes this itself; `scripts/libzync/genesis_anchor_id.py` computes it from this text
     alone. The values are PINNED (a chain keeps its anchor through every handover, HANDOVER.md §4): mainnet
     `852c5c7e009317c2dd232dcf9b1a9e2babd58df324add461a6ff61824f390167`, signet
     `3b67ead0618b7786b001525b68c7b088405d6a577292838361cc07e5bb938827` (`swiftsync-verify` unit tests pin both).
3a. **Minimum work.** `J.cum_work` must be at least Core v28's `nMinimumChainWork` for the network (mainnet
   `0x…88e186b70e0862c193ec44d6`, signet `0x…25dbd66e58f`). Below it the proof is a valid chain from genesis but no
   evidence it is the network's chain (a low-work fork costs little to mine): the verdict is DEMO (exit 2), not OK.
3b. **Checkpoint.** A libzync image pins one block (`prover/methods/guest/src/checkpoint.rs`): mainnet 970,000 =
   `000000000000000000014e6a…f29b5a07`, signet 325,000 = `000000057a36…e306fcd4`; none on regtest. The guest refuses any
   other block at that height, so a proof at or above it shows that its chain contains the checkpoint: a fork must
   branch after it and carry the work of that stretch of the real chain. Below the checkpoint height the verdict is at
   best DEMO, as for low work (step 3a). The checkpoint is fixed by the image id; the tip itself still needs step 7.
4.–6. **Coin commitments match the profile** (these steps checked design B's hints file; retired). The image keeps
   its coins in the coin log (profile bit 4; a design-B image is refused). Without bit 6, `live_set == 0` and
   `live_hash` is zero; without bit 5, `trie_root` is zero. Nothing is closed at a height: the coin log is sound at
   every block.
7. **Chain identity.** `J.tip_hash` is the block at height `J.height` on the network's chain. The proof says the chain
   it covers is valid from genesis; which chain it is, is its tip hash (each header commits to the previous one, so the
   tip hash identifies the whole history), and a valid fork has its own. Compare it with a source you trust: your own
   most-work header chain (`--bitcoin-cli`), a block explorer (`--mempool`: one GET of mempool.space's
   `/api/block-height/<J.height>`, signet `/signet/api`; `--mempool-url` for another esplora API), or a hash you
   already have (`--expect <height>:<hash>`). A mismatch is FAIL; with no comparison the OK line says the chain
   identity was NOT checked and prints the hash to compare.
8. The receipt is a real proof: refuse a dev-mode `FakeReceipt`, and never verify with `RISC0_DEV_MODE` set.
9. `J.profile` equals the profile of the image you accept (bit 1 set means scripts and signatures were **not**
   checked). M still has to be the image you trust: rebuild it from the published source commit with
   `reproduce/libzync.Dockerfile` and compare with the published id. The id depends on absolute build paths, so only
   a build in that container gives the published id.
10. (Retired: the hints file's canonical encoding, design B.)
11. `J.max_time <= now + 7200`, with `now` from your own clock (Core's `MAX_FUTURE_BLOCK_TIME`).
12. `J.block_leaves == J.height + 1`, and `J.block_roots` has a root exactly at each set bit of `J.block_leaves`.
13. `J.lineage` is 32 zero bytes (one image since genesis), or the digest (HANDOVER.md §5) of the handover list
    compiled into M. Trusting M then means trusting every image in that list: a handover takes the old image's claims
    on trust (HANDOVER.md §4).

**Verdicts** (`swiftsync-verify` prints `SWIFTSYNC_VERIFY_*`, `host libzync-verify` prints `LIBZYNC_VERIFY_*`):
- `OK` (exit 0): every check passed on a genesis-anchored proof with enough work, at or past the checkpoint;
- `DEMO` (exit 2): every check passed, but the proof is checked against a supplied anchor (`--anchor`), or is below
  the minimum work or the checkpoint height;
- `FAIL` (exit 1): any failed check, including an unreadable or malformed receipt or anchor.

An OK verdict for an assumevalid image says so on the verdict line itself. A coin, trie or transaction fact
(`--coin`, `--trie`, `--journey`) is checked against the same `ChainState`, so its answer is about the chain as of
`J.height`: a coin unspent then may have been spent since. A proof does not expire: every answer names the height
it is about (a trie fact must be about the proof's tip), and how stale an answer may be is the application's call.
