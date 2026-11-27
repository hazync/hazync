#!/bin/bash
# HANDOVER step 3 negative controls (v2.0.2 -> v2.1.0): each predecessor input below must be REFUSED by the guest's ovm-xver check
# handover block's collect pass (execute only: the run stops there, no proof is made).
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
E=/root/engine-kit/engine/host/target/release/hzengine; HX=/root/engine-src-xv3/prover/target/release/host
P=/root/engine/xv3pred; N=/root/engine/xv3neg; rm -rf "${N:?}"; mkdir -p $N; cd /root/engine || exit 1
case_() { # <name> <dir with block_199.ovm.{journal,bin} + state_199.{coinlog,trie}>
  local d=$2; rm -rf "${d:?}/out"
  HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=/root/engine/keys-xv3 HAZYNC_HANDOVER_PROOF=$d/block_199.ovm \
    HAZYNC_COINLOG_TXCOUNTS=/root/engine-kit/experimental/libzync-bench/cost/txcounts.txt \
    timeout 3600 nice $HX libzync-prove-range 200 200 /root/engine/xchain/blocks handover $d/out > $d/run.log 2>&1; local rc=$?
  local why=$(sed 's/\x1b\[[0-9;]*m//g' $d/run.log $d/out/engine/s0/*.log 2>/dev/null | grep -a "predecessor\|panicked at\|xver" | grep -av "^\s*$" | head -2 | tr '\n' ' ' | cut -c1-230)
  [ $rc != 0 ] && echo "refused  $1 (rc $rc): $why" || echo "ACCEPTED $1"
}
mk() { local d=$N/$1; mkdir -p $d; cp $P/block_199.ovm.journal $P/block_199.ovm.bin $P/state_199.coinlog $P/state_199.trie $d/; echo $d; }
# 1. one byte of the old proof flipped
d=$(mk flipped_proof); python3 -c "import sys;b=bytearray(open(sys.argv[1],'rb').read());b[len(b)//2]^=1;open(sys.argv[1],'wb').write(b)" $d/block_199.ovm.bin
case_ "the predecessor proof with one byte flipped" $d
# 2. the journal's cum_work changed (the host does not check it; the proof's public values bind it)
d=$(mk journal_cumwork); python3 - $d/block_199.ovm.journal <<'P'
import sys, struct
p=sys.argv[1]; b=bytearray(open(p,"rb").read())
# ChainState serde words: kind, version, tip_hash[32 words], utxo_roots (Vec<Option<[u8;32]>>: len, then per entry 0|1 [+32]), utxo_leaves u64, utxo_value u64, cum_work[32 words]
w=list(struct.unpack("<%dI"%(len(b)//4), b)); i=2+32; n=w[i]; i+=1
for _ in range(n): i += 1 + (32 if w[i] == 1 else 0)
i+=4   # utxo_leaves, utxo_value
w[i+31]^=1   # the last byte of cum_work
open(p,"wb").write(struct.pack("<%dI"%len(w), *w))
P
case_ "the predecessor's journal with cum_work changed" $d
# 3. the SAME block's proof by the v2.1.0 engine (keys-ch's proof of 199): another OpenVM version's encoding
d=$(mk other_version); $E wasm-inputs --dir keys-ch --proof /root/engine/core200/block_199.ovm --out $d/wi > /dev/null 2>&1 && cp $d/wi/proof.bin $d/block_199.ovm.bin
case_ "keys-ch's v2.1.0 proof of 199 where the v2.0.2 one belongs" $d
echo XV3_NEG_DONE
