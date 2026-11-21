#!/bin/bash
# run.sh <out dir>: re-make every profile in this directory from the PUBLISHED release hosts and repo fixtures.
#
# Downloads the v0.22.1 (Hazync v1, CORE guest 37987b85) and v0.27.0 (v2 / libzync, image 46666b6f) Linux hosts,
# checks each against its release's PGP-signed SHA256SUMS.txt, then profiles block 962,000 in EXECUTE mode (no
# proving, no GPU): v1 as one whole-block chunk; v2 as one whole-block chunk + its trie update + its aggregate.
# One run at a time; each needs ~0.55-0.65 GB RAM. Wall clock on a laptop: v1 ~11 min, v2 ~4 min.
# Needs: gh (or curl), gpg with the maintainer key 777FE81F8CC077FD3D08055E852C2B3190F5B928 (SECURITY.md), python3.
set -euo pipefail
OUT=$(realpath -m "${1:?usage: run.sh <out dir>}"); mkdir -p "$OUT"
HERE=$(cd "$(dirname "$0")" && pwd); REPO=$(cd "$HERE/../../.." && pwd)
BLOCK=$REPO/prover/block_962000.json            # the same blob (23dba40c) at v0.22.1, v0.27.0 and here
TXC=$REPO/experimental/libzync-bench/cost/txcounts.txt

fetch() {  # fetch <tag> <dir>: the Linux CPU host + signed sums, verified
    mkdir -p "$2"
    ( cd "$2"
      [ -f hazync-host-x86_64-linux-gnu ] || gh release download "$1" -R hazync/hazync \
          -p hazync-host-x86_64-linux-gnu -p SHA256SUMS.txt -p SHA256SUMS.txt.asc -D .
      gpg --verify SHA256SUMS.txt.asc SHA256SUMS.txt
      sha256sum -c --ignore-missing SHA256SUMS.txt
      chmod +x hazync-host-x86_64-linux-gnu )
}
fetch v0.22.1 "$OUT/v0.22.1"
fetch v0.27.0 "$OUT/v0.27.0"
V1=$OUT/v0.22.1/hazync-host-x86_64-linux-gnu
V2=$OUT/v0.27.0/hazync-host-x86_64-linux-gnu
"$V1" method-id | grep -q 37987b85ec665970 || { echo "v1 host is not image 37987b85"; exit 1; }
"$V2" method-id | grep -q 46666b6f7e3d1c1c || { echo "v2 host is not image 46666b6f"; exit 1; }

# v1: the whole block as ONE chunk (mode 4). The release guest is built with liftx-hint, so HAZYNC_LIFTX_HINT=1 is
# required at run time (v0.22.1 main.rs, write_chunk_inputs); HAZYNC_ECMULT_WINDOW is build-time only and has no
# effect here (kept because it is the recorded recipe). v0.22.1 has no HAZYNC_PROFILE_LAYOUT, so chunk-profile
# executes the chunk twice (count- and cost-packed, the same single chunk); the second run's profile is the file.
HAZYNC_LIFTX_HINT=1 HAZYNC_ECMULT_WINDOW=21 HAZYNC_BLOCK=$BLOCK HAZYNC_CHUNKS=1 HAZYNC_PROFILE_EXEC=1 \
  RISC0_PPROF_OUT=$OUT/v1_block962000_chunk.pb "$V1" chunk-profile > "$OUT/v1_chunk.log" 2>&1

# v2: hints first (batch-ECDSA verdicts + ECMH, collected by the guest itself), for the SAME chunking (1 chunk).
export HAZYNC_BLOCK=$BLOCK HAZYNC_CHUNKS=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_COINLOG_TXCOUNTS=$TXC
HAZYNC_HINT_THREADS=1 "$V2" libzync-hints "$OUT/v2_block962000_c1.lzb" > "$OUT/v2_hints.log" 2>&1
export HAZYNC_LZ_HINTS=$OUT/v2_block962000_c1.lzb
# the chunk, with its journal digest
HAZYNC_PROFILE_EXEC=1 RISC0_PPROF_OUT=$OUT/v2_block962000_chunk.pb "$V2" chunk-profile > "$OUT/v2_chunk.log" 2>&1
# agg-exec-fake executes, in order: the chunk (.0), the coin-log collect run (.1, host-side, never proved), the
# trie update (.2, mode 17) and the aggregate (.3, mode 5). Each overwrites RISC0_PPROF_OUT; capture keeps them all.
mkdir -p "$OUT/cap"
RISC0_PPROF_OUT=$OUT/cap/live.pb python3 -I "$HERE/capture_pprof.py" "$OUT/cap/live.pb" "$OUT/cap/v2_agg" -- \
  "$V2" agg-exec-fake > "$OUT/v2_agg.log" 2>&1
cp "$OUT/cap/v2_agg.2.pb" "$OUT/v2_block962000_trie.pb"
cp "$OUT/cap/v2_agg.3.pb" "$OUT/v2_block962000_aggregate.pb"

# decode
P="python3 -I $HERE/prof_split.py"
{ $P "$OUT/v1_block962000_chunk.pb"
  $P "$OUT/v2_block962000_chunk.pb" "$OUT/v2_block962000_trie.pb" "$OUT/v2_block962000_aggregate.pb"
  $P --sum "v2 block 962,000" "$OUT/v2_block962000_chunk.pb" "$OUT/v2_block962000_trie.pb" "$OUT/v2_block962000_aggregate.pb"
} > "$OUT/decoded.txt"
grep -h 'journal sha256\|cycles  \|executed:\|validation cycles' "$OUT"/v1_chunk.log "$OUT"/v2_chunk.log "$OUT"/v2_agg.log
echo "RUN_DONE -> $OUT/decoded.txt"
