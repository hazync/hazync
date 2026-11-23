#!/bin/bash
# Run 4 ON the pod (one RTX A6000): prove the aggregate port and the trie variants for block 976,379, each verified.
cd /workspace; log(){ echo "[$(date +%H:%M:%S)] $*"; }
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
apt-get update -qq && apt-get install -y -qq build-essential pkg-config libssl-dev clang curl git bc > /dev/null && log "apt ok"
wget -q https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2204/x86_64/cuda-keyring_1.1-1_all.deb && dpkg -i cuda-keyring_1.1-1_all.deb > /dev/null && apt-get update -qq && apt-get install -y -qq cuda-toolkit-12-9 > /dev/null && log "cuda 12.9 ok"
export PATH=/usr/local/cuda-12.9/bin:$PATH CUDA_HOME=/usr/local/cuda-12.9 CUDA_PATH=/usr/local/cuda-12.9
export CUDA_ARCHS=$(nvidia-smi --query-gpu=compute_cap --format=csv,noheader | head -1 | tr -d .)
curl -sSf https://sh.rustup.rs | sh -s -- -y -q --default-toolchain 1.91.1 > inst_rust.log 2>&1; source ~/.cargo/env
cargo +1.91.1 install --locked --git https://github.com/openvm-org/openvm.git --tag v2.1.0 --features cuda cargo-openvm > inst_openvm.log 2>&1 && log "openvm cli ok" || { log "openvm cli FAILED"; echo RUN4_DONE; exit 1; }
sha256sum run4/*.vmexe
cargo new -q --bin vd && cd vd   # a manifest for the CLI to sit in; --exe skips every build
for c in agg trie; do mkdir -p ../k_$c && cargo openvm keygen --config ../run4/$c.toml --output-dir ../k_$c > ../keygen_$c.log 2>&1; log "keygen $c rc=$?"; done
cargo openvm setup > ../setup_agg.log 2>&1; log "agg setup rc=$?"
( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -l 2 > ../smi.log ) & SMI=$!
pa() { # pa <tag> <exe> <config> <input>
  local s e rc v; s=$(date +%s.%N)
  cargo openvm prove app --exe ../run4/$2.vmexe --config ../run4/$3.toml --app-pk ../k_$3/app.pk --input ../run4/$4 --proof ../$1.app.proof > ../prove_$1.log 2>&1; rc=$?; e=$(date +%s.%N)
  cargo openvm verify app --app-vk ../k_$3/app.vk --proof ../$1.app.proof > ../verify_$1.log 2>&1; v=$?
  echo "RESULT tag=$1 action=app rc=$rc verify_rc=$v secs=$(echo "$e - $s" | bc)"
}
pst() { # pst <tag> <exe> <config> <input>
  local s e rc v; s=$(date +%s.%N)
  cargo openvm prove stark --exe ../run4/$2.vmexe --config ../run4/$3.toml --app-pk ../k_$3/app.pk --agg-prefix-pk ../k_$3/agg_prefix.pk --input ../run4/$4 --proof ../$1.stark.proof > ../prove_$1_stark.log 2>&1; rc=$?; e=$(date +%s.%N)
  cargo openvm verify stark --proof ../$1.stark.proof > ../verify_$1_stark.log 2>&1; v=$?
  echo "RESULT tag=$1 action=stark rc=$rc verify_rc=$v secs=$(echo "$e - $s" | bc) bytes=$(stat -c %s ../$1.stark.proof 2>/dev/null)"
}
pa agg       agg       agg  agg_in.json
pa trie_same trie_same trie trie_in.json
pa trie_nat  trie_nat  trie trie_nat_in.json
pa trie_b0   trie_same trie trie_b0_in.json
pst agg       agg       agg  agg_in.json
pst trie_same trie_same trie trie_in.json
kill $SMI; awk -F', ' '{gsub(/ %/,"",$1); s+=$1; n++} END {print "gpu util mean", s/n}' ../smi.log
echo RUN4_DONE
