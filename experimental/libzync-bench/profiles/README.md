# Cycle profiles of block 962,000: Hazync v1 vs v2, from release binaries

Execute-mode `RISC0_PPROF_OUT` profiles of mainnet block 962,000, made from the **published** Linux release hosts and
the repository's own block fixture. Anyone can re-make them with `run.sh` (no GPU, ~1 GB RAM, ~15 min on a laptop).
Results and the answers about the paper's v1 figures: [`RESULT.txt`](RESULT.txt).

| file | guest | what |
|---|---|---|
| `v1_block962000_chunk.pb` | v1 CORE, image `37987b85` (v0.22.1) | whole block as ONE chunk (mode 4) |
| `v2_block962000_chunk.pb` | v2 libzync, image `46666b6f` (v0.27.0) | whole block as ONE chunk (mode 4) |
| `v2_block962000_trie.pb` | same | the block's trie update (mode 17) |
| `v2_block962000_aggregate.pb` | same | the block's aggregate (mode 5) |
| `prof_split.py` | | decoder: categories, paging apart, top self functions, inclusive libsecp share |
| `capture_pprof.py` | | keeps every profile a multi-execution host command writes |
| `run.sh` | | the whole reproduction, download to decoded tables |
| `logs/*.out` | | the host output of the runs these files come from (paths replaced by `$OUT` / `$REPO`) |

## Binaries and inputs

| | sha256 |
|---|---|
| v0.22.1 `hazync-host-x86_64-linux-gnu` (241,405,064 B, tag commit `484f40f1`) | `68dd590d7f97a1e539158ca44ee214afd4aa216f9a3779ace1eb9714a717ac50` |
| v0.27.0 `hazync-host-x86_64-linux-gnu` (224,140,328 B, tag commit `fbf9454a`) | `4c79a0f144e06b4d1d66b3af775c372687ffa562c2c06ed4bff97318de45e82b` |
| `prover/block_962000.json` (git blob `23dba40c`, identical at v0.22.1, v0.27.0 and here) | `0e3349832b5d1e184b5d2cc594fba010b2b183f7de46bfa760c748e61971e1c4` |
| `experimental/libzync-bench/cost/txcounts.txt` (v2 coin-log model input) | `e29f03f010b6edb5a74f6e55e3e7cd1e8ff4c51518c11b3f389b1d6124ae756c` |
| v2 hint bundle `v2_block962000_c1.lzb` (made by `libzync-hints`, not committed; 774,540 B) | `7e84919dbcf8983fc185b4b91590d5cb54e84b62d1106cc2bb7b5ebf2fd54514` |

Both hosts were checked against their release's `SHA256SUMS.txt`, whose `.asc` gave a Good signature from
`777FE81F 8CC077FD 3D08055E 852C2B31 90F5B928` (SECURITY.md). `hazync-host method-id` prints
`37987b85ec665970…` and `46666b6f7e3d1c1c…` respectively.

## Commands

`$REPO` is this repository, `$OUT` a scratch directory. `run.sh $OUT` does all of this.

```bash
# v1 (v0.22.1). HAZYNC_LIFTX_HINT=1 is required at run time (the release guest is built with liftx-hint);
# HAZYNC_ECMULT_WINDOW is build-time only and changes nothing here (kept: it is the recorded recipe).
HAZYNC_LIFTX_HINT=1 HAZYNC_ECMULT_WINDOW=21 HAZYNC_BLOCK=$REPO/prover/block_962000.json HAZYNC_CHUNKS=1 \
HAZYNC_PROFILE_EXEC=1 RISC0_PPROF_OUT=$OUT/v1_block962000_chunk.pb \
  $OUT/v0.22.1/hazync-host-x86_64-linux-gnu chunk-profile

# v2 (v0.27.0): hints for the same chunking, then the chunk, then chunk + trie + aggregate
export HAZYNC_BLOCK=$REPO/prover/block_962000.json HAZYNC_CHUNKS=1 HAZYNC_PROFILE_LAYOUT=cost \
       HAZYNC_COINLOG_TXCOUNTS=$REPO/experimental/libzync-bench/cost/txcounts.txt
HAZYNC_HINT_THREADS=1 $OUT/v0.27.0/hazync-host-x86_64-linux-gnu libzync-hints $OUT/v2_block962000_c1.lzb
export HAZYNC_LZ_HINTS=$OUT/v2_block962000_c1.lzb
HAZYNC_PROFILE_EXEC=1 RISC0_PPROF_OUT=$OUT/v2_block962000_chunk.pb $OUT/v0.27.0/hazync-host-x86_64-linux-gnu chunk-profile
RISC0_PPROF_OUT=$OUT/cap/live.pb python3 -I capture_pprof.py $OUT/cap/live.pb $OUT/cap/v2_agg -- \
  $OUT/v0.27.0/hazync-host-x86_64-linux-gnu agg-exec-fake
#   -> v2_agg.0 chunk, .1 coin-log collect run (host-side, never proved: not used), .2 trie update, .3 aggregate

# decode
python3 -I prof_split.py v1_block962000_chunk.pb
python3 -I prof_split.py v2_block962000_chunk.pb v2_block962000_trie.pb v2_block962000_aggregate.pb
python3 -I prof_split.py --sum "v2 block 962,000" v2_block962000_chunk.pb v2_block962000_trie.pb v2_block962000_aggregate.pb
python3 -I prof_split.py --leaves 12 <file>   # audit: the largest functions in every category
```

Measured: v1 10 min 58 s wall, 540 MB peak RSS (the chunk executes twice, see below); v2 hints 61 s / 650 MB,
chunk 39 s / 548 MB, `agg-exec-fake` 2 min 05 s / 644 MB.

## Checks

- **v1 reproduces the shipped guest's numbers exactly**: 3,357,576,338 cycles and journal
  `4fb3e3c5…4656d`, `all_valid=1 binds=8006` (the oracle digest in `docs/BUILDS.md`); `CPubKey::Verify`
  cumulative 2,824.3 M of 3,677.9 M, as `BLOCK_962000.md` has it.
- **Each profile's non-paging cycles equal the host's printed count** to within 28 cycles (chunk, trie and
  aggregate alike), so every captured file is the execution it is named after.
- **Deterministic.** v1's `chunk-profile` executes the one chunk twice (v0.22.1 has no `HAZYNC_PROFILE_LAYOUT`, so
  both packings run; with one chunk they are the same); v2's chunk ran in `chunk-profile` and again inside
  `agg-exec-fake`. In both pairs the decoded tables are identical line for line. The `.pb` BYTES differ between
  runs (pprof id ordering), so compare decoded output, not file hashes. A full second `run.sh` gave identical
  decoded output for all four profiles (see RESULT.txt, end).

## What is modelled in v2

The chunk is entirely the real block (transactions, prevouts, signatures). The **trie update and the aggregate run
against a modelled prior state**, as every single-block libzync measurement does (`LIBZYNC.md` §4): `agg-exec-fake`
builds a coin log of the modelled size around the block's own spends (`lz::cl_synth`: size from `txcounts.txt`,
k = 1.64, seeded positions) and synthesises the old trie around the block's keys (`trie_model::synth`, seed
`0x5eed + 1`). It is deterministic, so the cycles reproduce, but the coin log and trie are not mainnet's state at
height 961,999; their cycles (trie 47.6 M, the aggregate's state work) are a model's.

## Why one chunk

v1's recorded profile was the whole block as one chunk; v2 is profiled the same way so the two are like for like.
Proving uses 16 chunks: c14's 16-chunk figure for this block is 375.7 M chunk + 64.9 M aggregate
(`handover/RESULT_c14.txt`); one chunk here is 352.1 M + 66.4 M, the difference being per-chunk fixed costs and
batch size.

## The decoder (`prof_split.py`)

It reuses `../cost/core_share.py`'s protobuf reader and its runtime rule (a sample is charged to its innermost
frame that is not memcpy/malloc/paging/syscall/Rust core), then classifies that frame by its **demangled** name
(binutils `c++filt`; without it, mangled names are matched and argument types leak in). Changes from the 28.10
`cat_prof.py` keyword lists, each found by auditing `--leaves` on these profiles:

- **Paging is its own line**, not spread over callers; both readings are printed.
- **Script is matched first**, with Core's interpreter functions named (`VerifyScript`, `VerifyWitnessProgram`,
  `ExecuteWitnessScript`, `EvalChecksig`, encoding checks, `CScript::` methods, `popstack`, …). `cat_prof.py`'s
  `Verify` filed `VerifyScript` / `VerifyWitnessProgram` under signatures, and `CScript` matched mangled argument
  types (e.g. `CheckECDSASignature(…, CScript const&)`).
- **Signatures** add libsecp's field-backend helpers `hz_add`, `hz_neg`, `hz_canon`, `hz_fold`, `hz_ge_p`,
  `hz_is_zero_mod`, `hz_sub_p` (`prover/methods/guest/field_bigint2_impl.h`, both releases; 9.6% of v1, filed under
  'other' by `cat_prof.py`), Core's signature-checker wrappers, and v2's `batch_ecdsa` / `hazync_batch_*`.
  The generic `Verify` / `scalar` / `Write` keywords are gone.
- **Hashing** adds `SHA256D64` / `hz_d64`; a sighash (`SignatureHash`) counts as hashing.
- **State** adds v2's `ttree` (the trie) and `coinlog_forest`; `prevector` moved from script to tx parse (it is
  CScript storage, copied during deserialisation).
- **`guest main / I/O`** is a bare `method[<hash>]` frame: Rust inlined into the guest's main. In v1 it is mostly
  the journal commit through risc0 serde (45.4 M); in v2's aggregate, the claim hashing of `env::verify`.
  `core_share.py` charges it to libzync; here it is kept apart because no name says what it is.

Categories remain name-based: code inlined into a function counts as that function. The inclusive line ("libsecp256k1
and its field backend on the stack") is name-independent of the category lists and is the figure to cite for
signature verification's share.
