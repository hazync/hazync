#!/usr/bin/env bash
# Check a genesis-anchored Hazync proof, and the coin list our bridge exports at the same height,
# against an unmodified Bitcoin Core at that height. Everything this needs is public.
#
#   tools/utxo-evidence/run.sh <proof.bin> [height]
#
# The proof must END at <height> (default 136432, the spine as of 2026-10-14). It is a FILE, not the
# /api/spine/proof URL: that endpoint moves forward as proving resumes, and a check is only
# repeatable against fixed bytes.
#
# What it does, all in $WORK (default ./utxo-evidence-work), with nothing installed system-wide:
#   1. downloads Bitcoin Core $CORE_VER and the Hazync $HZ_TAG host + verifier, and checks each
#      against its published SHA256SUMS before running it;
#   2. syncs a mainnet Core node just past <height> (-stopatheight; it never downloads the blocks
#      after it), restarts it offline and rolls it back to exactly <height>;
#   3. records Core's own UTXO-set summary there (gettxoutsetinfo muhash) and its block header;
#   4. walks our bridge from genesis to <height> on CPU against that node and exports HZUTXO v1;
#   5. checks the export against the proof with verifier-ffi (tools/utxo-evidence/dumpcheck);
#   6. compares proof, export and Core: tip, chain work, coin count, total amount, MuHash.
#
# ⚠ Build dumpcheck from the SAME tag as $HZ_TAG, so the library judging the export is the one that
# shipped. The final line is RESULT=0 only if every comparison PASSED.
#
# 📏 2026-10-14, laptop (16 threads, 8 GB): node sync to 136,440 ≈ 15 min, bridge walk 777 s.
set -Eeuo pipefail
trap 'echo "ERR: line $LINENO: $BASH_COMMAND exited $?" >&2' ERR

PROOF=$(realpath "${1:?usage: run.sh <proof.bin> [height]}")
H=${2:-136432}
case "$H" in ''|*[!0-9]*) echo "height must be a number, got '$H'" >&2; exit 2 ;; esac
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(realpath -m "${WORK:-./utxo-evidence-work}")
CORE_VER=${CORE_VER:-31.1}
HZ_TAG=${HZ_TAG:-v0.22.1}
DUMPCHECK=${DUMPCHECK:-$HERE/dumpcheck/target/release/dumpcheck}
[ -x "$DUMPCHECK" ] || { echo "build dumpcheck first: cargo build --release --manifest-path $HERE/dumpcheck/Cargo.toml" >&2; exit 2; }
mkdir -p "$WORK"/{core/data,hz,out,bridge}

echo "== 1. fetch and hash-check the binaries"
CT=bitcoin-$CORE_VER-x86_64-linux-gnu.tar.gz
(cd "$WORK/core"
 curl -sSfLO "https://bitcoincore.org/bin/bitcoin-core-$CORE_VER/$CT"
 curl -sSfLO "https://bitcoincore.org/bin/bitcoin-core-$CORE_VER/SHA256SUMS"
 grep " $CT\$" SHA256SUMS | sha256sum -c -
 tar xzf "$CT")
(cd "$WORK/hz"
 for a in hazync-host-x86_64-linux-gnu hazync-verify-x86_64-linux-gnu SHA256SUMS.txt; do
     curl -sSfLO "https://github.com/hazync/hazync/releases/download/$HZ_TAG/$a"
 done
 grep -E ' hazync-(host|verify)-x86_64-linux-gnu$' SHA256SUMS.txt | sha256sum -c -
 chmod +x hazync-host-x86_64-linux-gnu hazync-verify-x86_64-linux-gnu)
BIN=$WORK/core/bitcoin-$CORE_VER/bin
DD=$WORK/core/data
C="$BIN/bitcoin-cli -datadir=$DD"
export PATH="$BIN:$PATH"            # the bridge shells out to `bitcoin-cli`

echo "== 2. verify the proof and read the state it commits"
"$WORK/hz/hazync-verify-x86_64-linux-gnu" --json "$PROOF" > "$WORK/out/verify.json"
PH=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["height"])' "$WORK/out/verify.json")
[ "$PH" = "$H" ] || { echo "the proof ends at $PH, not $H" >&2; exit 1; }

echo "== 3. sync a reference node to just past $H, then hold it offline at exactly $H"
cat > "$DD/bitcoin.conf" <<EOF
server=1
listen=0
blocksonly=1
disablewallet=1
dbcache=2000
rpcbind=127.0.0.1
rpcallowip=127.0.0.1
rpcport=18432
EOF
"$BIN/bitcoind" -datadir="$DD" -stopatheight=$((H + 8)) -daemon
until $C getblockcount >/dev/null 2>&1; do sleep 2; done
# -stopatheight shuts the node down by itself; wait for that rather than for a height.
while $C getblockcount >/dev/null 2>&1; do sleep 20; done
"$BIN/bitcoind" -datadir="$DD" -connect=0 -stopatheight=0 -daemon
until $C getblockcount >/dev/null 2>&1; do sleep 2; done
if [ "$($C getblockcount)" -gt "$H" ]; then
    $C invalidateblock "$($C getblockhash $((H + 1)))"
fi
[ "$($C getblockcount)" = "$H" ] || { echo "node tip is $($C getblockcount), not $H" >&2; exit 1; }
$C getblockheader "$($C getblockhash "$H")" > "$WORK/out/header.json"
$C gettxoutsetinfo muhash > "$WORK/out/core.json"
cat "$WORK/out/core.json"

echo "== 4. the bridge walks genesis -> $H (CPU only; no bundles written)"
t0=$(date +%s)
HAZYNC_BRIDGE_OUT=$WORK/bridge HAZYNC_BITCOIN_DATADIR=$DD HAZYNC_BRIDGE_TO=$H HAZYNC_BRIDGE_ONCE=1 \
    HAZYNC_BRIDGE_EMIT_FROM=999999999 "$WORK/hz/hazync-host-x86_64-linux-gnu" bridge
echo "bridge walk took $(( $(date +%s) - t0 ))s"
HAZYNC_BRIDGE_OUT=$WORK/bridge "$WORK/hz/hazync-host-x86_64-linux-gnu" dump-snapshot "$WORK/out/coins-$H.hzutxo"
$C stop
sha256sum "$WORK/out/coins-$H.hzutxo"

echo "== 5. the export against the proof (verifier-ffi)"
"$DUMPCHECK" "$PROOF" "$WORK/out/coins-$H.hzutxo"

echo "== 6. proof, export and Bitcoin Core"
set +e
python3 -I "$HERE/compare.py" "$WORK/out/verify.json" "$WORK/out/coins-$H.hzutxo" \
    "$WORK/out/core.json" "$WORK/out/header.json"
RESULT=$?
echo "RESULT=$RESULT"
exit $RESULT
