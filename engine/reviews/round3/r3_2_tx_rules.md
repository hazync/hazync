# Round 3, review 2: transaction-level consensus rules vs Bitcoin Core v28.0

**Tree:** `r3tree` at `a9b78510`. **Core reference:** `~/.cache/hz-libzync-base/bitcoin-core`, tag `v28.0`, commit
`110183746`. Its `git status` shows only `crypto/sha256.cpp`, `hash.h`, `primitives/transaction.{h,cpp}`, `pubkey.cpp`,
`script/interpreter.cpp` and `serialize.h` modified. `consensus/tx_verify.cpp`, `consensus/validation.h` and
`validation.cpp` are pristine, so I compared against real v28.0 source, not my memory of it. I also read the
`transaction.{h,cpp}` patch (a lazy txid/wtxid plus `HzSetHashes`). It does not touch `GetValueOut`, `CTxIn()` or
serialization.

**Question:** can a block that Core v28.0 rejects be accepted through the transaction-level rules?

**Answer:** I did not find one. The split-input design holds up for every rule I traced. Fee, P2SH sigops and witness
sigops use other runs' prevouts, and the code binds those prevouts. Maturity and BIP68 are applied only where the
coin's real height, coinbase flag and MTP are present. Every other rule is either Core's own code, called with the
arguments Core's `ConnectBlock`/`ContextualCheckBlock` pass, or a sum whose limit is equivalent to Core's running
check. I found no Critical, High or Medium issue. The findings below are one Low (a guard placed later than its
comment says) and some Info items. Two of the Info items are assumptions this area relies on and that another review
must own.

---

## Findings, by severity

### L-1 (Low, robustness; confidence high that it exists, low that it matters): the 4 MB transaction guard runs after the first FFI call, not before "any FFI call"

`agg_lite.rs:235-242` refuses `raw_tx.len() > MAX_BLOCK_WEIGHT`, and its comment says this happens "BEFORE any FFI call".
In the chunk, `main.rs:1980` reads `raw_tx` and `main.rs:2003` calls `verify_inputs_batch_once`. That call runs Core's
deserialiser, `PrecomputedTransactionData::Init` and `VerifyScript` on the bytes. Only then does `main.rs:2029` call
`lite.record`, where the guard is. So Core's 32-bit `size_t` arithmetic (sighash serialization, `GetSerializeSize`)
does run on an unbounded transaction first. The buffer the guard really protects (`need`, `agg_lite.rs:247`) is
still protected, because it is sized after the check.

**Impact:** fail-closed in every path I can see. `record` returns false, `all_valid` is 0, and the aggregate refuses
the journal (`agg_lite.rs:545`). Reaching the wrap needs a transaction near 2^30 bytes in guest memory. The real
hazard would be memory corruption in code with no memory protection, which in theory could disturb `all_valid`.
I could not build such a case and do not think one is practical.

**Fix:** move the length check to `main.rs` right after `let tx_len: u32 = env::read()` (before `read_bytes`), or
correct the comment.

### I-1 (Info, verified sound): split runs with the other inputs stubbed out

`core_tx_rules.cpp:101-155`. Every run gets the full prevouts vector (`spent.size() == tx.vin.size()`, line 105). Only
the run's own coins carry their real coinbase flag and height. The others are `(fCoinBase=false, nHeight=0)`
(lines 108-111). Here is what each Core function reads in a split transaction:

| Core function | Reads | Source of a non-owned input's value | Sound? |
|---|---|---|---|
| `CheckTxInputs` maturity | `IsCoinBase`, `nHeight` | stubbed false/0, so it is skipped here; the owning run checks it with the real flag | yes: each coin is checked exactly once, in its own run |
| `CheckTxInputs` value ranges, `in-belowout`, fee range | `coin.out.nValue` for all inputs | prevouts blob | yes: see binding below |
| `SequenceLocks` | `nSequence`, `prevHeights`, `GetAncestor` | non-owned inputs are a default `CTxIn` (`nSequence = SEQUENCE_FINAL`, v28 `transaction.h:121-124`), so the disable bit skips them | yes: the AND of per-run evaluations equals Core's max over all inputs |
| `GetTransactionSigOpCost` (first run only) | `coin.out.scriptPubKey` for all inputs | prevouts blob | yes: see binding below |

**Binding of the non-owned prevouts:** (a) every record commits SHA-256 of the whole prevouts blob, and later runs must
match the input-0 run's value (`agg_lite.rs:619`). (b) Each run's coin leaf commits `spent[idx]` (value and full
scriptPubKey) together with height, flag and MTP (`verify_input.cpp:94-140`, computed in `verify_inputs_batch_once`
from the same `cached_spent` buffer that `hz_tx_rules` reads). (c) The runs tile `[0, n_vin)` exactly once
(`agg_lite.rs:601-606`, `619`, `624`, `654`). (d) Each leaf is opened against the coin log or the block's created set.
Together these mean the fee and sigops computed in run 0 use the coins the other runs verified. This depends on (d)
being sound (see I-6).

### I-2 (Info, verified sound): BIP68 and BIP113 MTP and heights

- **Spending block:** `prev.m_mtp = mtp` and `block.nHeight = height` (`core_tx_rules.cpp:127-134`). The chunk's `mtp` must
  equal the aggregate's `median_time_past(prev.recent_times)` (`agg_lite.rs:549`, `main.rs:2306`), which is Core's
  `pindex->pprev->GetMedianTimePast()`. The rule is enforced from `height >= csv_height` (`agg_lite.rs:290`). That is Core's
  `DeploymentActiveAt(*pindex, CSV)`, and `NET.csv_height` is pinned to Core's `CSVHeight` (`main.rs:364`).
- **Coin time:** Core takes `GetAncestor(max(nCoinHeight-1,0))->GetMedianTimePast()` (`tx_verify.cpp:73`). The glue keys on
  `ch==0 ? 0 : ch-1` (`core_tx_rules.cpp:84`) and returns the leaf's `cmtp`. Created leaves commit the creating block's
  parent MTP: the coinbase with the aggregate's `mtp` (`agg_lite.rs:480`), other transactions with the chunk's `self.mtp`
  (`agg_lite.rs:255`). That is exactly the MTP of block `ch-1`. A coin created in the same block gets `cmtp == mtp`
  through the created-set match, which is Core's `pprev`. A missing key hits Core's own `Assert` (trap), and so does a
  same-height disagreement (`abort`, line 88).
- **IsFinalTx:** the cutoff is `mtp` from `height >= csv_height`, otherwise the header time (`agg_lite.rs:283`). The height is
  the block's own. That matches `ContextualCheckBlock` (`validation.cpp:4222-4237`). The first run checks every
  non-coinbase transaction (`core_tx_rules.cpp:118`), and Core's lifted code checks the coinbase.
- **Speculative txlog chunks** (zero hash and time) are accepted only from CSV on, where the block time is never read,
  and only when the zero-hash flags equal the real flags (`agg_lite.rs:547-548`). At 692,261 the exception flags differ,
  so that block can never be speculative.

### I-3 (Info, verified sound): sigops, weight, fees and subsidy totals

- Sigop cost is Core's `GetTransactionSigOpCost`, using `GetBlockScriptFlags(height, real hash)` in both the chunk
  (`main.rs:1925`) and the aggregate (`agg_lite.rs:451`). The hash is bound by `c_hash == block_hash`. The coinbase counts
  legacy sigops only. The total `<= 80,000` (`agg_lite.rs:908`) is equivalent to Core's running `>` check because every
  term is ≥ 0. Core's legacy `bad-blk-sigops` in CheckBlock is implied, since each cost term includes legacy × 4.
- On rv32 the counters are `unsigned int` (32 bits, as on x86-64) and `CountWitnessSigOps` returns `size_t` (32 bits here,
  64 on x86-64). Per-input and per-transaction values are bounded by the 1 MB stripped and 4 MB total sizes (≤ 80 M), and
  they are added into `int64_t`. No divergence.
- Weight is the sum of Core's `GetTransactionWeight` plus `4·(80 + CompactSize(ntx))` (`agg_lite.rs:896-900`). That equals
  `GetBlockWeight`. Each transaction is ≤ 4,000,000 raw bytes (coinbase `agg_lite.rs:471`, others `:239`), so Core's
  `int32_t` and 32-bit `size_t` arithmetic cannot wrap.
- Fees: each `MoneyRange` is checked by `CheckTxInputs`. The block total is checked in `[0, MAX_MONEY]` (`agg_lite.rs:890`),
  which equals Core's running `accumulated-fee-outofrange` check for non-negative terms. `coinbase_val <= subsidy +
  total_fee` is `bad-cb-amount`. `GetBlockSubsidy` is Core's lifted text. The extra `subsidy + fee <= MAX_MONEY` check is
  stricter and unreachable, as the doc says.

### I-4 (Info, verified): coinbase and witness rules

The guest runs Core's lifted `ContextualCheckBlock` and `CheckWitnessMalleation`. `BlockWitnessMerkleRoot` is answered
from the committed wtxids. Every run commits its wtxid and the runs must agree (`agg_lite.rs:619`). The dummy witness
transaction only feeds `unexpected-witness`, and weight is macro-replaced, so the dummy adds nothing to it.
`bad-cb-missing` and `bad-cb-multiple` come from `is_coinbase_tx` on the coinbase and on every record (`agg_lite.rs:268`).
`CheckTransaction` runs on the coinbase (`check_tx`). Merkle mutation and `ids_distinct` cover `bad-txns-duplicate`.

### I-5 (Info, verified): BIP30/BIP34 and the two flag-exception blocks

The guest has no BIP34 skip window, so it enforces BIP30 at all heights through the tracked SMT
(`coinbase-smt/src/bip30.rs:1-60`). Every coinbase is inserted. Spends of untracked coinbases are never decremented,
so a repeat of one is always refused. Non-coinbase transactions are tracked exactly when all their inputs are, and the
claim is checked (`agg_lite.rs:609-611`, `628`, `655`). The divergences from Core all go toward stricter (the doc's
STRICTER rows). I found no path where Core's BIP30 rejects and the tree accepts. `hz_indicated_height` reproduces
`CScript() << Y` prefix matching (`verify_input.cpp:1084-1098`). Script flags at 170,060 and 692,261 come from Core's
`GetBlockScriptFlags` via `script_flag_exceptions` on the raw dSHA256 hash, which is uint256 internal order. I did not
re-derive the bip30.rs proofs line by line.

### I-6 (Info, reliance, not verified here): what this area takes on trust

1. The coin log or ECMH opens every external leaf, including the full value, script, height, coinbase/tracked flag and
   MTP. If it fails, then maturity, BIP68, fee and sigops all fail with it.
2. The chunk journals reaching the aggregate are proofs of this image, and `all_valid == 1` is required.
3. `prev.recent_times` and the prev state are bound by recursion (chain linkage).
4. **Address-keyed caches** (`cached_tx` and `cached_spent`, `verify_input.cpp:219-260`) compare only the first and last 32
   bytes on a hit. That is safe today because the chunk resets them before every transaction, the aggregate passes only
   the live coinbase buffer, and `c_heap::startup_check` asserts that Rust's allocator never reuses an address
   (`c_heap.rs:162-169`). This is a latent hazard if the OpenVM engine's Rust allocator differs. The startup assert
   would catch that, but only if it runs in the engine image. I did not check the engine's allocator.

### I-7 (Info): two fail-closed paths

- In a non-first run, `CheckTxInputs` calls `GetValueOut`, which throws on an out-of-range output, because
  `CheckTransaction` runs only in the first run. In the guest a throw aborts, so no proof exists, and that transaction is
  invalid in Core anyway. Only liveness is affected.
- `HzCoinsView::AccessCoin` uses `assert`, and `util/check.h` refuses to compile with `NDEBUG`, so the assertion is
  always on.

---

## Checks against CONSENSUS_RULES.md

These rows match the code at `a9b78510`: 182 (legacy sigops), 183 (nLockTime), 184 (BIP34), 188 (weight), 224 (BIP68),
225 (sigop cost), 226 (`bad-cb-amount`, STRICTER), 271-279 (flags and heights), and 42 (the 4 MB guard; but see L-1 for
where it sits). The doc's own header says it was read at `bf8e60764` and the table "has not been re-read line by line
at c14". I found nothing in the transaction rules that contradicts it.

## What I could not verify

- The coin log, coinlog_forest, ECMH and trie soundness, and the recursion binding: I-6 items 1-3.
- The OpenVM engine guest (`engine/guest/assemble.sh` output) and its allocator. I reviewed only the risc0 source tree.
- The generated `core_extract.inc` itself, since I built nothing. `build.rs`'s `lift()` cuts at the first `"\n}\n"`. I checked
  that in v28.0 the first column-0 `}` after each of the six signatures is the function's real end: `GetBlockSubsidy`
  1919-1930, `GetBlockScriptFlags` 2361-2400, `CheckWitnessMalleation` 3951-3997, `ContextualCheckBlockHeader` 4159-4212,
  `ContextualCheckBlock` 4220-4275, and pow.cpp's `GetNextWorkRequired` ending at line 47.
- I ran no tests and no rig cases. This review is source reading only.
