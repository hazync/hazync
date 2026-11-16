#!/bin/bash
# execute.sh <regtest host> <work dir>: the committed chain through the regtest libzync host in DEV MODE (every guest
# executed and its claims checked; nothing proved, the receipts are FakeReceipts), the way the GPU batch will prove it:
#   A  libzync-prove-range 1 112 over main 1..110 + a 111..112, from genesis
#   control: B resumed on a copy of A's output that still holds A's 111..112 must be REFUSED (A's receipt at 112 is not
#      B's chain: the replayed BIP30 tree differs)
#   B  A's output cut back to 110 (receipts, states, coin logs, tries above 110 removed), resumed (HAZYNC_RESUME=1) over
#      main 1..110 + b 111..113
#   the anchor: state_112's anchor_id == scripts/libzync/genesis_anchor_id.py regtest (the spec's genesis anchor)
# Writes <work>/stats_{a,b}.tsv (per block: proofs, segments, total cycles, user cycles, from the host's LZ_STATS lines)
# and prints EXECUTE_OK. HAZYNC_SEG_PO2 defaults to 21 here, the CUDA host's segment size, so the segment counts are the
# GPU batch's. A run of A already in <work>/outa (LZ_PROVE_RANGE_DONE in a.log) is reused.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); Z=$(cd "$HERE/../.." && pwd)
H=$(readlink -f "${1:?regtest host}"); W=${2:?work dir}; mkdir -p "$W"; W=$(readlink -f "$W")
export RISC0_DEV_MODE=1 HAZYNC_COINLOG_SAVE_EACH=1 HAZYNC_SEG_PO2=${HAZYNC_SEG_PO2:-21}
"$H" method-id | head -1
for b in ma mb; do rm -rf "$W/$b"; mkdir -p "$W/$b"; for h in $(seq 1 110); do ln -s "$HERE/blocks/main/block_$h.json" "$W/$b/"; done; done
for h in 111 112; do ln -s "$HERE/blocks/a/block_$h.json" "$W/ma/"; done
for h in 111 112 113; do ln -s "$HERE/blocks/b/block_$h.json" "$W/mb/"; done
fails=0
if ! grep -aq "LZ_PROVE_RANGE_DONE 1..=112" "$W/a.log" 2>/dev/null; then
  rm -rf "$W/outa"; nice -n 19 "$H" libzync-prove-range 1 112 "$W/ma" genesis "$W/outa" > "$W/a.log" 2>&1
fi
grep -aq "LZ_PROVE_RANGE_DONE 1..=112" "$W/a.log" && echo "PASS: A 1..112 accepted" || { echo "FAIL: A"; tail -5 "$W/a.log"; fails=$((fails+1)); }
# the control: A's 112 left in place
rm -rf "$W/outc"; cp -r "$W/outa" "$W/outc"
HAZYNC_RESUME=1 nice -n 19 "$H" libzync-prove-range 1 113 "$W/mb" genesis "$W/outc" > "$W/c.log" 2>&1; rc=$?
if [ $rc != 0 ] && grep -aq "replayed BIP30 tree at 112 != the receipt's smt_root" "$W/c.log"; then echo "PASS: control: resuming B on A's 112 refused"
else echo "FAIL: control (exit $rc): $(grep -a -m1 'panicked' "$W/c.log")"; fails=$((fails+1)); fi
rm -rf "$W/outc" "$W/outb"; cp -r "$W/outa" "$W/outb"
for h in 111 112; do rm -f "$W/outb"/{block,state,bundle}_$h.*; done
HAZYNC_RESUME=1 nice -n 19 "$H" libzync-prove-range 1 113 "$W/mb" genesis "$W/outb" > "$W/b.log" 2>&1
grep -aq "resuming after block 110" "$W/b.log" && grep -aq "LZ_PROVE_RANGE_DONE 1..=113" "$W/b.log" \
  && echo "PASS: B resumed after 110, 111..113 accepted" || { echo "FAIL: B"; tail -5 "$W/b.log"; fails=$((fails+1)); }
M=$("$H" method-id | head -1 | awk '{print $NF}')
python3 "$Z/scripts/libzync/genesis_anchor_id.py" regtest "$M" "$W/outa/state_112.json" | tail -1 | grep -q ANCHOR_MATCH \
  && echo "PASS: anchor_id == the spec's regtest genesis anchor" || { echo "FAIL: anchor"; fails=$((fails+1)); }
for r in a b; do
  awk '/^LZ_STATS /{ split($3,h,"="); split($4,s,"="); split($5,t,"="); split($6,u,"=");
         n[h[2]]++; seg[h[2]]+=s[2]; tot[h[2]]+=t[2]; usr[h[2]]+=u[2] }
       END { print "height\tproofs\tsegments\ttotal_cycles\tuser_cycles";
             for (k in n) print k "\t" n[k] "\t" seg[k] "\t" tot[k] "\t" usr[k] }' "$W/$r.log" | sort -n > "$W/stats_$r.tsv"
done
awk 'NR>1{p+=$2; s+=$3; t+=$4; u+=$5; n++} END{printf "A: %d blocks, %d proofs, %d segments, %d total cycles, %d user cycles\n", n,p,s,t,u}' "$W/stats_a.tsv"
awk 'NR>1 && $1>110{p+=$2; s+=$3; t+=$4; u+=$5; n++} END{printf "B above 110: %d blocks, %d proofs, %d segments, %d total cycles, %d user cycles\n", n,p,s,t,u}' "$W/stats_b.tsv"
[ $fails = 0 ] && echo "EXECUTE_OK" || { echo "EXECUTE_FAILED $fails"; exit 1; }
