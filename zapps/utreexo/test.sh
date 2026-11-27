#!/bin/bash
# The Utreexo zAPP's tests, run from this directory: the library's unit tests, the CLI and the WASM module (through its
# glue) on the engine's real proofs in tests/data/engine (the base: roots after block 100, and a tip step 100 -> 150,
# both over the engine chain's core proof of mainnet 1..200; below minimum work, so DEMO), and the WASM's sha256
# against the one wasm/README.md publishes. Engine proofs only since the cutover (27.11.2026); no engine image is
# published yet, so every call gives the image ids and the key.
set -euo pipefail
cd "$(dirname "$0")"
. ../../scripts/zapp-checks.sh
T=${CARGO_TARGET_DIR:-$PWD/cli/target}
V=$PWD/tests/data/engine
BASE=340f6675bafcdd893c12e0dfae0391043cc7fd8919b526ff756ec13d5d76cc95
TIPI=6ecf6f438938c0c63ed39daf089168fb607eefc806f4b28435d73d54eae09c1b
CORE=c7fe0eb85d2d4c10532742191b059a2adcc3e24e22aa81c1e632e2ee718d2fc9
H100=000000007bc154e0fa7ea32218a72fe2c1bb9f86cf8c9ebf9a715ed27fdb229a
H150=000000009ca75733b4cf527fe193b919201a2ed38c9e147a5665fdfade551f4d
H200=000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320
B="--image $BASE --vk $V/base100.vk --core-image $CORE"; P=$V/base100.hze
TK="--image $TIPI --vk $V/step150.vk --core-image $CORE --base-image $BASE"; R=$V/step150.hze
AB=$(printf 'ab%.0s' $(seq 32))
cargo test --release --manifest-path ./Cargo.toml --features tip
# the tip step's block rules natively (the BIP30 refusal on the real coinbases of 91,722 / 91,812): the guest's step.rs
cargo test --release --manifest-path ./tip-step/Cargo.toml
wasm_crate_test ./wasm
cargo test --release --manifest-path ./cli/Cargo.toml
cargo build --release --manifest-path ./cli/Cargo.toml --bin zapp-utreexo
Z="$T/release/zapp-utreexo"
n=0
want() { # want <exit> <what> -- <args>
    local w=$1 what=$2; shift 3
    set +e; "$Z" "$@" > "$T/out" 2> "$T/err"; local e=$?; set -e
    [ $e = "$w" ] || { cat "$T/out" "$T/err"; echo "::error::$what: exit $e, want $w"; exit 1; }
    n=$((n + 1))
}
want 2 "base: block 100 and the core tip named" -- verify $P $B --expect 100:$H100 --expect-tip 200:$H200
want 1 "base: another block 100" -- verify $P $B --expect 100:$H150 --expect-tip 200:$H200
want 1 "base: another core tip" -- verify $P $B --expect 100:$H100 --expect-tip 200:$H150
want 1 "base: T = 200 given as block 100's hash" -- verify $P $B --expect 100:$H100 --expect-tip 100:$H100
want 1 "another image id" -- verify $P --image $AB --vk $V/base100.vk --core-image $CORE --expect 100:$H100
want 1 "another core image" -- verify $P --image $BASE --vk $V/base100.vk --core-image $AB --expect 100:$H100
want 1 "the tip image's key for the base proof" -- verify $P --image $BASE --vk $V/step150.vk --core-image $CORE --expect 100:$H100
want 1 "no --vk (no engine image is published yet)" -- verify $P --image $BASE --core-image $CORE --expect 100:$H100
want 1 "no --core-image" -- verify $P --image $BASE --vk $V/base100.vk --expect 100:$H100
want 1 "an unknown flag (--expect-at, until 16.11.2026)" -- verify $P $B --expect-at 100:$H100
want 1 "a flag without its value" -- verify $P $B --expect-at
want 1 "two proofs" -- verify $P $R $B
want 1 "a truncated proof" -- verify <(head -c 1000 $P) $B
grep -q "FAIL.*the engine proof verifies" "$T/out" || { cat "$T/out"; echo "::error::the truncated proof is not refused as a proof"; exit 1; }
want 1 "a proof with a trailing byte" -- verify <(cat $P; printf '\0') $B --expect 100:$H100
want 2 "--network mainnet, named" -- verify $P $B --network mainnet --expect 100:$H100
want 1 "a mainnet proof checked as signet" -- verify $P $B --network signet --expect 100:$H100
grep -q "FAIL.*the proof is for signet" "$T/out" || { cat "$T/out"; echo "::error::the network mismatch is not the failed check"; exit 1; }
want 1 "--network regtest (no image)" -- verify $P $B --network regtest --expect 100:$H100
want 1 "--image twice" -- verify $P $B --image $BASE --expect 100:$H100
want 1 "--network twice" -- verify $P $B --network mainnet --network signet --expect 100:$H100
want 2 "--json on DEMO" -- verify $P $B --expect 100:$H100 --json
[ ! -s "$T/out" ] || { cat "$T/out"; echo "::error::--json printed a DEMO value without --allow-demo"; exit 1; }
want 2 "--json --allow-demo" -- verify $P $B --expect 100:$H100 --json --allow-demo
python3 -c "import json,sys; j=json.load(open(sys.argv[1])); assert j['verdict']=='demo' and j['network']=='mainnet' and j['identity_checked'] and j['height']==100 and j['image_source']=='caller' and j['image_label'] is None, j" "$T/out"
want 2 "--floresta --allow-demo" -- verify $P $B --expect 100:$H100 --floresta --allow-demo
head -1 "$T/out" | grep -q '^// mainnet:' && grep -q 'height: 100,' "$T/out" && ! grep -q ZAPP_UTREEXO "$T/out" || { cat "$T/out"; echo "::error::--floresta output"; exit 1; }
mknode() { # mknode <file> <hash at 100> <hash at 200> [the blocks of work it claims at 200, default 201 (genesis + 200)]
    printf '#!/bin/sh\ncase "$1 $2" in\n "getblockhash 100") echo %s;;\n "getblockhash 200") echo %s;;\n "getblockheader %s") echo "{\\"chainwork\\": \\"%064x\\"}";;\n "getblockheader %s") echo "{\\"chainwork\\": \\"%064x\\"}";;\nesac\n' \
        "$2" "$3" "$2" $((101 * 0x100010001)) "$3" $((${4:-201} * 0x100010001)) > "$1"; chmod +x "$1"
}
mknode "$T/node" $H100 $H200
mknode "$T/wrong200" $H100 "$(printf '%064d' 1)"
mknode "$T/badwork" $H100 $H200 202
want 2 "--bitcoin-cli, H = 100 below the core tip 200: both blocks asked (DEMO)" -- verify $P $B --bitcoin-cli "$T/node"
grep -q "your node's block 100 is $H100" "$T/out" && grep -q "your node's block 200 is $H200" "$T/out" || { cat "$T/out"; echo "::error::blocks 100 and 200 were not both asked"; exit 1; }
want 1 "--bitcoin-cli: the node's block 200 is not the core tip" -- verify $P $B --bitcoin-cli "$T/wrong200"
want 1 "--bitcoin-cli: the node's chainwork at block 200 is not the proven one" -- verify $P $B --bitcoin-cli "$T/badwork"
want 2 "--bitcoin-cli and --expect-tip (DEMO)" -- verify $P $B --bitcoin-cli "$T/node" --expect-tip 200:$H200
want 1 "--bitcoin-cli and --expect-tip: the node disagrees with the tip given" -- verify $P $B --bitcoin-cli "$T/wrong200" --expect-tip 200:$H200
# tip mode (UTREEXO_ZAPP.md §10): the engine's step 100 -> 150 on the base above, over the core proof of 1..200
want 2 "tip 150: block 150 and the core tip named (DEMO)" -- verify $R $TK --expect 150:$H150 --expect-tip 200:$H200
grep -q "tip mode: 1 steps from the base at 100" "$T/out" || { cat "$T/out"; echo "::error::the tip line"; exit 1; }
want 1 "tip 150, another block 150" -- verify $R $TK --expect 150:$H100 --expect-tip 200:$H200
want 1 "tip 150, another core tip" -- verify $R $TK --expect 150:$H150 --expect-tip 200:$H150
want 1 "tip 150, another tip image" -- verify $R --image $AB --vk $V/step150.vk --core-image $CORE --base-image $BASE --expect 150:$H150
want 1 "tip 150, another base image" -- verify $R --image $TIPI --vk $V/step150.vk --core-image $CORE --base-image $AB --expect 150:$H150
grep -q "FAIL.*the chain starts from a walk of the trusted base image" "$T/out" || { cat "$T/out"; echo "::error::the base image is not the failed check"; exit 1; }
want 1 "tip 150 without --base-image" -- verify $R --image $TIPI --vk $V/step150.vk --core-image $CORE --expect 150:$H150
want 1 "tip 150 checked as signet" -- verify $R $TK --network signet --expect 150:$H150
want 1 "--base-image on a base proof" -- verify $P $B --expect 100:$H100 --base-image $AB
want 1 "tip 150 with a trailing byte" -- verify <(cat $R; printf '\0') $TK --expect 150:$H150
want 2 "tip 150 --json --allow-demo" -- verify $R $TK --expect 150:$H150 --expect-tip 200:$H200 --json --allow-demo
python3 -c "import json,sys; j=json.load(open(sys.argv[1])); assert j['mode']=='tip' and j['steps']==1 and j['from']==100 and j['base_height']==100 and j['height']==150 and j['n_live'] is None and j['core_height']==200 and j['image_source']=='caller', j" "$T/out"
want 2 "tip 150 --floresta --allow-demo" -- verify $R $TK --expect 150:$H150 --floresta --allow-demo
grep -q 'height: 150,' "$T/out" && head -1 "$T/out" | grep -q '^// mainnet:' || { cat "$T/out"; echo "::error::--floresta output (tip)"; exit 1; }
echo "ZAPP_UTREEXO_CLI $n cases"
./wasm/build.sh
wasm_sha_is_the_readmes wasm/target/wasm32-unknown-unknown/release/zapp_utreexo_wasm.wasm wasm/README.md
node wasm/test.mjs "$V"
echo "ZAPP_UTREEXO_TESTS OK"
