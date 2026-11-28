#!/usr/bin/env python3
"""Which of adversarial.yml's heavy jobs a pull request needs, from the files it changes.

    git diff --no-renames --name-only BASE HEAD | python3 -I ci_changes.py --base BASE   -> image=… / soundness=…
    python3 -I ci_changes.py --selftest [--control]

Two answers, each "true" or "false":

  image      the reproducible libzync container and its ids could change (reproducible-image-id, and the hosts).
  soundness  the libzync soundness pass could change (libzync-hosts and every job that needs it). image implies it.

FAIL-SAFE BY CONSTRUCTION: a path is "light" (runs neither) only when a rule below names it as light; a path no
rule names is treated as an IMAGE change. A new top-level directory, a new root file, anything unforeseen, runs
everything. The heavy rules are checked first, so a light pattern can never hide a heavy file.

Everything the rules are DERIVED from is read from the BASE tree (`--base REV`, git; the filesystem only for the
selftest), never from the tree under test, so a pull request cannot reshape the derivation that judges it (an empty
Cargo.toml dropped into a crate to make it look like a separate package, a driver script that stops naming a file).
Any added, changed or deleted Cargo.toml, build.rs, .cargo/ file, .gitattributes, .gitmodules or .lfsconfig anywhere is
an image change outright (the last three change what a checkout writes: line endings, encodings, LFS pointers).
coordinator/ is sys.path[0] when the heavy jobs run `cd $C; python3 server.py`: a coordinator module the base does
not have, a new directory there, or a module named like a stdlib module is a soundness change (coordinator_module()).

The heavy set is worked out from the code, not listed by hand where it can be derived:
  * every crate the prover workspace reaches by a Cargo `path =` dependency, transitively (libzync, coinbase-smt,
    accumulator, swiftsync-verify, zapps/headers, zapps/hints today; vendor/* until CUTOVER C8): their files are image inputs,
    except files of a NESTED package that is not itself in that set (zapps/hints/cli, …/wasm, …/ffi);
  * every file those crates pull in by a literal relative path: include_str!, include_bytes!, include! and
    #[path = "…"]. Wherever such a target lives (even under a light directory) it is an image input;
  * every repo file the heavy jobs EXECUTE OR READ, transitively (`references()`): the repo paths named in the heavy
    jobs' blocks of adversarial.yml, then in each script or file those name ($Z/…, $REPO/…, $HERE/../…, /hazync-zkvm/…,
    bare names beside a script, python modules imported from beside it), and so on. Each is a soundness input
    (coordinator/deploy/libzync/libzync-prep-run.sh, run by prep/rewind_test.sh and prep/crash_test.sh, is one; the
    rest of coordinator/deploy/libzync/, runbooks and units nothing here runs, is light);
  * the coordinator files libzync-chain runs (server.py, the `hazync` CLI, web/), named explicitly as well.
The reference scan over-approximates (a name that resolves to an existing file anywhere it could is counted); that
costs a run, never a skipped test. Comments, docstrings and messages (echo/printf/fail/print/raise …) are not
scanned: they name files without running them.

Run from the BASE commit's copy in CI (adversarial.yml `changes`), so a pull request cannot loosen the filter that
judges it. The pull request's own WORKFLOW still runs (pull_request events run the head's adversarial.yml), so this
protects the script, not the workflow: that is why every change under .github/ runs everything.
"""
import os
import re
import subprocess
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
IMAGE_BASENAMES = {"Cargo.toml", "build.rs",     # anywhere: a package boundary or a build script moved
                   ".gitattributes", ".gitmodules", ".lfsconfig"}   # rewrite what a checkout holds (eol, encoding, lfs)

# Soundness only (the hosts and the jobs that run them; not the image ids).
SOUNDNESS_PREFIXES = (
    "scripts/libzync/",               # ci/ drivers (build-hosts.sh runs inside the container), the *_ref.py oracles
    "experimental/libzync-bench/",    # lib.sh's $B: leaf/join/prep/coord/handover/core_rules/cost harnesses
    "coordinator/web/",
)
COORD_RUNTIME = ("coordinator/server.py", "coordinator/hazync", "coordinator/mod_block.txt")

# The jobs `changes` gates, and the ones gated through them: their blocks in adversarial.yml seed references().
HEAVY_JOBS = ("reproducible-image-id", "libzync-hosts", "libzync-chain", "libzync-blocks", "libzync-vectors",
              "libzync-rig", "libzync-signet", "soundness-suite")
WORKFLOW = ".github/workflows/adversarial.yml"

# Light: nothing any heavy job builds or runs. Checked only after every heavy rule above.
LIGHT_PATTERNS = (
    "*.md", "LICENSE", ".gitignore",
    "docs/*", "history/*", "tools/*", "ooga-land/*", "zapps/*",
    "guest-pure-fuzz/*", "swiftsync-verify-ffi/*", "swiftsync-verify-wasm/*",
    "coordinator/*",          # the rest of it: tests, other deploy/ files, the tip fleet tooling (accumulator-tests)
    "scripts/*",              # the rest of it: release and check scripts (accumulator-tests runs those)
)


class FsTree:
    """A checkout on disk (the selftest, local use)."""
    def __init__(self, root):
        self.root = root
        fs = []
        for d, dirs, names in os.walk(root):
            dirs[:] = [x for x in dirs if x not in (".git", "target", "__pycache__", "node_modules")]
            fs += [os.path.relpath(os.path.join(d, n), root) for n in names]
        self.files = set(fs)

    def read(self, p):
        try:
            with open(os.path.join(self.root, p), encoding="utf-8", errors="replace") as f:
                return f.read()
        except OSError:
            return None


class GitTree:
    """A commit (CI: the base tip, HEAD^1). Every read comes from git's objects, not the working tree."""
    def __init__(self, rev, cwd="."):
        self.rev, self.cwd = rev, cwd
        out = subprocess.run(["git", "ls-tree", "-r", "-z", "--name-only", rev], cwd=cwd, check=True,
                             capture_output=True).stdout.decode("utf-8", "replace")
        self.files = {p for p in out.split("\0") if p}
        if not self.files:
            raise SystemExit(f"git ls-tree {rev}: no files")

    def read(self, p):
        r = subprocess.run(["git", "cat-file", "blob", f"{self.rev}:{p}"], cwd=self.cwd, capture_output=True)
        return r.stdout.decode("utf-8", "replace") if r.returncode == 0 else None


def norm(p):
    p = os.path.normpath(p)
    return None if p.startswith("..") or p.startswith("/") or p == "." else p


def is_dir(tree, d):
    pre = d + "/"
    return any(f.startswith(pre) for f in tree.files)


def path_dep_closure(tree):
    """The repo-relative dirs of every crate prover/ reaches through `path =` dependencies, prover's own included."""
    seen = set()
    todo = [os.path.dirname(f) for f in tree.files
            if f.startswith("prover/") and os.path.basename(f) == "Cargo.toml" and "/target/" not in f]
    while todo:
        d = norm(todo.pop())
        if d is None or d in seen:
            continue
        seen.add(d)
        t = tree.read(d + "/Cargo.toml")
        if t is None:
            continue
        for m in re.finditer(r'path\s*=\s*"([^"]+)"', t):
            p = norm(os.path.join(d, m.group(1)))
            if p and not p.endswith(".rs"):
                todo.append(p)
    return seen


def nested_packages(tree, crate_dirs):
    """Dirs holding a Cargo.toml of their own, inside a closure crate, that are NOT in the closure themselves."""
    out = set()
    for f in tree.files:
        if os.path.basename(f) != "Cargo.toml":
            continue
        d = os.path.dirname(f)
        if d not in crate_dirs and any(d.startswith(c + "/") for c in crate_dirs):
            out.add(d)
    return out


INCLUDE = re.compile(r'(?:include(?:_str|_bytes)?!\(\s*"([^"]+)"\s*\))|(?:#\[path\s*=\s*"([^"]+)"\s*\])')


def include_targets(tree, crate_dirs):
    """Files the closure crates pull in by a literal path: include_str!, include_bytes!, include!, #[path]."""
    out = set()
    for f in tree.files:
        if not f.endswith(".rs") or not any(f.startswith(c + "/") for c in crate_dirs):
            continue
        src = tree.read(f) or ""
        for m in INCLUDE.finditer(src):
            p = norm(os.path.join(os.path.dirname(f), m.group(1) or m.group(2)))
            if p:
                out.add(p)
    return out


def coordinator_imports(tree):
    """Local coordinator modules that server.py or the hazync CLI import (none today; kept so one cannot slip in)."""
    mods = {os.path.basename(f)[:-3] for f in tree.files if f.startswith("coordinator/") and f.count("/") == 1
            and f.endswith(".py")}
    out = set()
    for f in ("coordinator/server.py", "coordinator/hazync"):
        for m in re.finditer(r"^\s*(?:from\s+([A-Za-z_]\w*)|import\s+([A-Za-z_][\w, ]*))", tree.read(f) or "", re.M):
            names = [m.group(1)] if m.group(1) else [n.strip().split(" ")[0] for n in m.group(2).split(",")]
            out |= {f"coordinator/{n}.py" for n in names if n in mods}
    return out


# ── what the heavy jobs execute or read ────────────────────────────────────────────────────────────────────────────
# Variables the drivers use for repo directories (lib.sh: Z, B, T; coord tests: C; others: REPO, HERE, P). A token
# with an unknown variable is tried against every base; anything that resolves to an existing file counts.
VAR_BASES = {"Z": [""], "REPO": [""], "ROOT": [""], "R": [""], "GITHUB_WORKSPACE": [""],
             "B": ["experimental/libzync-bench", "prover/testdata/libzync/mainnet"], "T": ["prover/testdata/libzync"],
             "C": ["coordinator"], "P": ["prover"], "HERE": [None], "DIR": [None], "D": [None]}
TOKEN = re.compile(r'(?:\$\{?([A-Za-z_]\w*)\}?/|/hazync-zkvm/)?((?:\.\./|\./)*[A-Za-z0-9_][A-Za-z0-9_.+-]*(?:/[A-Za-z0-9_.+-]+)*)')
SCANNABLE = (".sh", ".py", ".bash", ".Dockerfile", ".yml", ".mjs", ".js")


# a message command, at command position only (line start, after ; & | ( { or then/else/do), up to ; | & or EOL
MESSAGE = re.compile(r"(^\s*|[;&|({]\s*|\b(?:then|else|do)\s+)(?:echo|printf|fail|die|warn|note|pass|wrong|usage|step)\b[^;|&\n]*")


def strip_prose(text, py=False):
    """Comments, messages and (python only) docstrings: text that names a file without running or reading it ("run
    provision-vps.sh first", "see docs/libzync/REORG.md"). What is left is code; a name in code counts."""
    if py:
        text = re.sub(r'(?s)("""|\'\'\').*?\1', "", text)        # docstrings / long strings
    out = []
    for line in text.splitlines():
        s = line.lstrip()
        if s.startswith("#") or (py and s.startswith(("print(", "raise ", "sys.exit(", "log(", "logging."))):
            continue
        line = re.sub(r'(^|\s)#(?![{!]).*$', "", line)             # trailing `# …` comments; keeps $#, ${#…}
        if not py:
            line = MESSAGE.sub(r"\1", line)                          # echo/printf/fail/die … at command position
        out.append(line)
    return "\n".join(out)


def resolve(tree, token_var, rest, here, extra_bases):
    bases = []
    if token_var is not None:
        bases = VAR_BASES.get(token_var, [None, ""] + [b for v in VAR_BASES.values() for b in v if b])
    else:
        bases = [None, ""] + list(extra_bases)
    found = set()
    for b in bases:
        p = norm(os.path.join(here if b is None else b, rest))
        if p and p in tree.files:
            found.add(p)
    return found


LIGHT_AREAS = re.compile(r"^(?:\.\./|\./)*(coordinator|docs|tools|zapps|scripts)(/|$)")


def scan(tree, text, here, extra_bases=(), py=False, unresolved=None):
    """Repo files a script/job text names (comments stripped), and python modules it imports from beside itself.
    A name of a light area (coordinator, docs, tools, zapps, scripts outside scripts/libzync, or under $C) that
    resolves to no file is appended to `unresolved`: the selftest's tripwire for a dynamic form this scan misses."""
    text = strip_prose(text, py)
    out = set()
    # `cd "$C"` / `cd ../x` / `cd prover`: bare names after it resolve there too (coord_test.sh: cd "$C"; python3 server.py)
    for var, rest in re.findall(r'\bcd\s+"?(?:\$\{?([A-Za-z_]\w*)\}?/?)?([A-Za-z0-9_./-]*)', text):
        for b in (VAR_BASES.get(var, []) if var else [None]):
            d = norm(os.path.join(here if b is None else b, rest or "."))
            if d and is_dir(tree, d):
                extra_bases = tuple(extra_bases) + (d,)
    for m in TOKEN.finditer(text):
        var, rest = m.group(1), m.group(2)
        if m.group(0).startswith("/hazync-zkvm/"):
            var = "Z"
        if "/" not in rest and "." not in rest and var is None:
            continue                                   # a bare word, not a path
        got = resolve(tree, var, rest, here, extra_bases)
        out |= got
        if not got and unresolved is not None and norm(rest.lstrip("./").rstrip("/")) != "coordinator":
            # (the coordinator DIRECTORY itself, `$Z/coordinator` / `C=…/coordinator`, is covered by COORD_RUNTIME and
            # coordinator_module(): what of it can run is named there)
            if (var == "C" or (var in (None, "Z", "REPO", "ROOT", "R") and LIGHT_AREAS.match(rest))) \
                    and "scripts/libzync" not in rest:
                unresolved.append((here, m.group(0)))
    for m in re.finditer(r"^\s*(?:from\s+([A-Za-z_]\w*)\s+import|import\s+([A-Za-z_]\w*))", text, re.M):
        p = norm(os.path.join(here, (m.group(1) or m.group(2)) + ".py"))
        if p and p in tree.files:
            out.add(p)
    return out


def job_blocks(text, names):
    """The text of each named top-level job in a workflow (`  name:` at two spaces, to the next one)."""
    blocks, cur = {}, None
    for line in text.splitlines():
        m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
        if m:
            cur = m.group(1) if m.group(1) in names else None
            if cur:
                blocks[cur] = []
            continue
        if cur:
            blocks[cur].append(line)
    return {k: "\n".join(v) for k, v in blocks.items()}


def references(tree, unresolved=None):
    """Every repo file the heavy jobs execute or read, transitively, seeded from their blocks in adversarial.yml."""
    wf = tree.read(WORKFLOW)
    if wf is None:
        raise SystemExit(f"{WORKFLOW} not in the tree: cannot derive what the heavy jobs run")
    blocks = job_blocks(wf, HEAVY_JOBS)
    missing = [j for j in HEAVY_JOBS if j not in blocks]
    if missing:
        raise SystemExit(f"heavy jobs not found in {WORKFLOW}: {missing}")
    seen = set()
    for b in blocks.values():
        cds = {norm(x) for x in re.findall(r"\bcd\s+([A-Za-z0-9_./-]+)", b)} - {None}
        seen |= scan(tree, b, "", cds, unresolved=unresolved)
    todo = list(seen)
    while todo:
        f = todo.pop()
        if not (f.endswith(SCANNABLE) or "." not in os.path.basename(f) or f.endswith("Dockerfile")):
            continue                                   # data (json, txt, …): read, not executed; counted, not scanned
        text = tree.read(f)
        if text is None or len(text) > 2_000_000:
            continue
        py = f.endswith(".py") or "python" in text.split("\n", 1)[0]     # coordinator/hazync is python, no .py
        for p in scan(tree, text, os.path.dirname(f), py=py,
                      unresolved=unresolved):
            if p not in seen:
                seen.add(p)
                todo.append(p)
    return seen


class Classifier:
    def __init__(self, tree, control=False):
        self.control = control
        self.closure = path_dep_closure(tree)       # prover's own crates included: their include targets count too
        self.crates = {c for c in self.closure if c != "prover" and not c.startswith("prover/")}
        self.nested = nested_packages(tree, self.closure)
        self.includes = include_targets(tree, self.closure)
        self.base_files = set(tree.files)
        self.coord_dirs = {f.split("/")[1] for f in tree.files if f.startswith("coordinator/") and f.count("/") >= 2}
        self.refs = references(tree)
        self.coord = set(COORD_RUNTIME) | coordinator_imports(tree)

    def classify(self, p):
        """'image', 'soundness' or 'light'; the first rule that names the path decides, heavy rules first."""
        if self.control:          # positive control: the filter with every heavy rule removed must fail the selftest
            return "light"
        base = os.path.basename(p)
        if (p.startswith(IMAGE_PREFIXES) or p in IMAGE_FILES or p in self.includes or base in IMAGE_BASENAMES
                or "/.cargo/" in "/" + p):
            return "image"
        for c in self.crates:
            if p.startswith(c + "/") and not any(p.startswith(n + "/") for n in self.nested):
                return "image"
        if p.startswith(SOUNDNESS_PREFIXES) or p in self.coord or p in self.refs or self.coordinator_module(p):
            return "soundness"
        if any(fnmatchcase(p, g) for g in LIGHT_PATTERNS):
            return "light"
        return "image"           # unnamed: run everything


def _coordinator_module(self, p):
    """coord_test.sh and board_rehearsal.sh run `cd $C; python3 server.py`: coordinator/ is sys.path[0], ahead of the
    standard library. So a coordinator/*.py the base does not have (a planted bisect.py shadows the stdlib's), a new
    directory there (a package), or any top-level module named like a stdlib module is a soundness change. A new
    top-level test_*.py is not: nothing imports a test_ name implicitly, and the stdlib has none."""
    if not p.startswith("coordinator/"):
        return False
    rel = p[len("coordinator/"):]
    top = rel.split("/")[0]
    if "/" not in rel:
        if not rel.endswith(".py"):
            return False
        mod = rel[:-3]
        if mod in getattr(sys, "stdlib_module_names", ()):
            return True
        return p not in self.base_files and not mod.startswith("test_")
    return top not in self.coord_dirs                      # a new directory (a package) under coordinator/


Classifier.coordinator_module = _coordinator_module


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
    ("zapps/headers/src/Cargo.toml", "image"),          # a NEW Cargo.toml anywhere is an image change
    ("zapps/hints/cli/build.rs", "image"),
    ("tools/x/.cargo/config.toml", "image"),
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
    ("coordinator/deploy/libzync/libzync-prep-run.sh", "soundness"),   # run by prep/rewind_test.sh, prep/crash_test.sh
    ("coordinator/deploy/libzync/SWITCH.md", "light"),                 # a runbook: nothing runs it
    ("coordinator/deploy/libzync/hazync-lz-check-spine.timer", "light"),
    ("coordinator/test_claim_nonce.py", "light"),
    ("coordinator/test_brand_new_check.py", "light"),           # a NEW test_*.py: nothing imports it implicitly
    ("coordinator/tip_board.py", "light"),                       # an existing module server.py does not import
    ("coordinator/bisect.py", "soundness"),                      # NEW, shadows the stdlib from sys.path[0] (round-2 HIGH-1)
    ("coordinator/json.py", "soundness"),
    ("coordinator/brand_new_module.py", "soundness"),            # NEW module: server.py could import it
    ("coordinator/newpkg/__init__.py", "soundness"),             # NEW directory: a package
    ("coordinator/deploy/brand-new-unit.service", "light"),      # a new file in an existing directory
    (".gitattributes", "image"),                                 # round-2 HIGH-2: rewrites what checkout writes
    ("prover/.gitattributes", "image"),
    ("docs/.gitattributes", "image"),
    ("tools/x/.gitmodules", "image"),
    (".lfsconfig", "image"),
    ("coordinator/deploy/headers-api/hazync-headers-api", "light"),
    (".github/workflows/adversarial.yml", "image"),
    (".github/scripts/ci_changes.py", "image"),
    ("provision-vps.sh", "image"),
    ("some-new-dir/thing.rs", "image"),                 # unnamed -> everything
    ("Cargo.toml", "image"),
]
# Files the reference scan must find (a scanner that goes blind would otherwise pass every path above by its prefix).
MUST_REFERENCE = [
    "scripts/libzync/ci/chain.sh", "scripts/libzync/ci/lib.sh", "scripts/libzync/ci/build-hosts.sh",
    "experimental/libzync-bench/prep/rewind_test.sh", "experimental/libzync-bench/prep/crash_test.sh",
    "coordinator/deploy/libzync/libzync-prep-run.sh", "coordinator/server.py", "coordinator/hazync",
    "scripts/libzync/coinlog_ref.py", "prover/chainparams_check.sh", "reproduce/libzync-ids.sh",
]


def selftest(tree, control):
    c = Classifier(tree, control=control)
    bad = [(p, w, c.classify(p)) for p, w in SELFTEST if c.classify(p) != w]
    for p, w, g in bad:
        print(f"MISMATCH {p}: want {w}, got {g}")
    for p in MUST_REFERENCE:
        if p not in c.refs:
            print(f"NOT REFERENCED {p}: the scan of what the heavy jobs run no longer finds it")
            bad.append(p)
    # every repo file the heavy jobs execute or read must be heavy, whatever the rules say
    light = sorted(p for p in c.refs if c.classify(p) == "light")
    for p in light:
        print(f"REFERENCED BUT LIGHT {p}")
    bad += light
    checks = [(decide([]), (False, False)), (decide(["light"]), (False, False)),
              (decide(["light", "soundness"]), (False, True)), (decide(["soundness", "image"]), (True, True))]
    bad += [x for x in checks if x[0] != x[1]]
    if "swiftsync-verify" not in c.crates or "zapps/hints/cli" not in c.nested:
        bad.append(("closure", sorted(c.crates), sorted(c.nested)))
    # HIGH-2 (review of d5e4d86): an empty Cargo.toml dropped into zapps/headers/src/ made src/lib.rs look like a nested
    # package's file. The rules come from the BASE tree (no such file there), and the Cargo.toml itself is image.
    if not control and (c.classify("zapps/headers/src/lib.rs"), c.classify("zapps/headers/src/Cargo.toml")) != ("image", "image"):
        bad.append("zapps/headers/src: lib.rs or a new Cargo.toml there is not an image change")
    if not control:
        # round-2 MEDIUM-1: include targets of the prover's OWN crates count. prover/host included reproduce/METHOD_ID
        # until CUTOVER C8; since then its include outside prover/ is engine/guest/Cargo.toml.in (lz.rs, the OOM pin)
        if "engine/guest/Cargo.toml.in" not in c.includes:
            bad.append("include targets of the prover's own crates are not scanned")
        # round-2 MEDIUM-2a: every job that needs `changes` or libzync-hosts is one references() is seeded from
        wf = tree.read(WORKFLOW) or ""
        allj = job_blocks(wf, set(re.findall(r"^  ([A-Za-z0-9_-]+):\s*$", wf, re.M)))
        for j, b in allj.items():
            m = re.search(r"^    needs:\s*(\[[^\]]*\]|\S+)", b, re.M)
            needs = set(re.findall(r"[A-Za-z0-9_-]+", m.group(1))) if m else set()
            if needs & {"changes", "libzync-hosts"} and j not in HEAVY_JOBS:
                print(f"UNLISTED HEAVY JOB {j}: needs {sorted(needs)} but is not in HEAVY_JOBS")
                bad.append(j)
        # round-2 MEDIUM-2b: tripwire. A heavy file naming a light area (coordinator, docs, tools, zapps, scripts outside
        # scripts/libzync) in code without the scan resolving it to a file: a dynamic form the scan misses.
        unresolved = []
        references(tree, unresolved)
        for here, tok in sorted(set(unresolved)):
            print(f"UNRESOLVED LIGHT-AREA NAME in {here or '(workflow)'}: {tok}")
        bad += unresolved
    if bad:
        print(f"CI_CHANGES_SELFTEST_FAILED {len(bad)}")
        return 1
    print(f"CI_CHANGES_SELFTEST_OK {len(SELFTEST)} paths, {len(c.refs)} files the heavy jobs run or read; "
          f"closure: {' '.join(sorted(c.crates))}")
    return 0


def main(argv):
    here_root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    if "--selftest" in argv:
        return selftest(FsTree(here_root), "--control" in argv)
    if "--base" in argv:
        tree = GitTree(argv[argv.index("--base") + 1])
    else:
        tree = FsTree(argv[1] if len(argv) > 1 else here_root)
    files = [l.strip() for l in sys.stdin if l.strip()]
    c = Classifier(tree)
    classes = [c.classify(p) for p in files]
    for p, k in zip(files, classes):
        print(f"    {k:9} {p}", file=sys.stderr)
    image, soundness = decide(classes)
    if not files:             # no list is not "nothing changed": a diff that failed upstream looks the same
        print("no changed files read: running everything", file=sys.stderr)
        image = soundness = True
    print(f"image={'true' if image else 'false'}")
    print(f"soundness={'true' if soundness else 'false'}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
