#!/bin/bash
# assemble.sh <hazync checkout>: build prover/methods/ovmagg from that checkout's RISC Zero guest + this kit.
set -euo pipefail
H=${1:?hazync checkout}; K=$(cd "$(dirname "$0")" && pwd); D=$H/prover/methods/ovmagg
rm -rf "$D"; cp -r "$H/prover/methods/guest" "$D"; rm -rf "$D/target" "$D/Cargo.lock"
cp "$K/batch_ecdsa_ovm.rs" "$D/src/"
python3 "$K/edit_main.py" "$D/src/main.rs"
echo 'openvm::init!();' >> "$D/src/main.rs"
sed "s|COMPAT|$K/compat|g" "$K/Cargo.toml.ovm" > "$D/Cargo.toml"
cp "$K/build.rs.ovm" "$D/build.rs"; cp "$K/../ovm-core/openvm.toml" "$D/openvm.toml"
R=$(ls -d ~/.cargo/registry/src/*/risc0-zkvm-3.1.0 | head -1)
mkdir -p "$K/compat/risc0-zkvm/src/serde"
for f in mod.rs deserializer.rs serializer.rs err.rs; do
  sed -e 's/use risc0_zkvm_platform::WORD_SIZE;/const WORD_SIZE: usize = 4;/' "$R/src/serde/$f" > "$K/compat/risc0-zkvm/src/serde/$f"
done
echo "assembled $D"
