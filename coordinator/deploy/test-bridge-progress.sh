#!/usr/bin/env bash
# Positive controls for hazync-check-bridge-progress (hazync#467).
#
# ⛔ A CHECK THAT CANNOT FAIL IS WORSE THAN NO CHECK. This one exists because the bridge stalls
# SILENTLY during catch-up, so its own failure path must be exercised rather than assumed.
#
#   test-bridge-progress.sh            # all cases must behave
#   test-bridge-progress.sh --control  # the stall guard removed; MUST fail
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
CHK="$HERE/hazync-check-bridge-progress.sh"
CONTROL=0; [ "${1:-}" = "--control" ] && { CONTROL=1; export HAZYNC_BRIDGE_NO_STALL_GUARD=1 HAZYNC_BRIDGE_NO_ACTIVATING_GUARD=1; }
fails=0
ok()  { echo "  ok   $*"; }
bad() { echo "  FAIL $*"; fails=$((fails+1)); }

# A fake `journalctl`/`systemctl`/`date` so the check can be driven to each outcome without a bridge.
STUB="$(mktemp -d)"; trap 'rm -rf "$STUB"' EXIT
mkstub() {  # $1=unit-active (yes|no|activating:AGE_S) $2=checkpoint-age-seconds ("none" for no line)
  # ⚠ `activating:AGE` answers InactiveExitTimestampMonotonic derived from the REAL /proc/uptime, so
  # the wedge test drives the check's own monotonic arithmetic instead of a parallel copy of it.
  _state=active; _mono=0
  case "$1" in
      no)           _state=inactive ;;
      activating:*) _state=activating
                    _mono="$(awk -v a="${1#activating:}" '{printf "%d", ($1 - a) * 1000000}' /proc/uptime)" ;;
  esac
  cat > "$STUB/systemctl" <<SH
#!/usr/bin/env bash
case "\$*" in *"is-active --quiet"*) [ "$_state" = active ] && exit 0 || exit 3;;
              *InactiveExitTimestampMonotonic*) echo "$_mono";;
              *is-active*) echo "$_state";; esac
SH
  cat > "$STUB/journalctl" <<SH
#!/usr/bin/env bash
[ "$2" = none ] && exit 0
echo "\$(( \$(date +%s) - $2 )).000000 host hazync-host-bridge[1]: bridge: ${3:-checkpoint @} ${4:-913457}${5:- (1 utxos, 1 leaves)}"
SH
  chmod +x "$STUB/systemctl" "$STUB/journalctl"
  # ⚠ No bitcoin-cli by default, so the cases above still exercise the TIME-based path exactly as
  # they did before. The tip-aware cases install one deliberately.
  rm -f "$STUB/bitcoin-cli"
}

# A node that reports `tip`, so the positional test can be reached.
mknode() { printf '#!/usr/bin/env bash\necho %s\n' "$1" > "$STUB/bitcoin-cli"; chmod +x "$STUB/bitcoin-cli"; }
run() { PATH="$STUB:$PATH" bash "$CHK" >/dev/null 2>&1; echo $?; }

# 1. healthy: checkpointed 5 minutes ago
mkstub yes 300
rc=$(run); [ "$rc" = 0 ] && ok "a fresh checkpoint exits 0" || bad "fresh checkpoint exited $rc, expected 0"

# 2. ⛔ THE ONE THAT MATTERS: alive but not advancing for 2 hours
mkstub yes 7200
rc=$(run)
if [ "$CONTROL" = 1 ]; then
    [ "$rc" = 1 ] && bad "CONTROL DID NOT FAIL: a 2-hour stall was still detected" \
                  || ok "control: with the guard removed a stall is missed (exit $rc)"
else
    [ "$rc" = 1 ] && ok "a 2-hour stall exits 1" || bad "a 2-hour stall exited $rc, expected 1"
fi

# 3. not running: cannot check, NOT a pass and NOT a stall
mkstub no 300
rc=$(run); [ "$rc" = 2 ] && ok "an inactive unit exits 2 (cannot check)" || bad "inactive exited $rc, expected 2"

# 4. no checkpoint line yet (a fresh restart reloading 21 GB): cannot check, not a stall
mkstub yes none
rc=$(run); [ "$rc" = 2 ] && ok "no checkpoint line exits 2, not 1" || bad "no-line exited $rc, expected 2"

# 5. ⚠ clock stepped backwards: unknown, never a pass
cat > "$STUB/journalctl" <<'SH'
#!/usr/bin/env bash
echo "$(( $(date +%s) + 600 )).000000 host hazync-host-bridge[1]: bridge: checkpoint @ 913457 (1 utxos, 1 leaves)"
SH
chmod +x "$STUB/journalctl"
rc=$(run); [ "$rc" = 2 ] && ok "a future timestamp exits 2, not 0" || bad "future ts exited $rc, expected 2"

# ── the caught-up regime (hazync#487) ─────────────────────────────────────────────────────────────
# ⛔ THIS IS WHAT PAGED HOURLY. Once the bridge catches up it writes `caught up to N`, not
# `checkpoint @ N`, and the next checkpoint is 2000 blocks away — a fortnight of chain at the tip.
# The check saw no progress line at all and cried stall against a perfectly healthy bridge.

# 6. a `caught up to` line is progress, even though it is not a checkpoint
mkstub yes 300 "caught up to" 968118 ""
rc=$(run); [ "$rc" = 0 ] && ok "a fresh 'caught up to' line exits 0" || bad "'caught up to' exited $rc, expected 0"

# 7. ⛔ AT THE TIP, IDLE IS HEALTHY HOWEVER LONG. Bitcoin's inter-block gaps are exponential, so a
#    70-minute quiet spell is normal and must not page.
mkstub yes 7200 "caught up to" 968118 ""
mknode 968218                       # tip - finality(100) == 968118, so the bridge is exactly where it belongs
rc=$(run); [ "$rc" = 0 ] && ok "at the tip, two hours idle is NOT a stall" || bad "tip-idle exited $rc, expected 0"

# 8. ⛔ AND IT MUST STILL BE ABLE TO FAIL. Same two hours, but the bridge is 5000 blocks BEHIND where
#    the node says it should be — that is a real stall and the positional test must not mask it.
mkstub yes 7200 "caught up to" 963118 ""
mknode 968218
rc=$(run)
if [ "$CONTROL" = 1 ]; then
    [ "$rc" = 1 ] && bad "CONTROL DID NOT FAIL: a behind-the-tip stall was still detected" \
                  || ok "control: with the guard removed a behind-the-tip stall is missed (exit $rc)"
else
    [ "$rc" = 1 ] && ok "behind the tip and not advancing exits 1" || bad "behind-tip exited $rc, expected 1"
fi

# 9. ⚠ a node that cannot be asked falls through to the time test rather than assuming health
mkstub yes 7200 "caught up to" 968118 ""
printf '#!/usr/bin/env bash\nexit 1\n' > "$STUB/bitcoin-cli"; chmod +x "$STUB/bitcoin-cli"
rc=$(run)
if [ "$CONTROL" = 1 ]; then
    [ "$rc" = 1 ] && bad "CONTROL DID NOT FAIL: an unreachable node still detected the stall" \
                  || ok "control: unreachable node + guard removed misses it (exit $rc)"
else
    [ "$rc" = 1 ] && ok "an unreachable node falls back to the time test, not to 'ok'" \
                  || bad "unreachable-node exited $rc, expected 1"
fi

# ── a start that can never fail (hazync#650, measured 2026-10-06) ─────────────────────────────────
# ⛔⛔ THE GAP THAT LET BUNDLE PRODUCTION STOP IN SILENCE. TimeoutStartUSec=infinity + an ExecStartPre
# looping on a node that was down = `activating` for ever. Never `failed`, so OnFailure= cannot fire;
# never `active`, so this check stood down. Nothing in the system was unhappy.

# 10. ⛔ wedged in `activating` for 2 hours — AND with a FRESH checkpoint, to prove the wedge is
#     judged before the stall test and recent progress cannot mask a unit that is not running.
mkstub activating:7200 300
rc=$(run)
if [ "$CONTROL" = 1 ]; then
    [ "$rc" = 1 ] && bad "CONTROL DID NOT FAIL: a 2-hour wedge was still detected" \
                  || ok "control: with the guard removed a wedged start is missed (exit $rc)"
else
    [ "$rc" = 1 ] && ok "a 2-hour wedged start exits 1, despite a 5-minute-old checkpoint" \
                  || bad "wedged start exited $rc, expected 1"
fi

# 11. ⚠ AND IT MUST NOT CRY WOLF ON AN HONEST START. Resuming from a 21 GB state file legitimately
#     spends minutes in `activating`; only a start that has outlived the limit is a wedge.
mkstub activating:60 300
rc=$(run); [ "$rc" = 2 ] && ok "a 60-second-old start exits 2, not 1" || bad "fresh start exited $rc, expected 2"

[ "$fails" = 0 ] && { echo; echo "bridge-progress check behaves on every path"; exit 0; }
echo; echo "$fails case(s) wrong"; exit 1
