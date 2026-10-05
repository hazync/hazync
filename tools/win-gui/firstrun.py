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
                    "cuda126", "cpu"):
            out.append(base / sub)
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
    found.sort(key=lambda i: (not i.get("startable"), i["kind"] != "cuda"))
    return found


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
