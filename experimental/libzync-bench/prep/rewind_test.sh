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
# prep alone: no coordinator acks the rewinds here (P4, the board's side, is tested by prep/reorg_e2e.sh)
export HAZYNC_REORG_ACK=0
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
    # (the rewind's own records for the board, reorg/records/<E>.json, P4, are there in the rewound run only)
    diff -rq --exclude=records $r/out/reorg $q/out/reorg >/dev/null || { ok=0; why="$why records"; }
    ls $r/out/reorg/records/*.json >/dev/null 2>&1 || { ok=0; why="$why no-board-record"; }
    cmp -s $r/out/PREP_TIP.json $q/out/PREP_TIP.json || { ok=0; why="$why tip"; }
    local dr dq; dr=$(digest $r $hi); dq=$(digest $q $hi)
    [ -n "$dr" ] && [ "$dr" = "$dq" ] || { ok=0; why="$why stores($(diff <(echo "$dr") <(echo "$dq") | sed -n 's/^< \([A-Z]*\) .*/\1/p' | tr '\n' ' '))"; }
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
# every check's line also into $O/control.out (the control counts what failed)
check() { local l; if [ "$1" = 1 ]; then l="  ok   $2"; else l="  FAIL $2"; fails=$((fails+1)); fi; echo "$l"; echo "$l" >> $O/control.out; }
[ $CONTROL = 1 ] && echo "CONTROL: a byte of each rewound coin log is changed before the comparison"

# L. a reorg inside prep's replay window (review round 3, HIGH-1; CI): prep died after 199 (saved at 150, the states
# 151..199 on disk), the node reorged at 190; the restarted prep, replaying from 150, must see that its old state_191 is
# another chain's: its own marker, exit 5, the runner's rewind (stale states moved, the board's record), then the new
# branch. --control-stale: the check off (HAZYNC_TEST_SKIP_STALE_CHECK): the case MUST fail.
case_l() {
    local fk=190 hi=196 br=$O/lbranch q=$O/lref r=$O/l rc env=""
    [ "${1:-}" = off ] && env="HAZYNC_TEST_SKIP_STALE_CHECK=1"
    python3 $HERE/reorg_branch.py $B $fk 6 l$fk $br > /dev/null
    reference $q $fk $br $hi
    root $r; cp $B/block_*.json $r/blocks/; run $r 200 HAZYNC_CRASH_AT=prep-after:199
    check "$([ -e $r/out/state_150.coinlog ] && [ -e $r/out/state_198.json ] && echo 1)" "L: prep died after 199, saved at 150, states above it on disk (to 198)"
    for h in $(seq $((fk + 1)) 200); do rm -f $r/blocks/block_$h.json; done; cp $br/block_*.json $r/blocks/
    run $r $hi $env; rc=$?
    check "$([ $rc = 5 ] && grep -aq 'a reorg inside the replay window' $r/run.log && [ -e $r/blocks/REORG ] && echo 1)" \
          "L: prep replaying from 150 sees its old state_191 is another chain's: its marker, exit 5 (exit $rc)"
    run $r $hi $env; rc=$?
    check "$([ $rc = 0 ] && grep -aq "PREP_REWOUND to $fk" $r/run.log && [ -e $r/out/reorg/records/1.json ] && [ -e $r/out/orphaned/1/state_198.json ] && echo 1)" \
          "L: the runner rewound to $fk: the stale states 191..198 moved, the board's record written (exit $rc)"
    same_as_ref $r $q $hi $fk "L"
}
if [ "${1:-}" = --control-stale ]; then
    echo "CONTROL --control-stale: the replay-window check off; case L MUST fail"
    case_l off
    [ $fails -gt 0 ] && { echo "CONTROL OK: $fails check(s) fail with the check off, as they must"; exit 0; }
    echo "CONTROL BROKEN: case L passed with the check off"; exit 1
fi
echo "== L. a reorg inside prep's replay window"
case_l

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
if [ $CONTROL = 1 ]; then
    # exactly the three comparisons fail, each on the coin log's content, and nothing else (review 16.11, N3): the
    # rewinds themselves passed, so it is the comparison that caught the changed byte
    nf=$(grep -c '^  FAIL ' $O/control.out || true); nc=$(grep '^  FAIL ' $O/control.out | grep -c 'stores(COINLOG' || true)
    [ "$fails" = 3 ] && [ "$nf" = 3 ] && [ "$nc" = 3 ] && { echo "CONTROL OK: exactly the 3 comparisons failed, each on the coin log (stores(COINLOG))"; exit 0; }
    echo "CONTROL BROKEN: $fails failure(s), $nc of them on the coin log's content"; exit 1
fi

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
# Bounded, and LOUD when a bound runs out (a loaded machine must not turn this into a non-test in which the node reorgs
# above prep's tip and nothing is rewound): HAZYNC_TEST_F_WAIT_S (default 1800) for prep to reach 200, and again for it
# to finish after the reorg.
F_WAIT=${HAZYNC_TEST_F_WAIT_S:-1800}
killtree() { local c; for c in $(pgrep -P $1 2>/dev/null); do killtree $c; done; kill $1 2>/dev/null; }
waited() {  # waited <condition command...>: 0 once it holds, 1 when F_WAIT runs out or the runner died first
    local t0; t0=$(date +%s)
    until "$@"; do
        [ $(( $(date +%s) - t0 )) -ge $F_WAIT ] && return 1
        kill -0 $RP 2>/dev/null || { "$@"; return; }
        sleep 0.2
    done
}
at200() { grep -q '"height": 200' $r/out/PREP_TIP.json 2>/dev/null; }
runner_done() { ! kill -0 $RP 2>/dev/null; }
if ! waited at200; then
    check 0 "F: prep did not reach 200 within $F_WAIT s (HAZYNC_TEST_F_WAIT_S), or its runner died: the end-to-end case did NOT run (see $r/run.log)"
    killtree $RP; killtree $FP; wait $RP $FP 2>/dev/null
else
    sleep 1; mv $r/world.json.new $r/world.json                 # the node reorgs: 199..202 of the branch
    if ! waited runner_done; then
        check 0 "F: after the reorg prep did not finish within $F_WAIT s (HAZYNC_TEST_F_WAIT_S) (see $r/run.log, $r/fetch.log)"
        killtree $RP; rc=timeout
    else
        wait $RP; rc=$?
    fi
    killtree $FP; wait $FP 2>/dev/null
    check "$([ "$rc" = 0 ] && grep -q 'FETCH_REORG_AUTO .*fork at 198, depth 2' $r/fetch.log && grep -q "prep's rewind is done" $r/fetch.log \
           && grep -q 'runner exit 5' $r/runs.log && grep -aq 'PREP_REWOUND to 198' $r/run.log && echo 1)" \
          "F: the fetcher saw the reorg (auto), prep exited 5, the runner rewound, the fetcher carried on, prep reached $hi (exit $rc)"
    same_as_ref $r $q $hi $f "F (end to end)" noleaves
fi
unset HAZYNC_RPC_CLI

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

echo "== J0. a run PINNED from the start (PIN 150: its trie holds every root from 150 on), rewound below nothing it pins"
echo "   (fork 188) and run on: the root of 150 must still read as state_150's trie (review 16.11, N1)"
d=12; f=$((200 - d)); hi=$((f + d + 2)); br=$O/branch$d; q=$O/ref$d
# (the PIN goes in after the first save point: the runner clears the stores of a run with none, PIN included)
r=$O/j0; root $r; cp $B/block_*.json $r/blocks/; run $r 100; echo "150 0" > $r/trie/PIN; run $r 200
cp $br/block_*.json $r/blocks/; marker $r $f $d
run $r $hi; rc=$?
want=$(python3 -c "import json,sys; print(bytes(json.load(open(sys.argv[1]))['trie_root']).hex())" $r/out/state_150.json 2>/dev/null || python3 -c "import json,sys; print(bytes(json.load(open(sys.argv[1]))['trie_root']).hex())" $r/out/orphaned/1/state_150.json)
got=$( (cd $r && HAZYNC_COINLOG_DIR=$r/cl HAZYNC_TRIE_DIR=$r/trie "$H" libzync-store-digest $r/out $hi --root-at 150 2>/dev/null) | awk '/^ROOT_AT/{print $3}')
intact=$(awk '{print $2}' $r/trie/INTACT)
check "$([ $rc = 0 ] && [ -n "$got" ] && [ "$got" = "$want" ] && [ "$intact" -le 150 ] && echo 1)" "J0: rewound and run on with PIN 150: the root of 150 reads whole as state_150's ($got), INTACT from $intact (exit $rc)"
same_as_ref $r $q $hi $f "J0 pinned run"

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

echo "== K. the board's record and ack (P4 review): an epoch never reused (H1), another rewind's record refused, and a"
echo "   rewind that stopped before removing the marker finished without a new epoch (L5)"
rw() {  # rw <root> [env...]: the rewind as the runner runs it
    local r=$1; shift
    (cd $r && env HAZYNC_COINLOG_DIR=$r/cl HAZYNC_TRIE_DIR=$r/trie HAZYNC_PREP_LEAF_BUNDLES=$r/leaves "$@" \
         $r/bin/host_lz libzync-prep-rewind $r/out $r/blocks >> $r/run.log 2>&1)
}
r=$O/k; copy_template $r; marker $r 198 2
rw $r; rc=$?
check "$([ $rc = 0 ] && [ -e $r/out/reorg/records/1.json ] && echo 1)" "K: a rewind to 198 (epoch 1; no coordinator: HAZYNC_REORG_ACK=0) (exit $rc)"
# out/orphaned/ cleared by hand; epoch 1's ack still in the ack directory
mkdir -p $r/acks; python3 -c "import hashlib,json,sys; print(json.dumps({'record_sha256': hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest()}))" \
    $r/out/reorg/records/1.json > $r/acks/1.ack
rm -rf $r/out/orphaned; r1=$(sha256sum < $r/out/reorg/records/1.json)
marker $r 196 2
rw $r HAZYNC_REORG_ACK=1 HAZYNC_REORG_ACK_DIR=$r/acks HAZYNC_REORG_ACK_TIMEOUT_S=3; rc=$?
check "$([ $rc = 4 ] && [ -e $r/out/reorg/records/2.json ] && [ "$(sha256sum < $r/out/reorg/records/1.json)" = "$r1" ] && [ -e $r/out/REWIND ] \
         && grep -aq PREP_REWIND_NO_ACK $r/run.log && echo 1)" \
      "K H1: the next rewind opens epoch 2 (not 1 again, whose old ack would pass), waits, no ack: exit 4 (exit $rc)"
python3 - $r/out/reorg/records/2.json <<'PY2'
import json, sys
v = json.load(open(sys.argv[1])); v["fork"] = 100; open(sys.argv[1], "w").write(json.dumps(v))
PY2
rw $r HAZYNC_REORG_ACK=0; rc=$?
check "$([ $rc = 4 ] && grep -aq "is another rewind's record" $r/run.log && [ -e $r/out/REWIND ] && echo 1)" \
      "K H1: records/2.json of another rewind (fork 100) under the intent of epoch 2: exit 4, the intent kept (exit $rc)"
r=$O/k5; copy_template $r; marker $r 198 2
rw $r; rc=$?
fh=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['hash'])" $r/out/PREP_TIP.json)
printf '{"fork": 198, "depth": 2, "class": "auto", "prep_tip": 200, "fork_hash": "%s"}\n' $fh > $r/blocks/REORG
n0=$(ls $r/out/orphaned | wc -l)
rw $r; rc2=$?
check "$([ $rc = 0 ] && [ $rc2 = 0 ] && [ ! -e $r/blocks/REORG ] && [ "$(ls $r/out/orphaned | wc -l)" = $n0 ] && [ ! -e $r/out/reorg/records/2.json ] \
         && grep -aq 'already at the fork' $r/run.log && echo 1)" \
      "K L5: the marker left after a finished rewind: removed, no new epoch, no new record (exit $rc, $rc2)"

echo
[ $fails = 0 ] && { echo "all rewind checks passed"; exit 0; }
echo "$fails check(s) FAILED"; exit 1
