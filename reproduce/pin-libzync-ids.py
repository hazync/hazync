#!/usr/bin/env python3
"""Pin the published libzync image ids (ROADMAP H7) from the reproducible container's output.

    python3 reproduce/pin-libzync-ids.py <ids file> <source commit>

<ids file>: the output of reproduce/libzync.Dockerfile (`docker run --rm <image>`), one line per network:
    libzync-product <network> METHOD_ID <64 hex> GUEST_ELF_SHA256 <64 hex>
Two builds must agree before pinning: pass the output of one, after diffing it with the other.

Writes, and changes nothing else:
  - reproduce/LIBZYNC_IDS: a header naming the source commit, then the lines verbatim;
  - swiftsync-verify/src/lib.rs IMAGES: each network's CURRENT image (its last entry). An unchanged id only has its
    profile set to the PRODUCT profile (libzync + coin log + counting trie + live-coin hash, plus signet's bit). A NEW
    id is APPENDED after the current one, which stays in the table so its proofs verify by id (a handover continues
    it; v0.26.0): the new entry copies the network's parameters, takes the PRODUCT profile, and its lineage from
    prover/methods/guest/src/lineage/<network>_coinlog.rs (the product variant's file). Record the superseded id in
    reproduce/METHOD_ID (mainnet) or a `// v... <network>` comment (signet), or the unit test fails;
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
    # the header-chain zyncAPP's line (ROADMAP N3) is kept verbatim, and swiftsync-verify's HEADERS_IMAGE_ID set from it
    HLINE = re.compile(r"^zyncapp-headers mainnet METHOD_ID ([0-9a-f]{64}) GUEST_ELF_SHA256 ([0-9a-f]{64})$")
    headers = [l.strip() for l in Path(argv[1]).read_text().splitlines() if l.strip().startswith("zyncapp-headers ")]
    if len(headers) != 1 or not HLINE.match(headers[0]):
        sys.exit(f"want exactly one zyncapp-headers mainnet line, got {headers}")
    hp = ROOT / "swiftsync-verify/src/headers.rs"
    hs = hp.read_text()
    hs2 = re.sub(r'pub const HEADERS_IMAGE_ID: &str = "[0-9a-f]{64}";',
                 f'pub const HEADERS_IMAGE_ID: &str = "{HLINE.match(headers[0]).group(1)}";', hs)
    if hs2.count("pub const HEADERS_IMAGE_ID") != 1:
        sys.exit(f"{hp}: HEADERS_IMAGE_ID not found")
    hp.write_text(hs2)
    commit = argv[2]
    if not re.fullmatch(r"[0-9a-f]{7,40}", commit):
        sys.exit("source commit: a git sha")

    (ROOT / "reproduce/LIBZYNC_IDS").write_text(
        f"# Published libzync image ids (frozen core c14, commit {commit} source): reproduce/libzync.Dockerfile output,\n"
        f"# verbatim below, reproduced by two builds. Product profile: coin log, counting trie, live-coin hash.\n"
        + "\n".join(lines) + "\n" + "".join(l + "\n" for l in headers))

    p = ROOT / "swiftsync-verify/src/lib.rs"
    s = p.read_text()
    for net, mid in ids.items():
        s = pin_network(s, net, mid, commit)
    p.write_text(s)

    p = ROOT / "swiftsync-verify-ffi/src/lib.rs"
    s = p.read_text()
    new = 'static IDS: [&str; 2] = [\n    "' + ids["mainnet"] + '\\0",\n    "' + ids["signet"] + '\\0",\n];'
    s, n = re.subn(r'static IDS: \[&str; 2\] = \[\s*"[0-9a-f]{64}\\0",\s*"[0-9a-f]{64}\\0",\s*\];', lambda m: new, s)
    if n != 1:
        sys.exit(f"{p}: found {n} IDS tables, want 1")
    p.write_text(s)
    print("pinned:", ", ".join(f"{k} {v[:8]}" for k, v in ids.items()), "from", commit)


def entries(s):
    """(start, end) of each `    Image {` ... `    },` block of IMAGES, in order."""
    a = s.index("pub const IMAGES: &[Image] = &[\n")
    b = s.index("\n];\n", a)
    out, i = [], a
    while True:
        i = s.find("    Image {\n", i, b)
        if i < 0:
            return out
        j = s.index("\n    },\n", i) + len("\n    },\n")
        out.append((i, j))
        i = j


def lineage_rust(net):
    """The product variant's pinned lineage, as swiftsync-verify writes it."""
    f = (ROOT / f"prover/methods/guest/src/lineage/{net}_coinlog.rs").read_text()
    m = re.search(r"pub const LINEAGE: &\[crate::handover_pin::Handover\] = (&\[.*?\]);", f, re.S)
    if not m:
        sys.exit(f"lineage/{net}_coinlog.rs: no LINEAGE")
    body = m.group(1).replace("crate::handover_pin::Handover", "Handover").replace("crate::handover_pin::", "pin::")
    return "\n".join(("        " + l if k else l) for k, l in enumerate(body.splitlines()))


def pin_network(s, net, mid, commit):
    blocks = [(i, j) for i, j in entries(s) if f'network: "{net}",' in s[i:j]]
    if not blocks:
        sys.exit(f"IMAGES has no {net} entry")
    i, j = blocks[-1]
    cur = s[i:j]
    old = re.search(r'method_id_hex: "([0-9a-f]{64})"', cur).group(1)
    prof = re.compile(r"(profile: )[^,]+(,)")
    if old == mid:
        return s[:i] + prof.sub(lambda m: m.group(1) + PRODUCT[net] + m.group(2), cur, count=1) + s[j:]
    if any(f'method_id_hex: "{mid}"' in s[a:b] for a, b in blocks):
        sys.exit(f"{net}: {mid[:8]} is already an earlier entry; refusing to re-append it")
    new = cur.replace(f'method_id_hex: "{old}"', f'method_id_hex: "{mid}"', 1)
    new = prof.sub(lambda m: m.group(1) + PRODUCT[net] + m.group(2), new, count=1)
    new, n = re.subn(r"lineage: &\[.*?\],\n    \},\n$", lambda m: f"lineage: {lineage_rust(net)},\n    }},\n", new, flags=re.S)
    if n != 1:
        sys.exit(f"{net}: the current entry has no lineage field")
    note = f"    // Pinned from {commit}: the CURRENT {net} image; the entry above stays so its proofs verify by id.\n"
    return s[:j] + note + new + s[j:]


if __name__ == "__main__":
    main(sys.argv)
