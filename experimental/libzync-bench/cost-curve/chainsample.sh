#!/bin/bash
# chainsample.sh <step> <tip>: block content of one block per <step> (heights step/2, step/2+step, ...), for the
# whole-chain projection's input/output counts: height, txs, ins, outs, total_weight (getblockstats, read-only RPC).
S=${1:?step}; T=${2:?tip}
echo -e "height\ttxs\tins\touts\ttotal_weight"
for h in $(seq $((S / 2)) "$S" "$T"); do
  bitcoin-cli getblockstats "$h" '["height","txs","ins","outs","total_weight"]' |
    python3 -I -c 'import json,sys; d=json.load(sys.stdin); print("\t".join(str(d[k]) for k in ["height","txs","ins","outs","total_weight"]))'
done
