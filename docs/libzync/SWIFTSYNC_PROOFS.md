# Proven SwiftSync hints: a proposal (draft v0.1, 2026-10-30)

**For:** Ruben Somsen, Robert Netzke, and anyone building on SwiftSync.
**From:** the Hazync project (`libzync` branch).
**Status:** working code and real measurements, as execute-mode sweeps and GPU proofs (signet, §10). The design has
had eight review rounds, which found bugs that are now fixed (`docs/libzync/LIBZYNC.md` §7). ⚠ **No receipt yet
reaches the full verdict:** every GPU proof so far starts from a trusted mid-chain state, and none is on the current
image (§10). The numbers say which image they were measured on, and estimates are marked as such. It still needs
outside eyes.

## 1. The problem this solves

- **SwiftSync makes initial block download cheap and parallel,** but it needs a **hints file**: one bit per output,
  "still unspent at height H?".
- **A wrong file cannot make a node accept a bad chain.** The aggregate check at H fails, and the node has wasted a
  sync. So the cost of a bad file is denial of service, and the question a user faces is **"who made this file, and
  should I spend a sync on it?"**
- **A Hazync proof answers that question without trusting whoever made the file.** It shows that a specific file is
  exactly right for a specific chain, and it hands over the UTXO set at H as a Utreexo accumulator. That is enough to
  skip IBD altogether. What it does rest on: the image M is the published code (rebuild it with `reproduce/libzync.Dockerfile`,
  §5 step 9), the soundness of risc0's proof system and recursion, and the assumptions of §4.

## 2. What is proven

For a receipt R produced by the Hazync libzync guest with image id M, journal J and a hints file F, the following
statement holds if the checks in §5 pass:

> There is a chain of blocks 1..H starting from the network's genesis, ending at `J.tip_hash` with cumulative work
> `J.cum_work`. Every block satisfies the consensus rules the guest enforces, which follow Bitcoin Core v28 except
> where listed below: PoW, retarget, MTP, merkle (including mutation), witness commitment, weight, sigops, subsidy and
> fees, BIP16/30/34/65/66/68/112/113/141/143/147/341/342, and scripts (unless M is an assumevalid image), plus BIP325
> on signet. And:
> 1. **F is exactly right:** for every height h ≤ H, F lists precisely the indices of h's indexed outputs that are
>    unspent at H.
> 2. **The UTXO set at H** is exactly the leaves of the Utreexo forest `(J.utxo_roots, J.utxo_leaves)`, in creation
>    order, except on mainnet for H in 91,722..91,879, where Core's set still holds the outputs of the coinbases of
>    91,722 and/or 91,812 until they are overwritten; the forest never holds them (`UTREEXO_FORMAT.md` §1).

**Where the rules come from.** Scripts, signatures, sighashes, transaction parsing and checks (`CheckTransaction`,
`CheckTxInputs`), sigop counting, `IsFinalTx`, BIP34 and the witness commitment run Bitcoin Core's own C++ (v28.0
with the patches in `patches/`). Signatures are verified by a batch equation instead of one libsecp call each, with
the same acceptance as libsecp (`LIBZYNC.md` §7, round 6). The block-level bookkeeping (coin accounting, maturity,
BIP68, weight and sigop totals, subsidy, BIP30) is transcribed in Rust (`prover/methods/guest/src/agg_lite.rs`). A
rule-by-rule table against Core's reject reasons is not written yet.

**Known differences from Core:**
- **BIP30 for non-coinbase transactions (audit 3, #26).** BIP30 is enforced for coinbase transactions through the
  coinbase-txid SMT. Core's rule also covers a non-coinbase transaction recreating a still-unspent outpoint. That can
  only happen as an exact descendant of a duplicated coinbase, and needs pre-BIP34 rules, so mainnet history (whose
  only duplicates are the two grandfathered coinbases) is unaffected. A fabricated low-work chain could still exploit
  it; step 7 of §5 (the tip is on your most-work chain) is what excludes one.
- **BIP30 for coinbases is stricter than Core.** The SMT rejects every duplicate coinbase txid. Core's `ConnectBlock`
  skips the BIP30 check between BIP34 activation and height 1,983,702, the first height where BIP34 allows a
  duplicate. Before that height the two agree.

**Two rules depend on the node, not the chain:**
- **`time-too-new`.** A block timestamp more than 2 hours ahead of the node's clock. A proof has no clock, so it
  commits the largest block timestamp in the chain (`max_time`) and the verifier applies the rule to that one number
  (§5, step 11). This is what Core does for every block during IBD. It matters because a far-future timestamp at an
  epoch's last block lowers the next difficulty.
- **Best chain.** The proof says this chain is valid, not that it has the most work. See §5, step 7.

## 3. Objects

### The hints file: hintsfile v0, unchanged

- **Format:** magic `UTXO` (4 bytes), version `0x00` (1 byte), stop height `H` (u32 LE). Then, for each height 1..=H,
  an Elias–Fano coded list of the **indices** of that block's outputs that are unspent at H.
- **Canonical encoding of one list** `i_1 < … < i_n` (what `hintsfile` 0.2.1's encoder emits, and what §5 step 10
  requires):
  - `n` as a CompactSize (1 byte for ≤ 0xFC, `0xFD` + u16 LE, `0xFE` + u32 LE, each the shortest form). If `n = 0`
    that byte is the whole list.
  - `m = i_n` as a CompactSize.
  - `l = ⌊log2((m + 1) / n)⌋` (integer division).
  - **Low bits:** the low `l` bits of each element, element by element, each from bit 0 up, packed into bytes
    least-significant bit first; `⌈n·l / 8⌉` bytes, unused bits zero.
  - **High bits:** for each element, `(i_k >> l) − (i_{k−1} >> l)` zero bits (with `i_0 >> l` taken as 0), then a one
    bit, packed least-significant bit first; `⌈(n + (m >> l)) / 8⌉` bytes, unused bits zero.
  - Nothing follows the list for height H.
- **Indexing** (the `hintsfile` crate's convention, which Hazync follows exactly):
  - only spendable outputs are counted: not OP_RETURN, not scripts over 10,000 bytes;
  - order: the coinbase first, then each transaction in block order, outputs in vout order;
  - on mainnet the coinbase outputs of blocks 91,722 and 91,812 (BIP30, Core's `IsBIP30Unspendable`) are not counted.
    The check uses height **and hash**, so on signet those heights are ordinary blocks.
- ⚠ **Core PR #34004 numbers outputs differently.** It counts every output, including unspendable ones and the BIP30
  coinbases' outputs. Section 8 asks to settle this.

### The commitment to F: `ss_hints`

```
h_0 = 0^32
h_k = SHA-256( "libzync/swiftsync-hints/v2" || h_{k-1} || k (u32 LE) || n_k (u32 LE) || i_1 .. i_n (u32 LE each) )
ss_hints = h_H
```

Here `i_1 < … < i_n` are F's indices at height k. The value needs **only F** to compute. Reference implementations:
- `libzync/src/ss_hints.rs::step`, used by both prover and verifier;
- `host swiftsync-file-hash <file>`.

**Test vectors:**

| file | H | `ss_hints` |
|---|---|---|
| utxohints.store mainnet `bitcoin.hints` (122.5 MB) | 970,000 | `e215fe7e73298dd9dcd509de6c772f47704068485f187dc3fed158d793b1171d` |
| utxohints.store signet `signet.hints` (25.0 MB, sha256 `a3c869625b278d53f957693c38aa894e4380a61ca7ed5a7e17a9613ff9e56218`) | 320,000 | `d9c88278135e495749dd04e8a635a444e99d6dc470a43d11f37ecd91f02b4947` |
| mainnet blocks 1..200, `experimental/libzync-bench/swiftsync/h200.hints` (607 B, sha256 `d1b2b93eab899748a128816496524205e3e29afcf7ac19f8832786163b849c0f`) | 200 | `e33c66b38a04cbf2e4cdf1813a6530a66121234d13a68e0a6f3ecd17907c4ccd` |
| signet blocks 1..2,600, `experimental/libzync-bench/signet/signet2600.hints` (3,081 B, sha256 `ca061f2a643ef05d39fb1afdd28f3d22688cbf5e56c8cc23e162a6c8a3380d98`) | 2,600 | `f097b8f2a52a23d5e75db23b24e9a473a31b804534ac43592d816806c7722f7b` |
| signet blocks 1..3,000, `experimental/libzync-bench/signet/signet3000.hints` (sha256 `357e2ccadf2f3f7ad8513f344675d6195a96684ccc16f15076306d2a74848334`) | 3,000 | `5979d283d04530bc35fd450b5b99fb89ad2cbb9f9c640b1038c8c72f837ed779` |

The last three files were written by `host swiftsync-make-hints`; each matches the published file's numbering
(§10).

### The proof's public output: `ChainState`

| field | meaning |
|---|---|
In field order (the journal encoding is in §5 step 3):

| field | meaning |
|---|---|
| `kind` | `0xC4A10002` (domain tag) |
| `tip_hash` | the chain proven: its last block's hash, internal byte order |
| `utxo_roots`, `utxo_leaves` | Utreexo forest of the coins marked unspent-at-H, in creation order |
| `cum_work`, `height` | cumulative work (32 bytes LE) and height of the tip |
| `prev_nbits`, `prev_time`, `epoch_start`, `recent_times` | difficulty and time context, so the chain can be extended |
| `anchor_id` | double-SHA-256 of the state the chain started from (genesis for a full proof) |
| `self_id` | the image id every recursive step ran |
| `ss_set`, `ss_acc` | the ECMH running total (`ss_set = 0` means the identity) |
| `ss_hints` | the running hash of §3 |
| `smt_root` | root of the coinbase-txid SMT (BIP30) after the tip |
| `max_time` | the largest block timestamp in the chain (for `time-too-new`, §5 step 11) |
| `profile` | what the image checked: bit 0 libzync, bit 1 scripts and signatures **skipped** (assumevalid), bit 2 signet. Set by the image from its compiled code, never carried from input |

## 4. How the guest checks a block (design "B")

For every block h ≤ H, each **indexed output** k gets the bit b_k from F:

| event | action |
|---|---|
| output created, b = 0 (spent by H) | `ECMH += P(leaf)` |
| output created, b = 1 (unspent at H) | append `leaf` to the Utreexo forest |
| an input spends an output created earlier in the same block | cancelled in the block; it must have b = 0 (else reject) |
| an input spends an older coin | `ECMH -= P(leaf)` |

- **The leaf** is the byte string of `UTREEXO_FORMAT.md` §2: a tagged SHA-256 of outpoint, amount, length-prefixed
  script, height, coinbase flag and creation MTP, so a spend has to name the coin exactly as it was created.
- **P** is a hash to secp256k1:
  - x = SHA-256(leaf ‖ ctr) read as a big-endian integer, with ctr a u32 little-endian counter from 0, taking the
    **first** ctr whose x is on the curve; every skipped ctr is proven off-curve by a hinted non-residue witness;
  - y is the **even** root;
  - a hash ≥ p makes the proof fail (probability about 2^-224), so the map stays single-valued.
  - So each leaf has exactly one point.

**Why the forest is the UTXO set at H.** Let A be the multiset of points added (b = 0 outputs) and S the multiset
subtracted (spends of coins from earlier blocks). `ECMH == identity` at H means ΣA = ΣS. Except with negligible
probability this forces A = S as multisets: finding distinct multisets with equal sums breaks the collision
resistance of ECMH, which holds for a random-oracle hash to the curve under the discrete-log assumption on secp256k1
(Maitin-Shepard, Tibouchi, Aranha, "Elliptic Curve Multiset Hash", 2016). From A = S:
- every b = 0 coin is spent exactly once by H, and every spend of an older coin names a coin that was created with
  b = 0 (a spend of a b = 1 coin would subtract a point never added);
- so F is right on every b = 0 output, and every b = 1 output is never spent by H: the forest holds exactly the coins
  unspent at H.

This needs:
- **Distinct leaves for distinct coins.** A leaf commits txid, vout and the creating height. Within a block, two
  transactions with one txid would spend the same coins, which the guest rejects as a double spend (and the
  malleable case fails the merkle-mutation check), and there is one coinbase. Across blocks, a repeated outpoint is
  BIP30's case, which the coinbase SMT covers for coinbases (with the #26 gap of §2 for their descendants).
- **F bound to the blocks.** Each block must carry exactly as many hint bits as it has indexed outputs (the guest
  rejects any other count), with unused bits zero. Every height 1..H steps `ss_hints`, including empty lists, so a
  block proved without hints leaves the hash where it was and the final value cannot equal `ss_hints(F)`.
- **A sound recursion.** The base step is not verified, so the statement holds only for a proof whose `anchor_id`
  is the genesis anchor (§5 step 3), and it rests on risc0's recursion being sound.
- **Which chain.** F carries no block hashes. The proof says F is right for the chain ending at `J.tip_hash`; a reorg
  below H makes it a statement about a chain you no longer follow (§5 step 7).
- **Genesis.** The genesis coinbase is not spendable and not in the set, so the forest starts empty.

**Why ECMH and not the salted sum.**
- SwiftSync's salted SHA-256 sum is secure because the salt is **secret** to the syncing node.
- A proof must be checkable by anyone, so it cannot have a secret.
- ECMH's security is the discrete-log hardness of secp256k1 (about 128 bits), with no secret.
- Cost in the zkVM: ~3.0k cycles per element against ~1.3k for the salted sum.
- MuHash3072, which Core already ships, would also work without a secret. It has not been measured in the guest;
  the choice of ECMH came from reusing the accelerated secp256k1 arithmetic the guest already has.

## 5. Verifier procedure: all of it

Given a receipt R, the hints file F and the network:

1. `R.verify(M)`, where M is the published libzync image id for that network (and for scripts or assumevalid). The
   current images (round 8, scripts checked, built by `reproduce/libzync.Dockerfile`): mainnet `9f469930c612139342e4f2a403eccba7663b1231ecc53b396470c6fe93e72f55`, signet `067822b958f7cc86d67ad9f37153b99c29664072a8e90a79a2d9af4dc71f5b36`,
   pinned in `reproduce/LIBZYNC_IDS`.
2. Decode the journal as `ChainState` J and check `J.kind == 0xC4A10002` and `J.self_id == M`.
3. `J.anchor_id == dsha256(genesis ChainState journal for the network)`.
   **This is essential:** the base step of the recursion is not verified, so a proof anchored anywhere else proves
   nothing about history.
   - The genesis ChainState, in field order: `kind = 0xC4A10002`; `tip_hash` = the genesis hash (internal byte
     order); `utxo_roots` = a list of **one** empty entry (`[None]`, not an empty list); `utxo_leaves = 0`; `cum_work` = `GetBlockProof(genesis nBits)` as 32 bytes
     little-endian; `height = 0`; `prev_nbits` = genesis nBits; `prev_time = epoch_start` = genesis time;
     `recent_times = [genesis time]`; `anchor_id` = 32 zero bytes; `self_id = M` (so the genesis journal, and its
     digest, are per image; M's 8 words are its 32 bytes read as little-endian u32s, in order); `ss_set = 0`;
     `ss_acc` = 16 zero words; `ss_hints` = 32 zero bytes; `smt_root` = the empty coinbase-SMT root, `e_256` where
     `e_0` = 32 zero bytes and `e_{d+1} = SHA-256(0x11 ‖ e_d ‖ e_d)`
     (`507d86c51845c467583449a973898eff681f18e74aa6d62b206da023cbcaec32`); `max_time` = genesis time; `profile = 0`.
   - The journal is risc0's serde word encoding (risc0-zkvm 3.0.5 `serde::to_vec`, each word little-endian): every
     integer up to 32 bits and every byte of a fixed array takes one u32 word, a u64 two (low word first), a list is its length then
     its elements, and an `Option` is a 0/1 word followed by the value when present.
   - `host libzync-verify` computes this itself. `scripts/libzync/genesis_anchor_id.py` computes it from this text
     alone, and matches what the guest commits after a base step on genesis. Test vectors:
     mainnet `9f469930…` → `5b2020ae48fdf2d180837b612b13c2b36192208f4afb94dee350553fa7ea1d44`;
     signet `067822b9…` → `636271d3f46ea120f22da161648247bdd004afba1cb92378ba3af407c9e4e2b2` (round-8 images).
4. `J.height == F.stop_height`.
5. `J.ss_set == 0`.
6. `J.ss_hints == ss_hints(F)` (§3).
7. `J.tip_hash` is the block at height `J.height` on your own most-work header chain, whose headers you have checked
   for `time-too-new` (§2). The proof says that chain is valid, not that it is the best one.
8. The receipt is a real proof: refuse a dev-mode `FakeReceipt`, and never verify with `RISC0_DEV_MODE` set.
9. Read `J.profile`: bit 1 set means scripts and signatures were **not** checked (assumevalid). The image sets the
   bit from the C++ that does (or skips) the checks, so the proof itself states it; check it is the profile you
   accept. M still has to be the image you trust: rebuild it from the published source commit with
   `reproduce/libzync.Dockerfile` and compare with `reproduce/LIBZYNC_IDS`. The id depends on absolute build paths, so
   only a build in that container gives the published id.
10. F is **canonically encoded** as specified in §3: re-encoding its decoded lists gives the same bytes, with nothing
    after height H. The proof covers the decoded lists. The `hintsfile` 0.2.1 decoder does not check padding, value
    bounds or trailing bytes, so a non-canonical file could decode differently in another implementation.
11. `J.max_time <= now + 7200`, with `now` from your own clock (Core's `MAX_FUTURE_BLOCK_TIME`).

`host libzync-verify <receipt> <file.hints>` does steps 1–6 and 8–11, and prints `tip_hash` and `cum_work` for
step 7. It ends in one of three verdicts:
- `LIBZYNC_VERIFY_OK` (exit 0): a genesis-anchored proof;
- `LIBZYNC_VERIFY_DEMO` (exit 2): a proof checked against a supplied anchor, which is never a proof of the file;
- `LIBZYNC_VERIFY_FAIL` (exit 1), including an unreadable or malformed receipt, anchor or hints file.

An OK verdict for an assumevalid image says so on the verdict line itself. `HAZYNC_VERIFY_NOW` replaces the clock in
step 11 for negative tests only; when it is set, that is itself a failed check, so the verdict is never OK or DEMO.

**Example output** (abridged: the `file:` line is dropped and the tip shortened). The real run of §10 on the round-4
signet image, re-verified on a laptop from the 226 KB receipt.
A demonstration anchor (§10) makes the verdict `LIBZYNC_VERIFY_DEMO`, never OK:

```
$ host libzync-verify block_2600.rcpt signet2600.hints anchor_effective.json
  [ok] receipt is a real proof (not a dev-mode FakeReceipt)
  [ok] RISC0_DEV_MODE is not set in this verifier's environment
  [ok] receipt verifies against METHOD_ID c3501ac900743bbc911c8f6dedcd71f1ee53fe4bea4f1cbc9726324478e63dae (libzync guest, signet, scripts checked)
  [ok] journal is a ChainState committed by this image (kind, self_id)
  [ok] profile in the proof 0x5 == this verifier's 0x5 (libzync, scripts checked, signet)
  [ok] time-too-new: largest block time 1599732600 <= this clock 1793397211 + 7200 s
  [ok] anchor_id is the GIVEN anchor at height 2500 (anchor_effective.json) -- NOT genesis
  [ok] the file is canonically encoded (re-encoding its lists gives the same bytes; no trailing data)
  [ok] height 2600 == the file's stop height 2600
  [ok] SwiftSync total is the identity (ss_set == 0)
  [ok] ss_hints f097b8f2a52a23d5e75db23b24e9a473a31b804534ac43592d816806c7722f7b == hash of signet2600.hints
  chain: tip ... (display order 00000127dfa07c0fd2f2fe1e22a0fea15c34cabd6204b1faac15a04ba28be6f3), cum_work 0x432982f4d
  UTXO set at H: 293 coins, Utreexo roots [...]
LIBZYNC_VERIFY_DEMO: consistent with signet2600.hints at height 2600 FROM THE GIVEN ANCHOR ONLY -- not a proof of the file
```

The anchor to pass is `anchor_effective.json`, which `libzync-prove-range` writes: for a demonstration anchor it
seeds the BIP30 tree and records the resulting root in the anchor.

The same tool **fails** a proof that stops early: a dev-mode receipt at 2,530 against the same file gives
`[FAIL] height 2530 == the file's stop height 2600` and `[FAIL] ss_hints …`. Without the anchor argument, check 3
fails for this demonstration proof, because it starts at 2,500 and not at genesis (§10).

## 6. What a SwiftSync node can do with it

1. **Verified hints.** Fetch F and R from anyone, run §5, then SwiftSync from genesis with full confidence that the
   sync will not fail at H.
2. **No IBD at all (assumeutxo, proven).** Fetch the UTXO set at H from anyone, rebuild the accumulator, check it
   against `J.utxo_roots`, check §5 against the tip you see, then validate from H onwards.
   - ⚠ **This is Hazync's own Utreexo variant**, not the utreexod/Floresta format:
     - leaf = SHA-256(0x00 ‖ txid ‖ vout ‖ value ‖ len(script) ‖ script ‖ height ‖ coinbase flag ‖ creation MTP);
     - parent = SHA-256(0x01 ‖ L ‖ R);
     - leaves in creation order.
   - The exact encoding (integer widths, endianness, which outputs, order, roots normalisation) is specified in
     [`UTREEXO_FORMAT.md`](UTREEXO_FORMAT.md), with a test vector and an independent Python implementation
     (`scripts/libzync/utreexo_ref.py`) that the guest's output is checked against.
   - An existing Utreexo node cannot use these roots as they are. The leaves commit each coin's creation-time MTP, and the forest is in creation order, so such a
   snapshot must carry both. A plain `dumptxoutset` does not.
   - **Compatibility is not a format change.** Utreexo roots depend on the add/delete history (a deletion
     moves the sibling up), so utreexod's roots at H are a function of every spend since genesis. Ours are
     an append-only forest of the coins that survive to H. Matching utreexod means proving every
     spend's deletion. Measured: a SHA-512/256 parent costs 14,530 cycles in the guest against 929 for
     ours; ESTIMATED total ~6× a current block's cycles
     (`experimental/libzync-bench/README.md` §7).
3. **Assumevalid flavour.** An assumevalid image skips script checks (about 3× cheaper to produce, §9). Its proof
   says exactly what SwiftSync's assumevalid mode assumes, so it is enough for verified hints, but it is not a full
   validation proof.

## 6a. Tip proofs vs proofs up to H

- **Before H:** a hints file exists for these blocks. Every output gets its bit: spent-by-H coins go through ECMH,
  survivors are appended to the forest. At H the total must be the identity, and the proof is SwiftSync-complete.
- **Past H, at the tip:** no hint bits exist yet. Our live tip runs (mainnet 973,752–755, past the published file's
  970,000) put every output into the ECMH total. That total is carried in the proof but **not yet checkable**, and
  it is the only thing that binds the coins a block spends. So a tip proof checks each block's PoW, merkle root,
  scripts and signatures **against the prevouts the prover supplied**; until a later epoch closes the ECMH total
  (§7), nothing proves those prevouts exist, so value, fees, maturity and BIP68 are not yet established and a tip
  proof alone does not rule out inflation. Those runs also proved each block on its own, from a state the host
  supplied, rather than as a chain.
- **The cost mix differs.** Ruben's point: by a week old, most outputs are spent. Under design B that means more ECMH
  (~3k cycles per output, added and removed once each) and fewer forest appends. On block 962,000 the ECMH is ~3% of
  the work, so the shift is small next to script and signature work. Measurements for the last blocks before
  H = 970,000, with the published file's real bits: §10a.

## 6b. Where the cost is (block 962,000, mainnet)

| | original Hazync guest | libzync, round 8 image |
|---|---|---|
| total guest cycles | 3.36 G | 409.5 M = chunks 345.2 M + aggregate 64.3 M (**8.2× fewer**) |
| signature validation | libsecp ECDSA ≈ **75%** of cycles | batched, one equation per chunk: **~19%** of chunk cycles |
| hashing, sighash, transaction parsing (Core's own code) | secondary | now the **largest** item: ~47% of chunk cycles run Core's C++ |
| ECMH (SwiftSync accounting) | — | **~3%** |

The shares in the last three rows are from a profile of a pre-audit image (`LIBZYNC.md` §4); the totals are round 8.

Remaining levers:
- per-proof fixed cost and recursion (about 8.7 s per empty block on one RTX PRO 6000);
- multi-block batching (not built);
- the BIP30 coinbase SMT (~1.5 M cycles per coinbase spend);
- mid-history transaction shapes (2–6× the cycles per transaction of today's blocks).

## 7. Epochs: moving H and localising failures (design; not built)

**Today:** one hints file, one running total from 1 to H.
- If F is wrong, the proof fails at H without saying where.
- Moving to a later H' means re-proving 1..H'.
- The host tool `swiftsync-diff-hints F <blocks>` names every wrong entry by replaying the blocks, but that is not part
  of the proof.

**Proposed: chained epochs H_1 < H_2 < …**

| | |
|---|---|
| **epoch k input** | an epoch file F_k for blocks (H_{k-1}, H_k]: one bit per output **created in the epoch**, "unspent at H_k?", plus the epoch-k start state = the proven `ChainState` at H_{k-1} |
| **coins created before H_{k-1}** | they are in the forest from earlier epochs. Spending one in epoch k is a **Utreexo deletion with an inclusion proof** (Hazync's canonical path already does this) |
| **coins created in epoch k** | design B as today: b = 0 → ECMH, b = 1 → appended to the forest |
| **at H_k** | `ECMH == identity` and `ss_hints == hash(F_k)`. The total restarts at the identity for epoch k+1. The epoch's hint chain must start from a value that binds H_{k-1} and the start state, not from 0^32, so an epoch file cannot be replayed against another range |
| **cost** | spends of pre-epoch coins carry inclusion proofs (the canonical Utreexo cost); everything else is the cheap SwiftSync path |
| **what you get** | a failure is confined to one epoch; H moves forward by proving one more epoch, not the whole chain; the tip phase is simply a very short epoch |

**Open questions for you:**
- Do epoch files fit SwiftSync's plans for refreshing hints?
- Should F_k cover only outputs created in the epoch, or every output up to H_k (one file per height range, or
  cumulative)?

## 8. Questions we would like your view on

1. **Index convention:** the hintsfile crate (spendable only, BIP30 coinbases skipped) or PR #34004 (every output).
   Proofs can follow either, but files must agree with their proofs.
2. **The commitment format** of §3. Should it be a hash over the raw file bytes instead? It could, if the hints-file
   spec adopted the canonical encoding of §3 for every encoder; today a proof covers the decoded lists and the
   verifier requires that encoding.
3. **ECMH vs the salted sum,** for a published aggregate.
4. **What the proof should output:** Utreexo roots, a MuHash of the UTXO set, both, or something else.
5. **Epoch spacing (§7),** and whether hints files will be published per epoch.

### Questions we expect, with what we can answer now

| question | answer today |
|---|---|
| Is there a genesis-anchored receipt that verifies OK? | Not yet (§10). Signet 1..2,600 from genesis is the next GPU run, once the image is final |
| How do I rebuild M? | `docker build -t hazync-libzync-repro -f reproduce/libzync.Dockerfile .`, then `docker run --rm hazync-libzync-repro`; compare with `reproduce/LIBZYNC_IDS`. About 55 minutes on a 16-thread laptop. Three builds on one machine agree (§5 step 9); a build on a second machine has not been done |
| What does the proof buy, if a wrong file only costs a sync? | Verified hints alone save one wasted sync. The larger product is the UTXO set at H with a proof (§6 item 2), which removes IBD |
| Why ECMH, not MuHash3072? | §4: reuse of the guest's accelerated secp256k1; MuHash not measured |
| Where does the proof differ from Core? | §2: BIP30 for coinbase descendants (#26), and the coinbase SMT being stricter after BIP34. A rule-by-rule table is not written yet |
| Batch ECDSA: how are x(R) ≥ n and the R lift handled? | A hint supplies R; acceptance requires R canonical and on the curve, and x(R) mod n == r, matching libsecp's `xr == r || xr == r + n`. The Fiat–Shamir weights are drawn from a hash of every (u1, u2, Q, R) in the batch (`LIBZYNC.md` §7, rounds 3 and 6) |
| Reorg near H? | F is tied to `J.tip_hash`; a reorg below H needs a new proof (§4, "Which chain") |
| Can it follow PR #34004's indexing? | Yes, by changing which outputs are indexed (`agg_lite.rs`); files and proofs must use the same rule (§8 Q1) |
| How does utreexod or Floresta use the forest? | Not directly: different leaves and an append-only forest (§6, `UTREEXO_FORMAT.md` §7). A snapshot for it must carry creation MTP and creation order |
| A tip proof before its epoch closes? | Checks blocks against supplied prevouts only; it does not rule out inflation until the epoch closes (§6a) |
| Receipt type and size? | A succinct STARK, about 227 KB, constant per block. A Groth16 wrap (a few KB) exists for the canonical guest and is not yet built for libzync |

## 9. Costs (ESTIMATED, ±50%)

These are extrapolations, not runs: cycles measured on 19 sampled mainnet blocks (round-16/17 images), converted to
card-seconds and dollars at measured rates ($2.49/hr per RTX PRO 6000, `experimental/libzync-bench/gpu/GPU_RESULTS.md`).
Method: `experimental/libzync-bench/cost/WHOLE_CHAIN_COST.md`.

| target | scripts | estimate |
|---|---|---|
| mainnet 1..970,000 (proves the published file) | on | ~$114–119k on A40s |
| mainnet 1..970,000 | off (assumevalid) | **~$37–42k** on A40s |
| signet 1..320,000 | off | ~$0.7k (batched; not built) to ~$2.5k (per block) |
| one mainnet tip block, 4× RTX PRO 6000 | on | measured: 266–332 s, ~$2.50 of fleet time (round-13 image `5e460bf3`) |

Mid-history blocks (2015–2021) are the expensive part: 2–6× the cycles per transaction of today's blocks.
Multi-block batching (one proof session per block range) is not built. It is worth about 4× on small-block chains.

## 10. What exists today (signet)

**The signet proof below was made on the round-4 image** (signet `c3501ac9…`, after three review rounds and the
`max_time` / `profile` fields). The current image is round 8's (signet `067822b9…`, mainnet `9f469930…`; the same source built outside the container gives other ids, `4b91b085…` and `6fcb65af…` on the development laptop, because the id depends on build paths): C++
hygiene fixes and a compiler flag, no rule change, the same committed states on every regression test. A receipt of the current image
needs a re-prove. ⚠ The mainnet measurements in §10a come from the image after review round 1 (`09a02eed`), before
rounds 2–8; their timing and size are representative, but they prove nothing under the current image.

All measured 2026-10-30.

| step | where | result |
|---|---|---|
| BIP325 check in isolation (Core's `CheckSignetBlockSolution`) | laptop | real block → valid; one txid changed → invalid |
| our numbering vs the **published** signet hints file (stop 320,000) | laptop | every one of the 606 coins it marks unspent among blocks 1..3,000 sits at the same index in ours |
| design B, blocks 1..3,000, from genesis | laptop, execute, pre-audit image | **BALANCED**; forest = the 695 coins unspent at 3,000; 687 s |
| design B, blocks 1..2,600, from genesis | laptop, execute | **BALANCED**; forest = 293 coins; state saved at 2,500 |
| forest vs the independent reference (`utreexo_ref.py`) at 2,600 | laptop | **REF_MATCH**: identical roots, 293 coins |
| **proof of blocks 2,501..2,600, round-4 image** | **1× RTX PRO 6000, RunPod** | **proved: a base step and 99 recursive steps, 1,452 s of proving, ~$1.52** including the 391 s build. Verified on the laptop (DEMO verdict, below); the verify on the pod **failed**, see below |

**The GPU proof:**
- **Anchor:** block 2,501 is a base step on the committed state at 2,500 from the laptop sweep. Every later block's
  aggregate verifies the previous block's receipt inside the proof.
- **What the anchor contains.** The bundle carried only blocks 2,501..2,600, so `libzync-prove-range` could not
  replay the BIP30 coinbase tree to 2,500. It seeded the tree with the pre-2,501 coinbases this range spends, set the
  anchor's `smt_root` to that root, and proved from that anchor, written as `anchor_effective.json`. It differs from
  the sweep's state at 2,500 **only in `smt_root`**. So this receipt trusts a BIP30 tree the prover chose, as well as
  the rest of the 2,500 state. The bundle script now ships blocks 1..2,500 too (round 6), so the next run replays the tree and checks it against
  the sweep's anchor.
- **Final receipt:** `experimental/libzync-bench/signet/gpu_r4/block_2600.rcpt` (226 KB), verified on a laptop
  against `anchor_effective.json` (`verify_local.log`: DEMO, exit 2). The verify **on the pod** gave the sweep's
  `anchor.json` and **failed on `anchor_id`** for the reason above (`verify_pod.log`). That pod check was the
  right one and still is: `pod/run.sh` verifies against the sweep's anchor, never the prover-written file. Negative checks in `verify_negatives.log`: a clock set before the last block fails
  `time-too-new`; the mainnet verifier fails it on image and profile; the receipt at 2,530 fails on height and hash.
- **Matches the node:** its tip is signet block 2,600 as the node reports it (`00000127dfa0…e6f3`).
- **Matches the laptop:** its committed state equals the laptop sweep's at 2,600 apart from two fields:
  - `smt_root`, because the proof's BIP30 tree was seeded from this range's coinbase spends (above), not replayed
    from genesis;
  - `anchor_id`: the sweep executes every block as a base step on the previous block's state, so its `anchor_id`
    is the digest of that state; the proof's is the digest of the 2,500 anchor it started from.
- **Per block:**
  - the base step, block 2,501 (coinbase-only, no previous receipt to verify): **14.7 s**;
  - coinbase-only recursive blocks: **8.7–9.2 s** (91 blocks);
  - blocks with transactions: **16.5–16.7 s** (7 blocks, one chunk each);
  - block 2,523: **514 s**. It spends 374 coinbase outputs, and 97% of its aggregate is Hazync's BIP30 coinbase SMT
    (~1.5 M cycles per coinbase spend). That is a known, separable cost (`LIBZYNC.md` §8).

**What this demonstrates, and what it does not:**
- **It does:** the whole pipeline on a real network, from the published hints format to a verified recursive proof
  that a hints file is exactly right at its stop height, with the UTXO set as Utreexo roots.
- **It does not:** prove history from genesis on the GPU. Blocks 1..2,500 were checked in execute mode on the laptop,
  not proven, so this receipt trusts the 2,500 anchor. Proving from genesis is the same command with the genesis
  anchor:
  - ESTIMATED from the measured per-block classes: 2,587 coinbase-only blocks at ~9 s, 13 transaction blocks at
    ~16.6 s, and 2,374 coinbase spends at ~1.3 s each in the BIP30 SMT ⇒ about 7.4 card-hours, ~$18.5 at $2.49/hr,
    for signet 1..2,600;
  - for all of signet against the published file, see §9.

## 10a. The last blocks before H on mainnet (969,996–970,000)

These are SwiftSync's final blocks before the published file's stop height, where the UTXO set is at its largest.
They were run with the file's **real** hint bits and proved on **1× RTX PRO 6000** as one recursive chain, with the
signature and ECMH hints generated per block. Measured 2026-10-30.

| block | inputs | ECMH adds / subtracts | aggregate cycles, hints vs none | proving time | receipt |
|---|---|---|---|---|---|
| 969,996 | 8,058 | 1,886 / 6,643 | 90.1 M vs 95.0 M | 810 s (base step) | 226,258 B |
| 969,997 | 8,854 | 800 / 5,167 | 72.3 M vs 74.6 M | 749 s | 227,282 B |
| 969,998 | 8,303 | 830 / 5,025 | 71.1 M vs 73.7 M | 773 s | 227,034 B |
| 969,999 | 8,034 | 65 / 6,083 | 83.6 M vs 88.8 M | 835 s | 227,290 B |
| **970,000 (= H)** | 7,283 | **0** / 2,133 | 52.7 M vs 53.3 M | **659 s** | **227,034 B** |

- **Proof size is constant, about 227 KB per block,** whatever the size of the block or the UTXO set. Each receipt also
  proves every earlier block in the chain (recursion).
- **Proving time on one card is about 11–14 minutes per full block.** It is dominated by the 16 chunks proved one
  after another, ~580–720 s. Spread over 4 cards, as in the live tip runs, the same work takes 4.5–5.5 minutes.
- **Next to H, most new outputs survive,** so they become cheap forest appends: at 970,000 there are zero ECMH adds.
  Ruben's case (most outputs spent, so more ECMH) is the blocks far before H. Either way the hint bits change only
  the aggregate, by 1–6% (≤ 6 M of ~500 M cycles per block), so ECMH is not a bottleneck in either regime.
- **The proof is checked against the real chain.** Its tip is mainnet block 970,000 as the node reports it
  (`000000000000000000014e6a9bad…5a07`).
- **The guest's indexing matched the published file on every block.** Each block's aggregate rejects a hint count
  different from its own indexed-output count; all five blocks, given the published file's bits, passed.
- **This is a measurement, not a hints proof.** The chain starts from a state taken from block 969,995's file, not
  from genesis, so `libzync-verify` correctly reports `ss_set ≠ 0` and an `ss_hints` mismatch; it passes height and
  image. A proof that the published file is correct needs the sweep from genesis (§9).
- **Cost:** $3.26 including the 447 s build. An earlier attempt the same day was stopped early (~$1.34) because the
  range command had not yet generated signature hints.

## 11. Code

- **Guest:** `prover/methods/guest/src/` (`agg_lite.rs`, `swiftsync.rs`, `batch_ecdsa.rs`).
- **Shared formats:** `libzync/` (`wire.rs`, `ss_hints.rs`, `net.rs`, `host.rs`).
- **Core patches:** `patches/libzync/`; reproduce the tree with `scripts/libzync-base.sh`.
- **Engineering doc:** `docs/libzync/LIBZYNC.md`.
