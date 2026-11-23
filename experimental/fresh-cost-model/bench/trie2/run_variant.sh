#!/bin/bash
# run_variant.sh <name> <features (comma list or "")> [input json]: build ovm-trie with those features into target_<name>, meter.
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
N=$1; F=$2; IN=${3:-/root/fresh-bench/trie_in.json}
cd /root/fresh-bench/trie2/ovm-trie
FA=""; [ -n "$F" ] && FA="--features $F"
cargo openvm build $FA --target-dir target_$N > ../build_$N.log 2>&1 || { echo "VARIANT $N BUILD_FAILED"; sed "s/\x1b\[[0-9;]*m//g" ../build_$N.log | grep -a -E "^openvm build: error|error\[" -A4 | head -20; exit 1; }
cargo openvm run $FA --target-dir target_$N --mode meter --input $IN > ../run_$N.log 2>&1; rc=$?
O=$(sed "s/\x1b\[[0-9;]*m//g" ../run_$N.log | grep -a -o -E "Execution output: \[[0-9, ]*\]" | sed "s/Execution output: //")
I=$(sed "s/\x1b\[[0-9;]*m//g" ../run_$N.log | grep -a -o -E "instructions executed: [0-9]+" | grep -o "[0-9]*$")
C=$(sed "s/\x1b\[[0-9;]*m//g" ../run_$N.log | grep -a -o -E "Total cost: [0-9]+" | grep -o "[0-9]*$")
H=$(python3 -c "o=$O; print(bytes(o[:28]).hex(), int.from_bytes(bytes(o[28:]),\"little\"))" 2>/dev/null)
echo "VARIANT $N rc=$rc instr=$I cost=$C out=$H"
