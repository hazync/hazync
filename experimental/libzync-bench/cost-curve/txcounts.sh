#!/bin/bash
# txcounts.sh <step> <tip>: cumulative transaction count (coinbases included) at every <step> heights and at <tip>,
# from the archive node (getchaintxstats 1 <hash of h> -> txcount = transactions in blocks 0..h). Read-only RPC.
S=${1:?step}; T=${2:?tip}
for h in $(seq 0 "$S" "$T") "$T"; do
  [ "$h" -eq 0 ] && { echo "0 1"; continue; }
  echo "$h $(bitcoin-cli getchaintxstats 1 "$(bitcoin-cli getblockhash "$h")" | python3 -I -c 'import json,sys;print(json.load(sys.stdin)["txcount"])')"
done | awk '!seen[$1]++'
