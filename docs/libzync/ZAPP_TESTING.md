# zAPP testing before merge (plan)

Status (16.11.2026): **plan only, for approval.** Nothing here is implemented. It covers the eight zAPP PRs that are
open as drafts today: #724 header MMR, #726 block check, #731 hints, #740 assumevalid / assumeutxo, #741 liveness,
#742 balance statement, #743 deposit, #744 Utreexo roots (prototype), and the code they share (`zapps/common`,
`zapps/corecheck`, `scripts/zapp-checks.sh`, `scripts/wasm-build.sh`). Each zAPP is its own module (ZYNCAPPS.md §9).

Every number marked **ESTIMATE** is mine and has not been measured. Every other number cites where it was measured.

## 1. What is wrong with today's tests

The tests on the branches are thorough, but most of them check our code against itself: unit tests, the zAPP's own
reference script, tampered inputs that must be refused, and real receipts only at very low heights. That catches a
change. It does not catch a spec misread that our code and our reference script share, and nothing runs at a scale
where modern blocks, deep trees or millions of coins appear.

| zAPP | real receipts today | oracle today | what is not independent |
|---|---|---|---|
| headers MMR (#724) | GPU proof 0..975,278 (`de54b94f`, RESULT_gpu_whole_chain_mmr.txt) | end hash and chainwork equal to the node's; MMR paths for 37 and 170 | the MMR paths come from our own path builder |
| block check (#726) | the same | a fake node with real headers 0..200 | the service and the verifier are both ours |
| hints (#731) | CPU proofs at H = 2, 3 under `46666b6f` | `hints_ref.py`, the `hintsfile` crate, the BIP's 3 vectors (HINTS_ZAPP.md §10) | `hints_ref.py` was written by us from the same doc; no node said which coins are unspent |
| assume (#740) | mainnet 1..200 (`8888d59c`) | Core's `dumptxoutset` at 200, 100,000, 174,000 (ASSUME_ZAPPS.md §5) | ECMH only against `assume_ref.py` and prep, not a proof, above 200 |
| liveness (#741) | mainnet 1..200 (`8888d59c`) | none outside our code | regtest data is execute-mode state, not a receipt |
| statement (#742) | mainnet 1..200 | the `bip322` crate | every non-zero balance is a trie built in the test; P2PK is our own code only |
| deposit (#743) | core 0..200, core 0..1 under `46666b6f`, headers 0..975,278 | none outside our code | the 64-byte case is synthetic only |
| Utreexo (#744) | CPU proofs at H = 2, 3 | Floresta's code via `utreexo-ref` to 100,000 (UTREEXO_ZAPP.md §2) | Floresta's own shipped values (939,969; 296,870) not reproduced |

Two facts shape the whole plan:

- **Every core proof below block 970,000 is DEMO** (the pinned checkpoint, `swiftsync-verify/src/lib.rs`). The board
  restarted from genesis under `46666b6f`, so on real data no core-based zAPP reaches its OK path for a long time.
  The OK branch of every verdict is therefore tested only by mutation testing (§3.5) and by a regtest test image
  (§3.2) until then.
- **A zAPP verdict is only as deep as the core receipt under it.** Native execution (no proof) can test a zAPP's
  derivation at any height whose blocks we have. A real zAPP receipt at height H needs a core receipt with tip ≥ H.

## 2. Rules for every test in this plan

1. **The oracle is not our code.** Bitcoin Core's RPCs, another project's implementation, or a value another project
   published. Where I write a small script to reshape an oracle's output, it reads only the oracle's documented
   format, not our source.
2. **Every check has a positive control**, as `adversarial.yml` already does for the release gate: a deliberately broken
   verifier (accept-all, or one check removed) must make the test fail. A test nobody has seen fail proves nothing.
3. **Results are committed** as `.out` files next to the script, with the date, the commit and the heights, so a
   reviewer reads what ran.
4. **Heavy runs are not CI.** Oracle nodes, fuzzing and mutation runs go on hazync-coord (not the laptop: its disk).
   CI replays their small artefacts (corpora, committed blocks, receipts) in seconds.

## 3. Shared infrastructure

### 3.1 The oracle node: `scripts/libzync/oracle_at.sh <H>` (new)

One throwaway Core node, the pattern `assume_check.sh` already uses: its own datadir, synced from our node over P2P
(`-connect=127.0.0.1:8333 -stopatheight=<H+1>`), restarted offline at H, pruned (`-prune=550`), with `-coinstatsindex`.
At H it writes `oracle/mainnet-<H>.json`:

| field | from |
|---|---|
| block hash, header, chainwork, MTP at H | `getblockhash`, `getblockheader` |
| `txouts`, `total_amount`, `muhash`, `hash_serialized_3` | `gettxoutsetinfo hash_serialized_3` and `gettxoutsetinfo muhash` |
| the UTXO set | `dumptxoutset <file>` (path and sha256; the file stays on the box) |
| unspent-at-H per outpoint, for all outputs of blocks 1..H (H ≤ 200,000) or a 1,000,000-outpoint sample | `gettxout <txid> <n>` batched |
| the SwiftSync hints | `generatetxohints` from a Core build of PR #34004 at a pinned commit, where it builds |
| a second chain source | mempool.space `/api/block-height/<H>` (hash only) |

Then the node is restarted with the next `-stopatheight`, so one pass serves every milestone. Our node is never
rolled back. Milestones: 200, 1,999, 10,000, 100,000, 147,396 (the first BIP30-tracked coin, ASSUME_ZAPPS.md §7),
174,000, then 481,824 (segwit), 709,632 (taproot), 840,000 (the assumeutxo height in Core v28's `chainparams.cpp`;
I read the file of the release I test against), 939,969 (Floresta's assumeutreexo value).

What does not depend on H runs on our own unpruned node instead, with no rollback: `getblock <hash> 3` (prevouts
from Core's undo data), `gettxoutproof` / `verifytxoutproof`, `getrawtransaction`. A pruned oracle node has
discarded those blocks.

Cost: 200 blocks synced in 28-31 s on hazync-coord (ASSUME_ZAPPS.md §5). **ESTIMATE:** to 174,000 under 2 h; to
939,969 6-12 h; chainstate about 12 GB pruned at 840,000; `dumptxoutset` at 840,000 about 9 GB. The 840,000 snapshot
may instead be the community file: its content hash is Core's compiled value, so where it came from does not matter.

### 3.2 The regtest harness: `scripts/zapp-regtest/` (new)

What mainnet cannot give soon: spends under a real proof, same-block spends, a BIP30 repeat, a real 64-byte
transaction, coins under keys we hold, the OK verdict, and reorgs. It reuses the core rules rig
(`experimental/libzync-bench/core_rules/rig.py`: Core's `test_framework`, `-testactivationheight=bip34@110`, the
tracked-coin bookkeeping).

**The chain** (about 116 blocks, fixed keys, RFC 6979 ECDSA and zero `aux_rand` Schnorr, fixed times, so it is
deterministic; the raw blocks are committed as hex and replayed with `submitblock`):

| blocks | content | used by |
|---|---|---|
| 1..101 | coinbases to harness keys, alternating P2PK (both forms), P2PKH, P2WPKH, P2SH-P2WPKH, P2TR; block 2's coinbase has no height push so it can repeat | all |
| 102 | spends of matured coinbases to every script type; an OP_RETURN deposit (several push forms); a zero-value output; a 10,001-byte script (unspendable); block 2's coinbase spent | statement, deposit, hints, utreexo, assume |
| 103 | same-block spend chains (1 hop and 3 hops) | hints, utreexo, liveness |
| 104 | 2^k + 1 transactions (odd count: the duplicated last hash), many outputs | deposit, block check |
| 105 | block 2's coinbase repeated byte for byte (allowed: the first is spent, below 110); a real 64-byte non-witness transaction (Core accepts it: rig case `tx_64_bytes`) | assume (tracked bit), liveness (`created=`), deposit |
| 106..110 | coins spent at varied heights, some created and spent within the range | liveness, hints |
| 111..112 (A) | deposit transaction D; coin X spent | reorg cases |
| 111'..113' (B) | more work than A; D absent; X unspent; coin Y spent instead | reorg cases |

**Proofs.** The regtest host (`HAZYNC_NETWORK=regtest HAZYNC_LIBZYNC=1`) proves the chain with
`libzync-prove-range 1 112 <block json dir> genesis <out>`, real proofs, no dev mode; branch B resumes from a copy of
the out dir at 110 (`HAZYNC_RESUME=1`; I must check that it accepts other block files above k). Only the tip receipts
the tests need are kept (110, A 112, B 113, and 104 for H < T cases), about 225 kB each, with each run's `history.bin`
and trie snapshot. **ESTIMATE:** on one RTX PRO 6000, 116 blocks × 22-27 s per early block (OPERATIONS.md, first v2
run) ≈ 45-55 min, about $2.50 at $2.79/hr; on hazync-coord's CPU, 116 × 943-1,062 s per block (HINTS_ZAPP.md §10,
mainnet 1..3) ≈ 32 h. I would run it in one GPU batch with the other proofs of §6, not on its own.

**Prerequisites** (code, decided before I build the harness):

- **A regtest image the verifiers accept, in test builds only.** `swiftsync-verify`'s image table has mainnet and
  signet only, and `corecheck::image_for` takes those two. I propose a static regtest `Image` (regtest genesis
  anchor, minimum work 0, checkpoint 0) behind a default-off `regtest-test` feature in `zapps/corecheck`, so the core
  verifier does not change and its `.wasm` sha stays the same (checked). With minimum work 0 the OK branch runs.
  Positive control: the shipped CLIs, WASM modules and C ABIs (default features) must refuse a regtest receipt.
- **The hints and Utreexo guests take mainnet or signet only** (a signet flag, two pinned anchors). A regtest build of
  each (test images, never published, never pinned) is needed to execute them on this chain. Execute mode with the
  core receipt as an unresolved assumption is enough there, as HINTS_ZAPP.md §10 does.
- **The header-chain guest is mainnet only** (no min-difficulty rule, no regtest pow limit). So on regtest the deposit
  runs core-only (no header receipts), and the block check is tested on regtest at the service level only.

**The run** (`scripts/zapp-regtest/run.sh`): start `bitcoind -regtest -testactivationheight=bip34@110 -txindex
-coinstatsindex` in a temp dir, submit 1..110 and branch A, run every zAPP against the Core oracle at 104, 110 and 112
(tables in §4), submit branch B (a reorg), run the reorg cases, `invalidateblock` back to A and run them once more.
**ESTIMATE:** 3-5 min with the receipts cached, plus 10-20 min to build the eight CLIs and WASM modules cold.

**CI.** The run fits CI with the cached receipts and blocks: it needs a Core binary (a sha-pinned tarball, cached;
an uncached download inside a test timeout has caused flakes before) and no `test_framework` (that is only for
regenerating the chain). Regenerating proofs is never CI. The repo is public, so standard runners do not bill, but
I would still ask before adding the job.

### 3.3 Differential surfaces: `scripts/zapp-diff.sh` (new)

Each zAPP has one library behind two or three surfaces. Today each surface has its own test list, partly
overlapping. Instead: one corpus per zAPP, `zapps/<name>/corpus/<case>/` (inputs plus `want`: verdict and answer),
run through every surface, and the outputs must be identical, not only the exit code.

| zAPP | CLI | WASM (node, the glue) | C ABI |
|---|---|---|---|
| headers / block check | `zapp-headers` | `verifyHeaders` | none |
| hints | `zapp-hints` | `verifyHints` | `swiftsync_verify_hints` |
| assume | `zapp-assume` | `verifyAssume` | `swiftsync_verify_assume` |
| liveness | `zapp-liveness` | `verifyLiveness` | `zapp_liveness_verify` |
| statement | `zapp-statement` | `verifyStatement` | none |
| deposit | `zapp-deposit` | `verifyDeposit` | none |
| Utreexo | `zapp-utreexo verify` | `zapp-utreexo.js` | none |

Compared: the verdict (OK / DEMO / FAIL), the answer fields (normalised: hex case, display order), and for FAIL that
it fails in the same check. The corpus is every existing test input, the regtest cases, every fuzz crash and every
mutant-killing case. It runs in each `test.sh` (seconds), so CI covers it.

### 3.4 Fuzzing: `zapps/<name>/fuzz/` (cargo-fuzz, new)

The repo has fuzz history (`history/audit-fuzz`, `guest-pure-fuzz`) but no zAPP target. Targets are the decoders that
read attacker-supplied bytes. Properties: no panic, no abort, no allocation larger than the input justifies (the H1
lesson in HINTS_ZAPP.md §2); and, structure-aware, a mutation of a valid input is never OK unless it re-encodes to the
same canonical bytes.

| zAPP | targets |
|---|---|
| headers | MMR path text; the `/tx` answer JSON (`from_json` / `parse`); `verify_tx_in_header` |
| hints | `ef_decode`, `decode_file` (round trip: decode then encode equals the input, else refused) |
| assume | `read_snapshot` (from a real dump), `parse_tracked`, the headers file |
| liveness | LZLV `decode` (decode then `encode` equals input); the question `parse` |
| statement | statement JSON; purpose / nonce validation; signature base64 and `p2pk::verify` |
| deposit | bundle JSON; the transaction parser; `op_return_payload` |
| Utreexo | `decode_receipt` (the 16 MiB limit) |
| shared | each WASM module's frame parser; `zapps/common`'s `--expect` parser |

**ESTIMATE:** 1 h per target per round on hazync-coord, about 15 targets, so about 15 core-hours a round. The
corpora are committed and replayed by `test.sh`.

### 3.5 Mutation testing: cargo-mutants (new)

`cargo mutants` on each verifier library, run on hazync-coord with `-j2`, listing first (`--list`) to count. A
mutant that survives in a **verdict-path function** (anything that can turn FAIL into DEMO or OK, or DEMO into OK) is
a missing test, and must be killed or explained in writing in the PR. Survivors elsewhere (printing, help text) are
listed and left for later. Per zAPP, the verdict-path functions are named in §4. **ESTIMATE:** 100-300 mutants per
library, 1-2 min each with an incremental release build, so 2-6 h per zAPP; killing survivors is the real work.

### 3.6 The standard adversarial matrix

Most of these exist somewhere. The work is a table in each PR, one row each, with the test that covers it, filled for
every surface:

flipped byte in the receipt · FakeReceipt · `RISC0_DEV_MODE` set · a receipt of another image (core receipt to a zAPP,
zAPP receipt to the core, a dev-built zAPP id) · an untrusted core id · a core with lineage or a pre-undo image ·
another network · `--expect` with the wrong height, the wrong hash · `--bitcoin-cli` naming another block · a node
without chainwork · two sources disagreeing · a truncated input, a trailing byte · OK withheld below minimum work ·
the answer about another question (another outpoint, script, txid, height).

## 4. Per zAPP

### 4.1 Header MMR (#724, `feat/headers-mmr`)

**Oracle.** Core's `getblockhash` / `getblockheader` for every height 0..975,278, and an MMR I write blind from the
text of ZYNCAPPS.md and `coordinator/deploy/headers/README.md` (Python, without reading `mmr.rs`), plus a second one
on the `ckb-merkle-mountain-range` crate with SHA-256 as its merge, comparing peaks.

**Scale.** All of it can run now, on the existing GPU receipt. One library-level loop verifies the receipt once and then,
for every h in 0..975,278: the path from `zyncapp headers-mmr-path` for (h, Core's hash at h) is accepted; Core's hash at
h+1 under the same path is refused; the blind MMR gives the same path. Peaks equal at every 2^k leaf count and the root
equals `bb22154a…` (RESULT_gpu_whole_chain_mmr.txt). End hash and chainwork against `getblockheader 975278` and
mempool.space, scripted (today it is a line in the RESULT file). **ESTIMATE:** under 30 min CPU.

**Adversarial.** The existing set (wrong order, gaps, the same run twice, paths short or long, 64-byte forgery) plus:
paths for every height of a 2^k + 1 chain (the edges of the peaks); a header receipt chained to another base
(`--after`). Fuzz and mutants as §3.4 and §3.5; verdict path: `verify_headers`, `verify_inclusion`,
`verify_tx_in_header`.

**Follow-up, not gate:** a header rules rig like the core's: diff-1 headers mined on a fork below block 2,016 (bad
bits, time-too-old, a broken link), each run through `zyncapp headers-exec` and Core's `submitheader` on a throwaway
mainnet node, the verdicts compared. The guest's rule code did not change in this PR (16 lines of `main.rs`, the MMR
only), so this is not a gate for it. **ESTIMATE:** a few CPU-minutes to mine each diff-1 header.

### 4.2 Block check (#726, `feat/block-check`)

**Oracle.** Core's `gettxoutproof` / `verifytxoutproof` (Core's own Merkle proof of a transaction in a block) and
esplora's `/tx/<txid>/merkle-proof` (mempool.space) for the branch and position; `getrawtransaction` for the raw bytes.

**Scale.** Now: 1,000 real mainnet transactions chosen by a script (every era, segwit and taproot spends, the largest
blocks, blocks with an odd transaction count, the first and last transaction of a block, coinbases). For each, the
service's `/tx` answer (run against our node, `txindex=1`) must equal what an independent builder makes from the two
oracles, and `zapp-headers --include … --tx <txid>:<answer>` must accept it against the GPU receipt. **ESTIMATE:** the
service's first sync is 975k hashes (its README); the sample about 1 h with `TX_RATE` raised for the test.

**Adversarial.** A lying-service proxy: for each of the 1,000 answers, 10 mutations of every field (`hash`, `path`
entries, `pos`, `header`, `raw`, `branch`, `coinbase`, `coinbase_branch`, the height asked); none may give INCLUDED.
A reorg on a real regtest node (`invalidateblock`, a longer branch) instead of the fake node: the service stops serving
until its held hashes end on the published hash, then the paths equal the blind MMR's.

**Gate:** the 1,000-transaction oracle sample, the lying proxy (zero INCLUDED), the regtest reorg, plus §3.3 and §3.6.
The service is Python, so no cargo-mutants; its verdicts are the verifier's, which #724's mutants cover.

### 4.3 SwiftSync hints (#731, `feat/hints-zapp`)

**Oracle.** The node itself, not a second generator: for every output of blocks 1..H, `gettxout` on the oracle node at
H is non-null exactly when the file lists its index (the index rule's exclusions, unspendable outputs and the two
BIP30 coinbases, read from hintsfile.md, not from `hints.rs`); `gettxoutsetinfo` `txouts` = `n_unspent`, `total_amount` =
the value. Then Core PR #34004's `generatetxohints` on the same node, converted by HINTS_ZAPP.md §2's rule, must
equal the file. Undo: each block's U0 hash rebuilt from `getblock <hash> 3` on our node (Core's prevout data) by a
script written from UNDO.md §2, equal to the undo file's entry. One care: an outpoint created twice (mainnet's
91,842 / 91,880 coinbases, regtest block 105) is compared at its later creation only, since `gettxout` sees one coin.

**Scale.** Now, native or execute mode, no proof needed: H = 1,999, 10,000, 100,000 (every output, 264,251 at
100,000, HINTS_ZAPP.md §10) and a 1,000,000-outpoint sample at 174,000. **ESTIMATE:** under 1 h each after the
oracle node is there. A real hints receipt above 3 waits for the board's core receipt to pass H: at H = 1,999 then
100,000, in the GPU batch. **ESTIMATE**, from the header proof's 1.26 s per segment (5,969.5 s / 4,739 segments,
RESULT_gpu_whole_chain_mmr.txt; a different guest, so rough): 100,000 (2,599 segments) ≈ 55 min, about $2.60.

**Adversarial.** As built (HINTS_ZAPP.md §8, §10) plus the regtest chain in execute mode (a regtest guest build): the
same-block chains, the repeated coinbase of block 105 (counted twice, the first spent; `gettxout` sees one), the
10,001-byte script, the zero-value output. Fuzz `ef_decode` / `decode_file`. Verdict path for mutants: `verify_hints`,
`ef_decode`, `decode_file`.

**Gate:** the `gettxout` and `gettxoutsetinfo` oracle at 1,999, 10,000 and 100,000; the undo oracle at the same
heights; §3.3-§3.6. **Follow-up:** #34004 (if the PR does not build against a recent Core, I say so and go on), the
regtest execute run, real receipts above 3, modern-block windows.

### 4.4 assumevalid / assumeutxo (#740, `feat/zapp-assume`)

**Oracle.** `gettxoutsetinfo hash_serialized_3` on the oracle node at H, equal to the hash the checker prints; and
`muhash` recomputed with Core's own `test_framework/crypto/muhash.py` over the coins the checker parsed, equal to
`gettxoutsetinfo muhash`. MuHash is a different hash over a different serialisation, so a pass shows our parse of
the snapshot is Core's set, separately from our ECMH. At 840,000: the printed `hash_serialized_3` equal to the value
compiled into Core's `chainparams.cpp` (`m_assumeutxo_data`), a number Core's developers published.

**Scale.** Now: 100,000 and 174,000 (both already dumped, ASSUME_ZAPPS.md §5) and 840,000, parse and both hashes only
(the ECMH half needs a proof with tip 840,000, which does not exist). **ESTIMATE:** 840,000 has about 180 M coins;
at the measured 1.9-23 µs/coin (§5 there) 6 min to 70 min. On regtest, a real proof: the harness tip with block 105's
repeated coinbase, so a **tracked coin is live under a real proof**, the case §5 there could check only against the
guest's code. `dumptxoutset` on the regtest node at the proof's tip must pass with the tracked list, and fail with it
emptied.

**Adversarial.** As built (16 tampered snapshots, key order, side inputs) plus fuzzing `read_snapshot` from a real
dump. Verdict path: `check_snapshot`, `read_snapshot_with`, `assume_valid`, `bind`, `coin_leaf`, `mtp_before`,
`parse_tracked`, `run_assume`.

**Gate:** the `gettxoutsetinfo` and MuHash oracle at 100,000 and 174,000; 840,000 against chainparams; the regtest
tracked-coin proof; §3.3-§3.6. **Follow-up:** a mainnet proof with tip at a snapshot height (§7 there is a board
decision), a tracked coin live under a mainnet proof (≥ 147,396).

### 4.5 Liveness (#741, `feat/zapp-liveness`)

**Oracle.** For UNSPENT: `gettxout` on the oracle node stopped at H (value, scriptPubKey, coinbase, and creation height
as best height − confirmations + 1). For SPENT: esplora's `/tx/<txid>/outspend/<vout>` gives the spending txid, input
and height; the transaction's position from `getblock <hash> 1`. Neither is our code.

**Scale.** Now: every coin of blocks 1..200 asked at the tip and at three lower heights (the `8888d59c` receipt and
its `history.bin`); the `46666b6f` board receipt at its current top, with its `history.bin`, every coin asked at the tip
and a sample below. Bigger heights follow the board. The regtest chain adds spends under a real proof at every height
of 102..112.

**Adversarial.** The reorg cases, which the user asked for:

| proof | identity given | must be |
|---|---|---|
| A, tip 112 | `--expect 112:<A's 112>` | OK for X spent at 111 or 112 |
| A, tip 112 | `--expect 113:<B's 113>` | FAIL (height and hash) |
| A, tip 112 | `--expect 112:<B's 112>` | FAIL |
| A, tip 112 | `--bitcoin-cli` of the regtest node after the reorg to B | FAIL |
| B, tip 113 | `--bitcoin-cli` after the reorg | X unspent: OK; "X spent" refused |

plus the repeated coinbase pinned by `created=`, and the bit-flip tests as they are. Fuzz LZLV `decode` and the
question `parse`. Verdict path: `check_liveness`, `check_liveness_fact`, `verify_liveness`, `parse`, and `liveness::decode`,
`check`, `verify`, `state_leaf` in libzync.

**Gate:** the oracle on 1..200 and on the board's `46666b6f` receipt; the reorg table; §3.3-§3.6. **Follow-up:**
mainnet-scale paths (needs the disk-backed log, ROADMAP C1); the privacy experiment.

### 4.6 Balance statement (#742, `feat/zapp-statement`)

**Oracle.** Two kinds:

- **Balances:** `scantxoutset start '["raw(<spk>)"]'` on a node at H: `total_amount` and the number of unspents equal
  the balance fact. On mainnet, the P2PK scripts of early coinbases (block 9 and others) at the board's tip; on regtest,
  our own keys of every type, with non-zero balances under a real proof (today every non-zero case is a test-built trie).
- **Signatures:** a second BIP-322 implementation, `bip322-js` (pinned version) in node: every signature
  `zapp-statement sign` makes (P2WPKH, P2SH-P2WPKH, P2TR simple; P2PKH full) verifies there, and the reverse. Legacy
  P2PKH against Core's `signmessage` / `verifymessage` both ways. BIP-322's official vectors through `check_signature`
  itself, not only through the crate's suite. P2PK has no second implementation that I know of except Core PR #24058
  (BIP-322 in `verifymessage`, unmerged): I will try it at a pinned commit.

**Scale.** The signature half is height-free and can run now. Balances: 1..200 now (zero for our keys, real for the
block-9 coin), the regtest proof, then the board's tip.

**Adversarial.** As built (§9 there) plus fuzzing the statement JSON, purpose / nonce validation and `p2pk::verify`.
Verdict path: `check_statement`, `check_signature`, `message`, `script_list_hash`, `check_age`, and every function of
`p2pk.rs`.

**Gate:** the bip322-js and Core cross-checks; regtest balances against `scantxoutset`; P2PK either cross-checked by
#24058 or, if that does not build, `p2pk.rs` with zero surviving mutants and a review; §3.3-§3.6. **Follow-up:** shape B.

### 4.7 Deposit (#743, `feat/zapp-deposit`)

**Oracle.** Core's `gettxoutproof` / `verifytxoutproof` (the transaction is in that block), `getrawtransaction` (its
outputs), `getblockheader` (depth: the tip's height − h + 1, and Core's `confirmations`); esplora's `/tx/<txid>/status`
and `/merkle-proof` as the second source.

**Scale.** Now: every transaction of blocks 1..200 under `8888d59c` with the header receipt to 975,278. Then a
sample of 1,000 under the board's `46666b6f` receipt as it climbs (blocks with spends, so a non-empty undo hash, the
open item of §8 there). Blocks above the core tip only as the PoW-only cases already tested.

**Adversarial.** On regtest, core-only (the headers guest has no regtest): the real 64-byte transaction of block 105
(valid, so Core accepts it; the zAPP must refuse it, as documented, and no inner node of block 105 may pass as a
transaction); OP_RETURN deposits in every push form against `--op-return` and `--script`; the odd-count block 104.
Reorg:

| bundle | identity given | must be |
|---|---|---|
| D in A's 111, proof tip 112 | `--expect 112:<A's 112>` | accepted |
| the same | `--expect 113:<B's 113>`, or `--bitcoin-cli` after the reorg | FAIL, never accepted |
| the same | `--allow-demo` with B's identity | FAIL (a wrong identity is not DEMO) |
| a bundle for D against B's proof | any | cannot be built (D is not in B); `zapp-deposit build` refuses |

Fuzz the bundle JSON, the transaction parser and `op_return_payload`. Verdict path: `verify_deposit`,
`op_return_payload`, and `verify_tx_in_header` from `zapps/headers` (shared).

**Gate:** the Core and esplora oracle on 1..200; the regtest 64-byte case and the reorg table; §3.3-§3.6.
**Follow-up:** the board's deeper receipt (non-empty undo hash), the composed guest (b).

### 4.8 Utreexo roots (#744, `feat/zapp-utreexo`, prototype)

**Oracle.** Values another project published, not only its code: Floresta's shipped `AssumeUtreexoValue` for signet
296,870 (11 roots, 126,113,034 leaves) and mainnet 939,969 (18 roots, 3,066,490,760 leaves) (UTREEXO_ZAPP.md §2). A
second implementation: utreexod (Go), whose roots at a height I compare where its RPC gives them (the RPC name I have
not checked). And a cheap count: `n_live` = the oracle node's `txouts` + 2 for H ≥ 91,812.

**Scale.** The signet value can be reproduced natively, with no proof: sync hazync-coord's signet node from 60,039 to
296,870 and run the native derivation (I would add a native-only mode to `utreexo-prove`, test only, which skips the
core-state comparison). **ESTIMATE:** a few hours of signet sync and minutes of derivation. Mainnet 939,969 natively
needs the walk over every block to it. **ESTIMATE:** at least a day, unmeasured (UTREEXO_ZAPP.md §7 measures nothing
past 100,000). The `n_live` check at 1,999, 10,000 and 100,000 is minutes.

**Adversarial.** As built (16 guest refusals, chained runs). Plus: the `--floresta` output compiled into a test crate
against rustreexo 0.7.0 and loaded as a Stump, so the Empty-root mapping (the 32-zero-byte trap of §1 there) is
checked by rustreexo, not by reading our text. Fuzz `decode_receipt`. Verdict path: `verify_utreexo`,
`decode_receipt`, `floresta_rust`.

**Gate (to leave draft as a prototype):** the signet 296,870 reproduction, `n_live` against `txouts` at three heights,
the `--floresta` compile test; §3.3-§3.6. **Follow-up:** mainnet 939,969, utreexod, a regtest guest build, a real receipt at
100,000 (**ESTIMATE**, at the 1.26 s per segment of §4.3: 5,678 segments ≈ 2 h, about $5.50), the answers to §9 there
from the Floresta developers.

## 5. Shared code

- **`zapps/common`** (every CLI's `--expect` / `--mempool` / `--bitcoin-cli`). Oracle: a real regtest node and a real
  esplora (mempool.space for mainnet heights; a local electrs on regtest) instead of `printf` fake nodes. Cases: the
  sources agree; they disagree (no answer, FAIL); one gives no chainwork; a black-hole port and a slow, oversized or
  malformed HTTP reply (a FAIL in bounded time, never a hang or a pass). Fuzz the `--expect` parser. Mutants:
  `resolve`, `tip_height`, `check_core_tip`, `expect_for_core`, `expect_at`, zero survivors.
- **`zapps/corecheck`**. Mutants on `verify_core` and `image_for`, zero survivors: `tip_checked` true only after a match.
  The regtest test image lives here (§3.2), with its positive control.
- **`scripts/zapp-checks.sh`**. Positive controls in a test: a README pinning another sha must fail
  `wasm_sha_is_the_readmes`, a README without the line must fail, and both under GNU grep, as CI runs it (a local
  `grep` that differs can pass what CI fails). `c_with_core` already links both archives; keep it.
- **`scripts/wasm-build.sh`**. Rebuild every zAPP's `.wasm` from a fresh clone at another path and on a second machine
  (hazync-coord and CI): the sha must equal the README's. Control: `HAZYNC_WASM_TOOLCHAIN=<another>` must give another
  sha, and `test.sh` must catch it.
- **`scripts/check-test-surfaces.sh`**. Extend it so a zAPP with a corpus but no `zapp-diff` call, or a fuzz target
  with no committed corpus replay, fails.

## 6. Merge gate

A PR leaves draft when its gate passes. Everything else is follow-up, listed in the PR.

| PR | gate (all must pass) | follow-up |
|---|---|---|
| every PR | §3.3 surfaces identical on its corpus; §3.4 fuzz rounds clean, corpus replayed; §3.5 zero surviving verdict-path mutants; §3.6 matrix filled; positive controls; results committed | |
| #724 headers MMR | every height 0..975,278 against `getblockhash` and the blind MMR; root `bb22154a…`; end state against Core and mempool.space | header rules rig vs `submitheader` |
| #726 block check | 1,000-transaction sample against `verifytxoutproof` and esplora; the lying proxy (zero INCLUDED); a real regtest reorg | |
| #731 hints | `gettxout` / `gettxoutsetinfo` oracle at 1,999, 10,000, 100,000; undo against `getblock 3` | #34004; regtest execute; real receipts above 3 |
| #740 assume | `gettxoutsetinfo` + MuHash at 100,000 and 174,000; 840,000 against chainparams; regtest tracked-coin proof | a mainnet proof at a snapshot height |
| #741 liveness | `gettxout` + esplora on 1..200 and the board's receipt; the reorg table | mainnet-scale paths |
| #742 statement | bip322-js and Core cross-checks; regtest balances against `scantxoutset`; P2PK by #24058 or zero mutants + review | shape B |
| #743 deposit | Core + esplora on 1..200; regtest 64-byte transaction; the reorg table | deeper `46666b6f` receipt; guest (b) |
| #744 Utreexo | Floresta's signet 296,870; `n_live` = `txouts` + 2; `--floresta` compiles to the same Stump | mainnet 939,969; utreexod |
| shared | §5 in full | |

The regtest harness (§3.2) is on the gate of #726, #740, #741, #742 and #743, so it is built first.

## 7. Effort and cost (ESTIMATE, all of it)

| work | my days | compute |
|---|---|---|
| oracle node script and the milestone pass to 174,000 (840,000 and 939,969 later) | 1.5 | hazync-coord: hours to 174,000; 6-12 h more to 939,969 |
| regtest harness: chain, test image, regtest guest builds, runner, reorg cases | 5 | one GPU batch, about 1 h, about $2.50 (or about 32 h of CPU) |
| differential surface runner and corpora for 7 zAPPs | 2 | none |
| fuzz targets (about 15) and the first round | 3 | about 15 core-hours a round |
| mutation runs and killing survivors | 6 | 2-6 h per zAPP on hazync-coord |
| per-zAPP oracle scripts (§4) | 6 | the signet sync; bip322-js; #34004 and #24058 builds |
| shared code (§5) | 1.5 | none |
| **gate total** | **about 25** | **under $10 of GPU** |
| follow-ups: real hints and Utreexo receipts at 100,000 | | about $8 GPU (§4.3, §4.8) |

Order: shared code and the surface runner first (they shape every corpus), then the regtest harness (five gates
wait on it), then oracles per PR, then fuzz and mutants per PR as it nears the gate.

## 8. Decisions I need

1. The regtest test image in `zapps/corecheck` behind a default-off feature (§3.2), with min work 0 so the OK path
   runs. The alternative, `--anchor`, caps every regtest verdict at DEMO and leaves OK untested.
2. One GPU batch for the regtest chain (and, later, the hints and Utreexo receipts at 100,000).
3. Building two unmerged Core PRs (#34004, #24058) as test oracles only, never shipped.
4. Adding the regtest run and the corpus replays to CI's `zapps` job (minutes per run).
5. Whether #744 should leave draft at all before the Floresta developers answer §9 of its doc.
