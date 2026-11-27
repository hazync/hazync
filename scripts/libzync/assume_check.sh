#!/bin/bash
# assume_check.sh <H> <proof.hze> <zapp-assume binary (zapps/assume)> [<bitcoin-cli of your node>]
# with IMAGE=<the core image id, 64 hex> VK=<its key file> VK_SHA256=<the key file's pin, 64 hex> (engine proofs: no
# image is built in until the Stage 4 freeze; the pin is the published one or your own build's, never the file's own hash)
#
# The assumeutxo and assumevalid checks (docs/libzync/ASSUME_ZAPPS.md) end to end against a REAL Core `dumptxoutset`:
# a throwaway node syncs blocks 0..H from your node over P2P (127.0.0.1:8333), goes offline, dumps its UTXO set at H
# (`dumptxoutset … rollback=H`), and the verifier checks that file against the chain proof at H, with the headers
# 0..H and the BIP30-tracked list derived from the same node. Your node is only a P2P peer and the identity check
# (`--bitcoin-cli`): it is never rolled back (`rollback` on a node at the tip would disconnect every block above H).
#
# Env: IMAGE, VK and VK_SHA256 (required), WORK (default $HOME/assume-check), PORT (your node's P2P port, 8333), RPCPORT (the throwaway node's, 28342),
# KEEP=1 keeps the throwaway node.
# Needs bitcoind / bitcoin-cli >= 28 on PATH (rollback=), python3. Cost: the sync to H (H=200: seconds) plus
# assume_inputs.py (the headers, and a scan of every block 1..H for the BIP30-tracked list).
set -euo pipefail
H=$1; RCPT=$(realpath "$2"); V=$(realpath "$3"); MAIN=${4:-bitcoin-cli}
IMAGE=${IMAGE:?IMAGE=<core image id> is required}; VK=$(realpath "${VK:?VK=<key file> is required}")
VK_SHA256=${VK_SHA256:?VK_SHA256=<the pin of the key file> is required}
HERE=$(dirname "$(realpath "$0")")
WORK=${WORK:-$HOME/assume-check}; PORT=${PORT:-8333}; RPCPORT=${RPCPORT:-28342}
D=$WORK/node; mkdir -p "$D"
C="bitcoin-cli -datadir=$D -rpcport=$RPCPORT"
start() { bitcoind -datadir="$D" -daemon -listen=0 -port=$((RPCPORT + 1)) -rpcport=$RPCPORT -rpcbind=127.0.0.1 -rpcallowip=127.0.0.1 \
            -dbcache=1000 -blocksonly=1 -printtoconsole=0 "$@" >/dev/null; $C -rpcwait getblockcount >/dev/null; }
stop() { $C stop >/dev/null 2>&1 || true; while pgrep -f -- "-datadir=$D " >/dev/null; do sleep 1; done; }
cleanup() { stop; [ "${KEEP:-0}" = 1 ] || rm -rf "$D"; }
trap cleanup EXIT

t0=$(date +%s)
if [ "$($C getblockcount 2>/dev/null || echo -1)" = -1 ]; then
  # sync to at least H (stopatheight shuts the node down a few blocks past it)
  bitcoind -datadir="$D" -daemon -connect=127.0.0.1:$PORT -listen=0 -port=$((RPCPORT + 1)) -rpcport=$RPCPORT -rpcbind=127.0.0.1 \
    -rpcallowip=127.0.0.1 -dbcache=1000 -blocksonly=1 -printtoconsole=0 -stopatheight=$((H + 1)) >/dev/null
  while pgrep -f -- "-datadir=$D " >/dev/null; do sleep 2; done
fi
start -connect=0   # offline: nothing moves while we read it
have=$($C getblockcount)
[ "$have" -ge "$H" ] || { echo "throwaway node stopped at $have < $H" >&2; exit 1; }
t1=$(date +%s)

rm -f "$WORK/utxo_$H.dat"
$C -rpcclienttimeout=0 -named dumptxoutset "$WORK/utxo_$H.dat" rollback=$H > "$WORK/dump_$H.json"
t2=$(date +%s)
python3 -I "$HERE/assume_inputs.py" "$H" --datadir "$D" --rpcport "$RPCPORT" --headers "$WORK/headers_0_$H.bin" --tracked "$WORK/tracked_$H.txt"
t3=$(date +%s)
B=$($C getblockhash $((H / 2)))
echo "throwaway node: sync ${have} blocks $((t1 - t0)) s; dumptxoutset rollback=$H $((t2 - t1)) s; headers + tracked list $((t3 - t2)) s"
echo "Core: $(tr -d '\n ' < "$WORK/dump_$H.json")"
echo "tracked coins unspent at $H: $(grep -vc '^#' "$WORK/tracked_$H.txt" || true)"

set +e
echo "== assumeutxo"
"$V" assumeutxo --image "$IMAGE" --vk "$VK" --vk-sha256 "$VK_SHA256" --bitcoin-cli "$MAIN" --headers "$WORK/headers_0_$H.bin" --tracked "$WORK/tracked_$H.txt" "$RCPT" "$WORK/utxo_$H.dat"
echo "exit=$?"
echo "== assumevalid, the tip and block $((H / 2))"
"$V" assumevalid --image "$IMAGE" --vk "$VK" --vk-sha256 "$VK_SHA256" --bitcoin-cli "$MAIN" "$RCPT" | grep -a 'assumevalid\|VERIFY_'; echo "exit=${PIPESTATUS[0]}"
"$V" assumevalid --image "$IMAGE" --vk "$VK" --vk-sha256 "$VK_SHA256" --bitcoin-cli "$MAIN" --block "$B" --headers "$WORK/headers_0_$H.bin" "$RCPT" | grep -a 'assumevalid\|VERIFY_'; echo "exit=${PIPESTATUS[0]}"
echo "== independent reference (scripts/libzync/assume_ref.py)"
python3 -I "$HERE/assume_ref.py" "$WORK/utxo_$H.dat" "$WORK/headers_0_$H.bin" "$WORK/tracked_$H.txt"
echo "== refused: the snapshot at $((H - 1)) against the proof at $H"
$C -rpcclienttimeout=0 -named dumptxoutset "$WORK/utxo_$((H - 1)).dat" rollback=$((H - 1)) >/dev/null
"$V" assumeutxo --image "$IMAGE" --vk "$VK" --vk-sha256 "$VK_SHA256" --headers "$WORK/headers_0_$H.bin" "$RCPT" "$WORK/utxo_$((H - 1)).dat" | grep -a 'assumeutxo\|VERIFY_'; echo "exit=${PIPESTATUS[0]} (want 1)"
