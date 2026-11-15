# Upgrading the board's image without re-proving: the handover runbook

The format (what a handover is, the lineage digest, what a verifier checks) is in
[libzync/HANDOVER.md](libzync/HANDOVER.md). This page is how to move the live board from image **O** to image **N** at
block **H**, so every proof O made stays in the chain.

## 1. The rule: only non-soundness upgrades

A handover takes O's proofs of blocks 1..H-1 on trust. The chain is then exactly as sound as O was.

| Why N exists | Handover? |
|---|---|
| A rule that starts at a height (a soft fork; ship N **before** activation, H ≤ activation) | yes |
| A cheaper or faster prover proving the same statement | yes |
| A RISC Zero upgrade for speed or features | yes (cross-version, §3) |
| New zyncAPP fields, appended to the state (write the predecessor decode first, libzync/HANDOVER.md §4) | yes |
| **A soundness bug in O** | **no: re-prove from genesis** |
| **A RISC Zero upgrade that fixes a soundness bug** in its circuit, verifier or recursion | **no: re-prove from genesis** |

Read the RISC Zero release notes before deciding. When in doubt, it is a soundness fix. RISC Zero 3.1.0 fixed
soundness bugs in 3.0.x, so nothing may hand over from a 3.0.x image; the board re-proved on 3.1.0.

## 2. Same RISC Zero (O and N on the same version)

N is O's source plus the change, plus one entry in the variant's lineage file
(`prover/methods/guest/src/lineage/mainnet_coinlog.rs` for the product image):

```rust
pub const LINEAGE: &[crate::handover_pin::Handover] = &[crate::handover_pin::Handover {
    image: [/* O's id as u32x8: `host method-id` prints it */], first_height: H, verifier: crate::handover_pin::SAME_ZKVM }];
```

The handover step verifies O's spine receipt with `env::verify`.

## 3. Across a RISC Zero upgrade

O's receipts were made under an older RISC Zero, whose recursion circuit N's cannot check (`env::verify` fails even
with O's control root: 3.0.5 → 3.1.0 changed the circuit's constraints). N carries O's version's recursion circuit and
checks O's seal in software (`prover/methods/guest/src/xver/`).

1. **Vendor O's version** if `xver/` does not have it yet: a new `xver/v<x_y_z>/` with `control_id.rs`, `info.rs`,
   `poly_ext.rs` and `taps.rs` copied verbatim from that version's `risc0-circuit-recursion` (`src/`), and a `mod.rs`
   like the others with that version's verifier-parameter digest. Add it to `KNOWN` and a guest feature `xver-v<x_y_z>`.
   `v3_1_0` (the published images' version) is already there.
2. **Check what else moved.** The vendored check uses N's `risc0-zkp` verifier and N's `ReceiptClaim` digest. If the
   new RISC Zero changed either (diff `risc0-zkp/src/verify/` and `risc0-zkvm/src/claim/` between the two versions),
   vendor O's copy of that too. `host xver-selftest` and the in-guest test below catch a mismatch on a real receipt.
3. **Turn the feature on** in `prover/methods/guest/Cargo.toml`: `default = ["serde", "xver-v<x_y_z>"]`. It is source,
   so N's id commits to it. N does not build if its lineage names a version it does not carry.
4. **Pin O** as in §2 with `verifier: crate::xver::v<x_y_z>::PARAMS` instead of `SAME_ZKVM`. The lineage digest
   then commits to O's RISC Zero too.
5. **Test on real receipts**, CPU only:
   - `host xver-selftest <a real O receipt>`: the digest recomputed from the vendored circuit, the receipt verified
     natively, and three negative controls (a seal word, the image, the journal) refused.
   - `experimental/libzync-bench/xver/`: the same check inside the zkVM, as the handover runs it, with the
     negative controls and a wrong-version control. Cost measured 11-13: about 22 M cycles per receipt.

## 4. The rehearsal

`experimental/libzync-bench/handover/board_rehearsal.sh` runs the whole switch on a local board in dev mode (mainnet
blocks 1..20, H = 11): O proves 1..H-1 and its spine, the coordinator moves to N, N hands over at H and continues, and
the verifiers accept the new spine and refuse a forged lineage. CI runs it and its `--control` (libzync-chain,
`scripts/libzync/ci/chain.sh` step 6) on the mainnet TEST VARIANTS `host_m0` / `host_mN` (`HAZYNC_TEST_LINEAGE=1`:
`lineage/test.rs` instead of the published lineage, ids in no pin table, the host prints `TEST_VARIANT`). Dev mode
has no real seals, so the cross-version check is covered by §3's tests, not by the rehearsal. Before a live switch, also run one real GPU confirmation (O's published image, a test N
pinning it, mainnet 1..20 handing over at 11) in the next GPU batch.

## 5. The live switch

1. **Pick H**, a few days beyond where claims will be when N ships. For a soft fork, H ≤ activation.
2. **Cap O** (any time before claims near H): set `LZ_HANDOVER_AT=H` on the coordinator and restart it. With O's host
   it runs in the "pre" era: no claim, submission or fold reaches H.
3. **Release N** (a normal release; discuss its contents first).
4. **Prep**: the board's prep needs nothing if N changes no state-transition rule (states are stamped by era: blocks
   below H as O's, H and above as N's, also on read). If N changes a rule keyed by height, stop O's prep with
   `LZ_HI=H-1` and continue with N's prep from `state_{H-1}.json` (`HAZYNC_PREP_CONTINUE=1`); check it is byte-identical
   to O below H with `HAZYNC_PREP_CHECK`. A rule change NOT keyed by height cannot be handed over: the handover refuses
   it (the bundle's state is not O's receipt's).
5. **Finish O's spine to H-1** with O's host: `HAZYNC_HOST=<O host> hazync spine`.
6. **Switch the coordinator to N's host** (`HAZYNC_HOST`, same database, `LZ_HANDOVER_AT=H`), and restart it: it is
   now the "post" era. O's stored ranges still verify (verify-any accepts O below H through N's lineage). Claims start
   at H+1; block H is never claimed.
7. **The handover**: `hazync spine` with N's host proves block H on O's spine and submits [1..H]. The coordinator
   records it as vrange [1..H], and the frontier walks on through N's blocks.
8. **Workers** move to N's release. An O worker's proof above H is refused as MISMATCH and the worker stops with
   "update the prover binary".
9. **Verifiers**: add N to `swiftsync-verify`'s `IMAGES` after O, with its lineage (lookup by id keeps O's proofs
   verifiable), rebuild the WASM, pin N on the website; set `HAZYNC_METHOD_ID` and `HAZYNC_HANDOVER=<O id>:<H>[:<verifier>]`
   on `hazync-lz-check-spine`; switch `ZAPP_HOST` (the Coin check follower) to N's host only after the web update.
10. **Keep** `state_{H-1}.json` and `state_H.json` for ever: verify-any's start and end checks need them.
