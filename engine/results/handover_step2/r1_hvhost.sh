#!/bin/bash
# R1-5 #3: rebuild the test-lineage host with the test-only bypass (run under unshare -m)
set -uo pipefail
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
W=/root/hvneg28
mount --bind $W/kit /root/engine-kit && mount --bind $W/src-xv /root/engine-src-xv || { echo MOUNT_FAIL; exit 1; }
t=$(date +%s)
(cd /root/engine-src-xv/prover && RISC0_SKIP_BUILD=1 HAZYNC_BASE=/root/product-base6 HAZYNC_LIBZYNC=1 HAZYNC_TEST_LINEAGE=1 \
   CARGO_BUILD_JOBS=4 nice cargo build --release -p host > $W/host2.log 2>&1) || { echo HOST_FAIL; grep -a "^error" -A6 $W/host2.log | head -20; exit 1; }
echo "HOST_REBUILT $(( $(date +%s)-t )) s"
