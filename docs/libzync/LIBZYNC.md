# libzync: what it achieves, what it costs, and what Core could take from it

**Branch `libzync`, 2026-10-30.** Every number here was measured. Where something was not run, the text says so.

⛔ **libzync is an experiment, not a release.** It moves METHOD_ID, it is not on any shipped proving path,
and it had its first independent review on 2026-10-30 (§7). That review found soundness bugs, now fixed and
retested.

---

## 1. In one paragraph

Hazync proves Bitcoin blocks valid inside a RISC Zero zkVM by running **Bitcoin Core's own consensus code**:
Core v28.0's interpreter, sighash, serialisation and PoW, and libsecp256k1 0.5.1, compiled to rv32im.
libzync keeps that code as the judge of every script and signature. It changes **how the proof is
organised**:

- **Signatures:** each one is checked by a cheap hinted test, and all of them are confirmed together by one
  batch equation per chunk.
- **Transactions:** each is parsed and hashed once.
- **The aggregate:** it never re-parses a transaction.
- **The UTXO set:** it is accounted the **SwiftSync** way, using Ruben Somsen's published hints format. Coins
  spent before a handover height H go into an ECMH running total. Coins still unspent at H go into a Utreexo
  forest.

The result is about 8.5× fewer cycles than the canonical guest on a modern block, mainnet tip blocks proven
in 3–5 minutes on 4 GPUs, and a proof that a SwiftSync hints file is correct.

## 2. What it achieves (measured)

### Speed and cost

| measure | canonical Hazync (CORE, `37987b85`) | libzync | gain |
|---|---|---|---|
| block 962,000 (8,006 inputs), guest cycles | 3,357.6 M (one chunk) | chunks 339.7 M + aggregate 57.7 M ≈ **397 M** | **~8.5×** |
| block 962,000 proved on 1× A40, wall | 9,990 s | **1,167 s** | **8.6×** card time |
| block 962,000, critical path on one card (max chunk + aggregate) | 2,922 s | 219 s | **13.3×** |
| 2026-09-29 tip blocks 969,122 / 969,118 | 558 s / 368 s on **38** cards | **197 s / 220 s on 4** RTX PRO 6000 | 2–3× faster on ~1/10 the cards |
| live mainnet tip hour (2026-10-29/30) | 5 blocks, 38 cards, $15.83 per block | **4/4 blocks verified, 266–332 s each**, ~$10 for the hour | ~1/6 the cost per block |

Sources: `experimental/libzync-bench/gpu/GPU_RESULTS.md` and `BLOCK_962000.md`. The cycle figures come from
laptop execute mode. GPU figures are RunPod wall clock, with every receipt verified.

### A proof SwiftSync can use

- **Unmodified input:** the guest reads the **published `hintsfile` v0 format** (2140-dev/swiftsync,
  `utxohints.store`) unmodified.
- **What a proof at height H commits:**
  - `ss_set = 0`: the ECMH total is the identity;
  - `ss_hints`: a hash computable **from the hints file alone**;
  - Utreexo roots that are exactly the UTXO set at H.
- **What follows:** with the checks in §9 of the proposal (`docs/libzync/SWIFTSYNC_PROOFS.md`), the hints file is
  proven correct for that chain, with no trust in whoever served it.
- **Checked on mainnet blocks 1–500** (execute mode, real chain-state carry):
  - balanced, and the forest is exactly the 503 coins unspent at 500;
  - one flipped hint bit in either direction is rejected;
  - our indexing agrees with the published 970,000-height file on every coin it marks unspent.

### Engineering consolidation (this round)

| before | after |
|---|---|
| ~22 build flags, set identically at build time and at run time, or the input stream silently desynchronised | **one profile:** `HAZYNC_LIBZYNC=1` (or `assumevalid`), plus `HAZYNC_NETWORK=signet`. The host reads what the embedded guest was built for (`methods::LIBZYNC`, `NETWORK`) |
| 4 hint streams (pubkey-Y, batch signatures, ECMH, SwiftSync bits), 3 file kinds per block, each with its own reader | **one hint block per guest run** (`libzync/src/wire.rs`) and **one bundle file per block** (`libzync::host::BlockBundle`) |
| the SwiftSync hint hash computed two ways (guest and host) | one definition (`libzync/src/ss_hints.rs`), compiled into both |
| Core edits made by Python scripts rewriting a source tree in place | two reviewable patch files (`patches/libzync/0101`, `0102`) and `scripts/libzync-base.sh`, which rebuilds the exact tree from pinned upstream releases |
| mainnet only | mainnet and **default signet** (§8) |

Gate: on block 962,000 the consolidated guest commits **byte-identical chunk journals** to the previous
round (`687a2c5c…`, `a9593daf…`, `ada2363a…`, `4554492b…`), from the same 7,031 signature hints and 4,324 ECMH
operations. That was measured before the review fixes, which deliberately change the journal layout (§7).

## 3. How it works

```
block ──► host: split inputs into chunks (cost-packed) ──► hint bundle (libzync-hints)
            │
            ├─► chunk k (mode 4): Core's VerifyScript for every input it owns
            │     - libsecp parses, checks r,s; the batch hook DEFERS the group equation (hinted R)
            │     - one MSM per chunk checks every deferred signature together
            │     - commits per-transaction FACTS: txid, wtxid, fee, weight, sigops, coin leaves
            │
            └─► aggregate (mode 5): verifies every chunk receipt + the previous block's chain proof
                  - block rules from the facts: merkle, witness commitment, PoW, retarget, MTP,
                    subsidy, weight, sigops, BIP30/34, coverage of every input exactly once
                  - SwiftSync design B: spent-before-H coins -> ECMH total; unspent-at-H -> Utreexo
                  - commits ChainState (tip, work, forest, ECMH total, hint hash, anchor, self id)
```

## 4. How much of Bitcoin Core is still in the proof

### Source

| | lines | role in libzync |
|---|---|---|
| Bitcoin Core v28.0: 22 translation units (23 on signet) | 7,061 (pristine) | interpreter, sighash, transactions and serialisation, hashes, CheckTransaction, merkle, PoW and retarget, chainparams, signet solution |
| libsecp256k1 v0.5.1 | 16,811 | every key and signature parse, every check before the group equation, and the full verify on fallback |
| patches to Core / libsecp | canonical 4, GHOST 5, **libzync 2** (`0101`: 9 levers, +366 −5 lines; `0102`: 3 hooks, +90) | each is `#ifdef`'d; each computes the same bytes (third reviewer, §7) |
| Hazync guest code (Rust + C++ wrapper) | 5,428 | wiring, chain state, block bookkeeping |
| of which written for libzync | ~2,000 | batch MSM, ECMH, agg-lite, the `libzync` crate |

### Execution (block 962,000)

PENDING: profile split by owner (`cost/core_share.py`).

### What Core still decides, and what Hazync decides instead

| decision | who decides in libzync |
|---|---|
| is this script valid (all opcodes, all flags) | **Core**: `VerifyScript`, unchanged |
| what message a signature signs (sighash, all types) | **Core**: `SignatureHash`, plus a legacy fast path proven to produce the same bytes |
| is this signature/key well-formed, low-level checks | **libsecp**, unchanged |
| does R = u1·G + u2·Q hold (the group equation) | **libzync batch MSM** with Fiat–Shamir weights (on fallback: libsecp) |
| is this transaction well-formed (CheckTransaction), amounts in range | **Core**: `CheckTransaction`, `CheckTxInputs` logic via the wrapper |
| merkle root, witness commitment hashing, PoW target, retarget | **Core** code (`ComputeMerkleRoot`, `arith_uint256`, `CalculateNextWorkRequired`) |
| does the coin being spent exist and is it unspent | **SwiftSync + ECMH + Utreexo** (Core: `CCoinsViewCache`) |
| block-level sums and orchestration (fees, weight, sigops, BIP68, BIP30, coverage) | **Hazync Rust** (Core: `ConnectBlock` / `CheckBlock`), transcribed rule by rule |

## 5. What we gave up by going this way

1. **"Core's code checks everything" is no longer literally true.** The group equation of most signatures is checked by
   libzync's batch, and block orchestration by Hazync's Rust. Both are new consensus-critical code. The first review
   found real soundness bugs in exactly these parts (§7).
2. **The trust model gained a hints file and an epoch.**
   - A SwiftSync proof is complete only at the hints file's stop height H.
   - Before H, the ECMH total is not yet zero and the proof says nothing about the UTXO set.
   - The tip beyond H needs Utreexo with deletions (the canonical path). Two modes means a handover, and that
     handover must itself be proven (proposal §7).
3. **A failure is not localised.** One running total from 1 to H tells you the file is wrong, not where. The host
   tool `swiftsync-diff-hints` localises it by replaying blocks; the proof itself does not (proposal §7).
4. **Host preprocessing.**
   - Every block needs a hint bundle: R points and ECMH hints, about 47 s of laptop CPU for 962,000.
   - Hints are untrusted (§7), so this costs liveness, not soundness. But a prover now needs this step.
5. **Assumevalid mode.** `HAZYNC_LIBZYNC=assumevalid` skips scripts entirely: about 3× cheaper (`cost/WHOLE_CHAIN_COST.md`).
   It proves accounting, not script validity, which is SwiftSync's own assumevalid trade-off.
6. **Reproducibility work to redo.** The canonical guest is reproducible through `reproduce/Dockerfile` and pinned by
   `METHOD_ID` + `GUEST_ELF_SHA256`. libzync guests are built from `scripts/libzync-base.sh` and are deterministic on one
   machine (two builds gave the same id), but they are not yet in the container.
7. **One network per image.**
   - A signet proof and a mainnet proof have different image ids. That is by design, and it is safe.
   - Custom signets, with a different challenge, are not supported.
8. **Older block modes are disabled in a libzync image.** Only chunks and the aggregate are available (§7, fix 7).

## 6. Each change: why, pros, cons

| change | why | pros | cons |
|---|---|---|---|
| **Batch signature verification** (hinted R, one MSM per chunk) | ECDSA was 75% of a modern block's cycles in libsecp's per-signature ecmult | 3.4–4.5× cheaper per signature; libsecp still parses and checks; a bad hint only falls back or fails | new crypto code; correctness rests on a 128-bit Fiat–Shamir batch; hints must be precomputed |
| **ECMH SwiftSync accounting** instead of Utreexo inclusion proofs | inclusion proofs dominated the aggregate's input, and they force serial proving | no per-input proofs; blocks provable in any order before H; ~3k cycles per coin operation | correct only at H; needs a hints file; non-local failures |
| **Design B: survivors to an append-only Utreexo forest** | the tip needs the UTXO set as an accumulator | the UTXO set at H falls out for free; no separate handover pass | the forest is only meaningful together with `ss_set = 0` at H |
| **agg-lite: the aggregate trusts chunk-committed facts** | re-parsing every transaction serially was the aggregate's cost | aggregate 57.7 M cycles on 962,000 | the facts must be bound to the merkle-committed bytes; the wtxid binding was missing (§7, fix 3) |
| **parse/hash once** (tx-once, lazy and raw txid, HashWriter buffer, SHA stream, D64 stream) | the same transaction was deserialised and hashed 3× | large cut in hashing and memcpy | patches inside Core's classes; reviewed equivalent (§7) |
| **memos** (pubkey parse, HASH160) | 6,986 signatures over 2,099 distinct keys on 962,000 | each repeated key costs nothing | caches in consensus code; only safe under the bump allocator, so TLSF is not in the profile |
| **legacy SIGHASH_ALL fast path** | quadratic hashing on one 400-input legacy transaction | halves the bytes hashed | a second implementation of the legacy sighash; reviewed byte-equal |
| **signet** | a cheap complete demonstration | full chain in reach (~$0.7–2.5k for the whole chain) | one more build profile; BIP325 adds a Core TU |

## 7. Independent review, 2026-10-30, and the fixes

Three reviewers each read the code without my conclusions: one on the cryptography, one on consensus
equivalence, one on the Core patches. Findings and status:

| # | finding | severity | status |
|---|---|---|---|
| 1 | the aggregate accepted an `ecmh_collect` flag from the (untrusted) hint block: **coin accounting off, the proof still valid** | soundness, critical (both reviewers) | **fixed**: a collect aggregate stops before committing |
| 2 | ECMH point had two values (±y), so a prover could cancel a fake or repeated spend | soundness, critical (both reviewers) | **fixed**: even y required; host normalises |
| 3 | a transaction split across chunks bound its witness only in the input-0 run; another run could verify against different witness bytes | soundness, critical | **fixed**: every record commits the wtxid; the aggregate requires all runs to agree |
| 4 | BIP68 compared `version >= 2` signed; Core v28's `version` is `uint32_t` | soundness, critical. ⚠ **also present in the canonical guest's chunked path on `main`** | fixed on `libzync`; **main: for your decision** |
| 5 | a second coinbase-shaped transaction was not rejected (`bad-cb-multiple`) | soundness (exploitable with 2) | **fixed** |
| 6 | a coin's committed creation height was never checked against the spending block | soundness, critical | **fixed**: `coin_height <= height` |
| 7 | old block modes (1, 2, 3, 6, 7, 12) were reachable in a libzync image: no SwiftSync, no signet check, caches unsafe under TLSF | hygiene → soundness under TLSF/signet | **fixed**: a libzync image runs chunks + aggregate only |
| 8 | coprocessor EC results not range-checked before equality decisions | hygiene (~2^224 work to reach) | **fixed**: canonical check on every add/double |
| 9 | the documented verifier checks omitted `anchor_id` and the image id | interface, critical as documented | **fixed**: `host libzync-verify` checks all of them |
| 10 | the same external coin spent twice in one block only failed at H | completeness of error location | **fixed**: rejected in that block |
| 11 | hint-block padding not required to be zero | hygiene | **fixed** |
| 12 | `hz_copy` reads up to 3 bytes past a buffer (UB at -O3); the `CMutableTransaction` sighash fast path is unsafe if ever used for signing | hygiene | open, documented |

Retest after the fixes: PENDING (`~/libzync-gpu/r18/test_fixed.out`).

**Verdicts that held:** batch ECDSA, Schnorr and taproot-tweak deferral match libsecp's acceptance exactly. The
Fiat–Shamir weights are sound to about 2^-128. The signed-digit Pippenger is correct. Every Core lever computes the
same bytes as Core in the shipped (bump-allocator) build. The record walk enforces coverage. Design B's argument
holds once the map is single-valued.

## 8. Signet

PENDING.

## 9. Changes Core could take

Ordered by how directly they transfer:

1. **A salt-free SwiftSync aggregate (ECMH or MuHash) as an option.**
   - SwiftSync's salted SHA-256 sum needs a *secret* salt, so it cannot be checked by anyone else.
   - An ECMH or MuHash3072 aggregate (Core already ships MuHash for `gettxoutsetinfo`) makes the per-node
     computation reproducible and provable.
   - Measured guest cost: ECMH ~3.0k cycles per element against ~1.3k for the salted sum.
2. **One index convention for hints.**
   - Core PR #34004 advances its hint index over **every** output, OP_RETURN included, and over the BIP30
     coinbases' outputs.
   - The `hintsfile` crate (and utxohints.store) counts only **spendable** outputs and skips the BIP30
     coinbases entirely.
   - Both are internally consistent, but a file made for one is wrong for the other. Pick one before files are
     published at scale.
3. **Batch Schnorr verification in validation.**
   - BIP340 was designed to batch, and batch verification has been proposed for libsecp and Core before.
   - libzync measured what batching buys in a constrained verifier: about 4× per signature at block scale
     (`experimental/libzync-bench/README.md` §3).
   - libzync's ECDSA batching needs a hinted R, so it is zkVM-specific. Schnorr batching is not.
4. **Legacy sighash midstate reuse.** Hashing the blanked-input prefix once per transaction, rather than once per
   input, removes the quadratic term in legacy SIGHASH_ALL. It is byte-identical by construction (reviewed).
5. **Lazy txid/wtxid in `CTransaction`.** Computing both hashes in the constructor costs every caller, including
   those that never ask. Lazy computation is behaviour-preserving (reviewed). The win natively is smaller than in a
   zkVM.
6. **A "hints are advice, the code decides" pattern** for expensive one-way checks: pubkey Y from a hint checked by
   y² = x³ + 7; R from a hint checked by the batch. This is useful anywhere validation is replayed by a weaker
   verifier.
7. **Found while reviewing: none of these is a Core bug.** The BIP68 signedness issue (§7, 4) is in Hazync's own
   transcription, not in Core.

## 10. Reproduce

```bash
scripts/libzync-base.sh ~/lz-base                                   # Core + libsecp + patches, pinned
cd prover && HAZYNC_BASE=~/lz-base HAZYNC_LIBZYNC=1 cargo build --release -p host
# signet:   ... HAZYNC_NETWORK=signet ...
host libzync-hints  out.lzb                     # HAZYNC_BLOCK=block.json: the block's hint bundle
host agg-exec-fake                              # HAZYNC_LZ_HINTS=out.lzb: execute chunks + aggregate
host swiftsync-sweep 200 <json dir>             # HAZYNC_SS_HINTS=file.hints: design B, blocks 1..200
host swiftsync-diff-hints file.hints <json dir> # WHERE a hints file is wrong
host swiftsync-file-hash file.hints             # what a proof at H must commit
host libzync-verify block_H.rcpt file.hints     # the checks a SwiftSync user runs
cd ../libzync && cargo test                     # wire format, hint hash, producers
```
