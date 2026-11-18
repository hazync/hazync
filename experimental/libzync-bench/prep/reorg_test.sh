#!/bin/bash
# Reorg phases P1 + P2 in libzync-prep (docs/libzync/REORG.md), on disk stores, mainnet 1..200 (prover/testdata), a save
# point every 50 blocks, undo records kept for J=60:
#   1. records: exactly u_141..u_200 in out/reorg/; each one's checksum, height, hash and prev match the states, and its
#      coin-log body spans the states' leaf counts; the states are byte-identical to a run with records off (J=0);
#   2. a crash after block 170 and a restart from the save point at 150: the records above 150 are removed at the
#      restart and written again, and all 60 are byte-identical to the uninterrupted run's;
#   3. a REORG marker in the block directory: exit 4 before anything is opened, no state written;
#   4. a block whose prev is not the state's tip, static files: exit 4 at that block (was a panic, a restart loop);
#   5. the same fed by the fetcher (HAZYNC_PREP_CONSUME=1): prep waits; the file replaced by one that extends the state,
#      it carries on and ends byte-identical; a marker written instead: exit 4; neither within HAZYNC_PREP_WAIT_S: exit 4;
#   7. (review 16.11) PREP_TIP.json says the stores' height, hash, J and the records' reach; no state_*.json.tmp is
#      left; the window is cut at save points only (after the crash at 170: 91..170, then J=60 below the save point);
#   6. a run that wrote no records (J=0: byte-identical stores and states to a host from before P1, case 1) continues
#      from its save point with records on: they start at the anchor + 1, and the states end byte-identical.
#   --control: prep_block's assert path (records off, no prev check) is NOT what is tested: the control runs case 4
#   with the block's prev intact and must then PASS that block, proving case 4 fails for the prev and nothing else.
set -u
H=${H:?the host: H=<path to host>}; B=${B:-$(cd "$(dirname "$0")/../../.." && pwd)/prover/testdata/libzync/mainnet}; O=${O:-/tmp/reorg-test}
rm -rf $O; mkdir -p $O
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_PREP_SAVE_EVERY=50
fails=0
check() { if [ "$1" = 1 ]; then echo "  ok   $2"; else echo "  FAIL $2"; fails=$((fails+1)); fi; }
prep() {  # prep <run dir> <lo> <hi> <anchor> [env...]: disk stores in <run dir>
    local r=$1 lo=$2 hi=$3 a=$4; shift 4
    env HAZYNC_COINLOG_DIR=$r/cl HAZYNC_TRIE_DIR=$r/trie "$@" $H libzync-prep $lo $hi ${BD:-$B} $a $r/out
}

# 1. the reference with records (J=60) and a run without (J=0)
prep $O/ref 1 200 genesis HAZYNC_PREP_UNDO_KEEP=60 > $O/ref.log 2>&1; rc=$?
check "$([ $rc = 0 ] && echo 1)" "1. reference run 1..200 with J=60 (exit $rc)"
prep $O/off 1 200 genesis HAZYNC_PREP_UNDO_KEEP=0 > $O/off.log 2>&1
same=1; for h in $(seq 1 200); do cmp -s $O/ref/out/state_$h.json $O/off/out/state_$h.json || same=0; done
for x in smt coinlog; do cmp -s $O/ref/out/state_200.$x $O/off/out/state_200.$x || same=0; done
# the .trie marker names its store's directory, which differs: compared without it
[ "$(sed 's#^HZTRIE-DISK [^ ]* ##' $O/ref/out/state_200.trie)" = "$(sed 's#^HZTRIE-DISK [^ ]* ##' $O/off/out/state_200.trie)" ] || same=0
# and the stores themselves, file by file (the coin log, its index, the trie arena and meta)
for s in cl trie; do for f in $(ls $O/ref/$s | grep -v '^LOCK$'); do cmp -s $O/ref/$s/$f $O/off/$s/$f || same=0; done; done
check "$same" "1. every state_<h>.json, the save point's .smt/.coinlog/.trie and every store file byte-identical with records off"
check "$([ ! -e $O/off/out/reorg ] || [ -z "$(ls -A $O/off/out/reorg)" ] && echo 1)" "1. J=0 writes no record"
python3 - "$O/ref/out" 141 200 > $O/records.txt <<'PY'
import hashlib, json, os, struct, sys
out, lo, hi = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
have = sorted(int(f[2:-4]) for f in os.listdir(f"{out}/reorg") if f.startswith("u_") and f.endswith(".bin"))
assert have == list(range(lo, hi + 1)), f"records {have[:3]}..{have[-3:]} ({len(have)}), want {lo}..{hi}"
st = lambda h: json.load(open(f"{out}/state_{h}.json"))
sizes = []
for h in have:
    b = open(f"{out}/reorg/u_{h}.bin", "rb").read()
    body, sum_ = b[:-32], b[-32:]
    assert hashlib.sha256(body).digest() == sum_ and body[:8] == b"LZUNDO01", h
    rh, = struct.unpack_from("<I", body, 8)
    assert rh == h, (h, rh)
    assert list(body[12:44]) == st(h)["tip_hash"] and list(body[44:76]) == st(h - 1)["tip_hash"], f"{h}: hash/prev"
    n, = struct.unpack_from("<Q", body, 76)
    cl = body[84:84 + n]
    n0, k = struct.unpack_from("<QQ", cl, 0)
    m, = struct.unpack_from("<Q", cl, 16 + 40 * k)
    assert n0 == st(h - 1)["utxo_leaves"] and n0 + m == st(h)["utxo_leaves"], f"{h}: coin log span {n0}+{m}"
    o = 84 + n
    rep = body[o]; o += 1
    nm, = struct.unpack_from("<I", body, o); o += 4 + 64 * nm
    ni, = struct.unpack_from("<I", body, o); o += 4 + 64 * ni
    ns, = struct.unpack_from("<I", body, o); o += 4 + 37 * ns
    assert o == len(body) and rep == 0 and nm == k, (h, o, len(body), rep, nm, k)
    sizes.append(len(b))
print(f"RECORDS_OK {len(have)} records {lo}..{hi}, {sum(sizes)} bytes, {min(sizes)}..{max(sizes)} per block")
PY
rc=$?
check "$([ $rc = 0 ] && echo 1)" "1. exactly u_141..u_200, each matching the states: $(tail -1 $O/records.txt)"
tipchk() {  # tipchk <out> <height> <undo_keep> <undo_from>: PREP_TIP.json is that, its hash state_<height>'s (display order)
    python3 - "$@" <<'PY'
import json, sys
out, h, k, f = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
t = json.load(open(f"{out}/PREP_TIP.json"))
want = bytes(json.load(open(f"{out}/state_{h}.json"))["tip_hash"])[::-1].hex()
sys.exit(0 if (t["height"], t["hash"], t["undo_keep"], t["undo_from"]) == (h, want, k, f) else 1)
PY
}
tipchk $O/ref/out 200 60 141; rc=$?
check "$([ $rc = 0 ] && [ -z "$(ls $O/ref/out | grep '\.tmp$')" ] && echo 1)" "7. PREP_TIP.json: height 200, state_200's hash, J 60, records from 141; no .tmp left"

# 2. crash after block 170 (save points at 50, 100, 150), restart from 150
prep $O/cr 1 200 genesis HAZYNC_PREP_UNDO_KEEP=60 HAZYNC_CRASH_AT=prep-after:170 > $O/crash.log 2>&1; rc=$?
check "$([ $rc = 9 ] && [ -e $O/cr/out/reorg/u_170.bin ] && [ ! -e $O/cr/out/reorg/u_171.bin ] && echo 1)" "2. died after block 170 (exit $rc), its record written"
n_cr=$(ls $O/cr/out/reorg | grep -c '^u_.*\.bin$'); lo_cr=$(ls $O/cr/out/reorg | sed -n 's/^u_\([0-9]*\)\.bin$/\1/p' | sort -n | head -1)
tipchk $O/cr/out 169 60 91; trc=$?
check "$([ "$n_cr" = 80 ] && [ "$lo_cr" = 91 ] && [ $trc = 0 ] && echo 1)" "7. after the crash: records 91..170 ($n_cr from $lo_cr; cut only at the save point 150), tip file at 169 (the last state written)"
prep $O/cr 151 200 $O/cr/out/state_150.json HAZYNC_PREP_UNDO_KEEP=60 HAZYNC_PREP_CONTINUE=1 > $O/resume.log 2>&1; rc=$?
same=1; for h in $(seq 141 200); do cmp -s $O/ref/out/reorg/u_$h.bin $O/cr/out/reorg/u_$h.bin || same=0; done
tipchk $O/cr/out 200 60 141; trc=$?
check "$([ $rc = 0 ] && grep -aq 'removed 20 reorg undo record(s) outside (90, 150]' $O/resume.log && [ "$same" = 1 ] && [ $(ls $O/cr/out/reorg | wc -l) = 60 ] && [ $trc = 0 ] && echo 1)" \
      "2. the restart removed the 20 records above 150 and wrote them again: all 60 byte-identical to the reference (exit $rc)"

# 3. a REORG marker: exit 4 before anything is opened
mkdir -p $O/mk; cp -r $B $O/mk/blocks; echo '{}' > $O/mk/blocks/REORG
BD=$O/mk/blocks prep $O/mk 1 200 genesis > $O/marker.log 2>&1; rc=$?
check "$([ $rc = 4 ] && grep -aq 'PREP_STOPPED_REORG at the start' $O/marker.log && [ ! -e $O/mk/out/state_1.json ] && echo 1)" "3. a marker: exit 4 at the start, nothing written (exit $rc)"

# 4. a block that does not extend the state, static files: exit 4 at it
mkdir -p $O/bad; cp -r $B $O/bad/blocks
if [ "${1:-}" != "--control" ]; then sed -i 's/"prev": "[0-9a-f]*"/"prev": "'"$(printf '%064d' 7)"'"/' $O/bad/blocks/block_101.json; fi
BD=$O/bad/blocks prep $O/bad 1 200 genesis > $O/bad.log 2>&1; rc=$?
check "$([ $rc = 4 ] && grep -aq 'PREP_STOPPED_REORG block 101 .* does not extend the state at 100: block files from another chain' $O/bad.log \
       && [ -e $O/bad/out/state_100.json ] && [ ! -e $O/bad/out/state_101.json ] && echo 1)" "4. static files, block 101 not extending 100: exit 4 at 101 (exit $rc)"
if [ "${1:-}" = "--control" ]; then
    [ $rc = 0 ] && { echo "CONTROL OK: with block 101's prev intact the run passes it (exit 0), so case 4 fails on the prev alone"; exit 0; }
    echo "CONTROL BROKEN: exit $rc"; exit 1
fi

# 5. fed by the fetcher (consume): wait, then (a) the file replaced, (b) a marker, (c) neither in time
# the stores at the save point 150 (the reference's are at 200)
prep $O/base 1 150 genesis HAZYNC_PREP_UNDO_KEEP=60 > $O/base.log 2>&1
consume() {  # consume <dir> [env...]: prep 151..200 from a copy of the stores at 150, in the background
    local r=$1; shift
    rm -rf $r; mkdir -p $r/out; cp -r $O/base/cl $O/base/trie $r/; cp $O/base/out/state_150.* $r/out/
    mkdir -p $r/blocks; for h in $(seq 151 200); do cp $B/block_$h.json $r/blocks/; done
    cp $r/blocks/block_171.json $r/good_171.json
    sed -i 's/"prev": "[0-9a-f]*"/"prev": "'"$(printf '%064d' 7)"'"/' $r/blocks/block_171.json
    BD=$r/blocks prep $r 151 200 $r/out/state_150.json HAZYNC_PREP_CONTINUE=1 HAZYNC_PREP_CONSUME=1 HAZYNC_PREP_UNDO_KEEP=60 "$@" > $r/run.log 2>&1 &
    PID=$!
    for _ in $(seq 1 100); do grep -aq 'waiting for the fetcher' $r/run.log && break; sleep 0.2; done
}
consume $O/c1; sleep 2; mv $O/c1/good_171.json $O/c1/blocks/block_171.json.tmp; mv $O/c1/blocks/block_171.json.tmp $O/c1/blocks/block_171.json
wait $PID; rc=$?
check "$([ $rc = 0 ] && grep -aq 'block 171 was replaced by one that extends the state: carrying on' $O/c1/run.log && cmp -s $O/ref/out/state_200.json $O/c1/out/state_200.json \
       && cmp -s $O/ref/out/reorg/u_200.bin $O/c1/out/reorg/u_200.bin && echo 1)" "5a. consume: waited at 171, the replaced file taken, state_200 and u_200 identical (exit $rc)"
consume $O/c2; sleep 2; echo '{}' > $O/c2/blocks/REORG
wait $PID; rc=$?
check "$([ $rc = 4 ] && grep -aq 'PREP_STOPPED_REORG block 171 .*, and .*REORG is there' $O/c2/run.log && [ ! -e $O/c2/out/state_171.json ] && echo 1)" "5b. consume: a marker while waiting: exit 4 (exit $rc)"
consume $O/c3 HAZYNC_PREP_WAIT_S=3
wait $PID; rc=$?
check "$([ $rc = 4 ] && grep -aq 'after 3 s (HAZYNC_PREP_WAIT_S), and the fetcher wrote no marker' $O/c3/run.log && echo 1)" "5c. consume: no verdict within HAZYNC_PREP_WAIT_S: exit 4 (exit $rc)"

# 6. a run without records (as the v0.26.0 host leaves it) continued by this host
prep $O/old 1 150 genesis HAZYNC_PREP_UNDO_KEEP=0 > $O/old.log 2>&1
prep $O/old 151 200 $O/old/out/state_150.json HAZYNC_PREP_CONTINUE=1 > $O/old2.log 2>&1; rc=$?
same=1; for h in $(seq 151 200); do cmp -s $O/ref/out/state_$h.json $O/old/out/state_$h.json || same=0; done
recs=$(ls $O/old/out/reorg 2>/dev/null | sort -t_ -k2 -n | sed -n '1p;$p' | tr '\n' ' ')
tipchk $O/old/out 200 2016 151; trc=$?
check "$([ $rc = 0 ] && [ "$same" = 1 ] && [ "$recs" = "u_151.bin u_200.bin " ] && [ $(ls $O/old/out/reorg | wc -l) = 50 ] && [ $trc = 0 ] && echo 1)" \
      "6. a run without records continues (default J, disk stores): records u_151..u_200, states 151..200 byte-identical (exit $rc)"

echo
[ $fails = 0 ] && { echo "all reorg prep checks passed"; exit 0; }
echo "$fails check(s) FAILED"; exit 1
