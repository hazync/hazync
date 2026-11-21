#!/bin/bash
# blockstats.sh <height ...>: block content from the archive node (bitcoin-cli getblockstats), one TSV row per block:
# height, txs (incl. coinbase), ins (non-coinbase inputs), outs, total_weight (WU, excl. header), total_size (bytes),
# swtxs (segwit txs), utxo_increase. Read-only RPC.
echo -e "height\ttxs\tins\touts\ttotal_weight\ttotal_size\tswtxs\tutxo_increase"
for h in "$@"; do
  bitcoin-cli getblockstats "$h" '["height","txs","ins","outs","total_weight","total_size","swtxs","utxo_increase"]' |
    python3 -I -c 'import json,sys; d=json.load(sys.stdin); print("\t".join(str(d[k]) for k in ["height","txs","ins","outs","total_weight","total_size","swtxs","utxo_increase"]))'
done
