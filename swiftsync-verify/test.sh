#!/bin/bash
# swiftsync-verify end-to-end tests on the real round-4 signet receipt (image c3501ac9, retired design B: its journal
# predates ChainState v2, so EVERY case must now FAIL) plus corrupted inputs and the retired hints-file arguments. The OK
# / DEMO cases come back with receipts of the frozen product image. Run from the repo root after `cargo build --release`.
# Writes swiftsync-verify/test.out. Every case prints its verdict line and exit code.
cd "$(dirname "$0")/.." || exit 1
BAD=0; N=0
V=swiftsync-verify/target/release/swiftsync-verify; E=experimental/libzync-bench/signet; G=$E/gpu_r4
OLD=c3501ac900743bbc911c8f6dedcd71f1ee53fe4bea4f1cbc9726324478e63dae
T=$(mktemp -d); trap 'rm -rf $T' EXIT
cat > $T/cli <<'S'
#!/bin/sh
case "$1" in
  getblockhash) [ "$MODE" = err ] && { echo "error code: -8" >&2; exit 1; }
                [ "$MODE" = fork ] && echo 0000000000000000000000000000000000000000000000000000000000000abc || echo 00000127dfa07c0fd2f2fe1e22a0fea15c34cabd6204b1faac15a04ba28be6f3 ;;
  getblockheader) echo '{"chainwork":"0000000000000000000000000000000000000000000000000000000432982f4d"}' ;;
esac
S
chmod +x $T/cli
head -c 1000 /dev/urandom > $T/junk.rcpt
run() { local want=$1; shift; echo "== $* (expect exit $want)"; "$@" 2>&1 | grep -a 'VERIFY_\|\[FAIL\]' | sed "s#$T#<tmp>#g"; local got=${PIPESTATUS[0]}; N=$((N+1)); if [ "$got" = "$want" ]; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi; }
A="--network signet --product --image $OLD --anchor $G/anchor_effective.json"
R=$G/block_2600.rcpt
run 1 $V $A $R
run 1 env MODE=good $V $A --bitcoin-cli $T/cli $R
run 1 env MODE=fork $V $A --bitcoin-cli $T/cli $R
run 1 $V --network signet --product $R
run 1 $V --network signet --coinlog --image $OLD $R
run 1 $V --network signet --image $OLD --anchor $G/anchor_effective.json $R
run 1 $V --network mainnet --product --image $OLD --anchor $G/anchor_effective.json $R
run 1 env RISC0_DEV_MODE=1 $V $A $R
run 1 $V $A $T/junk.rcpt
run 1 $V $A /nonexistent.rcpt
run 1 $V --network signet --product --image xyz $R
run 1 $V $A --now 99999999999 $R
run 1 $V $A $R $E/signet2600.hints
run 1 $V $A $R -
echo "swiftsync-verify test.sh: $((N-BAD))/$N as expected"
[ "$N" -gt 0 ] && [ "$BAD" = 0 ]
