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
#
# --deadman <seconds>: the POD-SIDE cap (scripts/release_gpu_gate.py arms it first, right after SSH comes up). After
# <seconds> the pod removes ITSELF, so a laptop that crashed, lost power or was SIGKILLed cannot leave it billing:
#   1. delete through RunPod's API with the pod-scoped key RunPod injects (RUNPOD_POD_ID, RUNPOD_API_KEY: "Pod-scoped
#      API key", docs.runpod.io/pods/references/environment-variables): runpodctl if the image has it, else REST;
#   2. else STOP it (RunPod: stopping "releases the GPU"; a stopped pod still bills its volume disk until deleted);
#   3. else end the container (kill PID 1). ⚠ What RunPod bills for a pod whose container exited is not documented
#      as such; assume the GPU may still be held until `scripts/release-gpu-gate.sh sweep --terminate` runs.
# The variables are read from PID 1's environment: an SSH session does not inherit the container's.
# (Test hooks, scripts/test-release-gpu-gate.sh: DEADMAN_ENVIRON = another environ file, DEADMAN_WAIT = the waits
# between attempts, DEADMAN_NO_KILL=1 = stop before killing PID 1.)
set -u
if [ "${1:-}" = "--deadman" ]; then
    sleep "${2:?seconds}"
    exec >> "${DEADMAN_LOG:-/workspace/deadman.log}" 2>&1
    echo "$(date -u +%FT%TZ) dead-man: the gate's time is up; removing this pod"
    eval "$(tr '\0' '\n' < "${DEADMAN_ENVIRON:-/proc/1/environ}" 2>/dev/null | grep -E '^RUNPOD_(POD_ID|API_KEY)=[A-Za-z0-9_.-]+$' | sed 's/^/export /')"
    if [ -n "${RUNPOD_POD_ID:-}" ]; then
        if command -v runpodctl >/dev/null; then
            runpodctl pod delete "$RUNPOD_POD_ID" || runpodctl remove pod "$RUNPOD_POD_ID"
        fi
        if [ -n "${RUNPOD_API_KEY:-}" ]; then
            curl -fsS -m 60 -X DELETE -H "Authorization: Bearer $RUNPOD_API_KEY" \
                -H "User-Agent: hazync-release-gpu-gate-deadman/1" "https://api.runpod.io/v2/pods/$RUNPOD_POD_ID" \
                || curl -fsS -m 60 -X DELETE -H "Authorization: Bearer $RUNPOD_API_KEY" \
                    -H "User-Agent: hazync-release-gpu-gate-deadman/1" "https://rest.runpod.io/v1/pods/$RUNPOD_POD_ID"
        fi
        sleep "${DEADMAN_WAIT:-60}"   # still here: the delete did not take
        command -v runpodctl >/dev/null && { runpodctl pod stop "$RUNPOD_POD_ID" || runpodctl stop pod "$RUNPOD_POD_ID"; }
        sleep "${DEADMAN_WAIT:-60}"
    fi
    echo "$(date -u +%FT%TZ) dead-man: still running; ending the container"
    [ "${DEADMAN_NO_KILL:-0}" = 1 ] && exit 0
    kill -TERM 1; sleep 30; kill -KILL 1
    exit 0
fi
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
