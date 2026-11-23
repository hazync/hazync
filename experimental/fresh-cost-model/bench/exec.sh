#!/bin/bash
source ~/.cargo/env
export PATH="$HOME/.risc0/bin:$HOME/.sp1/bin:$PATH"
cd /root/fresh-bench
BIN=/root/fresh-bench/b976379.bin
for m in 0 1 2 3; do
  python3 -c "import sys; d=open('$BIN','rb').read(); open('ovm_in_$m.json','w').write('{\"input\":[\"0x01'+bytes([$m]).hex()+d.hex()+'\"]}')"
done
for m in 0 1 2 3; do
  RISC0_INFO=1 ./risc0/target/release/bench-risc0 $m $BIN exec 2>risc0_exec_$m.err | grep RESULT
  ./sp1/script/target/release/bench-sp1 $m $BIN exec 2>sp1_exec_$m.err | grep RESULT
  ( cd openvm && /usr/bin/time -f "secs=%e" cargo openvm run --mode meter --input ../ovm_in_$m.json > ../ovm_exec_$m.log 2>&1; echo "RESULT vm=openvm mode=$m action=exec $(grep -i -E 'instructions|cost|cells|secs=' ../ovm_exec_$m.log | tr '\n' ' ' | sed 's/\x1b\[[0-9;]*m//g')" )
done
echo ALL_DONE
