#!/usr/bin/env bash
# Check that every DEPLOYED component is running the canonical guest — the deployment-layer twin of
# scripts/check-versions.sh (which only sees the repo).
#
# This exists because of a real outage. The v0.10.0 re-baseline swapped the coordinator's verifying
# binary and the provers' binaries, but NOT the archive bridge, whose unit still pointed at an old
# host predating v0.9.0. The bridge PRODUCES the witnesses everyone else consumes, so from the moment
# it resumed it emitted bundles in a format the current host cannot parse ("missing field `txs`").
# The board stalled dead, and because provers cache bundles by height and skip re-fetching, they kept
# replaying the bad copies long after the bridge was fixed.
#
# Three lessons are baked in below:
#   1. a re-baseline must update EVERY binary, not just the obvious two;
#   2. a component can be stopped for days without anything noticing;
#   3. a check that cannot reach something must SAY SO, never quietly pass.
#
#   ./scripts/check-deployment.sh                       # remote checks only (public API)
#   ./scripts/check-deployment.sh --local               # run ON the coordinator: units, binaries, bundles
#
# Env: COORD_URL (default https://api.hazync.org)
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

COORD_URL="${COORD_URL:-https://api.hazync.org}"
LOCAL=0; [ "${1:-}" = "--local" ] && LOCAL=1
fail=0; skipped=0
bad()  { printf 'FAIL %s\n' "$*"; fail=1; }
ok()   { printf '  ok   %s\n' "$*"; }
skip() { printf '  SKIP %s\n' "$*"; skipped=$((skipped+1)); }

CANON=$(grep -vE '^\s*(#|$)' reproduce/METHOD_ID 2>/dev/null | tr -d '[:space:]')
[ ${#CANON} -eq 64 ] || { echo "FAIL cannot read a canonical id from reproduce/METHOD_ID"; exit 1; }
echo "canonical guest: ${CANON:0:8}…"

echo "== coordinator (remote) =="
meta=$(curl -s --max-time 20 "$COORD_URL/api/meta" 2>/dev/null)
if [ -z "$meta" ]; then
    skip "coordinator unreachable at $COORD_URL — its guest id was NOT checked"
else
    served=$(grep -oE '"method_id"[[:space:]]*:[[:space:]]*"[0-9a-f]{64}"' <<<"$meta" | grep -oE '[0-9a-f]{64}')
    if [ "$served" = "$CANON" ]; then ok "coordinator advertises the canonical guest"
    else bad "coordinator advertises ${served:0:8}… but canonical is ${CANON:0:8}… — provers will have every proof rejected"; fi

    # SOURCE DRIFT. The guest-id check above says nothing about the coordinator's own Python: on
    # 2026-07-28 production was found on a stale branch, months behind main, with UNCOMMITTED edits to
    # server.py that existed nowhere in git. Nothing reported it, and a naive redeploy would have
    # destroyed them silently. The coordinator now self-reports sha256 of the source it actually
    # loaded (/api/meta.source_sha256); this compares it with the file in THIS checkout.
    #
    # A mismatch is not automatically wrong — this checkout may simply be at a different commit than
    # the deployment. It means "these two disagree, go and find out which is stale", which is exactly
    # the question nobody was in a position to ask before.
    remote_src=$(grep -oE '"source_sha256"[[:space:]]*:[[:space:]]*"[0-9a-f]+"' <<<"$meta" | grep -oE '[0-9a-f]{64}')
    local_src=$(sha256sum coordinator/server.py 2>/dev/null | cut -d' ' -f1)
    if [ -z "$remote_src" ]; then
        skip "coordinator does not report source_sha256 (older build) — source drift NOT checked"
    elif [ -z "$local_src" ]; then
        skip "cannot hash coordinator/server.py here — source drift NOT checked"
    elif [ "$remote_src" = "$local_src" ]; then
        ok "coordinator runs the same server.py as this checkout (${local_src:0:12}…)"
    else
        bad "coordinator source DRIFT: it runs ${remote_src:0:12}…, this checkout has ${local_src:0:12}…
       One of them is stale. Before redeploying, DIFF the deployed file against this one — production
       has previously carried fixes that were never committed, and copying over them loses that work."
    fi
fi

if [ "$LOCAL" -eq 0 ]; then
    echo
    echo "Remote mode: unit/binary/bundle checks were NOT run. Re-run with --local on the coordinator"
    echo "to check the bridge and coordinator binaries — that is where the drift actually hides."
    [ "$fail" -eq 0 ] && exit 0 || exit 1
fi

# HAZYNC V2. After coordinator/deploy/libzync/SWITCH.md the public board is the libzync coordinator on 8899 and the
# v1 (CORE) units are disabled on purpose. On such a box a DISABLED, stopped v1 unit is "retired", not a failure;
# anything still enabled is checked exactly as before (a retired unit someone re-enabled is caught).
V2=0
systemctl show hazync-lz-coordinator -p Environment --value 2>/dev/null | tr ' ' '\n' | grep -qx 'COORD_PORT=8899' && V2=1
[ "$V2" -eq 1 ] && echo "board: Hazync v2 (hazync-lz-coordinator on 8899); disabled v1 units are reported as retired"
retired() {  # $1 unit: true when this is a switched box and the unit is switched off, not merely between runs
    [ "$V2" -eq 1 ] || return 1
    [ "$(systemctl is-active "$1" 2>/dev/null)" = active ] && return 1
    case "$1" in *@*) return 1 ;; esac                      # a template (hazync-alert@) runs on demand
    # a timer-driven oneshot (the sponsor bot) is inactive between runs: it is live while its timer is enabled
    local t="${1%.service}.timer"
    systemctl cat "$t" >/dev/null 2>&1 && [ "$(systemctl is-enabled "$t" 2>/dev/null)" = enabled ] && return 1
    [ "$(systemctl is-enabled "$1" 2>/dev/null)" != enabled ]
}

echo "== every hazync systemd unit runs the canonical binary =="
# THE check that would have caught the outage. Any unit whose ExecStart is a hazync host binary must
# report the canonical id — the bridge is as load-bearing as the coordinator here.
found=0
while read -r unit; do
    [ -z "$unit" ] && continue
    if retired "$unit"; then printf '  note %s: retired (Hazync v1, disabled since the switch)\n' "$unit"; continue; fi
    exe=$(systemctl show "$unit" -p ExecStart --value 2>/dev/null | grep -oE '/[^ ]*host[^ ]*' | head -1)
    hostbin=$(systemctl show "$unit" -p Environment --value 2>/dev/null | tr ' ' '\n' | grep -oE 'HAZYNC_HOST=.*' | cut -d= -f2)
    for b in "$exe" "$hostbin"; do
        [ -n "$b" ] && [ -x "$b" ] || continue
        found=1
        id=$("$b" method-id 2>/dev/null | grep -oE '[0-9a-f]{64}' | head -1)
        if [ "$id" = "$CANON" ]; then ok "$unit -> $(basename "$b") is canonical"
        else bad "$unit runs $(basename "$b") with guest ${id:-unknown} — NOT canonical (${CANON:0:8}…)"; fi
    done
    # A component that is simply switched off produces no error anywhere. The bridge sat inactive for
    # two days and the only symptom was the board quietly failing to advance. But only the REQUIRED
    # services are a failure when stopped — optional helpers (CPU gap-fill and the like) are routinely
    # off on purpose, and failing on those would make this check noise that gets ignored.
    st=$(systemctl is-active "$unit" 2>/dev/null)
    case "$unit" in
        hazync-bridge.service|hazync-coordinator.service)
            [ "$st" = active ] && ok "$unit is active" \
                               || bad "$unit is $st — REQUIRED; a stopped producer stalls the board silently" ;;
        *)  [ "$st" = active ] && ok "$unit is active (optional)" \
                               || printf '  note %s is %s (optional service)\n' "$unit" "$st" ;;
    esac
done < <(systemctl list-unit-files 'hazync-*' --no-legend 2>/dev/null | awk '{print $1}' | grep -vE 'backup|\.timer|^hazync-lz-')
[ "$found" -eq 0 ] && skip "no hazync units found here — is this the coordinator?"

echo "== bundles the bridge is emitting are in the format the current host parses =="
if [ "$V2" -eq 1 ] && retired hazync-bridge.service; then
    printf '  note the v1 bridge is retired; Hazync v2 leaf bundles are checked below\n'
    BR=""; BR_RETIRED=1
else BR_RETIRED=0; BR="${HAZYNC_BRIDGE_OUT:-$(systemctl show hazync-bridge -p Environment --value 2>/dev/null | tr ' ' '\n' | grep -oE 'HAZYNC_BRIDGE_OUT=.*' | cut -d= -f2)}"
fi
if [ "$BR_RETIRED" -eq 1 ]; then :
elif [ -z "$BR" ] || [ ! -d "$BR" ]; then
    skip "bridge output dir not found — bundle format NOT checked"
else
    # find, not a glob: this directory holds >160k bundles and `ls "$BR"/bundle_*.json` overflows the
    # argument list, which made an earlier version of this check report a FALSE SKIP.
    newest=$(find "$BR" -maxdepth 1 -name 'bundle_*.json' -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)
    if [ -z "$newest" ]; then skip "no bundles in $BR"
    else
        # `txs`/`tx_prevouts` are the per-tx dedup blobs added in v0.9.0. A bundle without them is
        # pre-v0.9.0 output; the current host panics on it with "missing field `txs`".
        if python3 -c "
import json,sys
w=json.load(open('$newest')).get('witness',{})
sys.exit(0 if 'txs' in w and 'tx_prevouts' in w else 1)" 2>/dev/null
        then ok "newest bundle $(basename "$newest") has txs/tx_prevouts"
        else bad "$(basename "$newest") is missing txs/tx_prevouts — bridge is emitting a stale format; provers will panic"; fi
    fi
fi

echo "== the libzync board (beside the original until the switch) runs the published libzync image =="
# The libzync board's units (hazync-lz-*, libzync-*) verify and prove against the published libzync image, not the
# CORE id above, so they are checked here against reproduce/LIBZYNC_IDS. Absent units: a box without the libzync
# board, said as a SKIP, not passed.
LZ=$(awk '$1 == "libzync-product" && $2 == "mainnet" { print $4 }' reproduce/LIBZYNC_IDS 2>/dev/null)
if [ ${#LZ} -ne 64 ]; then
    bad "cannot read the libzync mainnet id from reproduce/LIBZYNC_IDS"
elif ! systemctl list-unit-files 'hazync-lz-coordinator.service' --no-legend 2>/dev/null | grep -q .; then
    skip "no hazync-lz-coordinator here — the libzync board was NOT checked"
else
    for unit in hazync-lz-coordinator.service libzync-board-prep.service libzync-prep.service; do
        env=$(systemctl show "$unit" -p Environment --value 2>/dev/null | tr ' ' '\n')
        b=$(grep -oE '^HAZYNC_HOST=.*' <<<"$env" | cut -d= -f2)
        root=$(grep -oE '^LZ_ROOT=.*' <<<"$env" | tail -1 | cut -d= -f2)
        [ -z "$b" ] && [ -n "$root" ] && b="$root/bin/host_lz"
        if [ -z "$b" ] || [ ! -x "$b" ]; then
            [ "$unit" = libzync-prep.service ] && { printf '  note %s: no host binary found (optional)\n' "$unit"; continue; }
            bad "$unit: no host binary found (HAZYNC_HOST or \$LZ_ROOT/bin/host_lz)"; continue
        fi
        id=$("$b" method-id 2>/dev/null | grep -oE '[0-9a-f]{64}' | head -1)
        if [ "$id" = "$LZ" ]; then ok "$unit -> $b is the published libzync image (${LZ:0:8}…)"
        elif [ "$unit" = libzync-prep.service ]; then
            # the separate whole-chain prep run (the paper's), not the board: a different image is worth knowing,
            # not a failure of the board
            printf '  note %s runs %s with guest %s, not the published image (%s…): the separate prep run, not the board\n' \
                "$unit" "$b" "${id:-unknown}" "${LZ:0:8}"
        else bad "$unit runs $b with guest ${id:-unknown} — NOT the published libzync image (${LZ:0:8}…)"; fi
        st=$(systemctl is-active "$unit" 2>/dev/null)
        case "$unit" in
            hazync-lz-coordinator.service|libzync-board-prep.service)
                [ "$st" = active ] && ok "$unit is active" \
                                   || bad "$unit is $st — REQUIRED for the libzync board" ;;
            *)  [ "$st" = active ] && ok "$unit is active (optional)" \
                                   || printf '  note %s is %s (optional service)\n' "$unit" "$st" ;;
        esac
    done
    # the coordinator serves prep's leaf bundles: the newest must be the format the image's leaf prover reads
    LV=$(systemctl show hazync-lz-coordinator -p Environment --value 2>/dev/null | tr ' ' '\n' | grep -oE '^HAZYNC_BRIDGE_OUT=.*' | cut -d= -f2)
    newest=$( [ -d "$LV" ] && find "$LV" -maxdepth 1 -name 'leaf_*.bin' -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)
    if [ -z "$newest" ]; then skip "no leaf bundles in ${LV:-the libzync coordinator HAZYNC_BRIDGE_OUT} — leaf format NOT checked"
    elif [ "$(head -c 8 "$newest")" = LZLEAF02 ]; then ok "newest leaf bundle $(basename "$newest") is LZLEAF02"
    else bad "$(basename "$newest") is not an LZLEAF02 leaf bundle — the board pass is emitting a stale format"; fi
fi

echo
if [ "$skipped" -gt 0 ]; then
    echo "NOTE: $skipped check(s) were SKIPPED and are therefore unverified — not passed."
fi
[ "$fail" -eq 0 ] && echo "deployment matches the canonical guest." \
                  || { echo "Deployment drift: a component is running the wrong guest, is stopped, or is emitting a stale format."; exit 1; }
