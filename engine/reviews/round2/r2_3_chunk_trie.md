# R2-3: what binds chunk and trie proofs to the block (reviewer report, 27.11.2026, as given)

No way found to get an invalid block accepted through the split: choosing boundaries, reusing chunks or trie parts from
another block or split, duplicating or dropping a chunk, or feeding a chunk prevouts the coin log does not hold; each is
refused in the guest or cannot be proved. High confidence for the shipped image (features libzync, agg-lite, coinlog,
trie; txlog off). Read only. The shipped image always takes the agg-lite path: the binding is the KIND_CHUNK_LITE record
journal (agg_lite.rs 26-31); ChunkOut and validate_block_wtx are compiled out.

1. LOW (real; impractical). The trie's coin key is a truncated XOR: agg_lite.rs:113 okey = txid with vout XORed into
   bytes 0..4; two outpoints collide when their txids agree on 224 bits (~2^112 work). A collision makes the trie mark /
   insert the wrong key: a refused block (Duplicate / Missing) or trie balances drifting from the coin log. Block validity
   unaffected: the coin log and the in-block match use the full leaf (verify_input.cpp:89-127).
2. LOW (medium; read, not differentially tested). Trie keys and undo digests come from Rust parsers (tx_io agg_lite.rs:65,
   prevout_fields :93) separate from Core's C++ parse of the coin leaf. Compared: canonical CompactSize, MAX_SIZE, the
   witness marker, trailing bytes match Core. A future divergence would desync the trie / undo hash from the coin log,
   not make an invalid block valid.
3. INFO. The txlog "speculative chunk" path is off in the shipped image (agg_lite.rs:547-549 accepts chunks with a zero
   block hash and time, safe only because the CSV height and the flags with a zero hash must match; rec_order :566-578
   lets records go unread, but they cannot repeat and the Merkle root binds the txid list). Not in default features
   (engine/guest/Cargo.toml.in:16,37): re-review before it is ever turned on.
4. INFO. The same-image guarantee rests on OpenVM's deferral (env::verify, engine/platform/risc0-zkvm/src/lib.rs:156-177;
   self_id tied to METHOD_ID via prev.self_id main.rs:428, the join :2567, the final verifier).

Checked and sound: context binding (tag KIND_CHUNK_LITE, all_valid == 1, height, block hash, MTP, time equal the
aggregate's, agg_lite.rs:542-552; height = prev.height + 1, MTP from the verified previous state; flags from height and
hash); a chunk from another block (block-hash check; its txids feed the Merkle root checked against the header
:826-827, mutation check, distinct txids :842); coverage, no gap or overlap (runs contiguous inside n_vin :257-262, an
empty run fails all_valid; later runs of a tx carry the same txid, wtxid, prevouts hash and start where the last ended
:619; counts non-zero within n_vin :624; the open tx fully covered before the next and at the end :586, 654; a duplicated
chunk repeats a txid or mismatches a start; an omitted one breaks coverage or the Merkle root); boundaries irrelevant to
soundness (tracked txs forced into one run :253 with :587/656); prevouts vs the coin log (every run hashes the full
prevouts blob, all runs of a tx agree; a leaf commits outpoint from the tx bytes, amount, script, coin height, coinbase
and tracked byte, creation MTP verify_input.cpp:94-127; external leaves unspent in the coin log at strictly increasing
positions with matching old roots coinlog_forest.rs:120-150, position count agg_lite.rs:742; starting roots ==
prev.utxo_roots main.rs:459; in-block spends match an earlier tx's created leaf, at most once :688-689; an external coin
claimed at this block's height refused :697, an external coin spent twice refused :696/708; so fee, sigops, maturity
core_tx_rules.cpp:109 and BIP68 run on bound coin data; BIP68 split across runs equivalent to Core's); the trie (single
receipt: tag KIND_TRIE, length 25, old root == the chain's, update-list digest == the aggregate's own list_digest
:806-815; combine names this image main.rs:391-396; counts mode enforced; parts each verified, KIND_TRIE_PART, used
exactly once at a slot of matching depth and prefix, digests cover keys, spenders, values ttree.rs:435-447, 500-512,
552-570; the aggregate runs the combine on its own lists; tags distinct, mode 10 needs ~3.3 billion cases); signatures
(batch collect mode returns 0 from hazync_batch_defer: libsecp verifies each fully, batch_ecdsa_ovm.rs:55-58).

Not reviewed in depth: ttree top_pass / sub_pass internals, the coin log's proof of tree shape, the BIP30 SMT, OpenVM's
deferral circuit.
