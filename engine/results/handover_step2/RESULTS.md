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

## Review round 1, R1-5 #3: the in-guest handover negatives, all run (28.11.2026)

hazync-coord, CPU, execute only (r1_setup.out, r1_runs2.sh / .out, r1_runs3.sh / .out, the exact guest messages in
r1_guest_messages.out). Block 200 as the handover block from a predecessor input; each case runs the aggregate's collect
pass (`s0/00001_b200_agg_collect`), where the handover checks run before the collect exit.

**Images** (built at kit f5207731, the test-lineage host at 58047ec3; r1_setup.out): the default image e3729469 (so the
build matches the canonical one and the ovm-xver switch leaves it unchanged), keys-xv 839a4646 (lineage: keys-ch's chain,
image c7fe0eb8, handed over at 200 with its key's pin b5b94f9c), and three purpose-built images for cases 5-7 (k5
91c0679d, k6 29e81e2b, k7 0c268a66; lineage files r1_lin1.rs, r1_lin6.rs, r1_lin7.rs from r1_mklin.py). The core
guest has moved since (review round 3: 755f5c5a at 5cb6724e), but its only changes are in chunk_prove (the 4 MB length
check) and a comment in agg_lite.rs: the handover code (main.rs's predecessor checks, src/xver) is the same, so these
images test the current handover path. Line numbers below are f5207731's.

| case | input | result | the guest's reason |
|---|---|---|---|
| control | keys-ch's genuine proof of 199 (keys-xv) | **passes** the collect pass (`HZEXIT:2`, the collect exit, reached after every handover check; step 2, the trie, starts), twice (normal host, bypass host) | - |
| 4 | a genuine predecessor proof at height 200, not 199 (keys-xv) | **refused in the guest** | main.rs:2311 "aggregate: block height 200 != chain height 201" |
| 5 | the embedded key file is not the pinned key (image k5) | **refused in the guest** | main.rs:2284 "predecessor chain proof invalid (cross-version): \"the carried key is not its pin's\"" |
| 6 | the lineage pins keys-ch's image with ANOTHER image's genuine key (k6) | **refused in the guest** | main.rs:2284 "predecessor chain proof invalid (cross-version): \"the predecessor proof's key is not the pinned image's\"" |
| 7 | a two-entry lineage the predecessor does not have (k7) | **refused in the guest** | main.rs:2306 "handover: predecessor lineage mismatch" |
| 8 | a non-chain journal: keys-ch's GENUINE chunk proof (block 130,000 chunk 0, proved 299 s, `hzengine verify` OK) | **refused in the guest** | main.rs:2289 "decode prev: Custom(\"invalid value: integer `130000`, expected u8\")" (the proof itself passes the cross-version check: it is a real proof of the predecessor image; its journal is not a chain state) |

Cases 4 and 8 are first refused by the HONEST host (lz.rs: the predecessor journal's kind / image and its height,
r1_runs2.out: "refused-NOT-in-guest"). A malicious prover runs its own host, so for the in-guest result the test host
(a scratch copy on the box, never committed) carried a TEST-ONLY bypass (r1_hvbypass.py: HZ_TEST_HANDOVER_STATE, the host
builds its witnesses from the genuine state at 199 and skips its own two checks, while the guest still receives the bogus
journal and proof; rebuilt 274 s, r1_hvhost.sh). The control passes with the bypass host too. Case 8 as first written
(the predecessor image's self-test leaf) could not run: the core guest has no leaf self-test mode (`selftest-leaf` exits
1); a genuine chunk proof is the non-chain journal the review asked for.
