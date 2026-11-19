#!/bin/bash
# check.sh [bitcoind]: the CI path. Replays the committed chain (chain/raw.txt) into a fresh Bitcoin Core 28.0 regtest
# node with export.py and requires every block file and oracle snapshot it writes to equal the committed ones, byte for
# byte. Needs only a bitcoind and Python 3 (no test_framework, no Rust). With no argument it uses BITCOIND, or fetches
# the sha256-pinned Core 28.0 release tarball (the one adversarial.yml's libzync-rig job pins) into a cache dir.
# Prints ZAPP_REGTEST_CHECK_OK, or ZAPP_REGTEST_CHECK_FAIL and exits 1.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
BD=${1:-${BITCOIND:-}}
if [ -z "$BD" ]; then
  C=${ZAPP_REGTEST_CACHE:-$HOME/.cache/zapp-regtest}; mkdir -p "$C"
  T=bitcoin-28.0-x86_64-linux-gnu.tar.gz
  [ -f "$C/$T" ] || curl -fsSL -o "$C/$T" "https://bitcoincore.org/bin/bitcoin-core-28.0/$T"
  echo "7fe294b02b25b51acb8e8e0a0eb5af6bbafa7cd0c5b0e5fcbb61263104a82fbc  $C/$T" | sha256sum -c --quiet
  [ -x "$C/bitcoin-28.0/bin/bitcoind" ] || tar xzf "$C/$T" -C "$C"
  BD=$C/bitcoin-28.0/bin/bitcoind
fi
"$BD" -version | head -1
(cd "$HERE" && sha256sum -c --quiet SHA256SUMS) || { echo "ZAPP_REGTEST_CHECK_FAIL: committed files != SHA256SUMS"; exit 1; }
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
BITCOIND=$BD python3 "$HERE/export.py" "$HERE/chain" "$W" | tail -1
chmod 644 "$W"/oracle/*.dat
if diff -r "$HERE/blocks" "$W/blocks" > "$W/d" && diff -r "$HERE/oracle" "$W/oracle" >> "$W/d"; then
  echo "ZAPP_REGTEST_CHECK_OK $(find "$HERE"/blocks -mindepth 2 -maxdepth 2 -name 'block_*' | wc -l) block files and $(find "$HERE"/oracle -mindepth 1 -maxdepth 1 ! -name '.*' | wc -l) oracle files reproduced by Core"
else
  echo "ZAPP_REGTEST_CHECK_FAIL: the replay differs from the committed fixtures"; head -20 "$W/d"; exit 1
fi
