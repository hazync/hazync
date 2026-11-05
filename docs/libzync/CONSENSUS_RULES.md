# Consensus rules: the libzync guest against Bitcoin Core v28.0

**For:** outside reviewers of the proposal in [`SWIFTSYNC_PROOFS.md`](SWIFTSYNC_PROOFS.md).

**Read at:** repo commit `bf8e60764` (branch `libzync`). Its `prover/`, `coinbase-smt/` and `libzync/` are clean, so the
guest read here is that commit. These guest inputs are newer than `c1101a3`, the source of the image ids in
`reproduce/LIBZYNC_IDS`, so those ids do not describe this guest. Bitcoin Core v28.0 `110183746` and libsecp256k1 v0.5.1
`642c885`, with the patches `scripts/libzync-base.sh` applies. `patches/` is unchanged since the byte-for-byte check of the
patched files at `278c239cd`. In the Core tree read here only `crypto/sha256.cpp`, `hash.h`, `primitives/transaction.{h,cpp}`,
`pubkey.cpp`, `script/interpreter.cpp` and `serialize.h` differ from v28.0, so `validation.cpp`, `pow.cpp`, `chain.h`,
`consensus/tx_verify.cpp`, `consensus/tx_check.cpp` and `consensus/merkle.cpp` are pristine.
Every reject string below came from reading the Core source, not from memory. Every `file:line` is at these commits.

## Status

| status | rows | meaning |
|---|---|---|
| SAME | 59 | Both sides read: the same blocks are rejected |
| STRICTER | 3 | The guest rejects something Core accepts |
| WEAKER | 0 | The guest enforces a looser form of the rule |
| NOT ENFORCED | 0 | The guest does not check the rule |
| N-A | 1 | The rule does not apply on mainnet or signet |
| NOT FOUND | 0 | No enforcing code could be located |

63 rows. 45 name a rig case, 18 are "read only". Counted for an image that checks scripts. Rows marked † are true only once
the proof is closed at H (below).

### Every row that is not SAME

A STRICTER row costs liveness: no proof exists for a chain Core accepts. It cannot make a proof claim something false.
A WEAKER or NOT ENFORCED row is the kind that matters for soundness: a proof could exist for a chain Core rejects.

| row | status | what it means for a proof |
|---|---|---|
| `bad-txns-oversize`, `bad-blk-length`, `bad-blk-weight` | **SAME** (was WEAKER, fixed) | On rv32 `size_t` is 32 bits, so `tx_check.cpp:20` wraps at a stripped size of 2^30, and Core's `GetTransactionWeight` (`consensus/validation.h:149`, `int32_t`) wraps the same way. The guest now refuses any transaction over 4,000,000 raw bytes before that arithmetic runs (`agg_lite.rs:127`, coinbase `:470`). Below that bound nothing wraps. It rejects nothing Core accepts: weight ≥ raw size, so such a transaction already fails `bad-blk-weight`. |
| `bad-txns-BIP30`, non-coinbase transactions | **SAME** † (tested, rig `bip30_repeat_*`) | The tree tracks every transaction whose inputs are all repeatable coins, the only ones that can repeat: 173 on mainnet. Refused while it has unspent outputs, allowed once it has none, as in Core. |
| `bad-txns-BIP30`, coinbases | **STRICTER** † on a chain that is not mainnet | Coinbases indicating a future height behave as in Core. An untracked coinbase repeated before BIP34 is refused even when fully spent, which Core allows. Mainnet has no such block: exactly two txids repeat, both grandfathered. Blocks at height 100,000,000 and above are refused. |
| Spending the coinbase coins of 91,722 and 91,812 | **STRICTER** (known) | These coins are never indexed, so a spend of one fails at H. Core allows the spend on a chain that holds the block but has not overwritten its coinbase. Mainnet never spent them. Only a fork reaches this, and no proof can be made for that fork |
| `bad-cb-amount` | **STRICTER** † (new, harmless) | The guest also requires subsidy + fees ≤ `MAX_MONEY`. Core does not check this. No chain reaches it: fees cannot exceed the coins already issued, and all subsidies together are below `MAX_MONEY` |
| `time-timewarp-attack` | **N-A** | testnet4 and regtest only |

**Assumevalid images only** (`HAZYNC_LIBZYNC=assumevalid`, `ChainState.profile` bit 1): these rows become NOT ENFORCED:
`mandatory-script-verify-flag-failed`, `block-validation-failed`, the three signature rows, and the script side of the
soft-fork table. The proof says so itself (§5 step 9). BIP325 stays enforced there, because it runs in the aggregate.

### Rows marked †: true only at H

A block's spent coins are prover-supplied. A spend names a coin by its leaf, and nothing checks that coin exists until
the ECMH total is the identity at H (`ss_set == 0`, §5 step 5). Everything computed from a prevout therefore rests on
that final check: value, fee, maturity, BIP68, P2SH and witness sigops, the script being satisfied, and which spends
decrement the BIP30 tree.

- A proof that passes the whole of §5 establishes these rows, under the ECMH collision assumption of §4.
- Core rejects the bad block itself. The guest may reject it only at H, without saying which block failed.
- A receipt not closed at H establishes none of them (§6a).

## Tested against Core

`experimental/libzync-bench/core_rules/rig.py` runs real Bitcoin Core and the regtest guest on the same crafted blocks.
Bitcoin Core 28.0, the release the guest compiles (`rig_v28.out`).
95 cases: 94 agree, 1 (`missing_coin`, a coin that does not exist) is deferred to H by design (regtest guest `r15reg`,
libzync `3d9526f`, 2026-11-02). Controls with no row: `valid_empty`, `valid_spend`.

The "Tested" columns name the cases that exercise a row, controls included. A case counts only if Core's answer in
`rig_v28.out` is that row's reject reason. Four cases named for a rule do not meet that, so they are not counted for it:

| case | named for | Core's answer | why |
|---|---|---|---|
| `merkle_mutated` | `bad-txns-duplicate` | `bad-witness-merkle-match` | Three leaves `[cb, t, t]`: the last `t` has no partner, so `mutated` stays false |
| `cb_missing` | `bad-cb-missing` | `decode-failed` | `submitblock` refuses it before validation |
| `tx_no_inputs` | `bad-txns-vin-empty` | `decode-failed` | No inputs parses as the witness marker. The guest never sees it: the host's parser stops it |
| `tx_null_prevout` | `bad-txns-prevout-null` | `bad-cb-multiple` | One input with a null prevout is a coinbase. Counted for `bad-cb-multiple` |

Limits of the column:

- The check is two-sided. A case is AGREE only if the guest gives Core's verdict AND Core's reason: Core's own string where
  the rule is Core's text (`core-reject: <reason>` from the guest), the guest's failing check where the rule is ours
  (`OURS` in `rig.py`). A refusal for another reason prints REASON.
- Regtest differs from mainnet in the branches it reaches. It never retargets, so `bad_bits` tests only the value inside an
  epoch. Its activation heights are 0 or 1 except BIP34 at 110, and every case is at height 233, so no case sits on an activation boundary.
- Core 28.0 here reports every script failure as `block-validation-failed`, so `mandatory-script-verify-flag-failed` stays
  read only.
- The two script-flag exception blocks, 170,060 and 692,261, are not rig cases. They were run on mainnet with a control: the
  host fails the exception's chunk when the block hash is changed (`core_rules/exception_blocks.out`).

### Core's own test vectors

Core 28's `script_tests.json`, `tx_valid.json` and `tx_invalid.json` run through the guest's consensus code (guest mode
10; cases from `scripts/libzync/core_vectors.py`, which follows Core's test drivers; `host core-vectors`). Image c10e10,
libsecp path: **1,420 / 1,420 agree, 0 differ**, the exact `ScriptError` for every script test
(`experimental/libzync-bench/core_vectors/RESULT_c10e10_base.txt`). The batched-signature path (honest and forged hints)
and the 22,666 flag variants: see ROADMAP I1.

### Where Core catches an exception, the guest aborts

The guest has no C++ unwinding: a `throw` in Core's code aborts it (no receipt). Core catches in two places the guest can
reach, and both REJECT the block, so the abort gives the same answer (no proof of a valid chain is ever lost):

| site | Core | the block |
|---|---|---|
| `script/interpreter.cpp` `EvalScript` | `catch (...)` -> `SCRIPT_ERR_UNKNOWN_ERROR`; every `EvalScript` caller in `VerifyScript` then fails | invalid (68 of Core's script / tx vectors, all agree) |
| `signet.cpp` `SignetTxs::Create` | malformed solution -> `nullopt` -> `CheckSignetBlockSolution` false | invalid |

Core's `logging.h` catches and CONTINUES, but `coreshim/logging.h` replaces it. The list is enforced at every guest
build: `prover/methods/guest/build.rs` walks the include graph from every C++ file the guest compiles (resolving headers
as the compiler does, shim first) and fails unless the catch sites are exactly `REVIEWED_CATCHES`; a Core bump that adds
one must be reviewed (does Core reject after it?) before the guest builds. The host reports a guest abort as
`BLOCK REFUSED` (`lz::guest_failure`).

## How to read the "where enforced" column

The guest compiles these Core sources unmodified (`prover/methods/guest/build.rs:311-325`, plus `signet.cpp` on signet at
`:388`): `script/interpreter.cpp`, `script/script.cpp`, `primitives/transaction.cpp`, `primitives/block.cpp`, `pubkey.cpp`,
`hash.cpp`, the `crypto/` hashes, `consensus/tx_check.cpp`, `consensus/merkle.cpp`, `arith_uint256.cpp`, `pow.cpp`,
`chain.cpp`, `kernel/chainparams.cpp`. `consensus/tx_verify.cpp` is also compiled unmodified, by `#include` in
`core_tx_rules.cpp:88`, so `IsFinalTx`, `Consensus::CheckTxInputs`, `SequenceLocks`, `GetTransactionSigOpCost` and
`GetTransactionWeight` are Core's code, run per run of a transaction's inputs.

It does **not** compile `validation.cpp` or `coins.cpp`. `build.rs:399-414` lifts six functions' text out of `validation.cpp`
and `pow.cpp` at build time into `core_extract.inc`, and the build fails if one is not found: `GetNextWorkRequired`,
`GetBlockSubsidy`, `CheckWitnessMalleation`, `GetBlockScriptFlags`, `ContextualCheckBlockHeader`, `ContextualCheckBlock`.
`core_block_rules.cpp:103` compiles them against the stand-ins listed in that file's header.

`CheckBlock` and `ConnectBlock` are **not** lifted: each walks a whole block, and no single guest run holds one. Their
checks are composed instead. `CheckProofOfWork` and `CheckTransaction` are called directly, in a chunk or in the aggregate;
the merkle, coinbase-position, coin and block-total checks are our Rust and glue around Core's primitives; `bad-blk-length`
and the legacy `bad-blk-sigops` have no check of their own and follow from a tighter limit. The rows say which.

| tag | meaning |
|---|---|
| **Core** | Core or libsecp source, compiled unchanged |
| **Core lifted** | Text of a Core function extracted at build time and compiled, unedited, against our stand-ins |
| **Core patched** | Core source with a patch from `patches/` in that file (listed at the end) |
| **Glue** | Hand-written C++ using Core's types, in `verify_input.cpp` or the stand-ins of `core_tx_rules.cpp` and `core_block_rules.cpp` |
| **Rust** | Guest Rust, `prover/methods/guest/src/` and `coinbase-smt/src/`. "(verifier)" marks Rust outside the guest: `libzync_verify_cmd`, host `main.rs:2015` |

Paths: `agg_lite.rs`, `main.rs`, `script_flags.rs`, `batch_ecdsa.rs` are in `prover/methods/guest/src/`; `verify_input.cpp`,
`core_tx_rules.cpp` and `core_block_rules.cpp` are in `prover/methods/guest/`; `bip30.rs` is in `coinbase-smt/src/`; "host
`main.rs`" is `prover/host/src/main.rs`; `validation.cpp`, `pow.cpp`, `signet.cpp`, `coins.cpp`, `tx_check.cpp`,
`tx_verify.cpp`, `transaction.h`, `chain.h` and `chainparams.cpp` are Core's.

**How a result takes effect.** A chunk folds every per-transaction result into `all_valid` (`main.rs:1897`, `:1901`, `:1967`)
and commits it (`:1975`). The aggregate requires it (`agg_lite.rs:254`), gathers its own flags (`main.rs:2108`) and asserts
them all (`main.rs:2131`). A failed assert means no receipt. Every chunk result is tied to the block by height, block
hash, MTP and time (`agg_lite.rs:258`), and to the block's transactions by the merkle root and witness commitment.
A libzync image accepts only modes 4, 5, 8, 9, 13, 15 and 16 (`main.rs:2173`), so a `ChainState` can come only from the chunk
and aggregate path. The older transcriptions still in `verify_input.cpp` and `main.rs` (`is_final_tx`, `check_input_locks`,
`tx_full_sigops`, `check_witness_commitment`, `check_bip34`, `validate_block`) cannot produce one.

## Header and chain

| reject reason | rule | where enforced in the guest | status | Tested |
|---|---|---|---|---|
| `high-hash` | `nBits` decodes to a target that is positive, not overflowing and ≤ `powLimit`; block hash ≤ target. `validation.cpp:3913`, `pow.cpp:137` | **Core** `CheckProofOfWork`, through glue `check_pow`, `verify_input.cpp:626`, which hashes the header; called `agg_lite.rs:383` | SAME. Core's function and Core's `powLimit` | `high_hash` |
| `bad-diffbits` | `nBits` equals `GetNextWorkRequired`: the parent's value inside an epoch, a retarget at each 2,016th height over the first and last timestamps of the epoch. `validation.cpp:4168`, `pow.cpp:13` | **Core lifted** `ContextualCheckBlockHeader` and `GetNextWorkRequired`, `core_block_rules.cpp:132`, called `main.rs:2116`; **Core** `CalculateNextWorkRequired` (`pow.cpp:49`) through glue `core_block_rules.cpp:86`, `calc_next_bits` `verify_input.cpp:967`. Epoch start `main.rs:2136` | SAME. Same first block as Core: height − 2,016, carried in `epoch_start` and answered by the stand-in's `GetAncestor`. The min-difficulty branch is now compiled, and dead on mainnet and signet (`chainparams.cpp:102`, `:463`) | `bad_bits`, inside an epoch only; the retarget is read only |
| `time-too-old` | Block time > median of the last 11 timestamps ending at the parent. `validation.cpp:4184`, `chain.h:278` | **Core lifted** `ContextualCheckBlockHeader`; **Core** `CBlockIndex::GetMedianTimePast` through glue `hz_median_time_past`, `verify_input.cpp:1015`, `main.rs:1461`, `:2097`. **Rust** window `main.rs:2137-2139` | SAME | `time_equal_mtp`, `time_above_mtp` |
| `bad-version(0x%08x)` | Version < 2 from BIP34, < 3 from BIP66, < 4 from BIP65, compared as signed `int32_t`. `validation.cpp:4204-4208` | **Core lifted** `ContextualCheckBlockHeader`. **Glue** `DeploymentActiveAfter`, `core_block_rules.cpp:78`, on Core's height table | SAME. Signed on both sides. The stand-in is Core's comparison without the version-bits cache | `bad_version`, `version_4`; the version 3 branch only |
| `time-timewarp-attack` | Testnet4 and regtest only (`enforce_BIP94`). `validation.cpp:4188-4193` | **Core lifted**, compiled and dead: `enforce_BIP94 = false` on mainnet and signet (`chainparams.cpp:103`, `:464`) | N-A | read only |
| `prev-blk-not-found` | The parent is known. `validation.cpp:4307` | **Rust** `main.rs:2109`: `hashPrevBlock` equals the verified previous state's tip | SAME | read only |
| `bad-prevblk` | The parent is not invalid. `validation.cpp:4312`, `:4353` | **Rust** `main.rs:2079`: the previous state is a verified receipt, so an invalid parent has no receipt to build on | SAME | read only |
| `bad-signet-blksig` | Signet: the block solution satisfies the challenge. `validation.cpp:4012-4013`, `signet.cpp:124` | **Core** `signet.cpp` through glue `hz_check_signet`, `verify_input.cpp:939`; called `agg_lite.rs:392-400` | SAME on signet, no such rule on mainnet. Non-coinbase transactions are placeholders with their txid seeded from the list the merkle check binds; `signet.cpp:57-64` reads only `GetHash()` from them. The signature runs through libsecp's own path: the aggregate installs no batch hints (`install` is called only at `main.rs:1798`) | read only |

## Block structure: `CheckBlock`, `ContextualCheckBlock`

| reject reason | rule | where enforced in the guest | status | Tested |
|---|---|---|---|---|
| `bad-txnmrklroot` | The header's merkle root equals the root over the txids. `validation.cpp:3925` | **Core** `ComputeMerkleRoot` (`consensus/merkle.cpp:45`, hash primitive patched) through glue `merkle_root`, `verify_input.cpp:711`; compared `agg_lite.rs:387`. The list is the coinbase txid, then each transaction's txid in chunk order (`agg_lite.rs:234`, `:275`) | SAME | `bad_merkle` |
| `bad-txns-duplicate` | No mutation of the tree (CVE-2012-2459). `validation.cpp:3935` | **Core** `mutated` flag, `agg_lite.rs:387` | SAME. The guest also refuses any two equal txids (`agg_lite.rs:402-406`, `:415`). That rejects nothing new: Core refuses them as a double spend, and a transaction with no inputs fails `bad-txns-vin-empty` | read only, `txns_duplicate` |
| `bad-blk-length` | At least one transaction; transaction count × 4 and stripped size × 4 ≤ 4,000,000. `validation.cpp:4028-4029` | No check of its own. **Core lifted** weight limit, as `bad-blk-weight`. The coinbase is always present | SAME (see above). Weight ≥ 4 × stripped size and each transaction is at least 60 bytes, so this holds whenever weight is computed without wrapping | `oversize_weight` |
| `bad-cb-missing` | The first transaction is a coinbase. `validation.cpp:4033` | **Glue** `is_coinbase_tx` (`verify_input.cpp:760`, Core's `IsCoinBase`), `agg_lite.rs:444-445`. Its txid is the first merkle leaf (`:234`) | SAME | read only |
| `bad-cb-multiple` | No other transaction is a coinbase. `validation.cpp:4036` | **Rust** `agg_lite.rs:104`, with glue `is_coinbase_tx`, on every run of every transaction | SAME | `cb_multiple`, `tx_null_prevout` |
| `bad-blk-sigops` (legacy) | Legacy sigops × 4 ≤ 80,000. `validation.cpp:4055-4056` | No check of its own. **Rust** limit `agg_lite.rs:465` on **Core** `GetTransactionSigOpCost` | SAME. The ConnectBlock total below starts from the same legacy × 4 for every transaction, so it can only be larger | `sigops_limit`, `sigops_at_limit` |
| `bad-txns-nonfinal` (nLockTime) | Every transaction, coinbase included, is final at this height and cutoff. The cutoff is the parent's MTP from CSV activation, the block time before it. `validation.cpp:4228-4238`, `tx_verify.cpp:17` | **Core** `IsFinalTx`, `core_tx_rules.cpp:110`, with **Rust** cutoff `agg_lite.rs:121`; **Core lifted** `ContextualCheckBlock` for the coinbase, `core_block_rules.cpp:170` | SAME. `height >= csv_height` is `DeploymentActiveAfter` | `locktime_height_future`, `locktime_height_ok`, `locktime_final_seq`, `locktime_time_mtp`, `locktime_time_past`; the coinbase path is read only |
| `bad-cb-height` | From BIP34, the coinbase scriptSig starts with `CScript() << height`. `validation.cpp:4242-4248` | **Core lifted** `ContextualCheckBlock`, called `agg_lite.rs:462` | SAME. Mainnet 227,931, signet 1 | `cb_wrong_height` |
| `unexpected-witness` | No transaction has a witness unless the block carries a valid commitment. `validation.cpp:3989` | **Core lifted** `CheckWitnessMalleation`. **Glue** `core_block_rules.cpp:157-162`: one stand-in transaction with a witness says "another transaction has one". `has_witness` covers the coinbase (`agg_lite.rs:236`) and every transaction (`:277`) | SAME | `witness_no_commit`, `no_commit_no_witness` |
| `bad-witness-nonce-size` | With a commitment, the coinbase witness is one 32-byte item. `validation.cpp:3962` | **Core lifted** `CheckWitnessMalleation`, on the real coinbase | SAME. Checked whenever a commitment output exists, even with no witness in the block, as in Core | `nonce_too_big_witness` |
| `bad-witness-merkle-match` | The last matching output (at least 38 bytes, prefix `6a24aa21a9ed`) holds SHA256d(witness root ‖ reserved value). `validation.cpp:3975` | **Core lifted** `CheckWitnessMalleation`. **Glue** `hz_witness_root`, `core_block_rules.cpp:93`, over the chunks' wtxids (`agg_lite.rs:276`) with **Core** `ComputeMerkleRoot` | SAME. Coinbase leaf zero, as in `BlockWitnessMerkleRoot`. Every run of a transaction commits the wtxid and they must agree (`agg_lite.rs:290`) | `witness_mismatch`, `merkle_mutated` |
| `bad-blk-weight` | Block weight ≤ 4,000,000, at every height. `validation.cpp:4271` | **Core lifted** limit in `ContextualCheckBlock`. **Glue** `GetBlockWeight` answered from the block total, `core_block_rules.cpp:101`. **Rust** total `agg_lite.rs:279`, `:455-459`, of **Core** `GetTransactionWeight`, `core_tx_rules.cpp:143`, `:154` | SAME (see above). `3×stripped + total` per transaction plus `4 × (80 + CompactSize(n))` equals `GetBlockWeight`. Each term is 32-bit here and the size bound keeps it from wrapping | `weight_over`, `weight_under` |

## Transactions: `CheckTransaction`

All nine run Core's `consensus/tx_check.cpp`, unchanged. The run holding input 0 calls it in `hz_tx_rules`
(`core_tx_rules.cpp:109`, from `agg_lite.rs:124`) for each non-coinbase transaction; the aggregate runs it for the coinbase
through glue `check_tx` (`verify_input.cpp:582`, from `agg_lite.rs:441`).

| reject reason | rule | where enforced in the guest | status | Tested |
|---|---|---|---|---|
| `bad-txns-vin-empty` | At least one input | **Core** `tx_check.cpp:15` | SAME | read only |
| `bad-txns-vout-empty` | At least one output | **Core** `tx_check.cpp:17` | SAME | `tx_no_outputs` |
| `bad-txns-oversize` | Stripped size × 4 ≤ 4,000,000 | **Core** `tx_check.cpp:20` | SAME (see above). On rv32 this `size_t` product would wrap at 2^30 bytes; `agg_lite.rs:127` refuses first | read only |
| `bad-txns-vout-negative` | No negative output value | **Core** `tx_check.cpp:28` | SAME | `tx_negative_out` |
| `bad-txns-vout-toolarge` | No output above `MAX_MONEY` | **Core** `tx_check.cpp:30` | SAME | `tx_huge_out` |
| `bad-txns-txouttotal-toolarge` | Output total stays in range | **Core** `tx_check.cpp:33` | SAME | `tx_total_huge` |
| `bad-txns-inputs-duplicate` | No repeated input (CVE-2018-17144) | **Core** `tx_check.cpp:44` | SAME | `tx_dup_inputs` |
| `bad-cb-length` | Coinbase scriptSig is 2 to 100 bytes | **Core** `tx_check.cpp:50` | SAME | `cb_script_short`, `cb_script_long` |
| `bad-txns-prevout-null` | No null prevout outside a coinbase | **Core** `tx_check.cpp:56` | SAME | `prevout_null` |

## Coins and block totals: `ConnectBlock`, `CheckTxInputs`

`hz_tx_rules` (`core_tx_rules.cpp:93`) calls Core's `CheckTxInputs` and `SequenceLocks` once per run of a transaction's inputs.
Every run sees all the transaction's prevouts, tied together by their hash (`agg_lite.rs:290`), but only its own coins'
height, coinbase flag and MTP. Each input belongs to exactly one run, so the runs together cover the transaction.

| reject reason | rule | where enforced in the guest | status | Tested |
|---|---|---|---|---|
| `bad-txns-BIP30` (coinbase) † | A coinbase may not create an outpoint that is still unspent. Skipped at 91,842 and 91,880, and from 227,932 to 1,983,701 on a chain through the BIP34 block. `validation.cpp:2505-2575` | **Rust** `bip30.rs:165`: the slot must be absent or `REPEATABLE`. Tracked or not: `agg_lite.rs:214-217` | STRICTER (see above). The guest has no skip window. In it Core's comment (`validation.cpp:2522-2523`, `:2539`) leaves only 490,897, indicated by 176,684's coinbase, spent at 185,956: `REPEATABLE`, allowed on both sides | read only. The rig's base chain stages allowed repeats at 130 and 131; no case repeats a coinbase while unspent |
| `bad-txns-BIP30` (non-coinbase) † | A non-coinbase transaction may not create an outpoint that is still unspent. Same conditions | **Rust** `bip30.rs:191`, tracked transactions; `agg_lite.rs:94`, `:268`, `:286` derive which they are | SAME. Whether a coin is tracked comes from its leaf flag, so this rests on H | `bip30_repeat_unspent`, `bip30_repeat_spent`, `bip30_child_of_repeat` |
| `bad-txns-inputs-missingorspent` † | Every input spends an unspent coin. For a coin made in this block: an earlier transaction made it. `tx_verify.cpp:168` | **Rust**: unknown or already spent coin: `swiftsync::sub`, `agg_lite.rs:365`, then **Rust** (verifier) `ss_set == 0`, host `main.rs:2071`. Same coin twice in a block: `agg_lite.rs:339-346`. In-block order and single spend: `:351-352`. Coin claiming a later height: `:117`. Coin claiming this height that this block does not create: `:361-364`. Core's `HaveInputs` runs but its stand-in cannot refuse a real input, `core_tx_rules.cpp:52` | SAME † | `double_spend_in_block`, `spend_later_in_block`, `chain_in_block`; `missing_coin` is deferred: Core refuses, the guest accepts the block and the rig stops short of H |
| `bad-txns-premature-spend-of-coinbase` † | A coinbase coin waits 100 blocks. `tx_verify.cpp:180` | **Core** `CheckTxInputs`, `core_tx_rules.cpp:115`, on this run's coins. **Rust** flag ≤ 3, `agg_lite.rs:117`; height and flag bound by the leaf (`verify_input.cpp:94`) | SAME †. Signet is also 100 | `premature`, `one_short_of_mature`, `exactly_mature` |
| `bad-txns-inputvalues-outofrange` † | Each input value and the running sum stay in `MoneyRange`. `tx_verify.cpp:187` | **Core** `CheckTxInputs` | SAME † | read only |
| `bad-txns-in-belowout` † | Inputs ≥ outputs. `tx_verify.cpp:193` | **Core** `CheckTxInputs` | SAME † | `in_below_out`, `valid_zero_fee` |
| `bad-txns-fee-outofrange` † | Fee in `MoneyRange`. `tx_verify.cpp:200` | **Core** `CheckTxInputs` | SAME † | read only |
| `bad-txns-accumulated-fee-outofrange` † | The block's running fee stays in `MoneyRange`. `validation.cpp:2632` | **Rust** `agg_lite.rs:278`, `:449` | SAME †. Checks the final sum, not each step. Every fee is ≥ 0, so the sum only grows and the two agree | read only |
| `bad-txns-nonfinal` (BIP68) † | From CSV, for version ≥ 2 and disable bit clear, each input's relative lock has passed. `validation.cpp:2645`, `tx_verify.cpp:39-108` | **Core** `SequenceLocks`, `core_tx_rules.cpp:131`, `:138`. **Rust** height gate `agg_lite.rs:126`. **Glue** block index `core_tx_rules.cpp:65-86`: `GetAncestor` answers from the coin's leaf-committed MTP | SAME †. Version is unsigned on both sides (`transaction.h:308`). The coin's MTP is the one `GetAncestor(coin − 1)` gives, and the leaf commits it. A coin made in this block uses this block's parent MTP, as in Core. In a split transaction, inputs outside the run are given `SEQUENCE_FINAL`, which Core's code skips (`core_tx_rules.cpp:133-138`) | `bip68_height_unmet`, `bip68_height_met`, `bip68_time_unmet`, `bip68_time_met`, `bip68_disabled`, `bip68_v1_ignored`, `bip68_uint_version`, `split_tx`, `split_tx_bad` |
| `bad-blk-sigops` (cost) † | Sum of legacy × 4 + P2SH × 4 + witness sigops ≤ 80,000; coinbase legacy only. `validation.cpp:2656`, `tx_verify.cpp:145` | **Core** `GetTransactionSigOpCost`, `core_tx_rules.cpp:144`, `:150`; **Rust** sum and limit `agg_lite.rs:280`, `:456`, `:465` | SAME †. Same flags: `GetBlockScriptFlags`, in chunk and aggregate | `sigops_witness_over`, `sigops_witness_under`, `sigops_p2sh_over`, `sigops_p2sh_under` |
| `bad-cb-amount` † | Coinbase value ≤ fees + subsidy. `validation.cpp:2689-2692` | **Rust** `agg_lite.rs:447-453`; **Core lifted** `GetBlockSubsidy`, `core_block_rules.cpp:108`; **Glue** `coinbase_value`, `verify_input.cpp:723` | STRICTER, unreachable. The limit itself is the same. The guest adds subsidy + fees ≤ `MAX_MONEY` | `cb_overpay`, `cb_fee_too_much`, `cb_exact` |
| `mandatory-script-verify-flag-failed (…)` † | Every input's script passes `VerifyScript` with the block's flags. `validation.cpp:2208` | **Core patched** `script/interpreter.cpp` through glue `verify_inputs_batch_once`, `verify_input.cpp:1200-1211`, `main.rs:1875`; result `agg_lite.rs:143` | SAME †. Same flags (table below). The interpreter's own script errors are not listed: the file is compiled with only the hunks at the end of this document | read only |
| `block-validation-failed` † | The same failure, as reported with parallel script threads. `validation.cpp:2697` | As above | SAME † | `script_fails`, `sig_p2wpkh_bad`, `sig_not_der`, `sig_zero_r`, `sig_p2pkh_other_key`, `multisig_order`, `multisig_dummy`, `sig_must_fail_but_good`, `taproot_key_bad`, `taproot_tweak_parity`, `taproot_script_bad_sig` |

## Rules with no reject string

| rule | Core | where enforced in the guest | status | Tested |
|---|---|---|---|---|
| Transaction parsing | `transaction.h:216-254`: canonical CompactSize, superfluous witness and unknown flag rejected. A failed parse means no block | **Core patched** `UnserializeTransaction` (`serialize.h` gains 32-bit `int` overloads; parsing unchanged), through glue `cached_tx`, `verify_input.cpp:219`. A failed parse aborts: no proof | SAME | read only. `tx_no_inputs` stops at the host's parser |
| Block height | From the block index | **Rust** `main.rs:2093`: exactly previous height + 1 | SAME | read only |
| Genesis and its coinbase | `validation.cpp:2451` skips genesis's transactions; its coin never exists | **Rust** (verifier) §5 step 3, host `main.rs:2059`: the proof must start from `genesis_anchor`, host `main.rs:1280`, with an empty forest and `recent_times = [genesis time]`. The guest commits the digest of its base state at `main.rs:2096` | SAME | read only |
| Unspendable outputs never become coins | `coins.cpp:72`: OP_RETURN, or script over 10,000 bytes | **Core** `IsUnspendable` in glue `tx_out_leaves`, `verify_input.cpp:358` | SAME | read only |
| Grandfathered coinbases 91,842 and 91,880 | `validation.cpp:6304`, height and hash | **Rust** `agg_lite.rs:201`, `:414`, `bip30.rs:180`; hashes `main.rs:334-335` (checked equal to Core's) | SAME. The hash alone fixes the height | read only |
| Spending the coinbase coins of 91,722 and 91,812 | Ordinary coins until overwritten. `IsBIP30Unspendable` (`validation.cpp:6310`) is not consensus code: only `coinstatsindex.cpp` calls it | **Rust** `agg_lite.rs:223`: never indexed, so a spend fails at H | STRICTER. Same final set as Core on the real chain, where neither was spent. Keyed on height and hash, so signet is unaffected | read only |
| 64-byte transactions | No consensus rule in v28. `IsBlockMutated` (`validation.cpp:4109`) is called only from `net_processing.cpp`, and tests for them only in a block with no coinbase | None | SAME. Neither side rejects them. The txid list a proof uses is bound by the merkle root, with the same inner-node ambiguity Core has | `tx_64_bytes` |

## Signatures

Each check is hooked in libsecp after its own parsing and range checks (`patches/libzync/0102`). If the hook returns 0, libsecp
verifies as usual (**Core patched**). If it returns 1, the check joins the chunk's batch, `finish()` runs before the chunk
commits (`main.rs:1967`), and a failure clears `all_valid`. In collect mode no check joins a batch: every one runs in libsecp.

**The hint cannot change a result.** A hint can send a signature to libsecp, or make the batch fail. Valid bit 0, a
missing hint, or a hint failing the tests below means libsecp decides. Valid bit 1 on a signature libsecp would reject
means the batch equation fails. The weights are 128-bit, drawn from a hash of every (u1, u2, Q, R) in the batch
(`batch_ecdsa.rs:167-192`), so a false equation passes with probability about 2^-128 per attempt. The result of
each coprocessor point operation is asserted canonical (`batch_ecdsa.rs:273`, `:281`).

| check | libsecp accepts when | batch accepts when (`batch_ecdsa.rs`) | status | Tested |
|---|---|---|---|---|
| ECDSA | r, s ≠ 0 and x(u1·G + u2·Q) mod n == r (`ecdsa_impl.h`, including the `r + n` case). Core normalises s first | `:92-96` computes u1, u2 from the same scalars; R on the curve with canonical coordinates (`:114-118`); `x(R) mod n == r` (`:125`); u1·G + u2·Q == R in the batch | SAME. x < p < 2n makes `mod n` the same two cases libsecp tests. R at infinity has no affine hint, so it falls to libsecp, which rejects | `sig_p2wpkh`, `sig_p2wpkh_bad`, `sig_high_s`, `sig_not_der`, `sig_zero_r`, `sig_p2pkh`, `sig_p2pkh_other_key`, `multisig_ok`, `multisig_order`, `sig_must_fail`, `sig_must_fail_but_good` |
| BIP340 Schnorr | s < n, r_x < p, x on the curve; s·G − e·P has even y and x == r_x | Hook after those checks, with libsecp's own e; `rx == r` and even y (`:127`) | SAME | `taproot_key`, `taproot_key_bad`, `taproot_script`, `taproot_script_bad_sig` |
| BIP341 tweak | t < n; P + t·G has x == the output key and the committed parity | t ≥ n, x ≥ p or a bad parity value skip the hook (`0102`); `rx == r` and parity (`:129`) | SAME | `taproot_tweak_parity`, `taproot_script` |

## Soft-fork activation

Core's buried heights use `height >= value` (`deploymentstatus.h`). The flags go to `VerifyScript` from Core's own
`GetBlockScriptFlags` (`validation.cpp:2361`), lifted, through `hz_block_flags`, `core_block_rules.cpp:110`, and `main.rs:83`.
Its height table and exception table are Core's compiled chainparams, so `script_flags.rs` no longer decides anything in a
libzync image. Two Rust gates remain: `BIP34_HEIGHT` (`main.rs:332`, used at `agg_lite.rs:215`) and `NET.csv_height`
(`agg_lite.rs:121`, `:126`). `assert_core_constants` (`main.rs:291-316`) pins the first to Core at start-up. It does not pin
`NET.csv_height`: it compares `script_flags::CSV_HEIGHT` instead. The `NET` value equals Core's on mainnet, signet and regtest
(`libzync/src/net.rs:32`, `:50`, `:64`) by reading only.

| rule | Core | mainnet | signet | where enforced in the guest | status | Tested |
|---|---|---|---|---|---|---|
| BIP16 P2SH | Flag always on, off for one block | always; none at the 170,060 exception | always | **Core lifted** `GetBlockScriptFlags` | SAME | `sigops_p2sh_over`, `sigops_p2sh_under`, through the sigop count; `p2sh_redeem_fails` (redeem script fails), `p2sh_ok`. Exception: mainnet control |
| BIP141/143 script rules | Flag always on, off for one block | always; none at 170,060 | always | **Core lifted** `GetBlockScriptFlags` | SAME | `valid_p2wsh`, `sig_p2wpkh`, `sig_p2wpkh_bad`. Exception: mainnet control |
| BIP341/342 Taproot | Flag always on, off for two blocks | always; none at 170,060 and 692,261 | always | **Core lifted** `GetBlockScriptFlags` | SAME. `min_activation_height` 709,632 is not read by `GetBlockScriptFlags` | `taproot_key`, `taproot_key_bad`, `taproot_script`, `taproot_script_bad_sig`, `taproot_tweak_parity`. Exceptions: mainnet control |
| BIP34 | `BIP34Height` | 227,931 | 1 | **Core lifted** `ContextualCheckBlock`, `ContextualCheckBlockHeader` | SAME | `cb_wrong_height` |
| BIP66 DERSIG | `BIP66Height` | 363,725 | 1 | **Core lifted** `GetBlockScriptFlags`, `ContextualCheckBlockHeader` | SAME | `sig_not_der` |
| BIP65 CLTV | `BIP65Height` | 388,381 | 1 | **Core lifted** `GetBlockScriptFlags`, `ContextualCheckBlockHeader` | SAME | `bad_version`, the version gate only; `cltv_future` (`OP_CHECKLOCKTIMEVERIFY` unmet), `cltv_met` |
| BIP112 CSV | `CSVHeight` | 419,328 | 1 | **Core lifted** `GetBlockScriptFlags` | SAME | `csv_op_met`. `csv_op_unmet` is refused first by BIP68 (`bad-txns-nonfinal`), so a failing `OP_CHECKSEQUENCEVERIFY` is read only |
| BIP68 sequence locks | `CSVHeight` | 419,328 | 1 | **Core** `SequenceLocks`; **Rust** height gate `agg_lite.rs:126` | SAME | `bip68_*`, `split_tx_bad`, `csv_op_unmet` |
| BIP113 MTP cutoff | `CSVHeight` | 419,328 | 1 | **Rust** cutoff `agg_lite.rs:121` for non-coinbase transactions; **Core lifted** `ContextualCheckBlock` for the coinbase | SAME | `locktime_time_mtp`, `locktime_time_past` |
| BIP141 commitment | `SegwitHeight` | 481,824 | 1 | **Core lifted** `ContextualCheckBlock`, `CheckWitnessMalleation` | SAME | `witness_mismatch`, `nonce_too_big_witness`, `witness_no_commit` |
| BIP147 NULLDUMMY | `SegwitHeight` | 481,824 | 1 | **Core lifted** `GetBlockScriptFlags` | SAME | `multisig_dummy` |

Mainnet chainparams: `kernel/chainparams.cpp:87-104`. Signet: `:454-465`.

## What is still our own code

Everything that is not tagged **Core** or **Core lifted**. "Every case" means the code runs on the way to every rig verdict,
not that a case targets it.

| piece | where | tested by |
|---|---|---|
| Stand-ins for Core's coin view, block index, chainstate and clock | `core_tx_rules.cpp:35-86`, `core_block_rules.cpp:46-105` | Every case. `split_tx_bad` for a partial run, `bip68_time_*` for `GetAncestor`. The retarget's first block: read only |
| Block composition: walking the chunk journals, chunk linkage, input coverage, totals across chunks | `agg_lite.rs:251-317`, `:447-465` | `split_tx`, `split_tx_bad`, `cb_overpay`, `cb_fee_too_much`, `cb_exact`, `weight_*`, `sigops_*`. A journal that lies about coverage cannot be crafted from a block, so no rig case reaches those checks |
| Coin accounting and SwiftSync | `agg_lite.rs:339-378`, `swiftsync.rs` | `double_spend_in_block`, `spend_later_in_block`, `chain_in_block`. The rig stops at the block, so no case reaches `ss_set == 0`: `missing_coin` is deferred |
| BIP30 tree and tracked transactions | `bip30.rs`, `agg_lite.rs:212-217`, `:286`, `:407-436`, `hz_indicated_height` `verify_input.cpp:1028` | `bip30_repeat_unspent`, `bip30_repeat_spent`, `bip30_child_of_repeat`, the rig's staged base chain, and the unit tests in `bip30.rs` |
| Batch signature verifier and its libsecp hooks | `batch_ecdsa.rs`, `patches/libzync/0102` | `sig_*`, `multisig_*`, `taproot_*` |
| Host hints the guest checks | `liftx_hint.rs`, ECMH hints in `swiftsync.rs` | Every case with a signature or a coin. No case hands the guest a false hint |
| Parse cache and txid seeding | `cached_tx` `verify_input.cpp:219`, `hz_seed_hashes` `:201` | Every case. Non-canonical encodings: read only |
| Coin leaves | `verify_input.cpp:69`, `:94`, `:340` | Every spend case. `premature` and `bip68_*` depend on the fields |
| Wrappers round Core calls | `check_pow`, `merkle_root`, `coinbase_value`, `is_coinbase_tx`, `check_tx`, `hz_check_signet` in `verify_input.cpp` | `high_hash`, `bad_merkle`, `cb_overpay`, `cb_multiple`, `cb_script_short`. `hz_check_signet`: read only |
| Rust height gate and lock cutoff | `agg_lite.rs:121`, `:126` | `locktime_time_*`, `bip68_*`. CSV is at height 1 on regtest, so the boundary is read only |
| Chain linkage and state | `main.rs:2079-2139`: previous receipt, prevhash, height, MTP window, epoch start, chainwork (`add_work`) | `time_equal_mtp`, `bad_bits`. The rest: read only |
| Verifier | `libzync_verify_cmd`, host `main.rs:2015` | Not in the rig |
| Network constants | `libzync/src/net.rs` | Regtest's values in every case. Mainnet's and signet's: `assert_core_constants`, and by reading for `csv_height` |
| Header shims | `coreshim/logging.h`, `sync.h`, `threadsafety.h`: they replace Core's own headers in every Core file compiled | Every case. They drop logging and locks; a lifted function's `LogPrintf` and `AssertLockHeld` compile to nothing |

## Rules left out on purpose

| rule | why it is not in the tables |
|---|---|
| `time-too-new` | Depends on the node's clock. Core's test is lifted with `ContextualCheckBlockHeader` but its stand-in clock never reads past a block (`core_block_rules.cpp:82`). The guest commits the largest block time (`main.rs:2157`); the verifier applies the 2-hour rule to it (§5 step 11, host `main.rs:2049`), as Core does for every block during IBD |
| Best chain, `too-little-chainwork` | A proof says a chain is valid, not that it has the most work: §5 step 7. `cum_work` is public in the journal (`add_work`, `verify_input.cpp:978`, transcribes `GetBlockProof`) |
| `bad-fork-prior-to-checkpoint` | Core's checkpoints, last at 295,000 on mainnet, are node-level anti-DoS. Lifted, and off in the stand-in (`core_block_rules.cpp:62`). Covered in effect by §5 step 7 |
| `duplicate`, `inconclusive-*`, `BLOCK_MUTATED` retention | Node bookkeeping for blocks already seen, RPC strings, and not caching a mutated block as invalid |
| `non-mandatory-script-verify-flag` | Mempool only. Every block flag is in `MANDATORY_SCRIPT_VERIFY_FLAGS` (`policy/policy.h:89`), so a block check can never produce it |
| Core's `assumevalid` | A node setting. The guest's equivalent is the assumevalid image, stated in `profile` |
| Mempool, relay and P2P rules | Not consensus. This covers every string `MemPoolAccept` raises in `validation.cpp` |
| testnet3, testnet4, regtest, custom signets | The guest proves mainnet and default signet only (`libzync/src/net.rs`). The regtest build exists for the rig and carries its own profile bit (`main.rs:1412`) |

## Patches on consensus paths

None of these changes a rule. Each is a cheaper way to compute the same bytes or points. The tags above say "Core patched"
wherever one applies. A hash mistake fails blocks rather than passing them, because txids, wtxids and sighashes all feed
the merkle root, the commitment or a signature.

| where | patch | what changes |
|---|---|---|
| `crypto/sha256.cpp` | 0002, 0009, 0010, 0101 | SHA-256 and `SHA256D64` through the zkVM accelerator |
| `primitives/transaction.{h,cpp}` | 0101 | Txid and wtxid computed lazily, or seeded from raw bytes when the parse used every byte (`verify_input.cpp:201`, `:233`) |
| `hash.h` | 0101 | `HashWriter` stages small writes in a 512-byte buffer; same bytes hashed |
| `serialize.h` | 0001 | `int` overloads for ILP32, where `int32_t` is `long` |
| `pubkey.cpp` | 0101 | Memo of parsed keys, full key compared |
| `script/interpreter.cpp` | 0101 | HASH160 memo for 33-byte items, full key compared; SIGHASH_ALL legacy sighash from preserialised slices, with the cache keyed on a per-transaction generation |
| libsecp field | 0012 | Field backend on the coprocessor |
| libsecp `group_impl.h` | 0013 | Pubkey y from a hint, verified by y² == x³ + 7; a bad hint falls back to the sqrt |
| libsecp `ecdsa_impl.h`, schnorrsig | 0005, 0006, 0008 | On libsecp's fallback path: the double scalar multiplication and scalar inverse through `risc0-crypto`, whose results are constrained and asserted canonical |
| libsecp ecdsa, schnorrsig, extrakeys | 0102 | The batch hooks above |
