#!/bin/bash
# swiftsync-verify end-to-end tests on the real round-4 signet receipt (image c3501ac9, from a supplied 2,500 anchor,
# so the best verdict is DEMO) plus corrupted inputs. Run from the repo root after `cargo build --release`.
# Writes swiftsync-verify/test.out. Every case prints its verdict line and exit code.
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
python3 - $E/signet2600.hints $T <<'P'
import sys
b=open(sys.argv[1],'rb').read(); T=sys.argv[2]
open(T+'/trail.hints','wb').write(b+b'\x00')
x=bytearray(b); x[-1]^=0x80; open(T+'/flip.hints','wb').write(bytes(x))
P
head -c 1000 /dev/urandom > $T/junk.rcpt
run() { local want=$1; shift; echo "== $* (expect exit $want)"; "$@" 2>&1 | grep -a 'VERIFY_\|\[FAIL\]' | sed "s#$T#<tmp>#g"; local got=${PIPESTATUS[0]}; echo "exit=$got $([ $got = $want ] && echo PASS || echo WRONG)"; }
A="--network signet --image $OLD --anchor $G/anchor_effective.json"
run 2 $V $A $G/block_2600.rcpt $E/signet2600.hints
run 2 env MODE=good $V $A --bitcoin-cli $T/cli $G/block_2600.rcpt $E/signet2600.hints
run 1 env MODE=fork $V $A --bitcoin-cli $T/cli $G/block_2600.rcpt $E/signet2600.hints
run 1 env MODE=err $V $A --bitcoin-cli $T/cli $G/block_2600.rcpt $E/signet2600.hints
run 1 $V --network signet $G/block_2600.rcpt $E/signet2600.hints
run 1 $V --network signet --image $OLD $G/block_2600.rcpt $E/signet2600.hints
run 1 $V $A $G/block_2600.rcpt $T/trail.hints
run 1 $V $A $G/block_2600.rcpt $T/flip.hints
run 1 $V $A $G/block_2600.rcpt experimental/libzync-bench/signet/signet3000.hints
run 1 $V --network mainnet --image $OLD --anchor $G/anchor_effective.json $G/block_2600.rcpt $E/signet2600.hints
run 1 env RISC0_DEV_MODE=1 $V $A $G/block_2600.rcpt $E/signet2600.hints
run 1 $V $A $T/junk.rcpt $E/signet2600.hints
run 1 $V $A /nonexistent.rcpt $E/signet2600.hints
run 1 $V $A $G/block_2600.rcpt
run 1 $V --network signet --image xyz $G/block_2600.rcpt $E/signet2600.hints
