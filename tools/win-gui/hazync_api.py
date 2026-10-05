"""The coordinator API, and the block map's geometry — pure, so both can be TESTED.

⛔ NOTHING HERE IS INVENTED. Every endpoint and every state code was read off the live API and out of
`coordinator/server.py`, because a dashboard that displays a number it guessed the meaning of is
worse than one that displays nothing.

    /api/state        progress{proven, folded, folds, frontier, tip, pct, contributors, spine_hi}
    /api/blockstatus  {tip, spine_hi, frontier, runs: [[lo, hi, state], ...]}
    /api/blockstatus?prover=<handle>   only the blocks that prover PROVED (not their folds)
    /api/block/<h>    everything about one block
    /api/meta         method_id, frontier

⭐ THE STATE CODES COME FROM server.py's OWN DOCSTRING, not from reading the colours off a page:

    3 proven · 4 folded · 5 anchored · "A block in no run is open."

⇒ So the OPEN blocks — the ones a prover can actually claim — are the GAPS BETWEEN RUNS. That is the
one derived fact the dashboard needs and the API does not state directly, and it is why `open_ranges`
exists here with a test rather than inline in a widget.

⚠ Claims are deliberately absent from /api/blockstatus ("they change by the second"), so the map
shows what is PROVED, never what someone is currently working on. A "pick a block" button that
assumed otherwise would hand out work already in progress.
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()   # ⛔ BEFORE anything prints: a ✅ on a cp1252 console raises, not degrades

import json
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_COORD = "https://api.hazync.org"

# server.py: "[[lo, hi, status], ...] with 3 proven, 4 folded and 5 anchored". OPEN is ours: it is
# the absence of a run, not a code the API emits.
PROVEN, FOLDED, ANCHORED = 3, 4, 5
OPEN = 0

STATE_NAMES = {OPEN: "open", PROVEN: "proven", FOLDED: "folded", ANCHORED: "anchored"}


class ApiError(Exception):
    pass


def fetch(path, coord=DEFAULT_COORD, timeout=20):
    """GET a JSON endpoint. Raises ApiError with something a person can act on."""
    url = coord.rstrip("/") + path
    req = urllib.request.Request(url, headers={"User-Agent": "hazync-win-gui"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as e:
        raise ApiError(f"{url} returned HTTP {e.code}") from e
    except urllib.error.URLError as e:
        raise ApiError(f"could not reach {url}: {e.reason}") from e
    except (ValueError, TimeoutError) as e:
        raise ApiError(f"{url} did not return usable JSON: {e}") from e


def progress(coord=DEFAULT_COORD, timeout=20):
    """The headline numbers. Returns the `progress` object, or raises ApiError.

    ⚠ Keys are passed through rather than renamed: `spine_hi` IS the anchored height, and calling it
    something friendlier in here would make it harder to check against the API by hand.
    """
    d = fetch("/api/state", coord, timeout)
    p = d.get("progress")
    if not isinstance(p, dict):
        raise ApiError("/api/state has no `progress` object — the API shape changed")
    return p


def block_status(coord=DEFAULT_COORD, prover=None, timeout=30):
    """(meta, runs). `prover` narrows runs to the blocks that prover PROVED."""
    path = "/api/blockstatus" + (f"?prover={urllib.parse.quote(prover)}" if prover else "")
    d = fetch(path, coord, timeout)
    runs = d.get("runs")
    if not isinstance(runs, list):
        raise ApiError("/api/blockstatus has no `runs` — the API shape changed")
    meta = {k: d.get(k) for k in ("tip", "spine_hi", "frontier", "prover")}
    return meta, [(int(a), int(b), int(c)) for a, b, c in runs]


def block_detail(height, coord=DEFAULT_COORD, timeout=20):
    return fetch(f"/api/block/{int(height)}", coord, timeout)


def open_ranges(runs, upto=None):
    """The ranges in NO run — i.e. the blocks a prover can claim. [(lo, hi), ...]

    ⛔ THIS IS THE ONE THING THE API DOES NOT SAY DIRECTLY, so it is derived here and tested.
    server.py: "A block in no run is open." Runs arrive sorted and non-overlapping; this does not
    assume that, because a sort order is a property of today's server and not of the contract.

    ⚠ `upto` bounds the answer. Without it, everything above the highest run looks open all the way
    to infinity, and the honest ceiling is the FRONTIER — the highest block the bridge has published
    a bundle for. Offering a block beyond it is offering work nobody can start.
    """
    if not runs:
        return [(1, upto)] if upto else []
    rs = sorted((int(a), int(b)) for a, b, _ in runs)
    gaps, cursor = [], 1
    for lo, hi in rs:
        if lo > cursor:
            gaps.append((cursor, lo - 1))
        cursor = max(cursor, hi + 1)
    if upto is not None and cursor <= upto:
        gaps.append((cursor, upto))
    if upto is None:
        return gaps
    # ⛔⛔ CLIP TO `upto`, DO NOT MERELY STOP AT IT. Runs exist ABOVE the frontier — tip blocks are
    # proved out of order, e.g. 969,305 — so the span between the frontier region and those runs is
    # a gap by construction. Unclipped, that reported 826,567 "open" blocks below a frontier of
    # 142,140, which is impossible on its face and is how the bug was caught: a derived number was
    # checked against a bound it could not exceed.
    out = []
    for lo, hi in gaps:
        if lo > upto:
            continue
        out.append((lo, min(hi, upto)))
    return out


def count_open(runs, upto):
    return sum(hi - lo + 1 for lo, hi in open_ranges(runs, upto))


def state_at(runs, height):
    """The state of one block: PROVEN / FOLDED / ANCHORED, or OPEN if no run covers it."""
    best = OPEN
    for lo, hi, st in runs:
        if lo <= height <= hi:
            best = max(best, st)      # server.py layers folds over proofs, genesis over both
    return best


def first_open(runs, upto, count=1):
    """The lowest `count` open heights — what a 'pick the next open block' button should offer.

    ⭐ LOWEST FIRST, DELIBERATELY. The project's whole direction is extending the genesis-anchored
    spine upward, and a proof only joins the spine when everything below it is proved. Handing out
    the newest open block instead produces proofs that cannot be anchored for a very long time.
    """
    out = []
    for lo, hi in open_ranges(runs, upto):
        h = lo
        while h <= hi and len(out) < count:
            out.append(h)
            h += 1
        if len(out) >= count:
            break
    return out


def map_rows(runs, upto, per_row, rows_max=None):
    """Compress the chain into rows of cells for a block map, each cell a (state, lo, hi) bucket.

    ⚠ ONE SQUARE PER BLOCK DOES NOT FIT ON A SCREEN. The website's map is "every block mined so far,
    one square each" and the chain is ~970k blocks; a window shows thousands of cells, not a million.
    So each cell covers a RANGE, and its state is the WORST state in that range — because a cell that
    showed its best state would paint a mostly-open chain as proved, which is the opposite of useful.
    """
    if upto is None or upto < 1 or per_row < 1:
        return [], 1
    total_cells = per_row * (rows_max or 1_000_000)
    per_cell = max(1, -(-upto // total_cells))      # ceil, so the whole chain fits
    rows, row = [], []
    lo = 1
    while lo <= upto:
        hi = min(upto, lo + per_cell - 1)
        row.append((worst_state(runs, lo, hi), lo, hi))
        if len(row) == per_row:
            rows.append(row)
            row = []
        lo = hi + 1
    if row:
        rows.append(row)
    return rows, per_cell


def worst_state(runs, lo, hi):
    """The LEAST advanced state in [lo, hi] — OPEN if any block in the span is unproved.

    ⛔ Deliberately pessimistic. A bucket of 500 blocks with one open block is not "proven", and a
    map that rounded it up would hide exactly the gaps a prover is looking for.

    ⚠ INTERVAL ARITHMETIC, NOT PER-BLOCK. The first version of this walked `range(a, b+1)` for every
    overlapping run, which against the genesis run (1..136,360) and a few thousand cells is
    quadratic — tens of billions of iterations to draw one map. This is O(runs) per cell.
    """
    clipped = [(max(a, lo), min(b, hi), st) for a, b, st in runs if not (b < lo or a > hi)]
    if not clipped:
        return OPEN
    # Is the whole span covered? Merge the clipped intervals and compare the union's length.
    clipped.sort()
    cursor, union = lo, 0
    for a, b, _ in clipped:
        if a > cursor:
            return OPEN                  # a hole before this interval: something is open
        if b >= cursor:
            union += b - cursor + 1
            cursor = b + 1
    if union < hi - lo + 1:
        return OPEN
    return min(st for _, _, st in clipped)


def summarise_progress(p):
    """One line a person can read, from the `progress` object."""
    return (f"{p.get('proven', 0):,} proven · {p.get('folded', 0):,} folded · "
            f"{p.get('spine_hi', 0):,} anchored · {p.get('pct', 0)}% of "
            f"{p.get('tip', 0):,} · {p.get('contributors', 0)} contributors")
