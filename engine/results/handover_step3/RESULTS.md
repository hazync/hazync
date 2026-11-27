# Handover across OpenVM versions, step 3: v2.0.2 to v2.1.0, proved (27.11.2026)

engine/HANDOVER.md step 3: the rehearsal with two REAL OpenVM versions. Build box, CPU (xv3.sh, xv3.out).

- **Predecessor: the whole engine on OpenVM v2.0.2** (engine/handover/v2.0.2): keys-v202, guest exe 78d2023d,
  image id 37adf7e2..., key file pin e2519a33...; block 199 proved on v2.0.2 (287 s, a base step on prep's state at
  198) and verified. Its proof's plain encoding: 273,091 bytes.
- **Successor: a v2.1.0 test image** (keys-xv3, exe 0065e63a, features test-lineage, ovm-xver, ovm-xver-v2_0_2)
  carrying v2.0.2's own verifier (hazync-engine-verify-v2-0-2) and the v2.0.2 key, and a lineage (xv3_lineage.rs)
  pinning the v2.0.2 image at height 200 with verifier = that key's pin. Built in 98 s; its host (same test lineage)
  in 2,355 s.
- **Block 200 proved as the handover block: 5,754 s** (43 aggregate segments; the trie 75 s), "handover: verifies
  predecessor"; `hzengine verify` OK; journal sha256 3ef7346a....
- **Its chain state is the same as the native one:** state_200.coinlog and state_200.trie are byte-identical to
  prep's block 200 and to step 2's handover. In state_200.json only the identity fields differ: self_id (this image),
  lineage (the pinned handover's digest) and anchor_id. anchor_id is carried unchanged from the v2.0.2 predecessor's
  state at 199, as in step 2.

Negative controls (xv3_neg.sh, xv3_neg.out). Each was refused IN THE GUEST by the cross-version check, in the
handover block's collect pass ("predecessor chain proof invalid (cross-version)"):

| predecessor input | the guest's reason |
|---|---|
| the v2.0.2 proof with one byte flipped | "the predecessor proof does not verify" |
| the journal's cum_work changed | "the predecessor proof is not of this journal" |
| a v2.1.0 proof of the same block 199 (keys-ch's) where the v2.0.2 one belongs | "the predecessor proof does not verify" |

The third case shows that a proof from a different version or key does not pass the v2.0.2 verifier, even when it
proves the same block.
