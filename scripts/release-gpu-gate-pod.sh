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
# --deadman <seconds> <expected pod id>: the POD-SIDE cap (scripts/release_gpu_gate.py arms it right after SSH
# comes up; between the create and that moment -- up to the gate's boot wait -- there is NO dead-man, only the
# laptop and `sweep`). After <seconds> the pod removes ITSELF, so a laptop that crashed, lost power or was SIGKILLed
# cannot leave it billing:
#   1. delete through RunPod's API with the pod-scoped key RunPod injects (RUNPOD_POD_ID, RUNPOD_API_KEY: "Pod-scoped
#      API key", docs.runpod.io/pods/references/environment-variables): runpodctl if the image has it, then REST;
#   2. else STOP it (RunPod: stopping "releases the GPU"; a stopped pod still bills its volume disk until deleted);
#   3. else kill the gate's own workload and log "COULD NOT REMOVE THIS POD": the pod then idles (and BILLS) until
#      `scripts/release-gpu-gate.sh sweep --terminate`. (Signalling PID 1 from inside its own namespace does nothing,
#      so the container cannot end itself; that is not attempted.)
# It refuses unless RUNPOD_POD_ID is set AND equals the expected id: never act anywhere but on the pod it was armed
# on. The variables are read from PID 1's environment (an SSH session does not inherit the container's). Each step
# that answered is logged to deadman.log, which the gate harvests.
# (Test hooks, scripts/test-release-gpu-gate.sh: DEADMAN_ENVIRON = another environ file, DEADMAN_WAIT = the waits
# between attempts, DEADMAN_LOG, DEADMAN_WORKDIR = where script.pid is.)
set -u
if [ "${1:-}" = "--deadman" ]; then
    want="${3:-}"
    sleep "${2:?seconds}"
    exec >> "${DEADMAN_LOG:-/workspace/deadman.log}" 2>&1
    log() { echo "$(date -u +%FT%TZ) dead-man: $*"; }
    log "the gate's time is up; removing pod ${want:-?}"
    eval "$(tr '\0' '\n' < "${DEADMAN_ENVIRON:-/proc/1/environ}" 2>/dev/null | grep -E '^RUNPOD_(POD_ID|API_KEY)=[A-Za-z0-9_.-]+$' | sed 's/^/export /')"
    if [ -z "$want" ] || [ -z "${RUNPOD_POD_ID:-}" ] || [ "${RUNPOD_POD_ID}" != "$want" ]; then
        log "REFUSING: RUNPOD_POD_ID='${RUNPOD_POD_ID:-}' is not the pod this was armed on ('$want'); doing nothing"
        exit 1
    fi
    UA="User-Agent: hazync-release-gpu-gate-deadman/1"
    if command -v runpodctl >/dev/null; then
        if runpodctl pod delete "$RUNPOD_POD_ID"; then log "runpodctl pod delete answered OK"
        elif runpodctl remove pod "$RUNPOD_POD_ID"; then log "runpodctl remove pod answered OK"
        else log "runpodctl could not delete"; fi
    fi
    if [ -n "${RUNPOD_API_KEY:-}" ]; then
        if curl -fsS -m 60 -X DELETE -H "Authorization: Bearer $RUNPOD_API_KEY" -H "$UA" \
               "https://api.runpod.io/v2/pods/$RUNPOD_POD_ID"; then log "REST DELETE (api.runpod.io/v2) answered OK"
        elif curl -fsS -m 60 -X DELETE -H "Authorization: Bearer $RUNPOD_API_KEY" -H "$UA" \
               "https://rest.runpod.io/v1/pods/$RUNPOD_POD_ID"; then log "REST DELETE (rest.runpod.io/v1) answered OK"
        else log "REST DELETE refused"; fi
    fi
    sleep "${DEADMAN_WAIT:-60}"   # still here: no delete took
    if command -v runpodctl >/dev/null; then
        if runpodctl pod stop "$RUNPOD_POD_ID" || runpodctl stop pod "$RUNPOD_POD_ID"; then log "runpodctl stop answered OK"
        else log "runpodctl could not stop"; fi
    fi
    sleep "${DEADMAN_WAIT:-60}"
    log "COULD NOT REMOVE THIS POD ($RUNPOD_POD_ID): it keeps billing until 'release-gpu-gate.sh sweep --terminate'; killing the gate workload"
    pid=$(cat "${DEADMAN_WORKDIR:-/workspace}/script.pid" 2>/dev/null)
    [[ "$pid" =~ ^[0-9]+$ ]] && kill -TERM -- "-$pid" 2>/dev/null && log "killed the workload (process group $pid)"
    exit 1
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
