#!/usr/bin/env bash
# test_split_cpu.sh [build dir] [-- harness args...]: build the CPU equivalence test and run it.
# Compiles the stock C++ twin (kernels/cxx/rust_poly_fp_[0-3].cpp) and the split C++ that gen_split.py emits
# from those same files, then checks split == stock bit for bit (random inputs by default).
#   split/test_split_cpu.sh                         # random, 2^14 rows, seeds 1 and 2
#   split/test_split_cpu.sh /tmp/b -- dump <dir>    # a real eval_check input dumped by the CUDA host
# Env: CXX (default g++), OPT (default -O1; the generated files are ~4 MB each), CUTS (default the shipped ones).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
CRATE=$(dirname "$HERE")
B=${1:-${TMPDIR:-/tmp}/eval-split-cpu}; shift || true
[ "${1:-}" = "--" ] && shift
mkdir -p "$B"
CXX=${CXX:-g++}; OPT=${OPT:-"-O1"}
R0SYS=$(ls -d ~/.cargo/registry/src/*/risc0-sys-1.5.1 2>/dev/null | head -1)
[ -n "$R0SYS" ] || { echo "risc0-sys-1.5.1 not in ~/.cargo/registry (cargo fetch in prover/)"; exit 2; }
INC="-I$R0SYS/cxx -I$CRATE/kernels/cxx"
python3 "$HERE/gen_split.py" cxx "$CRATE/kernels/cxx" "$B/split_poly_fp.cpp" ${CUTS:+--cuts "$CUTS"}
objs=()
for i in 0 1 2 3; do
  o="$B/stock_$i.o"; s="$CRATE/kernels/cxx/rust_poly_fp_$i.cpp"
  if [ ! -s "$o" ] || [ "$s" -nt "$o" ]; then $CXX $OPT -std=c++17 -fopenmp $INC -c "$s" -o "$o" & fi
  objs+=("$o")
done
$CXX $OPT -std=c++17 -fopenmp $INC -c "$B/split_poly_fp.cpp" -o "$B/split.o" &
wait
$CXX -O2 -std=c++17 -fopenmp $INC "$HERE/test_split_cpu.cpp" "${objs[@]}" "$B/split.o" -o "$B/test_split_cpu"
if [ $# -gt 0 ]; then
  "$B/test_split_cpu" "$@"
else
  "$B/test_split_cpu" random 14 1
  "$B/test_split_cpu" random 14 2
fi
