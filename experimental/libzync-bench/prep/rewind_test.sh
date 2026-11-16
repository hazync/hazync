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
#   --control: one byte of the rewound run's coin log changed after case A's rewinds: the comparison (the stores'
#   CONTENT digest) must catch it, so case A must FAIL.
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
same_as_ref() {  # same_as_ref <root> <ref root> <hi> <F> <tag> [noleaves]
    local r=$1 q=$2 hi=$3 f=$4 t=$5 ok=1 why=""
    for h in $(seq 1 $hi); do cmp -s $r/out/state_$h.json $q/out/state_$h.json || { ok=0; why="$why state_$h"; break; }; done
    for x in smt coinlog flags forest; do [ ! -e $q/out/state_$hi.$x ] || cmp -s $r/out/state_$hi.$x $q/out/state_$hi.$x || { ok=0; why="$why .$x"; }; done
    [ "$(sed 's#^HZTRIE-DISK [^ ]* ##' $r/out/state_$hi.trie)" = "$(sed 's#^HZTRIE-DISK [^ ]* ##' $q/out/state_$hi.trie)" ] || { ok=0; why="$why .trie"; }
    # (case F: a bundle carries its block's JSON as the FETCHER built it, with fields the test data files lack; the
    #  bundles above F must still have moved, and the states and stores are compared in full)
    if [ "${6:-}" = noleaves ]; then [ -z "$(ls $r/leaves/orphaned/1 2>/dev/null | sed -n 's/^leaf_\([0-9]*\)\.bin$/\1/p' | awk -v f=$f '$1<=f')" ] || { ok=0; why="$why leaves"; }
    else diff -rq --exclude=orphaned $r/leaves $q/leaves >/dev/null || { ok=0; why="$why leaves"; }; fi
    diff -rq $r/out/reorg $q/out/reorg >/dev/null || { ok=0; why="$why records"; }
    cmp -s $r/out/PREP_TIP.json $q/out/PREP_TIP.json || { ok=0; why="$why tip"; }
    local dr dq; dr=$(digest $r $hi); dq=$(digest $q $hi)
    [ -n "$dr" ] && [ "$dr" = "$dq" ] || { ok=0; why="$why stores($(echo "$dr" | awk '{print $1}' | tr '\n' ' '))"; }
    [ ! -e $r/blocks/REORG ] && [ ! -e $r/out/REWIND ] || { ok=0; why="$why markers"; }
    # trie/roots.log (ZAPP_FOLLOWER.md Z1): the same (height, block hash, root hash) for 1..hi (the slot references and
    # generations differ: arena layout and opens), and no entry of an orphaned block
    python3 - $r/trie/roots.log $q/trie/roots.log <<'PY' || { ok=0; why="$why roots.log"; }
import sys
def ents(p):
    b = open(p, "rb").read()
    return [(int.from_bytes(b[i:i + 4], "little"), b[i + 4:i + 36], b[i + 44:i + 76]) for i in range(0, len(b), 80)]
def newest(e):
    # a height logged twice (a rewind logs F's root again, in its new generation): the newest entry is the one that counts
    d = {}
    for x in e: d[x[0]] = x
    return [d[h] for h in sorted(d)]
a, q = newest(ents(sys.argv[1])), newest(ents(sys.argv[2]))
sys.exit(0 if a == q and [e[0] for e in a] == list(range(1, len(a) + 1)) else 1)
PY
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
[ $CONTROL = 1 ] && echo "CONTROL: a byte of each rewound coin log is changed before the comparison"

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
    [ $CONTROL = 1 ] && python3 -c "import sys; p=sys.argv[1]; b=bytearray(open(p,'rb').read()); b[40] ^= 1; open(p,'wb').write(b)" $r/cl/L0.bin
    same_as_ref $r $q $hi $f "A depth $d"
done
[ $CONTROL = 1 ] && { [ $fails -gt 0 ] && { echo "CONTROL OK: $fails check(s) fail: the changed coin log was caught"; exit 0; }; echo "CONTROL BROKEN: nothing failed"; exit 1; }

echo "== H. a stale journal (review 16.11, H1): prep died between the trie's flush at 200 and the journal's commit"
# HAZYNC_TEST_PREP_CRASH_AT=200 (dev mode): exit right after the trie's flush at 200, before its marker; pending.bin still
# holds 151..200 and the runner takes 200 as the save point (the trie is at 200). A rewind must undo only the journal's
# blocks ABOVE the fork, as a load does.
for fk in 198 200; do
    d=$((200 - fk)); hi=$((fk + 2)); br=$O/hbranch$fk
    python3 $HERE/reorg_branch.py $B $fk 2 h$fk $br > /dev/null
    q=$O/href$fk; reference $q $fk $br $hi
    r=$O/h$fk; root $r; cp $B/block_*.json $r/blocks/; run $r 200 HAZYNC_TEST_PREP_CRASH_AT=200
    check "$([ -e $r/out/state_200.coinlog ] && [ ! -e $r/out/state_200.trie ] && [ -s $r/cl/pending.bin ] && echo 1)" "H fork $fk: prep died after the trie's flush at 200, the journal (151..200) left"
    rm -f $r/blocks/block_*.json; cp $br/block_*.json $r/blocks/; marker $r $fk $d
    run $r $hi; rc=$?
    check "$([ $rc = 0 ] && grep -aq "PREP_REWOUND to $fk" $r/run.log && echo 1)" "H fork $fk: rewound and re-prepped (exit $rc)"
    same_as_ref $r $q $hi $fk "H fork $fk"
done

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

echo "== F. end to end, nothing faked but the node: the fetcher (follow, AUTO) from a fake bitcoin-cli serving real blocks,"
echo "   prep through the runner restarted as systemd would; the node reorgs at depth 2 while prep waits for block 201"
d=2; f=$((200 - d)); hi=$((f + d + 2)); br=$O/branch$d; q=$O/ref$d
r=$O/f$d; root $r; cp $REPO/prover/fetch_block_rpc.py $r/bin/
python3 - $B $br $f $r/world.json <<'PY'
import json, sys
b, br, f, w = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
json.dump({"chain": [f"{b}/block_{h}.json" for h in range(1, 201)]}, open(w, "w"))
json.dump({"chain": [f"{b}/block_{h}.json" for h in range(1, f + 1)] + [f"{br}/block_{h}.json" for h in range(f + 1, f + 5)]}, open(w + ".new", "w"))
PY
export HAZYNC_RPC_CLI="python3 $HERE/reorg_fake_node.py $r/world.json"
( env HAZYNC_REORG_AUTO=1 HAZYNC_FETCH_POLL_S=0.3 python3 $r/bin/fetch_block_rpc.py follow 1 $r/blocks --window 300 --jobs 2 --after-saves $r/out > $r/fetch.log 2>&1 ) & FP=$!
( while true; do run $r $hi; rc=$?; echo "runner exit $rc" >> $r/runs.log; [ $rc = 5 ] || exit $rc; sleep 1; done ) & RP=$!
for _ in $(seq 1 600); do grep -q '"height": 200' $r/out/PREP_TIP.json 2>/dev/null && break; sleep 0.2; done
sleep 1; mv $r/world.json.new $r/world.json                     # the node reorgs: 199..202 of the branch
wait $RP; rc=$?
kill $FP 2>/dev/null; wait $FP 2>/dev/null
unset HAZYNC_RPC_CLI
check "$([ $rc = 0 ] && grep -q 'FETCH_REORG_AUTO .*fork at 198, depth 2' $r/fetch.log && grep -q "prep's rewind is done" $r/fetch.log \
       && grep -q 'runner exit 5' $r/runs.log && grep -aq 'PREP_REWOUND to 198' $r/run.log && echo 1)" \
      "F: the fetcher saw the reorg (auto), prep exited 5, the runner rewound, the fetcher carried on, prep reached $hi (exit $rc)"
same_as_ref $r $q $hi $f "F (end to end)" noleaves

echo "== G. crashes inside a rewind (each run again by the runner from out/REWIND): during the coin log's undo, between"
echo "   the trie's flush at F and its marker, half way through the moves, between removing REWIND and the marker"
d=12; f=$((200 - d)); hi=$((f + d + 2)); br=$O/branch$d; q=$O/ref$d
for hook in rewind-mid-coinlog trie-flush rewind-mid-moves rewind-before-marker; do
    r=$O/g-$hook; copy_template $r; cp $br/block_*.json $r/blocks/; marker $r $f $d
    if [ $hook = trie-flush ]; then env="HAZYNC_TEST_PREP_CRASH_AT=$f"; else env="HAZYNC_CRASH_AT=$hook"; fi
    (cd $r && env HAZYNC_COINLOG_DIR=$r/cl HAZYNC_TRIE_DIR=$r/trie HAZYNC_PREP_LEAF_BUNDLES=$r/leaves $env \
         $r/bin/host_lz libzync-prep-rewind $r/out $r/blocks >> $r/run.log 2>&1); rc=$?
    run $r $hi; rc2=$?
    check "$([ $rc != 0 ] && [ $rc != 4 ] && [ $rc2 = 0 ] && echo 1)" "G $hook: the rewind died (exit $rc), the runner finished it and prep (exit $rc2)"
    same_as_ref $r $q $hi $f "G $hook"
done

echo "== I. the journal and the fork against the save point: T > A > F (prep at 199, saved at 150, fork 145),"
echo "   and F exactly AT the save point with a journal above it (fork 150)"
for fk in 145 150; do
    hi=$((fk + 6)); br=$O/ibranch$fk
    python3 $HERE/reorg_branch.py $B $fk 6 i$fk $br > /dev/null
    q=$O/iref$fk; reference $q $fk $br $hi
    r=$O/i$fk; root $r; cp $B/block_*.json $r/blocks/; run $r 200 HAZYNC_CRASH_AT=prep-after:199
    check "$([ -e $r/out/state_150.coinlog ] && [ -s $r/cl/pending.bin ] && echo 1)" "I fork $fk: prep died after 199, saved at 150, the journal holds 151..199"
    for h in $(seq $((fk + 1)) 200); do rm -f $r/blocks/block_$h.json; done; cp $br/block_*.json $r/blocks/
    marker $r $fk $((199 - fk))
    run $r $hi HAZYNC_REORG_AUTO_DEPTH=60; rc=$?
    check "$([ $rc = 0 ] && grep -aq "PREP_REWOUND to $fk" $r/run.log && echo 1)" "I fork $fk: rewound and re-prepped to $hi (exit $rc)"
    same_as_ref $r $q $hi $fk "I fork $fk"
done

echo "== J. a rewind with a reader's PIN in the trie (ZAPP_FOLLOWER Z1), and a SECOND reorg on the new branch"
d=12; f=$((200 - d)); hi=$((f + d + 2)); br=$O/branch$d; q=$O/ref$d
r=$O/j; copy_template $r; echo "150 0" > $r/trie/PIN; cp $br/block_*.json $r/blocks/; marker $r $f $d
run $r $hi; rc=$?
check "$([ $rc = 0 ] && [ -n "$(ls $r/trie/retain 2>/dev/null)" ] && echo 1)" "J: rewound with a PIN at 150 (slots held in retain/) (exit $rc)"
same_as_ref $r $q $hi $f "J with a PIN"
# the second reorg: one block deep on the branch, at 201
s2=$O/src2; mkdir -p $s2; for h in $(seq 1 $f); do cp $B/block_$h.json $s2/; done; cp $br/block_*.json $s2/
br2=$O/branch-second; python3 $HERE/reorg_branch.py $s2 201 3 second $br2 > /dev/null
q2=$O/ref2; root $q2; for h in $(seq 1 201); do cp $s2/block_$h.json $q2/blocks/; done; cp $br2/block_*.json $q2/blocks/; run $q2 204
cp $br2/block_*.json $r/blocks/; marker $r 201 1
run $r 204; rc=$?
check "$([ $rc = 0 ] && grep -aq 'PREP_REWOUND to 201 (epoch 2)' $r/run.log && [ -e $r/out/orphaned/2/state_202.json ] && echo 1)" "J: a second reorg (fork 201, epoch 2) (exit $rc)"
same_as_ref $r $q2 204 201 "J second reorg"

echo "== D. refusals: nothing changed"
fp() { (cd $1 && find cl trie out -type f ! -name LOCK -printf '%p %s %T@\n' | sort | sha256sum); }
r=$O/d1; copy_template $r; marker $r 187 13 manual; before=$(fp $r)
run $r 210; rc=$?
check "$([ $rc = 4 ] && grep -aq 'PREP_REWIND_REFUSED .*--allow-deep' $r/run.log && [ "$(fp $r)" = "$before" ] && [ ! -e $r/out/REWIND ] && echo 1)" "D: depth W+1 (manual) without --allow-deep: exit 4, nothing changed"
r=$O/d2; copy_template $r; marker $r 195 5; rm $r/out/reorg/u_197.bin; before=$(fp $r)
run $r 210; rc=$?
check "$([ $rc = 4 ] && grep -aq 'u_197.bin.*the records do not reach the fork' $r/run.log && [ "$(fp $r)" = "$before" ] && echo 1)" "D: a missing record: exit 4, the stores unchanged"
r=$O/d3; copy_template $r; echo '{}' > $r/blocks/REORG_TOO_DEEP; before=$(fp $r)
(cd $r && env HAZYNC_COINLOG_DIR=$r/cl HAZYNC_TRIE_DIR=$r/trie HAZYNC_PREP_LEAF_BUNDLES=$r/leaves $r/bin/host_lz libzync-prep-rewind $r/out $r/blocks >> $r/run.log 2>&1); rc=$?
check "$([ $rc = 4 ] && grep -aq 'REORG_TOO_DEEP' $r/run.log && [ "$(fp $r)" = "$before" ] && echo 1)" "D: REORG_TOO_DEEP: exit 4, nothing changed"

r=$O/d4; copy_template $r; marker $r 195 5; before=$(fp $r)
(cd $r && env HAZYNC_COINLOG_DIR=$r/cl HAZYNC_TRIE_DIR=$r/trie $r/bin/host_lz libzync-prep-rewind $r/out $r/blocks >> $r/run.log 2>&1); rc=$?
check "$([ $rc = 4 ] && grep -aq 'HAZYNC_PREP_LEAF_BUNDLES is not set' $r/run.log && [ "$(fp $r)" = "$before" ] && echo 1)" "D: the leaf bundles' directory not said: exit 4, nothing changed"

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
