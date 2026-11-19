#!/usr/bin/env python3
"""Exit 1 (the unit fails and pages) while the coordinator's reorg backstop is latched (REORG.md §17-§18): the board
holds a verified range that is not on prep's chain; or while the backstop has not run for CHECK_REORG_STALE_S (900 s:
the coordinator down, or its reorg thread stuck; round 4, M2); exit 2 when the database cannot be read. Reads the
coordinator's database read-only (COORD_DB)."""
import json, os, sqlite3, sys, time

STALE = float(os.environ.get("CHECK_REORG_STALE_S", "900"))
try:
    c = sqlite3.connect(f"file:{os.environ.get('COORD_DB', '/var/lib/hazync-lz/coordinator.db')}?mode=ro", uri=True)
    rows = dict(c.execute("SELECT k, v FROM meta WHERE k IN ('reorg_alert', 'reorg_backstop_at', 'reorg_backstop_note')").fetchall())
except sqlite3.Error as e:
    print(f"cannot check: {e}"); sys.exit(2)
a = json.loads(rows["reorg_alert"]) if rows.get("reorg_alert") else None
if a:
    print(f"REORG ALERT: {a['msg']} (heights {a['heights'][:10]}); REORG.md §11")
    sys.exit(1)
try:
    age = time.time() - float(rows.get("reorg_backstop_at") or 0)
except ValueError:
    age = float("inf")
if age > STALE:
    print(f"REORG BACKSTOP STALE: it last ran {age:.0f} s ago (CHECK_REORG_STALE_S={STALE:.0f}): is the coordinator up, "
          f"and its reorg thread running? (journal: [reorg] lines)")
    sys.exit(1)
try:
    note = json.loads(rows.get("reorg_backstop_note") or "{}")
except ValueError:
    note = {}
if note.get("quiet"):
    # nothing was compared (round 5, LOW 4): say so, not "on prep's chain"
    print(f"ok: no alert; the last backstop run ({age:.0f} s ago) compared nothing: {note['quiet']}")
else:
    print(f"ok: the {note.get('compared', '?')} verified range(s) in the reorg window are on prep's chain (checked {age:.0f} s ago"
          + (f"; {note['empty_states']} empty state file(s) after a crash, not compared" if note.get("empty_states") else "") + ")")
