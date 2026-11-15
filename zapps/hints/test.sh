#!/bin/bash
# The hints zAPP's tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the library's unit
# tests, the CLI, the C ABI and the WASM module (through its glue) on the real receipts in vectors/ (CPU proofs by the
# published hints image, docs/libzync/HINTS_ZAPP.md §10; H = 3 over the core's blocks 1..3: DEMO, below minimum work).
set -euo pipefail
cd "$(dirname "$0")"
T=${CARGO_TARGET_DIR:-$PWD/target}
FT=${CARGO_TARGET_DIR:-$PWD/ffi/target}
H3=0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449
V=$PWD/vectors
cargo test --release --manifest-path ./Cargo.toml
cargo test --release --manifest-path ./wasm/Cargo.toml
cargo build --release --manifest-path ./Cargo.toml --bin zapp-hints
set +e; "$T/release/zapp-hints" --expect 3:$H3 --undo $V/mainnet-3.undo $V/mainnet-3.receipt $V/mainnet-3.hints; e=$?; set -e
[ $e = 2 ] || { echo "::error::the proven hints file: exit $e, want 2 (DEMO)"; exit 1; }
tiny=$(mktemp); trap 'rm -f "$tiny"' EXIT
printf 'UTXO\0\377\377\377\377' > "$tiny"
set +e; "$T/release/zapp-hints" --expect 3:$H3 $V/mainnet-3.receipt "$tiny"; e=$?; set -e
[ $e = 1 ] || { echo "::error::a 9-byte file claiming H = 2^32 - 1: exit $e, want 1"; exit 1; }
# the chain identity through the other sources (zapp-common, every zAPP CLI's flags): a node that names block 3 as the
# proof does (DEMO: valid, below minimum work), one that names another block (FAIL)
printf '#!/bin/sh\n[ "$1" = getblockhash ] && echo %s\nexit 0\n' $H3 > "$T/node"; chmod +x "$T/node"
printf '#!/bin/sh\necho %064d\n' 1 > "$T/badnode"; chmod +x "$T/badnode"
set +e; "$T/release/zapp-hints" --bitcoin-cli "$T/node" $V/mainnet-3.receipt $V/mainnet-3.hints > "$T/id.out"; e=$?; set -e
[ $e = 2 ] && grep -q "your node's block 3 is $H3" "$T/id.out" || { cat "$T/id.out"; echo "::error::--bitcoin-cli, an agreeing node: exit $e, want 2"; exit 1; }
set +e; "$T/release/zapp-hints" --bitcoin-cli "$T/badnode" $V/mainnet-3.receipt $V/mainnet-3.hints > "$T/id.out"; e=$?; set -e
[ $e = 1 ] || { cat "$T/id.out"; echo "::error::--bitcoin-cli, a node naming another block: exit $e, want 1"; exit 1; }
cargo build --release --manifest-path ./ffi/Cargo.toml
(cd ffi && cc -O2 -Iinclude test_hints.c "$FT/release/libzapp_hints_ffi.a" -lpthread -ldl -lm -o "$T/htest" && "$T/htest" "$V" $H3)
./wasm/build.sh
node wasm/test-hints.mjs "$V" $H3
echo "ZAPP_HINTS_TESTS OK"
