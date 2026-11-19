#!/bin/bash
# run_regtest.sh <regtest host (built with --features trusted-state)> <ts-anchor> <work dir> [H]   -- EXPERIMENTAL
#
# The Trusted State proof of concept on the zAPP regtest harness (scripts/zapp-regtest), DEV MODE (every guest executed
# and its claims checked, nothing proved; docs/libzync/TRUSTED_STATE.md §5):
#   1. side inputs at H from the block files (headers, tracked coins, REPEATABLE txids)
#   2. the anchor at H from Core's own dumptxoutset (oracle/utxo_H.dat), refused unless its hash_serialized_3 and base
#      block are Core's (oracle/snap_H.json stands in for chainparams' m_assumeutxo_data)
#   3. the join check against the REAL receipt of the main lineage at H (receipts/a/block_H.rcpt, image eb2cd424)
#   4. the ANCHORED lineage: blocks H+1..112 (main to 110, branch a 111..112) on the UNCHANGED core image, block H+1 a
#      base step on the anchor (is_base = 1), no block file below H+1 in its directory
#   5. the MAIN lineage over the same blocks, from the main lineage's own state at H (receipts/a/state_H.*), for cycles
#   6. both lineages' states at 105 / 110 / 112 against each other and against the real receipts' states
#   7. negative controls: a wrong expected hash, a wrong tracked list, no REPEATABLE slot
# Writes <work>/*.log, <work>/cycles_<H>.tsv; prints TS_REGTEST_OK or TS_REGTEST_FAILED <n>.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); Z=$(cd "$HERE/../.." && pwd)
HOST=$(readlink -f "${1:?regtest host built with --features trusted-state}"); TS=$(readlink -f "${2:?ts-anchor}")
W=${3:?work dir}; H=${4:-104}; mkdir -p "$W"; W=$(readlink -f "$W")
G=$Z/scripts/zapp-regtest; R=$G/receipts/a; IMG=$(awk '{print $2}' "$G/receipts/method_id.txt")
export RISC0_DEV_MODE=1 HAZYNC_COINLOG_SAVE_EACH=1 HAZYNC_SEG_PO2=${HAZYNC_SEG_PO2:-21} HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1
fails=0
pass() { echo "PASS: $*"; }
fail() { echo "FAIL: $*"; fails=$((fails+1)); }
expect_of() { python3 -c "import json,sys; j=json.load(open(sys.argv[1])); print(f\"{j['height']}:{j['hash']}:{j['gettxoutsetinfo_hash_serialized_3']['hash_serialized_3']}\")" "$G/oracle/snap_$1.json"; }
"$HOST" method-id | head -1
# 1-2. side inputs and the anchor
python3 "$HERE/side_inputs.py" "$G/blocks/main" "$H" "$W/side_$H" || exit 1
"$TS" build --network regtest --snapshot "$G/oracle/utxo_$H.dat" --side "$W/side_$H" --expect "$(expect_of "$H")" \
      --out "$W/anchor_$H" --ingest "$W/anchor_$H.ingest" | tee "$W/anchor_$H.log" || { fail "anchor build"; exit 1; }
AID=$(awk '/^ANCHOR_ID/{print $2}' "$W/anchor_$H.log")
# 3. join check against the real main-lineage receipt at H
if "$TS" join --anchor "$W/anchor_$H.json" --main "$R/block_$H.rcpt" --image "$IMG" > "$W/join_$H.log" 2>&1 && grep -q JOIN_OK "$W/join_$H.log"
then pass "join at $H: anchored state = the real receipt's in every JOIN field"; else fail "join at $H"; tail -3 "$W/join_$H.log"; fi
# block dirs: the anchored lineage gets ONLY blocks H+1..112; the main lineage gets 1..112 (its BIP30 tree is replayed)
rm -rf "$W/blk_anc" "$W/blk_main"; mkdir -p "$W/blk_anc" "$W/blk_main"
for h in $(seq 1 110); do ln -s "$G/blocks/main/block_$h.json" "$W/blk_main/"; [ "$h" -gt "$H" ] && ln -s "$G/blocks/main/block_$h.json" "$W/blk_anc/"; done
for h in 111 112; do ln -s "$G/blocks/a/block_$h.json" "$W/blk_main/"; ln -s "$G/blocks/a/block_$h.json" "$W/blk_anc/"; done
# 4. the anchored lineage
rm -rf "$W/out_anc"
HAZYNC_TS_SMT_SEED="$W/anchor_$H.smt" nice -n 10 "$HOST" libzync-prove-range $((H+1)) 112 "$W/blk_anc" "$W/anchor_$H.json" "$W/out_anc" > "$W/anc.log" 2>&1
grep -aq "LZ_PROVE_RANGE_DONE $((H+1))..=112" "$W/anc.log" && pass "anchored lineage: blocks $((H+1))..112 accepted (base step on the anchor, then recursion)" \
  || { fail "anchored lineage"; grep -a -m3 "panicked\|REJECT" "$W/anc.log"; }
python3 - "$W/out_anc/state_112.json" "$AID" <<'P' && pass "anchored tip's anchor_id = ts-anchor's ANCHOR_ID" || fail "anchored anchor_id"
import json, sys
s = json.load(open(sys.argv[1])); sys.exit(0 if bytes(s["anchor_id"]).hex() == sys.argv[2] else 1)
P
# 5. the main lineage over the same blocks, from its own state at H (its coin log and trie: the receipts' files)
rm -rf "$W/main_anchor" "$W/out_main"; mkdir -p "$W/main_anchor"
cp "$R/state_$H.json" "$R/state_$H.coinlog" "$R/state_$H.trie" "$W/main_anchor/"
nice -n 10 "$HOST" libzync-prove-range $((H+1)) 112 "$W/blk_main" "$W/main_anchor/state_$H.json" "$W/out_main" > "$W/main.log" 2>&1
grep -aq "LZ_PROVE_RANGE_DONE $((H+1))..=112" "$W/main.log" && pass "main lineage: blocks $((H+1))..112 accepted from its state at $H" \
  || { fail "main lineage"; grep -a -m3 "panicked\|REJECT" "$W/main.log"; }
# 6. states: anchored vs main vs the real receipts (whose states the GPU batch proved)
for t in 105 110 112; do
  [ "$t" -le "$H" ] && continue
  "$TS" compare "$W/out_anc/state_$t.json" "$W/out_main/state_$t.json" > "$W/cmp_$t.log"
  for f in tip_hash utxo_value live_set live_hash cum_work recent_times max_time; do
    grep -q "^  $f  *equal" "$W/cmp_$t.log" || fail "state $t: $f differs between the lineages"; done
  for f in tip_hash utxo_value live_hash cum_work; do
    python3 -c "import json,sys; a=json.load(open(sys.argv[1])); b=json.load(open(sys.argv[2])); sys.exit(0 if a[sys.argv[3]]==b[sys.argv[3]] else 1)" \
      "$W/out_anc/state_$t.json" "$R/state_$t.json" "$f" 2>/dev/null || { [ -f "$R/state_$t.json" ] && fail "state $t: anchored $f != the real receipt's"; }
  done
  pass "state $t: live_hash, utxo_value, tip, work, time context equal on both lineages$([ -f "$R/state_$t.json" ] && echo " and the real receipt's")"
done
# per-block cycles, both lineages (LZ_STATS: proofs, segments, total, user)
for r in anc main; do
  awk '/^LZ_STATS /{ split($3,h,"="); split($4,s,"="); split($5,t,"="); split($6,u,"=");
         n[h[2]]++; seg[h[2]]+=s[2]; tot[h[2]]+=t[2]; usr[h[2]]+=u[2] }
       END { for (k in n) print k "\t" n[k] "\t" seg[k] "\t" tot[k] "\t" usr[k] }' "$W/$r.log" | sort -n > "$W/stats_$r.tsv"
done
{ printf "height\tanc_proofs\tanc_segments\tanc_total\tanc_user\tmain_proofs\tmain_segments\tmain_total\tmain_user\tuser_diff\n"
  join -t $'\t' "$W/stats_anc.tsv" "$W/stats_main.tsv" | awk -F'\t' -v OFS='\t' '{print $0, $5-$9}'; } > "$W/cycles_$H.tsv"
column -t "$W/cycles_$H.tsv"
# 7. negative controls
BAD=$(expect_of "$H" | python3 -c "import sys; s=sys.stdin.read().strip(); print(s[:-1] + ('1' if s[-1] == '0' else '0'))")
"$TS" build --network regtest --snapshot "$G/oracle/utxo_$H.dat" --side "$W/side_$H" --expect "$BAD" \
      --out "$W/neg_hash" > "$W/neg_hash.log" 2>&1 && fail "a wrong expected hash_serialized_3 was accepted" \
  || { grep -q "NOT the assumed snapshot" "$W/neg_hash.log" && pass "control: a wrong expected hash_serialized_3 is refused"; }
python3 "$HERE/side_inputs.py" "$G/blocks/main" 105 "$W/side_105" > /dev/null
cp "$W/side_105.headers" "$W/neg_trk.headers"; : > "$W/neg_trk.tracked"; cp "$W/side_105.repeatable" "$W/neg_trk.repeatable"
"$TS" build --network regtest --snapshot "$G/oracle/utxo_105.dat" --side "$W/neg_trk" --expect "$(expect_of 105)" --out "$W/neg_trk" > /dev/null
"$TS" join --anchor "$W/neg_trk.json" --main "$G/receipts/a/block_105.rcpt" --image "$IMG" > "$W/neg_trk_join.log" 2>&1 \
  && fail "control: an anchor with a coin's tracked bit dropped joined" \
  || { grep -q "live_hash  *JOIN *DIFFERS" "$W/neg_trk_join.log" && pass "control: a dropped tracked bit (same Core hash) is caught by the join (live_hash differs)"; }
if [ "$H" = 104 ]; then
  cp "$W/side_104.headers" "$W/neg_rep.headers"; cp "$W/side_104.tracked" "$W/neg_rep.tracked"; : > "$W/neg_rep.repeatable"
  "$TS" build --network regtest --snapshot "$G/oracle/utxo_104.dat" --side "$W/neg_rep" --expect "$(expect_of 104)" --out "$W/neg_rep" > /dev/null
  "$TS" join --anchor "$W/neg_rep.json" --main "$R/block_104.rcpt" --image "$IMG" > "$W/neg_rep_join.log" 2>&1 \
    && echo "  (no REPEATABLE slot: the anchor still joins at 104 -- the slot is not in any JOIN field)"
  rm -rf "$W/blk_neg" "$W/out_neg"; mkdir -p "$W/blk_neg"; ln -s "$G/blocks/main/block_105.json" "$W/blk_neg/"
  HAZYNC_TS_SMT_SEED="$W/neg_rep.smt" nice -n 10 "$HOST" libzync-prove-range 105 105 "$W/blk_neg" "$W/neg_rep.json" "$W/out_neg" > "$W/neg_rep.log" 2>&1
  if grep -aq "LZ_PROVE_RANGE_DONE 105..=105" "$W/neg_rep.log"; then
    python3 -c "import json,sys; a=json.load(open(sys.argv[1])); b=json.load(open(sys.argv[2])); sys.exit(0 if a['live_hash']!=b['live_hash'] else 1)" \
      "$W/out_neg/state_105.json" "$R/state_105.json" \
      && pass "control: without the REPEATABLE slot block 105 is accepted but its repeat coinbase is not tracked: live_hash at 105 != the main lineage's (divergence caught at the next join)" \
      || fail "control: no REPEATABLE slot changed nothing"
  else pass "control: without the REPEATABLE slot block 105 is refused: $(grep -a -m1 'panicked\|REJECT' "$W/neg_rep.log")"; fi
fi
[ $fails = 0 ] && echo "TS_REGTEST_OK" || { echo "TS_REGTEST_FAILED $fails"; exit 1; }
