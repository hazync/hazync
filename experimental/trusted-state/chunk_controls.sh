#!/bin/bash
# chunk_controls.sh <ts-anchor> <ts-ingest-host> <work dir> -- EXPERIMENTAL negative controls of the chunked ingest
# (docs/libzync/TRUSTED_STATE.md §12.3) on the regtest snapshot at 104 (169 coins): plan A cuts it into 3 chunks, plan B
# into 4, every chunk is executed honestly (dev mode), then the combine is fed wrong sets. Each must be refused, and the
# honest set accepted. Prints CONTROLS_OK or CONTROLS_FAILED <n>.
set -uo pipefail
TS=$1; IH=$2; W=$3
Z=$(cd "$(dirname "$0")/../.." && pwd); G=$Z/scripts/zapp-regtest
mkdir -p "$W"
export RISC0_DEV_MODE=1
python3 "$Z/experimental/trusted-state/side_inputs.py" "$G/blocks/main" 104 "$W/side" > /dev/null
for p in A:3 B:4; do
  d=$W/${p%%:*}; rm -rf "$d"
  env -u RISC0_DEV_MODE "$TS" chunk-plan --network regtest --snapshot "$G/oracle/utxo_104.dat" --side "$W/side" --chunks "${p#*:}" --out "$d" | head -1
  for f in "$d"/chunk_*.in; do "$IH" chunk "$f" "${f%.in}.rcpt" > "${f%.in}.log" 2>&1 || { echo "chunk $f failed"; exit 1; }; done
done
# a chunk whose header times were changed (one timestamp +1): executes fine on its own
python3 - "$W/A/chunk_1.in" "$W/A/chunk_1t.in" <<'P'
import sys
b = bytearray(open(sys.argv[1], "rb").read()); i = 0; secs = []
while i < len(b):
    n = int.from_bytes(b[i:i + 4], "little"); secs.append((i + 4, n)); i += 4 + n
off, n = secs[1]                       # section 2: the header times, u32 LE each
t = int.from_bytes(b[off + 40:off + 44], "little") + 1
b[off + 40:off + 44] = t.to_bytes(4, "little")
open(sys.argv[2], "wb").write(b)
P
"$IH" chunk "$W/A/chunk_1t.in" "$W/A/chunk_1t.rcpt" > /dev/null 2>&1 || { echo "the times-changed chunk did not execute"; exit 1; }
# a final state with one satoshi more
python3 - "$W/A/final.in" "$W/A/final_v.in" <<'P'
import sys
b = bytearray(open(sys.argv[1], "rb").read())
v = 4 + 4 + 4 + 32 + 8 + 8              # section length, started, magic, base, coins_total, idx -> value
b[v:v + 8] = (int.from_bytes(b[v:v + 8], "little") + 1).to_bytes(8, "little")
open(sys.argv[2], "wb").write(b)
P
bad=0
run() {  # run <want: ok|refused> <label> <final.in> <receipts...>
  local want=$1 label=$2 fin=$3; shift 3
  local o; o=$("$IH" combine "$fin" - "$@" 2>&1); local got=refused
  echo "$o" | grep -q "^COMBINE image" && got=ok
  local why; why=$(echo "$o" | grep -a -o "panicked.*\|Guest panicked: .*" | tail -1 | sed 's/.*Guest panicked: //')
  if [ "$got" = "$want" ]; then echo "PASS $label: $got ${why:+($why)}"; else echo "FAIL $label: $got, wanted $want ${why:+($why)}"; bad=$((bad+1)); fi
}
A=$W/A; B=$W/B
run ok      "honest A (3 chunks)"                          "$A/final.in" "$A/chunk_0.rcpt" "$A/chunk_1.rcpt" "$A/chunk_2.rcpt"
run ok      "honest B (4 chunks)"                          "$B/final.in" "$B"/chunk_{0,1,2,3}.rcpt
run refused "overlap: A0 (coins 0..56) + B1.. (43..)"      "$A/final.in" "$A/chunk_0.rcpt" "$B/chunk_1.rcpt" "$B/chunk_2.rcpt" "$B/chunk_3.rcpt"
run refused "dropped middle chunk: A0 A2"                  "$A/final.in" "$A/chunk_0.rcpt" "$A/chunk_2.rcpt"
run refused "dropped last chunk: A0 A1"                    "$A/final.in" "$A/chunk_0.rcpt" "$A/chunk_1.rcpt"
run refused "reordered: A1 A0 A2"                          "$A/final.in" "$A/chunk_1.rcpt" "$A/chunk_0.rcpt" "$A/chunk_2.rcpt"
run refused "repeated: A0 A1 A1 A2"                        "$A/final.in" "$A/chunk_0.rcpt" "$A/chunk_1.rcpt" "$A/chunk_1.rcpt" "$A/chunk_2.rcpt"
run refused "a chunk with other header times"             "$A/final.in" "$A/chunk_0.rcpt" "$A/chunk_1t.rcpt" "$A/chunk_2.rcpt"
run refused "final state +1 sat"                           "$A/final_v.in" "$A/chunk_0.rcpt" "$A/chunk_1.rcpt" "$A/chunk_2.rcpt"
[ $bad = 0 ] && echo CONTROLS_OK || { echo "CONTROLS_FAILED $bad"; exit 1; }
