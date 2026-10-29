#!/bin/bash
# Pod-side: provision, wait for the bundle, build the CUDA host, assert pins, execute check. Ends PREP_OK or PREP_FAIL.
bash /workspace/setup.sh > /workspace/setup.log 2>&1 || { echo PREP_FAIL setup; exit 1; }
export PATH=/usr/local/cuda-12.8/bin:$HOME/.cargo/bin:$PATH CUDA_PATH=/usr/local/cuda-12.8 LD_LIBRARY_PATH=/usr/local/cuda-12.8/lib64
cd /workspace/hazync && git log --oneline -1
for i in $(seq 1 120); do [ -f /workspace/libzync-bundle.tgz.sha256 ] && break; sleep 5; done
cd /workspace && [ "$(sha256sum libzync-bundle.tgz | cut -c1-64)" = "$(cut -c1-64 libzync-bundle.tgz.sha256)" ] || { echo PREP_FAIL bundle; exit 1; }
mkdir -p t && tar xzf libzync-bundle.tgz -C t && rm -rf libzync-bundle && mv t/libzync-bundle . && rmdir t
cd libzync-bundle && sha256sum -c --quiet SHA256SUMS || { echo PREP_FAIL sums; exit 1; }
cd /workspace/hazync/prover && T0=$SECONDS
HAZYNC_GUEST_ELF=/workspace/libzync-bundle/method.bin cargo build --release --features cuda -p host > /workspace/build.log 2>&1 || { echo PREP_FAIL build; exit 1; }
echo "built in $((SECONDS-T0)) s"
ldd target/release/host | grep -qi cuda || { echo PREP_FAIL nocuda; exit 1; }
target/release/host method-id | grep -q 5e460bf309a5d1ac || { echo PREP_FAIL methodid; exit 1; }
. /workspace/lzenv.sh
HAZYNC_CHUNKS=1 HAZYNC_PROFILE_EXEC=1 HAZYNC_BATCH_HINTS_DIR=/workspace/libzync-bundle/h1 target/release/host chunk-profile > /workspace/exec1.log 2>&1
grep -q 'journal sha256 98e2652d5184e30a' /workspace/exec1.log && grep -qE 'cycles +326652314' /workspace/exec1.log || { echo PREP_FAIL exec; exit 1; }
echo PREP_OK
