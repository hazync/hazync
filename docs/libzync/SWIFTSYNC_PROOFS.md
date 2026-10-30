# Proven SwiftSync hints: a proposal (draft v0.1, 2026-10-30)

**For:** Ruben Somsen, Robert Netzke, and anyone building on SwiftSync.
**From:** the Hazync project (`libzync` branch).
**Status:** working code and real measurements, as an execute-mode sweep and GPU proofs (signet, §10). The design has
had one independent review; it found bugs, which are now fixed (`docs/libzync/LIBZYNC.md` §7). It still needs more
outside eyes.

## 1. The problem this solves

- **SwiftSync makes initial block download cheap and parallel,** but it needs a **hints file**: one bit per output,
  "still unspent at height H?".
- **A wrong file cannot make a node accept a bad chain.** The aggregate check at H fails, and the node has wasted a
  sync. So the cost of a bad file is denial of service, and the question a user faces is **"who made this file, and
  should I spend a sync on it?"**
- **A Hazync proof answers that question without trust.** It shows that a specific file is exactly right for a specific
  chain, and it hands over the UTXO set at H as a Utreexo accumulator. That is enough to skip IBD altogether.

## 2. What is proven

For a receipt R produced by the Hazync libzync guest with image id M, journal J and a hints file F, the following
statement holds if the checks in §5 pass:

> There is a chain of blocks 1..H starting from the network's genesis, ending at `J.tip_hash` with cumulative work
> `J.cum_work`. Every block is valid under Bitcoin Core v28 consensus rules: PoW, retarget, MTP, merkle, witness
> commitment, weight, sigops, subsidy, BIP30/34/65/66/68/112/113/141/143/147, and scripts (unless M is an assumevalid
> image), plus BIP325 on signet. And:
> 1. **F is exactly right:** for every height h ≤ H, F lists precisely the indices of h's indexed outputs that are
>    unspent at H.
> 2. **The UTXO set at H** is exactly the leaves of the Utreexo forest `(J.utxo_roots, J.utxo_leaves)`, in creation
>    order.

## 3. Objects

### The hints file: hintsfile v0, unchanged

- **Format:** magic `UTXO`, version `0`, stop height `H` (u32 LE). Then, for each height 1..=H, an Elias–Fano coded list
  of the **indices** of that block's outputs that are unspent at H.
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
| utxohints.store signet `signet.hints` (25.0 MB, sha256 `a3c86962…`) | 320,000 | `d9c88278135e495749dd04e8a635a444e99d6dc470a43d11f37ecd91f02b4947` |
| mainnet blocks 1..200, from `swiftsync-make-hints` | 200 | `e33c66b38a04cbf2e4cdf1813a6530a66121234d13a68e0a6f3ecd17907c4ccd` |
| signet blocks 1..3,000, from `swiftsync-make-hints` | 3,000 | `5979d283d04530bc35fd450b5b99fb89ad2cbb9f9c640b1038c8c72f837ed779` |

### The proof's public output: `ChainState`

| field | meaning |
|---|---|
| `kind` | `0xC4A10002` (domain tag) |
| `tip_hash`, `height`, `cum_work` | the chain proven |
| `prev_nbits`, `prev_time`, `epoch_start`, `recent_times` | difficulty and time context, so the chain can be extended |
| `utxo_roots`, `utxo_leaves` | Utreexo forest of the coins marked unspent-at-H, in creation order |
| `ss_set`, `ss_acc` | the ECMH running total (`ss_set = 0` means the identity) |
| `ss_hints` | the running hash of §3 |
| `anchor_id` | double-SHA-256 of the state the chain started from (genesis for a full proof) |
| `self_id` | the image id every recursive step ran |

## 4. How the guest checks a block (design "B")

For every block h ≤ H, each **indexed output** k gets the bit b_k from F:

| event | action |
|---|---|
| output created, b = 0 (spent by H) | `ECMH += P(leaf)` |
| output created, b = 1 (unspent at H) | append `leaf` to the Utreexo forest |
| an input spends an output created earlier in the same block | cancelled in the block; it must have b = 0 (else reject) |
| an input spends an older coin | `ECMH -= P(leaf)` |

- **The leaf** is `H(outpoint ‖ amount ‖ script ‖ height ‖ coinbase flag ‖ creation MTP)`, so a spend has to name the coin
  exactly as it was created.
- **P** is a hash to secp256k1:
  - x = SHA-256(leaf ‖ ctr), taking the **first** ctr that lands on the curve; every skipped ctr is proven off-curve by a
    hinted non-residue witness;
  - y is the **even** root.
  - So each leaf has exactly one point.
- **At H,** `ECMH == identity` holds exactly when every b = 0 coin was created once and spent once, and nothing else
  was spent. That makes F right on every b = 0 coin; a b = 1 coin that was really spent would have subtracted a point
  that was never added. The forest therefore holds exactly the UTXO set.

**Why ECMH and not the salted sum.**
- SwiftSync's salted SHA-256 sum is secure because the salt is **secret** to the syncing node.
- A proof must be checkable by anyone, so it cannot have a secret.
- ECMH's security is the discrete-log hardness of secp256k1 (about 128 bits), with no secret.
- Cost in the zkVM: ~3.0k cycles per element against ~1.3k for the salted sum.

## 5. Verifier procedure: all of it

Given a receipt R, the hints file F and the network:

1. `R.verify(M)`, where M is the published libzync image id for that network (and for scripts or assumevalid).
2. Decode the journal as `ChainState` J and check `J.kind == 0xC4A10002` and `J.self_id == M`.
3. `J.anchor_id == dsha256(genesis ChainState journal for the network)`.
   **This is essential:** the base step of the recursion is not verified, so a proof anchored anywhere else proves
   nothing about history.
4. `J.height == F.stop_height`.
5. `J.ss_set == 0`.
6. `J.ss_hints == ss_hints(F)` (§3).
7. `J.tip_hash` is a block on the chain you follow (most work). The proof says that chain is valid, not that it is
   the best one.

`host libzync-verify <receipt> <file.hints>` does steps 1–6 and prints `tip_hash` and `cum_work` for step 7.

**Example output.** This is the real run of §10, re-verified on a laptop from the 226 KB receipt:

```
$ host libzync-verify block_2600.rcpt signet2600.hints anchor_state_2500.json
  [ok] receipt verifies against METHOD_ID cf7f75ff85f51f5845d4e37f8b7fe6e55f70632dd503eb2f25d7d0fb4979f405 (libzync guest, signet)
  [ok] journal is a ChainState committed by this image (kind, self_id)
  [ok] anchor_id is the GIVEN anchor at height 2500 (anchor_state_2500.json) -- NOT genesis
  [ok] height 2600 == the file's stop height 2600
  [ok] SwiftSync total is the identity (ss_set == 0)
  [ok] ss_hints f097b8f2a52a23d5e75db23b24e9a473a31b804534ac43592d816806c7722f7b == hash of signet2600.hints
  chain: tip ... (display order 00000127dfa07c0fd2f2fe1e22a0fea15c34cabd6204b1faac15a04ba28be6f3), cum_work 0x432982f4d
  UTXO set at H: 293 coins, Utreexo roots [...]
LIBZYNC_VERIFY_OK: signet2600.hints is proven correct for this chain at height 2600
```

The same tool **fails** a proof that stops early: a dev-mode receipt at 2,530 against the same file gives
`[FAIL] height 2530 == the file's stop height 2600` and `[FAIL] ss_hints …`. Without the anchor argument, check 3
fails for this demonstration proof, because it starts at 2,500 and not at genesis (§10).

## 6. What a SwiftSync node can do with it

1. **Verified hints.** Fetch F and R from anyone, run §5, then SwiftSync from genesis with full confidence that the
   sync will not fail at H.
2. **No IBD at all (assumeutxo, proven).** Fetch the UTXO set at H from anyone, check its Utreexo roots against
   `J.utxo_roots`, check §5 against the tip you see, then validate from H onwards. A Utreexo node needs only the
   roots.
3. **Assumevalid flavour.** An assumevalid image skips script checks (about 3× cheaper to produce, §9). Its proof
   says exactly what SwiftSync's assumevalid mode assumes, so it is enough for verified hints, but it is not a full
   validation proof.

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
| **at H_k** | `ECMH == identity` and `ss_hints == hash(F_k)`. The total restarts at the identity for epoch k+1 |
| **cost** | spends of pre-epoch coins carry inclusion proofs (the canonical Utreexo cost); everything else is the cheap SwiftSync path |
| **what you get** | a failure is confined to one epoch; H moves forward by proving one more epoch, not the whole chain; the tip phase is simply a very short epoch |

**Open questions for you:**
- Do epoch files fit SwiftSync's plans for refreshing hints?
- Should F_k cover only outputs created in the epoch, or every output up to H_k (one file per height range, or
  cumulative)?

## 8. Questions we would like your view on

1. **Index convention:** the hintsfile crate (spendable only, BIP30 coinbases skipped) or PR #34004 (every output).
   Proofs can follow either, but files must agree with their proofs.
2. **The commitment format** of §3. Should it be a hash over the raw file bytes instead? It cannot be today, because the
   Elias–Fano encoding is not canonical across encoders unless the spec fixes it.
3. **ECMH vs the salted sum,** for a published aggregate.
4. **What the proof should output:** Utreexo roots, a MuHash of the UTXO set, both, or something else.
5. **Epoch spacing (§7),** and whether hints files will be published per epoch.

## 9. Costs (measured, ±50%)

| target | scripts | estimate |
|---|---|---|
| mainnet 1..970,000 (proves the published file) | on | ~$114–119k on A40s |
| mainnet 1..970,000 | off (assumevalid) | **~$37–42k** on A40s |
| signet 1..320,000 | off | ~$0.7k (batched; not built) to ~$2.5k (per block) |
| one mainnet tip block, 4× RTX PRO 6000 | on | 266–332 s, ~$2.50 of fleet time |

Mid-history blocks (2015–2021) are the expensive part: 2–6× the cycles per transaction of today's blocks.
Multi-block batching (one proof session per block range) is not built. It is worth about 4× on small-block chains.

## 10. What exists today (signet)

All measured 2026-10-30. Signet guest METHOD_ID `cf7f75ff…`.

| step | where | result |
|---|---|---|
| BIP325 check in isolation (Core's `CheckSignetBlockSolution`) | laptop | real block → valid; one txid changed → invalid |
| our numbering vs the **published** signet hints file (stop 320,000) | laptop | every one of the 606 coins it marks unspent among blocks 1..3,000 sits at the same index in ours |
| design B, blocks 1..3,000, from genesis | laptop, execute | **BALANCED**; forest = the 695 coins unspent at 3,000; 687 s |
| design B, blocks 1..2,600, from genesis | laptop, execute | **BALANCED**; forest = 293 coins; state saved at 2,500 |
| **proof of blocks 2,501..2,600** | **1× RTX PRO 6000, RunPod** | **verified, 100 recursive steps, 1,464 s, $1.39** including the 380 s build |

**The GPU proof:**
- **Anchor:** block 2,501 is a base step on the committed state at 2,500 from the laptop sweep. Every later block's
  aggregate verifies the previous block's receipt inside the proof.
- **Final receipt:** `experimental/libzync-bench/signet/gpu/block_2600.rcpt` (226 KB), re-verified on a laptop.
- **Matches the node:** its tip is signet block 2,600 as the node reports it (`00000127dfa0…e6f3`).
- **Matches the laptop:** its whole committed state equals the laptop sweep's at 2,600, apart from `anchor_id`
  (the sweep started at genesis).
- **Per block:**
  - coinbase-only blocks: **8.7 s** (91 blocks, 8.5–8.9 s);
  - blocks with transactions: **15–16 s**;
  - block 2,523: **549 s**. It spends 374 coinbase outputs, and 97% of its aggregate is Hazync's BIP30 coinbase SMT
    (~1.5 M cycles per coinbase spend). That is a known, separable cost (`LIBZYNC.md` §8).

**What this demonstrates, and what it does not:**
- **It does:** the whole pipeline on a real network, from the published hints format to a verified recursive proof
  that a hints file is exactly right at its stop height, with the UTXO set as Utreexo roots.
- **It does not:** prove history from genesis on the GPU. Blocks 1..2,500 were checked in execute mode on the laptop,
  not proven, so this receipt trusts the 2,500 anchor. Proving from genesis is the same command with the genesis
  anchor:
  - about 2,500 × 8.7 s ≈ 6 card-hours ≈ $15 for signet 1..2,600;
  - for all of signet against the published file, see §9.

## 11. Code

- **Guest:** `prover/methods/guest/src/` (`agg_lite.rs`, `swiftsync.rs`, `batch_ecdsa.rs`).
- **Shared formats:** `libzync/` (`wire.rs`, `ss_hints.rs`, `net.rs`, `host.rs`).
- **Core patches:** `patches/libzync/`; reproduce the tree with `scripts/libzync-base.sh`.
- **Engineering doc:** `docs/libzync/LIBZYNC.md`.
