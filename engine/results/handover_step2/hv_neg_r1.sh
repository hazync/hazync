#!/bin/bash
# HANDOVER step 2, negatives added after review round 1 (R1-5 #3, 27.11.2026). NOT RUN YET: it needs hazync-coord's CPU
# and step 2's artefacts there (keys-xv, the test-lineage host /root/engine-src-xv, xvpred, core200), and keys-xv must be
# REBUILT first (the round-1 guest fixes move every engine image id). Same harness as hv_neg.sh: each predecessor input
# must be REFUSED by the guest's ovm-xver check in the handover block's collect pass (execute only, no proof).
#
# Runtime cases (inputs only), here:
#   4. a GENUINE proof of the predecessor image at another height (keys-ch's block 200, not 199): main.rs's height
#      checks (w.height == p.first_height, w.height == prev.height + 1) must refuse it.
# Cases that need a purpose-built image, not inputs (documented, to build when the image set is rebuilt):
#   5. key bytes != pin: an image embedding a key file whose SHA-256 is not the lineage pin. assemble.sh refuses such a
#      file by name and check_xver_provenance.py by record, so the build needs both bypassed (a test-only assembly).
#   6. another image's GENUINE key: the lineage pins image A with the pin of image B's real key (a test record that says
#      so, HZ_ENGINE_RECORDS=<dir>): verify-core Refusal::Image in the guest must refuse.
#   7. a predecessor lineage mismatch: a predecessor proof whose journal commits another lineage (an image with a
#      non-empty lineage of its own as the predecessor).
#   8. a non-chain journal: a genuine chunk proof of the predecessor image as the "predecessor proof" (kind check).
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
E=/root/engine-kit/engine/host/target/release/hzengine; HX=/root/engine-src-xv/prover/target/release/host
P=/root/engine/xvpred; N=/root/engine/xvneg_r1; rm -rf "${N:?}"; mkdir -p $N; cd /root/engine || exit 1
case_() { # <name> <dir with block_199.ovm.{journal,bin} + state_199.{coinlog,trie}>
  local d=$2; rm -rf "${d:?}/out"
  HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=/root/engine/keys-xv HAZYNC_HANDOVER_PROOF=$d/block_199.ovm \
    HAZYNC_COINLOG_TXCOUNTS=/root/engine-kit/experimental/libzync-bench/cost/txcounts.txt \
    timeout 3600 nice $HX libzync-prove-range 200 200 /root/engine/xchain/blocks handover $d/out > $d/run.log 2>&1; local rc=$?
  local why=$(sed 's/\x1b\[[0-9;]*m//g' $d/run.log $d/out/engine/s0/*.log 2>/dev/null | grep -a "predecessor\|panicked at\|xver\|height" | grep -av "^\s*$" | head -2 | tr '\n' ' ' | cut -c1-230)
  [ $rc != 0 ] && echo "refused  $1 (rc $rc): $why" || echo "ACCEPTED $1"
}
mk() { local d=$N/$1; mkdir -p $d; cp $P/state_199.coinlog $P/state_199.trie $d/; echo $d; }
# 4. block 200's genuine proof and journal (same predecessor image, keys-ch) in place of block 199's
d=$(mk other_height)
$E wasm-inputs --dir keys-ch --proof /root/engine/core200/block_200.ovm --out $d/wi > $d/wi.log 2>&1 && cp $d/wi/proof.bin $d/block_199.ovm.bin
cp /root/engine/core200/block_200.ovm.journal $d/block_199.ovm.journal
case_ "a genuine proof of the predecessor image at height 200 (not 199)" $d
echo HV_NEG_R1_DONE
