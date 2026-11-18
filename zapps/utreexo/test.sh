#!/bin/bash
# The Utreexo zAPP's tests, run from this directory: the library's unit tests, the CLI and the WASM module (through its
# glue) on the real receipts in vectors/ (CPU proofs, docs/libzync/UTREEXO_ZAPP.md §8; below minimum work, so DEMO),
# and the WASM's sha256 against the one wasm/README.md publishes.
set -euo pipefail
cd "$(dirname "$0")"
. ../../scripts/zapp-checks.sh
T=${CARGO_TARGET_DIR:-$PWD/cli/target}
V=$PWD/vectors
H3=0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449
H2=000000006a625f06636b8bb6ac7b960a8d03705d1ace08b1a19da3fdcc99ddbd
unset RISC0_DEV_MODE
cargo test --release --manifest-path ./Cargo.toml --features tip
cargo test --release --manifest-path ./wasm/Cargo.toml
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
want 2 "H = 3, block 3 named" -- verify $V/mainnet-3.receipt --expect 3:$H3
want 2 "H = 3, two runs" -- verify $V/mainnet-3-split.receipt --expect 3:$H3
want 1 "H = 3, another hash" -- verify $V/mainnet-3.receipt --expect 3:$H2
want 1 "H = 3, T = H, a wrong tip given" -- verify $V/mainnet-3.receipt --expect 3:$H3 --expect-tip 3:$H2
want 2 "H = 2 under T = 3, both named" -- verify $V/mainnet-2.receipt --expect 2:$H2 --expect-tip 3:$H3
want 1 "H = 2 under T = 3, a wrong tip" -- verify $V/mainnet-2.receipt --expect 2:$H2 --expect-tip 3:$H2
want 1 "another image id" -- verify $V/mainnet-3.receipt --expect 3:$H3 --image "$(printf 'ab%.0s' $(seq 32))"
want 1 "an unknown flag (--expect-at, until 16.11.2026)" -- verify $V/mainnet-3.receipt --expect-at 3:$H3
want 1 "a flag without its value" -- verify $V/mainnet-3.receipt --expect-at
want 1 "two receipts" -- verify $V/mainnet-3.receipt $V/mainnet-2.receipt
want 1 "a truncated receipt" -- verify <(head -c 1000 $V/mainnet-3.receipt)
grep -q "FAIL.*the receipt" "$T/out" || { cat "$T/out"; echo "::error::the truncated receipt is not refused as a receipt"; exit 1; }
want 1 "a receipt with a trailing byte" -- verify <(cat $V/mainnet-3.receipt; printf '\0') --expect 3:$H3
grep -q "FAIL.*the receipt decodes" "$T/out" || { cat "$T/out"; echo "::error::the trailing byte is not refused by the decode"; exit 1; }
# the caller names the network (default mainnet); a proof of another is FAIL
want 2 "--network mainnet, named" -- verify $V/mainnet-3.receipt --network mainnet --expect 3:$H3
want 1 "a mainnet proof checked as signet" -- verify $V/mainnet-3.receipt --network signet --expect 3:$H3
grep -q "FAIL.*the proof is for signet" "$T/out" || { cat "$T/out"; echo "::error::the network mismatch is not the failed check"; exit 1; }
want 1 "--network regtest (no image)" -- verify $V/mainnet-3.receipt --network regtest --expect 3:$H3
want 1 "--image twice" -- verify $V/mainnet-3.receipt --image $(printf 'ab%.0s' $(seq 32)) --image $(printf 'ab%.0s' $(seq 32)) --expect 3:$H3
want 1 "--network twice" -- verify $V/mainnet-3.receipt --network mainnet --network signet --expect 3:$H3
# --json / --floresta: nothing on stdout on DEMO; with --allow-demo exactly the JSON, marked demo
want 2 "--json on DEMO" -- verify $V/mainnet-3.receipt --expect 3:$H3 --json
[ ! -s "$T/out" ] || { cat "$T/out"; echo "::error::--json printed a DEMO value without --allow-demo"; exit 1; }
want 2 "--json --allow-demo" -- verify $V/mainnet-3.receipt --expect 3:$H3 --json --allow-demo
python3 -c "import json,sys; j=json.load(open(sys.argv[1])); assert j['verdict']=='demo' and j['network']=='mainnet' and j['identity_checked'] and j['leaves']==3 and j['image_source']=='builtin-unpinned' and j['image_label'], j" "$T/out"
want 2 "--floresta --allow-demo" -- verify $V/mainnet-3.receipt --expect 3:$H3 --floresta --allow-demo
grep -q 'acchashes!\[' "$T/out" && grep -q 'leaves: 3,' "$T/out" && head -1 "$T/out" | grep -q '^// mainnet:' && ! grep -q ZAPP_UTREEXO "$T/out" || { cat "$T/out"; echo "::error::--floresta output"; exit 1; }
# the chain identity through the other sources (zapp-common, every zAPP CLI's flags). A fake node answers by height
# (getblockhash <h>, and getblockheader with that block's chainwork: difficulty 1, 0x100010001 per block)
mknode() { # mknode <file> <hash at 2> <hash at 3> [the blocks of work it claims at 3, default 4 (genesis + 3)]
    printf '#!/bin/sh\ncase "$1 $2" in\n "getblockhash 2") echo %s;;\n "getblockhash 3") echo %s;;\n "getblockheader %s") echo "{\\"chainwork\\": \\"%064x\\"}";;\n "getblockheader %s") echo "{\\"chainwork\\": \\"%064x\\"}";;\nesac\n' \
        "$2" "$3" "$2" $((3 * 0x100010001)) "$3" $((${4:-4} * 0x100010001)) > "$1"; chmod +x "$1"
}
mknode "$T/node" $H2 $H3
mknode "$T/wrong3" $H2 "$(printf '%064d' 1)"
mknode "$T/badwork3" $H2 $H3 5
want 2 "--bitcoin-cli: block 3 as the proof says (DEMO)" -- verify $V/mainnet-3.receipt --bitcoin-cli "$T/node"
grep -q "your node's block 3 is $H3" "$T/out" || { cat "$T/out"; echo "::error::the node's answer is not named"; exit 1; }
want 1 "--bitcoin-cli: a node that names another block 3" -- verify $V/mainnet-3.receipt --bitcoin-cli "$T/wrong3"
want 2 "--bitcoin-cli, H = 2 below the core tip 3: both blocks asked (DEMO)" -- verify $V/mainnet-2.receipt --bitcoin-cli "$T/node"
grep -q "your node's block 2 is $H2" "$T/out" && grep -q "your node's block 3 is $H3" "$T/out" || { cat "$T/out"; echo "::error::blocks 2 and 3 were not both asked"; exit 1; }
want 1 "--bitcoin-cli, H = 2: the node's block 3 is not the core tip" -- verify $V/mainnet-2.receipt --bitcoin-cli "$T/wrong3"
want 1 "--bitcoin-cli, H = 2: the node's chainwork at block 3 is not the proven one" -- verify $V/mainnet-2.receipt --bitcoin-cli "$T/badwork3"
# --expect-tip given with a lookup source: block T is still asked, must agree with it, and its chainwork is compared
want 2 "--bitcoin-cli and --expect-tip, H = 2 under T = 3 (DEMO)" -- verify $V/mainnet-2.receipt --bitcoin-cli "$T/node" --expect-tip 3:$H3
grep -q "chainwork at block 3 equals the proven" "$T/out" || { cat "$T/out"; echo "::error::the node's chainwork at T was not compared"; exit 1; }
want 1 "--bitcoin-cli and --expect-tip: the node's chainwork at block 3 is not the proven one" -- verify $V/mainnet-2.receipt --bitcoin-cli "$T/badwork3" --expect-tip 3:$H3
want 1 "--bitcoin-cli and --expect-tip: the node disagrees with the tip given" -- verify $V/mainnet-2.receipt --bitcoin-cli "$T/wrong3" --expect-tip 3:$H3
# tip mode (UTREEXO_ZAPP.md §10): a real CPU proof of a step from the base at 3 (vectors/mainnet-3.receipt) to 103,
# linked to the core proof 1..3,836 (the spine of 18.11.2026)
H103=000000002109057cb846ff7d1d96bc77f9f65cb547938efe1961760a8bbb3f75
T3836=00000000e26d786351dd5c6926f1b04a54e8f7fd4a7c81cec40db01505b143ee
R=$V/mainnet-tip-103.receipt
want 2 "tip 103: block 103 and the core tip named (DEMO)" -- verify $R --expect 103:$H103 --expect-tip 3836:$T3836
grep -q "tip mode: 1 steps from the base at 3, the last from 3" "$T/out" || { cat "$T/out"; echo "::error::the tip line"; exit 1; }
want 1 "tip 103, another block 103" -- verify $R --expect 103:$H3 --expect-tip 3836:$T3836
want 1 "tip 103, another core tip" -- verify $R --expect 103:$H103 --expect-tip 3836:$H103
want 1 "tip 103, another tip image" -- verify $R --expect 103:$H103 --image "$(printf 'ab%.0s' $(seq 32))"
want 1 "tip 103, another base image" -- verify $R --expect 103:$H103 --base-image "$(printf 'ab%.0s' $(seq 32))"
grep -q "FAIL.*the chain starts from a walk of the trusted base image" "$T/out" || { cat "$T/out"; echo "::error::the base image is not the failed check"; exit 1; }
want 1 "tip 103 checked as signet" -- verify $R --network signet --expect 103:$H103
want 1 "--base-image on a base receipt" -- verify $V/mainnet-3.receipt --expect 3:$H3 --base-image "$(printf 'ab%.0s' $(seq 32))"
want 1 "tip 103 with a trailing byte" -- verify <(cat $R; printf '\0') --expect 103:$H103
want 2 "tip 103 --json --allow-demo" -- verify $R --expect 103:$H103 --expect-tip 3836:$T3836 --json --allow-demo
python3 -c "import json,sys; j=json.load(open(sys.argv[1])); assert j['mode']=='tip' and j['steps']==1 and j['from']==3 and j['base_height']==3 and j['height']==103 and j['leaves']==103 and j['n_live'] is None and j['core_height']==3836 and j['image_source']=='builtin-unpinned', j" "$T/out"
want 2 "tip 103 --floresta --allow-demo" -- verify $R --expect 103:$H103 --floresta --allow-demo
grep -q 'height: 103,' "$T/out" && head -1 "$T/out" | grep -q '^// mainnet:' || { cat "$T/out"; echo "::error::--floresta output (tip)"; exit 1; }
echo "ZAPP_UTREEXO_CLI $n cases"
./wasm/build.sh
wasm_sha_is_the_readmes wasm/target/wasm32-unknown-unknown/release/zapp_utreexo_wasm.wasm wasm/README.md
node wasm/test.mjs
echo "ZAPP_UTREEXO_TESTS OK"
