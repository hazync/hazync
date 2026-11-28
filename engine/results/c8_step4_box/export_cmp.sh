#!/bin/bash
# C8 step 4 checks on hazync-coord, part 2: engine-export of the repo's five real blocks (prover/block_*.json) with the
# base host (engine/openvm 7620ec78, RISC0_SKIP_BUILD) and the new host (c8-step4), the same hzengine and keys (keys-ch,
# the board dry run's core image); every file compared byte for byte. The base writes METHOD_ID = zero at bytes 4..36
# of agg.in / agg_collect.in and in method_id; the new host its engine image id: compared after putting the new id
# into the base's bytes at exactly those places. Usage: export_cmp.sh [blocks...]
set -u
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
R=/root/c8s4; E=$R/base/engine/host/target/release/hzengine; K=/root/engine/keys-ch
HB=$R/base/prover/target/release/host; HN=$R/new/prover/target/release/host
BL=${*:-130000 140000 741000 962000 965500}
export RAYON_NUM_THREADS=4; mkdir -p $R/exp
run() { # run <host> <block> <out>
  rm -rf "$3"; local s=$(date +%s)
  HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=$K HAZYNC_BLOCK=$R/new/prover/block_$2.json HAZYNC_COINLOG_TXCOUNTS=$R/new/experimental/libzync-bench/cost/txcounts.txt \
    taskset -c 12-15 nice "$1" engine-export "$3" > "$3.log" 2>&1; local rc=$?
  echo "  $(basename $(dirname $(dirname $(dirname $(dirname $1))))) block $2: rc=$rc $(( $(date +%s)-s )) s, $(grep -a ENGINE_EXPORT_DONE $3.log | cut -c1-120)"
}
for b in $BL; do
  run $HB $b $R/exp/base_$b; run $HN $b $R/exp/new_$b
  python3 - $R/exp/base_$b $R/exp/new_$b <<'PY'
import os, sys
a, b = sys.argv[1], sys.argv[2]
fa = sorted(f for f in os.listdir(a) if os.path.isfile(f"{a}/{f}") and f != "LOCK")
fb = sorted(f for f in os.listdir(b) if os.path.isfile(f"{b}/{f}") and f != "LOCK")
if fa != fb: print(f"  FILES DIFFER: only base {set(fa)-set(fb)}, only new {set(fb)-set(fa)}"); sys.exit(1)
nid = open(f"{b}/method_id", "rb").read(); oid = open(f"{a}/method_id", "rb").read()
print(f"  method_id: base {oid.hex()[:16]}.. new {nid.hex()[:16]}..")
same, idonly, bad = 0, [], []
for f in fa:
    x, y = open(f"{a}/{f}", "rb").read(), open(f"{b}/{f}", "rb").read()
    if x == y: same += 1; continue
    if f == "method_id": idonly.append(f); continue
    # the id at bytes 4..36 only
    if len(x) == len(y) and x[4:36] == oid and y[4:36] == nid and x[:4] + x[36:] == y[:4] + y[36:]: idonly.append(f); continue
    d = next(i for i in range(min(len(x), len(y))) if x[i] != y[i]) if len(x) == len(y) else -1
    bad.append(f"{f} (lengths {len(x)} / {len(y)}, first difference at {d})")
print(f"  {len(fa)} files: {same} byte-identical, {len(idonly)} differ only by the image id at 4..36 ({' '.join(idonly)}), {len(bad)} other differences {bad}")
print("EXPORT_CMP", "OK" if not bad else "DIFF", a.rsplit('_', 1)[1])
PY
done
echo EXPORT_CMP_DONE
