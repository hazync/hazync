#!/usr/bin/env bash
# Cases for hazync-check-board-progress, driven by a fake `curl` answering /api/state.
#
#   test-board-progress.sh
#
# The case that matters most is "the count FELL": after the v1 -> v2 switch the stored baseline was the old board's
# 145,223, the new board's count never passed it, and the check reported "nobody is proving" for 840 hours.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
CHK="$HERE/hazync-check-board-progress.sh"
fails=0
ok()  { echo "  ok   $*"; }
bad() { echo "  FAIL $*"; fails=$((fails+1)); }

T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin"
fake_state() {  # $1 = proven, $2 = 1 if the board offers work
  local board='[]'; [ "$2" = 1 ] && board='[{"id":"0-999","lo":0,"hi":999,"status":"open"}]'
  cat > "$T/bin/curl" <<SH
#!/usr/bin/env bash
echo '{"progress":{"proven":$1},"blocked":null,"board":$board}'
SH
  chmod +x "$T/bin/curl"
}
run() { PATH="$T/bin:$PATH" STATE_DIRECTORY="$T" BOARD_IDLE_ALERT_SECS=7200 bash "$CHK" 2>&1; }
expect() {  # $1 = name, $2 = want exit, $3 = text the output must contain
  local out rc; out=$(run); rc=$?
  if [ "$rc" = "$2" ] && printf '%s' "$out" | grep -qF -- "$3"; then ok "$1 (exit $rc)"; else bad "$1: exit $rc, want $2; output: $out"; fi
}
old=$(( $(date +%s) - 3*86400 ))

fake_state 100 1; rm -f "$T/board-progress"
expect "first run sets a baseline" 0 "advanced; clock reset"

fake_state 120 1
expect "a rise resets the clock" 0 "advanced; clock reset"

echo "120 $old" > "$T/board-progress"
expect "unchanged for 3 days with work is an alarm" 1 "NOBODY IS PROVING"

fake_state 120 0
expect "unchanged with no work is not an alarm" 0 "offering no work"

fake_state 1849 1; echo "145223 $old" > "$T/board-progress"
expect "a fallen count is a new board, not a stall" 0 "a different board"
read -r p ts < "$T/board-progress"
[ "$p" = 1849 ] && [ "$ts" -gt "$old" ] && ok "the new board's count is the baseline" || bad "baseline is '$p $ts'"
expect "...and the next run counts from it" 0 "last advanced"

echo
[ "$fails" = 0 ] && { echo "test-board-progress: all cases behave"; exit 0; }
echo "test-board-progress: $fails case(s) FAILED"; exit 1
