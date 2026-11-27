#!/bin/bash
# swiftsync-verify end-to-end tests on the engine chain's real proof of mainnet 1..200 (vectors/mainnet_1_200_engine.*,
# from tests/data/engine200): DEMO for the valid proof (below Core's minimum chainwork), FAIL for every broken input,
# the argument rules (--image and --vk together; no published engine image yet), step 7 against a stand-in node, and
# the retired hints-file arguments. Run from the repo root after `cargo build --release`. Writes swiftsync-verify/test.out.
cd "$(dirname "$0")/.." || exit 1
BAD=0; N=0
V=swiftsync-verify/target/release/swiftsync-verify; D=swiftsync-verify/vectors
P=$D/mainnet_1_200_engine.hze; ID=$(cat $D/mainnet_1_200_engine.image_id); K="--image $ID --vk $D/mainnet_1_200_engine.vk"
TIP=000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320
T=$(mktemp -d); trap 'rm -rf $T' EXIT
cat > $T/cli <<S
#!/bin/sh
case "\$1" in
  getblockhash) [ "\$MODE" = err ] && { echo "error code: -8" >&2; exit 1; }
                [ "\$MODE" = fork ] && echo 0000000000000000000000000000000000000000000000000000000000000abc || echo $TIP ;;
  getblockheader) echo '{"chainwork":"000000000000000000000000000000000000000000000000000000c900c900c9"}' ;;
esac
S
chmod +x $T/cli
head -c 1000 /dev/urandom > $T/junk.hze
run() { local want=$1; shift; echo "== $* (expect exit $want)" | sed "s#$T#<tmp>#g"; "$@" 2>&1 | grep -a 'VERIFY_\|\[FAIL\]' | sed "s#$T#<tmp>#g"; local got=${PIPESTATUS[0]}; N=$((N+1)); if [ "$got" = "$want" ]; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi; }
run 2 $V $K $P
run 2 env MODE=good $V $K --bitcoin-cli $T/cli $P
run 1 env MODE=fork $V $K --bitcoin-cli $T/cli $P
run 1 env MODE=err $V $K --bitcoin-cli $T/cli $P
run 2 $V $K --expect 200:$TIP $P
run 1 $V $K --expect 200:0000000000000000000000000000000000000000000000000000000000000abc $P
run 1 $V --network signet $K $P
run 1 $V $P
run 1 $V --image $ID $P
run 1 $V --vk $D/mainnet_1_200_engine.vk $P
run 1 $V --image xyz --vk $D/mainnet_1_200_engine.vk $P
run 1 $V --image $ID --vk $D/mainnet_1_200_engine_key_flipped.vk $P
run 1 $V $K $D/mainnet_1_200_engine_proof_flipped.hze
run 1 $V $K $T/junk.hze
run 1 $V $K /nonexistent.hze
run 1 $V $K --now 1231006505 $P
run 1 $V $K --now 99999999999 $P
run 1 $V $K $P $D/signet2600.hints
run 1 $V $K $P -
echo "swiftsync-verify test.sh: $((N-BAD))/$N as expected"
[ "$N" -gt 0 ] && [ "$BAD" = 0 ]
