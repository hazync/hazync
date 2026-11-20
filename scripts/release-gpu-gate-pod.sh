#!/bin/bash
# Runs ON A RENTED POD, in /workspace, for scripts/release_gpu_gate.py (never run it anywhere else). Real proofs, no
# dev mode, with the EXACT staged release CUDA host:
#
#   /workspace/host             the staged hazync-host-x86_64-linux-gnu-cuda (pushed by the gate)
#   /workspace/want_host_sha256 its sha256, as the operator gave it to the gate
#   /workspace/gate/            the fixtures (prover/testdata/libzync/gpu-gate: GATE, SHA256SUMS, leaves, states)
#
# For each line of gate/GATE (<height> <block hash> <leaf> <anchor> <state>) it proves the leaf exactly as a board
# prover does: HAZYNC_LEAF_BUNDLE=<leaf> host libzync-prove-range h h unused leaf out/p<h>. Each run is timed into
# out/proves.tsv. Nothing is judged here beyond "did it run": the receipts are verified on the maintainer's machine,
# with the staged verifier, after harvest. Ends by writing ONE word (and a reason) to out/RESULT:
#   PROVED | NO_GPU | HOST_SHA_BAD | FIXTURE_BAD | METHOD_ID_BAD | PROVE_FAILED <h> rc=<n>
# No retry: a release gate that retries hides the fault it is there to see (#119-class faults included).
set -u
W=/workspace; O=$W/out
mkdir -p "$O"; cd "$W" || exit 1
unset RISC0_DEV_MODE HAZYNC_CHUNKS HAZYNC_PROFILE_LAYOUT HAZYNC_SEG_PO2 HAZYNC_NO_MEMPOOL HAZYNC_RESUME
finish() { echo "$*" > "$O/RESULT"; exit 0; }
date -u +%FT%TZ > "$O/started"
nvidia-smi --query-gpu=name,compute_cap,driver_version,memory.total --format=csv,noheader > "$O/gpu.txt" 2>&1 \
    || finish NO_GPU
# GeForce cards on RunPod need the image's CUDA forward-compat libraries out of the way (sponsor_bot.BOOT_SH).
nvidia-smi --query-gpu=name --format=csv,noheader | grep -q GeForce \
    && for d in /usr/local/cuda*/compat; do [ -d "$d" ] && mv "$d" "${d}.disabled"; done
ldconfig 2>/dev/null
sha256sum host | cut -c1-64 > "$O/host_sha256"
[ "$(cat "$O/host_sha256")" = "$(tr -dc '0-9a-f' < want_host_sha256)" ] || finish HOST_SHA_BAD
(cd gate && sha256sum -c --quiet SHA256SUMS) > "$O/fixtures.txt" 2>&1 || finish FIXTURE_BAD
chmod +x host
timeout 120 ./host method-id > "$O/method_id.txt" 2>&1 < /dev/null || finish METHOD_ID_BAD
while read -r h _hash leaf _anchor _state; do
    case "$h" in ''|\#*) continue ;; esac
    rm -rf "$O/p$h"
    t0=$(date +%s.%N)
    ( cd "$O" && HAZYNC_LEAF_BUNDLE="$W/gate/$leaf" timeout "${PROVE_TIMEOUT_S:-2400}" \
        "$W/host" libzync-prove-range "$h" "$h" unused leaf "$O/p$h" ) > "$O/prove_$h.log" 2>&1 < /dev/null
    rc=$?
    t1=$(date +%s.%N)
    printf '%s\t%s\t%s\n' "$h" "$rc" "$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.1f", b-a}')" >> "$O/proves.tsv"
    [ "$rc" = 0 ] || finish "PROVE_FAILED $h rc=$rc"
done < gate/GATE
finish PROVED
