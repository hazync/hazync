#!/bin/bash
# Round 19: ChainState gains max_time + profile. Mainnet + signet checks on the laptop, execute mode.
R=~/libzync-gpu/r18; O=$R/r5/reg; rm -rf $O; mkdir -p $O/s200 $O/sig; H=$R/host_main; HS=$R/host_signet
S=~/libzync-gpu/ssweep; B=~/dev/hazync-libzync/prover/block_962000.json; J=~/libzync-gpu/signet/json
REF=~/dev/hazync-libzync/scripts/libzync/utreexo_ref.py
export HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=4 HAZYNC_HINT_THREADS=2
echo "== 1. block 962,000 valid"
HAZYNC_BLOCK=$B $H libzync-hints $O/b962.lzb 2>&1 | grep -a 'batch:\|ecmh:'
HAZYNC_BLOCK=$B HAZYNC_LZ_HINTS=$O/b962.lzb $H agg-exec-fake 2>&1 | grep -a 'validation cycles\|panicked at\|INVALID' | grep -v collect; echo "rc=${PIPESTATUS[0]}"
echo "== 2. negative: is_base = 2 (must be refused)"
HAZYNC_BLOCK=$B HAZYNC_LZ_HINTS=$O/b962.lzb HAZYNC_TEST_ISBASE2=1 $H agg-exec-fake 2>&1 | grep -a -o 'is_base must be 0 or 1, got 2' | head -1 || echo NOT_REFUSED
echo "== 3. mainnet sweep 1..200 (saved), reference forest, max_time/profile; then a flip"
export HAZYNC_SS_HINTS=~/libzync-gpu/r15/h200.hints
HAZYNC_SWEEP_SAVE=$O/s200 $H swiftsync-sweep 200 $S 2>&1 | grep -a 'ss_set=\|SWEEP\|panicked'
python3 $REF $S 200 $O/s200/state_200.json | tail -1
python3 - $O/s200/state_200.json $S <<'PY'
import json,sys,glob
st=json.load(open(sys.argv[1]))
mx=max(json.load(open(f"{sys.argv[2]}/block_{h}.json"))["time"] for h in range(0,201) if glob.glob(f"{sys.argv[2]}/block_{h}.json"))
print(f"  max_time {st['max_time']} (block files' largest time 1..200: {mx}) -> {'OK' if st['max_time']==max(mx,1231006505) else 'WRONG'}; profile 0x{st['profile']:x} -> {'OK' if st['profile']==1 else 'WRONG'}")
PY
HAZYNC_SS_FLIP=9:0 $H swiftsync-sweep 200 $S 2>&1 | grep -a 'SWEEP'
unset HAZYNC_SS_HINTS
echo "== 4. signet sweep 1..2600 (saved): reference forest, profile = libzync|signet"
HAZYNC_SS_HINTS=~/dev/hazync-libzync/experimental/libzync-bench/signet/signet2600.hints HAZYNC_SWEEP_SAVE=$O/sig \
  $HS swiftsync-sweep 2600 $J 2>&1 | grep -a 'ss_set=\|forest:\|SWEEP\|panicked'
python3 $REF $J 2600 $O/sig/state_2600.json | tail -1
python3 -c "import json;s=json.load(open('$O/sig/state_2600.json'));print(f\"  profile 0x{s['profile']:x} ->\", 'OK' if s['profile']==5 else 'WRONG', ' max_time', s['max_time'])"
echo "== 5. signet BIP325 check still works"
$HS signet-check-test $J/block_2523.json 2>&1 | tail -2
echo R4_TESTS_DONE
