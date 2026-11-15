# zyncAPP: a proven Shielded CSV nullifier accumulator for light clients (design notes)

Status (15.11.2026): **design notes only.** No code, no build, nothing measured for this zAPP. Every cost below is an
ESTIMATE derived from numbers measured for other guests in this repo, and says which. Written to prepare for a yes from
the Shielded CSV authors; it changes nothing in the core or in any other zAPP.

Shielded CSV is Jonas Nick, Liam Eagen and Robin Linus's protocol. This zAPP adds nothing to it. It is an additive
product on a Hazync chain proof: *"after block H of the chain that the core receipt proves valid from genesis, the
Shielded CSV nullifier accumulator has value v_H"*, so a light client can hold v_H without scanning blocks.

**What I read.** The paper, IACR ePrint 2025/068 (32 pages, dated 20.09.2024). The ePrint PDF is byte-identical to
the authors' GitHub release `shieldedcsv.pdf` (ShieldedCSV/ShieldedCSV, release `2024-09-20`): both have SHA-256
`889acc42…37a6a96a`, 936,772 bytes, and the same PDF creation date (20.09.2024 15:44:58 UTC). So the ePrint changes
nothing, and every section and page number below is the ePrint's and the whitepaper's alike; the slips listed in §1
are in the ePrint as published. The
pseudocode repository (ShieldedCSV/ShieldedCSV `main` `3a1c603b`, `src/node.rs`, `src/lib.rs`, `src/primitives.rs`;
the `paper` branch differs only in the README); the bitcoindev thread (24.09.2024 onwards) and Jonas Nick's reply of
26.09.2024 on light clients; Argo (idealgroup/argo `aa28ba8`, code only, not the ePrint 2026/2165 text); and, as a
third-party data point only, the zkCoins documentation (`zkcoins-docs.pages.dev/protocol`), which describes an
implementation of the paper with its own registered deviations.

## 1. What the paper specifies

**The nullifier.** §2 (pp. 7-8) builds it up: a nullifier is a *nullifier public key* plus a Schnorr signature by it
over a fixed message that commits to the Shielded CSV transaction by sign-to-contract, so the transaction hash is not
on chain: `Nullifier := (Nullifier Public Key, Signature)` (p. 8). Each public key nullifies one account state, and the
key is fresh per state; an account's earliest nullifier key is its account ID (§4.2 "Account Creation", p. 22).

**On chain only as aggregates.** "In Shielded CSV, nullifiers appear only in the form of AggregateNullifiers" (§4.2,
p. 19). `Aggregate Nullifier := (Nullifier Public Keys, Aggregate Signature, Publisher Address)` (p. 10): a list of
keys, one NISSHAC half-aggregate signature `(R_1, …, R_n, s)` (§4.1, p. 17), and a commitment to the publisher's
account for the fee (a Pedersen commitment, one group element, §4 p. 17; `fee_acct_comm: Commitment`, 32 bytes, in
`lib.rs`). One-key aggregates are allowed (p. 8). **The 64 bytes** are, per nullifier, one public key and one group
element R_i on a 256-bit curve; the scalar `s`, the publisher address and "the remainder of the blockchain transaction"
are a constant per aggregate (p. 8, "approaches 64 bytes"). The code types keys as secp256k1 `XOnlyPublicKey`
(32 bytes); the encoding of R_i is not given.

**Publishers** collect nullifiers, half-aggregate them and post them (§1 "Publishing Nullifiers" p. 4, §2 p. 8,
§4.2 "Publishing" p. 21); anyone may be one, and the earliest to post a nullifier gets its fee (p. 10). Competing
publishers posting the same nullifier is therefore expected, not an attack.

**How an aggregate is carried in a Bitcoin transaction is not specified.** The paper says only "posted to the
blockchain" and that for non-users nullifiers are "arbitrary data" with no effect on the UTXO set (pp. 5, 7);
`Node::process_block` takes `aggregate_nullifiers: Vec<AggregateNullifier>` as an argument, already extracted. No
OP_RETURN, witness, tag, magic bytes or serialisation appears in the paper or the code. (zkCoins' documentation puts it
in witness data, "~64 bytes of witness data (≈ 16 vB) per transition"; that is theirs, not the paper's.)

**The per-nullifier check.** An aggregate "is valid if the signature verifies for the public keys and the static
message 'Shielded CSV: state update'" (§4.2, p. 19): one `SigAggregateVerify` over (pk_i, m) for all i (§3.4, p. 11),
the half-aggregation of Chalkias et al. [13]. The sign-to-contract part is invisible to it: `R_i = R'_i +
HSigComm(R'_i, m_SC)·G` (§4.1 `SigSign`, p. 17), and `SigCommRetrieve(σ, i) = R_i` is the transaction commitment that
gets stored. So a scanner checks the half-aggregate equation and nothing about the transaction it commits to.
In `node.rs`, an aggregate whose signature fails is skipped whole (`continue`).

**process_block** (§4.2, p. 19; `node.rs`): for each aggregate in the block, for each key in it, if the key is already
in the node's key-value store it is **ignored** ("crucial … analogous to a double-spend"); otherwise insert
`pk → (R_i, location, publisher address)`. Then append the block's set of new entries to the nullifier accumulator,
and append the new accumulator value to the list of historic values. `location` is `height·2^24 + k` taken as 6
bytes, k counting inserted keys in the block (`node.rs`); `lib.rs`'s comment says 21 bits of height and 22 of index,
which does not match the code.

**The accumulator** is a ToS-accumulator, a tuple of sets (§3.6, pp. 14-15): append a set, remove the last set,
membership in the union, IsPrefix (one value's tuple is a prefix of another's), DistinctElement. The instantiation
ToSAccMT (Appendix A.2, pp. 30-32, Fig. 4): an outer Merkle tree whose leaf for set i is `(i, v0, v1)`, v0 the outer
root before it, v1 the root of an inner Merkle tree over the set's elements. Membership = inner path + outer path;
IsPrefix(v, v') = the outer leaf `(n+1, v, ·)` under v'; DistinctElement = two outer leaves at the same i with
different v1. The set element in code is `(PublicKey, SigCommitment, BlockchainLocation, Commitment)`. **No
non-membership:** that is the separate account-level spent accumulator (§3.5), not this one.

**What a node needs it for.** `accept_payment` (§4.2, p. 19) checks the coin's PCD proof, which is only relative to the
accumulator value inside the coin, then that this value is in the node's list of historic values, i.e. a value the
node computed from the chain. With ToSAccMT that list check is an IsPrefix proof against the latest value. A sender's
proofs need the **union membership** of its own nullifier (the compliance predicate, p. 21), so it needs the inner tree
of the block holding it. A wallet keeps everything else itself (§6.2, p. 24).

**Reorgs.** Remove one set per disconnected block, trim the history, re-run on the new blocks (p. 19); the
"conditional NAV" keeps an account usable after a reorg (§2 pp. 10-11, §4.2 pp. 20-21).

**Light clients.** Not in the paper. Jonas Nick, 26.09.2024: a light client could follow the chain by proof of work,
"obtain the nullifier accumulator value from elsewhere" with a succinct proof that the chain and the accumulator are
valid, and so receive; to send it must prove inclusion in the accumulator, which needs the nullifiers, and for that
there is "nothing concrete yet".

**Height rules.** None in the base protocol beyond block order. Appendix A.1.1 (p. 28) sketches time-locks: a
locktime h in the nullifier, the message becomes "Shielded CSV: state update h", and the scanner skips nullifiers
whose locktime is not yet reached (the printed sentence says "less than the current blockchain height", the inverse
of the rule stated just above it; I read it as a slip).

### Left open or as pseudocode

1. **The Bitcoin carrier**: where an aggregate sits (output script, OP_RETURN, witness, annex), how it is recognised,
   its byte layout, and whether a transaction may carry several.
2. **The start height**: `process_block` runs "when receiving a new block"; from which height the accumulator begins
   is not stated, and v_H depends on it (one outer leaf per block, empty blocks included, since `append_set` is called
   for every block).
3. **Order**: which of two copies of a key in one block wins (aggregate order within a transaction, transaction order).
   It decides the stored R_i, publisher and location.
4. **Hashes and trees**: HSig, HSigComm, the half-aggregation coefficients, the Merkle tree (an append-only tree with
   `Remove`), leaf and node encodings, the empty inner tree, the encoding of `(i, v0, v1)`. All primitives in
   `primitives.rs` are `unimplemented!()`.
5. **The message**: `Message::from_digest_slice("Shielded CSV: state update")` is 26 bytes and would be refused by
   rust-secp256k1 (it needs 32); BIP-340 takes any length, so the intended hashing is open.
6. **The signing equation as printed** (§4.1 `SigSign` line 4, "s = k + HSig(R, P, m)", and p. 8 "sG = R +
   HSig(R, P, m)G") drops the key and the commitment tweak; the intent is clearly `s = k + HSigComm(R', m_SC) +
   HSig(R, P, m)·x`. Verification is unaffected.
7. **Invalid aggregates**: the paper says valid aggregates are processed; the code skips a whole invalid one. Whether a
   partially valid aggregate can count is not discussed (half-aggregation cannot tell which signature failed).

## 2. The design

Everything here is conditional on a pinned answer to items 1-4 above. The zAPP carries a `spec` field; any change to
the carrier, start height, order or hashes is a new `spec` and a new image, never a silent change (as `format` in the
hints zAPP).

### Statement (`ScsvStatement`, kind 'ZS' version 1)

```
ScsvStatement {
  kind, self_id, network            as the Utreexo zAPP (UTREEXO_ZAPP.md §1)
  spec          u32       the Shielded CSV rule set: carrier, start height A, order, hashes, tree encodings
  start         u32       A, the earliest height scanned (blocks below A contribute no set)
  height        u32       H
  block_hash    [u8;32]   block H, internal byte order
  to, runs, state_out, done          chained runs, as the Utreexo zAPP
  acc_value     [u8;32]   the ToSAccMT outer root after H: v_H, what a node's history list ends with
  n_sets        u64       H − A + 1 (one per block, empty sets included)
  n_nullifiers  u64       keys inserted (later copies not counted)
  n_aggregates  u64, n_invalid u64, n_repeats u64     aggregates seen, aggregates skipped, keys ignored as repeats
  seen_root     [u8;32]   root of the seen-key set (§2.4), so "key K was never nullified by H" is provable
  core binding            core_id, lineage, anchor_id, core height T (H ≤ T), tip_hash, cum_work, max_time, profile
}
```

### 2.1 Inputs and framing

As the hints and Utreexo zAPPs: the core receipt and journal, network, `spec`, H; then per block A..H one frame
`block bytes ‖ state_h ‖ undo_h`, plus whatever the carrier needs (§2.2), plus the seen-key witnesses (§2.4); then the
block-forest leaf hashes for 1..A−1 and H+1..T. Blocks below A are not walked: their leaves are host-supplied hashes
that must fold to the core's block forest (the hints zAPP measured about 1,600 cycles for such a leaf).

### 2.2 The walk, bound to the core proof

Per block h = A..H, the hints walk (HINTS_ZAPP.md §3 step 2): hash the header, require it to follow the previous one,
rebuild the Merkle root from the transactions (CVE-2012-2459 refused, depth fixed by the coinbase), and fold
`leaf_u(hash, h, state_h, undo_h)` into a block forest that must equal the journal's `block_roots` / `block_leaves`
(UNDO.md §3). That binds every header and so every transaction to the proven chain. `state_h` and `undo_h` are opaque
here; nothing about coins is checked.

**Completeness is the reason to walk every block.** Leaving out an aggregate is not harmless: if the earliest copy of a
key is dropped, a later copy committing to a different transaction is stored instead, which is exactly the double
nullification §4.2 warns about. So the guest must see every transaction of every block A..H, not Merkle branches to
the ones the host names.

**Finding aggregates** depends on the carrier:
- *In an output script* (OP_RETURN or similar): the non-witness bytes are enough, as in the hints walk.
- *In witness data*: the guest must also take every transaction's witness, compute every wtxid, and check the
  coinbase's witness commitment, because a witness is bound only through it. That roughly doubles the bytes hashed.
- *Under a fixed scriptPubKey* (a spendable marker output): the core's counting trie already gives a script's complete
  history with counts (ZYNCAPPS.md §2), so carriers could be enumerated from the trie with no full walk. The paper wants
  no UTXO-set effect, so I do not expect this; I list it because it changes the cost by orders of magnitude.

### 2.3 Checking aggregates

For each aggregate found, in block order: decode it (a malformed one is skipped, per `spec`), check the half-aggregate
equation over its keys and the fixed message, and if it fails skip it whole (as `node.rs`).

Verifying aggregates one by one is wasteful in a zkVM. The core batches signatures into one multi-scalar
multiplication per chunk (LIBZYNC.md §3; `batch_ecdsa.rs`), and a half-aggregate equation is the same shape:
`s·G = Σ z_i·(R_i + e_i·P_i)`. The guest can batch every aggregate of a run with random weights, with the host saying
which aggregates are invalid: claimed-valid ones go into the batch (a wrong claim fails the batch), each claimed-invalid
one is checked alone and must fail (a wrong claim fails the run). R_i and P_i are lifted from x by checked y hints, as in
the core. ZYNCAPPS.md §7 says a zAPP should reuse the core's libsecp; libsecp 0.5.1 has BIP-340 but no
half-aggregation, so this part is new code either way.

### 2.4 The accumulator update

Two structures, both in the run state:

1. **The ToSAccMT** (A.2): per block, an inner tree over the block's new elements `(pk, R_i, location, publisher)` in
   the pinned order, then one outer leaf `(h − A + 1, v_before, v_inner)`. The outer tree is append-only with no
   removal in the guest (a reorg is a new proof from an earlier receipt, §2.6), so the state is a frontier of O(log n)
   roots, like the coin log and the block forest.
2. **The seen-key set**, which the paper's node keeps as a plain key-value store. A guest cannot hold it, and the
   repeat rule needs non-membership, which the ToSAccMT does not give. So the state carries a sorted or sparse Merkle
   tree keyed by pk: each new key comes with a host-supplied non-membership path and is inserted; a repeat comes with a
   membership path and is counted in `n_repeats`. A wrong path breaks the root. The core's counting trie (`trie_fact`)
   already does absence and insertion on a keyed tree; the zAPP would reuse that shape, not its contents. Its root is
   committed as `seen_root`.

### 2.5 Runs and chaining

As the Utreexo zAPP (UTREEXO_ZAPP.md §4 "Runs"): a run without a predecessor starts from the empty state at A; a later
run verifies the previous run's receipt under `self_id` and takes the state only if its SHA-256 is the previous
`state_out`; the final run checks the core statement (ZYNCAPPS.md §4) and sets `done = 1`. To extend a published
receipt for H to H' > H, the same chaining applies with the previous final receipt as the predecessor, against a core
receipt with T ≥ H'. Runs are serial over heights; their size is set by the prover.

### 2.6 Reorgs

A proof is about the chain ending in `block_hash`. After a reorg below H the receipt is simply not about the client's
chain any more (the client's header check fails); the prover re-runs from a kept receipt below the fork. Keeping
receipts at regular heights bounds that work. Nothing is removed inside a proof.

### 2.7 What a light client verifies

1. The receipt under the published zAPP id; not a FakeReceipt; `RISC0_DEV_MODE` unset; `done = 1`; `spec` one it knows.
2. The core binding as every zAPP (ZYNCAPPS.md §4 "What the zyncAPP's verifier checks"): trusted core image and
   lineage, pinned genesis anchor, `cum_work` ≥ `nMinimumChainWork`, `max_time` ≤ now + 7200.
3. **`block_hash` is its own header chain's block at H** (an SPV header sync, or the header-chain zAPP). The proof
   cannot say which chain is current; the client must.
4. Then v_H is its history list. For a received coin carrying value v_c, the sender (or any server) supplies the
   IsPrefix proof (one outer path under v_H, Fig. 4) and the client checks it.

What it does **not** get: the inner trees. To send, it needs the union membership of its own nullifier, i.e. the
block's elements; a server can serve a block's elements and the client checks them against the outer leaf under v_H,
but asking for one block tells the server which block. Fetching whole blocks' elements (at most 64 bytes a nullifier)
or a private-retrieval scheme is the open part Jonas Nick described; this zAPP does not solve it.

**Freshness.** The proof reaches only H ≤ T, the core's tip. Blocks above T the client would scan itself, or wait.

## 3. What Hazync's full validation adds, honestly

The accumulator is a function of the bytes of blocks A..H only. Nullifiers are arbitrary data to Bitcoin (p. 7); no
Bitcoin rule decides whether one counts. So the same walk bound to proof of work alone, for example to the header-chain
zAPP's receipt (`2db34e12`) instead of the core's block forest, would give the same v_H on any chain whose blocks are
valid. The difference between the two is exactly this:

- **Adds**: the blocks are valid under Core's rules (scripts checked). A PoW-only proof follows the most-work header
  chain even if its blocks are invalid; full Shielded CSV nodes follow the most-work *valid* chain, as Bitcoin nodes do.
  If hash power ever produced an invalid most-work chain, PoW-only light clients would hold a different v than full
  nodes and could accept coins full nodes would not. With the core binding they cannot.
- **Adds, minor**: the block forest is already proven, so the zAPP neither parses PoW nor keeps a header chain of its
  own; and a later zAPP that needs coin facts (for example a peg-out that spends a real coin) has them in the same proof.
- **Adds nothing**: for nullifier signatures (the zAPP's own work, the core does not check them); for data availability
  of the inner trees; for which chain is current; for reorgs; and in practice for as long as no invalid most-work chain
  occurs, which is the usual SPV assumption.
- **Costs**: the core proof must reach H. It does not cover the mainnet chain yet (LIBZYNC.md, top), so a mainnet
  zAPP on the core waits on that; a PoW-only variant could run now. If I built this, I would build the walk once and
  let it bind to either, with the binding recorded in `profile`-like bits so a client sees which it got.

## 4. Cost (ESTIMATES; nothing measured for this zAPP)

### Per nullifier

| part | estimate | from (measured elsewhere) |
|---|---|---|
| signature, one BIP-340 verify each, k256 with risc0's accelerator | ~300 k cycles | statement zAPP prototype, `RESULT_statement.txt` (`feat/zapp-statement`): `signatures` phase 29,877,041 for 100 scripts, P2WPKH and P2TR mixed, k256 0.13.4-risczero.1; it includes per-script digest work, so it is an upper figure for the curve work |
| signature, single verify, risc0-crypto | ~84 k cycles | ECDSA single verify 83,763 (`experimental/libzync-bench/README.md` §1); Schnorr has the same two-scalar shape, not measured |
| signature, libsecp Schnorr in the core chunks before Schnorr batching | ~166 k cycles per input | `BLOCK_962000.md` round 3 (includes the input's other script work) |
| **signature, batched half-aggregate** (§2.3): 2 MSM points + 2 lifted y + 2 short hashes | **~15-25 k cycles** | `msm_raw` 5,984-10,395 cycles per point for 50,000 down to 877 points; y as a checked hint 347; SHA-256 of 32 / 64 bytes 487 / 1,158 (README.md §1-2). Batch ECDSA measured 16,194 per signature at 50,000 |
| seen-key set: non-membership + insert, ~3·log2(N) hashes | ~6-47 k cycles for N = 2^20..2^30 | coin-log bench, 100-525 cycles a hash (`txproofs/README.md` §2) |
| inner-tree leaf (~102-byte element) + amortised parent | ~1-3 k cycles | same hash costs |
| **together, batched** | **~25-75 k cycles** | sum of the rows above; ~300 k+ unbatched with k256 |

### Per block

| part | estimate | from |
|---|---|---|
| walk, early-chain blocks | 13.4-24 k cycles (measured for the hints walk, not this zAPP) | HINTS_ZAPP.md §10: ~13.5 k a block to 41,000, 23,898 averaged over 1..100,000 |
| walk, modern blocks | not measured anywhere in this repo; a SHA-256 floor of ~9-18 cycles per byte hashed gives the order of 10^7 cycles per MB, doubled if the carrier is in witness data (§2.2) | derived from the 64-byte SHA-256 figure above |
| outer leaf + block-forest leaf | ~2-3 k cycles | ~1,600 a block-forest leaf (HINTS_ZAPP.md §10) plus one outer leaf |
| blocks below A | ~1,600 cycles each | the block-forest leaf only |

At the paper's own capacity figure (~100 Shielded CSV transactions a second, §1.1 p. 5, so ~60,000 nullifiers in a
full 10-minute block), the per-nullifier estimate gives of the order of 1.5-4.5 × 10^9 cycles for such a block,
against 440,594,143 measured for the core's proof of block 962,000 (LIBZYNC.md §4). That is a scale, not a forecast:
today no blocks carry Shielded CSV nullifiers, and the real figure depends on usage. I give no total.

## 5. On Bitcoin: Argo and bridges

**Who needs on-chain verification.** A light client does not: it verifies the succinct receipt locally (the core
verifier runs in WASM; the header-chain receipt checks in 39 ms on a laptop, LIBZYNC.md §4). A BitVM-style bridge does:
a peg-out has to show, on Bitcoin, that a burn or withdrawal is valid. That is (a) the Shielded CSV coin proof (a PCD
proof, a different proof system) and (b) that its nullifier is in the canonical chain's accumulator, (b) being this
zAPP's v_H plus a union-membership path, on a chain with enough work. A bridge would most likely want one statement
covering both, e.g. a later zAPP that verifies the PCD proof and the path against v_H; this design is only (b).

**The one-input issue.** Argo garbles a Groth16 verifier over BN254 for `e(A,B)·e(C,H1)·e([x]G,H2) = c^{-1}` with a
single scalar `x` (`src/garble/groth16_verifier/mod.rs`; `[x]G` is one fixed-base table). RISC Zero's Groth16 receipt
has five public inputs: `[a0, a1, c0, c1, id_bn254]`, the control root and claim digest each split in two 128-bit
halves, and the BN254 control id (`risc0-groth16` 3.0.6 `verifier.rs`, `Verifier::new`). Groth16's input term is
`IC_0 + Σ x_i·IC_i`, so inputs that are constants for a RISC Zero version (a0, a1, id_bn254) can be folded into IC_0 by
public computation, leaving the two claim-digest halves. Those cannot become one scalar directly: a SHA-256 digest is
256 bits and BN254's scalar field is 254. Options, without a recommendation:

| option | what it takes | trade-offs |
|---|---|---|
| re-wrap circuit: a circuit whose single public input is a hash of the claim (fits 254 bits), verifying RISC Zero's STARK (a modified copy of its stark-to-Groth16 circuit) | its own Groth16 trusted setup (phase 2), its own audit; the zAPP and core unchanged | one more setup to trust; one more circuit to keep in step with every RISC Zero release; proving cost of a second wrap, not measured |
| extend Argo to two inputs (`[c0]·IC_3 + [c1]·IC_4` with the constants folded) | a second fixed-base table and its input authentication in Argo; RISC Zero's existing setup | larger garbled tables and more on-chain input data (not quantified here); needs Argo's authors; ties the bridge to RISC Zero's ceremony |
| ride a bridge's existing wrapper: a bridge that already proves its own statements with a one-input Groth16 verifies this receipt inside its own zkVM or circuit | nothing new on Bitcoin; depends on a bridge willing to verify RISC Zero receipts | trust and release cadence become the bridge's; verifying a RISC Zero receipt inside another system needs care with control roots (RISC Zero 3.1.0 compression checks a custom root only if it is named in `verify_assumption`) |

The hints zAPP decided against a Groth16 wrap for nodes (HINTS_ZAPP.md §9, Q7); that reasoning is about light clients
and still holds for them. A bridge is the case where a wrap is needed at all.

## 6. Questions for the Shielded CSV authors

0. **Is there a later specification** than ePrint 2025/068, which is the 20.09.2024 text unchanged? Questions 1-4 are
   about what that text leaves open.
1. **Carrier**: where is an aggregate nullifier in a Bitcoin transaction (OP_RETURN, witness, annex), how is it
   recognised, and what is its exact serialisation (R_i encoding, publisher address, several per transaction)?
2. **Start height**: from which block does the canonical accumulator begin, and does every block (empty ones too)
   append a set?
3. **Order and repeats**: which copy of a key wins inside one block; is a malformed or invalid aggregate skipped whole;
   is a repeat key in a valid aggregate ignored as in `node.rs`?
4. **Hashes and trees**: HSig (BIP-340's challenge?), HSigComm, the half-aggregation coefficients (the half-aggregation
   BIP draft?), the Merkle trees and the encodings of elements and `(i, v0, v1)`, and the location format (21 + 22 bits
   or 24 + 24).
5. **Light client**: is "v_H at a proven H plus IsPrefix proofs from senders" what you had in mind for receiving, and is
   a full-validation binding wanted, or is proof of work enough for you (§3)?
6. **Sending**: would per-block element files checked against v_H be acceptable, given what fetching one block reveals?
7. **Bridge**: would a BitVM bridge want this statement on its own, or only combined with the PCD coin proof, and in
   which proof system?
