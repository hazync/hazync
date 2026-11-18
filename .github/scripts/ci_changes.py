#!/usr/bin/env python3
"""Which of adversarial.yml's heavy jobs a pull request needs, from the files it changes.

    git diff --no-renames --name-only BASE HEAD | python3 -I ci_changes.py <repo root>   -> image=… / soundness=…
    python3 -I ci_changes.py --selftest [--control]

Two answers, each "true" or "false":

  image      the reproducible libzync container and its ids could change (reproducible-image-id, and the hosts).
  soundness  the libzync soundness pass could change (libzync-hosts and every job that needs it). image implies it.

FAIL-SAFE BY CONSTRUCTION: a path is "light" (runs neither) only when a rule below names it as light; a path no
rule names is treated as an IMAGE change. A new top-level directory, a new root file, anything unforeseen, runs
everything. The heavy rules are checked first, so a light pattern can never hide a heavy file.

The heavy set is worked out from the code, not listed by hand where it can be derived:
  * every crate the prover workspace reaches by a Cargo `path =` dependency, transitively (libzync, coinbase-smt,
    accumulator, swiftsync-verify, zapps/headers, zapps/hints and vendor/* today): their files are image inputs,
    except files of a NESTED package that is not itself in that set (zapps/hints/cli, …/wasm, …/ffi);
  * every file those crates include_str!/include_bytes! by a literal relative path;
  * the coordinator files libzync-chain runs (coord_test.sh, board_rehearsal.sh: server.py, the `hazync` CLI, web/)
    and any coordinator module either of them imports.

Run from the BASE commit's copy in CI (adversarial.yml `changes`), so a pull request cannot loosen the filter that
judges it: a change to this file is a change under .github/, which runs everything.
"""
import os
import re
import sys
from fnmatch import fnmatchcase

# Always image (the full run): the workflows and this filter, the container recipe and everything it consumes.
IMAGE_PREFIXES = (
    ".github/",
    "reproduce/",            # Dockerfiles, libzync-ids.sh, LIBZYNC_IDS (the pins the jobs compare against)
    "prover/",               # the guest, the hosts, the zyncAPP guests, their lockfiles and rust-toolchain.toml
    "vendor/",
    "patches/",              # applied to Core's source by libzync-base.sh, copied into the deps stage
    "coreshim/",
)
IMAGE_FILES = {"provision-vps.sh", ".dockerignore", "scripts/libzync-base.sh"}

# Soundness only (the hosts and the jobs that run them; not the image ids).
SOUNDNESS_PREFIXES = (
    "scripts/libzync/",               # ci/ drivers (build-hosts.sh runs inside the container), the *_ref.py oracles
    "experimental/libzync-bench/",    # lib.sh's $B: leaf/join/prep/coord/handover/core_rules/cost harnesses
)
COORD_RUNTIME = ("coordinator/server.py", "coordinator/hazync", "coordinator/mod_block.txt")
COORD_RUNTIME_PREFIXES = ("coordinator/web/",)

# Light: nothing any heavy job builds or runs. Checked only after every heavy rule above.
LIGHT_PATTERNS = (
    "*.md", "LICENSE", ".gitignore",
    "docs/*", "history/*", "tools/*", "ooga-land/*", "zapps/*",
    "guest-pure-fuzz/*", "swiftsync-verify-ffi/*", "swiftsync-verify-wasm/*",
    "coordinator/*",          # the rest of it: tests, deploy/, the tip fleet tooling (accumulator-tests runs those)
    "scripts/*",              # the rest of it: release and check scripts (accumulator-tests runs those)
    "experimental/README.md",
)


def path_dep_closure(root):
    """The repo-relative dirs of every crate prover/ reaches through `path =` dependencies, prover's own included."""
    seen, todo = set(), []
    for d, _, fs in os.walk(os.path.join(root, "prover")):
        if "target" in d.split(os.sep):
            continue
        if "Cargo.toml" in fs:
            todo.append(os.path.relpath(d, root))
    while todo:
        d = os.path.normpath(todo.pop())
        if d in seen or d.startswith(".."):
            continue
        seen.add(d)
        t = os.path.join(root, d, "Cargo.toml")
        if not os.path.isfile(t):
            continue
        for m in re.finditer(r'path\s*=\s*"([^"]+)"', open(t, encoding="utf-8", errors="replace").read()):
            p = os.path.normpath(os.path.join(d, m.group(1)))
            if not p.endswith(".rs"):
                todo.append(p)
    return seen


def nested_packages(root, crate_dirs):
    """Dirs holding a Cargo.toml of their own, inside a closure crate, that are NOT in the closure themselves."""
    out = set()
    for c in crate_dirs:
        for d, _, fs in os.walk(os.path.join(root, c)):
            if "target" in d.split(os.sep):
                continue
            rel = os.path.relpath(d, root)
            if rel != c and "Cargo.toml" in fs and rel not in crate_dirs:
                out.add(rel)
    return out


def include_targets(root, crate_dirs):
    """Files the closure crates pull in with include_str!/include_bytes! of a literal relative path."""
    out = set()
    pat = re.compile(r'include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)')
    for c in crate_dirs:
        for d, _, fs in os.walk(os.path.join(root, c)):
            if "target" in d.split(os.sep):
                continue
            for f in fs:
                if f.endswith(".rs"):
                    src = open(os.path.join(d, f), encoding="utf-8", errors="replace").read()
                    for m in pat.finditer(src):
                        out.add(os.path.normpath(os.path.relpath(os.path.join(d, m.group(1)), root)))
    return out


def coordinator_imports(root):
    """Local coordinator modules that server.py or the hazync CLI import (none today; kept so one cannot slip in)."""
    out = set()
    cdir = os.path.join(root, "coordinator")
    mods = {f[:-3] for f in os.listdir(cdir) if f.endswith(".py")} if os.path.isdir(cdir) else set()
    for f in ("server.py", "hazync"):
        p = os.path.join(cdir, f)
        if not os.path.isfile(p):
            continue
        for m in re.finditer(r"^\s*(?:from\s+([A-Za-z_]\w*)|import\s+([A-Za-z_][\w, ]*))", open(p, encoding="utf-8", errors="replace").read(), re.M):
            names = [m.group(1)] if m.group(1) else [n.strip().split(" ")[0] for n in m.group(2).split(",")]
            out |= {f"coordinator/{n}.py" for n in names if n in mods}
    return out


class Classifier:
    def __init__(self, root, control=False):
        self.control = control
        self.crates = {c for c in path_dep_closure(root) if not c.startswith("prover")}
        self.nested = nested_packages(root, self.crates)
        self.includes = include_targets(root, self.crates)
        self.coord = set(COORD_RUNTIME) | coordinator_imports(root)

    def classify(self, p):
        """'image', 'soundness' or 'light'; the first rule that names the path decides, heavy rules first."""
        if self.control:          # positive control: the filter with every heavy rule removed must fail the selftest
            return "light"
        if p.startswith(IMAGE_PREFIXES) or p in IMAGE_FILES or p in self.includes:
            return "image"
        for c in self.crates:
            if p.startswith(c + "/") and not any(p.startswith(n + "/") for n in self.nested):
                return "image"
        if p.startswith(SOUNDNESS_PREFIXES) or p in self.coord or p.startswith(COORD_RUNTIME_PREFIXES):
            return "soundness"
        if any(fnmatchcase(p, g) for g in LIGHT_PATTERNS):
            return "light"
        return "image"           # unnamed: run everything


def decide(classes):
    image = any(c == "image" for c in classes)
    return image, image or any(c == "soundness" for c in classes)


SELFTEST = [  # (path, expected class) against this checkout
    ("docs/libzync/LIBZYNC.md", "light"),
    ("README.md", "light"),
    ("prover/README.md", "image"),                      # under an image prefix: heavy rules come first
    ("tools/win-gui/app.py", "light"),
    ("zapps/hints/cli/src/main.rs", "light"),           # a nested package outside the closure
    ("zapps/hints/src/lib.rs", "image"),                # zyncapp-host depends on zapps/hints
    ("zapps/headers/Cargo.toml", "image"),
    ("swiftsync-verify/src/lib.rs", "image"),           # through zapps/headers -> swiftsync-verify
    ("swiftsync-verify-wasm/src/lib.rs", "light"),
    ("libzync/src/lib.rs", "image"),
    ("coinbase-smt/src/lib.rs", "image"),
    ("vendor/risc0-zkvm/src/lib.rs", "image"),
    ("prover/Cargo.lock", "image"),
    ("prover/rust-toolchain.toml", "image"),
    ("reproduce/LIBZYNC_IDS", "image"),
    ("experimental/libzync-bench/headers/h0_200.txt", "image"),   # include_str! from zapps/headers
    ("experimental/libzync-bench/leaf/leaf_test.sh", "soundness"),
    ("scripts/libzync/ci/chain.sh", "soundness"),
    ("scripts/libzync-base.sh", "image"),
    ("scripts/check-workflows.sh", "light"),
    ("coordinator/server.py", "soundness"),
    ("coordinator/hazync", "soundness"),
    ("coordinator/web/index.html", "soundness"),
    ("coordinator/test_claim_nonce.py", "light"),
    ("coordinator/deploy/headers-api/hazync-headers-api", "light"),
    (".github/workflows/adversarial.yml", "image"),
    (".github/scripts/ci_changes.py", "image"),
    ("provision-vps.sh", "image"),
    ("some-new-dir/thing.rs", "image"),                 # unnamed -> everything
    ("Cargo.toml", "image"),
]


def selftest(root, control):
    c = Classifier(root, control=control)
    bad = [(p, w, c.classify(p)) for p, w in SELFTEST if c.classify(p) != w]
    for p, w, g in bad:
        print(f"MISMATCH {p}: want {w}, got {g}")
    checks = [(decide([]), (False, False)), (decide(["light"]), (False, False)),
              (decide(["light", "soundness"]), (False, True)), (decide(["soundness", "image"]), (True, True))]
    bad += [x for x in checks if x[0] != x[1]]
    if not c.control and "swiftsync-verify" not in c.crates:
        bad.append(("closure", "has swiftsync-verify", sorted(c.crates)))
    if bad:
        print(f"CI_CHANGES_SELFTEST_FAILED {len(bad)}")
        return 1
    print(f"CI_CHANGES_SELFTEST_OK {len(SELFTEST)} paths; closure: {' '.join(sorted(c.crates))}")
    return 0


def main(argv):
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    if "--selftest" in argv:
        return selftest(root, "--control" in argv)
    root = argv[1] if len(argv) > 1 else root
    files = [l.strip() for l in sys.stdin if l.strip()]
    c = Classifier(root)
    classes = [c.classify(p) for p in files]
    for p, k in zip(files, classes):
        print(f"{k:9} {p}", file=sys.stderr)
    image, soundness = decide(classes)
    if not files:             # no list is not "nothing changed": a diff that failed upstream looks the same
        print("no changed files read: running everything", file=sys.stderr)
        image = soundness = True
    print(f"image={'true' if image else 'false'}")
    print(f"soundness={'true' if soundness else 'false'}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
