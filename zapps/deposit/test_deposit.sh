#!/bin/bash
# End-to-end test of the deposit-proof zyncAPP CLI zapp-deposit (docs/libzync/DEPOSIT_ZAPP.md) on real ENGINE proofs:
# `zapp-deposit build` builds bundles for block 170's Satoshi-to-Hal transaction against the engine chain's proof of
# mainnet 1..200 (swiftsync-verify/vectors), alone and bound to the engine's header chain 0..4,031
# (zapps/headers/tests/data/engine4031); `zapp-deposit` checks them with the caller's expectations. Both proofs are below
# Core's minimum work, so the best real verdict is DEMO (exit 2); the unit tests (src/tests.rs) reach OK on a zero-work
# image. No engine image is published yet: every call gives the image ids and keys. Usage: test_deposit.sh
# [<zapp-deposit>] (default: cli/target/release). Prints every verdict line and exit code.
Z=$(realpath "${1:-$(dirname "$0")/cli/target/release/zapp-deposit}")
cd "$(dirname "$0")/../.." || exit 1
B=$(dirname "$Z"); D=experimental/libzync-bench/deposit; X=swiftsync-verify/vectors; H=zapps/headers/tests/data/engine4031
CORE=$X/mainnet_1_200_engine.hze; HIST=swiftsync-verify/tests/data/engine200/history.bin
K="--image $(cat $X/mainnet_1_200_engine.image_id) --vk $X/mainnet_1_200_engine.vk"
HK="--headers-image $(cat $H/image_id) --headers-vk $H/vk.bin"
T=f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16
CB=b1fea52486ce0c62bb442b530a3f0132b826c74e473d1f2c220bfa78111c5082 # block 170's coinbase
TIP=4031:00000000f037ad09d0b05ee66b8c1da83030abaf909d2b1bf519c3c7d2cd3fdf
W=$(mktemp -d); trap 'rm -rf $W' EXIT
BAD=0; N=0
run() { local want=$1; shift; echo "== ${*#$B/} (expect exit $want)" | sed "s#$W#<tmp>#g"; "$@" 2>&1 | grep -a 'VERIFY_\|\[FAIL\]\|zapp-deposit' | sed "s#$W#<tmp>#g" | cut -c1-260; local got=${PIPESTATUS[0]}; N=$((N+1)); if [ "$got" = "$want" ]; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi; }
MK="$Z build --core $CORE $K --history $HIST --height 170 --block $D/block_170_raw.txt"
run 0 $MK --txid $T -o $W/core.json
run 0 $MK $HK --txid $T --headers $H/proof.hze --tip-path $H/mmr_path_200_tip4031.txt -o $W/tip.json
run 1 $MK --txid 00${T:2} -o $W/none.json
run 1 $Z build --core $CORE $K --history $HIST --height 171 --block $D/block_170_raw.txt --txid $T -o $W/none.json
run 1 $Z build --core $CORE --history $HIST --height 170 --block $D/block_170_raw.txt --txid $T -o $W/none.json
V="$Z $K"
run 2 $V --txid $T --vout 0 --amount 1000000000 --min-depth 31 $W/core.json
run 2 $V $HK --txid $T --min-depth 6 $W/tip.json
# DEMO is never accepted and its statement is withheld unless asked for
run 2 $V --txid $T --vout 0 --amount 1000000000 --allow-demo $W/core.json
run 2 $V $HK --txid $T --min-depth 6 --no-require-identity $W/tip.json
run 2 $V $HK --txid $T --vout 0 --amount 1000000000 --min-depth 3862 --expect $TIP $W/tip.json
run 1 $V $HK --txid $T --min-depth 3863 --expect $TIP $W/tip.json
run 2 $V $HK --txid $T --min-validated-depth 31 --expect $TIP $W/tip.json
run 1 $V $HK --txid $T --min-validated-depth 32 $W/tip.json
run 1 $V --txid $T --min-depth 6 $W/tip.json
run 1 $V --txid $T --min-depth 32 $W/core.json
run 1 $V --txid $CB $W/core.json
run 1 $V --txid $T --vout 0 --amount 999999999 $W/core.json
run 1 $V --txid $T --vout 1 --script 76a914 $W/core.json
run 1 $V --txid $T --vout 9 $W/core.json
run 1 $V --txid $T --expect 199:${TIP#*:} $W/core.json
run 1 $V $HK --txid $T --max-tip-age 60 $W/tip.json
# the image ids and keys (no engine image is published yet)
run 1 $Z --txid $T --allow-demo --no-require-identity $W/core.json
run 1 $Z --image $(printf 'ab%.0s' $(seq 32)) --vk $X/mainnet_1_200_engine.vk --txid $T --allow-demo --no-require-identity $W/core.json
run 1 $Z --image $(cat $X/mainnet_1_200_engine.image_id) --vk $X/mainnet_1_200_engine_key_flipped.vk --txid $T --allow-demo --no-require-identity $W/core.json
run 1 $V --headers-image $(printf 'ab%.0s' $(seq 32)) --headers-vk $H/vk.bin --txid $T --allow-demo --no-require-identity $W/tip.json
run 1 $V --txid $T --network signet $W/core.json
run 1 $V $W/core.json
run 1 $V --txid $T /nonexistent.json
# the chain identity through the other sources (zapp-common, every zAPP CLI's flags): a node that names the header tip
# as the bundle does (DEMO: below minimum work), one that names another block, gives no chainwork or another (FAIL)
printf '#!/bin/sh\ncase "$1" in getblockhash) echo %s;; getblockheader) echo '"'"'{"chainwork": "%064x"}'"'"';; esac\n' "${TIP#*:}" $((4032 * 0x100010001)) > $W/node; chmod +x $W/node
printf '#!/bin/sh\n[ "$1" = getblockhash ] && echo %s\nexit 0\n' "${TIP#*:}" > $W/nowork; chmod +x $W/nowork
printf '#!/bin/sh\ncase "$1" in getblockhash) echo %s;; getblockheader) echo '"'"'{"chainwork": "00ff"}'"'"';; esac\n' "${TIP#*:}" > $W/otherwork; chmod +x $W/otherwork
printf '#!/bin/sh\necho %064d\n' 1 > $W/badnode; chmod +x $W/badnode
run 2 $V $HK --txid $T --min-depth 6 --bitcoin-cli $W/node $W/tip.json
run 1 $V $HK --txid $T --min-depth 6 --bitcoin-cli $W/badnode $W/tip.json
run 1 $V $HK --txid $T --min-depth 6 --bitcoin-cli $W/nowork $W/tip.json
run 1 $V $HK --txid $T --min-depth 6 --bitcoin-cli $W/otherwork $W/tip.json
head -c 500 /dev/urandom > $W/junk.json; run 1 $V --txid $T $W/junk.json
echo "test_deposit.sh: $((N-BAD))/$N as expected"
[ "$N" -gt 0 ] && [ "$BAD" = 0 ]
