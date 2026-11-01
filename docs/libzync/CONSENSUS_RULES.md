# Consensus rules: the libzync guest against Bitcoin Core v28.0

**For:** outside reviewers of the proposal in [`SWIFTSYNC_PROOFS.md`](SWIFTSYNC_PROOFS.md).

**Read at:** repo commit `278c239cd` (branch `libzync`), which holds the same guest inputs as `c1101a3`, the source of
the round-8 image ids in `reproduce/LIBZYNC_IDS`. Bitcoin Core v28.0 `110183746` and libsecp256k1 v0.5.1 `642c885`, with
the patches `scripts/libzync-base.sh` applies. The Core and libsecp files those patches touch were checked byte for byte
against pristine files plus the patches.
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

Counted for an image that checks scripts. Rows marked † are SAME only once the proof is closed at H (below).

### Every row that is not SAME

A STRICTER row costs liveness: no proof exists for a chain Core accepts. It cannot make a proof claim something false.
A NOT ENFORCED row is the one that matters for soundness: a proof can exist for a chain Core rejects.

| row | status | what it means for a proof |
|---|---|---|
| `bad-txns-BIP30`, non-coinbase transactions | **SAME by construction** (fixed, #26) | No path checks these transactions directly. A non-coinbase transaction can repeat only if its inputs repeat, which traces back to a repeated coinbase, and the guest refuses every repeated coinbase except the two grandfathered ones. The older coin at each of those is never spendable here, so its descendants cannot exist. |
| `bad-txns-BIP30`, coinbases | **STRICTER** (#26) | The tree refuses any repeat of an earlier coinbase at every height, spent or not, except at 91,842 and 91,880. Core refuses only a repeat with unspent outputs, and skips the check from 227,932 to 1,983,701 on a chain through the BIP34 block. Measured on mainnet: of all coinbases to height 230,000 plus block 490,897, exactly two txids repeat, and they are those two blocks. The next height where BIP34 allows a repeat is 1,983,702, so a proof could stall there, about 19 years away, if a miner repeats an old coinbase. |
| Spending the coinbase coins of 91,722 and 91,812 | **STRICTER** (known) | These coins are never indexed, so a spend of one fails at H. Core allows the spend on a chain that holds the block but has not overwritten its coinbase. Mainnet never spent them. Only a fork reaches this, and no proof can be made for that fork |
| `bad-cb-amount` | **STRICTER** (new, harmless) | The guest also requires subsidy + fees ≤ `MAX_MONEY`. Core does not check this. No chain reaches it: fees cannot exceed the coins already issued, and all subsidies together are below `MAX_MONEY` |
| `time-timewarp-attack` | **N-A** | testnet4 and regtest only |

**Assumevalid images only** (`HAZYNC_LIBZYNC=assumevalid`, `ChainState.profile` bit 1): these rows become NOT ENFORCED:
`mandatory-script-verify-flag-failed`, `block-validation-failed`, the three signature rows, and the script side of the
soft-fork table. The proof says so itself (§5 step 9). BIP325 stays enforced there, because it runs in the aggregate.

### Rows marked †: true only at H

A block's spent coins are prover-supplied. A spend names a coin by its leaf, and nothing checks that coin exists until
the ECMH total is the identity at H (`ss_set == 0`, §5 step 5). Everything computed from a prevout therefore rests on
that final check: value, fee, maturity, BIP68, P2SH and witness sigops, and the script being satisfied.

- A proof that passes the whole of §5 establishes these rows, under the ECMH collision assumption of §4.
- Core rejects the bad block itself. The guest may reject it only at H, without saying which block failed.
- A receipt not closed at H establishes none of them (§6a).

## How to read the "where enforced" column

The guest compiles these Core sources (`prover/methods/guest/build.rs:311-324`, plus `signet.cpp` on signet at `:384`):
`script/interpreter.cpp`, `script/script.cpp`, `primitives/transaction.cpp`, `primitives/block.cpp`, `pubkey.cpp`,
`hash.cpp`, the `crypto/` hashes, `consensus/tx_check.cpp`, `consensus/merkle.cpp`, `arith_uint256.cpp`, `pow.cpp`,
`chain.cpp`, `kernel/chainparams.cpp`.

It does **not** compile `validation.cpp`, `consensus/tx_verify.cpp` or `coins.cpp`. So `CheckBlock`, `ContextualCheckBlock`,
`ConnectBlock`, `CheckTxInputs`, `IsFinalTx`, `SequenceLocks` and `GetTransactionSigOpCost` are transcriptions, even
where §2 of the proposal says "Core's own C++". The tags say which is which:

| tag | meaning |
|---|---|
| **Core** | Core or libsecp source, compiled unchanged |
| **Core patched** | The same, with a patch from `patches/` in that file (listed at the end) |
| **Glue** | Hand-written C++ in `prover/methods/guest/verify_input.cpp`, using Core's types: a transcription |
| **Rust** | Guest Rust, `prover/methods/guest/src/` |
| **Verifier** | Outside the guest: `libzync_verify_cmd`, host `main.rs:2006` |

Paths: `agg_lite.rs`, `main.rs`, `script_flags.rs`, `batch_ecdsa.rs` are in `prover/methods/guest/src/`; `verify_input.cpp` is
in `prover/methods/guest/`; "host `main.rs`" is `prover/host/src/main.rs`; `validation.cpp`, `pow.cpp`, `signet.cpp`,
`coins.cpp`, `tx_check.cpp`, `tx_verify.cpp`, `transaction.h`, `chain.h` and `chainparams.cpp` are Core's.

**How a result takes effect.** A chunk folds every per-transaction result into `all_valid` (`main.rs:1852`, `:1856`, `:1922`)
and commits it (`:1930`). The aggregate requires it (`agg_lite.rs:259`), gathers its own flags (`main.rs:2063`) and asserts
them all (`main.rs:2076`). A failed assert means no receipt. Every chunk result is tied to the block by height, block
hash, MTP and time (`agg_lite.rs:263`), and to the block's transactions by the merkle root and witness commitment.

## Header and chain

| reject reason | rule | where enforced in the guest | status |
|---|---|---|---|
| `high-hash` | `nBits` decodes to a target that is positive, not overflowing and ≤ `powLimit`; block hash ≤ target. `validation.cpp:3913`, `pow.cpp:137` | **Glue** `check_pow`, `verify_input.cpp:598`; called `agg_lite.rs:381` | SAME. Mainnet compares against `SetCompact(0x1d00ffff)`, not Core's `powLimit` (2^224 − 1). The numbers differ, but no compact value decodes between them, so the accepted `nBits` are identical. Signet uses Core's `powLimit` directly |
| `bad-diffbits` | `nBits` equals `GetNextWorkRequired`: the parent's value inside an epoch, a retarget at each 2,016th height over the first and last timestamps of the epoch. `validation.cpp:4168`, `pow.cpp:13` | **Rust** `main.rs:2067-2068` selects; **Core** `CalculateNextWorkRequired` (`pow.cpp:49`) through glue `calc_next_bits`, `verify_input.cpp:929`. Epoch start `main.rs:2081` | SAME. Mainnet and signet both have `fPowAllowMinDifficultyBlocks = false` and `fPowNoRetargeting = false` (`chainparams.cpp:102-104`, `:463-465`), so the min-difficulty branch, absent from the guest, is dead on both. Same first block as Core: height − 2,016 |
| `time-too-old` | Block time > median of the last 11 timestamps ending at the parent. `validation.cpp:4184`, `chain.h:278` | **Rust** `main.rs:2069`; median `main.rs:1421`; window `main.rs:2082-2084` | SAME. Same index `n/2`, same behaviour with fewer than 11 blocks |
| `bad-version(0x%08x)` | Version < 2 from BIP34, < 3 from BIP66, < 4 from BIP65, compared as signed `int32_t`. `validation.cpp:4204-4208` | **Rust** `agg_lite.rs:207-212` | SAME. Signed on both sides. Heights from `NET`, pinned to Core at start-up (`main.rs:259`) |
| `time-timewarp-attack` | Testnet4 and regtest only (`enforce_BIP94`). `validation.cpp:4188-4193` | None | N-A. `enforce_BIP94 = false` on mainnet and signet |
| `prev-blk-not-found` | The parent is known. `validation.cpp:4307` | **Rust** `main.rs:2064`: `hashPrevBlock` equals the verified previous state's tip | SAME |
| `bad-prevblk` | The parent is not invalid. `validation.cpp:4312`, `:4353` | **Rust** `main.rs:2034`: the previous state is a verified receipt, so an invalid parent has no receipt to build on | SAME |
| `bad-signet-blksig` | Signet: the block solution satisfies the challenge. `validation.cpp:4012-4013`, `signet.cpp:124` | **Core** `signet.cpp` through glue `hz_check_signet`, `verify_input.cpp:901`; called `agg_lite.rs:396-404` | SAME on signet, no such rule on mainnet. Non-coinbase transactions are placeholders with their txid seeded from the list the merkle check binds; `signet.cpp:57-64` reads only `GetHash()` from them. The signature runs through libsecp's own path: the aggregate installs no batch hints (`install` is called only at `main.rs:1753`) |

## Block structure: `CheckBlock`, `ContextualCheckBlock`

| reject reason | rule | where enforced in the guest | status |
|---|---|---|---|
| `bad-txnmrklroot` | The header's merkle root equals the root over the txids. `validation.cpp:3925` | **Core** `ComputeMerkleRoot` (`consensus/merkle.cpp:45`, hash primitive patched) through glue `merkle_root`, `verify_input.cpp:680`; compared `agg_lite.rs:385`. The list is the coinbase txid, then each transaction's txid in chunk order (`agg_lite.rs:242`, `:278`) | SAME |
| `bad-txns-duplicate` | No mutation of the tree (CVE-2012-2459). `validation.cpp:3935` | **Core** `mutated` flag, `agg_lite.rs:385` | SAME. The guest also refuses any two equal txids (`agg_lite.rs:407-411`). That rejects nothing new: Core refuses them as a double spend, and a transaction with no inputs fails `bad-txns-vin-empty` |
| `bad-blk-length` | At least one transaction; transaction count × 4 and stripped size × 4 ≤ 4,000,000. `validation.cpp:4028-4029` | **Rust**, implied by `agg_lite.rs:461` | SAME. No explicit check, none needed: weight ≥ 4 × stripped size, and each transaction is at least 60 bytes, so anything over either bound is already over the weight limit. The coinbase is always present |
| `bad-cb-missing` | The first transaction is a coinbase. `validation.cpp:4033` | **Glue** `is_coinbase_tx` (`verify_input.cpp:729`), `agg_lite.rs:443-444`. Its txid is the first merkle leaf | SAME |
| `bad-cb-multiple` | No other transaction is a coinbase. `validation.cpp:4036` | **Rust** `agg_lite.rs:99`, on every run of every transaction | SAME |
| `bad-blk-sigops` (legacy) | Legacy sigops × 4 ≤ 80,000. `validation.cpp:4055-4056` | **Rust**, implied by `agg_lite.rs:462` | SAME. The ConnectBlock total below starts from the same legacy × 4 for every transaction, so it can only be larger |
| `bad-txns-nonfinal` (nLockTime) | Every transaction, coinbase included, is final at this height and cutoff. The cutoff is the parent's MTP from CSV activation, the block time before it. `validation.cpp:4228-4238`, `tx_verify.cpp:17` | **Rust** cutoff `agg_lite.rs:112`, `:206`; **Glue** `is_final_tx`, `verify_input.cpp:417`; called `agg_lite.rs:113`, coinbase `:463-466` | SAME. `is_final_tx` is a line-for-line transcription of `IsFinalTx`. Activation matches: `height >= csv_height` is `DeploymentActiveAfter` |
| `bad-cb-height` | From BIP34, the coinbase scriptSig starts with `CScript() << height`. `validation.cpp:4242-4248` | **Glue** `check_bip34`, `verify_input.cpp:665`, using Core's `CScript` serialiser; `agg_lite.rs:405` | SAME. Same push encoding, including `OP_1`..`OP_16` for low signet heights. Mainnet 227,931, signet 1 |
| `unexpected-witness` | No transaction has a witness unless the block carries a valid commitment. `validation.cpp:3989` | **Rust** `agg_lite.rs:392` before segwit; **Glue** `check_witness_commitment` returns −1, `verify_input.cpp:636`, after it. `has_witness` covers the coinbase (`agg_lite.rs:244`) and every transaction (`:280`) | SAME |
| `bad-witness-nonce-size` | With a commitment, the coinbase witness is one 32-byte item. `validation.cpp:3962` | **Glue** `verify_input.cpp:643`; `agg_lite.rs:388-390` | SAME. Checked whenever a commitment output exists, even with no witness in the block, as in Core |
| `bad-witness-merkle-match` | The last matching output (at least 38 bytes, prefix `6a24aa21a9ed`) holds SHA256d(witness root ‖ reserved value). `validation.cpp:3975` | **Glue** `verify_input.cpp:629-656`; **Core** `ComputeMerkleRoot`. Wtxids from the chunks (`agg_lite.rs:279`), coinbase leaf zero (`:246`) | SAME. Output search transcribes `GetWitnessCommitmentIndex`. Every run of a transaction commits the wtxid and they must agree (`agg_lite.rs:292`) |
| `bad-blk-weight` | Block weight ≤ 4,000,000, at every height. `validation.cpp:4271` | **Rust** `agg_lite.rs:454-461`; **Glue** per-transaction weight `tx_wu_sigops`, `verify_input.cpp:785`, called `agg_lite.rs:115`, with Core's `GetSerializeSize` | SAME. Sum of `3×stripped + total` plus `4 × (80 + CompactSize(n))`, which equals `GetBlockWeight` |

## Transactions: `CheckTransaction`

All nine run Core's `consensus/tx_check.cpp`, unchanged, through glue `check_tx` (`verify_input.cpp:577`). The chunk holding
input 0 runs it for each non-coinbase transaction (`agg_lite.rs:111`); the aggregate runs it for the coinbase (`:439-444`).

| reject reason | rule | where enforced in the guest | status |
|---|---|---|---|
| `bad-txns-vin-empty` | At least one input | **Core** `tx_check.cpp:15` | SAME |
| `bad-txns-vout-empty` | At least one output | **Core** `tx_check.cpp:17` | SAME |
| `bad-txns-oversize` | Stripped size × 4 ≤ 4,000,000 | **Core** `tx_check.cpp:20` | SAME. On rv32 this `size_t` product wraps at 2^30 bytes. A transaction that large still fails `bad-blk-weight`, which is computed in 64 bits |
| `bad-txns-vout-negative` | No negative output value | **Core** `tx_check.cpp:28` | SAME |
| `bad-txns-vout-toolarge` | No output above `MAX_MONEY` | **Core** `tx_check.cpp:30` | SAME |
| `bad-txns-txouttotal-toolarge` | Output total stays in range | **Core** `tx_check.cpp:33` | SAME |
| `bad-txns-inputs-duplicate` | No repeated input (CVE-2018-17144) | **Core** `tx_check.cpp:44` | SAME |
| `bad-cb-length` | Coinbase scriptSig is 2 to 100 bytes | **Core** `tx_check.cpp:50` | SAME |
| `bad-txns-prevout-null` | No null prevout outside a coinbase | **Core** `tx_check.cpp:56` | SAME |

## Coins and block totals: `ConnectBlock`, `CheckTxInputs`

| reject reason | rule | where enforced in the guest | status |
|---|---|---|---|
| `bad-txns-BIP30` (coinbase) | A coinbase may not create an outpoint that is still unspent. Skipped at 91,842 and 91,880, and from 227,932 to 1,983,701 on a chain through the BIP34 block | **Rust** `coinbase-smt/src/bip30.rs` `apply_block`: absence proof against the incoming root; a fully spent coinbase stays in the tree as `SPENT`, so absence fails for it too | STRICTER |
| `bad-txns-BIP30` (non-coinbase) | A non-coinbase transaction may not create an outpoint that is still unspent. Same conditions | Follows from the coinbase row: a repeat needs repeated inputs, which needs a repeated coinbase | SAME (by construction) |
| `bad-txns-inputs-missingorspent` † | Every input spends an unspent coin. For a coin made in this block: an earlier transaction made it. `tx_verify.cpp:168` | **Rust**: unknown or already spent coin: `swiftsync::sub`, `agg_lite.rs:363`, then **Verifier** `ss_set == 0`, host `main.rs:2062`. Same coin twice in a block: `agg_lite.rs:337-344`. In-block order and single spend: `:349-350`. Coin claiming a later height: `:136`. Coin claiming this height that this block does not create: `:359-362` | SAME † |
| `bad-txns-premature-spend-of-coinbase` † | A coinbase coin waits 100 blocks. `tx_verify.cpp:180` | **Rust** `agg_lite.rs:141`, flag exactly 0 or 1 at `:139`; height and flag bound by the leaf (`verify_input.cpp:94`) | SAME †. `h < c + 100` is Core's `h − c < 100`. Signet is also 100 |
| `bad-txns-inputvalues-outofrange` † | Each input value and the running sum stay in `MoneyRange`. `tx_verify.cpp:187` | **Glue** `check_tx`, `verify_input.cpp:582-586` | SAME † |
| `bad-txns-in-belowout` † | Inputs ≥ outputs. `tx_verify.cpp:193` | **Glue** `verify_input.cpp:588` | SAME † |
| `bad-txns-fee-outofrange` † | Fee in `MoneyRange`. `tx_verify.cpp:200` | **Glue**, implied by `verify_input.cpp:582-588` | SAME †. No explicit check: inputs are in range, outputs ≥ 0 and inputs ≥ outputs, so the fee is in range |
| `bad-txns-accumulated-fee-outofrange` † | The block's running fee stays in `MoneyRange`. `validation.cpp:2632` | **Rust** `agg_lite.rs:281`, `:448` | SAME †. Checks the final sum, not each step. Every fee is ≥ 0, so the sum only grows and the two agree |
| `bad-txns-nonfinal` (BIP68) † | From CSV, for version ≥ 2 and disable bit clear, each input's relative lock has passed. `validation.cpp:2645`, `tx_verify.cpp:39-108` | **Rust** `agg_lite.rs:146-150` | SAME †. Core's `coin + n − 1 >= height` and `coinMTP + (n << 9) − 1 >= MTP` rearrange to the guest's tests. Version is unsigned on both sides (`transaction.h:308`). The coin's MTP is the one `GetAncestor(coin − 1)` gives, and the leaf commits it. A coin made in this block uses this block's parent MTP, as in Core |
| `bad-blk-sigops` (cost) † | Sum of legacy × 4 + P2SH × 4 + witness sigops ≤ 80,000; coinbase legacy only. `validation.cpp:2656`, `tx_verify.cpp:145` | **Glue** `tx_full_sigops`, `verify_input.cpp:806`, using **Core** `GetSigOpCount` and `CountWitnessSigOps`; summed `agg_lite.rs:283`, `:457`, `:462` | SAME †. Transcribes `GetTransactionSigOpCost`, same flags, same P2SH guard |
| `bad-cb-amount` † | Coinbase value ≤ fees + subsidy. `validation.cpp:2689-2692` | **Rust** `agg_lite.rs:445-452`; **Glue** `coinbase_value` and `block_subsidy`, `verify_input.cpp:692`, `:956` | STRICTER, unreachable. The limit itself is the same, and `block_subsidy` transcribes `GetBlockSubsidy` (`validation.cpp:1919`). The guest adds subsidy + fees ≤ `MAX_MONEY` |
| `mandatory-script-verify-flag-failed (…)` † | Every input's script passes `VerifyScript` with the block's flags. `validation.cpp:2208` | **Core patched** `script/interpreter.cpp` through glue `verify_inputs_batch_once`, `verify_input.cpp:1109-1120`; `agg_lite.rs:131` | SAME †. Same flags (table below). The interpreter's own script errors are not listed: the file is compiled with only the hunks at the end of this document |
| `block-validation-failed` † | The same failure, as reported with parallel script threads. `validation.cpp:2697` | As above | SAME † |

## Rules with no reject string

| rule | Core | where enforced in the guest | status |
|---|---|---|---|
| Transaction parsing | `transaction.h:216-254`: canonical CompactSize, superfluous witness and unknown flag rejected. A failed parse means no block | **Core patched** `UnserializeTransaction` (`serialize.h` gains 32-bit `int` overloads; parsing unchanged), through glue `cached_tx`, `verify_input.cpp:219`. A failed parse aborts: no proof | SAME |
| Block height | From the block index | **Rust** `main.rs:2048`: exactly previous height + 1 | SAME |
| Genesis and its coinbase | `validation.cpp:2451` skips genesis's transactions; its coin never exists | **Verifier** §5 step 3, host `main.rs:2050`: the proof must start from `genesis_anchor`, host `main.rs:1271`, with an empty forest and `recent_times = [genesis time]`. The guest commits the digest of its base state at `main.rs:2051` | SAME |
| Unspendable outputs never become coins | `coins.cpp:72`: OP_RETURN, or script over 10,000 bytes | **Core** `IsUnspendable` in glue `tx_out_leaves`, `verify_input.cpp:358` | SAME |
| Grandfathered coinbases 91,842 and 91,880 | `validation.cpp:6304`, height and hash | **Rust** `agg_lite.rs:215`, `:417`; hashes `main.rs:302-303` (checked equal to Core's) | SAME. The hash alone fixes the height |
| Spending the coinbase coins of 91,722 and 91,812 | Ordinary coins until overwritten. `IsBIP30Unspendable` (`validation.cpp:6310`) is not consensus code: only `coinstatsindex.cpp` calls it | **Rust** `agg_lite.rs:231`: never indexed, so a spend fails at H | STRICTER. Same final set as Core on the real chain, where neither was spent. Keyed on height and hash, so signet is unaffected |
| 64-byte transactions | No consensus rule in v28. `IsBlockMutated` (`validation.cpp:4109`) is called only from `net_processing.cpp`, and tests for them only in a block with no coinbase | None | SAME. Neither side rejects them. The txid list a proof uses is bound by the merkle root, with the same inner-node ambiguity Core has |

## Signatures

Each check is hooked in libsecp after its own parsing and range checks (`patches/libzync/0102`). If the hook returns 0, libsecp
verifies as usual (**Core patched**). If it returns 1, the check joins the chunk's batch, `finish()` runs before the chunk
commits (`main.rs:1922`), and a failure clears `all_valid`.

**The hint cannot change a result.** A hint can send a signature to libsecp, or make the batch fail. Valid bit 0, a
missing hint, or a hint failing the tests below means libsecp decides. Valid bit 1 on a signature libsecp would reject
means the batch equation fails. The weights are 128-bit, drawn from a hash of every (u1, u2, Q, R) in the batch
(`batch_ecdsa.rs:167-192`), so a false equation passes with probability about 2^-128 per attempt. The result of
each coprocessor point operation is asserted canonical (`batch_ecdsa.rs:273`, `:281`).

| check | libsecp accepts when | batch accepts when (`batch_ecdsa.rs`) | status |
|---|---|---|---|
| ECDSA | r, s ≠ 0 and x(u1·G + u2·Q) mod n == r (`ecdsa_impl.h`, including the `r + n` case). Core normalises s first | `:92-96` computes u1, u2 from the same scalars; R on the curve with canonical coordinates (`:114-118`); `x(R) mod n == r` (`:125`); u1·G + u2·Q == R in the batch | SAME. x < p < 2n makes `mod n` the same two cases libsecp tests. R at infinity has no affine hint, so it falls to libsecp, which rejects |
| BIP340 Schnorr | s < n, r_x < p, x on the curve; s·G − e·P has even y and x == r_x | Hook after those checks, with libsecp's own e; `rx == r` and even y (`:127`) | SAME |
| BIP341 tweak | t < n; P + t·G has x == the output key and the committed parity | t ≥ n, x ≥ p or a bad parity value skip the hook (`0102`); `rx == r` and parity (`:129`) | SAME |

## Soft-fork activation

Core's buried heights use `height >= value` (`deploymentstatus.h`). Rust values are in `script_flags.rs:26-43` and `libzync/src/net.rs`,
and `assert_core_constants` (`main.rs:259-285`) compares them with Core's compiled chainparams, so a wrong value aborts the guest.
The flags go to `VerifyScript` from `block_script_flags`, `script_flags.rs:55`, against Core's `GetBlockScriptFlags`,
`validation.cpp:2361`.

| rule | Core | mainnet | signet | guest | status |
|---|---|---|---|---|---|
| BIP16 P2SH | Flag always on, off for one block | always; none at the 170,060 exception | always | `script_flags.rs:56-57` | SAME. Exception hash equal to Core's |
| BIP141/143 script rules | Flag always on, off for one block | always; none at 170,060 | always | `script_flags.rs:56-57` | SAME |
| BIP341/342 Taproot | Flag always on, off for two blocks | always; none at 170,060 and 692,261 | always | `script_flags.rs:56-58` | SAME. Exception hash equal to Core's. `min_activation_height` 709,632 is not read by `GetBlockScriptFlags`, so the comment at `script_flags.rs:49-54` calling this stricter than Core is out of date |
| BIP34 | `BIP34Height` | 227,931 | 1 | `agg_lite.rs:208`, `verify_input.cpp:667` | SAME |
| BIP66 DERSIG | `BIP66Height` | 363,725 | 1 | `script_flags.rs:59`, `agg_lite.rs:209` | SAME |
| BIP65 CLTV | `BIP65Height` | 388,381 | 1 | `script_flags.rs:60`, `agg_lite.rs:210` | SAME |
| BIP112 CSV | `CSVHeight` | 419,328 | 1 | `script_flags.rs:61` | SAME |
| BIP68 sequence locks | `CSVHeight` | 419,328 | 1 | `agg_lite.rs:146` | SAME |
| BIP113 MTP cutoff | `CSVHeight` | 419,328 | 1 | `agg_lite.rs:112`, `:206` | SAME |
| BIP141 commitment | `SegwitHeight` | 481,824 | 1 | `agg_lite.rs:388` | SAME |
| BIP147 NULLDUMMY | `SegwitHeight` | 481,824 | 1 | `script_flags.rs:62` | SAME |

Mainnet chainparams: `kernel/chainparams.cpp:87-104`. Signet: `:454-465`.

## Rules left out on purpose

| rule | why it is not in the tables |
|---|---|
| `time-too-new` | Depends on the node's clock. The guest commits the largest block time (`main.rs:2102`); the verifier applies the 2-hour rule to it (§5 step 11, host `main.rs:2040`), as Core does for every block during IBD |
| Best chain, `too-little-chainwork` | A proof says a chain is valid, not that it has the most work: §5 step 7. `cum_work` is public in the journal (`add_work`, `verify_input.cpp:940`, transcribes `GetBlockProof`) |
| `bad-fork-prior-to-checkpoint` | Core's checkpoints, last at 295,000 on mainnet, are node-level anti-DoS. Covered in effect by §5 step 7 |
| `duplicate`, `inconclusive-*`, `BLOCK_MUTATED` retention | Node bookkeeping for blocks already seen, RPC strings, and not caching a mutated block as invalid |
| `non-mandatory-script-verify-flag` | Mempool only. Every block flag is in `MANDATORY_SCRIPT_VERIFY_FLAGS` (`policy/policy.h:89`), so a block check can never produce it |
| Core's `assumevalid` | A node setting. The guest's equivalent is the assumevalid image, stated in `profile` |
| Mempool, relay and P2P rules | Not consensus. This covers every string `MemPoolAccept` raises in `validation.cpp` |
| testnet3, testnet4, regtest, custom signets | The guest builds for mainnet and default signet only (`libzync/src/net.rs`) |

## Patches on consensus paths

None of these changes a rule. Each is a cheaper way to compute the same bytes or points. The tags above say "Core patched"
wherever one applies. A hash mistake fails blocks rather than passing them, because txids, wtxids and sighashes all feed
the merkle root, the commitment or a signature.

| where | patch | what changes |
|---|---|---|
| `crypto/sha256.cpp` | 0002, 0009, 0010, 0101 | SHA-256 and `SHA256D64` through the zkVM accelerator |
| `primitives/transaction.{h,cpp}` | 0101 | Txid and wtxid computed lazily, or seeded from raw bytes when the parse used every byte (`verify_input.cpp:201`, `:233`) |
| `serialize.h` | 0001 | `int` overloads for ILP32, where `int32_t` is `long` |
| `pubkey.cpp` | 0101 | Memo of parsed keys, full key compared |
| `script/interpreter.cpp` | 0101 | HASH160 memo for 33-byte items, full key compared; SIGHASH_ALL legacy sighash from preserialised slices, with the cache keyed on a per-transaction generation |
| libsecp field | 0012 | Field backend on the coprocessor |
| libsecp `group_impl.h` | 0013 | Pubkey y from a hint, verified by y² == x³ + 7; a bad hint falls back to the sqrt |
| libsecp `ecdsa_impl.h`, schnorrsig | 0005, 0006, 0008 | On libsecp's fallback path: the double scalar multiplication and scalar inverse through `risc0-crypto`, whose results are constrained and asserted canonical |
| libsecp ecdsa, schnorrsig, extrakeys | 0102 | The batch hooks above |
