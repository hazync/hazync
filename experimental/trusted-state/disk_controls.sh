#!/bin/bash
# disk_controls.sh <regtest host (--features trusted-state)> <ts-anchor> <work dir> -- EXPERIMENTAL (TRUSTED_STATE.md §14.2)
# The anchor's coin log and trie written straight into the host's disk stores (ts-anchor build --coinlog-dir / --trie-dir,
# then HAZYNC_COINLOG_DIR / HAZYNC_TRIE_DIR), and a resumed Trusted State run (HAZYNC_TS_SMT_SEED + HAZYNC_RESUME=1), on
# the regtest harness anchored at 104, blocks 105..112, dev mode. Checks:
#   1. the disk-store anchor's .json and .smt = the in-memory anchor's, byte for byte (same anchor_id and roots);
#   2. the anchored lineage from the disk stores reaches the in-memory run's state at 112, byte for byte;
#   3. a run stopped after 108 and resumed reaches the same state at 112 (in memory and on disk);
#   4. controls: a resume whose state_108.smt has one value changed is refused (its root is not block_108.rcpt's), and
#      one with state_108.smt missing is refused; each control's directory is otherwise the honest one.
# Prints DISK_CONTROLS_OK or DISK_CONTROLS_FAILED <n>.
set -uo pipefail
HOST=$(readlink -f "$1"); TS=$(readlink -f "$2"); W=$3
Z=$(cd "$(dirname "$0")/../.." && pwd); G=$Z/scripts/zapp-regtest
mkdir -p "$W"; cd "$W" || exit 1
export RISC0_DEV_MODE=1 HAZYNC_SEG_PO2=21 HAZYNC_CHUNKS=1
bad=0; pass() { echo "PASS $1"; }; fail() { echo "FAIL $1"; bad=$((bad+1)); }
tsa() { env -u RISC0_DEV_MODE "$TS" "$@"; }
E=$(python3 -c "import json,sys; j=json.load(open(sys.argv[1])); print(f\"{j['height']}:{j['hash']}:{j['gettxoutsetinfo_hash_serialized_3']['hash_serialized_3']}\")" "$G/oracle/snap_104.json")
python3 "$Z/experimental/trusted-state/side_inputs.py" "$G/blocks/main" 104 side > /dev/null
mkdir -p blk; for h in 105 106 107 108 109 110; do ln -sf "$G/blocks/main/block_$h.json" blk/; done
for h in 111 112; do ln -sf "$G/blocks/a/block_$h.json" blk/; done
# anchor <name> [disk]: a fresh anchor (the disk stores are advanced in place by a run, so each run gets its own)
anchor() {
  rm -rf "$1" "$1.cl" "$1.tr"; mkdir -p "$1"
  if [ "${2:-}" = disk ]; then tsa build --network regtest --snapshot "$G/oracle/utxo_104.dat" --side side --expect "$E" --out "$1/a" --coinlog-dir "$1.cl" --trie-dir "$1.tr" > "$1/build.log"
  else tsa build --network regtest --snapshot "$G/oracle/utxo_104.dat" --side side --expect "$E" --out "$1/a" > "$1/build.log"; fi
}
# run <anchor> <out> <hi> [disk] [env...]
run() {
  local a=$1 o=$2 hi=$3 d=${4:-}; shift 4
  local st=(); [ "$d" = disk ] && st=(HAZYNC_COINLOG_DIR="$W/$a.cl" HAZYNC_TRIE_DIR="$W/$a.tr")
  env "${st[@]}" HAZYNC_TS_SMT_SEED="$W/$a/a.smt" "$@" "$HOST" libzync-prove-range 105 "$hi" blk "$W/$a/a.json" "$o" > "$o.log" 2>&1
}
done112() { grep -aq "LZ_PROVE_RANGE_DONE 105..=112" "$1.log"; }
# 1
anchor mem; anchor dsk disk
cmp -s mem/a.json dsk/a.json && cmp -s mem/a.smt dsk/a.smt && grep -q "^ANCHOR_ID 8fce9ea6" dsk/build.log \
  && pass "1. disk-store anchor = in-memory anchor (.json, .smt; ANCHOR_ID 8fce9ea6...)" || fail "1. disk-store anchor differs"
# 2
rm -rf o_mem o_dsk; run mem o_mem 112 ""; run dsk o_dsk 112 disk
done112 o_mem && done112 o_dsk && cmp -s o_mem/state_112.json o_dsk/state_112.json \
  && pass "2. anchored lineage 105..112 from the disk stores: state_112 = the in-memory run's" || { fail "2. disk-store run"; grep -a -m2 "panicked" o_dsk.log; }
# 3
for m in mem disk; do
  a=r_$m; anchor "$a" "$m"; rm -rf "o_$a"
  run "$a" "o_$a" 108 "$m"; run "$a" "o_$a" 112 "$m" HAZYNC_RESUME=1
  done112 "o_$a" && grep -aq "resuming after block 108" "o_$a.log" && cmp -s o_mem/state_112.json "o_$a/state_112.json" \
    && pass "3. resumed after 108 ($m): state_112 = the unbroken run's" || { fail "3. resume ($m)"; grep -a -m2 "panicked" "o_$a.log"; }
done
# 4
anchor c1; rm -rf o_c1; run c1 o_c1 108 ""
python3 - o_c1/state_108.smt <<'PYEOF'
import sys
p = sys.argv[1]; l = open(p).read().splitlines(); k, v = l[0].split(); l[0] = f"{k} {int(v) + 1}"; open(p, "w").write("\n".join(l) + "\n")
PYEOF
run c1 o_c1 112 "" HAZYNC_RESUME=1
grep -aq "do not hash to block_108.rcpt's smt_root" o_c1.log && ! done112 o_c1 \
  && pass "4a. resume with a changed state_108.smt: refused (not block_108.rcpt's smt_root)" || { fail "4a. changed state_108.smt"; grep -a -m2 "panicked" o_c1.log; }
anchor c2; rm -rf o_c2; run c2 o_c2 108 ""; rm -f o_c2/state_108.smt
run c2 o_c2 112 "" HAZYNC_RESUME=1
grep -aq "state_108.smt" o_c2.log && ! done112 o_c2 \
  && pass "4b. resume with state_108.smt missing: refused" || { fail "4b. missing state_108.smt"; grep -a -m2 "panicked" o_c2.log; }
[ "$bad" = 0 ] && echo DISK_CONTROLS_OK || { echo "DISK_CONTROLS_FAILED $bad"; exit 1; }
