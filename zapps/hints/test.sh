#!/bin/bash
# The hints zAPP's tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the library's unit
# tests, the CLI (cli/), the C ABI alone and beside the core verifier's in one program, and the WASM module (its
# glue, its raw frame, its sha against the README) on the real receipts in vectors/ (CPU proofs by the published hints
# image, docs/libzync/HINTS_ZAPP.md §10; H = 3 over the core's blocks 1..3: DEMO, below minimum work).
set -euo pipefail
cd "$(dirname "$0")"
. ../../scripts/zapp-checks.sh
T=${CARGO_TARGET_DIR:-$PWD/target}
CT=${CARGO_TARGET_DIR:-$PWD/cli/target}
FT=${CARGO_TARGET_DIR:-$PWD/ffi/target}
H2=000000006a625f06636b8bb6ac7b960a8d03705d1ace08b1a19da3fdcc99ddbd
H3=0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449
V=$PWD/vectors
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
want 2 "the proven hints file (DEMO)" "$Z" --expect 3:$H3 --undo $V/mainnet-3.undo $V/mainnet-3.receipt $V/mainnet-3.hints
printf 'UTXO\0\377\377\377\377' > "$W/tiny"
want 1 "a 9-byte file claiming H = 2^32 - 1" "$Z" --expect 3:$H3 $V/mainnet-3.receipt "$W/tiny"
want 1 "RISC0_DEV_MODE set" env RISC0_DEV_MODE=1 "$Z" --expect 3:$H3 $V/mainnet-3.receipt $V/mainnet-3.hints
# the chain identity through the other sources (zapp-common, every zAPP CLI's flags). A fake node answers by height:
# getblockhash <h>, and getblockheader with that block's chainwork (difficulty 1: 0x100010001 per block).
mknode() { # mknode <file> <hash at 2> <hash at 3>
    printf '#!/bin/sh\ncase "$1 $2" in\n "getblockhash 2") echo %s;;\n "getblockhash 3") echo %s;;\n "getblockheader %s") echo "{\\"chainwork\\": \\"%064x\\"}";;\n "getblockheader %s") echo "{\\"chainwork\\": \\"%064x\\"}";;\nesac\n' \
        "$2" "$3" "$2" $((3 * 0x100010001)) "$3" $((4 * 0x100010001)) > "$1"; chmod +x "$1"
}
mknode "$W/node" $H2 $H3
mknode "$W/wrong3" $H2 "$(printf '%064d' 1)"
want 2 "--bitcoin-cli: a node that names block 3 as the proof does (DEMO)" "$Z" --bitcoin-cli "$W/node" $V/mainnet-3.receipt $V/mainnet-3.hints
grep -q "your node's block 3 is $H3" "$W/out" || { cat "$W/out"; echo "::error::the node's answer is not named"; exit 1; }
want 1 "--bitcoin-cli: a node that names another block 3" "$Z" --bitcoin-cli "$W/wrong3" $V/mainnet-3.receipt $V/mainnet-3.hints
# H = 2 under a core tip of 3: the source is asked for block 2 AND for the core tip, block 3 (no --expect-tip given)
want 2 "--bitcoin-cli, H = 2 below the core tip 3: both blocks as the proof says (DEMO)" "$Z" --bitcoin-cli "$W/node" $V/mainnet-2.receipt $V/mainnet-2.hints
grep -q "your node's block 2 is $H2" "$W/out" && grep -q "your node's block 3 is $H3" "$W/out" || { cat "$W/out"; echo "::error::blocks 2 and 3 were not both asked"; exit 1; }
want 1 "--bitcoin-cli, H = 2: the node's block 3 is not the core tip" "$Z" --bitcoin-cli "$W/wrong3" $V/mainnet-2.receipt $V/mainnet-2.hints
cargo build --release --manifest-path ./ffi/Cargo.toml
(cd ffi && cc -O2 -Iinclude test_hints.c "$FT/release/libzapp_hints_ffi.a" -lpthread -ldl -lm -o "$W/htest" && "$W/htest" "$V" $H3)
c_with_core ffi/test_with_core.c ffi/include "$FT/release" zapp_hints_ffi "$W/both" && "$W/both"
./wasm/build.sh
wasm_sha_is_the_readmes wasm/target/wasm32-unknown-unknown/release/zapp_hints_wasm.wasm wasm/README.md
node wasm/test-hints.mjs "$V" $H3
echo "ZAPP_HINTS_TESTS OK"
