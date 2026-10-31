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

**Method:**
- Execute-mode `RISC0_PPROF_OUT` profiles of the fixed guest (`09a02eed…`), 4 chunks plus the aggregate.
- Each cycle is charged to the **innermost frame that is not runtime**, so memcpy, paging and the SHA
  accelerator count for the code that asked for them.
- Tool: `experimental/libzync-bench/cost/core_share.py`.
- Totals include paging, so they exceed the executor's user cycles.

| | chunks (406.6 M cycles) | aggregate (74.7 M) |
|---|---|---|
| **Bitcoin Core C++** (interpreter, sighash, CSHA256, transaction parsing, CheckTransaction, merkle, pow) | **46.7%** | 0.8% |
| **libsecp256k1** (incl. GHOST coprocessor field/lift-x shims) | 6.7% | 0.0% |
| **Hazync/libzync own code** | 46.7% | **99.2%** |
| ↳ of which the batch-verification MSM (guest-logged `finish_cycles`) | 78.5 M (19% of chunks) | — |

**How to read it:**
- **Chunks: roughly half the cycles run Core's own code.** That half is all the script work, sighash, hashing and
  parsing.
- **The other half is libzync's machinery:** the batch MSM, caches, leaf hashing and per-transaction records.
  libsecp's share is small because the expensive part of each signature, the group equation, now happens in the
  batch.
- **The aggregate is almost entirely Hazync's own code.** That is by design: agg-lite replaced Core's block-level
  orchestration with checks on chunk-committed facts. The Core code it still calls (`ComputeMerkleRoot`, PoW,
  retarget, chainparams) is a small part of its cycles.
- **Attribution limit:** inlined Rust in the guest crate shows up as an unnamed frame and is charged to libzync.

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

**Retest after the fixes** (guest `09a02eed…`; output in `experimental/libzync-bench/swiftsync/r18_fixed_tests.out`):

| check | result |
|---|---|
| block 962,000, valid | ✅ valid. Chunks 342.6 M cycles (+0.9%), aggregate 64.3 M (+11%: wtxid per record, the duplicate-spend sort, range checks) |
| ECMH hint with the odd root | ✅ refused: `ECMH: hinted y must be the even root` |
| a spend whose coin claims a creation height after the block | ✅ chunk `all_valid = 0` |
| design B sweep 1..200 | ✅ BALANCED; forest 204 = the coins unspent at 200; `ss_hints` `e33c66b3…` = the file's hash (unchanged) |
| one hint bit flipped, either way | ✅ NOT BALANCED (both) |
| `swiftsync-diff-hints` on the exact file / one toggled entry, both directions | ✅ EXACT / names exactly that height, index and outpoint, and for a spent coin the height that spent it |
| sweep 1..200 wall time | 40 s (382 s before the cached guest image) |

### Second review round (same day, after the fixes)

Three new reviewers:
- one re-checked every fix above;
- one made a fresh adversarial pass over the guest, without the earlier findings;
- one reviewed the verifier and the host tooling.

**All 11 fixes were confirmed correct and complete.** New findings:

| # | finding | severity | status |
|---|---|---|---|
| 13 | a coinbase flag of **2**: the leaf reads it as coinbase, maturity tested `== 1`, so an immature coinbase output could be spent | soundness, critical | **fixed**: flag must be 0 or 1 |
| 14 | the BIP30 coinbase-SMT root was never carried between blocks, so each block's starting tree was the prover's choice | soundness (exploitable only by re-mining pre-BIP34 history) | **fixed**: `ChainState.smt_root`, checked every block; the host keeps one real tree across sweeps and chains |
| 15 | an external spend claiming the current block's height was only caught at H | completeness | **fixed**: flagged by the chunk, rejected in that block |
| 16 | `libzync-verify` accepted a dev-mode FakeReceipt when `RISC0_DEV_MODE` was set | verifier soundness | **fixed**: fake receipts and the dev-mode variable are refused |
| 17 | with a supplied anchor, `libzync-verify` printed the same OK line as a genesis proof | verifier soundness for anyone automating on it | **fixed**: `LIBZYNC_VERIFY_DEMO`, exit code 2 |
| 18 | a hints list that is not strictly ascending was sorted, then accepted | verifier soundness against the spec | **fixed**: rejected |
| 19 | a scripts-off image (assumevalid, or a bare `HAZYNC_NO_SCRIPTS=1`) was not labelled as such | misleading output | **fixed**: recorded in the profile and printed by the verifier |
| 20 | a prebuilt guest's network/profile labels came from the shell, not the pinned bytes | hygiene | **fixed**: the pin directory carries a `PROFILE` file, which must match |
| 21 | `swiftsync-diff-hints` could call a file with duplicate indices EXACT | misleading output | **fixed** |
| 22 | no tool wrote a genesis anchor | completeness | **fixed**: `libzync-anchor-json --genesis` |
| 23 | the claim "every Core rule" includes `time-too-new` (MAX_FUTURE_BLOCK_TIME), which needs a clock | documentation | **closed in round 4**: the proof commits `max_time` and the verifier applies the rule |

### Third round: adversarial audit (same day)

Two fresh auditors: one on the guest, one on the verifier and spec.

| # | finding | severity | status |
|---|---|---|---|
| 24 | **`is_base` was a free u32: any value other than 0 or 1 skipped the previous proof's verification and copied `anchor_id` from the unverified journal**, so a forged "genesis-anchored" state at H−1 plus one real block passed every verifier check | soundness, critical. ⚠ **The canonical guest on `main` has the same code** (`chain_step` and `aggregate`) | fixed on `libzync` (must be 0 or 1); **main: for your decision, urgent** |
| 25 | the proof covers the hints file as `hintsfile` 0.2.1 decodes it; that decoder accepts non-canonical encodings and trailing bytes | verifier vs spec | **fixed**: `libzync-verify` and `swiftsync-file-hash` require canonical bytes |
| 26 | BIP30 is enforced for coinbases only; a non-coinbase descendant of a duplicated coinbase could recreate an unspent outpoint on a fabricated pre-BIP34 chain | soundness-theoretical (not on mainnet history) | documented (proposal §2) |
| 27 | spec: "Utreexo nodes need only the roots" (it is Hazync's own variant); §5 missing the time-too-new step; the evidence predated the audit images; epoch hash chaining unspecified | spec | **fixed** in the proposal |
| 28 | the assumevalid label is build metadata, not carried in the receipt | interface | **closed in round 4**: `ChainState.profile`; reproducible builds still needed to trust M itself |

**Verdicts that held:** batch ECDSA, Schnorr and taproot-tweak deferral match libsecp's acceptance exactly. The
Fiat–Shamir weights are sound to about 2^-128. The signed-digit Pippenger is correct. Every Core lever computes the
same bytes as Core in the shipped (bump-allocator) build. The record walk enforces coverage. Design B's argument
holds once the map is single-valued.

### Round 4: closing the open items (2026-10-30)

| item | change |
|---|---|
| `time-too-new` | `ChainState.max_time` = the largest block timestamp so far. `libzync-verify` requires `max_time <= now + 7200` on its own clock, as Core does for every block during IBD. OpenTimestamps was considered and does not help: it bounds a time by Bitcoin block times, which are miner-set within the same 2 hours |
| scripts-on vs assumevalid in the proof | `ChainState.profile`: bit 0 libzync, bit 1 scripts SKIPPED, bit 2 signet. Bit 1 comes from `hz_no_scripts()` in `verify_input.cpp`, the file that does or skips the checks. In a libzync image (always the tx-once path, where the `#ifdef` acts) the label is exact; a non-tx-once build with the flag would check scripts yet claim it skipped them, the safe direction. A recursive step requires the previous profile to equal its own; the verifier requires the profile it was built for |
| Utreexo format | standalone spec [`UTREEXO_FORMAT.md`](UTREEXO_FORMAT.md) with a test vector, and an independent implementation `scripts/libzync/utreexo_ref.py` that checks the guest's committed forest from the block files alone |
| Utreexo compatibility | measured (bench §7): a SHA-512/256 parent costs 14,530 cycles against 929. The blocker is that utreexod's roots depend on deletion history, so compatibility means proving every deletion |

Guests: mainnet `ddcaf1d0…`, signet `c3501ac9…`.

### Round 5: audit of round 4 and the verifier (2026-10-31)

Three fresh reviews: a code review of `196086d..` (round 4), an adversarial pass over the guest, and one over the
verifier, specs and evidence. The guest held: no way was found to make it commit a wrong state, and the round-4
fields are bound (`max_time` through `anchor_id`, `profile` as a constant of the image). Every fix below is
host-side or documentation, so the image ids are unchanged.

| # | finding | severity | status |
|---|---|---|---|
| 29 | **`verify-chain` on a libzync build accepted a libzync receipt** with none of libzync-verify's checks (hints file, `ss_set == 0`, profile, time-too-new, dev-mode refusal): a chain spending a fabricated coin printed `CHAIN PROOF VERIFIED` | verifier soundness, critical for that command | **fixed**: refused on a libzync build; on a canonical build it now refuses fake/dev-mode receipts and applies time-too-new |
| 30 | `HAZYNC_VERIFY_NOW` (test clock) was honoured silently; an unparsable value fell back to the real clock | verifier hygiene | **fixed**: when set it is itself a failed check, so the verdict is never OK or DEMO |
| 31 | the OK verdict line did not say ASSUMEVALID | interface (same class as #17) | **fixed** |
| 32 | §5 step 3 (genesis `anchor_id`) could not be implemented from the spec | spec | **fixed**: field values and encoding in the proposal, an independent `scripts/libzync/genesis_anchor_id.py`, test vectors; it matches the guest on both networks (and caught that genesis `utxo_roots` is `[None]`, not empty) |
| 33 | signet evidence: the pod's own verify **failed** on `anchor_id`; the passing verify used `anchor_effective.json`, whose BIP30 tree the prover seeded from the range's coinbase spends; the docs said "verified" and that the state equalled the sweep's apart from `anchor_id` (`smt_root` differs too) | evidence integrity | **fixed** in the docs; the scripts were fixed in round 6 (#40) |
| 34 | the GPU deadline watcher terminated the pod without fetching its output | evidence loss | **fixed**: the deadline harvests first; the main loops stop when it fires |
| 35 | an empty METHOD_ID pin passed `prep.sh` (`grep -q ""`) | pin check | **fixed**: a 64-hex id is required and matched as a word |
| 36 | `max_time` back-fill for old anchors used only the last 11 timestamps | hygiene (demonstration anchors only) | **fixed**: exact from the block files when they are present; a warning otherwise |
| 37 | unreadable inputs made `libzync-verify` exit 101 (a panic), outside the documented 0 / 2 / 1 | interface | **fixed**: `LIBZYNC_VERIFY_FAIL`, exit 1 |
| 38 | `swiftsync-file-hash` labelled a non-canonical file but exited 0, contrary to #25's status | interface | **fixed**: exit 1 |
| 39 | docs: per-block times counted the base step as a transaction block; the Utreexo spec's "exactly the coins unspent at H" missed the mainnet 91,722 / 91,812 window; "the label cannot disagree" overstated (exact in the tx-once path every libzync image uses); `tx_out_leaves`' parameter name and comment are stale | docs | **fixed** in the docs; the `.cpp` comment is left, since editing the guest source moves the image id |

**Considered and not changed:** the aggregate's `prev.profile == chain_profile()` check is implied by `self_id`
(kept as a cheap second statement of it); `chain_step` has no profile check (unreachable in a libzync image); the
canonical image id on this branch differs from `main` (expected since audit 3); a float conversion in a fleet
fetcher (false: it parses with `Decimal`).

### Round 6: code review of round 5, and the C++ side (2026-10-31)

A code review of `167789e..14a7b49`, and a fresh audit of `verify_input.cpp` and both patches (last reviewed
independently in round 1) against unpatched Core v28 and libsecp 0.5.1. **The C++ side holds:** every lever in 0101
and hook in 0102 accepts exactly what unpatched Core and libsecp accept for any prover input. Batch ECDSA, Schnorr and
the taproot tweak defer only after libsecp's own parse and range checks; a hint can only route a signature to libsecp
or make the batch fail; `finish()` runs on every path before a commit. The legacy-sighash fast path matches Core's
preimage byte for byte. The raw-txid seeding relies on Core's canonical deserialiser.

| # | finding | severity | status |
|---|---|---|---|
| 40 | round 5 fixed the docs but not the scripts: the bundle still shipped only 2,501..2,600, and `pod/run.sh` verified against the prover-written `anchor_effective.json`, a circular check | evidence integrity | **fixed**: the bundle ships blocks 1..2,600 (13 MB) so the tree is replayed; the pod verifies against the sweep's `anchor.json` |
| 41 | the deadline watcher and the main script could both harvest and terminate at once; and (found testing the fix) a failed fetch skipped the terminate | evidence loss, cost leak | **fixed**: an atomic lock makes exactly one caller harvest, and it always terminates; tested with stubs |
| 42 | the `is_base = 2` negative test could never print `NOT_REFUSED` (`head`'s exit status) | test | **fixed** |
| 43 | `libzync-verify` with a missing argument still exited 101; a closed stdout could abort the panic hook (134) | interface | **fixed**: hook installed before the arguments are read; its write ignores errors |
| 44 | an empty `HAZYNC_VERIFY_NOW` counted as an override | interface | **fixed**: empty = unset, as for `RISC0_DEV_MODE` |
| 45 | dev-mode and time-too-new logic duplicated across the two verifiers; `max_time` fill parsed every block file twice | maintenance, cost | **fixed**: shared helpers; the fill comes from the replay's own pass |
| 46 | round-5 evidence not reproducible from committed scripts | evidence | **fixed**: `audit_r5/run_r5.sh` regenerates both outputs |
| 47 | C++: `tx_full_sigops` returns legacy-only cost when the prevouts blob is empty (safe only because the chunk already trapped on it); `uint32_t` wraparound in `check_input_locks*` (paths not in a libzync image); `hz_copy` also reads up to 3 bytes **before** the source (addendum to #12; data correct) | hygiene | **open**: each is a guest-source change that moves the image id; batch them with the next image change |

## 8. Signet

**Build:** `HAZYNC_NETWORK=signet`, guest `c3501ac9…` (round 4; first built as `cf7f75ff…`). What changes:
- Core's own `CChainParams::SigNet` (default challenge) replaces `Main()`;
- Core's `signet.cpp` is compiled in;
- the aggregate calls Core's unmodified `CheckSignetBlockSolution`;
- every network constant the guest uses (`libzync/src/net.rs`) is asserted against Core's compiled chainparams at
  run time: genesis, powLimit, and buried heights all at 1.

The aggregate holds only txids, not the block's other transactions. So the check is given placeholder transactions
whose txid is seeded with the merkle-bound value. Core's code reads nothing else from them (third reviewer: exactly
equivalent).

**Measured on the laptop** (execute mode, signet blocks from a local `bitcoind -signet`):

| check | result |
|---|---|
| BIP325 alone (mode 16), block 2,148 (400 inputs) and block 100 (coinbase-only) | real block → 1; one txid changed → 0 (both) |
| our hints file at 3,000 vs the **published** signet file (stop 320,000) | **SUBSET_OK**: all 606 coins the published file marks unspent sit at the same index in ours |
| `swiftsync-diff-hints` on our 3,000 file | EXACT for 1..=3,000 |
| design B sweep 1..3,000 | **BALANCED** in 687 s; forest = **695 coins** = the UTXO set at 3,000; `ss_hints` `5979d283…` = the file's hash |
| design B sweep 1..2,600 (the GPU demo's handover height) | **BALANCED**; forest = 293 coins; `ss_hints` `f097b8f2…` |
| `libzync-prove-range 2501 2530` in dev mode (fake receipts) | base step on the 2,500 state, then 29 recursive steps, including 9 transaction blocks |

**GPU** (proposal §10):
- first run (pre-audit `cf7f75ff`): signet 2,501..2,600 in 1,464 s ($1.39), verified;
- **re-proved on the round-4 image `c3501ac9`:** 1,452 s of proving on 1× RTX PRO 6000, ~$1.52; verified on the
  laptop (DEMO verdict) against the prover-written `anchor_effective.json`, whose BIP30 tree was seeded from this
  range's coinbase spends; the pod's own verify against the sweep anchor failed on `anchor_id` (proposal §10);
  receipts 225–227 KB; evidence in `experimental/libzync-bench/signet/gpu_r4/`;
- 8.7–9.2 s per coinbase-only recursive block, 14.7 s for the base step, 16.5–16.7 s per small transaction block,
  514 s for block 2,523.

**Found on the GPU run: the BIP30 coinbase-txid SMT dominates blocks that spend many coinbases.**
- Signet block 2,523 spends 374 coinbase outputs (514–549 s on the GPU). Its aggregate executes **565 M cycles**, of which **557 M (97%)** is
  `coinbase-smt` (SHA-256 path updates) at about **1.5 M cycles per coinbase spend**. Every other part of the block
  costs 18 M.
- The SMT is existing Hazync machinery (#54), not libzync. It closes BIP30 permanently, which is stricter than Core:
  `ConnectBlock` skips the BIP30 check between BIP34 activation and height 1,983,702.
- A cheaper rule there (or batched SMT updates) would remove this cost wherever coinbases are spent in bulk. That
  happens in mining-pool payout blocks on mainnet, and in faucet sweeps on signet.
- **Open lever, not changed.**

**Signet's first 2,147 blocks are coinbase-only.** Transactions start at 2,148. A full signet run (to 320,000,
against the published file) is the cost-table item in the proposal, §9.

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
