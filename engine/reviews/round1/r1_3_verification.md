# R1-3 verification: the batch signature fixes (27.11.2026)

The fixes (0382bb3f, engine/guest/batch_ecdsa_ovm.rs and msm_ovm.rs) against the reviewer's findings
(r1_3_batch_ecdsa.md), on the files themselves: before (0382bb3f^) and after.

## 1. The negatives, natively (engine/guest/batch-test)

engine/guest/batch-test compiles the guest's batch_ecdsa_ovm.rs and msm_ovm.rs on the host (OpenVM v2.1.0's k256 /
ecc / algebra guest crates have host implementations; RustCrypto's k256 0.13 builds the test points and scalars
independently) and drives `hazync_batch_defer` / `finish` as libsecp's hook does. build.rs inlines either file set
into one module, so the tests read the module's own state (DEFERRED, FALLBACK). Same test code for both:

    cd engine/guest/batch-test
    cargo test --release --features old -- --test-threads=1 --nocapture   # the files before the fixes (old/ = 0382bb3f^)
    cargo test --release -- --test-threads=1 --nocapture                  # the current files

| case | before (0382bb3f^) | after (0382bb3f) |
|---|---|---|
| (a) control: a valid BIP340 item, its true R hinted | deferred (1), finish true | deferred (1), finish true |
| (a) FORGED BIP340: r = 0, s = e·d (s·G − e·P = O; libsecp rejects), hint {valid=1, x=0, y=0} | **deferred (1), finish TRUE: the forgery passes** | not deferred (0: left to libsecp's own check), fallback 1 |
| (a) the same item with no hint (the guest's direct check) | 2 (invalid) | 2 (invalid) |
| (b) 6 valid ECDSA items | finish true | finish true |
| (b) item 2's R negated (x(R) mod n == r holds for both) | transcript bytes of item 2 **IDENTICAL** (160 bytes: the weights do not depend on the sign); finish false | transcript bytes DIFFER (192 bytes: R.y bound); finish false |
| (c) MSM window picked, 2^4 .. 2^21 points of 128 / 256 bits | **16** from 524,288 points of 256 bits (2^19, 2^20, 2^21) | at most 15 |
| (c) a digit of +32768 at c = 16, as the MSM stores it (i16) | -32768 (wrapped) | (c = 16 is no longer chosen) |
| (c) MSM vs a naive sum, n = 3, 4, 5, 9, 17, 40 (an identity base, a zero scalar, equal and opposite points) | equal | equal |

(b) shows the cause of the reviewer's attack, not the attack: one flipped R always fails the batch; the attack (a
subset-sum over many flips, Wagner's algorithm) needs weights that do not depend on the flips, which the old
transcript gave and the new one does not. (c) at c = 16 is a liveness bug (a valid batch rejected), never a forgery,
as the reviewer said.

## 2. Honest blocks: the same outcome on the fixed image

The fixed core image, built in the container from the commit (reproduce/engine-ids.sh, IMAGES=core) on the laptop,
twice: at d0fb2eac and at cd17699c (which only adds KEEP_VMEXE to the script), both

| image | id | exe | vm |
|---|---|---|---|
| core before the fixes (keys-ch) | c7fe0eb8... | 0ddcb255... | 00e00077... |
| **core with the fixes** | **8d1bae9c5bef0b0d6db6945a82bfd22d25605f73b55ca209edf18198225eee8f** | ca934e67b8ce91757d9ba861fd1f845a9410793f292bb81c7b90af0a013acc0e | 00e00077... (unchanged) |

guest.vmexe sha256 88583bbc...; keygen in the container gave sdk.pk f0155abe, agg.vk 15d415ec, openvm.toml 75b745db,
byte-identical to keys-ch's, as the vm commitment says.

Every chunk of five real blocks executed (no proof) on the fixed image through one `hzengine serve`
(r1_3_run_chunks.py; output r1_3_run_chunks.out), each journal compared with the export's recorded journal from the
old image (chunk_NN.j, the final-image runs of 26.11: edgef/, xf2/) and RISC Zero's (chunk_NN.r0journal):

| block (export) | chunks | journal == old image's | == RISC Zero's |
|---|---|---|---|
| 91,880 (edgef) | 1 | 1/1 | 1/1 |
| 185,956 (edgef) | 1 | 1/1 | 1/1 |
| 481,824 (edgef, SegWit activation) | 4 | 4/4 | 4/4 |
| 976,379 (xf2/976379t, 2026, Taproot spends) | 5 | 5/5 | 5/5 |
| 364,292 (edgef, the legacy-sighash block) | 16 | 16/16 | 16/16 |

27 of 27 byte-identical to both (the runner's last line counts 0/27 because `serve`'s execute reply carries exit:
null; the journals are what is compared). A chunk journal does not contain the image id, so nothing was exempted.
Control: the OLD image executed here on 91,880's chunk gives the recorded journal (== RISC Zero's).

The batch path ran on them (the guest's own log, `hzengine execute --log`): 976,379 chunk_04 deferred=1752
fallback=0 ok=true; 481,824 chunk_00 deferred=1396 fallback=113 ok=true; 481,824 chunk_03 deferred=1560 fallback=4
ok=true, the SAME counts as the old image's recorded log for that chunk (chunk_03.in.guest.log: deferred=1560
fallback=4 ok=true). So the fixes change no honest verdict and no honest item's path; only the weights (the seed now
covers R.y) and the image id change.

Not done here: proving (execution only); the other images (no zAPP guest links the batch module: no batch_ecdsa / hazync_batch_defer in prover/zyncapps/methods/*/src or engine/zapps, grep 27.11); the fixed image's
fixtures (the engine fixtures keep the old image's proofs and keys, and verify as such).
