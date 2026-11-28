#!/bin/bash
# C8 step 4 checks on hazync-coord, part 1: two checkouts under /root/c8s4 (base = engine/openvm 7620ec78, the
# parent of c8-step4; new = c8-step4 at $1), hzengine built once (engine/host is the same in both), the base host built
# as the board dry run builds it (RISC0_SKIP_BUILD=1: no RISC Zero guest, METHOD_ID zero), the new host plain.
# <= 4 cores, nice. Usage: setup.sh <new commit>
set -u
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
N=${1:?commit}; R=/root/c8s4; export CARGO_BUILD_JOBS=4
t(){ local s=$(date +%s); "$@"; local rc=$?; echo "STEP rc=$rc secs=$(( $(date +%s)-s )) :: $*"; return $rc; }
mkdir -p $R
if [ ! -d $R/base ]; then t cp -a /root/engine-src $R/base; fi
cd $R/base && git fetch -q origin engine/openvm c8-step4 && git checkout -q -f 7620ec78 && git log --oneline -1 | cut -c1-80
[ -d $R/base/engine/host/target ] || t cp -a /root/engine-kit/engine/host/target $R/base/engine/host/
if [ ! -d $R/new ]; then git worktree add -f $R/new $N > /dev/null 2>&1; fi
cd $R/new && git checkout -q -f $N && git log --oneline -1 | cut -c1-80
echo "== hzengine (engine/host at 7620ec78)"
t bash -c "cd $R/base/engine/host && nice cargo build --release -q > $R/hz_build.log 2>&1" || { grep -a '^error' -A6 $R/hz_build.log | head; exit 1; }
echo "== base host (7620ec78, RISC0_SKIP_BUILD=1)"
t bash -c "cd $R/base/prover && RISC0_SKIP_BUILD=1 HAZYNC_BASE=/root/product-base6 HAZYNC_LIBZYNC=1 nice cargo build --release -q -p host > $R/base_build.log 2>&1" || { grep -a 'error' -A6 $R/base_build.log | head -30; exit 1; }
echo "== new host ($N)"
t bash -c "cd $R/new/prover && HAZYNC_LIBZYNC=1 nice cargo build --release -q -p host > $R/new_build.log 2>&1" || { grep -a 'error' -A6 $R/new_build.log | head -30; exit 1; }
ls -la $R/base/prover/target/release/host $R/new/prover/target/release/host $R/base/engine/host/target/release/hzengine
echo C8S4_SETUP_DONE
