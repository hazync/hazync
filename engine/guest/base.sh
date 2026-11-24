#!/bin/bash
# base.sh <base>: the C/C++ consensus sources the engine guest compiles.
#   <base>/bitcoin-core, <base>/coreshim  the product's (scripts/libzync-base.sh: Core v28.0 + 0001 0002 0009 0010
#                                         + libzync/0101), then core-0201-sha-openvm-native.patch (the same SHA-256 on
#                                         OpenVM's native state layout instead of RISC Zero's byte-swapped syscall)
#   <base>/secp256k1                      libsecp256k1 v0.5.1, PRISTINE + secp-0201-openvm-batch-liftx.patch (the batch
#                                         hooks of libzync/0102 at the same call sites and kinds, plus the sqrt hook)
#                                         + secp-0202-sha-openvm-native.patch (libsecp's SHA-256 on OpenVM's extension);
#                                         none of the product's RISC Zero field/group patches (0005 0006 0008 0012 0013)
set -euo pipefail
B=${1:?usage: base.sh <base>}
K=$(cd "$(dirname "$0")" && pwd); R=$(cd "$K/../.." && pwd)
"$R/scripts/libzync-base.sh" "$B"
git -C "$B/bitcoin-core" apply --check "$K/core-0201-sha-openvm-native.patch"
git -C "$B/bitcoin-core" apply "$K/core-0201-sha-openvm-native.patch"
echo "  bitcoin-core <- engine/guest/core-0201-sha-openvm-native.patch (SHA-256 in OpenVM's native layout)"
git -C "$B/secp256k1" checkout -q -- . && git -C "$B/secp256k1" clean -qfd
git -C "$B/secp256k1" apply --check "$K/secp-0201-openvm-batch-liftx.patch"
git -C "$B/secp256k1" apply "$K/secp-0201-openvm-batch-liftx.patch"
git -C "$B/secp256k1" apply --check "$K/secp-0202-sha-openvm-native.patch"
git -C "$B/secp256k1" apply "$K/secp-0202-sha-openvm-native.patch"
echo "  secp256k1 <- engine/guest/secp-0201-openvm-batch-liftx.patch secp-0202-sha-openvm-native.patch (pristine v0.5.1 + engine hooks)"
