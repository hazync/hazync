#!/usr/bin/env python3
"""Z3's side-by-side check (docs/libzync/ZAPP_FOLLOWER.md §10): before the relay switches to zync-serve's tip mode, both
services must answer the same facts at the same height. Asks each for /tip, then, at the height both serve, every fact
kind for every script given (the old service at its tip, the new one with ?at=<that height>), and compares the JSON
bodies byte for byte. Read-only.

  hazync-lz-zapp-compare.py [--old URL] [--new URL] [SPK_HEX ...]
    --old  today's zync-serve (default http://127.0.0.1:8601)
    --new  zync-serve --tip (default http://127.0.0.1:8602)
    SPK    output scripts, hex (default: three early pay-to-pubkey scripts: blocks 1 and 9's coinbases, and the first
           transaction's payment in block 170)
Exit: 0 every answer equal; 1 a difference (each printed); 2 could not compare (a service down, no common height).
"""
import json, sys, urllib.error, urllib.request

DEFAULT_SPKS = [
    "410496b538e853519c726a2c91e61ec11600ae1390813a627c66fb8be7947be63c52da7589379515d4e0a604f8141781e62294721166bf621e73a82cbf2342c858eeac",
    "410411db93e1dcdb8a016b49840f8c53bc1eb68a382e97b1482ecad7b148a6909a5cb2e0eaddfb84ccf9744464f82e160bfa9b8b64f9d4c03f999b8643f656b412a3ac",
    "4104ae1a62fe09c5f51b13905f07f06b99a2f7159b2225f374cd378d71302fa28414e7aab37397f554a7df5f142c21c1b7303b8a0626f1baded5c72a704f7e6cd84cac",
]


def get(url):
    try:
        with urllib.request.urlopen(url, timeout=60) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def main(argv):
    old, new, spks = "http://127.0.0.1:8601", "http://127.0.0.1:8602", []
    i = 0
    while i < len(argv):
        if argv[i] in ("--old", "--new"):
            if argv[i] == "--old": old = argv[i + 1]
            else: new = argv[i + 1]
            i += 2
        else:
            spks.append(argv[i].lower()); i += 1
    spks = spks or DEFAULT_SPKS
    try:
        a = json.loads(get(old + "/tip")[1])["tip"]
        b = json.loads(get(new + "/tip")[1])
    except (OSError, ValueError, KeyError) as e:
        print(f"COULD NOT COMPARE: /tip: {e}"); return 2
    if a not in b.get("at", [b.get("tip")]):
        print(f"COULD NOT COMPARE: the old service is at {a}, the new one serves {b.get('at')}: wait for both to follow the spine")
        return 2
    paths = [f"/trie/{k}/{s}" for s in spks for k in ("balance", "history")]
    # a coin fact for an outpoint that does not exist at that height answers "absent" on both: comparable too
    paths += [f"/trie/coin/{s}/{'00' * 32}/0" for s in spks]
    diff = 0
    for p in paths:
        x, y = get(old + p), get(new + p + f"?at={a}")
        if x != y:
            diff += 1
            print(f"DIFFERENT {p[:60]}: old {x[0]} {x[1][:120]!r} | new {y[0]} {y[1][:120]!r}")
    print(f"{len(paths) - diff} of {len(paths)} facts equal at {a}")
    return 1 if diff else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
