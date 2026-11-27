#!/bin/bash
# The header-chain zAPP's tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the library's
# unit tests (the engine proof 0..4,031, MMR paths, transaction inclusion), the WASM crate's frame tests (natively), the
# CLI on that proof (test_headers_verify.sh) with every chain-identity source, and the WASM module through its glue
# (wasm/test-headers.mjs). Engine proofs only since the cutover (27.11.2026).
set -euo pipefail
cd "$(dirname "$0")"
. ../../scripts/zapp-checks.sh
T=${CARGO_TARGET_DIR:-$PWD/cli/target}
cargo test --release --manifest-path ./Cargo.toml
cargo test --release --manifest-path ./wasm/Cargo.toml
cargo build --release --manifest-path ./cli/Cargo.toml --bin zapp-headers
bash test_headers_verify.sh "$T/release/zapp-headers"
# the chain identity through the other sources (zapp-common, every zAPP CLI's flags): a node that names block 4,031 as
# the proof does (DEMO: valid, below minimum work), one that gives no chainwork, one that names another block (FAIL)
D=tests/data/engine4031; K=(--image "$(cat $D/image_id)" --vk $D/vk.bin)
TIP=00000000f037ad09d0b05ee66b8c1da83030abaf909d2b1bf519c3c7d2cd3fdf
# (block 4,031's chainwork: 4,032 blocks of difficulty 1, 0x100010001 each)
printf '#!/bin/sh\ncase "$1" in getblockhash) echo %s;; getblockheader) echo '"'"'{"chainwork": "%064x"}'"'"';; esac\n' $TIP $((4032 * 0x100010001)) > "$T/node"; chmod +x "$T/node"
printf '#!/bin/sh\n[ "$1" = getblockhash ] && echo %s\nexit 0\n' $TIP > "$T/nowork"; chmod +x "$T/nowork"
printf '#!/bin/sh\necho %064d\n' 1 > "$T/badnode"; chmod +x "$T/badnode"
set +e; "$T/release/zapp-headers" "${K[@]}" --bitcoin-cli "$T/node" $D/proof.hze > "$T/id.out"; e=$?; set -e
[ $e = 2 ] && grep -q "your node's block 4031 is" "$T/id.out" || { cat "$T/id.out"; echo "::error::--bitcoin-cli, an agreeing node: exit $e, want 2"; exit 1; }
set +e; "$T/release/zapp-headers" "${K[@]}" --bitcoin-cli "$T/nowork" $D/proof.hze > "$T/id.out"; e=$?; set -e
[ $e = 1 ] || { cat "$T/id.out"; echo "::error::--bitcoin-cli, a node that gives no chainwork: exit $e, want 1"; exit 1; }
set +e; "$T/release/zapp-headers" "${K[@]}" --bitcoin-cli "$T/badnode" $D/proof.hze > "$T/id.out"; e=$?; set -e
[ $e = 1 ] || { cat "$T/id.out"; echo "::error::--bitcoin-cli, a node naming another block: exit $e, want 1"; exit 1; }
./wasm/build.sh
wasm_sha_is_the_readmes wasm/target/wasm32-unknown-unknown/release/zapp_headers_wasm.wasm wasm/README.md
node wasm/test-headers.mjs
echo "ZAPP_HEADERS_TESTS OK"
