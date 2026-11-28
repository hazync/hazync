#!/usr/bin/env python3
"""verify_manifest.py <tag> <SHA256SUMS.txt> <SHA256SUMS.txt.asc>: exit 0 only when the manifest is signed by the
maintainer key pinned in appupdate.py (SECURITY.md) and its signed first line names <tag>. windows-installer.yml runs it
before trusting a release's hashes (review R3-6 #8, 28.11.2026): a manifest on the release is not evidence by itself."""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import appupdate  # noqa: E402


def main(argv):
    if len(argv) != 3:
        print("usage: verify_manifest.py <tag> <SHA256SUMS.txt> <SHA256SUMS.txt.asc>", file=sys.stderr)
        return 2
    tag, mpath, apath = argv
    try:
        manifest, asc = open(mpath, "rb").read(), open(apath, "rb").read()
    except OSError as e:
        print(f"::error::{e}")
        return 1
    ok, why = appupdate.check_manifest(tag, manifest, asc)
    print(("" if ok else "::error::") + f"{mpath}: {why}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
