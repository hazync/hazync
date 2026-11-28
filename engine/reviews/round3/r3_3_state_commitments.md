# Round 3, review 3: state commitments (coin log, counting trie, BIP30 tree)

Reviewer: independent, read-only. Tree reviewed: `r3tree` at a9b78510. Nothing was built or run. Every conclusion
below comes from reading the code.

Scope: `libzync/src/coinlog.rs`, `libzync/src/coinlog_forest.rs`, `libzync/src/ttree.rs`, `libzync/src/trie_fact.rs`
(+ `trie_model::decode`), `libzync/src/facts.rs` (`member`), `coinbase-smt/src/{bip30.rs,roots.rs}`,
`prover/methods/guest/src/{agg_lite.rs,main.rs (aggregate, trie modes 17/19/20, join),swiftsync.rs,utreexo.rs}`,
the leaf code in `prover/methods/guest/verify_input.cpp`, `engine/guest/{coinlog_ovm.py,edits.py}`,
`engine/platform/risc0-zkvm/src/lib.rs` (env::verify / finalize), and the specs COINLOG.md, ZYNCAPPS.md §2, UNDO.md,
node_format2/RESULTS.md.

## Verdict

**I found no Critical, High or Medium soundness finding.** For each of the questions (a) to (e), I could not build a
witness or hint that makes the guest accept a false state transition. All findings below are Low or Informational:
liveness problems, stale docs, and assumptions I could not check from this tree. Confidence that (a) to (e) hold
against the code as written: **high** for the coin log and the BIP30 tree, **high** for the trie update and its
part/combine binding, and **medium-high** for the engine build. The medium part is because the OpenVM SHA-256
intrinsic and the semantics of deferred verification come from outside this tree (see "Not verified").

## Why each attack fails (evidence)

### (a) Spending a coin that does not exist or is already spent
- Each input's leaf is computed **in the chunk** by Core-side C++ (`verify_input.cpp:94-133`, called from
  `verify_inputs_batch[_once]` at `:1174/:1242` after `spent.size()==tx.vin.size()`). It is computed from the
  outpoint in the tx bytes and from prevout `i`'s value and script, with the prover's `ch/cb/cmtp`. The chunk refuses
  `ch>height` and `cb>3` (`agg_lite.rs:278-280`). Every field that maturity and BIP68 read is therefore inside the leaf.
- In the aggregate, a spend is in-block exactly when its leaf equals a leaf created in this block (`agg_lite.rs:682-694`):
  created by a strictly earlier tx (`:688`), spent at most once (`:689`). Every other spend is external: refused if
  it claims this height (`:697`), refused if the same external leaf appears twice (`:696,:708`), and opened in the log
  at a prover-named position (`:741-756`).
- `coinlog_forest::apply` (`coinlog_forest.rs:120-175`): positions must be strictly increasing (`:124`) and `< n`
  (`:127`). Each touched tree's old root must equal the carried root (`:142`), and the witness must be used exactly
  (`:150`). The positions alone fix every path: the split at `mid` (`:41-42`), the offset bits in `update_one`
  (`:60-71`), and level `i+1` IVs (`:102-106`). A spent leaf is `compress(IV_SPENT, c‖s)` and an unspent leaf is a
  full tagged SHA-256, so a flipped leaf cannot be opened as unspent, and no coin leaf can open where another sits.
- The carried roots and count equal the verified previous state (`main.rs:2318`, asserted at the `assert!` after it).
  `shape_ok` forces one root per set bit of n (`coinlog_forest.rs:114-116`).
- A permuted position list only pairs a leaf with the wrong slot, which ends in `OldRoot`.

### (b) Creating or omitting a coin, or changing its value/script/height/flags
- Appends are exactly `output_leaves` minus those spent in-block (`agg_lite.rs:719-725`). `output_leaves` is the
  coinbase's spendable outputs, which the aggregate parses itself (`:470-506`), plus each tx's first record, all
  computed by `tx_out_leaves_mv` (`verify_input.cpp:355-400`, every spendable vout, in vout order). The Merkle root
  over the walked txids (`agg_lite.rs:826-827`), together with `ids_distinct` (`:842-846`), binds the walk to the
  block's exact tx list and order. Coverage (`:586,:624,:654`) forces exactly one input-0 record per tx.
- Height = the block's height. MTP = the chunk's MTP, required equal to the verified `prev_mtp` (`:549`). The tracked
  bit is derived either from the inputs' leaf-bound flags (`:253`, cross-checked as `tr_all == tr_claim` at
  `:587,:656`) or, for the coinbase, from its bytes and an SMT-bound `coinbase_prior` (`:477-480`, `:847-852`).

### (c) Roots, leaf count or live value diverging
- `utxo_leaves` counts appends (`coinlog_forest.rs:153-173`).
- `utxo_value = prev + survivors − (Σ_tx(fee+total_out) − in_block_spent)` (`agg_lite.rs:593,:693,:819`,
  `main.rs:2365`, checked subtraction). Fee and total_out come from Core's `CheckTxInputs` over the prevouts blob. All
  runs of a tx must agree on that blob's hash (`:619`), and every prevout in it is leaf-bound (log or in-block). So
  the subtraction is exactly the external spends' value.
- `live_hash` (ECMH) adds and subtracts the same leaves (`:731-735`). Its try-and-increment map has one output per
  leaf: non-residue witnesses, even-y, and `y<p` (`swiftsync.rs:65-96`). The base-step carry is curve-checked
  (`:117-130`).

### (d) A BIP30 duplicate passing
- The SMT is fixed at depth 256 and domain-tagged (`roots.rs:36-59`). `Some(0)` never occurs: the priors allowed are
  `None|REPEATABLE`, spends must be `is_live`, and `outputs==0` is refused (`bip30.rs:170,187,197,206`).
- Order: coinbase check, then tracked inserts, then decrements (`bip30.rs:162-211`), so a block cannot spend a
  duplicate to zero and then claim the slot. `in_smt_root == prev.smt_root` (`main.rs`, `smt_ok`).
- The guest's lists must equal the witness's (`agg_lite.rs:847-852`): the coinbase txid and count, each tracked
  spend's txid and flag, each tracked new tx. Untracked coinbases are never decremented, so their slots stay live
  forever, and a repeat of one is always refused (stricter than Core; see L3).
- The argument for tracking only inputs that are all tracked (UTREEXO_FORMAT §2a) holds by induction over leaf-bound
  flags.

### (e) Desyncing the counting trie
- **Counts mode is forced** in every path: mode 17 (`main.rs:2473`), mode 20 (`:2513`), mode 19 (`ttree.rs:523`),
  and the aggregate's parts path (`agg_lite.rs:795`). The old root must be the chain's (`agg_lite.rs:795,808`; the
  computed old root is checked again at `ttree.rs:510`). The update list must be the aggregate's own: `list_digest`,
  with fixed-size records and counts (`ttree.rs:677-692`, `agg_lite.rs:804-811`).
- `make_ups` with a supplied order is a true permutation with strictly increasing (S,O) (`ttree.rs:471-481`), which
  refuses a repeated key.
- **Shape:** inner, coin and address nodes are domain-separated (IV 50 / 51 / 53+54). Every update path must be
  expanded: tag 1 is allowed only with no updates under it (`ttree.rs:303,395`). One-sided chains check that the
  smallest and largest update fall on the expanded side (`:208-212,:259-262`). An inner node at depth 256 is refused
  (`:306,:398`).
- A mark must land on an unspent coin with an equal O (`:332-341`). Otherwise it reaches `build_sub`/`build_top`,
  where `cx.ins(mark)` is `None`, which is `Missing` (`:281,:347`).
- Counts are `count − mc + pc` with checked arithmetic, values bound by the leaf hash (`:425-428`).
- **Part/combine binding:** `TAG_PART` takes the next part journal (`ttree.rs:438-440`), which must have this depth,
  the prefix of the first and last update (`:441-443`), and `ups_digest` of exactly the updates routed to this node
  (`:444`; injective: count, then 64-byte key, a type byte and a fixed tail, `:552-570`). Every part is used, in order
  (`:509`). Parts are verified as proofs of `self_id` (`main.rs:2246`, `:2508-2512`), and a combine must name the
  aggregate's own id (`main.rs:2250-2254`). A part cannot contain `TAG_PART` (`cx.parts` is empty, `:523`). A part's
  subtree hash is accepted only through the combine's root equality, so a part for the wrong subtree or the wrong
  depth cannot fold to the old root. The prefix check is redundant given the digest, but harmless.
- Reader side: `trie_model::decode` is one-to-one (`trie_model.rs:243-302`). `find_addr`/`coin` reach ABSENT only at
  an empty slot or a leaf with a different full key (`trie_fact.rs:77-115`). This is sound because the guest only ever
  places leaves on their own key path (`build_top`/`build_sub` split on the key's bits) and never removes a node.
- The trie key set covers the coin log: both use the same `output_leaves` filter, and the trie also keeps in-block
  spends, so every external spend has a trie coin.

### Join, engine glue
- `join` (`main.rs:2555-2577`) needs `link_digest(left) == right.anchor_id`. That digest covers every ChainState
  field except anchor_id (`main.rs:644-650`), including utxo_roots, utxo_value, trie_root, smt_root, block forest and
  live set.
- Engine `env::verify` checks the image (exe+vm commit) and `sha256(journal)` (`engine/platform/risc0-zkvm/src/lib.rs:156-178`).
  A dry run cannot finalize (`:212`), and every supplied proof must be consumed (`:214`). `exit()` panics before any
  reveal (`:203-206`), so a collect run cannot yield public values.
- `coinlog_ovm.py` keeps the hash definition: the IV comes in as native state words, the block is `[L,R]` as raw
  bytes, and the output is the state words. It also introduces no aliasing: each `compress_blk` reads one pair and
  writes a different one or a local.

## Findings (ranked)

### L1. A tracked transaction split across chunk runs makes its block unprovable (liveness, Low; confidence high)
`agg_lite.rs:253`: `tracked_tx` is true only if this run holds **all** inputs (`cb.len() == n_vin_all`). If the host
splits a transaction whose inputs are all tracked, the input-0 run claims "untracked" and the aggregate refuses
(`:587,:656`). The host is documented to keep such transactions whole. This is not a soundness problem: an honest but
mis-partitioned host stalls, and a malicious one gains nothing. Fix (optional): document it as a host invariant with
a test, or let the claim be "unknown" and derive it in the aggregate.

### L2. `okey` is not injective: a crafted transaction can make an honest block unprovable in theory (Low; confidence high)
The coin key is the txid with vout XORed into its first 4 bytes (`agg_lite.rs:113-117`). If an insert and a mark
(or two inserts) share (S, okey), the update fails with `Duplicate` (`ttree.rs:480,494`). Building such a pair needs
two txids that agree in 224 bits. That is out of reach, and the failure is a refusal, never a wrong root. It is
documented in ZYNCAPPS §2. Informational.

### L3. BIP30 is stricter than Core for untracked pre-BIP34 coinbases (consensus divergence on non-mainnet chains; Low)
`agg_lite.rs:638` sends a spend to the SMT only if the coin is tracked, so an untracked coinbase's slot is never
decremented and a later byte-identical repeat is refused (`bip30.rs:170`), even after every output was spent.
Core would accept it. On mainnet that is unreachable (BIP34; the two grandfathered pairs are handled), and
`bip30.rs:30-32` documents it. On a custom chain the guest can reject a valid chain. It cannot accept an invalid one.
Informational.

### L4. Leniency in the guest's trie witness reader (Informational)
`ttree.rs:322-328`: a coin leaf's flag `f != 1` is read as unspent, and unbound spender words are ignored. This is
harmless, because the guest commits only roots and the list digest, and the reader was made canonical (11-09,
`trie_model.rs:282-302`). The difference must stay: any future path that commits or forwards **witness bytes** from
the guest would bring the 11-09 malleability back.

### L5. Stale spec text (Informational)
- `docs/libzync/COINLOG.md:52-57` gives the test vectors `cdcc3dda…` / `fcd6a122…` and the 1..200 chain vector roots
  (`70d43a0c…`). These are **format-1** values. The code's test now asserts format 2 (`da3dcccd…`, `22a1d6fc…`,
  `coinlog_forest.rs:503-505`). An independent implementer following §2 (format 2 by default) would fail against the
  printed vectors. The same applies to the chain vector, which is from `handover/retest_c4.out`, a format-1 run.
- `agg_lite.rs:18` says "The coins themselves are bound by the SwiftSync total, as before". They are bound by the coin
  log (design B was retired). Misleading for a reviewer, with no code effect.

### L6. Shared IVs between the coin log and the trie (Informational)
The trie uses `IV_NODE[50,51,53,54]` (`ttree.rs:15-19`). A coin-log node at level 50 or more would hash with the
same function. The log cannot reach 2^50 leaves, and readers check each structure against its own root
(`utxo_roots` against `trie_root`), so nothing is confusable today. Any future structure that shares a root with the
log, or a log that is never pruned past ~10^15 coins, should use separate tags.

## Not verified (outside this tree, or not read in depth)
1. **OpenVM's `zkvm_sha256_impl(state, input, output)`**: that it is one SHA-256 compression with the state as native
   little-endian u32 words. I rely on the cross-check in node_format2/RESULTS.md (identical chunk and trie journals,
   and an aggregate differing only in the image id, on 976,379) and on the host recomputing roots every block. A
   one-block equality would not catch a level-dependent or IV-specific divergence that block 976,379 does not touch.
   IV_NODE is used at every level the log reaches, so the exposure is small.
2. **`openvm_verify_stark_guest::verify_stark_unchecked`** (engine `env::verify`): whether deferred verification is
   enforced by the outer proof, and whether an inner proof of a non-zero or panicking exit could ever carry revealed
   public values. The shim reveals only in `finalize`, after every assert, which mitigates the second point.
3. The C++ consensus helpers I read only at their call sites: `hz_tx_rules_c` (the fee over all prevouts, which
   (c) depends on), `hz_indicated_height` (its BIP34 prefix parse, which (d) depends on), `NET.is_bip30_unspendable`,
   `block_script_flags`, `tx_vin_count`, and the `cached_tx`/`cached_spent` cache keying (round 11 territory).
4. The equivalence of Rust's `tx_io`/`prevout_fields` and Core's deserializer. I checked it by reasoning (strict
   CompactSize, the witness-marker cases, trailing bytes ignored by both), not by differential testing. A divergence
   would desync the **trie** key (a mark or insert under the wrong outpoint or script) while the coin log stays right.
   This is the most consequential assumption behind (e), and I would like a differential fuzz of these two parsers
   against Core's `UnserializeTransaction` / `vector<CTxOut>`.
5. Host-side code (`coinlog::Store`/`DiskNodes`, `history.rs`, `zync-path`, `tracked.rs`). It can only cause
   liveness failures or wrong served facts, which readers then refuse.
6. The zAPP readers beyond `facts::member` and `trie_fact`: liveness, statement, deposit, assume.
7. Engine edit 10 (`sha2::short_digest_limbs` in `swiftsync::x_for`), which affects `live_hash` only.
