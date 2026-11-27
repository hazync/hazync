#!/bin/bash
# The hints zAPP's tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the library's unit
# tests, the CLI (cli/), the C ABI alone and beside the core verifier's in one program, and the WASM module (its
# glue, its raw frame, its sha against the README) on the engine's real hints proof in tests/data/engine (H = 200
# over the engine chain's core proof of mainnet 1..200: DEMO, below minimum work). Engine proofs only since the cutover
# (27.11.2026); no engine image is published yet, so every call gives the hints image id, its key and the core image.
set -euo pipefail
cd "$(dirname "$0")"
. ../../scripts/zapp-checks.sh
CT=${CARGO_TARGET_DIR:-$PWD/cli/target}
FT=${CARGO_TARGET_DIR:-$PWD/ffi/target}
H200=000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320
CORE=c7fe0eb85d2d4c10532742191b059a2adcc3e24e22aa81c1e632e2ee718d2fc9
V=$PWD/tests/data/engine
K=(--image "$(cat $V/image_id)" --vk $V/vk.bin --core-image $CORE)
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
cargo test --release --manifest-path ./Cargo.toml
cargo test --release --manifest-path ./wasm/Cargo.toml
cargo build --release --manifest-path ./cli/Cargo.toml --bin zapp-hints
Z=$CT/release/zapp-hints
want() { # want <exit> <what> <command...>: the command's exit code is <exit>
    local w=$1 what=$2 e; shift 2
    set +e; "$@" > "$W/out" 2>&1; e=$?; set -e
    if [ "$e" = "$w" ]; then echo "ok   $what (exit $e)"; else cat "$W/out"; echo "::error::$what: exit $e, want $w"; exit 1; fi
}
want 2 "the proven hints file (DEMO)" "$Z" "${K[@]}" --expect 200:$H200 --undo $V/mainnet-200.undo $V/proof.hze $V/mainnet-200.hints
printf 'UTXO\0\377\377\377\377' > "$W/tiny"
want 1 "a 9-byte file claiming H = 2^32 - 1" "$Z" "${K[@]}" --expect 200:$H200 $V/proof.hze "$W/tiny"
want 1 "H = T = 200, a wrong --expect-tip (compared also when T = H)" "$Z" "${K[@]}" --expect 200:$H200 --expect-tip 200:$(printf '%064d' 1) $V/proof.hze $V/mainnet-200.hints
want 1 "no --core-image (no engine image is published yet)" "$Z" --image "$(cat $V/image_id)" --vk $V/vk.bin $V/proof.hze $V/mainnet-200.hints
want 1 "another core image" "$Z" --image "$(cat $V/image_id)" --vk $V/vk.bin --core-image "$(cat $V/image_id)" $V/proof.hze $V/mainnet-200.hints
# the chain identity through the other sources (zapp-common, every zAPP CLI's flags). A fake node answers getblockhash
# 200, and getblockheader with that block's chainwork (difficulty 1: 0x100010001 per block).
mknode() { # mknode <file> <hash at 200>
    printf '#!/bin/sh\ncase "$1 $2" in\n "getblockhash 200") echo %s;;\n "getblockheader %s") echo "{\\"chainwork\\": \\"%064x\\"}";;\nesac\n' \
        "$2" "$2" $((201 * 0x100010001)) > "$1"; chmod +x "$1"
}
mknode "$W/node" $H200
mknode "$W/wrong" "$(printf '%064d' 1)"
want 2 "--bitcoin-cli: a node that names block 200 as the proof does (DEMO)" "$Z" "${K[@]}" --bitcoin-cli "$W/node" $V/proof.hze $V/mainnet-200.hints
grep -q "your node's block 200 is $H200" "$W/out" || { cat "$W/out"; echo "::error::the node's answer is not named"; exit 1; }
want 1 "--bitcoin-cli: a node that names another block 200" "$Z" "${K[@]}" --bitcoin-cli "$W/wrong" $V/proof.hze $V/mainnet-200.hints
cargo build --release --manifest-path ./ffi/Cargo.toml
(cd ffi && cc -O2 -Iinclude test_hints.c "$FT/release/libzapp_hints_ffi.a" -lpthread -ldl -lm -o "$W/htest" && "$W/htest" "$V" $H200 $CORE)
c_with_core ffi/test_with_core.c ffi/include "$FT/release" zapp_hints_ffi "$W/both"
"$W/both"
./wasm/build.sh
wasm_sha_is_the_readmes wasm/target/wasm32-unknown-unknown/release/zapp_hints_wasm.wasm wasm/README.md
node wasm/test-hints.mjs "$V" $H200 $CORE
echo "ZAPP_HINTS_TESTS OK"
