#!/bin/bash
# Runs ON A RENTED POD, in /workspace, for scripts/release_gpu_gate.py (never run it anywhere else). Real proofs with the
# EXACT staged release CUDA prover (the engine, OpenVM; engine/RELEASE_TOOLING.md):
#
#   /workspace/hzengine          the staged hazync-engine-x86_64-linux-gnu-cuda (pushed by the gate)
#   /workspace/want_host_sha256  its sha256, as the operator gave it to the gate
#   /workspace/keys/<asset>      the staged key tarballs (hazync-keys-<name>.tar.gz) the entries name
#   /workspace/want_keys_sha256  "<sha256>  <asset>" for each of them
#   /workspace/gate/             the fixtures (engine/gpu-gate: GATE, SHA256SUMS, block exports, the header stream)
#
# For each line of gate/GATE (<label> <kind> <keys> <input> <height> <hash> <anchor|-> <want>):
#   block    the export (a .tgz of `engine-export`'s directory): hzengine prove-block --dir <keys> --export <it>
#   headers  the headers zAPP's stream (gzip allowed): hzengine prove --dir <keys> --input <it>
# then `hzengine pack` writes out/<label>.hze, the HZE1 file a reader verifies (pack checks it with the readers' own
# verifier first). Raw proofs stay in /workspace/work (not harvested: they are large); each run is timed into
# out/proves.tsv. Nothing is judged here beyond "did it run": the HZE1 files are verified on the maintainer's machine,
# with the staged verifiers, after harvest. Ends by writing ONE word (and a reason) to out/RESULT:
#   PROVED | NO_GPU | HOST_SHA_BAD | KEYS_SHA_BAD | FIXTURE_BAD | HOST_BAD | PROVE_FAILED <label> rc=<n> | PACK_FAILED <label> rc=<n>
# No retry: a release gate that retries hides the fault it is there to see.
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
    # The workload is the pod script's whole SESSION (it was started with setsid, so its pid is the session id):
    # a process group is not enough, because `timeout … host` puts the prover in a group of its own. Only a pid that
    # is still the pod script is touched (> 1, and its cmdline names this script), never PID 1 or a reused pid.
    pid=$(cat "${DEADMAN_WORKDIR:-/workspace}/script.pid" 2>/dev/null)
    if [[ "$pid" =~ ^[0-9]+$ ]] && [ "$pid" -gt 1 ] && tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null | grep -q 'release-gpu-gate-pod\.sh'; then
        pkill -TERM -s "$pid"; sleep "${DEADMAN_KILL_WAIT:-5}"; pkill -KILL -s "$pid"
        log "killed the workload (session $pid)"
    else
        log "no live gate workload to kill (script.pid '$pid')"
    fi
    exit 1
fi
W=/workspace; O=$W/out; WK=$W/work
mkdir -p "$O" "$WK"; cd "$W" || exit 1
finish() { echo "$*" > "$O/RESULT"; exit 0; }
date -u +%FT%TZ > "$O/started"
nvidia-smi --query-gpu=name,compute_cap,driver_version,memory.total --format=csv,noheader > "$O/gpu.txt" 2>&1 \
    || finish NO_GPU
# GeForce cards on RunPod need the image's CUDA forward-compat libraries out of the way (sponsor_bot.BOOT_SH).
nvidia-smi --query-gpu=name --format=csv,noheader | grep -q GeForce \
    && for d in /usr/local/cuda*/compat; do [ -d "$d" ] && mv "$d" "${d}.disabled"; done
ldconfig 2>/dev/null
sha256sum hzengine | cut -c1-64 > "$O/host_sha256"
[ "$(cat "$O/host_sha256")" = "$(tr -dc '0-9a-f' < want_host_sha256)" ] || finish HOST_SHA_BAD
# every key tarball by sha256, recorded for the judge, then extracted to work/k/<name>
: > "$O/keys_sha256.txt"
while read -r want asset; do
    [ -n "$asset" ] || continue
    got=$(sha256sum "keys/$asset" 2>/dev/null | cut -c1-64)
    echo "$got $asset" >> "$O/keys_sha256.txt"
    [ "$got" = "$want" ] || finish KEYS_SHA_BAD
    n=${asset#hazync-keys-}; n=${n%.tar.gz}
    mkdir -p "$WK/k/$n" && tar xzf "keys/$asset" -C "$WK/k/$n" || finish KEYS_SHA_BAD
done < want_keys_sha256
(cd gate && sha256sum -c --quiet SHA256SUMS) > "$O/fixtures.txt" 2>&1 || finish FIXTURE_BAD
chmod +x hzengine
timeout 120 ./hzengine --help > "$O/hzengine_help.txt" 2>&1 < /dev/null || finish HOST_BAD
while read -r label kind keys input _h _hash _anchor _want; do
    case "$label" in ''|\#*) continue ;; esac
    K="$WK/k/$keys"; P="$WK/p_$label"; rm -rf "$P" "$O/$label.hze"
    t0=$(date +%s.%N)
    case "$kind" in
        block)
            mkdir -p "$WK/x_$label" && tar xzf "gate/$input" -C "$WK/x_$label" || finish "FIXTURE_BAD $label"
            timeout "${PROVE_TIMEOUT_S:-2400}" ./hzengine prove-block --dir "$K" --export "$WK/x_$label" --out "$P" \
                > "$O/prove_$label.log" 2>&1 < /dev/null
            rc=$?; proof="$P/block" ;;
        headers)
            in="gate/$input"; case "$input" in *.gz) gunzip -c "$in" > "$WK/in_$label" || finish "FIXTURE_BAD $label"; in="$WK/in_$label" ;; esac
            timeout "${PROVE_TIMEOUT_S:-2400}" ./hzengine prove --dir "$K" --input "$in" --out "$P" \
                > "$O/prove_$label.log" 2>&1 < /dev/null
            rc=$?; proof="$P" ;;
        *) finish "FIXTURE_BAD $label kind $kind" ;;
    esac
    t1=$(date +%s.%N)
    printf '%s\t%s\t%s\n' "$label" "$rc" "$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.1f", b-a}')" >> "$O/proves.tsv"
    [ "$rc" = 0 ] || finish "PROVE_FAILED $label rc=$rc"
    ./hzengine pack --dir "$K" --proof "$proof" --out "$O/$label.hze" > "$O/pack_$label.log" 2>&1 < /dev/null
    rc=$?; [ "$rc" = 0 ] || finish "PACK_FAILED $label rc=$rc"
done < gate/GATE
finish PROVED
