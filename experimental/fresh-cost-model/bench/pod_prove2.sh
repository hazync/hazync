#!/bin/bash
# Run 2 ON the pod: sighash hashing (mode 4) on a tip block and a legacy-heavy block, then wrapping to ONE proof.
source ~/.cargo/env
export PATH=/usr/local/cuda-12.9/bin:$HOME/.sp1/bin:$HOME/.cargo/bin:$PATH
cd /workspace
for f in b*.bin.zst; do [ -f ${f%.zst} ] || zstd -q -d $f; done
nvidia-smi --query-gpu=name --format=csv,noheader
mkjson(){ python3 -c "import sys; d=open('$1','rb').read(); open('$3','w').write('{\"input\":[\"0x01'+bytes([$2]).hex()+d.hex()+'\"]}')"; }
ovm(){ # $1 = stark|app  $2 = json  $3 = tag
  ( cd openvm && s=$(date +%s.%N); cargo openvm prove $1 --input ../$2 > ../ovm_$3.log 2>&1; rc=$?; e=$(date +%s.%N)
    cargo openvm verify $1 > ../ovm_verify_$3.log 2>&1; vrc=$?
    echo "RESULT vm=openvm tag=$3 action=$1 rc=$rc verify_rc=$vrc secs=$(echo "$e - $s" | bc)" ); }
sp1(){ # $1 mode $2 bin $3 action $4 tag
  SP1_PROVER=cuda ./sp1/script/target/release/bench-sp1 $1 $2 $3 > sp1_$4.log 2>&1; grep RESULT sp1_$4.log | sed "s/^RESULT/RESULT tag=$4/" || echo "RESULT vm=sp1 tag=$4 FAILED"; }
for B in 976379 510500; do
  mkjson b$B.bin 4 in_${B}_4.json
  sp1 4 b$B.bin prove sighash_$B
  ovm app in_${B}_4.json sighash_$B
done
mkjson b976379.bin 1 in_976379_1.json
sp1 1 b976379.bin compress ecdsa_wrap_976379
ovm stark in_976379_1.json ecdsa_wrap_976379
sp1 4 b510500.bin compress sighash_wrap_510500
ovm stark in_510500_4.json sighash_wrap_510500
echo PROVE_DONE
