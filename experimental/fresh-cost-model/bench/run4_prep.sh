#!/bin/bash
# Run 4 prep (build box): the executables to prove on one GPU, each rebuilt under its own name and re-metered so the
# bundle carries exactly what was measured.
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
R=/root/fresh-bench/run4; rm -rf $R; mkdir -p $R
meter() { cargo openvm run --exe $1 --config $2 --mode meter --input $3 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -a -E "Execution output|Total cost" | tr "\n" " "; echo; }
# aggregate port (already built by agg_build.sh)
A=/root/fresh-bench/hzhost/prover/methods/ovmagg
cp $A/openvm/release/ovmagg.vmexe $R/agg.vmexe; cp $A/openvm.toml $R/agg.toml; cp /root/fresh-bench/agg_in.json $R/
(cd $A && echo "agg: $(meter $R/agg.vmexe $R/agg.toml $R/agg_in.json)")
# trie variants (trie2's guest; minimal config)
T=/root/fresh-bench/trie2/ovm-trie; cd $T
for v in "same:ovm_iv,zerocopy,ovm_sha" "nat:ovm_nat,zerocopy,ovm_sha"; do
  n=${v%%:*}; f=${v#*:}
  cargo openvm build --features $f --target-dir target_r4_$n > $R/build_$n.log 2>&1 || { echo "BUILD $n FAILED"; continue; }
  cp openvm/release/ovm-trie.vmexe $R/trie_$n.vmexe
done
cp openvm.toml $R/trie.toml; cp /root/fresh-bench/trie_in.json $R/trie_in.json
cp /root/fresh-bench/trie2/trie_nat_in.json /root/fresh-bench/trie2/trie_b0_in.json $R/
echo "trie_same: $(meter $R/trie_same.vmexe $R/trie.toml $R/trie_in.json)"
echo "trie_nat:  $(meter $R/trie_nat.vmexe $R/trie.toml $R/trie_nat_in.json)"
echo "trie_b0:   $(meter $R/trie_same.vmexe $R/trie.toml $R/trie_b0_in.json)"
sha256sum $R/*.vmexe; du -sh $R
echo PREP4_DONE
