# Hazync: the exact security claim

> This page describes **Hazync v2** (the libzync engine), the images published in release v0.24.0. The statement
> for Hazync v1 (the CORE guest, image `37987b85…`, last released as v0.22.1) is kept in
> [`history/SOUNDNESS.md`](history/SOUNDNESS.md); v1 proofs still verify with v0.22.1's verifier.

The page a reviewer should read first. It says what a v2 receipt proves, what that rests on, what it does not
prove, and what is known to be open. It summarises; the normative text is
[`libzync/SWIFTSYNC_PROOFS.md`](libzync/SWIFTSYNC_PROOFS.md) (the file name is historical: it is the statement, the
`ChainState` layout and the verifier procedure) and the code it cites, `swiftsync-verify/src/lib.rs` and
`host libzync-verify`.

## 1. The claim

A receipt of a published Hazync v2 image, accepted by the verifier with verdict **OK**, says:

> every block from genesis to `tip_hash` is valid under Bitcoin Core v28.0's consensus rules as compiled into the
> image, and `ChainState` is the state after the last one.

The images are pinned in [`../reproduce/LIBZYNC_IDS`](../reproduce/LIBZYNC_IDS):

| image | id |
|---|---|
| mainnet product (v0.26.0; continues `8888d59c` at block 2000) | `1ff5a129d1ebc2022262f1249c1b41a0c096b8d5d16f985d7fc018138c67c66a` |
| mainnet product (v0.24.0, v0.25.0; blocks 1..1999 of the board) | `8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4` |
| signet product (v0.26.0) | `28e396006e97a8075daeb9fb12ce3ef1b3a20e44eac935256a1c2e8c3d8d57f3` |
| signet product (v0.24.0, v0.25.0) | `d4210dae455340743cca097f2b422eee9846c9c02af0dae31d1268fadef4f387` |
| header-chain zyncAPP (mainnet) | `2db34e12cc993dd1edb003f00f85c93d02e0e61208589b57fc5c969d39b3e632` |

How faithful "Core's rules as compiled into the image" is, rule by rule, is
[`libzync/CONSENSUS_RULES.md`](libzync/CONSENSUS_RULES.md): **59 rows SAME, 3 STRICTER, 0 WEAKER, 0 NOT ENFORCED,
1 N-A** (63 rows, counted for an image that checks scripts). A STRICTER row costs liveness (no proof exists for a
chain Core would accept); it cannot make a proof claim something false. A WEAKER or NOT ENFORCED row would be a
soundness problem; there are none listed. Note that the table was read at a commit newer than the source of the
published ids, and says so in its header.

## 2. What the proof's output (`ChainState` version 2) commits

The journal is `ChainState` version 2 (full layout: [`libzync/SWIFTSYNC_PROOFS.md`](libzync/SWIFTSYNC_PROOFS.md) §3):
the tip hash and height; the **coin log** roots, leaf count and live value ([`libzync/COINLOG.md`](libzync/COINLOG.md));
cumulative work and the time and difficulty context; the **counting trie** root
([`libzync/ZYNCAPPS.md`](libzync/ZYNCAPPS.md) §2); the **live-coin multiset hash**; the BIP30 tree root; the **block
forest** (one leaf per block, [`libzync/HANDOVER.md`](libzync/HANDOVER.md) §3); `max_time`; `profile`; `self_id`;
`lineage`; and `anchor_id`.

`profile` is set from the image's compiled code, never carried from input. The product images are `0x71` (mainnet)
and `0x75` (signet): libzync, coin log, counting trie, live-coin hash, **scripts and signatures checked** (bit 1, the
assumevalid bit, is clear).

## 3. When the claim holds: the verifier's checks

A receipt means nothing until a verifier has made every check in
[`libzync/SWIFTSYNC_PROOFS.md`](libzync/SWIFTSYNC_PROOFS.md) §5. The ones that carry the claim:

- **The image.** The receipt verifies against the published image id M, and `self_id == M`. The receipt must be a
  real proof: a dev-mode `FakeReceipt` is refused, and the verifier is built with dev mode compiled out.
- **The genesis anchor.** `anchor_id` must equal the network's pinned genesis anchor (mainnet
  `852c5c7e009317c2dd232dcf9b1a9e2babd58df324add461a6ff61824f390167`). The base step of the recursion is not
  verified, so a proof anchored anywhere else proves nothing about history. The value can be recomputed from the
  text of §5 step 3 alone (`scripts/libzync/genesis_anchor_id.py`).
- **Enough work, and the checkpoint.** The image pins one block (`prover/methods/guest/src/checkpoint.rs`: mainnet
  970,000, signet 325,000) and refuses any other block at that height. A proof at or above that height, with at
  least Core v28's `nMinimumChainWork`, contains the real chain's checkpoint.
- **The block forest, the lineage, the profile** match what the image should commit. For the published product
  images the lineage list is empty (`prover/methods/guest/src/lineage/mainnet_coinlog.rs`): one image since genesis,
  so no predecessor image is trusted through a handover.

**Verdicts:** **OK** (exit 0): every check passed on a genesis-anchored proof with enough work, at or past the
checkpoint. **DEMO** (exit 2): every check passed, but the proof is below the minimum work or the checkpoint height,
or was checked against a supplied `--anchor`. **FAIL** (exit 1): anything else, including an unreadable receipt.
Every proof the public board makes until its chain passes block 970,000 is, correctly, DEMO.

## 4. What the claim rests on

1. **Bitcoin Core v28.0's own code and libsecp256k1 v0.5.1**, compiled into the guest: the script interpreter,
   sighash, transaction checks and serialisation, merkle, proof of work and retarget. Three patches sit on
   top (`patches/libzync/0101-core-libzync-levers.patch`, `0102-secp-libzync-batch-hooks.patch`,
   `0103-secp-sha256-via-accelerator.patch`, applied by `scripts/libzync-base.sh`), each `#ifdef`'d and reviewed to
   compute the same bytes as Core; 0102 is the hook the batch signature check below uses. Who decides what, rule by rule: [`libzync/LIBZYNC.md`](libzync/LIBZYNC.md) §6.
2. **Hazync's own code**, which is consensus-critical and is where a soundness bug is most likely. The reviews so
   far found real soundness bugs in exactly these parts ([`libzync/LIBZYNC.md`](libzync/LIBZYNC.md) §5):
   - the **coin log**: a spend must open the coin's unspent leaf against the current roots
     (`libzync/src/coinlog.rs`, `libzync/src/coinlog_forest.rs`);
   - the **counting trie** and its separate update receipt (`libzync/src/ttree.rs`, `libzync/src/trie_fact.rs`;
     guest mode 17, `trie_update()` in `prover/methods/guest/src/main.rs`);
   - the **batch signature check**: libsecp parses and range-checks every key and signature, and the group
     equation of every hinted signature in a chunk is checked in one weighted multi-scalar multiplication
     (`prover/methods/guest/src/batch_ecdsa.rs`); a bad hint can only fall back to libsecp or fail;
   - the **block rules applied from the chunks' facts**: fees, weight, sigops, BIP30/34/68, coverage of every input
     (`prover/methods/guest/src/agg_lite.rs`, `aggregate()` in `main.rs`);
   - **handover and joins**: `prover/methods/guest/src/handover_pin.rs`, and guest mode 18, `join()` in `main.rs`,
     which requires the right proof to start exactly where the left one ends.
3. **SHA-256** collision resistance: the coin log, the trie, the block forest, the BIP30 tree and every link digest
   are SHA-256 commitments.
4. **The soundness of RISC Zero 3.1.0** (risc0-zkvm `=3.1.0`, pinned in `prover/methods/guest/Cargo.toml` and
   `swiftsync-verify/Cargo.toml`), including its recursion. 3.1.0 fixes soundness bugs in RISC Zero's proof system
   that were present in 3.0.x ([`RELEASE_NOTES_v0.24.0.md`](RELEASE_NOTES_v0.24.0.md) lists them). That is why
   v0.23.0 (image `863c9ce3…`, risc0 3.0.5) is withdrawn: **a 3.1.0 verifier refuses every 3.0.x receipt**, and a
   test vector checks this with v0.23.0's real proof of blocks 1..200
   (`swiftsync-verify/vectors/mainnet_1_200_863c9ce3_risc0_305.rcpt`).
5. **The image matches the published source.** The claim is about the program the id names. Rebuild it with
   `reproduce/libzync.Dockerfile` and compare with `reproduce/LIBZYNC_IDS`; the id depends on absolute build paths,
   so only a build in that container gives the published id.
6. **The genesis block.** The anchor is the genesis state, computed from the genesis block's own fields. There is
   no other trusted starting point.

What the claim does **not** rest on: the prover, the coordinator, or the host's hints and witnesses. Signature and
multiset-hash hints, coin-log and trie witnesses and the prepared state before each block are all untrusted input;
a wrong one makes proving fail (a liveness cost), it cannot make a false block prove valid.

## 5. What is not proven

- **Which chain it is.** The proof says the chain it covers is valid from genesis; a valid fork would have its own
  tip hash. The verifier compares `tip_hash` with a source you choose (`--bitcoin-cli`, `--mempool`, `--expect`); with
  none, its OK line says the chain identity was NOT checked and prints the hash to compare.
- **`time-too-new`.** A block's time may not be more than two hours past the validating node's clock. A proof has no
  clock, so the verifier checks `max_time <= now + 7200` against **its own** clock.
- **Anything after `tip_hash`.** A coin unspent at the proof's height may have been spent since. Every answer names
  the height it is about.
- **That nobody has audited it.** No commissioned external audit has happened ([`../SECURITY.md`](../SECURITY.md)).

## 6. Known open issues

From [`libzync/CONSENSUS_RULES.md`](libzync/CONSENSUS_RULES.md), [`libzync/LIBZYNC.md`](libzync/LIBZYNC.md) §7 and
[`libzync/ROADMAP.md`](libzync/ROADMAP.md) §I:

- **The STRICTER rows** (liveness, not soundness): BIP30 for coinbases on a chain other than mainnet (an untracked
  coinbase repeated before BIP34 is refused even when fully spent); spending the coinbase coins of blocks 91,722 and
  91,812, which are never added to the coin log (mainnet never spent them); and `bad-cb-amount`, where the guest also
  requires subsidy + fees ≤ `MAX_MONEY`. Separately, by design, no proof exists for a chain that forks below the
  image's checkpoint.
- **The batch signature check's margin is untested at the edge:** the x = r + n case is infeasible for real r and
  effectively untested (ROADMAP I1).
- **Fuzzing and a wider rule rig are open** (ROADMAP I1). What has run: the rig of crafted blocks, guest against
  Bitcoin Core 28.0 on regtest, 96/96 agree on the frozen core; Core's script and transaction vectors with every
  flag variant, 22,666 / 22,666 agree (on an earlier build, c10e12); forged signature hints, 0 Core-invalid cases
  pass (c10e11). Each row names its image in [`libzync/LIBZYNC.md`](libzync/LIBZYNC.md) §5.
- **One zkVM.** Every proof rests on one pinned RISC Zero version (ROADMAP I3; a second-zkVM check of checkpoints is
  an idea, not built). The published images are on 3.1.0 (ROADMAP I3).
- **No external audit or bug bounty** (ROADMAP I2): needed before anyone relies on a proof for money.
- **No proof of the whole mainnet chain yet.** The board restarted from genesis under `8888d59c` on 2026-11-12.

## 7. Reporting

A way to make an invalid block prove valid is the finding that matters most. Report it privately, as
[`../SECURITY.md`](../SECURITY.md) describes; where to look first is [`EXTERNAL_REVIEW.md`](EXTERNAL_REVIEW.md).
