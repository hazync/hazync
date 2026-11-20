#!/usr/bin/env python3
"""Exit 1 (the unit fails and pages) while the coordinator's reorg backstop is latched (REORG.md §17-§18): the board
holds a verified range that is not on prep's chain; or while the backstop has not run for CHECK_REORG_STALE_S (900 s:
the coordinator down, or its reorg thread stuck; round 4, M2); exit 2 when the database cannot be read. Reads the
coordinator's database read-only (COORD_DB). With CHECK_TIP_RUN_PREP_TIP (D1, REORG.md §19): also exit 1 while the tip run
stands still below the node's tip."""
import json, os, sqlite3, sys, time

STALE = float(os.environ.get("CHECK_REORG_STALE_S", "900"))
try:
    c = sqlite3.connect(f"file:{os.environ.get('COORD_DB', '/var/lib/hazync-lz/coordinator.db')}?mode=ro", uri=True)
    rows = dict(c.execute("SELECT k, v FROM meta WHERE k IN ('reorg_alert', 'reorg_backstop_at', 'reorg_backstop_note', "
                          "'handover_alert')").fetchall())
except sqlite3.Error as e:
    print(f"cannot check: {e}"); sys.exit(2)
a = json.loads(rows["reorg_alert"]) if rows.get("reorg_alert") else None
if a:
    print(f"REORG ALERT: {a['msg']} (heights {a['heights'][:10]}); REORG.md §11")
    sys.exit(1)
hv = json.loads(rows["handover_alert"]) if rows.get("handover_alert") else None
if hv:
    print(f"HANDOVER ALERT: {hv['msg']} (REORG.md §19: the board pass and the tip run must hold the same chain)")
    sys.exit(1)
try:
    age = time.time() - float(rows.get("reorg_backstop_at") or 0)
except ValueError:
    age = float("inf")
if age > STALE:
    print(f"REORG BACKSTOP STALE: it last ran {age:.0f} s ago (CHECK_REORG_STALE_S={STALE:.0f}): is the coordinator up, "
          f"and its reorg thread running? (journal: [reorg] lines)")
    sys.exit(1)
# D1 (REORG.md §19): the tip run follows the node with no timeout, so a stalled fetcher shows only as its PREP_TIP.json
# standing still while it is below the node's tip. CHECK_TIP_RUN_PREP_TIP (its out/PREP_TIP.json; unset: not checked)
# older than CHECK_TIP_RUN_STALE_S (3,600 s; a save point takes minutes) with the node's tip (NODE_TIP_FILE) above it pages.
# It measures how long the node has been AHEAD of the tip run, not how old PREP_TIP.json is: after a quiet hour with no
# block both are old, and the next block would page at once (review, LOW 11). CHECK_TIP_RUN_STATE (a file this check
# writes) keeps when it first saw the node ahead of the tip run at its current height. No PREP_TIP.json yet (a host
# from before it, or a restart in progress): said, not paged (review, MEDIUM 4).
tp = os.environ.get("CHECK_TIP_RUN_PREP_TIP")
if tp:
    stale = float(os.environ.get("CHECK_TIP_RUN_STALE_S", "3600"))
    sf = os.environ.get("CHECK_TIP_RUN_STATE", "/var/lib/hazync-lz/check-tip-run.json")
    if not os.path.exists(tp):
        print(f"ok (tip run): no {tp} yet: not checked"); tp = None
if tp:
    try:
        h_tp = int(json.load(open(tp))["height"])
        nt = int(open(os.environ.get("NODE_TIP_FILE", "/var/lib/hazync/node_tip")).read().strip())
    except (OSError, ValueError, KeyError, TypeError) as e:
        print(f"cannot check the tip run: {e}"); sys.exit(2)
    try: seen = json.load(open(sf))
    except (OSError, ValueError): seen = {}
    if h_tp >= nt:
        seen = {}
    elif seen.get("height") != h_tp:
        seen = {"height": h_tp, "behind_since": time.time()}
    try:
        with open(sf + ".tmp", "w") as f: json.dump(seen, f)
        os.replace(sf + ".tmp", sf)
    except OSError as e:
        print(f"cannot check the tip run: {sf}: {e}"); sys.exit(2)
    behind = time.time() - seen["behind_since"] if seen else 0
    w = os.environ.get("CHECK_TIP_RUN_LEAVES")
    held = sum(1 for n in os.listdir(w) if n.startswith("leaf_") and n.endswith(".bin")) if w and os.path.isdir(w) else None
    win = int(os.environ.get("CHECK_TIP_RUN_LEAF_WINDOW", "0") or 0)
    if behind > stale:
        why = (f"paused by its leaf window ({held} bundles waiting to be proved, HAZYNC_PREP_LEAF_WINDOW {win}): tip blocks "
               f"are not being proved (TIP_CLAIM_FIRST, the sponsor bot)") if win and held is not None and held >= win else \
              "is libzync-fetch (follow) running and fetching? (journalctl -u libzync-fetch -u libzync-prep)"
        print(f"TIP RUN STALLED: its stores have been at {h_tp} for {behind:.0f} s while the node is at {nt}: {why}")
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
