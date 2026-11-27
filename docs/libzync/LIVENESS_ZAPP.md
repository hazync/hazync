# libzync: coin liveness

Status: built and tested 15.11.2026, independently reviewed the same day (no soundness break found) and released in
v0.27.0 (#741, 20.11.2026) on RISC Zero; engine (OpenVM) core proofs only since 27.11.2026, node format 2. Verifier
and host code only: no guest of its own. No engine image is published yet (the Stage 4 freeze), so the check names
the core image and its key (`--image`, `--vk`). The privacy variant (§8) is an experiment, measured in execute mode only.

## 1. What it answers

One outpoint's state after a block H, from a verified core proof and a small liveness proof, and nothing else:

- **UNSPENT:** `txid:vout`, paying V sat to scriptPubKey S, created at height c, is unspent after block H;
- **SPENT:** that coin was spent at height h ≤ H, by input `vin` of the transaction at position `tx` in block h.

H is the core proof's tip or any earlier height. Anyone with the core proof, the liveness proof and a verifier (CLI, C ABI, WASM)
can check it. Uses:

- **Bridges and pegs:** check that a deposit outpoint has not moved, at each new tip proof. A SPENT answer gives the
  height and the input that spent it.
- **Lightning:** check that a channel's funding output exists, with the announced amount and script, and is
  unspent, without running a full node (gossip validation).
- **Ark and statechains:** check that the on-chain root output is still live.

## 2. Do the existing facts answer it?

The committed data does. The existing fact formats are not quite right for it, so this adds a stricter encoding.

- **The coin log leaf binds the outpoint.** Each leaf hashes the txid, vout, amount, script, creation height,
  coinbase flag, BIP30 flag and creation MTP (UTREEXO_FORMAT.md §2). A spend replaces the leaf with
  `spent(leaf, spender)`, where the spender is (height, tx, vin) (COINLOG.md §2). So liveness is `facts::coin_unspent`
  or `facts::coin_spent_by` at the coin's position. H below the tip is opened through the block forest
  (`block_in_chain`, HANDOVER.md §3). No core change, no new guest.
- **Why not `CoinFact` as it is.** It is keyed by log position, and the coin's fields are optional (required only for
  UNSPENT). At the tip its `log_*` fields are ignored, but they are not required to be empty, and its hex is
  case-insensitive. That is the #695 shape: bytes the check does not bind. It is fine for a journey, but an answer a
  bridge acts on should have exactly one encoding.
- **Why not the trie's coin fact.** It works at the tip only. Its coin key is not injective ((txid ^ d, vout ^ d)
  share a key), so an UNSPENT or SPENT answer needs the creating transaction attached (`origin`). The coin log
  leaf binds the outpoint by itself, at any height.
- **What none of them gives:** proof that an outpoint never existed. The log is in birth order, not keyed by
  outpoint. The trie's ABSENT is per (script, outpoint): "no coin with this outpoint was ever paid to this
  script". For liveness this does not matter: an outpoint that never existed has no proof, so every claim about it is
  refused.

## 3. The proof (`LZLV` version 1, `libzync/src/liveness.rs`)

All integers are little-endian. The brackets mark parts present only when the state or a flag says so.

```
"LZLV" ‖ version 1 ‖ state (0 unspent, 1 spent) ‖ flags (bit 0 coinbase, 1 BIP30-tracked, 2 opening, 3 undo)
‖ txid 32 (internal order) ‖ vout u32 ‖ value u64 ‖ creation height u32 ‖ creation_mtp u32 ‖ script length u16 ‖ script
‖ [spent: height u32 ‖ tx u32 ‖ vin u32]
‖ at u32 ‖ log position u64 ‖ path length u8 ‖ path 32 × length                     the coin log after `at`
‖ [opening, iff at < tip: log leaves u64 ‖ live value u64 ‖ one root per set bit of the leaf count, lowest level first
   ‖ block hash 32 ‖ [undo hash 32] ‖ block path length u8 ‖ block path 32 × length]
```

**One encoding per proof.** Every byte is read. Unknown flag bits and states are refused, as are a script over
10,000 bytes, an undo hash without an opening, trailing bytes and any input whose re-encoding differs. The root
count follows from the leaf count, and a path must be exactly its tree's height. The test `every_byte_is_bound` flips
every bit of the five real proofs (18,904 flips): every one is refused by the proof check alone.

## 4. The check

The verifier (`zapp_liveness::check_liveness`, in the zAPP's own module `zapps/liveness`, shared by its CLI, C ABI and
WASM; ZYNCAPPS.md §9) runs only on a core proof that passed the core checks (`verify`: image and pinned key, the engine proof, genesis anchor, profile, block forest, time). Then:

1. **The question is the caller's.** The proof must name the outpoint and height the caller asked about. If the
   caller gives an amount, script, creation height (`created=`), coinbase flag or claimed state, those must match the
   proof too. A true fact about another coin or height is not an answer (ZYNCAPPS.md §8). A question has one
   spelling: lower-case hex, decimal numbers without a sign or leading zeros; the C ABI and WASM take the proof as
   exact lower-case hex.
2. `at ≤ tip`. At the tip, the journal's coin log roots are used. Below it, the opening's
   `utxo_state(leaves, value, roots)` and block hash must give H's leaf in the tip's block forest. The undo hash is
   needed exactly from the image's undo height on (UNDO.md).
3. The coin's leaf is recomputed from its fields. UNSPENT needs that leaf at the position; SPENT needs
   `spent(leaf, spender)` there, with the spend height ≤ H.

The answer is `unspent` or `spent:<h>:<tx>:<vin>`, followed by the coin's amount, script and creation height.

## 5. What is proven and what is not

Proven by an OK verdict: the chain from genesis to the tip is valid (SOUNDNESS.md §1), and in it the coin with
exactly these fields was in the stated state after block H.

Not proven, or left to the caller:

- **Which chain.** As for the core proof, compare the tip with a source you trust (`--expect`, `--mempool`,
  `--bitcoin-cli`, or `expect` in the C and WASM calls). Without that, or below the minimum chainwork, the verdict is
  DEMO. Every proof the board makes before block 970,000 is DEMO.
- **Anything after the tip.** A coin unspent at H may have been spent since. Freshness comes from proving the tip
  again; the receipt shows only how old its tip is.
- **The producer is not trusted.** zync-path and zync-serve hold no secret. A wrong proof is refused.
- **A repeated outpoint: pin it with `created=`.** On mainnet the two BIP30 pairs have one leaf each
  (UTREEXO_FORMAT.md §3 rule 2). An outpoint created again over its own spent coin (possible from 1,983,702) gets a
  second leaf, so without a pin BOTH `=spent` (the old coin) and `=unspent` (the new one) verify, each true of its own
  coin. Give `created=<h>` (and `coinbase=0|1` if you like): the check compares it before the answer. The producer,
  asked by outpoint alone, gives the latest coin; `--height` picks another. I tested this on a synthetic chain
  (`a_recreated_outpoint_is_pinned_by_created_and_the_producer_names_the_latest`); mainnet has no such coin yet.
- **Coins with no leaf:** a coin created and spent in the same block, unspendable outputs and the genesis coinbase.
  None of them has a liveness proof.

## 6. Tools

```sh
# producer: the history of a proving run, and the coin's fields from block files or your own node
zapp-liveness prove <history.bin> <tip> <at|tip> <txid>:<vout> (--blocks <dir> | --bitcoin-cli "<cmd>") [--height <h>] > p.hex
zapp-liveness serve <history.bin> <addr:port> --blocks <dir>                GET /live/<at|tip>/<txid>/<vout>, GET /tip
# paths in node format 2, the engine chain's
# verifier: YOUR question, then the proof (the core proof is verified by the core verifier's library)
zapp-liveness --image <core id> --vk core.vk --vk-sha256 <the key file's pin> [--expect <h>:<hash> | --mempool | --bitcoin-cli "<cmd>"] --liveness p.hex '<txid>:<vout>@<h|tip>[=unspent|=spent[:<h>[:<tx>:<vin>]]][,value=<sat>][,script=<hex>][,created=<h>][,coinbase=0|1]' <proof.hze>
```

In the C ABI (`zapp_liveness_verify`, `zapps/liveness/ffi/include/zapp_liveness.h`) and WASM (`verifyLiveness` in
`zapps/liveness/wasm/zapp-liveness.js`), the facts JSON is `{"liveness": [{"proof": "<hex>", "ask": "<question>"}]}`, those keys only: any other key at either level, a key given twice, or a top level or fact that is not an object is refused (`zapp_liveness::facts_from_json`, review 18.11.2026). There the question must claim its answer (`=unspent` or
`=spent…`), because those calls return only whether a claim holds.

The fields come from the creating block. A block does not show the BIP30 flag, so the producer tries both values and
keeps the leaf the log holds. With `--bitcoin-cli` and no `--height`, the node needs `-txindex`.

## 7. Tests and measured numbers

All against the engine chain's proof of mainnet 1..200 (`swiftsync-verify/tests/data/engine200`: `proof.hze`, its
image id and key, and that run's `history.bin`), the liveness proofs in `zapps/liveness/vectors/engine` (node format
2), 27.11.2026; all of it run by `zapps/liveness/test.sh`. Every honest claim is DEMO: 200 blocks are below Core's
minimum work.

| suite | result |
|---|---|
| libzync unit tests (`liveness::tests`) | 2/2 |
| zapp-liveness `tests/liveness.rs` and `tests/engine.rs`: honest answers, wrong amount / script / height / flag, spent claimed unspent, unspent claimed spent, an outpoint that never existed, H above the tip, mismatched questions, undo misuse, a tampered proof, `created=` / `coinbase=`, a re-created outpoint, one spelling per question, the 0x05 undo leaf, regtest above block 200, every bit flipped, the facts JSON's keys | 13 + 4 (1 timing test ignored) |
| `zapp-liveness serve` `/live` route | 1/1 |
| CLI and producer, `vectors/run.sh` | 33/33 |
| C ABI, `ffi/test_liveness.c` | 20/20 |
| WASM, `wasm/test-liveness.mjs` | 30/30 |
| `zapps/common`, `zapps/corecheck` | pass (`zapps/common/test.sh`, `zapps/corecheck/test.sh`) |

The RISC Zero runs of 15.11-18.11.2026 (34, 21 and 32 cases on that chain's receipt of 1..200) are in git history. On RISC Zero, hazync-coord, 15.11.2026: single-bit flips, each refused by the proof check alone: 18,904 on the five
mainnet proofs, 27,760 on five 0x05 undo-leaf openings, 20,840 on six regtest proofs, and every bit of the two
synthetic re-created-outpoint proofs.

Beyond mainnet 1..200, three more sets of data:

- **The 0x05 undo leaf** on real data: the mainnet 1..200 run of a development image with the undo commitment
  (`vectors/undo_dev_1_200_*`), from hazync-coord, 14.11.2026 (node format 1). The test takes the host's chain state,
  not a verified proof; the engine chain's own proof of 1..200 has undo leaves at every block and is checked above.
- **Above block 200:** the regtest base chain of the consensus rig, 0..232, Core 28.0 with
  `-testactivationheight=bip34@110` (`vectors/regtest_*`, node format 1), the proofs written by `zapp-liveness prove
  --bitcoin-cli` against a copy of that node. It includes coins spent at 200 and 201, openings at 200 and 220, and two
  **BIP30-tracked coins** (flag bit 1), which mainnet 1..200 does not have. These are execute-mode states too.
- **A re-created outpoint:** synthetic, as above (§5).

The real answers: block 9's coinbase (`0437cd7f…:0`) is UNSPENT after 169 and SPENT at 170 by transaction 1 input 0.
The first payment's 10 BTC to Hal Finney (`f4184fc5…:0`) is UNSPENT at 200. Its 40 BTC change (`…:1`) is unspent
after 170 and SPENT at 181.

| proof | bytes (the engine vectors) | check (one core, release, Ryzen 7 7730U; on the RISC Zero chain's proofs, 15.11.2026) |
|---|---|---|
| at the tip, unspent / spent (log path 5 or 6 hashes) | 333–377 | 17.9–29.9 µs |
| below the tip, with the opening (block path 7 hashes) | 606–766 (574–734 on the RISC Zero chain) | 20.8–32.1 µs |
| the core proof, once per tip (engine, `engine200/proof.hze`) | 275,183 | not re-measured (RISC Zero's 227,362-byte receipt: 49.5 ms) |

Mainnet sizes are not measured here. By the format, each extra log level costs 32 bytes, and an opening costs 32
bytes per root and per block-path level.

**WASM frame fix.** Since `f713e3a6` (13.11.2026), the WASM `verify_facts` read its optional lineage chunk before
the facts, so it took the facts JSON for a lineage. Every `verifyFacts` call with facts failed, and so did `verify`
with an expected tip. The lineage is now the last chunk in both frames (PR #734, `swiftsync-verify-wasm/test-facts-frame.mjs`).

## 8. Privacy variant (experiment, `experimental/libzync-bench/liveness-anon`; RISC Zero, execute mode, not ported to the engine)

**Statement:** "I know a coin unspent at the core tip, worth at least X sat, whose script satisfies a policy."
It is a small zyncAPP guest. It verifies the core receipt in the zkVM (`env::verify`), checks the core statement
against the network's pinned genesis anchor, and runs the same coin check on private inputs. It commits the core
statement, X, the policy (with the set's root and depth) and a context tag, never the coin.

- **Policy 0, script set:** the script is a leaf of a Merkle tree whose root R and depth are public, so R can commit
  "one of these scripts". Leaves are SHA-256(0x00 ‖ length ‖ script) and nodes SHA-256(0x01 ‖ L ‖ R), and the path
  must be exactly the committed depth. My first version hashed the bare script and let the path be any length, so a
  coin paying the 65-byte script 0x01 ‖ L ‖ R could stand in for an inner node (review, 15.11.2026). There is no
  ownership and no nullifier: anyone who knows such a coin can prove it, as often as they like.
- **Policy 1, key:** the coin is P2TR, and the prover knows the secret d of its output key (x(d·G) equals the key in
  the script). An x-only key is met by d and n − d alike, so the guest first takes the one whose point has even Y
  (as BIP340 does), then commits `nullifier = SHA-256("lzlv-anon/nullifier" ‖ context ‖ d_even ‖ txid ‖ vout)`: one
  per coin and context. My first version hashed d itself, which gave two nullifiers per coin. Without the secret, the
  nullifier cannot be matched to a coin by trying every outpoint.

**What the verifier of such a receipt must check,** because the guest cannot: `core_id` is a core image it trusts
(an allow-list) and `lineage` is that image's; `cum_work` reaches the network's minimum and `tip_hash` is the
network's block at `height` (from a source it trusts); `max_time` is not in the future; X, the policy, R and its
depth are the ones it asked for; and the `context` is its own, fixed in advance. A context the prover chooses makes
the nullifier worthless.

**What it hides:** which coin, its exact amount, its script or key, and its position. This relies on RISC Zero
receipts being zero-knowledge, which RISC Zero states for its proof system; I have not checked that.

**What it does not hide:**

- that some coin worth at least X, meeting the policy, existed unspent at that tip;
- the core tip and image, X, R, the depth and the context;
- the anonymity set: every unspent coin worth at least X that meets the policy. A small R or a high X can leave one
  candidate;
- **the execution length.** The cycle count grows with the script's length and with the coin's log-path length, and
  the path length says which tree of the coin log holds the coin: a coarse range of its birth order. I did not pad
  either, so assume a receipt leaks them until both are padded to fixed maxima;
- under policy 1, the nullifier. It links every proof for the same coin in the same context, and it is the same at
  every tip, so an observer sees the set of coins it could be shrink as coins are spent, and sees when it stops
  appearing;
- timing, and whatever the network layer reveals.

**Who can use policy 1.** The guest needs the raw tweaked secret as input. A hardware signer never releases it, and a
MuSig2 or FROST key has no single secret, so policy 1 as built excludes both. Lightning funding outputs are
multi-party (2-of-2) keys, so it does not serve Lightning gossip as it stands; that would need another design (a signature checked
in the guest, and a nullifier that does not come from a single secret), which I have not built. The prover sees every input, so the coin's holder must prove on
their own hardware. Under policy 0 the statement says nothing about who holds the coin.

**Cycles.** Execute mode, r0vm 3.1.0, hazync-coord, 15.11.2026, after the fixes above. Real inputs: the 1..200 receipt
and Hal's coin.

| stage | cycles |
|---|---|
| reading the inputs (the 2,068-byte core journal, the coin, the paths; risc0 serde reads each byte as a word) | 291,188 (set of 1) – 383,823 (set of 2^20) |
| the core statement (`env::verify` call, journal decode, `check_core_statement` with the pinned anchor) | 39,674 |
| the coin: its leaf, its log path (6 levels at block 200), value ≥ X | 8,233 |
| policy 0, set membership, 2^0 / 2^10 / 2^20 scripts | 2,580 / 15,224 / 27,839 |
| **policy 0 in all**, set of 1 / 2^10 / 2^20 | **368,708 / 427,826 / 486,751** (one segment, 2^19 / 2^19 / 2^20 after padding) |
| policy 1's own work, measured alone: d·G (risc0's accelerated k256), the even-Y secret, the nullifier | 112,303 (d) / 148,853 (n − d) |

I ran these refusals, each one counted only when the guest panicked for the stated reason: min_value above the
coin's amount, a script not in the set, a set path shorter than the committed depth, and the mainnet receipt asked as
signet. d and n − d give the same key and the same nullifier. Policy 1 was not run end to end on real data: blocks
1..200 have no P2TR coin, and no real coin's key is mine. Reading the inputs dominates the cost. Reading the journal
as raw words (`env::read_slice`) is the obvious next step, and I have not measured it. Not counted: resolving the
core receipt (the assumption) at proving time, one fixed recursion step per receipt. I did not measure proving time
(no GPU run). Raw output: `experimental/libzync-bench/liveness-anon/RESULT.txt`.

## 9. Open

- **Mainnet-scale numbers.** The host's history replay is in memory; a mainnet path service needs the disk-backed
  log (COINLOG.md §8, ROADMAP C1).
- **Non-membership.** "This outpoint never existed" would need an outpoint-keyed commitment, which is a core change.
- **A liveness zyncAPP receipt.** The `coin` guest (ZYNCAPPS.md) already proves the tip case as its own receipt.
  Whether a receipt that answers at H below the tip is worth publishing is undecided.
- **Privacy variant:** a design that works with hardware and multi-party keys (Lightning); padding the execution
  length; whether X should be hidden too (a range); P2WPKH (RIPEMD-160 in the guest); proving cost on a user's own
  machine (not measured).
