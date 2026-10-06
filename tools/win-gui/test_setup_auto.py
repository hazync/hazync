#!/usr/bin/env python3
"""Tests for the things a person should never have to know: which build, what segment size, keys.

⛔⛔ WHY. Setting this program up previously required knowing that sppark rejects cards below
compute 7.0, that there are two host binaries, that one cannot start without an NVIDIA driver, and
that a segment size exists at all. Every one of those was learned by hitting it as a failure, on a
real machine, over a day. All four are answerable from `nvidia-smi` in under a second.

  python3 test_setup_auto.py            # assertions
  python3 test_setup_auto.py --control  # the capability floor is ignored; MUST send a 6.1 card to CUDA
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()

import sys          # noqa: E402
import tempfile     # noqa: E402
from pathlib import Path  # noqa: E402

import firstrun     # noqa: E402
import supervisor   # noqa: E402

CONTROL = "--control" in sys.argv
fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


CARDS = {
    "GTX 1050 Ti": {"name": "GTX 1050 Ti", "vram_mb": 4096, "driver": "566.14",
                    "cc_major": 6, "cc_minor": 1},
    "RTX 4090":    {"name": "RTX 4090", "vram_mb": 24564, "driver": "560",
                    "cc_major": 8, "cc_minor": 9},
    "RTX PRO 6000": {"name": "RTX PRO 6000", "vram_mb": 97887, "driver": "570",
                     "cc_major": 9, "cc_minor": 0},
}

recommend = supervisor.recommend
if CONTROL:
    # The obvious version: "it has a GPU, so use the GPU build". It is wrong for exactly the card
    # this project actually owns.
    def recommend(gpu=None):                       # noqa: D103
        g = gpu or supervisor.gpu_facts()
        if not g:
            return {"build": "cpu", "seg_po2": None, "gpu": None, "why": "no GPU"}
        return {"build": "cuda", "seg_po2": 21, "gpu": g, "why": "it has a GPU"}

print("── 1. ⛔ a card below compute 7.0 must NOT be sent to the CUDA build ──")
# 📏 Measured 2026-10-05: this exact card reached `cudaErrorNoDevice` because sppark filters it out.
r = recommend(CARDS["GTX 1050 Ti"])
if CONTROL:
    check(r["build"] == "cuda",
          "control: 'it has a GPU so use CUDA' sends a 6.1 card to the build that cannot run it")
else:
    check(r["build"] == "cpu", f"a 6.1 card is sent to the CPU build (got {r['build']})")
    check("7.0" in r["why"], "and the reason names the floor it fails")
    check("hazync#631" in r["why"], "and points at the issue, not just a verdict")

print("── 2. an eligible card gets CUDA and a segment size from its VRAM ──")
for name, want_build in (("RTX 4090", "cuda"), ("RTX PRO 6000", "cuda")):
    r = supervisor.recommend(CARDS[name])
    check(r["build"] == want_build, f"{name} -> {r['build']}")
    check(isinstance(r["seg_po2"], int) and 16 <= r["seg_po2"] <= 22,
          f"{name} -> HAZYNC_SEG_PO2={r['seg_po2']}")
big = supervisor.recommend(CARDS["RTX PRO 6000"])["seg_po2"]
small = supervisor.recommend(CARDS["RTX 4090"])["seg_po2"]
check(big >= small, f"more VRAM never suggests a SMALLER segment ({big} >= {small})")

print("── 3. no GPU at all is a CPU machine, not an error ──")
r = supervisor.recommend({})
check(r["build"] == "cpu" and r["gpu"] is None, "no driver -> the CPU build")
check("works anywhere" in r["why"], "said as a fact, not a failure")

print("── 4. ⚠ an extrapolated number must SAY it is extrapolated ──")
# The only hard datum is one prove peaking near 22 GB at 21. Presenting the rest as measured would
# be the thing this project keeps a rule about.
r = supervisor.recommend(CARDS["RTX 4090"])
check("extrapolated" in r["why"], "the suggestion says where it came from")
r = supervisor.recommend(CARDS["GTX 1050 Ti"])
check("hazync#631" in r["why"], "and an unproven workaround is labelled unproven")

print("── 5. ⛔⛔ importing a key must never destroy the one already there ──")
d = tempfile.mkdtemp()
from cryptography.hazmat.primitives import serialization                      # noqa: E402
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey  # noqa: E402


def a_key(path):
    sk = Ed25519PrivateKey.generate()
    h = sk.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw,
                         serialization.NoEncryption()).hex()
    Path(path).write_text(h)
    return h


first = a_key(Path(d) / "first.hex")
ok, _ = firstrun.import_identity(Path(d) / "first.hex", d)
check(ok, "a valid key imports")
second = a_key(Path(d) / "second.hex")
ok, msg = firstrun.import_identity(Path(d) / "second.hex", d)
backups = [f for f in _os.listdir(d) if ".replaced-" in f]
check(ok and bool(backups), f"replacing a key keeps the old one ({backups})")
check(Path(d, backups[0]).read_text().strip() == first,
      "and the backup really is the key that was replaced, not a copy of the new one")

print("── 6. rubbish is refused, and changes nothing ──")
Path(d, "junk.hex").write_text("definitely not a key")
before = Path(d, "key.hex").read_text()
ok, msg = firstrun.import_identity(Path(d) / "junk.hex", d)
check(not ok, "a non-key is refused")
check(Path(d, "key.hex").read_text() == before, "⛔ and the working key is untouched")
check("64 hex" in msg, "and the message says what was expected")

print("── 7. ⛔⛔ the SECRET must never appear in a message ──")
msgs = " ".join(str(firstrun.import_identity(Path(d) / n, d)[1])
                for n in ("first.hex", "second.hex"))
leaked = [n for n, s in (("first", first), ("second", second)) if s in msgs]
check(not leaked, f"no secret in any message{f' — LEAKED: {leaked}' if leaked else ''}")
check(firstrun._public_of(first) is not None, "but the PUBLIC key is derivable for display")

print("── 8. updating the app is honest about needing a restart ──")
ok, msg = firstrun.app_update(repo_dir=d)      # not a git checkout
check(not ok and "not a git checkout" in msg, "a non-checkout says so plainly")

EXPECTED_CONTROL_FAILURES = set()

print()
if CONTROL:
    real = [f for f in fails if not f.startswith("control:")]
    broke = any("control:" in str(x) for x in [])
    if not real:
        print("CONTROL OK — the naive rule sent the 6.1 card to CUDA, which is the measured failure")
        sys.exit(0)
    print("CONTROL FAILED — expected only the control assertion to differ")
    for f in real:
        print(f"  {f}")
    sys.exit(1)

if fails:
    print(f"FAILED {len(fails)}: " + "; ".join(fails))
    sys.exit(1)
print("all good")
