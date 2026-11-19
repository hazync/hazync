# zyncAPP checkers: assumevalid and assumeutxo from the chain proof (#721)

Status: built on `feat/zapp-assume` (15.11.2026), not merged, not reviewed. No new guest and no change to any image:
both are checks a verifier makes on a core chain proof it has already verified. Normative source:
`zapps/assume/src/lib.rs`, the zAPPs' own module (`zapps/assume`, ZYNCAPPS.md §9), on the core verifier as a library. `scripts/libzync/assume_ref.py` is an independent implementation of §3 written
from this document and Core's snapshot format.

Bitcoin Core ships two shortcuts that rest on a value compiled into each release: **assumevalid** (skip script
checks for the ancestors of a hardcoded block) and **assumeutxo** (load a UTXO snapshot whose hash is hardcoded).
A libzync chain proof to height H already proves what both of them trust. These checkers let a node check the
compiled value, or a value it is offered, against a proof. They add a check and remove nothing: the node's existing
behaviour (its own assumevalid, its own `m_assumeutxo_data` hash, the background validation of a snapshot) stays the
fallback.

## 1. Commands

```sh
zapp-assume assumevalid [--block <hash>] [--headers <file>] [--expect <h>:<hash> | --bitcoin-cli "<cmd>" | --mempool] <receipt>
zapp-assume assumeutxo --headers <file> [--tracked <file>] [--expect … | --bitcoin-cli … | --mempool] <receipt> <snapshot>
```

Both run the full receipt verification first (SWIFTSYNC_PROOFS.md §5) and check nothing against a receipt that
fails it. They print the usual verdict line; the exit codes are the verifier's (0 OK, 2 DEMO, 1 FAIL). As for every
fact, OK needs the chain identity checked (`--expect`, `--bitcoin-cli` or `--mempool`) and the proof at or past
Core's minimum chain work and the pinned checkpoint; a proof below that is at best DEMO. On success they print:

```
assumevalid: -assumevalid=<hash> (height <h>; scripts proven valid for blocks 0..<H>)
assumeutxo: <snapshot> is the proven UTXO set at height <H>; Core's hash_serialized_3 of it: <hex>
live_hash x <hex> y <hex>
```

The same checks are in the zAPP's C ABI (`swiftsync_verify_assume`, `zapps/assume/ffi/include/zapp_assume.h`) and
its WASM module (`verify_assume`; `verifyAssume` in `zapps/assume/wasm/zapp-assume.js`), all three through one library
function, `zapp_assume::run_assume`. `scripts/libzync/assume_inputs.py` writes the two side inputs (`--headers`, `--tracked`) from a
node; `scripts/libzync/assume_check.sh` runs the whole thing against a real `dumptxoutset` (§6).

**WASM cannot take a mainnet snapshot.** The page hands the module the whole file in one frame: the bytes are held
twice (the page's copy and the module's) and each chunk's length is a u32, so a file over 4 GiB cannot be framed, and
a 32-bit WebAssembly memory is 4 GiB in all. Mainnet's snapshot is several GB. The WASM entry is for low heights and
for demonstration; a mainnet check runs the CLI or the C ABI, which stream the file. A streaming WASM entry (the page
feeding the file in pieces to a reader that keeps only the running sums) is possible, but I do not think it is worth
it now: the hashing would still be single-threaded in the browser (no workers in this module), so a mainnet set would
take hours on a page, and there is no proof at a mainnet snapshot height yet (§7).

## 2. assumevalid

**What the proof says.** A receipt of a libzync image with profile bit 1 clear (scripts checked; every published
image) says every block from genesis to `tip_hash` passed Core v28's script and signature rules, as compiled into
the image: Core's interpreter, sighash and libsecp256k1 (CONSENSUS_RULES.md). Core's `-assumevalid=X` skips script
checks for X and its ancestors only. So any X in the proven chain at a height h ≤ H is covered by the proof.

**The check.** `assume_valid(cs, headers, block)`:

1. `cs` is the state of a receipt that verified; `cs.profile` has bit 0 (libzync) and NOT bit 1. An image that
   skipped scripts (bit 1, the assumevalid build) is refused: it proves nothing here.
2. No `--block`, or `--block` = the tip: X = `tip_hash`, h = H. This is the value a node needs: `-assumevalid=tip`
   covers every block the proof covers.
3. Any other X needs `--headers`: raw 80-byte headers, oldest first, ending at the proven tip. They are bound to the
   proof by hashing alone: each header's `prev` field must be the double SHA-256 of the one before it, and the last
   one must hash to `tip_hash` (`HeaderChain::bind`). Their heights follow from the tip's. X must be one of them.
   The run may start anywhere at or above genesis (a node needs only the headers from X to the tip).

**Which membership proof, and why.** The header chain, not the block forest or the header MMR zyncAPP
(`feat/headers-mmr`). `tip_hash` is in the journal, and SHA-256d links bind every ancestor to it, so the headers a
node already holds (it syncs headers before blocks) are a complete membership proof with no prover data. A block
forest path is shorter (log H hashes) but needs each block's coin state and undo hash from the prover's records
(HANDOVER.md §3); the header MMR proves header validity (work, retargets), which the core proof already covers for
every block to H. Neither adds anything for this question.

**What it does not say.** It says nothing about blocks above H, and nothing about whether the proven chain is the
network's: that is the identity check, as for every fact. Core keeps its own conditions for using assumevalid (the
block in its best header chain, minimum chain work, two weeks of work below the best header); they are not changed.

## 3. assumeutxo

### 3.1 What the proof commits

Every libzync chain proof with profile bit 6 (every published image: `0x71` mainnet, `0x75` signet) commits, at its
tip and only there, `live_set` / `live_hash`: an ECMH multiset hash of the **live** coins (`prover/methods/guest/src/swiftsync.rs`).

**The element.** Each live coin contributes the point of its 32-byte coin leaf (UTREEXO_FORMAT.md §2):

```
leaf = SHA-256( 0x00 ‖ txid (32, internal order) ‖ vout u32 LE ‖ value u64 LE ‖ len(scriptPubKey) u32 LE
              ‖ scriptPubKey ‖ height u32 LE ‖ flags u8 ‖ creation_mtp u32 LE )
flags = bit 0: created by a coinbase; bit 1: tracked by the BIP30 tree (UTREEXO_FORMAT.md §2a)
creation_mtp = the median of the timestamps of blocks max(0, h-11) .. h-1, h the creating block's height
```

**The map to the curve.** For ctr = 0, 1, …: x = SHA-256(leaf ‖ ctr u32 LE), read big-endian; the first x with
x³ + 7 a square mod p is the point's x, and y is its EVEN square root (x ≥ p is refused, as in the guest; p is
secp256k1's field prime). The hash is the sum of the points of all live coins in secp256k1's group, committed as
`live_set = 0` for the identity, else `live_set = 1` and `live_hash` = x's eight 32-bit words, least significant
first, then y's.

Which coins: the coin log's (COINLOG.md §3): spendable outputs (no `OP_RETURN` first byte, script ≤ 10,000 bytes),
not the outputs of the two BIP30-overwritten coinbases (91,722 and 91,812; their outpoints come back as 91,880's and
91,842's), not spent by the tip. Genesis's output was never added. Zero-value outputs are included. That is exactly
the set Core keeps (`AddCoin` skips `IsUnspendable`; `IsBIP30Unspendable` skips the two coinbases; the genesis
output is never added).

### 3.2 What a Core snapshot coin carries, and the gap

Core's `dumptxoutset` (format version 2, Core 28 to 31; checked on v31.1.0): `utxo\xff`, version u16, the network's
message start, the base block hash, the coin count u64, then the coins grouped by txid: txid, CompactSize count,
and per coin CompactSize vout and a `Coin` (VARINT code = height·2 + coinbase, then the compressed `CTxOut`:
VARINT of the compressed amount and the compressed script; a P2PK key of special type 4/5 is stored as x only and
decompressed to 65 bytes).

| leaf field | in the snapshot | from |
|---|---|---|
| txid, vout | yes | the snapshot |
| value | yes (compressed) | the snapshot, `DecompressAmount` |
| scriptPubKey | yes (compressed) | the snapshot, Core's `DecompressScript` (type 4/5: y from x and the parity) |
| height | yes (`code >> 1`) | the snapshot |
| flags bit 0, coinbase | yes (`code & 1`) | the snapshot |
| flags bit 1, BIP30-tracked | **no** | a side input: the list of tracked live outpoints (`--tracked`) |
| creation_mtp | **no** | a side input: the header chain from genesis to H (`--headers`) |

So the live-coin hash **can** be recomputed from a snapshot, with two side inputs a snapshot lacks. The 11-02
worry, a coin's position in its transaction or block, does not arise: the ECMH element has no position in it.
Positions are in the coin log's Merkle forest, not in the multiset hash. No new zyncAPP is needed, and no change to
the core image. (The issue's fallback, a zyncAPP that proves Core's own `hash_serialized_3` for H from the coin log,
was therefore not built or measured.)

- **The headers** are the node's own (a node syncs headers before it loads a snapshot), bound to the proof exactly
  as in §2 and required to run from genesis to the tip. 80·(H + 1) bytes: 16,080 at H = 200.
- **The tracked list** is small: on mainnet to 974,111, 173 transactions are tracked and 22 of their outputs are
  unspent (ROADMAP J4); measured here (§5): 0 at 200 and at 100,000, 101 at 174,000. It is derived from the blocks (`assume_inputs.py --tracked`:
  utreexo_ref.py's own `Tracked` rule over every block 1..H). A node that loads a snapshot does not have the blocks,
  so for mainnet the list would have to come with the proof or be published (§7).

**Neither side input is trusted.** The check passes only if the multiset of leaves built from the snapshot and the
side inputs has the proven hash. Every snapshot field is in the leaf preimage, so (SHA-256 and the ECMH being
collision-resistant) a pass means the snapshot's coins, field for field, are the live coins, each exactly once. A
wrong MTP or tracked bit changes a leaf: the true snapshot then fails, and a false one does not pass. Choosing the
side inputs adversarially is finding an ECMH collision with chosen elements, which is what the ECMH's security
(discrete logarithm in secp256k1, the hash to the curve modelled as a random oracle) rules out. The headers are bound
anyway; the tracked list is free, and can only refuse.

### 3.3 The check

`check_snapshot(cs, snapshot, headers, tracked)`, against the state of a receipt that verified:

1. `cs.profile` has bit 0 (libzync) and bit 6 (the live-coin hash), and NOT bit 1: an image that skipped scripts and
   signatures proves no coin set a node may load, as for assumevalid (§2). The headers run from genesis to the proven
   tip (§2 step 3).
2. Read the snapshot and refuse what Core's loader (`loadtxoutset`) refuses: another magic or version; a CompactSize
   that is not canonical or over `MAX_SIZE`; a VARINT out of range; a txid group of 0 coins or more than are left;
   vout ≥ 2³²−1; an amount outside `MoneyRange`; a coin above H; fewer coins than the header names; any byte after the
   last coin. And, stricter than Core's loader, in three places:
   - **Core's key order.** Txids strictly ascending (internal byte order, so each txid has exactly one group), and
     within a group the ENCODED vouts strictly ascending, compared bytewise as the coins database orders its keys
     ('C' ‖ txid ‖ VARINT(vout)). Not numerically: VARINT(16,511) = `ff7f` sorts after VARINT(16,512) = `808000`.
     Core's loader takes the coins in any order; the ECMH does not depend on it either, but the printed
     `hash_serialized_3` does, and only Core's own order gives Core's hash.
   - **A special-type 4/5 key whose x is not on the curve** (Core's `DecompressScript` fails and leaves an empty
     script; Core only compresses a key that passes `IsFullyValid`, so no real set has one).
   - **A raw script over 10,000 bytes** (Core reads it as `OP_RETURN`; such a coin is unspendable and never in a set).
   A coin at height 0, or above the headers' tip, is refused as well (before anything is sized by its height).
3. Each coin: its leaf (§3.1) with `creation_mtp` from the headers and flag bit 1 iff its outpoint is in the tracked
   list; add the leaf's point to the sum; add its value; feed Core's `hash_serialized_3` (below).
4. Require: the network magic is the proof's (mainnet `f9beb4d9`, signet `0a03cf40`); the base block is `tip_hash`;
   the total value is `utxo_value`; the sum is (`live_set`, `live_hash`).

**Only at the tip.** `live_hash` exists only in a proof's final state, so a snapshot at height S needs a proof whose
tip is S. A snapshot at another height is refused at step 4 (base block), and its coins relabelled with the right
base fail the hash.

**Core's own hash.** The checker also computes Core's `hash_serialized_3` of the snapshot (SHA-256d over each coin's
outpoint ‖ u32 (height·2 + coinbase) ‖ `CTxOut`, in the file's order) and prints it. The file's order is Core's
database order because step 2 requires it; without that, a true snapshot permuted or with a txid's coins split into
two groups would pass the ECMH and print a hash that is not Core's (review, 15.11.2026).
After a pass that value is derived from the proven UTXO set, so a node built with another `m_assumeutxo_data` entry,
or a reader comparing with the one compiled into a release, can use it without trusting whoever published it.

## 4. What is proven and what is trusted

| | |
|---|---|
| proven by the receipt | the chain 0..H is valid under Core v28's rules as compiled into image M, scripts and signatures included; the `ChainState` at H, including `live_set`, `live_hash`, `utxo_value`, `tip_hash` (the live hash is bound to the coin log, below) |
| checked by the verifier | the receipt (SWIFTSYNC_PROOFS.md §5); the headers link to `tip_hash`; the block named is among them; the snapshot's coins hash to the proven ECMH and sum to the proven value; base block and network |
| trusted | the image id M (rebuild it: `reproduce/`), RISC Zero's soundness, SHA-256 and SHA-256d collision resistance, the ECMH's collision resistance, the verifier's code, and the source used for the chain identity (`--expect`, your node, mempool.space) |
| not trusted | the snapshot, the headers file and the tracked list: each is checked or can only cause a refusal |
| not proven | that the proven chain is the network's (the identity check is the user's); anything above H |

**How the guest binds `live_hash` to the coin log.** In each block's aggregate (`prover/methods/guest/src/agg_lite.rs`,
`validate_block_lite`): every spend of an earlier coin pushes that coin's leaf onto `ext` (line 688), and every coin
the block creates and does not spend is pushed onto `appends` (line 709). The live hash then adds exactly `appends` and
subtracts exactly `ext` (lines 719-720), and the coin log is updated with the same two lists: each `ext` leaf is
flipped at its position, which must hold that unspent leaf under the previous roots, and `appends` are appended
(`coinlog_forest::apply`, line 736; a mismatch is `REJECT coin log`). The sum starts from the previous proven state's
(`main.rs` line 2294, `swiftsync::start`, which checks the carried point) and is committed after the block (line 2335).
From the genesis identity on, `live_hash` is therefore the ECMH of the coin log's unspent leaves. The header comment of
`prover/methods/guest/src/swiftsync.rs` (lines 1-9: "Not sound on its own … no inclusion proofs") is left from the
retired design B, where the sum stood alone; it is stale, but it is guest source, so changing it would move the image
id, and I have left it.

## 5. Measured (15.11.2026)

Real data only: the GPU proof of mainnet 1..200 under `8888d59c` (`swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt`,
profile `0x71`) and Core v31.1.0 `dumptxoutset` files from hazync-coord's node (a throwaway node synced from it over
P2P, §6). Laptop: WSL2, release build.

| what | result |
|---|---|
| the snapshot at 200 (`rollback=200`, 14,270 bytes, sha256 `39a837ca…`) | 204 coins, 1,000,000,000,000 sat; ECMH = the proof's `live_hash` (x `55ea17b1…3d01`); `hash_serialized_3` `a51f51ae…9d67` = Core's own `txoutset_hash` for the same dump |
| two dumps at 200 from two throwaway nodes | byte-identical (same sha256) |
| `assume_ref.py` (independent, pure Python) on the same files | the same `live_hash` and `hash_serialized_3` |
| BIP30-tracked live coins at 200 | 0 (`assume_inputs.py`); the match with an empty list agrees |
| assumeutxo check at 200, laptop | 0.01 s (the CLI's own timer) |
| throwaway node to 200 on hazync-coord | sync 28-31 s, `dumptxoutset rollback=200` 14-15 s |
| WASM build | the zAPP's own module `zapps/assume/wasm` (no longer part of the core's `hazync-verify.wasm`): 1,165,792 bytes raw, 331,972 gzipped, sha256 `582dc8fa…` (before it `5be95c43…`) (`zapps/assume/wasm/build.sh`: Rust 1.99.0 in the pinned container `reproduce/wasm.Dockerfile`, its published image; laptop and hazync-coord, 18.11.2026; on main's strict receipt decoder #758; before it `09704efb…`) |

**Beyond 200: against a prover run's state, not a proof.** No receipt in the repo reaches past 200, so the encoding
was also checked against hazync-coord's `libzync-prep` run of 07.11.2026 (`state_<h>.json`, the host's native
`ChainState`, profile 113 = `0x71`; its `state_200.json` has the proof's `live_hash` at 200) with
`cargo run --release --example assume_state` (testing only; it proves nothing about the chain). Snapshots by Core
v31.1.0 from throwaway nodes synced from hazync-coord's node.

| height | coins | result | rate (laptop, 16 threads / 1 core) |
|---|---|---|---|
| 100,000 | 71,888 (73% P2PK uncompressed, special types 4/5) | ECMH = prep's; `hash_serialized_3` `c10e49b7…9a06` = Core's `txoutset_hash`; 0 tracked coins live | 7.4-7.9 µs/coin / 20-23 µs/coin |
| 174,000 | 1,468,699 | ECMH = prep's **only with an EMPTY tracked list**; with the 101 tracked coins `assume_inputs.py` finds (73 coinbase, 28 not; created 149,732..172,561), and with either subset, it differs | 1.87-1.95 µs/coin / 15.8 µs/coin |

At 100,000 the single-reader stage bounds the rate (each P2PK key is decompressed, a square root, before the hash);
at 174,000 the map to the curve, run on every core, dominates. Earlier builds: 34-42 µs/coin at 100,000 with
`Fq::sqrt` and one thread.

**The 174,000 mismatch is the prover host's, not the checker's (finding, not fixed here).** The guest sets flag
bit 1 itself for a coinbase mined before BIP34 that indicates a later height (`agg_lite.rs`, `root || repeat`), e.g.
`a9f87755…6e10` (scriptSig `04 1f7e3201`, indicating 20,086,303). The host reads that flag from the block file
(`prep.rs` / `main.rs`: `coinbase_flags`, default 1; prevouts' `coin_is_coinbase` bit 1), and
`prover/fetch_block_rpc.py`, the deployed fetcher, writes neither (only `experimental/.../gpu/fleet/mkblock.py` and
the core-rules rig do). So prep's leaves for tracked coins lack bit 1, its live hash differs from what the guest
commits, and its coin log does too; by the guest code the first block proved on such a state (from the first
tracked coinbase, at or before 149,732) cannot join or spend cleanly. Not confirmed by a guest run here. The checker
follows the guest; until a proof at such a height exists, the tracked bit is checked against the guest's code only.

**Tests** (15.11.2026, this branch; all run by `zapps/assume/test.sh`, CI's zapps job): `cargo test --release` in `zapps/assume` (14 tests: the ECMH map and
sum against a num-bigint reference, partial sums, Core's amount and VARINT vectors, the MTP rule, the real snapshot, 16 tampered
or reframed snapshots, Core's key order (a permuted file against the real proof; a split group and a within-group
swap on Core's own dump at 1003; vouts 16,512 before 16,511), other heights, headers, profiles (scripts skipped, not
libzync), a coin above the headers' tip, assumevalid), `test-assume.sh` (31 CLI
cases with 3 identity cases and a permuted file, `test-assume.out`), the C ABI's `cargo test` (2 tests) and
`ffi/test_assume.c` (6 cases), `wasm/test-assume.mjs` (11 cases), and zapps/common's 6 (one explorer flag; a node
without chainwork is a FAIL). Run on hazync-coord, 15.11.2026: all pass. Every refusal case of the issue: a tampered
snapshot (txid, amount, height, coinbase flag, vout, script, a coin dropped, duplicated, split, or added), a wrong
height (199, 1003; the 199 coins relabelled as 200), a wrong chain (another network magic, another base block,
another tip in `--expect`, a flipped receipt), wrong side inputs (headers short, one timestamp changed, not from
genesis; a false tracked bit), and bad framing (count, trailing byte, truncation, version, magic).

## 6. Reproduce against a real node

```sh
scripts/libzync/assume_check.sh <H> <receipt> <swiftsync-verify> "bitcoin-cli -datadir=<your node>"
```

On a box with Core ≥ 28 and an unpruned node: a throwaway node (its own datadir under `$WORK`) syncs blocks 0..H from
your node over P2P (`-connect=127.0.0.1:8333 -stopatheight=H+1`), restarts offline, and dumps its UTXO set with
`dumptxoutset … rollback=H`. Then the headers and the tracked list (`assume_inputs.py`), both checks against the
receipt with `--bitcoin-cli` of your node for the identity, the independent reference, and a refusal (the snapshot at
H−1). Your node is never rolled back: `rollback` on a node at the tip disconnects every block above H and back,
which for H = 200 on hazync-coord's node (at 975,954 on 15.11.2026) is 975,754 blocks each way. Output at H = 200 on hazync-coord:
`scripts/libzync/assume_check_200.out`.

## 7. Open

- **A proof at the snapshot height.** `live_hash` is committed at a proof's tip only, so a node needs a proof whose
  tip is the snapshot's base block (for Core's own `m_assumeutxo_data` heights, a proof ending exactly there). The
  board's tip proof moves on; keeping (or joining to) a proof at chosen heights is a board decision.
- **The tracked list for mainnet.** 22 live coins at 974,111. Options: publish it with the proof, pin it in the
  verifier per network (like Core's chainparams), or have prep write it. Only completeness depends on it.
- **The large-H cost.** The check is one SHA-256, one hash-to-curve (a square root) and one point addition per coin,
  streamed (memory: the header times and one batch of leaves per worker); the map to the curve runs on one worker per
  core (one thread in WASM, or if threads cannot be started). Only the rates in §5 are measured.
- **The host's tracked flag (§5).** `fetch_block_rpc.py` writes no `coinbase_flags` and no prevout bit 1, so the
  prover host and prep gave tracked coins no bit 1 while the guest does. Confirmed by executing the guest on block
  147,396 (the first tracked coin): it accepts, and commits another coin log and live hash than prep. Fixed in the
  host (PR #736, `prover/host/src/tracked.rs`); prep restarts from genesis.
- **Core integration.** Core loads a snapshot only at heights in its chainparams and checks its compiled hash. This
  checker runs before `loadtxoutset` (or a patched node takes the `hash_serialized_3` it prints). Nothing in Core is
  changed.
