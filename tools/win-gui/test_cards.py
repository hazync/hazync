#!/usr/bin/env python3
"""One worker per graphics card: who gets which card, tested without owning two.

    python3 test_cards.py             # the real plan
    python3 test_cards.py --control   # the plan as it was, which must get these wrong

⛔ WHAT THIS CANNOT SHOW. That two real cards prove at once. Nobody has run this on a two-card
machine yet; what is tested is the DECISION (here) and that each child process really receives its
own card and its own lock (test_home_flow.py, through real children).
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()

import sys

import cards  # noqa: E402
import supervisor  # noqa: E402

fails = 0
TABLE = supervisor._SEG_PO2_FOR_VRAM

# nvidia-smi's own output shape for cards.QUERY. The first is the pod this was developed against
# (2026-10-09, `nvidia-smi --query-gpu=index,uuid,name,memory.total,driver_version,compute_cap`).
ONE = "0, GPU-3a1f0c52-0000-0000-0000-000000000001, NVIDIA A40, 46068 MiB, 550.127.05, 8.6\n"
TWINS = ("0, GPU-aaaa, NVIDIA GeForce RTX 3080, 10240 MiB, 566.14, 8.6\n"
         "1, GPU-bbbb, NVIDIA GeForce RTX 3080, 10240 MiB, 566.14, 8.6\n")
OLD_FIRST = ("0, GPU-old, NVIDIA GeForce GTX 1060, 6144 MiB, 566.14, 6.1\n"
             "1, GPU-new, NVIDIA GeForce RTX 3080, 10240 MiB, 566.14, 8.6\n")
BIG_SMALL = ("0, GPU-big, NVIDIA GeForce RTX 4090, 24564 MiB, 566.14, 8.9\n"
             "1, GPU-sml, NVIDIA GeForce RTX 3060 Ti, 8192 MiB, 566.14, 8.6\n")
THREE = OLD_FIRST + "2, GPU-third, NVIDIA GeForce RTX 2070, 8192 MiB, 566.14, 7.5\n"


def check(ok, what):
    global fails
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails += 1


def ix(p):
    """The card index each worker got; None for a worker that was not given one."""
    return [c["index"] if c else None for c in p]


def checks(plan=cards.plan, pin_env=cards.pin_env, lock_name=cards.lock_name,
           seg=cards.seg_po2_for):
    print("── 1. reading the cards ──")
    got = cards.parse(TWINS)
    check([c["index"] for c in got] == [0, 1] and got[1]["uuid"] == "GPU-bbbb"
          and got[0]["vram_mb"] == 10240 and (got[0]["cc_major"], got[0]["cc_minor"]) == (8, 6),
          "index, uuid, memory and compute capability are read for each card")
    check(cards.parse("") == [] and cards.parse("No devices were found\n") == []
          and cards.parse(None) == [], "no cards, or words instead of cards, is an empty list")
    check(len(cards.parse(TWINS + "garbage, line\n" + TWINS)) == 2,
          "a line it cannot read is skipped, and a card listed twice is one card")

    print("── 2. ⛔ two cards, two workers — the whole point ──")
    p = plan(cards.parse(TWINS), "run", 1)
    check(len(p) == 2, f"asked for ONE worker on a two-card machine, it starts two (got {len(p)})")
    check(ix(p) == [0, 1], "one on each card, not two on the first")
    envs = [pin_env(c) for c in p]
    check([e.get("CUDA_VISIBLE_DEVICES") for e in envs] == ["0", "1"],
          "each worker is shown exactly its own card")
    check(len({lock_name(c) for c in p}) == 2,
          "⛔ and each card has its OWN lock — one shared lock is what left a card idle")
    check(all(e.get("CUDA_DEVICE_ORDER") == "PCI_BUS_ID" for e in envs),
          "⛔ CUDA is told to number the cards the way nvidia-smi does — its own order is "
          "fastest-first, which REVERSES an old card in slot 0 and a new one in slot 1")

    print("── 3. more workers than cards still take turns per card ──")
    p = plan(cards.parse(TWINS), "run", 4)
    check(ix(p) == [0, 1, 0, 1], "four workers on two cards: two to a card")
    check(lock_name(p[0]) == lock_name(p[2]) != lock_name(p[1]),
          "two workers on the same card share that card's lock")

    print("── 4. a machine with one usable card is left exactly as it was ──")
    for name, text in (("one card", ONE), ("an old card beside a new one", OLD_FIRST),
                       ("no cards", "")):
        p = plan(cards.parse(text), "run", 2)
        check(p == [None, None] and pin_env(p[0]) == {} and lock_name(p[0]) == "hazync-gpu.lock",
              f"{name}: nothing pinned, the same lock, the same count")
    check(plan(cards.parse(TWINS), "run", 1, cuda_build=False) == [None],
          "the CPU build is never pinned to a card")
    check(plan(cards.parse(TWINS), "run", 1, per_card=False) == [None],
          "and with the setting off, nothing changes at all")

    print("── 5. cards that cannot prove get no worker ──")
    p = plan(cards.parse(THREE), "run", 1)
    check(ix(p) == [1, 2],
          "⛔ a GTX 1060 beside two RTX cards: workers on the two RTX cards only — a worker "
          "pinned to the 1060 would fail on its first block, over and over")
    p = plan(cards.parse(THREE), "run", 1, floor=6)
    check(ix(p) == [0, 1, 2], "with the floor lowered on purpose, it is included")

    print("── 6. ⛔ anchoring is not multiplied ──")
    p = plan(cards.parse(TWINS), "spine", 1)
    check(len(p) == 1 and p[0] is not None,
          "one anchor worker on a two-card machine: a second can only race the first")
    p = plan(cards.parse(BIG_SMALL), "spine", 1)
    check(ix(p) == [0], "and it goes on the most capable card")
    check(len(plan(cards.parse(TWINS), "fold", 1)) == 2, "folding does use every card")

    print("── 7. a smaller card beside a bigger one gets a smaller segment size ──")
    big, small = cards.parse(BIG_SMALL)
    check(seg(big, "20", TABLE) == "20", "the 24 GB card keeps the size that was set")
    check(seg(small, "20", TABLE) == "18",
          "⛔ the 8 GB card is lowered to its own — the big card's size would run it out of memory")
    check(seg(small, "17", TABLE) == "17", "a size set LOWER on purpose is never raised")
    check(seg(small, "", TABLE) is None and seg(None, "20", TABLE) == "20",
          "blank stays blank, and an unpinned worker gets exactly what was set")

    print("── 8. two identical cards can be told apart on screen ──")
    lab = cards.labels(cards.parse(TWINS))
    check(lab == {0: "RTX 3080 (1)", 1: "RTX 3080 (2)"}, f"twins are numbered: {lab}")
    lab = cards.labels(cards.parse(BIG_SMALL))
    check(lab == {0: "RTX 4090", 1: "RTX 3060 Ti"}, f"different cards keep their plain names: {lab}")

    print("── 9. the environment a pinned worker is started with ──")
    c = cards.parse(TWINS)[1]
    env = supervisor.worker_env("h", "w", None, None, base_env={}, force_shim=False, card=c)
    check(env.get("CUDA_VISIBLE_DEVICES") == "1" and env["HAZYNC_GPU_LOCK"].endswith("hazync-gpu-1.lock"),
          "worker_env carries the card and its lock")
    env = supervisor.worker_env("h", "w", None, None, force_shim=False,
                                base_env={"CUDA_VISIBLE_DEVICES": "3"})
    check(env.get("CUDA_VISIBLE_DEVICES") == "3" and "CUDA_DEVICE_ORDER" not in env
          and env["HAZYNC_GPU_LOCK"].endswith("hazync-gpu.lock"),
          "⚠ with no card given, a person's own CUDA_VISIBLE_DEVICES is left alone")


# ── the plan as it was ───────────────────────────────────────────────────────────────────────────
def naive_plan(found, job, n_workers, cuda_build=True, per_card=True, floor=cards.FLOOR):
    """What the window did: N workers, none of them told which card."""
    return [None] * max(1, int(n_workers))


def naive_pin(card):
    """The obvious pin: the card's number, and nothing about how CUDA counts."""
    return {} if card is None else {"CUDA_VISIBLE_DEVICES": str(card["index"])}


def control():
    global fails
    print("── control: every worker shares one lock and no card is chosen ──")
    checks(plan=naive_plan, pin_env=naive_pin, lock_name=lambda c: "hazync-gpu.lock",
           seg=lambda card, configured, table: (str(configured).strip() or None))
    broke, fails = fails, 0
    print()
    if broke < 8:
        print(f"⛔ CONTROL BROKEN: only {broke} checks failed against the old behaviour")
        return 1
    print(f"CONTROL OK: the old behaviour fails {broke} checks the real plan passes")
    return 0


def main():
    checks()
    print()
    if fails:
        print(f"FAIL {fails}")
        return 1
    print("All checks passed.")
    return 0


if __name__ == "__main__":
    sys.exit(control() if "--control" in sys.argv else main())
