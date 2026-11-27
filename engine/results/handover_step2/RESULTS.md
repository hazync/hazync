# Handover across OpenVM versions, step 2: the guest path, proved (27.11.2026)

engine/HANDOVER.md step 2. A test image (the test variant, lineage/test.rs replaced in the assembled copy:
xv_lineage.rs) pins keys-ch's chain as an ovm-xver predecessor at height 200 (verifier = SHA-256 of keys-ch's key
file b5b94f9c...), carries that key (HZ_OVM_XVER_KEYS), and proves block 200 from keys-ch's proof of block 199, the
predecessor's proof checked IN SOFTWARE in the guest (xver_ovm.rs -> verify-core). Same OpenVM (2.1.0) on both sides,
forced through the cross-version path: the mechanism, not yet two versions (step 3). Build box, CPU (hv.sh, hv.out).

- The default engine image is UNCHANGED by the ovm-xver switch: rebuilt at HEAD with and without it, exe b7d7c148
  both (edit 1 on one line: panic locations carry line and column).
- keys-xv (test-lineage, ovm-xver): exe 434f9f6e; a host of the same variant (built from a copy of the checkout).
- **Block 200 proved as the handover block: 6,130 s** (aggregate 6,023 s, 43 segments; the trie 68 s), "handover:
  verifies predecessor"; `hzengine verify` OK.
- **Its state = the predecessor chain's own block 200 in every field but self_id and lineage** (the new image's
  id; the lineage digest of the pinned handover, 73b0a5ff...); anchor_id carried unchanged.

Negative controls (hv_neg.sh, hv_neg.out): each refused IN THE GUEST by the cross-version check (main.rs's
"predecessor chain proof invalid (cross-version)", the handover block's collect pass), with its own reason:

| predecessor input | the guest's reason |
|---|---|
| the proof with one byte flipped | "the predecessor proof does not verify" |
| the journal's cum_work changed (the host does not check it; the proof's public values bind it) | "the predecessor proof is not of this journal" |
| another real proof (block 200's) given for block 199's journal | "the predecessor proof is not of this journal" |

(The pin, the image id and a non-pinned key are refused by verify-core itself: its own test and engine/VERIFIER.md.)
