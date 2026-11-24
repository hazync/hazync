#!/bin/bash
# base.sh <base>: the C/C++ consensus sources the engine guest compiles.
#   <base>/bitcoin-core, <base>/coreshim  exactly the product's (scripts/libzync-base.sh: Core v28.0 + 0001 0002 0009 0010
#                                         + libzync/0101)
#   <base>/secp256k1                      libsecp256k1 v0.5.1, PRISTINE + secp-0201-openvm-batch-liftx.patch (the batch
#                                         hooks of libzync/0102 at the same call sites and kinds, plus the sqrt hook);
#                                         none of the product's RISC Zero field/group patches (0005 0006 0008 0012 0013)
set -euo pipefail
B=${1:?usage: base.sh <base>}
K=$(cd "$(dirname "$0")" && pwd); R=$(cd "$K/../.." && pwd)
"$R/scripts/libzync-base.sh" "$B"
git -C "$B/secp256k1" checkout -q -- . && git -C "$B/secp256k1" clean -qfd
git -C "$B/secp256k1" apply --check "$K/secp-0201-openvm-batch-liftx.patch"
git -C "$B/secp256k1" apply "$K/secp-0201-openvm-batch-liftx.patch"
echo "  secp256k1 <- engine/guest/secp-0201-openvm-batch-liftx.patch (pristine v0.5.1 + engine hooks)"
