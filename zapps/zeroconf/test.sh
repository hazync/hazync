#!/bin/bash
# The zeroconf zAPP's tests, run from this directory: the library on the real engine proof (tests/engine.rs), then the
# CLI on the same proof (the regtest harness: the dev-profile build with the tests-only regtest-test feature), each
# case with the exit code it must give (0 OK, 2 DEMO, 1 FAIL).
# the key pin a reader gives with every key (review R1-6 #1: a key is never pinned to itself); here the fixture key's own
# SHA-256 stands for "your own build's vk.sha256"
pin() { sha256sum "$1" | cut -c1-64; }
set -euo pipefail
cd "$(dirname "$0")"
D=$PWD/tests/data/engine
cargo test --release
cargo build --manifest-path ./cli/Cargo.toml --features regtest-test
Z=${CARGO_TARGET_DIR:-$PWD/cli/target}/debug/zapp-zeroconf
ZC=$(cat $D/image_id); CORE=$(cat $D/core_image_id); TXID=$(python3 -c "import json;print(json.load(open('$D/T.json'))['txid'])")
TIP=110:2d5cc9c833f072d2744d26a9354e94111e77d05669b5004f7d68641c42ae9d53
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
n=0
want() { # want <exit> <what> <args...>
    local w=$1 what=$2 e; shift 2
    set +e; "$Z" "$@" > "$W/out" 2>&1; e=$?; set -e
    if [ "$e" = "$w" ]; then n=$((n+1)); echo "ok   $what (exit $e)"; else cat "$W/out"; echo "::error::$what: exit $e, want $w"; exit 1; fi
}
B=("$D/proof.hze" --vk "$D/vk.bin" --vk-sha256 "$(pin "$D/vk.bin")" --image "$ZC" --core-image "$CORE" --network regtest --anchor "$D/anchor_state_109.json")
want 2 "T, the tip compared (DEMO: a given anchor)"             "${B[@]}" --txid $TXID --expect $TIP
want 2 "T, no tip given (DEMO)"                                  "${B[@]}" --txid $TXID
want 1 "another txid"                                            "${B[@]}" --txid "${TXID/6/7}" --expect $TIP
want 1 "another tip"                                             "${B[@]}" --txid $TXID --expect "${TIP/d53/d54}"
want 1 "another core image"                                      $D/proof.hze --vk $D/vk.bin --vk-sha256 "$(pin $D/vk.bin)" --image $ZC --core-image $ZC --network regtest --anchor $D/anchor_state_109.json --txid $TXID
want 1 "genesis claimed (no --anchor)"                           $D/proof.hze --vk $D/vk.bin --vk-sha256 "$(pin $D/vk.bin)" --image $ZC --core-image $CORE --network regtest --txid $TXID
want 1 "another image id"                                        $D/proof.hze --vk $D/vk.bin --vk-sha256 "$(pin $D/vk.bin)" --image $CORE --core-image $CORE --network regtest --anchor $D/anchor_state_109.json --txid $TXID
printf 'x' > "$W/k"; cat $D/vk.bin >> "$W/k"
want 1 "another key file"                                        $D/proof.hze --vk "$W/k" --vk-sha256 "$(pin "$W/k")" --image $ZC --core-image $CORE --network regtest --anchor $D/anchor_state_109.json --txid $TXID
want 1 "the key without --vk-sha256 (review R1-6 #1)"            $D/proof.hze --vk $D/vk.bin --image $ZC --core-image $CORE --network regtest --anchor $D/anchor_state_109.json --txid $TXID
want 1 "the key with another pin"                                $D/proof.hze --vk $D/vk.bin --vk-sha256 0000000000000000000000000000000000000000000000000000000000000000 --image $ZC --core-image $CORE --network regtest --anchor $D/anchor_state_109.json --txid $TXID
want 1 "mainnet asked of a regtest proof"                        $D/proof.hze --vk $D/vk.bin --vk-sha256 "$(pin $D/vk.bin)" --image $ZC --core-image $CORE --network mainnet --anchor $D/anchor_state_109.json --txid $TXID
want 1 "no --txid"                                               "${B[@]}"
WTXID=$(python3 -c "import json;print(json.load(open('$D/T.json'))['wtxid'])")
want 2 "T with its wtxid (review R2-6 #3)"                       "${B[@]}" --txid $TXID --wtxid $WTXID --expect $TIP
want 1 "T with another wtxid"                                    "${B[@]}" --txid $TXID --wtxid "${WTXID/a/b}" --expect $TIP
set +e; "$Z" "${B[@]}" --txid $TXID --expect $TIP --json 2> /dev/null > "$W/j"; set -e
python3 -c "import json,sys;j=json.load(open('$W/j'));assert j['verdict']=='DEMO' and j['txid']=='$TXID' and j['next_block']==111, j" && { n=$((n+1)); echo "ok   --json: verdict, txid, next block"; }
echo "ZAPP_ZEROCONF_CLI: $n cases as expected"
