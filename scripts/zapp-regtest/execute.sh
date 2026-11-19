#!/bin/bash
# execute.sh <regtest host> <work dir>: the committed chain through the regtest libzync host in DEV MODE (every guest
# executed and its claims checked; nothing proved, the receipts are FakeReceipts), staged exactly as the GPU batch
# proves it (gpu/pod/run.sh), with the GPU batch's settings:
#   base  libzync-prove-range 1 101 from genesis (coinbase-only blocks; reused if <work>/out101 is complete)
#   A     the base resumed (HAZYNC_RESUME=1) over main 1..110 + a 111..112
#   control: B resumed on a copy of A's output that still holds A's 111..112 must be REFUSED (A's receipt at 112 is not
#         B's chain: the replayed BIP30 tree differs)
#   B     A's output cut back to 110 (every file of 111..112 removed), resumed over main 1..110 + b 111..113
#   anchor: state_112's anchor_id == scripts/libzync/genesis_anchor_id.py regtest (the spec's genesis anchor)
# Writes <work>/stats_{base,a,b}.tsv (per block: proofs, segments, total cycles, user cycles, from the host's LZ_STATS
# lines) and prints EXECUTE_OK. HAZYNC_SEG_PO2 defaults to 21 here, the CUDA host's segment size, so the segment counts
# are the GPU's.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); Z=$(cd "$HERE/../.." && pwd)
H=$(readlink -f "${1:?regtest host}"); W=${2:?work dir}; mkdir -p "$W"; W=$(readlink -f "$W")
export RISC0_DEV_MODE=1 HAZYNC_COINLOG_SAVE_EACH=1 HAZYNC_SEG_PO2=${HAZYNC_SEG_PO2:-21}
export HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=${HAZYNC_CHUNKS:-1}
"$H" method-id | head -1
for b in ma mb; do rm -rf "${W:?}/$b"; mkdir -p "$W/$b"; for h in $(seq 1 110); do ln -s "$HERE/blocks/main/block_$h.json" "$W/$b/"; done; done
for h in 111 112; do ln -s "$HERE/blocks/a/block_$h.json" "$W/ma/"; done
for h in 111 112 113; do ln -s "$HERE/blocks/b/block_$h.json" "$W/mb/"; done
fails=0
cut_above() {  # cut_above <out dir> <k>: remove every per-height file above k (receipt, state, coin log, trie, bundle)
  local f h; for f in "$1"/block_*.rcpt "$1"/state_*.* "$1"/bundle_*.lzb; do
    [ -e "$f" ] || continue
    h=$(basename "$f" | sed -E 's/^[a-z]+_([0-9]+)\..*/\1/'); [ "$h" -gt "$2" ] && rm -f "$f"; done; return 0; }
prove() { nice -n 19 "$H" libzync-prove-range "$@"; }
# BASE_RUN=<out dir> BASE_LOG=<its log>: take the base from an earlier dev-mode run from genesis over the same block files
# that reached 101 or beyond (cut back to 101, its LZ_STATS lines for 1..101 kept): blocks 1..101 hold no transaction,
# so no chunk, and HAZYNC_CHUNKS does not change them
if [ -n "${BASE_RUN:-}" ] && [ ! -f "$W/base.log" ]; then
  n=$(grep -ao "LZ_PROVE_RANGE_DONE 1..=[0-9]*" "$BASE_LOG" | tail -1 | sed 's/.*=//')
  [ -n "$n" ] && [ "$n" -ge 101 ] || { echo "FAIL: BASE_LOG $BASE_LOG did not finish a range 1..>=101"; exit 1; }
  rm -rf "$W/out101"; cp -r "$BASE_RUN" "$W/out101"; cut_above "$W/out101" 101
  { echo "# base taken from $BASE_RUN (a dev-mode run 1..$n from genesis), cut back to 101"
    awk '/^LZ_STATS /{split($3,h,"="); if (h[2] <= 101) print}' "$BASE_LOG"; echo "BASE_FROM_RUN_DONE 1..=$n"; } > "$W/base.log"
elif ! grep -aq "LZ_PROVE_RANGE_DONE 1..=101\|BASE_FROM_RUN_DONE" "$W/base.log" 2>/dev/null; then
  rm -rf "$W/out101"; prove 1 101 "$W/ma" genesis "$W/out101" > "$W/base.log" 2>&1
fi
grep -aq "LZ_PROVE_RANGE_DONE 1..=101\|BASE_FROM_RUN_DONE" "$W/base.log" && echo "PASS: base 1..101 accepted" || { echo "FAIL: base"; tail -5 "$W/base.log"; exit 1; }
rm -rf "$W/outa"; cp -r "$W/out101" "$W/outa"
HAZYNC_RESUME=1 prove 1 112 "$W/ma" genesis "$W/outa" > "$W/a.log" 2>&1
grep -aq "resuming after block 101" "$W/a.log" && grep -aq "LZ_PROVE_RANGE_DONE 1..=112" "$W/a.log" \
  && echo "PASS: A 102..112 accepted" || { echo "FAIL: A"; tail -5 "$W/a.log"; fails=$((fails+1)); }
rm -rf "$W/outc"; cp -r "$W/outa" "$W/outc"
HAZYNC_RESUME=1 prove 1 113 "$W/mb" genesis "$W/outc" > "$W/c.log" 2>&1; rc=$?
if [ $rc != 0 ] && grep -aq "replayed BIP30 tree at 112 != the receipt's smt_root" "$W/c.log"; then echo "PASS: control: resuming B on A's 112 refused"
else echo "FAIL: control (exit $rc): $(grep -a -m1 'panicked' "$W/c.log")"; fails=$((fails+1)); fi
rm -rf "$W/outc" "$W/outb"; cp -r "$W/outa" "$W/outb"; cut_above "$W/outb" 110
HAZYNC_RESUME=1 prove 1 113 "$W/mb" genesis "$W/outb" > "$W/b.log" 2>&1
grep -aq "resuming after block 110" "$W/b.log" && grep -aq "LZ_PROVE_RANGE_DONE 1..=113" "$W/b.log" \
  && echo "PASS: B 111..113 accepted (resumed after 110)" || { echo "FAIL: B"; tail -5 "$W/b.log"; fails=$((fails+1)); }
M=$("$H" method-id | head -1 | awk '{print $NF}')
python3 "$Z/scripts/libzync/genesis_anchor_id.py" regtest "$M" "$W/outa/state_112.json" | tail -1 | grep -q ANCHOR_MATCH \
  && echo "PASS: anchor_id == the spec's regtest genesis anchor" || { echo "FAIL: anchor"; fails=$((fails+1)); }
for r in base a b; do
  awk '/^LZ_STATS /{ split($3,h,"="); split($4,s,"="); split($5,t,"="); split($6,u,"=");
         n[h[2]]++; seg[h[2]]+=s[2]; tot[h[2]]+=t[2]; usr[h[2]]+=u[2] }
       END { print "height\tproofs\tsegments\ttotal_cycles\tuser_cycles";
             for (k in n) print k "\t" n[k] "\t" seg[k] "\t" tot[k] "\t" usr[k] }' "$W/$r.log" | sort -n > "$W/stats_$r.tsv"
done
sum() { awk -v w="$2" 'NR>1{p+=$2; s+=$3; t+=$4; u+=$5; n++} END{printf "%s: %d blocks, %d proofs, %d segments, %d total cycles, %d user cycles\n", w,n,p,s,t,u}' "$1"; }
sum "$W/stats_base.tsv" "base 1..101"; sum "$W/stats_a.tsv" "A 102..112"; sum "$W/stats_b.tsv" "B 111..113"
[ $fails = 0 ] && echo "EXECUTE_OK" || { echo "EXECUTE_FAILED $fails"; exit 1; }
