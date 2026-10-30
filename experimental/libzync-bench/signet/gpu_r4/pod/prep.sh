#!/bin/bash
# Pod-side: provision, unpack the bundle (repo snapshot + guest + pins + blocks), build the CUDA host around the
# prebuilt signet guest, assert pins. Ends PREP_OK or PREP_FAIL.
bash /workspace/setup.sh > /workspace/setup.log 2>&1 || { echo PREP_FAIL setup; exit 1; }
export PATH=/usr/local/cuda-12.8/bin:$HOME/.cargo/bin:$PATH CUDA_PATH=/usr/local/cuda-12.8 LD_LIBRARY_PATH=/usr/local/cuda-12.8/lib64
for i in $(seq 1 120); do [ -f /workspace/sb.tgz.sha256 ] && break; sleep 5; done
cd /workspace && [ "$(sha256sum sb.tgz | cut -c1-64)" = "$(cut -c1-64 sb.tgz.sha256)" ] || { echo PREP_FAIL bundle; exit 1; }
rm -rf sb && mkdir sb && tar xzf sb.tgz -C sb && cd sb && sha256sum -c --quiet SHA256SUMS || { echo PREP_FAIL sums; exit 1; }
mkdir -p /workspace/hazync && tar xzf /workspace/sb/repo.tgz -C /workspace/hazync
cd /workspace/hazync/prover && T0=$SECONDS
(cargo --version; nvcc --version | tail -1) > /workspace/toolchain.txt 2>&1
HAZYNC_GUEST_ELF=/workspace/sb/method.bin HAZYNC_GUEST_PIN_DIR=/workspace/sb/pins HAZYNC_LIBZYNC=1 HAZYNC_NETWORK=$(cat /workspace/sb/NETWORK) \
  cargo build --release --features cuda -p host > /workspace/build.log 2>&1 || { echo PREP_FAIL build; tail -n 15 /workspace/build.log; exit 1; }
echo "built in $((SECONDS-T0)) s"
ldd target/release/host | grep -qi cuda || { echo PREP_FAIL nocuda; exit 1; }
target/release/host method-id | grep -q "$(cat /workspace/sb/pins/METHOD_ID | grep -v '^#' | head -1)" || { echo PREP_FAIL methodid; exit 1; }
echo PREP_OK
