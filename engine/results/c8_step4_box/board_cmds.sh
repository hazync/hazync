#!/bin/bash
# C8 step 4 checks on hazync-coord, part 3: the board dry run's host commands (engine/results/board_dryrun) on mainnet
# 1..8, base host (engine/openvm 7620ec78) against the new host (c8-step4), keys-ch (the dry run's core image):
#   prep 1..8 with leaf bundles (states and bundles compared byte for byte), libzync-genesis-tip, method-id (board key),
#   seg-po2, prove-range-bridge for the leaves LEAVES (default 1 2; each journal compared, base vs new), fold-range of
#   1+2 (journals compared), verify-any (leaves, the fold, --check-end) and its refusals (a journal byte, a proof byte,
#   block 2 as range 1, no board key). <= 4 cores (taskset 12-15), nice. Usage: board_cmds.sh [leaves...]
set -u
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
R=/root/c8s4; E=$R/base/engine/host/target/release/hzengine; K=/root/engine/keys-ch
HB=$R/base/prover/target/release/host; HN=$R/new/prover/target/release/host; W=$R/board
LEAVES=${*:-1 2}
export RAYON_NUM_THREADS=4
T="taskset -c 12-15 nice"
t(){ local s=$(date +%s); "$@"; local rc=$?; echo "STEP rc=$rc secs=$(( $(date +%s)-s )) :: ${*: -3}"; return $rc; }
same() { if cmp -s "$1" "$2"; then echo "  SAME $3"; else echo "  DIFF $3 ($1 vs $2)"; fi; }
mkdir -p $W && cd $W
if [ ! -f $W/key.bin ]; then $E pack --dir $K --proof /root/engine/core200/block_200.ovm --out $W/b200.hze --key $W/key.bin > $W/pack.out; fi
PIN=$(awk '/^key_pin/{print $2}' $W/pack.out); echo "key pin ${PIN:0:16}.."
for x in base new; do
  H=$HB; [ $x = new ] && H=$HN
  rm -rf $W/$x && mkdir -p $W/$x/bundles $W/$x/prep
  t env HAZYNC_PREP_LEAF_BUNDLES=$W/$x/bundles HAZYNC_ENGINE_KEYS=$K $T $H libzync-prep 1 8 /root/engine/blocks genesis $W/$x/prep > $W/$x/prep.log 2>&1
  echo "  $x genesis tip $($H libzync-genesis-tip)"
  echo "  $x method-id (board key) $(HAZYNC_ENGINE_VK=$W/key.bin HAZYNC_ENGINE_VK_PIN=$PIN $H method-id | head -1)"
  echo "  $x method-id (keys dir) $(HAZYNC_ENGINE_KEYS=$K $H method-id | head -1)"
  echo "  $x seg-po2 (engine set up) $(HAZYNC_ENGINE=$E $H seg-po2)"
done
echo "  new method-id with no engine set up: $($HN method-id 2>&1 | head -c 160); exit ${PIPESTATUS[0]}"
for f in $(cd $W/base/prep && ls state_*.json); do same $W/base/prep/$f $W/new/prep/$f "prep $f"; done
for f in $(cd $W/base/bundles && ls leaf_*.bin); do same $W/base/bundles/$f $W/new/bundles/$f "bundle $f"; done
for x in base new; do
  H=$HB; [ $x = new ] && H=$HN
  for n in $LEAVES; do
    (cd $W/$x && t env HAZYNC_BRIDGE_OUT=$W/$x/bundles HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=$K $T $H prove-range-bridge $n > leaf_$n.log 2>&1)
  done
  set -- $LEAVES
  if [ $# -ge 2 ]; then (cd $W/$x && t env HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=$K $T $H fold-range range_$1.bin range_$2.bin fold_$1_$2.bin > fold.log 2>&1); fi
done
python3 - $W $LEAVES <<'PY'
import sys, hashlib, struct
W, leaves = sys.argv[1], sys.argv[2:]
def journal(p):
    b = open(p, "rb").read()
    assert b[:4] == b"HZE1", p
    n = struct.unpack("<I", b[4:8])[0]
    return b[8 + n:]
names = [f"range_{n}.bin" for n in leaves] + ([f"fold_{leaves[0]}_{leaves[1]}.bin"] if len(leaves) >= 2 else [])
for f in names:
    try:
        a, b = open(f"{W}/base/{f}", "rb").read(), open(f"{W}/new/{f}", "rb").read()
    except FileNotFoundError as e:
        print(f"  MISSING {e}"); continue
    ja, jb = journal(f"{W}/base/{f}"), journal(f"{W}/new/{f}")
    print(f"  {f}: journals {'IDENTICAL' if ja == jb else 'DIFFER'} ({len(ja)} bytes, sha256 {hashlib.sha256(jb).hexdigest()[:16]}..), proof files {'identical' if a == b else 'differ'} ({len(a)} / {len(b)} bytes)")
PY
for x in base new; do for f in $(cd $W/$x && ls *.log | grep -E 'leaf_|fold'); do echo "  $x $f: $(grep -a 'JOURNAL_SHA256\|range_[0-9]*.bin\|JOINED' $W/$x/$f | tr '\n' ' ' | cut -c1-200)"; done; done
echo "== verify-any (new host, board key)"
V="env HAZYNC_ENGINE_VK=$W/key.bin HAZYNC_ENGINE_VK_PIN=$PIN HAZYNC_BRIDGE_OUT=$W/new/bundles HAZYNC_LZ_STATE_DIR=$W/new/prep $HN verify-any"
set -- $LEAVES
for n in $LEAVES; do echo "  leaf $n: $($V $W/new/range_$n.bin $n --check-end 2>&1 | cut -c1-150)"; done
if [ $# -ge 2 ]; then echo "  fold $1+$2: $($V $W/new/fold_$1_$2.bin $1 --check-end 2>&1 | cut -c1-150)"; fi
python3 -c "b=bytearray(open('$W/new/range_1.bin','rb').read());b[-5]^=1;open('$W/bad_journal.bin','wb').write(b)"
python3 -c "b=bytearray(open('$W/new/range_1.bin','rb').read());b[2000]^=1;open('$W/bad_proof.bin','wb').write(b)"
echo "  tampered journal byte: $($V $W/bad_journal.bin 1 2>&1 | cut -c1-200); exit $?"
echo "  tampered proof byte: $($V $W/bad_proof.bin 1 2>&1 | cut -c1-200)"
[ -f $W/new/range_2.bin ] && echo "  block 2 as range 1: $($V $W/new/range_2.bin 1 2>&1 | cut -c1-200)"
echo "  no board key: $(env HAZYNC_BRIDGE_OUT=$W/new/bundles $HN verify-any $W/new/range_1.bin 1 2>&1 | cut -c1-200)"
echo "== the same verify-any lines from the base host"
VB="env HAZYNC_ENGINE_VK=$W/key.bin HAZYNC_ENGINE_VK_PIN=$PIN HAZYNC_BRIDGE_OUT=$W/base/bundles HAZYNC_LZ_STATE_DIR=$W/base/prep $HB verify-any"
for n in $LEAVES; do echo "  leaf $n: $($VB $W/base/range_$n.bin $n --check-end 2>&1 | cut -c1-150)"; done
if [ $# -ge 2 ]; then echo "  fold $1+$2: $($VB $W/base/fold_$1_$2.bin $1 --check-end 2>&1 | cut -c1-150)"; fi
echo BOARD_CMDS_DONE
