#!/bin/bash
# verify_all.sh <gpu out dir> <inputs dir> <hazync-verify> <host>: every receipt of the GPU run, on a glibc 2.39 box.
#  A leaves: the v0.27.0 release verifier (hazync-verify-x86_64-linux-gnu --network mainnet --product) with the leaf's
#    anchor (anchor_effective.json = the bundle's S(h-1)) and --expect h:<hash from the local archive node>; the
#    anchor must equal prep's state_<h-1>.json and the committed state (state_<h>.json from the receipt) prep's state_<h>.
#  B blocks: the same verifier on block_<h>.receipt (no anchor file exists: agg-chunks synthesised it).
HERE=$(cd "$(dirname "$0")/.." && pwd); IN=$(readlink -f "${2:?}"); V=$(readlink -f "${3:?}"); cd "${1:?}" || exit 1
for h in 170 1000 100000 200000 270000 280000 290000 300000 400000; do
  E="$h:$(bitcoin-cli getblockhash $h)"
  $V --network mainnet --product --anchor o$h/anchor_effective.json --expect "$E" o$h/block_$h.rcpt > verify_$h.log 2>&1; rc=$?
  a=$(python3 -I "$HERE/cmp_state.py" o$h/anchor_effective.json "$IN/out/state_$((h-1)).json")
  s=$(python3 -I "$HERE/cmp_state.py" o$h/state_$h.json "$IN/out/state_$h.json")
  echo -e "A $h\trc=$rc\t$(grep -ao 'SWIFTSYNC_VERIFY_[A-Z]*' verify_$h.log | tail -1)\tanchor: ${a%% *}\tstate: ${s%% *}\treceipt $(stat -c %s o$h/block_$h.rcpt) B"
done
for h in 700000 976000; do
  E="$h:$(bitcoin-cli getblockhash $h)"
  $V --network mainnet --product --expect "$E" b$h/block_$h.receipt > verify_b$h.log 2>&1; rc=$?
  echo -e "B $h\trc=$rc\t$(grep -ao 'SWIFTSYNC_VERIFY_[A-Z]*.\{0,90\}' verify_b$h.log | tail -1)\treceipt $(stat -c %s b$h/block_$h.receipt) B; chunks $(stat -c %s b$h/receipts/chunk_*.hzk | tr '\n' ' ')"
done
