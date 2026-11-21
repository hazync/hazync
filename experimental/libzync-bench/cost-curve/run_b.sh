#!/bin/bash
# run_b.sh <host> <blocks dir> <out dir> [height ...]
# (B) BLOCK WORK REAL, STATE MODELLED, as ../profiles/run.sh does for 962,000 (same commands; only the chunking is the
# shipped one, as in run_a.sh): the block's signature/ECMH hints (`libzync-hints`), then `agg-exec-fake`, which
# executes the chunks (the real block: transactions, prevouts, signatures), a coin-log collect run (host-side, never
# proved), the trie update against a SYNTHESISED old trie (trie_model::synth) and the aggregate against a SYNTHESISED
# coin log (lz::cl_synth: size k=1.64 x cumulative transactions from ../cost/txcounts.txt). Trie and aggregate cycles
# are therefore MODELLED. HAZYNC_CHUNKS unset (host default 2, cost-packed); segments at po2 21 as in run_a.sh.
# Block files: fetch_block_rpc.py <h> <dir>/block_<h>.json against a txindex'd archive node.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
H=$(readlink -f "${1:?host}"); B=$(readlink -f "${2:?blocks dir}"); O=$(readlink -m "${3:?out}"); shift 3
HS=${*:-100000 150000 200000 250000 300000 350000 400000 450000 500000 550000 600000 650000 700000 750000 800000 850000 900000 950000 962000 976000}
export HAZYNC_SEG_PO2=21 HAZYNC_COINLOG_TXCOUNTS=$HERE/../cost/txcounts.txt
unset HAZYNC_CHUNKS HAZYNC_PROFILE_LAYOUT HAZYNC_LZ_HINTS RISC0_DEV_MODE HAZYNC_HINT_THREADS RISC0_PPROF_OUT
mkdir -p "$O"; cd "$O" || exit 1
for h in $HS; do
  rm -f "hints_$h.log" "agg_$h.log" "b$h.lzb"
  HAZYNC_BLOCK=$B/block_$h.json python3 -I "$HERE/timed.py" "hints_$h.log" -- nice -n 10 "$H" libzync-hints "b$h.lzb"
  HAZYNC_BLOCK=$B/block_$h.json HAZYNC_LZ_HINTS=$O/b$h.lzb python3 -I "$HERE/timed.py" "agg_$h.log" -- \
    nice -n 10 "$H" agg-exec-fake
  echo "== $h"; grep -a 'executed: \|padded\|validation cycles\|journal sha256\|TIMED' "agg_$h.log"
done
echo RUN_B_DONE
