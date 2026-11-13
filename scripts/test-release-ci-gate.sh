#!/usr/bin/env bash
# The release CI gate refuses everything the old one let through (review 11-12, release #4).
#
#   scripts/test-release-ci-gate.sh            # must PASS
#   scripts/test-release-ci-gate.sh --control  # a COPY of the gate reduced to one green job; MUST FAIL
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
CONTROL=0; [ "${1:-}" = "--control" ] && CONTROL=1
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
GATE=./scripts/release-ci-gate.sh
if [ "$CONTROL" = 1 ]; then
    # the old gate's meaning: one passing job is enough, whatever else is missing, running or cancelled
    sed 's/^REQUIRED=.*/REQUIRED="dependency-audit"/' "$GATE" > "$T/gate.sh"; chmod +x "$T/gate.sh"; GATE=$T/gate.sh
    echo "CONTROL: the gate reduced to one green job -- the checks below MUST fail"
fi
REQ=$(sed -n 's/^REQUIRED="\(.*\)"$/\1/p' scripts/release-ci-gate.sh)
fails=0
check() { if [ "$1" = 1 ]; then echo "  ok   $2"; else echo "  FAIL $2"; fails=$((fails+1)); fi; }
# fixture <file> <name:status:conclusion:id>...  (every required job success, then the overrides, then extras)
fixture() {
    local f=$1; shift
    python3 - "$f" "$REQ" "$@" <<'PY'
import json, sys
f, req, extra = sys.argv[1], sys.argv[2].split(), sys.argv[3:]
runs = {n: {"name": n, "status": "completed", "conclusion": "success", "id": 100 + i} for i, n in enumerate(req)}
out = list(runs.values())
for e in extra:
    name, status, concl, rid = e.split(":")
    if rid == "replace":
        out = [r for r in out if r["name"] != name]
        if status == "drop": continue
        rid = runs.get(name, {}).get("id", 1)
    out.append({"name": name, "status": status, "conclusion": concl or None, "id": int(rid)})
json.dump({"total_count": len(out), "check_runs": out}, open(f, "w"))
PY
}
run() { CI_GATE_JSON="$1" "$GATE" deadbeefcafe 2>&1; }

fixture "$T/green" ;                                              run "$T/green" >/dev/null; check "$([ $? = 0 ] && echo 1)" "every required job passed: green"
fixture "$T/missing" "soundness-suite:drop::replace";            run "$T/missing" >/dev/null; check "$([ $? = 1 ] && echo 1)" "soundness-suite never ran: refused"
fixture "$T/running" "libzync-chain:in_progress::replace";       run "$T/running" >/dev/null; check "$([ $? = 1 ] && echo 1)" "libzync-chain still running: refused"
fixture "$T/queued" "reproducible-image-id:queued::replace";     run "$T/queued" >/dev/null;  check "$([ $? = 1 ] && echo 1)" "reproducible-image-id queued: refused"
fixture "$T/cancel" "libzync-rig:completed:cancelled:replace";   run "$T/cancel" >/dev/null;  check "$([ $? = 1 ] && echo 1)" "a cancelled job is not a pass: refused"
fixture "$T/rerun_ok" "libzync-chain:completed:failure:replace" "libzync-chain:completed:success:999"
run "$T/rerun_ok" >/dev/null; check "$([ $? = 0 ] && echo 1)" "failed, then re-run green: the newest attempt counts (green)"
fixture "$T/rerun_bad" "libzync-chain:completed:failure:999";    run "$T/rerun_bad" >/dev/null; check "$([ $? = 1 ] && echo 1)" "green, then re-run red: the newest attempt counts (refused)"
python3 -c 'import json,sys;json.dump({"total_count":1,"check_runs":[{"name":"windows-prove","status":"completed","conclusion":"success","id":5}]},open(sys.argv[1],"w"))' "$T/win"
run "$T/win" >/dev/null; check "$([ $? = 1 ] && echo 1)" "only windows-prove green (the main suite never ran): refused"
echo 'gh: HTTP 502' > "$T/junk"
run "$T/junk" >/dev/null; check "$([ $? = 2 ] && echo 1)" "an unreadable answer is refused (exit 2), not 'continuing'"
RELEASE_CI_UNREADABLE_OK=1 CI_GATE_JSON="$T/junk" "$GATE" deadbeef >/dev/null 2>&1; check "$([ $? = 0 ] && echo 1)" "... unless the operator types RELEASE_CI_UNREADABLE_OK=1"
out=$(run "$T/running"); check "$(echo "$out" | grep -q 'libzync-chain: in_progress' && echo 1)" "the refusal names the job and its state ($out)"

if [ "$CONTROL" = 1 ]; then
    if [ "$fails" -ge 5 ]; then echo "CONTROL OK: $fails checks fail under the old meaning, as they must"; exit 0; fi
    echo "CONTROL BROKEN: only $fails failed -- this test proves nothing"; exit 1
fi
[ "$fails" = 0 ] && echo PASS || echo "FAIL ($fails)"
exit $([ "$fails" = 0 ] && echo 0 || echo 1)
