#!/usr/bin/env python3
"""run_vectors.py <verifier binary> -- run every case of manifest.json against a verifier that takes
swiftsync-verify's arguments, and compare exit codes. Run from this directory. Exit 1 if any case differs."""
import json, os, subprocess, sys
here = os.path.dirname(os.path.abspath(__file__))
verifier = os.path.abspath(sys.argv[1])
m = json.load(open(os.path.join(here, "manifest.json")))
bad = 0
for c in m["cases"]:
    r = subprocess.run([verifier, *c["args"]], cwd=here, capture_output=True, text=True,
                       env={k: v for k, v in os.environ.items() if k != "RISC0_DEV_MODE"})
    last = (r.stdout.strip().splitlines() or [""])[-1]
    ok = r.returncode == c["expect_exit"]
    bad += not ok
    print(f"{'PASS' if ok else 'WRONG'}  {c['name']:<20} exit {r.returncode} (want {c['expect_exit']})  {last[:110]}")
print(f"{len(m['cases']) - bad}/{len(m['cases'])} cases as expected")
sys.exit(1 if bad else 0)
