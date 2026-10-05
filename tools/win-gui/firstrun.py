"""Getting from "I downloaded this" to "it is proving" — the setup logic, headless and testable.

⛔ THE PROBLEM THIS SOLVES. Before this, a Windows newcomer had to: install Python, pip install
cryptography, find and download a host binary, find and download hazync-worker, set environment
variables, invent a handle, and only then press Start. Every one of those is a place to give up.

⇒ So each step here reports three things: is it DONE, WHY not, and can the program FIX IT ITSELF.
The GUI just renders that and offers the fixable ones as buttons.

⚠ ONE STEP CANNOT BE AUTOMATED, AND IT IS NAMED RATHER THAN HIDDEN. `hazync-worker` is a public
release asset and downloads with no credentials (measured: HTTP 200 unauthenticated). The Windows
`host.exe` is NOT a release asset — it exists only as a CI artifact, which GitHub refuses to serve
anonymously even for a public repository. Until it is published as a release asset, that download
stays manual, and `publish_host_asset.sh` beside this file is the one command that fixes it.

⛔ NOTHING IS TRUSTED JUST BECAUSE IT DOWNLOADED. The worker is checked by running its own CLI; the
host binary is checked by reading the canonical guest out of it, which works even for a build that
cannot start on this machine.
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()   # ⛔ BEFORE anything prints: a ✅ on a cp1252 console raises, not degrades

import json
import time
import shutil
import os
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import nethttp
import supervisor  # noqa: E402

REPO = "hazync/hazync"
WORKER_ASSET = "hazync-worker"
CONFIG_NAME = "win-gui.json"


# ── config, because nothing surviving a restart is its own kind of broken ────────────────────────
def hazync_home():
    return Path(os.environ.get("HAZYNC_HOME") or (Path.home() / ".hazync"))


def config_path():
    return hazync_home() / CONFIG_NAME


def load_config():
    """Saved settings, or sensible defaults. Never raises — a corrupt file must not block startup."""
    d = {"host": "", "worker": "", "identity": str(hazync_home()),
         "coord": "https://api.hazync.org", "workers": 1, "seg_po2": "", "dark": False,
         "mode": "auto"}
    try:
        d.update(json.loads(config_path().read_text(encoding="utf-8")))
    except (OSError, ValueError):
        pass
    return d


def save_config(d):
    """Persist settings. Returns (ok, detail) — a failure to save is worth telling someone about."""
    try:
        p = config_path()
        p.parent.mkdir(parents=True, exist_ok=True)
        tmp = p.with_suffix(".tmp")
        # ⚠ Atomic: a half-written config read on the next launch would look like lost settings.
        tmp.write_text(json.dumps(d, indent=1, sort_keys=True), encoding="utf-8")
        os.replace(tmp, p)
        return True, str(p)
    except OSError as e:
        return False, f"could not save settings to {config_path()}: {e}"


# ── finding a host binary ────────────────────────────────────────────────────────────────────────
def candidate_dirs():
    """Where a downloaded binary plausibly is, best guess first."""
    here = Path(__file__).resolve().parent
    out = [here, Path.cwd(), Path.home() / "Downloads", Path.home() / "Desktop", Path.home()]
    # gh run download puts an artifact in a folder named after it
    for base in list(out):
        for sub in ("hazync-host-windows-x86_64-cuda", "hazync-host-windows-x86_64-cpu",
                    "cuda126", "cpu", "cuda", "gpu", "pascal", "hazync", "prover"):
            out.append(base / sub)
    # ⚠ ONE LEVEL DEEPER UNDER HOME AND Downloads, because that is where the artifacts actually
    # land. Measured on a real machine 2026-10-05: three host.exe files in ~/, ~/cpu/ and
    # ~/pascal/ -- and only the first was in this list, which is why the person had to Browse.
    for base in (Path.home(), Path.home() / "Downloads"):
        try:
            for child in sorted(base.iterdir())[:60]:   # bounded: a home directory can be huge
                if child.is_dir() and not child.name.startswith("."):
                    out.append(child)
        except OSError:
            pass
    return out


def find_hosts():
    """Every host binary we can find, classified, best-for-this-machine first.

    ⭐ SORTED BY WHETHER IT CAN ACTUALLY START HERE. Offering someone the CUDA build on a machine
    with no NVIDIA driver is offering them an executable Windows refuses to load, and the error
    reads like a corrupt download.
    """
    seen, found = set(), []
    for d in candidate_dirs():
        for name in ("host.exe", "host"):
            p = d / name
            try:
                if not p.is_file():
                    continue
                key = p.resolve()
            except OSError:
                continue
            if key in seen:
                continue
            seen.add(key)
            info = supervisor.classify_host(p)
            info["path"] = str(p)
            found.append(info)
    # ⛔ "STARTABLE" IS NOT "USABLE", AND SORTING CUDA FIRST WAS WRONG BECAUSE OF IT. A CUDA build
    # starts fine on a machine with an NVIDIA driver and then fails at the first GPU call if the
    # card is below sppark's compute floor — measured on a GTX 1050 Ti, 2026-10-05. So the order
    # follows what this machine should ACTUALLY use, not what sounds faster.
    want = supervisor.recommend().get("build", "cpu")
    found.sort(key=lambda i: (not i.get("startable"), i["kind"] != want))
    return found


def adopt_hosts(cfg=None):
    """Fill in the per-build paths from whatever is already on this disk. (cfg-shaped dict)

    ⭐ THE "OUT OF THE BOX" PART. Pointing a program at a file it could have found itself is the
    kind of setup step that makes someone give up before they start, and classify_host can tell a
    CPU build from a CUDA one by its import table — so the two slots can be filled without asking.
    ⚠ It never overwrites a path somebody chose deliberately.
    """
    out = dict(cfg or {})
    for info in find_hosts():
        kind = info.get("kind")
        if kind not in ("cpu", "cuda"):
            continue
        key = f"host_{kind}"
        if not out.get(key):
            out[key] = info["path"]
    if not out.get("host"):
        want = supervisor.recommend().get("build", "cpu")
        out["host"] = out.get(f"host_{want}") or out.get("host_cpu") or out.get("host_cuda") or ""
        if out["host"]:
            out["build_kind"] = want if out.get(f"host_{want}") else (
                "cpu" if out.get("host_cpu") else "cuda")
    return out


# ── fetching the worker, which IS public ─────────────────────────────────────────────────────────
def latest_release_tag(timeout=20):
    # ⛔ Through nethttp, not urllib directly: on a stock Windows box the system trust store can
    # fail to supply an issuer and this is one of the three calls that then dies. See nethttp.py.
    d = nethttp.get_json(f"https://api.github.com/repos/{REPO}/releases/latest",
                         timeout=timeout, headers={"Accept": "application/vnd.github+json"})
    return d.get("tag_name") or ""


def fetch_worker(dest_dir=None, tag=None, timeout=120):
    """Download `hazync-worker` from the public release. (ok, path_or_message)

    ⚠ Verified by RUNNING it, not by its size: a GitHub error page is a perfectly valid file.
    """
    dest_dir = Path(dest_dir or hazync_home())
    try:
        tag = tag or latest_release_tag()
    except (nethttp.NetError, urllib.error.URLError, urllib.error.HTTPError,
            ValueError, TimeoutError) as e:
        return False, f"could not ask GitHub for the latest release: {e}"
    if not tag:
        return False, "GitHub did not name a latest release"
    url = f"https://github.com/{REPO}/releases/download/{tag}/{WORKER_ASSET}"
    dest = dest_dir / WORKER_ASSET
    try:
        dest_dir.mkdir(parents=True, exist_ok=True)
        data = nethttp.get(url, timeout=timeout)
    except (nethttp.NetError, urllib.error.URLError, urllib.error.HTTPError, TimeoutError) as e:
        return False, f"could not download {url}: {e}"
    if not data.startswith(b"#!") or b"def main" not in data:
        return False, (f"what came back from {url} is not the worker "
                       f"({len(data):,} bytes, starts {data[:24]!r})")
    try:
        dest.write_bytes(data)
        dest.chmod(0o755)
    except OSError as e:
        return False, f"downloaded it but could not save to {dest}: {e}"
    chk = supervisor.check_worker(dest)
    if not chk.ok:
        return False, f"saved to {dest} but it does not run: {chk.detail}"
    return True, str(dest)


# ── your identity: importing one, and never losing one ───────────────────────────────────────────
KEY_NAME = "key.hex"


def identity_public(identity_dir=None):
    """The PUBLIC key of the identity in this folder, or None. (never the secret)"""
    f = Path(identity_dir or hazync_home()) / KEY_NAME
    try:
        raw = f.read_text().strip()
    except OSError:
        return None
    return _public_of(raw)


def _public_of(hex_secret):
    """The public key for a hex secret, or None if it is not a usable ed25519 key.

    ⛔ THE SECRET IS NEVER RETURNED, LOGGED OR SHOWN. A signing key that reaches a log reaches a
    screenshot, and every block this machine has ever proved is credited to it.
    """
    s = (hex_secret or "").strip()
    if len(s) != 64:
        return None
    try:
        bytes.fromhex(s)
    except ValueError:
        return None
    try:
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        sk = Ed25519PrivateKey.from_private_bytes(bytes.fromhex(s))
        return sk.public_key().public_bytes(serialization.Encoding.Raw,
                                            serialization.PublicFormat.Raw).hex()
    except Exception:          # noqa: BLE001 - an unusable key is a result, not a crash
        return None


def import_identity(src, identity_dir=None):
    """Bring an existing signing key onto this machine. (ok, message)

    ⛔⛔ IT BACKS UP FIRST, ALWAYS. The window's own text says losing this file means losing the
    credit for everything you have proved — so an import that overwrote one silently would be the
    single most destructive button in the program. The existing key is copied to
    key.hex.replaced-<timestamp> before anything is written, and the message says where it went.

    ⚠ IT VALIDATES BEFORE IT TOUCHES ANYTHING. A truncated paste or the wrong file would otherwise
    leave a machine that cannot sign, discovered at the end of the first prove rather than now.
    """
    dest_dir = Path(identity_dir or hazync_home())
    src = Path(src)
    try:
        text = src.read_text(errors="replace").strip()
    except OSError as e:
        return False, f"could not read {src}: {e}"
    # ⚠ Accept the file the worker writes (bare hex) and a key pasted with surrounding whitespace
    # or a trailing newline, which is what happens when someone copies it out of a terminal.
    candidate = "".join(text.split())
    pub = _public_of(candidate)
    if not pub:
        return False, (f"{src.name} is not an ed25519 signing key — expected 64 hex characters "
                       f"(32 bytes), got {len(candidate)} character(s). Nothing was changed.")
    existing = dest_dir / KEY_NAME
    moved = ""
    try:
        dest_dir.mkdir(parents=True, exist_ok=True)
        if existing.exists():
            if "".join(existing.read_text(errors="replace").split()) == candidate:
                return True, f"that key is already the one in use here (public {pub[:16]}…)"
            backup = existing.with_suffix(f".hex.replaced-{time.strftime('%Y%m%d-%H%M%S')}")
            shutil.copy2(existing, backup)
            moved = f" The key it replaced was kept as {backup.name}."
        tmp = existing.with_suffix(".hex.tmp")
        tmp.write_text(candidate)
        tmp.replace(existing)          # ⚠ atomic: a half-written key is an unusable machine
        try:
            _os.chmod(existing, 0o600)
        except OSError:
            pass                        # Windows ACLs, not POSIX modes — not a failure
    except OSError as e:
        return False, f"could not write {existing}: {e}"
    return True, f"imported — this machine now signs as public key {pub[:16]}….{moved}"


def backup_identity(dest, identity_dir=None):
    """Copy the signing key somewhere safe. (ok, message)"""
    src = Path(identity_dir or hazync_home()) / KEY_NAME
    if not src.is_file():
        return False, f"there is no {KEY_NAME} in {src.parent} yet — run the client once first"
    try:
        shutil.copy2(src, dest)
    except OSError as e:
        return False, f"could not write {dest}: {e}"
    return True, (f"saved to {dest}. ⚠ Anyone holding this file can claim your blocks — keep it "
                  f"like a password, not like a document.")


# ── updating this program ────────────────────────────────────────────────────────────────────────
def app_update(repo_dir=None):
    """Pull the newest version of this program. (ok, message)

    ⭐ It is a git checkout, so updating it is a fetch and a fast-forward — no installer, no
    download, and the person does not have to find a terminal. --ff-only deliberately: a merge
    conflict in a tool someone is only trying to RUN is not a thing to hand them.
    """
    d = Path(repo_dir or Path(__file__).resolve().parent)
    rc, out = supervisor._run(["git", "-C", str(d), "rev-parse", "--is-inside-work-tree"],
                              timeout=30)
    if rc != 0 or "true" not in out:
        return False, (f"{d} is not a git checkout, so there is nothing to pull. Download it again "
                       f"from GitHub to update.")
    rc, before = supervisor._run(["git", "-C", str(d), "rev-parse", "--short", "HEAD"], timeout=30)
    rc, out = supervisor._run(["git", "-C", str(d), "pull", "--ff-only"], timeout=180)
    if rc != 0:
        return False, (f"could not update: {out.strip()[-300:]}\n"
                       f"If this says the branch has diverged, the checkout has local changes.")
    rc, after = supervisor._run(["git", "-C", str(d), "rev-parse", "--short", "HEAD"], timeout=30)
    if before.strip() == after.strip():
        return True, f"already up to date ({after.strip()})"
    # ⛔ A RUNNING PYTHON PROGRAM DOES NOT PICK UP ITS OWN NEW SOURCE. Saying "updated" and leaving
    # the old code running is how someone reports a bug that was fixed an hour ago.
    return True, (f"updated {before.strip()} → {after.strip()}. ⚠ Close and reopen Hazync for the "
                  f"new version to take effect — a running program keeps the code it started with.")


# ── the signing library ──────────────────────────────────────────────────────────────────────────
def have_cryptography(python_exe=None):
    rc, out = supervisor._run([python_exe or sys.executable, "-c",
                               "import cryptography;print(cryptography.__version__)"], timeout=60)
    return (rc == 0, out.strip())


def install_cryptography(python_exe=None, timeout=300):
    """pip install cryptography. (ok, detail)

    ⚠ --user so it works without an administrator prompt, which is where a newcomer stops.
    """
    py = python_exe or sys.executable
    rc, out = supervisor._run([py, "-m", "pip", "install", "--user", "cryptography"],
                              timeout=timeout)
    if rc != 0:
        return False, f"pip exited {rc}: {out.strip()[-300:]}"
    ok, ver = have_cryptography(py)
    return ok, (f"cryptography {ver}" if ok else f"pip reported success but the import still fails: {ver}")


# ── identity ─────────────────────────────────────────────────────────────────────────────────────
def identity(worker, host=None, ident_dir=None, coord=None, python_exe=None):
    """(handle, pubkey) by asking the worker, or (None, None). Creates the key if absent."""
    env = supervisor.worker_env(host, worker, ident_dir, None, coord_url=coord)
    rc, out = supervisor._run(supervisor.worker_command(python_exe, worker, "id"),
                              env=env, timeout=90)
    if rc != 0:
        return None, None
    handle = pub = None
    for line in out.splitlines():
        if line.lower().startswith("handle"):
            handle = line.split(":", 1)[-1].strip()
        elif line.lower().startswith("pubkey"):
            pub = line.split(":", 1)[-1].strip()
    return handle, pub


def set_handle(worker, name, host=None, ident_dir=None, coord=None, python_exe=None):
    """Name this prover. (ok, detail)

    ⛔ WHY THIS IS A SETUP STEP AND NOT A PREFERENCE. With no handle the worker credits work to
    `ghost:<first 6 of the pubkey>` — the worker itself warns about it, because every block proved
    under a machine-generated name is credited to something nobody recognises, publicly and
    permanently.
    """
    name = (name or "").strip()
    if not name:
        return False, "a handle cannot be empty"
    env = supervisor.worker_env(host, worker, ident_dir, None, coord_url=coord)
    rc, out = supervisor._run(supervisor.worker_command(python_exe, worker, "id", (name,)),
                              env=env, timeout=90)
    if rc != 0:
        return False, f"`hazync id {name}` exited {rc}: {out.strip()[:200]}"
    got, _ = identity(worker, host, ident_dir, coord, python_exe)
    return (got == name), (f"this prover is now {got!r}" if got == name
                           else f"asked for {name!r} but it reads back as {got!r}")


def is_default_handle(handle):
    """A machine-generated name, which is the thing worth prompting about."""
    return bool(handle) and handle.startswith("ghost:")


def key_path(ident_dir=None):
    return Path(ident_dir or hazync_home()) / "key.hex"


# ── the ordered setup ────────────────────────────────────────────────────────────────────────────
class Step:
    def __init__(self, key, title, done, detail, fix=None, fix_label=None, manual=None):
        self.key, self.title, self.done, self.detail = key, title, done, detail
        self.fix, self.fix_label, self.manual = fix, fix_label, manual

    def __repr__(self):
        return f"<{'done' if self.done else 'todo'} {self.key}: {self.detail[:60]}>"


def setup_steps(cfg, python_exe=None):
    """Where setup has got to, in the order it should be done. Each step knows its own fix."""
    steps = []

    # 1 — the signing library
    ok, ver = have_cryptography(python_exe)
    steps.append(Step(
        "crypto", "Signing library",
        ok, f"cryptography {ver}" if ok else "not installed — the worker signs every claim and "
                                             "cannot run without it",
        fix=(None if ok else (lambda: install_cryptography(python_exe))),
        fix_label="Install it"))

    # 2 — the worker
    w = cfg.get("worker") or ""
    wok = bool(w) and Path(w).is_file()
    chk = supervisor.check_worker(w, python_exe) if wok else None
    wgood = bool(chk and chk.ok)
    steps.append(Step(
        "worker", "The client (hazync-worker)",
        wgood,
        (chk.detail if chk else "not found") if wok else "not downloaded yet",
        fix=(None if wgood else (lambda: fetch_worker())),
        fix_label="Download it"))

    # 3 — the prover
    h = cfg.get("host") or ""
    hinfo = supervisor.classify_host(h) if h and Path(h).is_file() else None
    hgood = bool(hinfo and hinfo.get("startable"))
    if hinfo:
        detail = supervisor.host_kind_sentence(hinfo)
    else:
        found = find_hosts()
        detail = ("not set. " + (f"Found one at {found[0]['path']}" if found
                                 else "Download it from the GitHub Actions run and point at it — "
                                      "see the Setup tab."))
    steps.append(Step(
        "host", "The prover (host.exe)",
        hgood, detail,
        manual=("The Windows prover is not a release asset yet, so it cannot be fetched "
                "automatically. Download the artifact from the latest windows-prove run and "
                "select host.exe." if not hgood else None)))

    # 4 — identity
    handle = pub = None
    if wgood:
        handle, pub = identity(w, cfg.get("host") or None, cfg.get("identity") or None,
                               cfg.get("coord") or None, python_exe)
    named = bool(handle) and not is_default_handle(handle)
    steps.append(Step(
        "identity", "Your name on the board",
        named,
        (f"{handle}  (key {key_path(cfg.get('identity')).name})" if named else
         (f"currently {handle!r} — a machine-generated name. Work proved under it is credited "
          f"publicly to that, not to you." if handle else
          "unknown until the client is installed")),
        fix_label="Set a name"))

    return steps


def ready(steps):
    """Can proving start? The prover and the client are required; a name is strongly advised."""
    need = {s.key: s.done for s in steps}
    return need.get("crypto") and need.get("worker") and need.get("host")


def summarise(steps):
    todo = [s for s in steps if not s.done]
    if not todo:
        return "ready to prove"
    return f"{len(todo)} step(s) left: " + ", ".join(s.title.lower() for s in todo)
