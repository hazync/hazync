#!/usr/bin/env python3
"""Exit 1 (the unit fails and pages) while the coordinator's reorg backstop is latched (REORG.md §17-§18): the board
holds a verified range that is not on prep's chain. Reads the coordinator's database read-only (COORD_DB)."""
import json, os, sqlite3, sys

c = sqlite3.connect(f"file:{os.environ.get('COORD_DB', '/var/lib/hazync-lz/coordinator.db')}?mode=ro", uri=True)
try:
    r = c.execute("SELECT v FROM meta WHERE k='reorg_alert'").fetchone()
except sqlite3.OperationalError as e:
    print(f"cannot check: {e}"); sys.exit(2)
a = json.loads(r[0]) if r and r[0] else None
if a:
    print(f"REORG ALERT: {a['msg']} (heights {a['heights'][:10]}); REORG.md §11")
    sys.exit(1)
print("ok: every verified range in the reorg window is on prep's chain")
