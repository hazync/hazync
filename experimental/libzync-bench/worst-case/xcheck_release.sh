#!/bin/bash
# xcheck_release.sh <block json> <release host> <txcounts> <out dir>: the SAME block through the published v0.27.1 mainnet
# host (image 46666b6f), as cost-curve/run_b.sh runs a block: agg-exec-fake (chunks executed, collect mode, no hints).
# The block's height is relabelled to 900,000 so every mainnet script flag is active (regtest activates them all too).
# Only the CHUNK cycles are comparable: the mainnet aggregate checks mainnet PoW/retarget, which a regtest block fails.
set -uo pipefail
J=$(readlink -f "$1"); H=$(readlink -f "$2"); T=$(readlink -f "$3"); O=$(readlink -m "$4"); HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$O"; cd "$O" || exit 1
python3 -c "import json,sys; j=json.load(open(sys.argv[1])); j['height']=900000; json.dump(j,open('block_900000.json','w'))" "$J"
HAZYNC_BLOCK=$O/block_900000.json HAZYNC_SEG_PO2=21 HAZYNC_CHUNKS=2 HAZYNC_COINLOG_TXCOUNTS=$T \
  python3 -I "$HERE/timed.py" agg_release.log -- "$H" agg-exec-fake
echo XCHECK_DONE >> agg_release.log
