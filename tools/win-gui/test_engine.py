#!/usr/bin/env python3
"""The app on the engine (OpenVM): host.exe drives, hzengine.exe proves, keys\\ holds the image (headless).

    python3 test_engine.py             # the real logic
    python3 test_engine.py --control   # the app as it was (no engine awareness): must fail

⭐ THE LAYOUT (engine/RELEASE_TOOLING.md, the installer): prover\\host.exe, prover\\hzengine.exe, prover\\keys\\ (the
release's hazync-keys-core.tar.gz unpacked). With HAZYNC_ENGINE + HAZYNC_ENGINE_KEYS in its environment the host
proves through the engine and `host method-id` prints the ENGINE image id, read from keys\\image.json
(prover/host/src/engine_backend.rs). The fake host below does exactly that, and prints the RISC Zero id otherwise.

Asserted: a host alone is still the RISC Zero host it was; hzengine + keys beside it make the worker's environment
carry HAZYNC_ENGINE / HAZYNC_ENGINE_KEYS and the image check ask the host in that environment; the person's own
HAZYNC_ENGINE / HAZYNC_ENGINE_KEYS win; a key folder missing a file, or a missing hzengine, is fatal and named; an
engine image that is not the board's (or, board unknown, not this app's) is fatal; prover_image and the stale-prover
advice speak engine ids for an engine host.
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()

import hashlib
import json
import os
import stat
import sys
import tempfile
from pathlib import Path

import supervisor  # noqa: E402

CONTROL = "--control" in sys.argv
fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


EXE = "11" * 32
VM = "00e00077" + "22" * 28
ENGINE_ID = hashlib.sha256(bytes.fromhex(EXE) + bytes.fromhex(VM)).hexdigest()

FAKE_HOST = f'''#!{sys.executable}
import hashlib, json, os, sys
if sys.argv[1:] == ["method-id"]:
    if os.environ.get("HAZYNC_ENGINE_VK"):     # the real host: the id the KEY commits to, under its pin (board_key)
        vk = open(os.environ["HAZYNC_ENGINE_VK"], "rb").read()
        if hashlib.sha256(vk).hexdigest() != os.environ.get("HAZYNC_ENGINE_VK_PIN"):
            print("HAZYNC_ENGINE_VK is not the pinned key", file=sys.stderr); sys.exit(1)
        v = json.loads(vk)                     # (the fake key "encodes" its exe / vm as JSON)
        print("METHOD_ID " + hashlib.sha256(bytes.fromhex(v["exe"]) + bytes.fromhex(v["vm"])).hexdigest())
    elif os.environ.get("HAZYNC_ENGINE"):
        keys = os.environ["HAZYNC_ENGINE_KEYS"]
        try:
            v = json.load(open(os.path.join(keys, "image.json")))
        except OSError as e:
            print(f"{{keys}}/image.json: {{e}}", file=sys.stderr); sys.exit(101)
        print("METHOD_ID " + hashlib.sha256(bytes.fromhex(v["exe"]) + bytes.fromhex(v["vm"])).hexdigest())
    else:
        print("METHOD_ID {supervisor.CANONICAL_METHOD_ID}")
    sys.exit(0)
sys.exit(2)
'''


def executable(p, text):
    p.write_text(text)
    p.chmod(p.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def layout(root, engine=True, keys=True, drop=None, exe=EXE, vm=VM, key_exe=None):
    d = root / "prover"
    d.mkdir(parents=True)
    host = d / "host"
    executable(host, FAKE_HOST)
    if engine:
        executable(d / "hzengine", "#!/bin/sh\nexit 0\n")
    if keys:
        k = d / "keys"
        k.mkdir()
        for f in supervisor.ENGINE_KEY_FILES:
            if f == drop:
                continue
            (k / f).write_text(json.dumps({"exe": exe, "vm": vm}) if f == "image.json"
                               else json.dumps({"exe": key_exe or exe, "vm": vm}) if f == "vk.bin" else "x")
    return host


def main():
    if CONTROL:   # the app as it was: nothing knows the engine is there
        supervisor.engine_sidecar = lambda host_path, base_env=None: None
    clean = {k: v for k, v in os.environ.items() if k not in ("HAZYNC_ENGINE", "HAZYNC_ENGINE_KEYS")}
    os.environ.clear(); os.environ.update(clean)

    with tempfile.TemporaryDirectory() as t:
        t = Path(t)

        print("a RISC Zero host alone is unchanged")
        h0 = layout(t / "r0", engine=False, keys=False)
        r = supervisor.check_host_binary(str(h0))
        check(r.ok and "canonical METHOD_ID" in r.detail, f"RISC Zero host passes as before: {r.detail[:70]}")
        env = supervisor.worker_env(str(h0), "w", None, None, base_env={})
        check("HAZYNC_ENGINE" not in env and "HAZYNC_ENGINE_KEYS" not in env, "no engine variables for it")

        print("hzengine + keys beside the host: the engine")
        h1 = layout(t / "eng")
        side = supervisor.engine_sidecar(str(h1), {})
        check(bool(side) and side["source"] == "beside", f"found beside the host: {side}")
        env = supervisor.worker_env(str(h1), "w", None, None, base_env={})
        check(env.get("HAZYNC_ENGINE") == str((h1.parent / "hzengine").resolve()),
              f"worker env HAZYNC_ENGINE = the hzengine beside it ({env.get('HAZYNC_ENGINE')})")
        check(env.get("HAZYNC_ENGINE_KEYS") == str((h1.parent / "keys").resolve()),
              f"worker env HAZYNC_ENGINE_KEYS = keys beside it ({env.get('HAZYNC_ENGINE_KEYS')})")
        check(env.get("HAZYNC_HOST") == str(h1), "HAZYNC_HOST still the host (it drives)")
        r = supervisor.check_host_binary(str(h1), board_id=ENGINE_ID)
        check(r.ok and "engine image" in r.detail, f"the board's engine image passes: {r.detail[:90]}")
        r = supervisor.check_host_binary(str(h1), board_id=supervisor.CANONICAL_METHOD_ID)
        check(not r.ok and r.fatal and "keys" in r.detail, f"a board on another image is fatal: {r.detail[:90]}")
        r = supervisor.check_host_binary(str(h1), board_id=None, board_why="no coordinator")
        check(not r.ok and r.fatal and "this app was built for" in r.detail,
              f"board unknown: compared with this app's engine image: {r.detail[:90]}")
        check(supervisor.prover_image(str(h1)) == ENGINE_ID, "prover_image is the ENGINE id for an engine host")
        a = supervisor.stale_prover_advice(str(h1), ENGINE_ID, ENGINE_ID, installed={})
        check(a is None, "no stale advice when the engine image is the board's")
        a = supervisor.stale_prover_advice(str(h1), ENGINE_ID, None, installed={})
        check(a is not None and supervisor.CANONICAL_ENGINE_IMAGE_ID[:8] in a["text"],
              "stale advice compares an engine host with this app's ENGINE image")

        print("the app's own engine image, board unknown")
        canon_exe, canon_vm = "33" * 32, "44" * 32
        h2 = layout(t / "canon", exe=canon_exe, vm=canon_vm)
        saved = supervisor.CANONICAL_ENGINE_IMAGE_ID
        supervisor.CANONICAL_ENGINE_IMAGE_ID = hashlib.sha256(bytes.fromhex(canon_exe + canon_vm)).hexdigest()
        r = supervisor.check_host_binary(str(h2))
        check(r.ok, f"passes against this app's engine image: {r.detail[:80]}")
        supervisor.CANONICAL_ENGINE_IMAGE_ID = saved

        print("the key, not image.json, is the image (review R3-6 #9)")
        h5 = layout(t / "mixed", key_exe="55" * 32)
        r = supervisor.check_host_binary(str(h5), board_id=ENGINE_ID)
        check(not r.ok and r.fatal and "disagree" in r.detail,
              f"⛔ image.json naming the board's image over another image's key is fatal: {r.detail[:90]}")
        saved_pin = supervisor.CANONICAL_ENGINE_VK_PIN
        supervisor.CANONICAL_ENGINE_VK_PIN = "00" * 32
        r = supervisor.check_host_binary(str(h1), board_id=ENGINE_ID)
        check(not r.ok and r.fatal and "published key" in r.detail,
              f"⛔ a key that is not the release's published pin is fatal: {r.detail[:90]}")
        supervisor.CANONICAL_ENGINE_VK_PIN = hashlib.sha256((h1.parent / "keys" / "vk.bin").read_bytes()).hexdigest()
        r = supervisor.check_host_binary(str(h1), board_id=ENGINE_ID)
        check(r.ok, f"the published key passes: {r.detail[:70]}")
        supervisor.CANONICAL_ENGINE_VK_PIN = saved_pin

        print("what is missing is named, and fatal")
        h3 = layout(t / "nokey", drop="agg.vk")
        r = supervisor.check_host_binary(str(h3), board_id=ENGINE_ID)
        check(not r.ok and r.fatal and "agg.vk" in r.detail, f"a key file missing: {r.detail[:90]}")
        h4 = layout(t / "nokeys", keys=False)
        r = supervisor.check_host_binary(str(h4), board_id=ENGINE_ID)
        check(not r.ok and r.fatal and "hazync-keys-core.tar.gz" in r.detail, f"no key folder: {r.detail[:90]}")

        print("the first-run checklist: the prover step needs the engine's parts too")
        import firstrun  # noqa: E402
        def host_step(h):
            return next(x for x in firstrun.setup_steps({"host": str(h)}) if x.key == "host")
        st = host_step(h3)
        check(not st.done and "agg.vk" in st.detail, f"keys missing: the prover step is not done ({st.detail[-70:]})")
        st = host_step(h1)
        check("hzengine" in st.detail, f"complete: the step names the engine ({st.detail[-70:]})")

        print("the person's own environment wins")
        other = t / "elsewhere"; other.mkdir()
        executable(other / "hzengine", "#!/bin/sh\nexit 0\n")
        mine = {"HAZYNC_ENGINE": str(other / "hzengine"), "HAZYNC_ENGINE_KEYS": str(h1.parent / "keys")}
        env = supervisor.worker_env(str(h0), "w", None, None, base_env=dict(mine))
        check(env.get("HAZYNC_ENGINE") == mine["HAZYNC_ENGINE"], "HAZYNC_ENGINE from the environment kept")
        os.environ.update({"HAZYNC_ENGINE": str(t / "nothere" / "hzengine"), "HAZYNC_ENGINE_KEYS": str(h1.parent / "keys")})
        r = supervisor.check_host_binary(str(h0), board_id=ENGINE_ID)
        check(not r.ok and r.fatal and "not there" in r.detail, f"an environment hzengine that is missing: {r.detail[:90]}")
        os.environ.clear(); os.environ.update(clean)

    print()
    if fails:
        print(f"FAILED {len(fails)}" + (" (control: expected)" if CONTROL else ""))
        sys.exit(1)
    print("all passed")


if __name__ == "__main__":
    main()
