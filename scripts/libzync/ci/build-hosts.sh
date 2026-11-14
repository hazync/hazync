#!/bin/bash
# libzync CI: build the test hosts INSIDE the reproducible container (reproduce/libzync.Dockerfile's image, whose build
# stage left the published guest ELFs in /root/libzync-elf), so the soundness jobs test what is published:
#   host_m    mainnet host embedding the published mainnet ELF (the id in reproduce/LIBZYNC_IDS)
#   host_sig  signet host embedding the published signet ELF
#   host_reg  regtest image O (built here; regtest is not published)
#   host_regN regtest image N: O pinned as its predecessor at height 101 (the handover test; the pin is not committed)
#   host_m0   TEST VARIANT of the mainnet image (HAZYNC_TEST_LINEAGE=1: lineage/test.rs, empty, instead of the published
#             lineage), for the tests that prove mainnet from genesis: the published image pins its predecessor below
#             its handover height, so by design it never proves those blocks. Its id is in no pin table (checked).
#   host_mN   TEST VARIANT: host_m0 pinned in lineage/test.rs at height 11 (the board rehearsal's O -> N; not committed)
# Every host is stripped (the tests need no symbols); ids.txt lists what was built.
# usage (in the container): build-hosts.sh <out dir>
set -euo pipefail
OUT=${1:?out dir}; mkdir -p "$OUT"
REPO=/hazync-zkvm; BASE=/root/hazync-libzync-base; P=$REPO/prover
cd "$P"
# the pinned host toolchain (prover/rust-toolchain.toml): it reaches the guest image ids (stable 1.100.0, 2026-11-12)
RUSTUP_TOOLCHAIN=$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml); export RUSTUP_TOOLCHAIN
[ -n "$RUSTUP_TOOLCHAIN" ] || { echo "no pinned channel in prover/rust-toolchain.toml" >&2; exit 1; }
for v in $(compgen -e | grep '^HAZYNC_'); do unset "$v"; done
ids() { "$1" method-id | head -1 | awk '{print $NF}'; }
pins() {  # pins <net> <dir>: the published pins for net, from reproduce/LIBZYNC_IDS
  local l; l=$(grep "^libzync-product $1 " "$REPO/reproduce/LIBZYNC_IDS"); mkdir -p "$2"
  echo "$l" | awk '{print $4}' > "$2/METHOD_ID"; echo "$l" | awk '{print $6}' > "$2/GUEST_ELF_SHA256"
  echo "libzync=1 assumevalid=0 network=$1 coinlog=1 trie=1 utxohash=1" > "$2/PROFILE"
}
build() {  # build <name> <env...>
  local n=$1; shift
  rm -rf target/riscv-guest
  env "$@" cargo build --locked --profile fasthost -p host -j"$(nproc)" >&2
  cp target/fasthost/host "$OUT/$n"; strip "$OUT/$n"
  echo "$n $(ids "$OUT/$n")" | tee -a "$OUT/ids.txt"
}
: > "$OUT/ids.txt"
for net in mainnet signet; do
  pins $net /tmp/pins_$net
  n=host_m; [ $net = signet ] && n=host_sig
  netenv=(); [ $net = signet ] && netenv=(HAZYNC_NETWORK=signet)
  build $n HAZYNC_BASE=$BASE HAZYNC_LIBZYNC=1 "${netenv[@]}" HAZYNC_GUEST_ELF=/root/libzync-elf/$net.bin HAZYNC_GUEST_PIN_DIR=/tmp/pins_$net
  grep -q "^$n $(cat /tmp/pins_$net/METHOD_ID)$" "$OUT/ids.txt" || { echo "FATAL: $n is not the published $net image" >&2; exit 1; }
done
build host_reg HAZYNC_BASE=$BASE HAZYNC_LIBZYNC=1 HAZYNC_NETWORK=regtest
# image N: O pinned at 101 in the regtest lineage, built, then the pin reverted (git checkout)
L=methods/guest/src/lineage/regtest_coinlog.rs
W=$("$OUT/host_reg" method-id | sed -n 's/.*u32x8 *\[\(.*\)\]/\1/p')
sed -i "s/^pub const LINEAGE: &\[crate::handover_pin::Handover\] = &\[\];/pub const LINEAGE: \&[crate::handover_pin::Handover] = \&[crate::handover_pin::Handover { image: [$W], first_height: 101, verifier: crate::handover_pin::SAME_ZKVM }];/" $L
grep -q 'first_height: 101' $L || { echo "FATAL: the regtest pin edit did not apply" >&2; exit 1; }
build host_regN HAZYNC_BASE=$BASE HAZYNC_LIBZYNC=1 HAZYNC_NETWORK=regtest
git -C "$REPO" checkout -- "prover/$L" 2>/dev/null || sed -i "s/^pub const LINEAGE: .*first_height: 101.*\];/pub const LINEAGE: \&[crate::handover_pin::Handover] = \&[];/" $L
# the mainnet test variants (O = empty test lineage, N = O pinned at 11), from source like the regtest pair
build host_m0 HAZYNC_BASE=$BASE HAZYNC_LIBZYNC=1 HAZYNC_TEST_LINEAGE=1
test_variant() {  # test_variant <host>: says TEST_VARIANT, and its id is in no pin table (published ids, lineage files)
  local h=$OUT/$1 m l id w
  m=$("$h" method-id); l=$("$h" libzync-lineage); id=$(echo "$m" | head -1 | awk '{print $NF}')
  w=$(echo "$m" | sed -n 's/.*u32x8 *\[\(.*\)\]/\1/p')
  grep -q '^TEST_VARIANT ' <<< "$m" || { echo "FATAL: $1 method-id does not print TEST_VARIANT" >&2; exit 1; }
  grep -q '^TEST_VARIANT ' <<< "$l" || { echo "FATAL: $1 libzync-lineage does not print TEST_VARIANT" >&2; exit 1; }
  if [ -z "$w" ] || grep -rqi "$id" "$REPO/reproduce" || grep -qF "$w" methods/guest/src/lineage/*.rs; then
    echo "FATAL: $1's id $id is a pinned id (or unreadable): a test variant must not be confusable with a published image" >&2; exit 1; fi
}
test_variant host_m0
grep -q '^HANDOVER none$' <<< "$("$OUT/host_m0" libzync-lineage)" || { echo "FATAL: host_m0 pins a handover" >&2; exit 1; }
LT=methods/guest/src/lineage/test.rs; cp $LT /tmp/lineage_test.rs
W=$("$OUT/host_m0" method-id | sed -n 's/.*u32x8 *\[\(.*\)\]/\1/p')
sed -i "s/^pub const LINEAGE: &\[crate::handover_pin::Handover\] = &\[\];/pub const LINEAGE: \&[crate::handover_pin::Handover] = \&[crate::handover_pin::Handover { image: [$W], first_height: 11, verifier: crate::handover_pin::SAME_ZKVM }];/" $LT
grep -q 'first_height: 11' $LT || { echo "FATAL: the test lineage pin edit did not apply" >&2; exit 1; }
build host_mN HAZYNC_BASE=$BASE HAZYNC_LIBZYNC=1 HAZYNC_TEST_LINEAGE=1
cp /tmp/lineage_test.rs $LT
test_variant host_mN
grep -q "^HANDOVER k=0 image=$(ids "$OUT/host_m0") first_height=11 " <<< "$("$OUT/host_mN" libzync-lineage)" \
  || { echo "FATAL: host_mN does not pin host_m0 at 11" >&2; exit 1; }
# Core's own test vectors as guest cases, from the Core tree the guest is built from
python3 "$REPO/scripts/libzync/core_vectors.py" "$BASE/bitcoin-core" "$OUT/core_vectors.json" > "$OUT/core_vectors.log" 2>&1
python3 "$REPO/scripts/libzync/core_vectors.py" "$BASE/bitcoin-core" "$OUT/core_vectors_base.json" --no-variants >> "$OUT/core_vectors.log" 2>&1
tail -2 "$OUT/core_vectors.log"
ls -la "$OUT"
# the host's own unit tests (the host is libzync-only)
if env HAZYNC_BASE=$BASE HAZYNC_LIBZYNC=1 RISC0_SKIP_BUILD=1 cargo test --profile fasthost -p host -j"$(nproc)" > "$OUT/host_unit.log" 2>&1; then
  echo "HOST_UNIT_OK $(grep -a '^test result' "$OUT/host_unit.log" | awk '{p+=$4; f+=$6} END{print p" passed, "f" failed"}')" | tee -a "$OUT/ids.txt"
else
  echo "HOST_UNIT_FAILED" | tee -a "$OUT/ids.txt"; grep -a 'FAILED\|panicked' "$OUT/host_unit.log" | head -20 >&2
fi
