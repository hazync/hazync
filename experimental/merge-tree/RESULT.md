# Would a wider merge tree make Hazync v2 cheaper?

Question: the whole-chain cycle split is chunks 67.5%, trie 17.0%, aggregate 15.6%. If more receipts were merged per
step (a k-ary join tree, or several blocks per aggregate), how many cycles per block would merging cost, against today?

Image: origin/main `cac5a2e` (core image 46666b6f) **plus an experimental patch** (`merge-tree.patch`: guest mode 19, a
k-ary join; mode 20, an empty session; host `mt-join`). The shipped guest is not changed; the patched image has its own
id. All cycle counts are risc0 **executor** counts (no proving), on `hazync-coord`.

## 1. What a merge step costs, from the code

There are two merge steps in v2 (`prover/methods/guest/src/main.rs`):

| step | merges | guest work per merged receipt | guest work per session | per block of data |
|---|---|---|---|---|
| **aggregate** (mode 5) | 16 chunk receipts (2 on the board) + the trie receipt + the previous chain proof (not on a leaf) | `env::verify(self_id, journal)`: hashes the claim (SHA-256 of the journal, accelerated) and records an assumption; then the journal is consumed by `validate_block_lite` | startup (`__libc_init_array`, the allocator check, `assert_core_constants`), decoding the previous `ChainState`, header rules, forest append, commit | ALL the rest: merkle/witness commitment, BIP30/34/68, every input covered once, coin-log apply, ECMH, undo hash, trie-list check |
| **join** (mode 18, backfill) | 2 chain proofs | `env::verify` + serde decode of a `ChainState` + 4 field checks; `link_digest` (re-serialise + SHA-256d) on the left | startup, commit | none |

What the executor does **not** count: every `env::verify` is only *recorded* in execution. It is discharged at proving
time by risc0's recursion programs: each segment is **lifted**, the segment receipts are **joined** into one, and each
assumption is **resolved** (one recursion proof per merged receipt), then the result is made succinct. The project's own
code says so (`prover/host/src/lz.rs`, `report_agg_execute_only`: "In execute mode `env::verify` merely RECORDS an
assumption; RESOLVING it is recursion, and that cost lands in PROVING"). This project has always counted those costs as
GPU **wall time**, never as cycles:

- **resolve: 0.28 s each, linear in the count** (`docs/history/TEN_MINUTE_BLOCK.md` §(b), block 741,000, 4 and 16 chunks;
  v1 aggregate, 2026-08). In v2 the aggregate's tail after the last chunk is **~3.5 s** for 16 chunks + trie
  (`experimental/libzync-bench/gpu/GPU_RESULTS.md`, 969,122, 4x RTX PRO 6000).
- **join: 6.4 s each on one RTX PRO 6000**, one 2^20 segment (`GPU_RESULTS.md` top, c14, 7 real joins; the time is the
  prove call: execute + segment proof + lift + 2 resolves + succinct).

So a merge step has three parts, and only the first is in any executor count:
(a) guest cycles per merged receipt, (b) guest cycles per session (startup, padding to a power of two),
(c) recursion per merged receipt (resolve) and per segment (lift, segment join).

TBD sections 2-5.
