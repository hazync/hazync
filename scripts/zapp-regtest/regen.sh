#!/bin/bash
# regen.sh: regenerate the committed chain and fixtures from nothing, TWICE, and refuse unless the two runs are
# byte-identical (the determinism proof). Then install them into this directory (chain/, blocks/, oracle/).
# Needs Bitcoin Core 28.0 (BITCOIND) and a Core v28.0 source tree (HZ_CORE_SRC, for test_framework). Not CI: CI replays
# the committed chain with check.sh. Run it only when the chain itself must change; every receipt proved on the old
# chain (README.md "GPU batch") is then void.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
for r in 1 2; do
  python3 "$HERE/gen_chain.py" "$W/g$r" | tail -1
  python3 "$HERE/export.py" "$W/g$r" "$W/e$r" | tail -1
done
cmp "$W/g1/raw.txt" "$W/g2/raw.txt" && cmp "$W/g1/scenarios.json" "$W/g2/scenarios.json" \
  || { echo "REGEN_NOT_DETERMINISTIC: the two generator runs differ"; exit 1; }
diff -r "$W/e1" "$W/e2" > /dev/null || { echo "REGEN_NOT_DETERMINISTIC: the two exports differ"; diff -r "$W/e1" "$W/e2" | head; exit 1; }
rm -rf "$HERE/chain" "$HERE/blocks" "$HERE/oracle"
mkdir -p "$HERE/chain"
cp "$W/g1/raw.txt" "$W/g1/scenarios.json" "$HERE/chain/"
cp -r "$W/e1/blocks" "$W/e1/oracle" "$HERE/"
chmod 644 "$HERE"/oracle/*.dat
(cd "$HERE" && sha256sum chain/* blocks/*/* oracle/* > SHA256SUMS)
echo "REGEN_OK two runs identical; raw.txt $(sha256sum "$HERE/chain/raw.txt" | cut -c1-16), b 113 $(tail -1 "$HERE/chain/raw.txt" | cut -d' ' -f3)"
