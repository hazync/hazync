#!/bin/bash
# End-to-end test of the deposit-proof zyncAPP CLI zapp-deposit (docs/libzync/DEPOSIT_ZAPP.md) on real receipts: `zapp-deposit build` (was zync-deposit) builds
# bundles for block 170's Satoshi-to-Hal transaction against the core proof of mainnet 0..200 (8888d59c), alone, bound to
# the header chain 0..200 and to the whole header chain 0..975,278 (de54b94f); `zapp-deposit` (was swiftsync-verify deposit) checks them with
# the caller's expectations. Usage: test_deposit.sh [<zapp-deposit>] (default: target/release, after `cargo build --release` here). Prints every
# verdict line and exit code; the unit tests (src/tests.rs) hold the finer refusals.
Z=$(realpath "${1:-$(dirname "$0")/target/release/zapp-deposit}")
cd "$(dirname "$0")/../.." || exit 1
B=$(dirname "$Z"); D=experimental/libzync-bench/deposit; H=experimental/libzync-bench/headers
CORE=swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt
T=f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16
CB=b1fea52486ce0c62bb442b530a3f0132b826c74e473d1f2c220bfa78111c5082 # block 170's coinbase
TIP=975278:00000000000000000001d951592f7a06f0d24697e4f9c362ae173fced55583e3
W=$(mktemp -d); trap 'rm -rf $W' EXIT
BAD=0; N=0
run() { local want=$1; shift; echo "== ${*#$B/} (expect exit $want)" | sed "s#$W#<tmp>#g"; "$@" 2>&1 | grep -a 'VERIFY_\|\[FAIL\]\|zapp-deposit' | sed "s#$W#<tmp>#g" | cut -c1-260; local got=${PIPESTATUS[0]}; N=$((N+1)); if [ "$got" = "$want" ]; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi; }
MK="$Z build --core $CORE --history $D/history_0_200.bin --height 170 --block $D/block_170_raw.txt"
run 0 $MK --txid $T -o $W/core.json
run 0 $MK --txid $T --headers $H/hdr_0_100.rcpt --headers $H/hdr_101_200.rcpt --headers-file $H/h0_200.txt -o $W/h200.json
run 0 $MK --txid $T --headers $H/headers_0_tip.rcpt --tip-path $D/mmr_path_200_tip975278.txt -o $W/tip.json
run 1 $MK --txid 00${T:2} -o $W/none.json
run 1 $Z build --core $CORE --history $D/history_0_200.bin --height 171 --block $D/block_170_raw.txt --txid $T -o $W/none.json
V="$Z"
run 2 $V --txid $T --vout 0 --amount 1000000000 --min-depth 31 $W/core.json
run 2 $V --txid $T --min-depth 6 $W/h200.json
run 0 $V --txid $T --vout 0 --amount 1000000000 --min-depth 6 --expect $TIP $W/tip.json
run 0 $V --txid $T --min-validated-depth 31 --expect $TIP $W/tip.json
run 1 $V --txid $T --min-validated-depth 32 $W/tip.json
run 1 $V --txid $T --min-depth 32 $W/core.json
run 1 $V --txid $CB $W/core.json
run 1 $V --txid $T --vout 0 --amount 999999999 $W/core.json
run 1 $V --txid $T --vout 1 --script 76a914 $W/core.json
run 1 $V --txid $T --vout 9 $W/core.json
run 1 $V --txid $T --expect 975278:${TIP#*:} $W/core.json
run 1 $V --txid $T --max-tip-age 60 $W/tip.json
run 1 env RISC0_DEV_MODE=1 $V --txid $T $W/core.json
run 1 $V --txid $T --network signet $W/core.json
run 1 $V $W/core.json
run 1 $V --txid $T /nonexistent.json
# the chain identity through the other sources (zapp-common, every zAPP CLI's flags): a node that names the header tip
# as the bundle does (OK), one that names another block (FAIL)
printf '#!/bin/sh\n[ "$1" = getblockhash ] && echo %s\nexit 0\n' "${TIP#*:}" > $W/node; chmod +x $W/node
printf '#!/bin/sh\necho %064d\n' 1 > $W/badnode; chmod +x $W/badnode
run 0 $V --txid $T --min-depth 6 --bitcoin-cli $W/node $W/tip.json
run 1 $V --txid $T --min-depth 6 --bitcoin-cli $W/badnode $W/tip.json
head -c 500 /dev/urandom > $W/junk.json; run 1 $V --txid $T $W/junk.json
echo "test_deposit.sh: $((N-BAD))/$N as expected"
[ "$N" -gt 0 ] && [ "$BAD" = 0 ]
