#!/bin/bash
# The Utreexo zAPP's tests, run from this directory: the library's unit tests, the CLI and the WASM module (through its
# glue) on the real receipts in vectors/ (CPU proofs, docs/libzync/UTREEXO_ZAPP.md §8; below minimum work, so DEMO),
# and the WASM's sha256 against the one wasm/README.md publishes.
set -euo pipefail
cd "$(dirname "$0")"
T=${CARGO_TARGET_DIR:-$PWD/target}
V=$PWD/vectors
H3=0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449
H2=000000006a625f06636b8bb6ac7b960a8d03705d1ace08b1a19da3fdcc99ddbd
unset RISC0_DEV_MODE
cargo test --release --manifest-path ./Cargo.toml
cargo build --release --manifest-path ./Cargo.toml --bin zapp-utreexo
Z="$T/release/zapp-utreexo"
n=0
want() { # want <exit> <what> -- <args>
    local w=$1 what=$2; shift 3
    set +e; "$Z" "$@" > "$T/out" 2> "$T/err"; local e=$?; set -e
    [ $e = "$w" ] || { cat "$T/out" "$T/err"; echo "::error::$what: exit $e, want $w"; exit 1; }
    n=$((n + 1))
}
want 2 "H = 3, block 3 named" -- verify $V/mainnet-3.receipt --expect-at 3:$H3
want 2 "H = 3, two runs" -- verify $V/mainnet-3-split.receipt --expect-at 3:$H3
want 1 "H = 3, another hash" -- verify $V/mainnet-3.receipt --expect-at 3:$H2
want 1 "H = 3, T = H, a wrong tip given" -- verify $V/mainnet-3.receipt --expect-at 3:$H3 --expect-tip 3:$H2
want 2 "H = 2 under T = 3, both named" -- verify $V/mainnet-2.receipt --expect-at 2:$H2 --expect-tip 3:$H3
want 1 "H = 2 under T = 3, a wrong tip" -- verify $V/mainnet-2.receipt --expect-at 2:$H2 --expect-tip 3:$H2
want 1 "another image id" -- verify $V/mainnet-3.receipt --expect-at 3:$H3 --image "$(printf 'ab%.0s' $(seq 32))"
want 1 "an unknown flag" -- verify $V/mainnet-3.receipt --expect 3:$H3
want 1 "a flag without its value" -- verify $V/mainnet-3.receipt --expect-at
want 1 "two receipts" -- verify $V/mainnet-3.receipt $V/mainnet-2.receipt
want 1 "a truncated receipt" -- verify <(head -c 1000 $V/mainnet-3.receipt)
# --json / --floresta: nothing on stdout on DEMO; with --allow-demo exactly the JSON, marked demo
want 2 "--json on DEMO" -- verify $V/mainnet-3.receipt --expect-at 3:$H3 --json
[ ! -s "$T/out" ] || { cat "$T/out"; echo "::error::--json printed a DEMO value without --allow-demo"; exit 1; }
want 2 "--json --allow-demo" -- verify $V/mainnet-3.receipt --expect-at 3:$H3 --json --allow-demo
python3 -c "import json,sys; j=json.load(open(sys.argv[1])); assert j['verdict']=='demo' and j['network']=='mainnet' and j['identity_checked'] and j['leaves']==3, j" "$T/out"
want 2 "--floresta --allow-demo" -- verify $V/mainnet-3.receipt --expect-at 3:$H3 --floresta --allow-demo
grep -q 'acchashes!\[' "$T/out" && grep -q 'leaves: 3,' "$T/out" && ! grep -q ZAPP_UTREEXO "$T/out" || { cat "$T/out"; echo "::error::--floresta output"; exit 1; }
echo "ZAPP_UTREEXO_CLI $n cases"
./wasm/build.sh
node wasm/test.mjs
want_sha=$(sed -n 's/^sha256 `\([0-9a-f]\{64\}\)`$/\1/p' wasm/README.md)
got_sha=$(sha256sum wasm/target/wasm32-unknown-unknown/release/zapp_utreexo_wasm.wasm | cut -d' ' -f1)
[ "$want_sha" = "$got_sha" ] || { echo "::error::wasm sha256 $got_sha, wasm/README.md says $want_sha"; exit 1; }
echo "ZAPP_UTREEXO_TESTS OK"
