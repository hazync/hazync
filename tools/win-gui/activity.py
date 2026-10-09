"""Turn the worker's output into what a person wants to know: what is it doing, and how far along.

⛔ WHY THIS EXISTS. The window used to show progress as a scrolling log on a tab nobody was looking
at. A prove takes minutes on a GPU and the better part of an hour on a CPU, and for all of that time
the only honest reading of a still window is "it broke". Everything needed to say otherwise is
already in the worker's own output — which block, which piece of how many, how long is left — it
just was not being read.

⭐ PURE, AND TESTED WITHOUT A WINDOW. `Activity.feed(line)` takes one line and updates four plain
fields; nothing here imports tkinter. hazync_gui.py only puts them on screen.

    headline   one short sentence: what is happening right now
    detail     the smaller line under it: which piece, how long left
    fraction   0.0-1.0 when the work has a measurable length, None when it does not
    done       how many blocks / folds / anchor steps have LANDED this session

⚠ THE FRACTION IS NEVER INVENTED. A prove reports `segment i/n`, so it gets a real bar. A fold and an
anchor step print nothing between starting and finishing, so they report None and the window shows
a moving bar instead of a number it would have had to make up.

⚠ A LINE THIS DOES NOT RECOGNISE CHANGES NOTHING. The worker's output is not an API and will drift;
when it does, the headline goes stale rather than wrong, and the window's own "last heard from the
prover Ns ago" clock still shows the process is alive.
"""
import re

JOBS = ("run", "fold", "spine")

# What each job is called on screen, and the one sentence that says what it does. The window's three
# choices and the feed both read these, so the words cannot disagree between the two places.
JOB_NAMES = {"run": "Prove", "fold": "Fold", "spine": "Anchor"}
JOB_BLURBS = {
    "run": "Prove a block of Bitcoin's history. The heavy work, and what the project needs most.",
    "fold": "Combine two finished proofs into one. Light work, fine on a machine without a GPU.",
    "spine": "Join finished proofs onto the chain back to the first block. One step at a time.",
}
_UNITS = {"run": ("block", "blocks"), "fold": ("fold", "folds"), "spine": ("step", "steps")}

_STARTING = {
    "run": "Asking for a block to prove…",
    "fold": "Looking for two proofs to combine…",
    "spine": "Looking for the next proof to anchor…",
}

# The share of a prove's bar each phase gets. Segments are almost all of the time, so they get
# almost all of the bar; the ends are small fixed slices so the bar moves the moment work starts
# and is not already full while the proof is still being checked.
_SEG_FROM, _SEG_TO = 0.04, 0.86
_ASSEMBLING, _ASSEMBLED, _PROVED = 0.88, 0.93, 0.96

_RE = {
    "claimed": re.compile(r"claimed block (\S+)"),
    "nothing": re.compile(r"nothing to claim right now:\s*(.*)"),
    "executed": re.compile(r"executed, (\d+) segments"),
    "segment": re.compile(r"segment (\d+)/(\d+)\s+(\d+)s elapsed(?:, ~(\d+)s left)?"),
    "assembling": re.compile(r"assembling (\d+) segment receipts"),
    "assembled": re.compile(r"assembled (\d+) segment receipts"),
    "proved": re.compile(r"proved range \[(\d+)\.\.(\d+)\].*? in ([\d.]+)s"),
    "accepted": re.compile(r"range (\S+): the coordinator re-verified"),
    "taken": re.compile(r"range (\S+) was already proved by someone else"),
    "gpu_wait": re.compile(r"waited (\d+)s for the GPU"),
    "still": re.compile(r"still proving, (\d+)m elapsed"),
    "folding": re.compile(r"folding (\S+) \+ (\S+) -> (\S+)"),
    "folded": re.compile(r"(\S+): folded and verified on the board"),
    "fold_lost": re.compile(r"(\S+) was folded by someone else first"),
    "no_fold": re.compile(r"nothing to fold"),
    "absorbing": re.compile(r"absorbing (\S+) into \[1\.\.(\d+)\]"),
    "anchored": re.compile(r"spine now \[1\.\.(\d+)\]"),
    "ahead": re.compile(r"another extender is ahead"),
    "not_proven": re.compile(r"block (\d+) is not proven yet"),
    "no_absorb": re.compile(r"nothing starting at block (\d+) would absorb"),
}


def pretty(rng):
    """`144263` -> `144,263`; `137121-137128` -> `137,121–137,128`. Anything else is left alone."""
    parts = str(rng).split("-")
    if not all(p.isdigit() for p in parts) or not 1 <= len(parts) <= 2:
        return str(rng)
    return "–".join(f"{int(p):,}" for p in parts)


def clock(seconds):
    """A duration a person reads at a glance: `45s`, `3m 05s`, `1h 12m`."""
    s = max(0, int(seconds))
    if s < 60:
        return f"{s}s"
    if s < 3600:
        return f"{s // 60}m {s % 60:02d}s"
    return f"{s // 3600}h {(s % 3600) // 60:02d}m"


def tally(job, n):
    """`3 blocks proven`, `1 fold done` — the session count, in the job's own words."""
    one, many = _UNITS.get(job, _UNITS["run"])
    verb = {"run": "proven", "fold": "done", "spine": "anchored"}.get(job, "done")
    return f"{n:,} {one if n == 1 else many} {verb}"


class Activity:
    """What one worker is doing, kept current by feeding it the worker's output line by line."""

    def __init__(self, job="run"):
        self.job = job if job in JOBS else "run"
        self.headline = _STARTING[self.job]
        self.detail = ""
        self.fraction = None
        self.done = 0
        self.block = None          # the block or range in hand, as the worker printed it
        self.waiting = False       # True while there is simply nothing to do yet

    def _set(self, headline, detail="", fraction=None, waiting=False):
        self.headline, self.detail, self.fraction, self.waiting = headline, detail, fraction, waiting

    def feed(self, line):
        """Read one line of worker output. Returns a sentence for the feed, or None.

        ⚠ Only things that HAPPENED return a sentence. Progress within a step (segment 12 of 51)
        moves the bar and the detail line but is not an event, or the feed would be nothing else.
        """
        s = line.strip()
        if not s:
            return None
        return (self._prove(s) if self.job == "run" else
                self._fold(s) if self.job == "fold" else
                self._spine(s))

    # ── proving ─────────────────────────────────────────────────────────────────────────────────
    def _prove(self, s):
        m = _RE["claimed"].search(s)
        if m:
            self.block = m.group(1)
            self._set(f"Proving block {pretty(self.block)}", "Fetching the block's data…", 0.0)
            return f"Took block {pretty(self.block)} to prove"
        m = _RE["nothing"].search(s)
        if m:
            self._set("No block is free right now", "Waiting, then asking again. " + m.group(1)[:120],
                      None, waiting=True)
            return "No block was free — waiting to ask again"
        m = _RE["executed"].search(s)
        if m:
            n = int(m.group(1))
            self.detail = f"Split into {n:,} piece{'' if n == 1 else 's'} — starting on the first"
            self.fraction = _SEG_FROM / 2
            return None
        m = _RE["segment"].search(s)
        if m:
            i, n = int(m.group(1)), max(1, int(m.group(2)))
            left = f" · about {clock(int(m.group(4)))} left" if m.group(4) else ""
            self.detail = f"Piece {i:,} of {n:,}{left}"
            self.fraction = _SEG_FROM + (_SEG_TO - _SEG_FROM) * min(i, n) / n
            self.waiting = False
            return None
        if _RE["assembled"].search(s):
            self.detail, self.fraction = "Pieces joined — finishing the proof", _ASSEMBLED
            return None
        if _RE["assembling"].search(s):
            self.detail, self.fraction = "Joining the pieces into one proof…", _ASSEMBLING
            return None
        m = _RE["proved"].search(s)
        if m:
            lo, hi, secs = m.group(1), m.group(2), float(m.group(3))
            rng = lo if lo == hi else f"{lo}-{hi}"
            self._set(f"Proof of block {pretty(rng)} finished", "Sending it to be checked…", _PROVED)
            return f"Finished the proof of block {pretty(rng)} in {clock(secs)}"
        m = _RE["accepted"].search(s)
        if m:
            self.done += 1
            self._set(f"Block {pretty(m.group(1))} is on the board", "Checked and accepted. "
                      "Asking for the next one…", 1.0)
            return f"Block {pretty(m.group(1))} checked and accepted — it is on the board"
        m = _RE["taken"].search(s)
        if m:
            self._set("Asking for a block to prove…", "", None)
            return f"Someone else finished block {pretty(m.group(1))} first — moving on"
        m = _RE["gpu_wait"].search(s)
        if m:
            self.detail = f"Waited {clock(int(m.group(1)))} for the graphics card to be free"
            return None
        m = _RE["still"].search(s)
        if m and self.fraction is None:
            self.detail = f"Still proving — {m.group(1)} min so far"
        return None

    # ── folding ─────────────────────────────────────────────────────────────────────────────────
    def _fold(self, s):
        m = _RE["folding"].search(s)
        if m:
            self.block = m.group(3)
            self._set(f"Combining proofs for blocks {pretty(self.block)}",
                      "Two proofs become one. This usually takes under a minute on a GPU.", None)
            return None
        m = _RE["folded"].search(s)
        if m:
            self.done += 1
            rng = m.group(1).lstrip("✓").strip()
            self._set(f"Blocks {pretty(rng)} combined", "Checked and accepted. Looking for the "
                      "next pair…", 1.0)
            return f"Combined the proofs for blocks {pretty(rng)}"
        m = _RE["fold_lost"].search(s)
        if m:
            return f"Someone else combined {pretty(m.group(1))} first — moving on"
        if _RE["no_fold"].search(s):
            self._set("Nothing to combine right now", "Every finished pair is already combined. "
                      "Waiting for new proofs…", None, waiting=True)
            return "Nothing to combine — waiting for new proofs"
        return None

    # ── anchoring ───────────────────────────────────────────────────────────────────────────────
    def _spine(self, s):
        m = _RE["absorbing"].search(s)
        if m:
            self.block = m.group(1)
            self._set(f"Anchoring blocks {pretty(self.block)}",
                      f"Joining them onto the chain, which reaches block {int(m.group(2)):,}.", None)
            return None
        m = _RE["anchored"].search(s)
        if m:
            self.done += 1
            top = int(m.group(1))
            self._set(f"The chain is anchored up to block {top:,}",
                      "Checked and accepted. Looking for the next step…", 1.0)
            return f"Anchored the chain up to block {top:,}"
        if _RE["ahead"].search(s):
            return "Someone else anchored that step first — moving on"
        m = _RE["not_proven"].search(s) or _RE["no_absorb"].search(s)
        if m:
            self._set("Nothing to anchor right now",
                      f"Block {int(m.group(1)):,} comes next and is not ready yet. Waiting…",
                      None, waiting=True)
            return f"Nothing to anchor yet — block {int(m.group(1)):,} is not ready"
        return None
