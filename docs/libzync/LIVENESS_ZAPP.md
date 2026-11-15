# libzync: coin liveness

Status (15.11.2026): built and tested on branch `feat/zapp-liveness`. Verifier and host code only: no guest changes,
so no image id moves. The privacy variant (§8) is an experiment, measured in execute mode only.

## 1. What it answers

One outpoint's state after a block H, from a verified core receipt and a small proof, and nothing else:

- **UNSPENT:** `txid:vout`, paying V sat to scriptPubKey S, created at height c, is unspent after block H;
- **SPENT:** that coin was spent at height h ≤ H, by input `vin` of the transaction at position `tx` in block h.

H is the proof's tip or any earlier height. Anyone with the receipt, the proof and a verifier (CLI, C ABI, WASM)
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

The verifier (`swiftsync_verify::check_liveness`, shared by the CLI, C ABI and WASM) runs only on a receipt that
passed the core checks (`verify`: image, real proof, genesis anchor, profile, block forest, time). Then:

1. **The question is the caller's.** The proof must name the outpoint and height the caller asked about. If the
   caller gives an amount, script or claimed state, those must match the proof too. A true fact about another coin
   or height is not an answer (ZYNCAPPS.md §8).
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
- **A repeated outpoint.** The answer is about the coin created at the reported height. On mainnet the two BIP30
  pairs have one leaf each (UTREEXO_FORMAT.md §3 rule 2). An outpoint created again over its own spent coin (possible
  from 1,983,702) would get a second leaf. A bridge should pin the creation height it expects.
- **Coins with no leaf:** a coin created and spent in the same block, unspendable outputs and the genesis coinbase.
  None of them has a liveness proof.

## 6. Tools

```sh
# producer: the history of a proving run, and the coin's fields from block files or your own node
zync-path live <history.bin> <tip> <at|tip> <txid>:<vout> (--blocks <dir> | --bitcoin-cli "<cmd>") [--height <h>] > p.hex
zync-serve <history.bin> <addr:port> [state_<tip>.trie] --blocks <dir>      GET /live/<at|tip>/<txid>/<vout>
# verifier: YOUR question, then the proof
swiftsync-verify --liveness p.hex '<txid>:<vout>@<h|tip>[=unspent|=spent[:<h>[:<tx>:<vin>]]][,value=<sat>][,script=<hex>]' <receipt>
```

In the C ABI (`swiftsync_verify_facts`) and WASM (`verifyFacts`), the facts JSON takes
`"liveness": [{"proof": "<hex>", "ask": "<question>"}]`. There the question must claim its answer (`=unspent` or
`=spent…`), because those calls return only whether a claim holds.

The fields come from the creating block. A block does not show the BIP30 flag, so the producer tries both values and
keeps the leaf the log holds. With `--bitcoin-cli` and no `--height`, the node needs `-txindex`.

## 7. Tests and measured numbers

All against the real proof of mainnet 1..200 (image `8888d59c`, `swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt`)
and that run's `history.bin` (`vectors/liveness/`), 15.11.2026:

| suite | result |
|---|---|
| libzync unit tests (`liveness::tests`) | 2/2 |
| swiftsync-verify `tests/liveness.rs`: honest answers, wrong amount / script / height / flag, spent claimed unspent, unspent claimed spent, an outpoint that never existed (209 positions × 2 coins), H above the tip, mismatched questions, undo misuse, every bit flipped, a dev-mode and a tampered receipt | 8/8 |
| zync-serve `/live` route | 1/1 |
| CLI, `vectors/liveness/run.sh` | 23/23 |
| C ABI, `swiftsync-verify-ffi/test.c` | 20/20 (7 liveness) |
| WASM, `swiftsync-verify-wasm/test-liveness.mjs` | 13/13 |

The real answers: block 9's coinbase (`0437cd7f…:0`) is UNSPENT after 169 and SPENT at 170 by transaction 1 input 0.
The first payment's 10 BTC to Hal Finney (`f4184fc5…:0`) is UNSPENT at 200. Its 40 BTC change (`…:1`) is unspent
after 170 and SPENT at 181.

| proof | bytes | check (one core, release, Ryzen 7 7730U) |
|---|---|---|
| at the tip, unspent / spent (log path 5 or 6 hashes) | 333–377 | 17.9–29.9 µs |
| below the tip, with the opening (block path 7 hashes) | 574–734 | 20.8–32.1 µs |
| the core receipt, once per tip | 227,362 | 49.5 ms |

Mainnet sizes are not measured here. By the format, each extra log level costs 32 bytes, and an opening costs 32
bytes per root and per block-path level.

**WASM frame fix.** Since `f713e3a6` (13.11.2026), the WASM `verify_facts` read its optional lineage chunk before
the facts, so it took the facts JSON for a lineage. Every `verifyFacts` call with facts failed, and so did `verify`
with an expected tip. The lineage is now the last chunk in both frames, and `test-liveness.mjs` checks a trie fact
and an expect.

## 8. Privacy variant (experiment, `experimental/libzync-bench/liveness-anon`)

**Statement:** "I know a coin unspent at the core tip, worth at least X sat, whose script satisfies a policy."
It is a small zyncAPP guest. It verifies the core receipt in the zkVM (`env::verify`) and runs the same coin check on
private inputs. It commits the core statement, X, the policy and a context tag, never the coin.

- **Policy 0, script set:** SHA-256(script) is in a Merkle tree whose root R is public, so R can commit "one of these
  scripts". There is no ownership and no nullifier: anyone who knows such a coin can prove it, as often as they like.
- **Policy 1, key:** the coin is P2TR, and the prover knows the secret of its output key (x(sk·G) equals the key in
  the script). The guest commits `nullifier = SHA-256("lzlv-anon/nullifier" ‖ context ‖ sk ‖ txid ‖ vout)`: one per
  coin and context. A gossip layer can then allow one announcement per nullifier. Without sk, the nullifier cannot be
  matched to a coin by trying every outpoint.

**What it hides:** which coin, its exact amount, its script or key and its position. This relies on RISC Zero
receipts being zero-knowledge, which RISC Zero states for its proof system; that was not checked here.

**What it does not hide:**

- that some coin worth at least X, meeting the policy, existed unspent at that tip;
- the core tip and image, X, R and the context;
- the anonymity set, which is every unspent coin worth at least X that meets the policy. A small set R, or a high X,
  can leave one candidate;
- under policy 1, the nullifier, which links every proof made for the same coin in the same context;
- timing, and whatever the network layer reveals.

The prover sees everything, so the coin's owner must prove on their own hardware. Under policy 0, the statement says
nothing about who holds the coin.

**Cycles** (execute mode, r0vm 3.1.0, 15.11.2026; real inputs: the 1..200 receipt, Hal's coin):

RESULTS_PLACEHOLDER

## 9. Open

- **Mainnet-scale numbers.** The host's history replay is in memory; a mainnet path service needs the disk-backed
  log (COINLOG.md §8, ROADMAP C1).
- **Non-membership.** "This outpoint never existed" would need an outpoint-keyed commitment, which is a core change.
- **A liveness zyncAPP receipt.** The `coin` guest (ZYNCAPPS.md) already proves the tip case as its own receipt.
  Whether a receipt that answers at H below the tip is worth publishing is undecided.
- **Privacy variant:** whether to keep the key policy P2TR-only (P2WPKH would add RIPEMD-160), whether X should be
  hidden too (a range), and proving cost on a user's own machine (not measured).
