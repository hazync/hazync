#!/usr/bin/env python3
"""Carry the original board's contributors onto the libzync board at the switch (operator decision, 2026-11-09).

Copies each contributor's identity from the original coordinator's database to the libzync one: pubkey, handle,
first_seen, last_version, last_version_at. NOT `blocks`: that counts blocks proved under the CORE image, whose proofs
do not carry over, so the libzync board starts every count at 0. A pubkey already on the libzync board is left exactly
as it is. The original database is opened read-only and never changed. Nothing else carries over (sponsorships and
the donations record stay with the original database, by the same decision).

  carry-contributors.py --from /var/lib/hazync/coordinator.db --to /var/lib/hazync-lz/coordinator.db           # plan
  carry-contributors.py --from ... --to ... --apply                                                             # copy
Exit: 0 done (or nothing to do), 2 a database could not be opened or has no contributors table.
Run it with the libzync coordinator STOPPED (step 4.2 of SWITCH.md), so nothing writes the table meanwhile.
"""
import argparse
import sqlite3
import sys

COLS = ("pubkey", "handle", "first_seen", "last_version", "last_version_at")


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--from", dest="src", required=True)
    ap.add_argument("--to", dest="dst", required=True)
    ap.add_argument("--apply", action="store_true")
    a = ap.parse_args(argv)
    try:
        src = sqlite3.connect(f"file:{a.src}?mode=ro", uri=True)
        dst = sqlite3.connect(f"file:{a.dst}?mode={'rw' if a.apply else 'ro'}", uri=True)
        rows = src.execute(f"SELECT {','.join(COLS)} FROM contributors ORDER BY first_seen").fetchall()
        have = {r[0] for r in dst.execute("SELECT pubkey FROM contributors")}
    except sqlite3.Error as e:
        print(f"[carry] cannot read the databases: {e}")
        return 2
    new = [r for r in rows if r[0] not in have]
    print(f"[carry] {len(rows)} contributors on the original board, {len(rows) - len(new)} already on the libzync "
          f"board (left as they are), {len(new)} to carry")
    for r in new:
        print(f"  {r[0][:16]}…  handle {r[1]!r}")
    if not a.apply:
        print("[carry] plan only: run with --apply to copy")
        return 0
    with dst:
        dst.executemany(f"INSERT OR IGNORE INTO contributors({','.join(COLS)},blocks) VALUES(?,?,?,?,?,0)", new)
    n = dst.execute("SELECT count(*) FROM contributors").fetchone()[0]
    print(f"[carry] copied {len(new)}; the libzync board now has {n} contributors (blocks 0 for every carried one)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
