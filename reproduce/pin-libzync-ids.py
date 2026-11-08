#!/usr/bin/env python3
"""Pin the published libzync image ids (ROADMAP H7) from the reproducible container's output.

    python3 reproduce/pin-libzync-ids.py <ids file> <source commit>

<ids file>: the output of reproduce/libzync.Dockerfile (`docker run --rm <image>`), one line per network:
    libzync-product <network> METHOD_ID <64 hex> GUEST_ELF_SHA256 <64 hex>
Two builds must agree before pinning: pass the output of one, after diffing it with the other.

Writes, and changes nothing else:
  - reproduce/LIBZYNC_IDS: a header naming the source commit, then the lines verbatim;
  - swiftsync-verify/src/lib.rs IMAGES: each network's method_id_hex, and its profile set to the PRODUCT profile
    (libzync + coin log + counting trie + live-coin hash, plus signet's bit);
  - swiftsync-verify-ffi/src/lib.rs IDS: the same ids, in network order (mainnet, signet).
swiftsync-verify's unit test `images_match_the_published_ids` then fails if the table and LIBZYNC_IDS ever disagree.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PRODUCT = {"mainnet": "PROFILE_LIBZYNC | PROFILE_COINLOG | PROFILE_TRIE | PROFILE_UTXOHASH",
           "signet": "PROFILE_LIBZYNC | PROFILE_SIGNET | PROFILE_COINLOG | PROFILE_TRIE | PROFILE_UTXOHASH"}
LINE = re.compile(r"^libzync-product (mainnet|signet) METHOD_ID ([0-9a-f]{64}) GUEST_ELF_SHA256 ([0-9a-f]{64})$")


def main(argv):
    if len(argv) != 3:
        sys.exit(__doc__)
    lines = [l.strip() for l in Path(argv[1]).read_text().splitlines() if l.strip().startswith("libzync-product ")]
    ids = {}
    for l in lines:
        m = LINE.match(l)
        if not m:
            sys.exit(f"not an id line: {l!r}")
        ids[m.group(1)] = m.group(2)
    if set(ids) != {"mainnet", "signet"} or len(lines) != 2:
        sys.exit(f"want exactly one mainnet and one signet line, got {lines}")
    commit = argv[2]
    if not re.fullmatch(r"[0-9a-f]{7,40}", commit):
        sys.exit("source commit: a git sha")

    (ROOT / "reproduce/LIBZYNC_IDS").write_text(
        f"# Published libzync image ids (frozen core c14, commit {commit} source): reproduce/libzync.Dockerfile output,\n"
        f"# verbatim below, reproduced by two builds. Product profile: coin log, counting trie, live-coin hash.\n"
        + "\n".join(lines) + "\n")

    p = ROOT / "swiftsync-verify/src/lib.rs"
    s = p.read_text()
    for net, mid in ids.items():
        # the Image block of this network: from `network: "<net>",` to the next `},`
        block = re.compile(r'(network: "' + net + r'",\s*method_id_hex: ")[0-9a-f]{64}(",\s*profile: )[^,]+(,)', re.S)
        s, n = block.subn(lambda m: m.group(1) + mid + m.group(2) + PRODUCT[net] + m.group(3), s)
        if n != 1:
            sys.exit(f"{p}: found {n} IMAGES entries for {net}, want 1")
    p.write_text(s)

    p = ROOT / "swiftsync-verify-ffi/src/lib.rs"
    s = p.read_text()
    new = 'static IDS: [&str; 2] = [\n    "' + ids["mainnet"] + '\\0",\n    "' + ids["signet"] + '\\0",\n];'
    s, n = re.subn(r'static IDS: \[&str; 2\] = \[\s*"[0-9a-f]{64}\\0",\s*"[0-9a-f]{64}\\0",\s*\];', lambda m: new, s)
    if n != 1:
        sys.exit(f"{p}: found {n} IDS tables, want 1")
    p.write_text(s)
    print("pinned:", ", ".join(f"{k} {v[:8]}" for k, v in ids.items()), "from", commit)


if __name__ == "__main__":
    main(sys.argv)
