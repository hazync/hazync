# zAPP regtest harness

The shared regtest chain of `docs/libzync/ZAPP_TESTING.md` §3.2 (branch `docs/zapp-testing`): what mainnet cannot give
soon (spends under a real proof, same-block spends, a BIP30 repeat, a real 64-byte transaction, coins under keys we
hold, the OK verdict, reorgs). It is on the merge gate of #726 (block check), #740 (assume), #741 (liveness), #742
(statement) and #743 (deposit). This directory holds the chain, the block files the libzync host reads, Core's oracle
values, the dev-mode execution and the prepared (not run) GPU batch. The zAPP runner (`run.sh`, plan §3.2 "The run")
is a later step: it needs the `regtest-test` image in `zapps/corecheck` (below).

| file | what | needs |
|---|---|---|
| `gen_chain.py` | builds the chain with Bitcoin Core's own `test_framework`, every block submitted to a Core node, which must accept it | Core 28.0 `bitcoind`, a Core v28.0 tree |
| `export.py` | replays `chain/raw.txt` into a fresh node; writes `blocks/` (via `prover/fetch_block_rpc.py`) and `oracle/` from the node alone | `bitcoind` only |
| `regen.sh` | generator and export twice from nothing, refused unless byte-identical; installs `chain/ blocks/ oracle/ SHA256SUMS` | both |
| `check.sh` | **the CI path**: replay, every block file and oracle file must equal the committed one (sha-pinned Core 28.0 tarball) | `bitcoind`, Python 3 |
| `execute.sh` | the chain through a regtest host in dev mode, staged as the GPU batch (A, a refused control, B) | a regtest host |
| `extract_guest.py` | the regtest guest program out of CI's `host_reg` (the regtest image is not published) | the host binary |
| `gpu/` | the GPU batch: `mkbundle.sh`, `gpu_run.sh` (⚠ spends money), `pod/` | see "GPU batch" |

Core is 28.0, as the CI rig (`adversarial.yml` libzync-rig: tarball sha256 `7fe294b0…`, `test_framework` from tag
v28.0), started with `-regtest -testactivationheight=bip34@110` (the regtest guest's BIP34 height, `libzync/src/net.rs`).

## The chain

115 blocks: main 1..110, branch **a** 111..112, branch **b** 111'..113' (one block more: the reorg). Keys are
`sha256("hazync zapp-regtest key " + name) mod n`, published on purpose (regtest only; `chain/scenarios.json` has every
secret, pubkey, script and regtest address). ECDSA is RFC 6979, Schnorr has `aux_rand` = 32 zero bytes, block times are
genesis + 600·h (+1 on b), so two runs give the same bytes (`regen.sh`).

| height | content (txid prefixes; full ids in `chain/scenarios.json`) | for |
|---|---|---|
| 1..101 | coinbase-only, coinbase h pays `MINER[(h-1) % 6]`: P2PK compressed, P2PK uncompressed, P2PKH, P2WPKH, P2SH-P2WPKH, P2TR | all |
| 2 | its coinbase's script reads as height **105** (`0169`): pre-BIP34, indicating a later height, so tracked and repeatable at 105 (`73821225…`) | assume |
| 102 | `bffc2f43` spends coinbase 1 (P2PK c) to `ALICE` keys of all six types, two OP_TRUE coins, a **zero-value** output (:8), a **10,001-byte script** (:9, unspendable by size: never a coin), change (:10, spent in the same block); `e9e18cfb` the **deposit**: 1.23456789 BTC to `DEPOSIT` P2WPKH + OP_RETURN `hazync-deposit-0102`, spending block 2's tracked coinbase + a same-block coin (so not a tracked tx); `c3b9b9fd` OP_RETURN in 8 forms: OP_0, a 75-byte push, PUSHDATA1 80, PUSHDATA2 300, PUSHDATA4 5 (`hello`), two pushes `ab` `cd`, OP_RETURN OP_1 (not data-only), bare OP_RETURN | statement, deposit, assume |
| 103 | same-block chains: `474b2e34` spends coinbase 3 (P2PKH); 1 hop `6a122bb8`; 3 hops `8b19a039` → `6ac0b829` → `204328df` | hints, liveness |
| 104 | **17 transactions** (2^4 + 1: odd, the last hash duplicated): fan-out `90b6708b` (coinbase 4, P2WPKH) with 40 outputs, 15 children | deposit, block check |
| 105 | block 2's coinbase again **byte for byte** (`73821225`, the first fully spent at 102; Core: BIP30 holds; the guest: REPEATABLE, the repeat tracked and live); a real **64-byte** non-witness transaction `5c692392` at position 1; P2PK c `aa970d97` and P2PKH `db028750` spends; no witness anywhere (the coinbase must be identical, so no commitment) | assume, deposit, liveness |
| 106..110 | spends at varied heights: 106 coinbase 5 (P2SH-P2WPKH) + coinbase 6 (P2TR key path) + ALICE P2SH-P2WPKH; 107 coinbase 7 (P2PK c); 108 ALICE P2TR key path; 109 coinbase 9 (P2PKH); L1 106→107, L2 107→108, L3 109→110; L4 = X created 109 | liveness, hints |
| a 111 | `29d336e4` deposit **D**: 2.5 BTC to `DEPOSIT` + OP_RETURN `hazync-deposit-A111`; `83c0dcf5` spends **X** (`e0b6ab75…:1`) | reorg tables |
| a 112 | `0218b468` (coinbase 12) | |
| b 111' | `d3ad698c` spends **Y** (`90b6708b…:0`); D absent, X unspent | reorg tables |
| b 112', 113' | coinbase-only; `079e8637` (coinbase 13) | |

Tips: 110 `2d5cc9c8…`, a 112 `67c1d2e7…`, b 113 `1b5db3da…`. Every script type is spent under the chain: P2PK c
(102, 105, 107, b 113), P2PK u (102), P2PKH (103, 105, 109), P2WPKH (103, 104, change), P2SH-P2WPKH (106, a 111), P2TR
key path (106, 108, a 112).

**One correction to the plan.** §3.2 has block 2's coinbase with *no* height push "so it can repeat". On this guest it
could not: an untracked coinbase, once spent, is `SPENT` in the BIP30 tree and a repeat is refused (`coinbase-smt`
`bip30.rs`: "there the guest is stricter than Core"), while Core would accept it. The harness chain must be one the
guest accepts, so block 2's coinbase indicates 105 (the shape of mainnet's tracked coinbases) and repeats there. The
stricter-than-Core case stays a rig case, not a harness block.

## Oracle (`oracle/`, from the node only)

- `snap_<101|104|105|110|a112|b113>.json`: the tip and its header; `gettxoutsetinfo hash_serialized_3` and `muhash`
  (`disk_size` dropped); `gettxout` for **every** output the active chain created (value, script, coinbase, creation
  height, or null); `scantxoutset raw(<spk>)` for every harness script (total, count, each coin); `dumptxoutset`'s
  summary and the file itself (`utxo_<label>.dat`, 6-10 kB).
- `blocks_<main|a|b>.json`: per block the header, txids in order, sizes, every input's spend (`prev` → `txid`, `vin`),
  and `gettxoutproof` for every transaction (each checked by `verifytxoutproof`).
- `chaintips.json` after the reorg.
- On the way, `export.py` checks the reorg: b 111 and 112 are `inconclusive` (stored, not connected), b 113 takes the
  tip; `invalidateblock` b 111 returns to a 112 and `reconsiderblock` to b 113, each snapshot equal to the first.

⚠ **`gettxout` must be asked with `include_mempool=false`.** After the reorg the disconnected branch's transactions are
back in the mempool, and the default (`true`) then reports a coin that is unspent on the active chain as spent: X read
`null` at b 113 because a's spend of it was pooled. Any zAPP CLI that asks a node `gettxout` (`--bitcoin-cli`) has the
same trap on a reorg test.

Values: `txouts` 101 / 169 / 170 / 177 / 181 / 180 and `total_amount` 5,050 / 5,199.99999 / 5,249.99999 / 5,499.99999 /
5,599.99999 / 5,649.99999 BTC at 101 / 104 / 105 / 110 / a 112 / b 113 (the missing 1,000 sat is the 10,001-byte
output). At 105 the repeated coinbase is one coin in Core's set (`gettxout` height 105).

## Determinism

`regen.sh` (16.11.2026, laptop): two generator runs from empty datadirs, `raw.txt` sha256 `3f7a199b82e5641e…` and
`scenarios.json` `d95435e10f2707e4…` both times; the two exports identical file for file (`diff -r`). `check.sh` replays
the committed chain and reproduces all 115 block files and 16 oracle files byte for byte; its control (one oracle value
changed by 1 sat, SHA256SUMS updated so only the replay can catch it) fails.

## Receipts (`receipts/`, the GPU batch's keep set, MEASURED 16.11.2026)

The real proofs of `gpu/RESULT_gpu_eb2cd424.txt` (RISC0_DEV_MODE unset, one RTX PRO 6000), as plan §3.2 keeps them,
1.4 MB, every file in `receipts/SHA256SUMS` (`cd receipts && sha256sum -c SHA256SUMS`): `a/block_<104|105|110|112>.rcpt`
and `b/block_113.rcpt` (succinct core receipts, image `eb2cd424…`), each run's `history.bin`, and `state_<h>.json` /
`.coinlog` / `.trie` at those heights; `verify_a.log` / `verify_b.log` (the host's `libzync-verify`, LIBZYNC_VERIFY_OK
both), `times.tsv`, `anchor_effective.json`, `card.txt`, `method_id.txt`. The zAPPs' OK-verdict tests read them with the
oracle snapshots above as the independent answer (each zAPP's `test.sh`, the `regtest-test` feature below).

## What the `regtest-test` feature in `zapps/corecheck` must do (not built here)

`zapps/corecheck` and `zapps/common` are being changed elsewhere, so this is the specification, not the code. Every
value below is measured on this chain (see "Execution").

1. **A default-off feature** `regtest-test = []` in `zapps/corecheck/Cargo.toml`, off in every shipped build.
2. **A static regtest `Image` inside corecheck, not in `swiftsync-verify`'s `IMAGES`**, so the core verifier, its CLI
   and its `.wasm` sha do not change. Its fields (`swiftsync_verify::Image`):
   - `network: "regtest"`
   - `method_id_hex: "eb2cd424b94a4aa74f3458efc0d5edb1d3c35d46615eab8363696f1fa4b47ccd"`: CI's `host_reg`
     (`build-hosts.sh`, the reproducible container; run 43835411815 on main `797b777e`). It moves with any guest
     change: re-pin from the `libzync-hosts` artefact's `ids.txt`, never from a laptop build (another id).
   - `profile: 0x79` = LIBZYNC 1 | regtest 8 | COINLOG 16 | TRIE 32 | UTXOHASH 64 (the executed `state_*.json`;
     `describe_profile` already prints bit 8 as "regtest"). `with_coinlog` / `with_product` then change nothing.
   - `genesis_hash_hex: "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206"`, `genesis_time: 1_296_688_602`,
     `genesis_bits: 0x207f_ffff`, `genesis_work: 2` (2^256 / (target + 1)).
   - `genesis_anchor_hex: "99509fc506d022f06dfd72bb146fc894718ce8047e69e10b201e1a78d4bccf63"`: from
     `scripts/libzync/genesis_anchor_id.py regtest` (regtest added on this branch) and equal to the executed chain's
     `anchor_id` (`execute.sh` checks it).
   - `min_work_hex`: 64 zeros (Core's regtest `nMinimumChainWork` is 0), so **the OK branch is reached**: this is the
     point of the feature (an `--anchor` would cap every verdict at DEMO).
   - `checkpoint_height: 0` (the regtest guest pins none: `checkpoint.rs` `CHECKPOINT = None`), `lineage: &[]`
     (`lineage/regtest_coinlog.rs` is empty on main).
3. **`image_for("regtest", …)`** returns it under the feature and stays `unknown network` without (today
   `Image::by_network("regtest")` is `None`). `verify_core`, `image_used` and the pinned-image rule need no change:
   `earlier_image_named_by` finds nothing for an id outside `IMAGES`, and `pin::PRE_UNDO_IMAGES` does not list it, so
   leaves carry the undo hash from block 1, as this image commits them.
4. **Refused by release builds** (decision 1): `#[cfg(all(feature = "regtest-test", not(debug_assertions)))]
   compile_error!(…)`, so the harness builds its CLIs with the dev profile; plus a check in `scripts/wasm-build.sh` /
   the release scripts that no artefact's feature list holds `regtest-test`; plus a first check line in every
   regtest report, "TEST IMAGE (regtest-test): never a release", so a verdict cannot be mistaken for a mainnet one.
5. **Positive controls**: the shipped CLIs, WASM modules and C ABIs (default features) must refuse the regtest receipt
   (FAIL: unknown network, or the image mismatch when a mainnet image is named); with the feature, a 112 receipt with
   `--expect 112:67c1d2e7…` must be OK and with `--expect 113:1b5db3da…` FAIL; and a mutant of the feature's `Image`
   (min work non-zero) must turn the OK into DEMO.
6. **`zapps/common`** (the same feature): `--network regtest`, `--bitcoin-cli "bitcoin-cli -regtest -datadir=…"`, and
   every `gettxout` it issues with `include_mempool=false` (the reorg trap above). Its tip lookups must name the active
   chain (`getbestblockhash` / `getblockhash`), which after `invalidateblock` is the other branch.

Also outside the core image: the hints and Utreexo guests take mainnet or signet only (regtest builds needed to execute
them here, plan §3.2), and the header-chain guest is mainnet-only, so the deposit runs core-only on this chain.

## Execution (dev mode, MEASURED 16.11.2026; `execute/RESULT_execute_eb2cd424.txt`)

On hazync-coord with CI's `host_reg` (`eb2cd424…`, run 43835411815, main `797b777e`), `RISC0_DEV_MODE=1` (every
guest executed and its claims checked, nothing proved), `HAZYNC_SEG_PO2=21` (the CUDA host's segment size, so the
counts are the GPU's), `HAZYNC_CHUNKS=1 HAZYNC_PROFILE_LAYOUT=cost` (the GPU batch's settings):

- **every block accepted**: 1..112 from genesis (A), and B 111..113 resumed (`HAZYNC_RESUME=1`) from A's output cut
  back to 110, so the prover does accept other block files above k (plan §3.2's open question);
- the control: B resumed on A's output still holding A's 112 is refused (`replayed BIP30 tree at 112 != the receipt's
  smt_root`);
- `anchor_id` of the chain = the spec's regtest genesis anchor `99509fc5…`;
- per block (`execute/stats_*.tsv`, segments at po2 21): a coinbase-only block is 2 receipts (trie 1 segment,
  aggregate 2), 3 segments, 3.3 M cycles; a block with transactions 3 receipts (+ 1 chunk), 4 segments (102: 6),
  5.4-7.1 M cycles. The whole batch (base 1..101, A 102..112, B 111..113) is **115 block proofs, 243 receipts, 360
  segments, 408,027,136 cycles** (308,214,402 user).
- CPU execution time on that box (shared with a release build): 19-21 s per coinbase-only block, 31-44 s per block
  with transactions; a first run with the default 2 chunks took 2,491 s for 1..112 and was accepted too.

## GPU batch (prepared, NOT run; needs the user's go)

**ESTIMATE: about 55-70 min of one RTX PRO 6000, about $2.30-2.90** (`gpu/estimate.py`): 115 block proofs at 15.1 s
per coinbase-only block (MEASURED: mainnet 2..200 on one PRO 6000, risc0 3.1.0, `RESULT_risc0_3_1_gpu.txt`; block 1
21.0 s), + 2.2 s per extra po2-21 segment (MEASURED: block 962,000's chunks, 12-14 segments in 27-31 s,
`gpu/GPU_RESULTS.md`) + 3 s per extra receipt (an allowance) ≈ 30 min proving, ×1.5 for margin (the first board run
measured ~22-27 s per early block on 4 cards, `docs/OPERATIONS.md`); + 25 min bring-up (CUDA toolkit, the CUDA host
build, 381 s MEASURED on a 128-core PRO 6000 pod, a possible first-use sm_120 JIT); at $2.49/h (as billed 11-11). Well
inside the approved < $10, so it can also carry the later hints/Utreexo receipts (plan §4.3, §4.8).

What it needs, all prepared:

1. **The image**: the regtest image is not published, so the guest comes from CI's `host_reg`:
   `extract_guest.py host_reg regtest_eb2cd424.bin` → 70,197,404 bytes, sha256 `1c3d671a…f79b`. The extractor was
   validated on `host_m` (all 6 embedded copies hash to the published mainnet `f7b87bf1…`), and
   `risc0_binfmt::compute_image_id` (3.0.6, the prover's lock) of the extracted program is **`eb2cd424…`** (checked on
   hazync-coord). On the pod, `prover/methods/build.rs` recomputes that id from the bytes again and refuses a mismatch.
   The file is at `hazync-coord:/root/hz-regtest-work/regtest_eb2cd424.bin`.
2. **The bundle**: `gpu/mkbundle.sh <guest> <dir>` (repo HEAD, the guest, pins `METHOD_ID eb2cd424…`,
   `GUEST_ELF_SHA256 1c3d671a…`, `PROFILE libzync=1 assumevalid=0 network=regtest coinlog=1 trie=1 utxohash=1`, block
   files `json_a/` 112 and `json_b/` 113). Dry run on hazync-coord: `BUNDLE_OK`, 100 MB. Rebuild it from the final
   commit before the run (the pod builds git HEAD).
3. **The run**: `gpu/gpu_run.sh <dir>` from the laptop (RunPod key and ssh key as the signet runs): deploys one RTX PRO
   6000 (`lz-zapp-regtest`), `pod/prep.sh` builds `cargo build --release --features cuda -p host` with
   `HAZYNC_GUEST_ELF/HAZYNC_GUEST_PIN_DIR/HAZYNC_LIBZYNC=1/HAZYNC_NETWORK=regtest` and asserts `method-id` =
   `eb2cd424…`; `pod/run.sh` proves for real (`RISC0_DEV_MODE` unset): `libzync-prove-range 1 112 json_a genesis out/a`,
   then `out/b` = `out/a` cut back to 110 and `HAZYNC_RESUME=1 libzync-prove-range 1 113 json_b genesis out/b`
   (`HAZYNC_CHUNKS=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_COINLOG_SAVE_EACH=1`, five tries each on a GPU fault), then
   `libzync-verify` on a 112 and b 113. A watcher harvests everything but the leaf bundles and only then terminates,
   at the deadline (150 min) or a balance under $1.
4. **What to keep** (plan §3.2): receipts a 104, 105, 110, 112 and b 113 (~225 kB each), each run's `history.bin`,
   `state_*.json`, and the coin log + trie snapshot at those heights, committed beside the chain; the rest of the
   harvest stays off the repo.
