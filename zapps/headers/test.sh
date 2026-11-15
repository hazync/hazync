#!/bin/bash
# The header-chain zAPP's tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the library's
# unit tests (real receipts, MMR paths, transaction inclusion), the WASM crate's frame tests (natively), the CLI on the
# real chained receipts (test_headers_verify.sh) and the whole chain genesis..975,278, and the WASM module through its
# glue (wasm/test-headers.mjs). The receipts are the experiment's (experimental/libzync-bench/headers).
set -euo pipefail
cd "$(dirname "$0")"
T=${CARGO_TARGET_DIR:-$PWD/target}
cargo test --release --manifest-path ./Cargo.toml
cargo test --release --manifest-path ./wasm/Cargo.toml
cargo build --release --manifest-path ./Cargo.toml --bin zapp-headers
bash test_headers_verify.sh "$T/release/zapp-headers"
"$T/release/zapp-headers" --expect 975278:00000000000000000001d951592f7a06f0d24697e4f9c362ae173fced55583e3 \
    ../../experimental/libzync-bench/headers/headers_0_tip.rcpt
# the chain identity through the other sources (zapp-common, every zAPP CLI's flags): a node that names block 200 as the
# receipts do (DEMO: valid, below minimum work), one that names another block (FAIL)
H=../../experimental/libzync-bench/headers
printf '#!/bin/sh\n[ "$1" = getblockhash ] && echo 000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320\nexit 0\n' > "$T/node"; chmod +x "$T/node"
printf '#!/bin/sh\necho %064d\n' 1 > "$T/badnode"; chmod +x "$T/badnode"
set +e; "$T/release/zapp-headers" --bitcoin-cli "$T/node" $H/hdr_0_100.rcpt $H/hdr_101_200.rcpt > "$T/id.out"; e=$?; set -e
[ $e = 2 ] && grep -q "your node's block 200 is" "$T/id.out" || { cat "$T/id.out"; echo "::error::--bitcoin-cli, an agreeing node: exit $e, want 2"; exit 1; }
set +e; "$T/release/zapp-headers" --bitcoin-cli "$T/badnode" $H/hdr_0_100.rcpt $H/hdr_101_200.rcpt > "$T/id.out"; e=$?; set -e
[ $e = 1 ] || { cat "$T/id.out"; echo "::error::--bitcoin-cli, a node naming another block: exit $e, want 1"; exit 1; }
./wasm/build.sh
node wasm/test-headers.mjs
echo "ZAPP_HEADERS_TESTS OK"
