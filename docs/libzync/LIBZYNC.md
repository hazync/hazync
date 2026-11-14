# libzync: the engine of Hazync v2

**libzync is the engine of Hazync v2.** Hazync v1 was the original "Core" design (one CORE guest, last released as
v0.22.1, image `37987b85…`); its documents are kept as a record in [`docs/history/`](../history/).

**On `main` since 2026-11-08 (#670). Released as v0.24.0 (2026-11-12) on RISC Zero 3.1.0**
([`RELEASE_NOTES_v0.24.0.md`](../RELEASE_NOTES_v0.24.0.md)). **Published images** (frozen core c14 guest source,
tag `libzync-c14` = `ce4e1e3`, rebuilt on risc0 3.1.0 at `7dfb05b`; built by `reproduce/libzync.Dockerfile`,
reproduced by two builds and by CI): mainnet **`1ff5a129…`** (v0.26.0; continues v0.24.0's `8888d59c…` at block 2000,
[`../HANDOVER.md`](../HANDOVER.md)), signet **`28e39600…`**, header-chain zyncAPP
**`2db34e12…`** ([`../../reproduce/LIBZYNC_IDS`](../../reproduce/LIBZYNC_IDS)). v0.23.0 (risc0 3.0.5, mainnet
`863c9ce3…`, signet `73b62530…`) is **withdrawn**: 3.1.0 fixes soundness bugs in RISC Zero's proof system, and 3.1.0
verifiers refuse 3.0.x receipts. Laptop builds of the same source give other ids (on 3.0.5, mainnet `bbf82800…`),
because ids depend on absolute build paths; several results below were measured on those or on 3.0.5 images and say
so. Every number names what it was measured on and says MEASURED or ESTIMATED. Status of every item, built or
open: [`ROADMAP.md`](ROADMAP.md). The report this replaces (design B, review rounds 1-8 in full) is
[`LIBZYNC_2026-10-31.md`](../../history/experimental/libzync-bench/retired-docs/LIBZYNC_2026-10-31.md).

**In service (2026-11-12):** the public board (api.hazync.org) runs Hazync v2 under `8888d59c` since 2026-11-12 (`1ff5a129` from block 2000),
restarted from genesis (first run: 1,287 blocks proven on 4× RTX PRO 6000 in 2 h 38 min); the whole header chain is
proved in one receipt under `2db34e12` and followed at the tip on CPU; zyncAPP pages are live on hazync.org
(`/zapps/`, `/headers/`, `/zapp/`); the Windows CUDA host proves natively.

⛔ **Not audited, no proof of the whole mainnet chain's blocks yet** (the header chain is proved). See §7.

---

## 1. In one paragraph

libzync, the engine of Hazync v2, is a RISC Zero zkVM guest (risc0-zkvm 3.1.0) and its host, which prove every Bitcoin
block valid from genesis by running **Bitcoin Core v28.0's own consensus code** (interpreter, sighash, serialisation,
CheckTransaction, merkle, PoW and retarget) and libsecp256k1 0.5.1, compiled to rv32im. A block is split into
**chunks** that run Core's `VerifyScript` on the inputs they own, with every signature's group equation checked by one
batch per chunk; an **aggregate** verifies the chunk receipts and the previous block's chain proof, applies the
block-level rules, and commits the new chain state. Coins are kept in an append-only **coin log** that is sound at
every block; a **counting trie**, updated in its own receipt, answers script-keyed questions (absence, history,
balances); a **live-coin multiset hash** lets a new node check a downloaded UTXO set. A **block forest** commits every
block and the coin state after it, a **handover** lets a new image continue a chain, and **joins** fold independently
proved blocks into one proof. **zyncAPPs** are separate guests that verify a core proof and answer one question with
it. SwiftSync hints files (design B) are no longer part of the proof: retired in E2, 2026-11-05.

## 2. What a proof states

Normative: [`SWIFTSYNC_PROOFS.md`](SWIFTSYNC_PROOFS.md) (the file name is historical; it is now exactly the
statement, the `ChainState` layout and the verifier procedure). In short:

- A receipt of a libzync image says every block from genesis to `tip_hash` is valid under Core's rules as compiled into
  the image ([`CONSENSUS_RULES.md`](CONSENSUS_RULES.md): 59 rows SAME, 3 STRICTER, 0 WEAKER, 0 NOT ENFORCED).
- Its journal, `ChainState` version 2, is the state after the tip: coin-log roots and live value, trie root,
  live-coin hash, block forest, work, time context, BIP30 tree root, `profile`, `self_id`, `lineage`, `anchor_id`.
- It means something only if `anchor_id` is the network's pinned genesis anchor (mainnet `852c5c7e…390167`).
- Verdicts: OK (genesis-anchored, enough work, at or past the pinned checkpoint, mainnet 970,000), DEMO (exit 2:
  supplied anchor, low work or below the checkpoint), FAIL. Which chain it is stays a separate check against a source
  the verifier trusts (`--bitcoin-cli`, `--mempool`, `--expect`).
- Product profile `0x71` mainnet / `0x75` signet: libzync, coin log, counting trie, live-coin hash; scripts checked.

## 3. How it works

```
block ─► host: chunks (cost-packed), signature + ECMH hints, coin-log and trie witnesses
          ├─► chunk k        (mode 4)  Core VerifyScript per input; batch check of deferred signatures;
          │                            commits per-transaction facts (txid, wtxid, fee, weight, sigops, coin leaves)
          ├─► trie update    (mode 17) the counting trie: old root, update list by digest, new root
          └─► aggregate      (mode 5)  verifies chunk receipts, the trie receipt and the previous chain proof;
                                       block rules from the facts; coin-log flips and appends; live-coin hash;
                                       appends the block-forest leaf; commits ChainState
two chain proofs ─► join     (mode 18) left ends where right starts ─► one proof of both ranges
```

### Chunks and aggregate

Chunks run Core's code unchanged as the judge of every script; libsecp parses and range-checks every key and
signature, and a hinted R defers the group equation to one Fiat-Shamir-weighted multi-scalar multiplication per chunk
(a bad hint can only fall back to libsecp or fail). The aggregate never re-parses a transaction: it checks the
chunk-committed facts against the merkle-committed bytes and enforces merkle root, witness commitment, PoW, retarget,
MTP, subsidy, weight, sigops, BIP30/34/68, and that every input is covered exactly once. Any C++ exception the guest
can reach rejects the block (no unwinding, by decision; pinned in guest `build.rs` `REVIEWED_CATCHES`).

### Coin log — [`COINLOG.md`](COINLOG.md)

Every coin that outlives its block is appended in birth order to an append-only forest of perfect Merkle trees; a
spend flips the coin's leaf in place to a spent leaf recording (height, tx, vin). A spend opens the unspent leaf at
its position against the current roots, so a missing or already-spent coin cannot be spent, **at every block**: no
hints file, no closing height. The coin leaf's bytes are in [`UTREEXO_FORMAT.md`](UTREEXO_FORMAT.md) §2 (also a
historical name; it is not the utreexod format). Mainnet's log is about 2.4 billion leaves (COINLOG §8).

### Counting trie — ROADMAP E1c / E2

One two-level trie, script hash then coin, keyed by (SHA-256(scriptPubKey), outpoint), with history kept and a
per-script unspent count and value. It gives what the birth-ordered log cannot: absence ("no coin of script S"),
a script's complete history, and its balance. It is updated in **its own receipt** (guest mode 17), proved beside the
chunks, and verified inside the aggregate; its old root comes from the previous `ChainState`, never from the witness,
and the new root is committed as `trie_root`. Its contents, including the BIP30 edge cases, are listed in
[`ZYNCAPPS.md`](ZYNCAPPS.md) §2. The host keeps it in an arena of 64-byte records, in memory or memory-mapped on disk,
copy-on-write, with a passing crash test (ROADMAP E2).

### Live-coin hash (instant IBD) — ROADMAP E2b

`live_set` / `live_hash` are an ECMH multiset hash of the **live** coins, so a node that downloads a UTXO set checks it
against the proof instead of replaying the chain.

### Block forest and handover — [`HANDOVER.md`](HANDOVER.md)

The block forest has one leaf per block, genesis first, each committing the block hash, its height and the coin state
after it; so a tip proof commits the coin log at every earlier height, and a path shows any block (and, with a merkle
branch, any transaction) is in the chain. A handover (`is_base = 2`) lets a new image continue at a pinned
`first_height` from a pinned predecessor, recording it in `lineage`. Allowed for a soft fork or a cheaper prover with
the same statement; **never for a soundness fix** (re-prove from before the bug).

### Joins of independent block proofs — ROADMAP H-B

Today's chain is linear (block N's aggregate verifies N−1's receipt). For a whole-chain backfill, the host prepares
the state before every block natively (untrusted starting points); each block is proved as a base step on its state
in any order, on any number of cards; a join (guest mode 18) verifies two adjacent proofs of the same image and
requires the right one to start where the left one ends (the link digest excludes `anchor_id`, so balanced trees
work). A wrong host state can only fail a join, never forge. N blocks need N−1 joins over ~log2 N levels, every level
parallel. Prep writes a per-block leaf bundle (the state before the block and every witness its proof needs),
and the coordinator serves them as work items (ROADMAP H-B 8, e).

### zyncAPPs — [`ZYNCAPPS.md`](ZYNCAPPS.md)

Separate small guests that verify a core receipt inside the zkVM (the core image id is an input, so they survive
handovers) and answer one question. Built: `coin` (unspent / spent at the core tip) and `headers` (the header chain
from genesis: proved genesis..975,278 in one receipt under `2db34e12`, followed at the tip by `hazync-headers-follow`).
Being built: `zeroconf`. Live on hazync.org: `/zapps/` (the list), `/headers/` (header chain, checked in the
browser), `/zapp/` (Coin check: balance and history facts from the counting trie against the fixed mainnet 1..200
core proof, checked in the browser by the WASM verifier). New core facts are appended to
`ChainState` only at a handover. Proof bundle format: [`PROOF_BUNDLE.md`](PROOF_BUNDLE.md).

## 4. Measured

| what | measured on | result | source |
|---|---|---|---|
| block 962,000, guest cycles | c14, execute mode, 16 chunks, laptop | **440,594,143** = chunks 375,697,303 + aggregate 64,896,840 (the trie update's own receipt is not in this sum) | `handover/RESULT_c14.txt` |
| trie update, block 962,000 | c10e5, execute mode (trie's old state synthesised) | 47 M cycles; core 440 M; total 487 M | `absence/README.md` "The product, integrated" |
| block 962,000 vs canonical CORE, cycles | c10e5 incl. trie (487 M) vs CORE ~4,430 M | **9.09x** fewer | `absence/README.md` |
| block 962,000 proved, one RTX PRO 6000 | c14, 11-06, quiet host (load ~13) | **537 s** = 16 chunks 406 s + aggregate 132 s (incl. the trie proof, 51.1 s, run serially here); every receipt verified | `gpu/GPU_RESULTS.md` (top) |
| same, loaded host (load ~55) | c14, 11-06, same image and bundle | 894 s (chunks 678 s, aggregate 223 s incl. trie 92.8 s): the host, not the image | `gpu/GPU_RESULTS.md` |
| GPU busy, mean | c14, quiet host | 42% (host-side witness generation dominates; lever M3) | `gpu/GPU_RESULTS.md` |
| block 962,000, one RTX PRO 6000 | design B, round-13 image `5e460bf3`, 10-29 | 517 s (452 + 65) | `gpu/GPU_RESULTS.md` (10-29) |
| block 962,000, 4× RTX PRO 6000 | design B, round 13, 10-29 | 174 s (141.2 + 32.6) | same |
| block 962,000 vs CORE, card time on one A40 | design B round 13 vs CORE `37987b85`, 10-29 | **8.6x MEASURED** (9,990 vs 1,167 s); critical path 13.3x (2,922 vs 219 s) | same |
| c14 vs CORE, card time | — | **~8.2x ESTIMATED, not measured**: chaining design B's A40 time by c14/design B on the PRO 6000 (1,167 × 537/517 ≈ 1,212 s vs 9,990 s) | derived from the two rows above |
| join of two chain proofs | c14, 11-06, real (non-dev) receipts, one PRO 6000 | **6.4 s** each, one 2^20 segment (padded; raw cycles not logged); 8 leaves (18.2 s each) + 7 joins over 3 levels; root journal identical to the linear chain's block 8, verified; root receipt 226,002 bytes | `gpu/GPU_RESULTS.md` (top) |
| joins for a whole-chain backfill | ESTIMATED from the 6.4 s join | ~975 k joins, ~1,700 PRO 6000 card-hours, ~$4.3k at $2.49/hr | same |
| whole mainnet chain to 970,000 | ESTIMATED 11-07 on c14 (19 sampled blocks, chunk + aggregate + trie receipt; trie modelled) | 335 T cycles: ~$148-156k on A40, ~$278-295k on PRO 6000, scripts ON (design B was 257.7 T, ~$114-119k A40); ±50% | `cost/WHOLE_CHAIN_COST.md` |
| header chain zyncAPP, execute | `headers` guest, execute mode, real mainnet headers | 2,427 cycles per header (genesis..2,100); whole chain 975,279 headers = 2,350,725,289 cycles, 2,437 segments | ROADMAP K1; `RESULT_risc0_3_1_gpu.txt` |
| header chain zyncAPP, proved | `2db34e12` (risc0 3.1.0), one RTX PRO 6000, 11-11 | genesis..975,278 in one receipt: **59.4 min** (3.0.5 image `4d04d6bf`, now retired: 57.2 min), ~$2.50; 224,138 bytes; WASM verifier 39 ms on a laptop under Node | `RESULT_risc0_3_1_gpu.txt` |
| mainnet 1..200 from genesis | `8888d59c` (risc0 3.1.0), one RTX PRO 6000, 11-11 | **3,074 s** (block 1 21.0 s, then ~15.1 s each); receipt 227,362 bytes; verifier DEMO (below minimum work) | `RESULT_risc0_3_1_gpu.txt` |
| v2 vs v1 (CORE `37987b85`), real mainnet state | 8 blocks 200k-300k | **19.9x** fewer proven cycles on block 200,000, 9.5x across the eight; GPU time up to 15.1x less, 7.0x across the eight | `results/CYCLES_REAL_STATE.md` |
| first public-board run | `8888d59c`, 4× RTX PRO 6000, 2026-11-12, from genesis | 1,287 blocks in 2 h 38 min (~490 blocks/hr), ~$0.021 a block on these early blocks, $27.13 | board run, 11-12 (operator's measurement; not in the repo) |

The GPU benchmark's coin log and trie around block 962,000 are **MODELLED** (`lz::cl_synth_around`, from the block's
own spends and real transaction counts): the cycles are a real block's, the coin state is not mainnet history
(`gpu/gpu-libzync-multi.sh`). Paths above are under `experimental/libzync-bench/`.
Rows marked c14 / c10e* / design B ran on risc0 3.0.5 images (laptop builds for c14); the guest source of the
published 3.1.0 images is c14's unchanged, so their cycles are the same (the header chain's execute cycles and
segments were identical on 3.0.5 and 3.1.0), but proving times on 3.1.0 can differ (+3.8% on the header chain).

## 5. Testing and review

### Tests on the frozen core (c14)

| test | image | result | source |
|---|---|---|---|
| unit tests | c14 | libzync 29/29, swiftsync-verify 16/16, -ffi 2/2, host 68/68 | `handover/RESULT_c14.txt` |
| rig: blocks built to hit each rule, guest vs Bitcoin Core 28.0 (regtest) | c14 | **96/96 agree**, 0 deferred | same |
| handover O → N (regtest, every tampered handover refused) | c14 | **16/16** as expected | same |
| mainnet 1..200 from genesis (dev mode) | c14 (and c13) | coin log = `coinlog_ref.py`, block forest = `block_forest_ref.py` (independent implementations), genesis anchor `852c5c7e`, 0 panics | same; `RESULT_c13.txt` |
| signet from genesis (dev mode) | c13: **1..2,600**; c14: 1..100 (short pass) | coin log and block forest match the references (2,600: 2,693 leaves, 293 live coins) | `RESULT_c13.txt`, `RESULT_c14.txt` |
| edge blocks (2016, 91,842, 91,880, 185,956, 227,931, 363,725, 364,292, 388,381, 419,328, 481,824) | c14 | 10/10 valid | `RESULT_c14.txt` |
| Core's `script_tests`, `tx_valid`, `tx_invalid`, every flag variant (guest mode 10) | c10e12 | **22,666 / 22,666 agree**, 0 differ (679 by a guest abort where Core catches a throw: both reject) | `core_vectors/RESULT_c10e12_variants.txt` |
| same, batched signature path with forged hints | c10e11 | honest 1,420/1,420; forged: 0 Core-invalid cases pass | ROADMAP I1 |
| joins, dev mode: 200 leaves, balanced tree | c10e12 | 199 joins over 8 levels, root **byte-identical** to the linear block 200; join(linear 100, leaf 101) = linear 101; gap, swapped, twice, tampered anchor, other image all refused | `join/RESULT_c10e12.txt` |
| trie store: resume, tampered snapshots, on-disk = in-memory | c10e7 / c10e12 | identical roots; tampered refused; native prep 1..200 byte-identical to the proved run | ROADMAP E2 |

c14 differs from c13 only in how a finished collect run exits (`COLLECT_EXIT` instead of a panic); its cost moved by
+2 aggregate cycles and about −2,000 chunk cycles a block. c13 had the full pass, c14 the short one (ROADMAP H4). These passes ran on the laptop-built c14 (risc0 3.0.5). The 3.1.0 rebuild
(`7dfb05b`) moved no guest source; it was re-checked by the GPU batch in `RESULT_risc0_3_1_gpu.txt` and the
verifier test vectors (CLI, C ABI, WASM) that CI runs on every push.

### Review history

Cold reviews, each by fresh reviewers given the code, not the author's conclusions. Rounds 1-8 were on design B;
the numbering restarted for the reviews after E2. Full findings for 1-8:
[`LIBZYNC_2026-10-31.md`](../../history/experimental/libzync-bench/retired-docs/LIBZYNC_2026-10-31.md) §7.

| round | date | what it found | outcome |
|---|---|---|---|
| 1 | 10-30 | 12 findings, soundness-critical among them: a hint-block flag that switched coin accounting off, ECMH points with two y values, witness bytes unbound across chunks, BIP68 signed version compare, a second coinbase, coin height unchecked | all fixed and retested |
| 2 | 10-30 | coinbase flag 2 spent immature coinbase; BIP30 tree root not carried between blocks; verifier accepted a dev-mode receipt | fixed |
| 3 | 10-30 | `is_base` any value skipped verifying the previous proof (forgery); BIP30 descendants of a repeated coinbase | fixed (the same two bugs in the CORE guest: #668, closed as superseded when libzync replaced it) |
| 4 | 10-30 | open items: `time-too-new`, scripts-skipped label | `max_time` and `profile` in the proof |
| 5 | 10-31 | `verify-chain` accepted a libzync receipt with none of its checks; signet GPU evidence had failed its own verify | fixed (host and docs; image unchanged) |
| 6 | 10-31 | C++ side (patches, `verify_input.cpp`) accepts exactly what Core and libsecp accept; evidence scripts circular | scripts fixed |
| 7 | 10-31 | guest hygiene (`hz_copy` out-of-bounds read, lock arithmetic) | fixed; image moved |
| 8 | 10-31 | a stale guest build; claims stronger than the code | clean builds only; proposal tightened |
| E2 r1 | 11-05 | trie facts must carry their claimed answer | fixed (`a4a7c40`) |
| E2 r2 | 11-05 | trie soundness: no forgery found | low items hardened (`b551bc9`) |
| E2 r3 | 11-05 | verifier: no forgery found; built-in image table verifies nothing current | dev mode compiled out (`3d753ae`); table replaced at the freeze |
| E2 r4 | 11-05 | joins and mode 10: no soundness break | tests made able to fail (`050dce4`) |
| E2 r5 | 11-05 | join link digest included `anchor_id` (only left combs joinable); prep's BIP30 counting | fixed (`aa4036c`) |
| — | 11-05 | the rig found a coin re-created over its own spent record (BIP30, Core allows it) stalling the chain | liveness fix in guest, model and store (`d6f8b4b`) → c13 |
| E2 r6 | 11-06 | that fix: no accept or root divergence reachable | model / store depth aligned, undo refuses after a re-creation (`838bf5b`) |
| E2 r7 | 11-06 | host and harness: every committing run checked, undo stack, GPU script flakes | fixed, image unchanged (`73977b7`) |

## 6. How much of Bitcoin Core is in the proof

Measured on the design-B guest `09a02eed` (10-30), not re-measured on c14. The chunk code is the same design; the
aggregate has since gained the coin log, the trie receipt check and the live-coin hash, all Hazync code.

| source | lines | role |
|---|---|---|
| Bitcoin Core v28.0, 22 translation units (23 on signet) | 7,061 | interpreter, sighash, transactions and serialisation, hashes, CheckTransaction, merkle, PoW and retarget, chainparams, signet solution |
| libsecp256k1 v0.5.1 | 16,811 | every key and signature parse and check before the group equation; the full verify on fallback |
| libzync patches | 3 (`patches/libzync/0101`, `0102`, `0103` sha256 via the accelerator; `scripts/libzync-base.sh`) | each `#ifdef`'d, each reviewed to compute the same bytes as Core |

Execution profile of block 962,000 (`cost/core_share.py`, cycles charged to the innermost non-runtime frame):
chunks **46.7% Core C++**, 6.7% libsecp, 46.7% Hazync/libzync (of which the batch MSM 19%); the aggregate 0.8% Core,
99.2% Hazync, by design.

| decision | who decides |
|---|---|
| script validity (all opcodes, all flags), sighash | **Core** (`VerifyScript`, `SignatureHash`; a legacy SIGHASH_ALL fast path reviewed byte-equal) |
| key and signature well-formedness | **libsecp**, unchanged |
| the group equation R = u1·G + u2·Q | **libzync batch MSM** (fallback: libsecp) |
| transaction well-formedness, amounts | **Core** `CheckTransaction`, `CheckTxInputs` logic |
| merkle root, PoW target, retarget | **Core** code |
| the spent coin exists and is unspent | **coin log** (+ trie update marks it spent) (Core: `CCoinsViewCache`) |
| block sums and orchestration (fees, weight, sigops, BIP68, BIP30, coverage) | **Hazync Rust**, transcribed rule by rule (Core: `ConnectBlock` / `CheckBlock`) |

**What that costs.** "Core's code checks everything" is not literally true: the batch MSM, the block orchestration
and the coin accounting are new consensus-critical code, and the reviews found real soundness bugs in exactly those
parts. A prover now needs host preprocessing (signature and ECMH hints: 118 s for hints plus an execute check on the c14 GPU host; untrusted, so a
liveness cost only). One network per image. A libzync image runs only chunks, aggregate, trie update and join among
the block modes.

**What Core could take** (from the 10-30 report, still applicable): a salt-free, provable SwiftSync aggregate (ECMH
or MuHash: ~3.0k cycles per element measured in the guest, vs ~1.3k for the salted sum); one hint-index convention
(Core PR #34004 and the `hintsfile` crate count outputs differently); batch Schnorr verification (about 4× per
signature at block scale in the guest); legacy sighash midstate reuse (removes the quadratic term); lazy
txid / wtxid; "hints are advice, the code decides" for one-way checks. None of these is a Core bug fix: the BIP68
signedness bug was in Hazync's own transcription.

## 7. Limits and open work

- **No proof of the whole mainnet chain's blocks yet.** Real receipts from published 3.1.0 images: mainnet 1..200
  from genesis (`8888d59c`, 3,074 s on one RTX PRO 6000; `experimental/libzync-bench/zapp/`), the whole header chain
  (`2db34e12`), and the public board's proofs from genesis (1,287 blocks in its first run, 11-12). Earlier receipts
  (3.0.5 image `863c9ce3`, and laptop-built images: block 962,000, the 8-block join tree, 16 leaf proofs on real
  mainnet state; `experimental/libzync-bench/results/`) are kept as records; 3.1.0 verifiers refuse them. Signet is
  proved 1..2,600 from genesis in dev mode.
- **The board proves from genesis; the tip is not followed yet.** The board box's paced prep pass
  (`libzync-board-prep`) writes one leaf bundle per block (LZLEAF02) under `/srv/bulk/libzync-board/leaves`; the
  coordinator (`server.py` with `HAZYNC_LIBZYNC=1`) serves them as claims, and adjacent proofs are folded by joins.
  There is no v2 bridge: following the tip with libzync is not running yet (ROADMAP N). The separate paper prep run
  (`libzync-prep`, state before every block toward the tip) is described in ROADMAP, mainnet prep plan.
- **No external audit or bug bounty** (ROADMAP I2): needed before anyone relies on a proof for money. Fuzzing and a
  wider rig are open (I1).
- **The batch-signature soundness margin is untested at the edge:** x = r + n is infeasible for real r and effectively
  untested (ROADMAP I1).
- **Trust in risc0:** one pinned version, 3.1.0 (I3). 3.0.x receipts, including v0.23.0's, are refused.
- Not consensus: adoption is through the verifiers and, later, Ghost's coinbase (I6). Tip latency and the cost levers
  are in ROADMAP I7 and M.

## 8. Reproduce

```bash
scripts/libzync-base.sh ~/lz-base                       # Core v28.0 + libsecp 0.5.1 + patches, pinned
cd prover && rm -rf target/riscv-guest                  # always build the guest from clean (round 8: stale ELF)
HAZYNC_BASE=~/lz-base HAZYNC_LIBZYNC=1 cargo build --release -p host     # signet: HAZYNC_NETWORK=signet
host method-id                                          # a laptop build prints its own id; the published 1ff5a129 is the container build
HAZYNC_BLOCK=block.json host libzync-hints out.lzb      # a block's hint bundle
HAZYNC_BLOCK=block.json HAZYNC_LZ_HINTS=out.lzb host agg-exec-fake      # execute chunks + aggregate, print cycles
host libzync-prove-range 1 200 <json dir> genesis <out>                 # prove a range from genesis
host libzync-prep <lo> <hi> <json dir> genesis <out>                    # native states for anchored leaves
host libzync-join left.rcpt right.rcpt out.rcpt                         # join two adjacent proofs
host libzync-verify block_H.rcpt --bitcoin-cli "bitcoin-cli"            # §5 of SWIFTSYNC_PROOFS.md
host core-vectors                                       # Core's script / tx vectors (scripts/libzync/core_vectors.py)
```

Full test pass: `experimental/libzync-bench/handover/retest_final.sh <tag> <previous tag>` (`SN=100` for the short
pass). GPU: `experimental/libzync-bench/gpu/run_c14_pod.sh`, `gpu-libzync-multi.sh`, `gpu-join.sh`. A published id
needs the container build, `reproduce/libzync.Dockerfile` (ids depend on absolute build paths).

Related Hazync docs: [`SECURITY.md`](../../SECURITY.md),
[`BUILD_AND_RELEASE.md`](../BUILD_AND_RELEASE.md), [`COORDINATOR.md`](../COORDINATOR.md),
[`OPERATIONS.md`](../OPERATIONS.md).
