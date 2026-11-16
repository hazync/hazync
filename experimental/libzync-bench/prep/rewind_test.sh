#!/bin/bash
# Reorg phase P3, prep's rewind (docs/libzync/REORG.md §5-§6, §12 "prep round trip"), through the deployed runner
# (coordinator/deploy/libzync/libzync-prep-run.sh) with HAZYNC_REORG_AUTO=1, on disk stores: mainnet 1..200
# (prover/testdata) as the OLD chain, a synthetic branch from F = 200 - d (reorg_branch.py) as the NEW one, W = 12,
# J = 60, a save point every 50 blocks. For each case the stores and states after the rewind and the re-prep of the new
# branch must equal a FRESH prep of the new branch (mainnet 1..F + the branch, from genesis):
#   - every state_<h>.json byte-identical, and the leaf bundles, the undo records and PREP_TIP.json;
#   - the save point at the new tip: .smt, .coinlog, the forest and flags markers byte-identical, .trie without its
#     store path;
#   - the stores' CONTENT (`host libzync-store-digest`): the trie's canonical snapshot, the coin log's leaves, the
#     position index's live entries (their files differ: arena slots and tombstones depend on history);
#   - the orphaned states moved to out/orphaned/1/, the marker and out/REWIND gone.
# Cases: A. stores saved at 200, a fork of depth 1, 2 and W (12): undone from the records;
#        B. prep stopped between save points (journal of 151..200, depth 1 and 12): nothing to undo, the states above F
#           move and prep resumes from 150;
#        C. the automatic path end to end, depth 2: prep waiting for block 201 sees the marker, exits 5, the runner
#           (started again, as systemd does) rewinds and prep goes on;
#        D. refusals, nothing changed: depth W+1 without --allow-deep; a missing record; REORG_TOO_DEEP;
#        E. a crash in the middle of a rewind (after the save point at F, before the moves): run again, same result;
#           and --allow-deep for depth W+1.
#   --control: the rewind's coin log undo skipped (records ignored): case A must FAIL.
set -u
H=${H:?the host: H=<path to host>}; HERE=$(cd "$(dirname "$0")" && pwd); REPO=$(cd "$HERE/../../.." && pwd)
B=${B:-$REPO/prover/testdata/libzync/mainnet}; O=${O:-/tmp/rewind-test}; RUN=$REPO/coordinator/deploy/libzync/libzync-prep-run.sh
rm -rf $O; mkdir -p $O
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_PREP_SAVE_EVERY=50 HAZYNC_PREP_UNDO_KEEP=60 HAZYNC_REORG_AUTO_DEPTH=12 HAZYNC_PREP_WAIT_S=60
fails=0
check() { if [ "$1" = 1 ]; then echo "  ok   $2"; else echo "  FAIL $2"; fails=$((fails+1)); fi; }
root() {  # root <dir>: an LZ_ROOT with bin/
    rm -rf $1; mkdir -p $1/bin $1/blocks; ln -s "$H" $1/bin/host_lz; cp "$RUN" $1/bin/; chmod +x $1/bin/libzync-prep-run.sh
}
run() {  # run <root> <hi> [env...]: the runner, as the unit runs it
    local r=$1 hi=$2; shift 2
    env LZ_ROOT=$r LZ_HI=$hi HAZYNC_REORG_AUTO=1 "$@" bash $r/bin/libzync-prep-run.sh >> $r/run.log 2>&1
}
marker() {  # marker <root> <fork> <depth> [class]
    printf '{"fork": %d, "depth": %d, "class": "%s", "prep_tip": 200}\n' $2 $3 "${4:-auto}" > $1/blocks/REORG
}
digest() { (cd $1 && HAZYNC_COINLOG_DIR=$1/cl HAZYNC_TRIE_DIR=$1/trie "$H" libzync-store-digest $1/out $2 2>/dev/null); }
same_as_ref() {  # same_as_ref <root> <ref root> <hi> <F> <tag>
    local r=$1 q=$2 hi=$3 f=$4 t=$5 ok=1 why=""
    for h in $(seq 1 $hi); do cmp -s $r/out/state_$h.json $q/out/state_$h.json || { ok=0; why="$why state_$h"; break; }; done
    for x in smt coinlog flags forest; do [ ! -e $q/out/state_$hi.$x ] || cmp -s $r/out/state_$hi.$x $q/out/state_$hi.$x || { ok=0; why="$why .$x"; }; done
    [ "$(sed 's#^HZTRIE-DISK [^ ]* ##' $r/out/state_$hi.trie)" = "$(sed 's#^HZTRIE-DISK [^ ]* ##' $q/out/state_$hi.trie)" ] || { ok=0; why="$why .trie"; }
    diff -rq $r/leaves $q/leaves >/dev/null || { ok=0; why="$why leaves"; }
    diff -rq $r/out/reorg $q/out/reorg >/dev/null || { ok=0; why="$why records"; }
    cmp -s $r/out/PREP_TIP.json $q/out/PREP_TIP.json || { ok=0; why="$why tip"; }
    local dr dq; dr=$(digest $r $hi); dq=$(digest $q $hi)
    [ -n "$dr" ] && [ "$dr" = "$dq" ] || { ok=0; why="$why stores($(echo "$dr" | awk '{print $1}' | tr '\n' ' '))"; }
    [ ! -e $r/blocks/REORG ] && [ ! -e $r/out/REWIND ] || { ok=0; why="$why markers"; }
    local orph; orph=$(ls $r/out/orphaned/1/ 2>/dev/null | grep -c '^state_[0-9]*\.json$')
    check $ok "$t: states, save point, leaves, records, tip and store contents = a fresh prep of the new branch; $orph orphaned states moved${why:+ — differs:$why}"
}
reference() {  # reference <ref root> <F> <branch dir> <hi>: a fresh prep of mainnet 1..F + the branch, from genesis
    root $1
    for h in $(seq 1 $2); do cp $B/block_$h.json $1/blocks/; done
    cp $3/block_*.json $1/blocks/
    run $1 $4
}
CONTROL=0; [ "${1:-}" = --control ] && CONTROL=1
if [ $CONTROL = 1 ]; then export HAZYNC_TEST_REWIND_SKIP_COINLOG=1; echo "CONTROL: the rewind's coin log undo is skipped"; fi

# the old chain once: prep 1..200, stores saved at 200, kept as a template
T=$O/template; root $T; cp $B/block_*.json $T/blocks/; run $T 200; rc=$?
check "$([ $rc = 0 ] && [ -e $T/out/state_200.coinlog ] && echo 1)" "the old chain: prep 1..200 (exit $rc)"
copy_template() { rm -rf $1; cp -a $T $1; rm -f $1/bin/host_lz; ln -s "$H" $1/bin/host_lz; : > $1/run.log; }

echo "== A. the stores saved at 200; a fork of depth d, undone from the records"
for d in 1 2 12; do
    f=$((200 - d)); hi=$((f + d + 2)); br=$O/branch$d
    python3 $HERE/reorg_branch.py $B $f $((d + 2)) b$d $br > /dev/null
    q=$O/ref$d; reference $q $f $br $hi
    r=$O/a$d; copy_template $r; cp $br/block_*.json $r/blocks/; marker $r $f $d
    run $r $hi; rc=$?
    check "$([ $rc = 0 ] && grep -aq "PREP_REWOUND to $f (epoch 1)" $r/run.log && grep -aq "$d block(s) undone" $r/run.log && echo 1)" "A depth $d: rewound to $f and re-prepped to $hi (exit $rc)"
    same_as_ref $r $q $hi $f "A depth $d"
done
[ $CONTROL = 1 ] && { [ $fails -gt 0 ] && { echo "CONTROL OK: $fails check(s) fail without the coin log undo"; exit 0; }; echo "CONTROL BROKEN: nothing failed"; exit 1; }

echo "== B. prep stopped between save points (its journal holds 151..200)"
for d in 1 12; do
    f=$((200 - d)); hi=$((f + d + 2)); br=$O/branch$d; q=$O/ref$d
    r=$O/b$d; root $r; cp $B/block_*.json $r/blocks/; run $r 200 HAZYNC_CRASH_AT=prep-after:200
    check "$([ -e $r/out/state_150.coinlog ] && [ ! -e $r/out/state_200.coinlog ] && [ -s $r/cl/pending.bin ] && echo 1)" "B depth $d: prep died after applying 200, saved at 150, journal kept"
    cp $br/block_*.json $r/blocks/; marker $r $f $d
    run $r $hi; rc=$?
    check "$([ $rc = 0 ] && grep -aq 'the newest save point 150 is at or below the fork' $r/run.log && grep -aq "libzync-prep: 151..$hi from state_150.json" $r/run.log && echo 1)" "B depth $d: nothing to undo, prep resumed from 150 (exit $rc)"
    same_as_ref $r $q $hi $f "B depth $d"
done

echo "== C. the automatic path: prep waits for 201, the marker arrives, exit 5, the runner rewinds, prep goes on"
d=2; f=$((200 - d)); hi=$((f + d + 2)); br=$O/branch$d; q=$O/ref$d
r=$O/c$d; root $r; cp $B/block_*.json $r/blocks/
run $r $hi & P=$!
for _ in $(seq 1 300); do [ -e $r/out/state_200.coinlog ] && break; sleep 0.2; done
sleep 1; marker $r $f $d; cp $br/block_*.json $r/blocks/
wait $P; rc=$?
check "$([ $rc = 5 ] && grep -aq 'PREP_STOPPED_FOR_REWIND' $r/run.log && echo 1)" "C: prep exited 5 for the rewind (exit $rc)"
run $r $hi; rc=$?
check "$([ $rc = 0 ] && grep -aq "PREP_REWOUND to $f" $r/run.log && echo 1)" "C: the runner, started again, rewound and prep finished (exit $rc)"
same_as_ref $r $q $hi $f "C"

echo "== D. refusals: nothing changed"
fp() { (cd $1 && find cl trie out -type f ! -name LOCK -printf '%p %s %T@\n' | sort | sha256sum); }
r=$O/d1; copy_template $r; marker $r 187 13 manual; before=$(fp $r)
run $r 210; rc=$?
check "$([ $rc = 4 ] && grep -aq 'PREP_REWIND_REFUSED .*--allow-deep' $r/run.log && [ "$(fp $r)" = "$before" ] && [ ! -e $r/out/REWIND ] && echo 1)" "D: depth W+1 (manual) without --allow-deep: exit 4, nothing changed"
r=$O/d2; copy_template $r; marker $r 195 5; rm $r/out/reorg/u_197.bin; before=$(fp $r)
run $r 210; rc=$?
check "$([ $rc = 4 ] && grep -aq 'u_197.bin.*the records do not reach the fork' $r/run.log && [ "$(fp $r)" = "$before" ] && echo 1)" "D: a missing record: exit 4, the stores unchanged"
r=$O/d3; copy_template $r; echo '{}' > $r/blocks/REORG_TOO_DEEP; before=$(fp $r)
env LZ_ROOT=$r HAZYNC_REORG_AUTO=1 $r/bin/host_lz libzync-prep-rewind $r/out $r/blocks >> $r/run.log 2>&1; rc=$?
check "$([ $rc = 4 ] && grep -aq 'REORG_TOO_DEEP' $r/run.log && [ "$(fp $r)" = "$before" ] && echo 1)" "D: REORG_TOO_DEEP: exit 4, nothing changed"

echo "== E. a crash in the middle of a rewind, and --allow-deep"
d=12; f=$((200 - d)); hi=$((f + d + 2)); br=$O/branch$d; q=$O/ref$d
r=$O/e1; copy_template $r; cp $br/block_*.json $r/blocks/; marker $r $f $d
(cd $r && env HAZYNC_COINLOG_DIR=$r/cl HAZYNC_TRIE_DIR=$r/trie HAZYNC_PREP_LEAF_BUNDLES=$r/leaves HAZYNC_CRASH_AT=rewind-after-save \
     $r/bin/host_lz libzync-prep-rewind $r/out $r/blocks >> $r/run.log 2>&1); rc=$?
check "$([ $rc = 9 ] && [ -e $r/out/REWIND ] && [ -e $r/out/state_$f.coinlog ] && [ -e $r/out/state_200.json ] && echo 1)" "E: died after the save point at $f, before the moves (exit $rc; REWIND kept)"
run $r $hi; rc=$?
check "$([ $rc = 0 ] && grep -aq 'the stores are already at' $r/run.log && echo 1)" "E: the runner finished the rewind from out/REWIND (exit $rc)"
same_as_ref $r $q $hi $f "E (crash mid-rewind)"
d=13; f=$((200 - d)); hi=$((f + d + 2)); br=$O/branch$d
python3 $HERE/reorg_branch.py $B $f $((d + 2)) b$d $br > /dev/null
q=$O/ref$d; reference $q $f $br $hi
r=$O/e2; copy_template $r; cp $br/block_*.json $r/blocks/; marker $r $f $d manual
(cd $r && env HAZYNC_COINLOG_DIR=$r/cl HAZYNC_TRIE_DIR=$r/trie HAZYNC_PREP_LEAF_BUNDLES=$r/leaves $r/bin/host_lz libzync-prep-rewind $r/out $r/blocks --allow-deep >> $r/run.log 2>&1); rc=$?
run $r $hi; rc2=$?
check "$([ $rc = 0 ] && [ $rc2 = 0 ] && echo 1)" "E: depth 13 with --allow-deep (the runbook's one command), then the runner (exit $rc, $rc2)"
same_as_ref $r $q $hi $f "E depth 13 --allow-deep"

echo
[ $fails = 0 ] && { echo "all rewind checks passed"; exit 0; }
echo "$fails check(s) FAILED"; exit 1
