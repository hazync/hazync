# R2-2: block (aggregate) and join statement logic (reviewer report, 27.11.2026, as given)

No way found to get a committed ChainState that is not a valid chain from the pinned anchor. Read only. Every
host-supplied value is checked against the verified previous state, recomputed in the guest, or covered by link_digest.

- L1 LOW (by design; high). A base step accepts ANY anchor state unverified; everything rests on the reader pinning
  anchor_id. main.rs:2287 (anchor_id = link_digest(&prev) when is_base == 1), :2382 (lineage = prev.lineage). With
  is_base = 1, prev comes from input: self_id, profile, lineage, cum_work, height, both forests, smt_root, trie_root,
  live_hash taken as given; only kind, version, block_leaves == height + 1 and a well-formed ECMH point checked
  (swiftsync.rs:117). Scenario: base-step on a made-up state (height 900,000, huge cum_work, easy nbits): passes every
  guest check; a join keeps the made-up origin (copies l.anchor_id, never checks it). Stopped only by the reader:
  swiftsync-verify/src/lib.rs:551-556 (genesis anchor pinned, or a supplied anchor -> at most Demo); the zAPP guests also
  leave the anchor to the final verifier (prover/zyncapps/methods/guest/src/main.rs:107). link_digest (main.rs:644)
  covers every field but anchor_id, so a pinned anchor fixes all of them. Any NEW consumer that skips the anchor check
  accepts invented chains.
- L2 INFO (high). The handover path is unreachable (every lineage/*.rs has LINEAGE = &[], so is_base == 2 hits the
  assert at main.rs:2262; the stub cross-version verifier panics). Once a lineage is pinned, a join over a handover fails
  (l.lineage == r.lineage, and the handover step carries the predecessor's anchor_id, not a link digest): liveness.
- L3 LOW (not followed into coinlog_forest; low). main.rs:2302 compares normalize(w.root_prev.roots) with
  normalize(prev.utxo_roots), then agg_lite.rs:663 applies the coin log to the HOST's w.root_prev.roots, not
  prev.utxo_roots: the host may add trailing None rows; harmless if coinlog_forest::apply places roots by num_leaves
  alone. Worth one look, or apply to prev.utxo_roots.
- I1 (trust assumption). Every parent-child binding depends on OpenVM's deferral circuit: env::verify
  (engine/platform/risc0-zkvm/src/lib.rs:156-176) takes (exe, vm, public values) from verify_stark_unchecked, checked
  only by the deferral circuit. Around it: commitments used in call order, finalize requires all used; a dry run cannot
  finalize; a collect run panics before committing; bench-noverify has its own id and tagged public values.

Checked and sound: previous-state binding (is_base == 0: journal verified under self_id main.rs:2259, kind/version;
prev.self_id == self_id and prev.profile == chain_profile() :2273-2276; the reader checks the final self_id and the
proof's image, lib.rs:529, so every level runs this image); height (w.height == prev.height + 1 :2284; the chunk context
height, hash, MTP, time must match exactly agg_lite.rs:530; the txlog speculative path off, Cargo.toml.in:16); tip hash
and order (prevhash_ok; tip_hash recomputed; one forest leaf per height main.rs:2361-2370; the reader re-checks shape
lib.rs:540); cum_work (add_work on the header's nBits, which Core's ContextualCheckBlockHeader checks,
core_block_rules.cpp:133; check_pow; epoch_start and the 11-entry recent_times derived; Core's MTP routine); max_time
(max(prev, block) then the reader's clock); the checkpoint enforced in the guest (:2357); coin forest, SMT, trie, live
hash (carry_ok; in_smt_root == prev.smt_root; the trie receipt's old root == prev.trie_root, its update-list digest
recomputed from the block; trie parts and combine bound to self_id; live hash from prev.live_hash; utxo_value checked
subtraction; a collect run never committed); domain separation (chunk, trie, trie part, chain state, test-mode
journals have distinct first words, none steerable to KIND_CHAIN / KIND_CHUNK_LITE; mode 10's first word a vector
length, ~3.3 billion entries); join (main.rs:2555: both children under self_id, same kind, version, self_id, profile,
equal lineage; adjacency link_digest(l) == r.anchor_id over all of l's state incl. block_roots; a self-join needs a
hash fixed point; anchor_id from the left child is right: the only field link_digest leaves out, no step reads it);
modes outside libzync (1, 2, 6, 7, 12) refused by the assert at :2422, libzync a default feature of the engine build.

## L3 checked (27.11.2026, me)

libzync/src/coinlog_forest.rs `apply` (:120): shape_ok requires a root exactly at each set bit of n and len <= 64;
spends walk the levels skipping None; an append takes the first empty level (an existing trailing None or a pushed
slot, the same position), and trailing None rows are popped at the end. So the result does not depend on trailing
None rows in the host's w.root_prev.roots: no issue.
