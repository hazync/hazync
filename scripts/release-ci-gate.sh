#!/usr/bin/env bash
# The release CI gate: has the MAIN SUITE passed on this exact commit?
#
# Review (Fable, 2026-11-12, release #4): the old gate in release.sh counted ANY successful workflow run on HEAD, so one
# green windows-prove satisfied it; a queued, running or cancelled `adversarial` run was not "bad", so the image-id and
# soundness jobs need not have finished; and an unreadable answer from gh was "could not read CI status (continuing)".
#
# Now: every job named in REQUIRED must have a check run on <sha> whose LATEST attempt completed with `success`.
# Missing, queued, running, cancelled, failed: refused, each named. An unreadable answer is refused too; the only way
# past it is RELEASE_CI_UNREADABLE_OK=1, typed by the operator, and the release log says so.
#
#   scripts/release-ci-gate.sh <sha> [owner/repo]     exit 0 = green, 1 = not green, 2 = could not read
#   CI_GATE_JSON=<file>                               read the check runs from a file (tests), not the API
set -uo pipefail
SHA="${1:?usage: release-ci-gate.sh <sha> [owner/repo]}"
REPO="${2:-hazync/hazync}"
# The jobs a release must not ship without (adversarial.yml). A job renamed there must be renamed here: a name that
# matches nothing is refused as missing, never skipped.
REQUIRED="reproducible-image-id soundness-suite libzync-hosts libzync-chain libzync-rig libzync-signet libzync-blocks libzync-tests accumulator-tests dependency-audit"

if [ -n "${CI_GATE_JSON:-}" ]; then
    json=$(cat "$CI_GATE_JSON" 2>/dev/null) || json=""
else
    # REST, not `gh pr checks --json` / `gh run list`: the installed gh may predate those flags, and an error must
    # not read as an empty list.
    json=$(gh api "repos/$REPO/commits/$SHA/check-runs?per_page=100" 2>/dev/null) || json=""
fi
out=$(REQUIRED="$REQUIRED" python3 - "$json" <<'PY'
import json, os, sys
try:
    runs = json.loads(sys.argv[1])["check_runs"]
except Exception:
    print("UNREADABLE"); sys.exit(0)
latest = {}
for r in runs:   # a re-run adds a check run with the same name: the newest attempt is the one that counts
    k = r.get("name")
    if k not in latest or (r.get("id") or 0) > (latest[k].get("id") or 0):
        latest[k] = r
bad = []
for name in os.environ["REQUIRED"].split():
    r = latest.get(name)
    if r is None:
        bad.append(f"{name}: no run on this commit")
    elif r.get("status") != "completed":
        bad.append(f"{name}: {r.get('status')}")
    elif r.get("conclusion") != "success":
        bad.append(f"{name}: {r.get('conclusion')}")
print("OK" if not bad else "BAD " + "; ".join(bad))
PY
)
case "$out" in
    OK)          echo "CI green on ${SHA:0:8}: all $(echo $REQUIRED | wc -w) required jobs passed"; exit 0 ;;
    UNREADABLE)  if [ "${RELEASE_CI_UNREADABLE_OK:-}" = 1 ]; then
                     echo "WARNING: could not read CI for ${SHA:0:8}; continuing ONLY because RELEASE_CI_UNREADABLE_OK=1"; exit 0
                 fi
                 echo "could not read CI status for ${SHA:0:8} (an unreadable answer is not a pass;"
                 echo "  RELEASE_CI_UNREADABLE_OK=1 overrides, if you have checked the runs yourself)"; exit 2 ;;
    *)           echo "CI not green on ${SHA:0:8}: ${out#BAD }"; exit 1 ;;
esac
