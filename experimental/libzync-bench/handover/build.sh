#!/bin/bash
# build.sh <tag> <out dir> [env...]: one guest + host from this worktree, always from a clean guest (a reused guest has
# produced stale image ids twice). `pin <hex words> <first height>` in place of env pins a predecessor for this build only.
T=$1; D=$2; shift 2
cd "$(dirname "$0")/../../../prover" || exit 1
rm -rf target/riscv-guest
env HAZYNC_BASE=$HOME/hzb-r7 HAZYNC_LIBZYNC=1 "$@" cargo build --profile fasthost -j2 -p host > "$D/build_$T.log" 2>&1; e=$?
echo "REAL_EXIT=$e" >> "$D/build_$T.log"
[ $e = 0 ] && cp target/fasthost/host "$D/host_$T" && echo "METHOD_ID $("$D/host_$T" method-id 2>/dev/null | head -1)" >> "$D/build_$T.log"
echo BUILD_DONE >> "$D/build_$T.log"; exit $e
