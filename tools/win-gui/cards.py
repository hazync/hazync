"""Which graphics card each worker gets, on a machine with more than one.

⛔ WHY THIS EXISTS. Every worker the window started shared ONE lock and none was told which card to
use, so a machine with two cards proved on one of them and the other sat idle — the lock made the
second worker wait its turn for a card it was never going to use. Measured from the code on
2026-10-09, after the question "what about people who have more than one card?".

⭐ ONE WORKER PER CARD. Each worker is started seeing exactly one card (`CUDA_VISIBLE_DEVICES`) and
takes a lock that belongs to that card alone, so two cards prove two blocks at the same time and two
workers on the SAME card still take turns.

⭐ PURE, AND TESTED WITHOUT A CARD. `parse` reads nvidia-smi's text and `plan` decides; nothing here
runs a program or imports tkinter. supervisor.py does the asking and hazync_gui.py the starting.

⚠ A MACHINE WITH ONE USABLE CARD IS LEFT EXACTLY AS IT WAS. `plan` returns no card at all for it, so
nothing is pinned, the lock is the one it always was, and the prover picks its card the way it
always has. Everything new here happens only when there are two or more cards that can prove.

⛔ NOT YET RUN ON A REAL TWO-CARD MACHINE. The pinning is the mechanism the Linux fleet scripts
already use (`prover/rangecluster.sh`), and that each child really receives its own card and lock
is tested through real child processes — but nobody has watched two cards prove under this window.
"""
import re

# nvidia-smi's column order for QUERY below. ⚠ index and uuid come first because a card's NAME is the
# one free-text field, and two identical cards have the same name — which is the ordinary case for
# anyone who bought two of the same.
QUERY = "index,uuid,name,memory.total,driver_version,compute_cap"

# sppark keeps cards at compute capability 7.0 and newer; see supervisor.check_gpu.
FLOOR = 7

# One measured prove at segment size 21 peaked near 22 GB; each step down roughly halves it. The
# same table as supervisor._SEG_PO2_FOR_VRAM, kept there as the source and passed in by the caller.


def parse(text):
    """nvidia-smi's CSV for QUERY -> [{index, uuid, name, vram_mb, driver, cc_major, cc_minor}].

    ⚠ A line it cannot read is skipped, never guessed at. A card with no index cannot be pinned, and
    pinning the wrong one is worse than not pinning.
    """
    out, seen = [], set()
    for line in (text or "").strip().splitlines():
        parts = [x.strip() for x in line.split(",")]
        if len(parts) < 6 or not parts[0].isdigit():
            continue
        mb = re.search(r"(\d+)", parts[3])
        cc = re.match(r"(\d+)\.(\d+)", parts[5])
        index = int(parts[0])
        if not (mb and cc) or index in seen:
            continue
        seen.add(index)
        out.append({"index": index, "uuid": parts[1], "name": parts[2], "vram_mb": int(mb.group(1)),
                    "driver": parts[4], "cc_major": int(cc.group(1)), "cc_minor": int(cc.group(2))})
    return out


def usable(cards, floor=FLOOR):
    """The cards that can prove, in nvidia-smi's own order."""
    return [c for c in cards if c["cc_major"] >= floor]


def labels(cards):
    """{index: what to call this card on screen}.

    ⚠ Two identical cards are the common case, so a name alone would give two lines a person cannot
    tell apart. They are numbered only when that happens: "RTX 3080" stays "RTX 3080" beside a
    different card, and becomes "RTX 3080 (1)" and "RTX 3080 (2)" beside its twin.
    """
    short = {c["index"]: re.sub(r"^NVIDIA\s+(GeForce\s+)?", "", c["name"]).strip() or c["name"]
             for c in cards}
    out, count = {}, {}
    for c in cards:
        n = short[c["index"]]
        if list(short.values()).count(n) > 1:
            count[n] = count.get(n, 0) + 1
            out[c["index"]] = f"{n} ({count[n]})"
        else:
            out[c["index"]] = n
    return out


def key(card):
    """What a card is remembered by between runs: its UUID, which does not change when a card is
    added or moved to another slot. ⚠ The index does, so a choice saved by index would silently
    come to mean a different card."""
    return card.get("uuid") or f"index-{card['index']}"


def chosen(cards, skip=(), floor=FLOOR):
    """The usable cards a person has NOT switched off.

    ⛔ NEVER NONE AT ALL. Every card switched off would leave Start with nothing to start, on a
    machine whose cards all work — so a list that excludes everything is read as excluding
    nothing. (The window also refuses to untick the last card; this is the second fence.)
    """
    ok = usable(cards or [], floor)
    skip = set(skip or ())
    return [c for c in ok if key(c) not in skip] or ok


def plan(cards, job, n_workers, cuda_build=True, skip=(), floor=FLOOR):
    """Which card each worker gets: a list, one entry per worker to start, each a card or None.

    None means "do not pin this worker" — it runs exactly as every worker did before this existed.
    `skip` is the cards a person switched off, by `key`.

    ⚠ THE COUNT CAN GROW, NEVER SHRINK. Asked for one worker on a machine with three usable cards,
    this returns three, because the point is that no card sits idle. Asked for four on two cards it
    returns four, two to a card, which is what somebody who set four meant.

    ⛔ THE ANCHOR JOB IS NOT MULTIPLIED. It is strictly one step after another against one chain, so
    a second worker can only race the first: it fetches the head, waits, and by the time it runs
    the head has moved (see coordinator/run-workers.sh). It keeps the count it was given.
    """
    n = max(1, int(n_workers))
    ok = usable(cards or [], floor)
    if not cuda_build or len(ok) < 2:
        return [None] * n
    # ⚠ PINNED EVEN WHEN ONE CARD IS LEFT. Somebody who switched a card off wants it left alone,
    # and an unpinned worker makes no such promise: the prover would choose for itself.
    ok = chosen(cards, skip, floor)
    if job == "spine":
        best = max(ok, key=lambda c: (c["cc_major"], c["cc_minor"], c["vram_mb"]))
        return [best] * n
    n = max(n, len(ok))
    return [ok[i % len(ok)] for i in range(n)]


def seg_po2_for(card, configured, table):
    """The segment size for a worker on `card`, given what the settings say.

    ⚠ ONE SETTING, CARDS OF DIFFERENT SIZES. The setting is worked out for the most capable card.
    Beside it may sit a smaller one — a 24 GB card and an 8 GB one is an ordinary pairing — and the
    big card's size would run the small one out of memory on its first piece. So a smaller card gets
    its own smaller size. It is only ever LOWERED: a size somebody set low on purpose stays.

    ⚠ Blank stays blank. Nothing was chosen, so nothing is chosen on their behalf here either.
    """
    s = str(configured or "").strip()
    if not s.isdigit() or card is None:
        return s or None
    own = next(po2 for least, po2 in table if card["vram_mb"] >= least)
    return str(min(int(s), own))


def pin_env(card):
    """The environment that makes a worker see exactly this card. {} for no card.

    ⛔⛔ THE NUMBER ALONE IS NOT ENOUGH. `CUDA_VISIBLE_DEVICES=1` means "the second card in CUDA's
    order", and CUDA's default order is fastest-first while nvidia-smi's is by PCI slot. On a
    machine with an old card in the first slot and a new one in the second, the two orders are
    REVERSED — so the worker meant for the new card would be handed the old one, and the reverse.
    `CUDA_DEVICE_ORDER=PCI_BUS_ID` makes CUDA count the way nvidia-smi does.
    """
    if card is None:
        return {}
    return {"CUDA_DEVICE_ORDER": "PCI_BUS_ID", "CUDA_VISIBLE_DEVICES": str(card["index"])}


def lock_name(card):
    """The lock file's name for a worker on `card`. One lock per card, shared by nothing else."""
    return "hazync-gpu.lock" if card is None else f"hazync-gpu-{card['index']}.lock"
