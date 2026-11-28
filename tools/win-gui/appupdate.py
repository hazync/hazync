"""Keeping the installed Windows app current: is there a newer release, fetch it VERIFIED, run its installer.

Pure logic and I/O helpers, no Tk, so test_appupdate.py can run it headless with the network mocked.

What is verified before an installer is run (all of it, or nothing runs):

  1. SHA256SUMS.txt.asc is a good OpenPGP signature over SHA256SUMS.txt by the maintainer key PINNED HERE (an Ed25519
     key; its fingerprint, computed from the pinned packet, is the one SECURITY.md publishes: test_appupdate.py checks
     the two agree). Verified in Python with `cryptography`, which the app already ships for the worker's signing; no
     gpg is needed on the PC.
  2. The manifest's first line names THIS tag (`# hazync <tag> commit <sha> image <id>`, signed since v0.25.0), so an
     older signed manifest cannot be replayed under a newer tag.
  3. The installer's sha256 is the one that manifest gives for hazync-setup-x86_64-windows.exe.

The installer is NOT Authenticode-signed (owner's decision, 11-13: windows-installer.yml), so there is no Windows code
signature to check; the PGP-signed manifest is the trust anchor, exactly as SECURITY.md tells a person to check by hand.
"""

import base64
import hashlib
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = "hazync/hazync"
INSTALLER = "hazync-setup-x86_64-windows.exe"
MANIFEST = "SHA256SUMS.txt"
VERSION_FILE = "version.ini"        # written into {app} by installer/hazync.iss
CHECK_EVERY_S = 6 * 3600
TAG_RE = re.compile(r"v(\d+)\.(\d+)\.(\d+)")     # used with fullmatch (review L3)
# size caps for the update's downloads (review L6): streamed, and abandoned past these
MAX_INSTALLER = 600 * 1024 * 1024
MAX_SMALL = 1024 * 1024
UPDATE_PREFIX = "hazync-update-"

# The maintainer's OpenPGP v4 public-key packet body (SECURITY.md: "defenwycke <defenwycke@icloud.com>",
# 777FE81F 8CC077FD 3D08055E 852C2B31 90F5B928, Ed25519, created 2026-04-26, expires 2028-04-25), exported from
# https://github.com/defenwycke.gpg. Pinned as the PACKET so the fingerprint is computed from it, not asserted beside it.
MAINTAINER_KEY_PACKET = bytes.fromhex(
    "0469ee64fc16092b06010401da470f01010740b6803d72aab084f6a8128ccf6ba9573994c1f4fc4227a584e978adb8df919833")
_ED25519_OID = bytes.fromhex("2b06010401da470f01")
# The key's expiry, from its self-signature (subpacket 9: 63,072,000 s after creation 1777231100 = 2028-04-25 19:18:20
# UTC). Pinned as a number: the self-signature itself is not carried or verified here. A signature made after this is
# refused; one made before still verifies (SECURITY.md). After it, a new key means a manual reinstall (fails closed).
KEY_EXPIRES = 1777231100 + 63072000
CLOCK_SKEW_S = 24 * 3600                 # a signature dated further in the future than this is refused
_KNOWN_SUBPACKETS = {2, 3, 16, 33}       # creation time, signature expiry, issuer key id, issuer fingerprint


def key_fingerprint(packet=MAINTAINER_KEY_PACKET):
    """The v4 fingerprint (40 hex) of a public-key packet body: SHA1(0x99 || len16 || body)."""
    return hashlib.sha1(b"\x99" + len(packet).to_bytes(2, "big") + packet).hexdigest().upper()


def _ed25519_raw(packet=MAINTAINER_KEY_PACKET):
    """The 32-byte Ed25519 public key inside a v4 EdDSA (algo 22) key packet."""
    if packet[0] != 4 or packet[5] != 22:
        raise ValueError("not a v4 EdDSA key packet")
    oid_len = packet[6]
    if packet[7:7 + oid_len] != _ED25519_OID:
        raise ValueError("not an Ed25519 key")
    mpi = packet[7 + oid_len + 2:]
    if len(mpi) != 33 or mpi[0] != 0x40:
        raise ValueError("unexpected Ed25519 point encoding")
    return mpi[1:]


# ── versions ───────────────────────────────────────────────────────────────────────────────────────────────────────
def parse_version(s):
    """(major, minor, patch) from 'v0.26.1', '0.26.1' or '0.26.1-dev.12' (the dev suffix is ignored), or None."""
    m = re.match(r"^v?(\d+)\.(\d+)\.(\d+)", (s or "").strip())
    return tuple(int(x) for x in m.groups()) if m else None


def is_newer(tag, current):
    """True when release `tag` is a later version than `current`. Unknown either side: False (never nag on a guess)."""
    a, b = parse_version(tag), parse_version(current)
    return bool(a and b and a > b)


def app_version(exe=None):
    """This installed app's version, from the version.ini the installer writes beside Hazync.exe. "" from source."""
    if exe is None:
        if not getattr(sys, "frozen", False):
            return ""
        exe = sys.executable
    f = Path(exe).resolve().parent / VERSION_FILE
    try:
        for line in f.read_text(encoding="utf-8", errors="replace").splitlines():
            k, _, v = line.partition("=")
            if k.strip().lower() == "version" and parse_version(v):
                return v.strip()
    except OSError:
        pass
    return ""


def latest_tag(get_json, timeout=20):
    """The latest release's tag (validated vX.Y.Z), or raise ValueError. `get_json` is nethttp.get_json."""
    d = get_json(f"https://api.github.com/repos/{REPO}/releases/latest",
                 timeout=timeout, headers={"Accept": "application/vnd.github+json"})
    tag = (d or {}).get("tag_name") or ""
    if not TAG_RE.fullmatch(tag):
        raise ValueError(f"GitHub named no usable latest release ({tag!r})")
    return tag


def asset_url(tag, name):
    return f"https://github.com/{REPO}/releases/download/{tag}/{name}"


# ── OpenPGP: just enough to check one detached Ed25519 signature ──────────────────────────────────────────────────
def dearmor(text):
    """The binary inside an ASCII-armoured block (the CRC line is dropped; the signature itself is what is checked)."""
    lines = (text or "").replace("\r", "").split("\n")
    try:
        start = next(i for i, l in enumerate(lines) if l.startswith("-----BEGIN PGP SIGNATURE-----"))
        end = next(i for i, l in enumerate(lines) if i > start and l.startswith("-----END PGP SIGNATURE-----"))
    except StopIteration:
        raise ValueError("not an armoured PGP signature") from None
    body = lines[start + 1:end]
    # header lines ("Comment: ...") end at the first blank line
    if "" in body:
        body = body[body.index("") + 1:]
    body = [l for l in body if l and not l.startswith("=")]
    return base64.b64decode("".join(body), validate=True)


def _packets(data):
    """[(tag, body)] of OpenPGP packets (old and new format headers, definite lengths)."""
    out, i = [], 0
    while i < len(data):
        h = data[i]
        if not h & 0x80:
            raise ValueError("bad packet header")
        if h & 0x40:                        # new format
            tag = h & 0x3F
            o = data[i + 1]
            if o < 192:
                n, i = o, i + 2
            elif o < 224:
                n, i = ((o - 192) << 8) + data[i + 2] + 192, i + 3
            elif o == 255:
                n, i = int.from_bytes(data[i + 2:i + 6], "big"), i + 6
            else:
                raise ValueError("partial-length packets are not supported")
        else:                               # old format
            tag, lt = (h >> 2) & 0x0F, h & 3
            if lt == 3:
                raise ValueError("indeterminate-length packets are not supported")
            w = (1, 2, 4)[lt]
            n, i = int.from_bytes(data[i + 1:i + 1 + w], "big"), i + 1 + w
        if i + n > len(data):
            raise ValueError("truncated packet")
        out.append((tag, data[i:i + n]))
        i += n
    return out


def _mpi(buf, at):
    bits = int.from_bytes(buf[at:at + 2], "big")
    n = (bits + 7) // 8
    if at + 2 + n > len(buf):
        raise ValueError("truncated MPI")
    return buf[at + 2:at + 2 + n], at + 2 + n


def _subpackets(buf):
    out, i = [], 0
    while i < len(buf):
        o = buf[i]
        if o < 192:
            n, i = o, i + 1
        elif o < 255:
            n, i = ((o - 192) << 8) + buf[i + 1] + 192, i + 2
        else:
            n, i = int.from_bytes(buf[i + 1:i + 5], "big"), i + 5
        if n < 1 or i + n > len(buf):
            raise ValueError("bad subpacket")
        out.append((buf[i] & 0x7F, bool(buf[i] & 0x80), buf[i + 1:i + n]))
        i += n
    return out


def verify_signature(data, armored, key_packet=MAINTAINER_KEY_PACKET, now=None):
    """(ok, why): is `armored` a good detached OpenPGP signature over `data` (bytes) by the pinned key?

    Only what release-sign.yml produces is accepted: one v4 signature packet, binary document (class 0x00), EdDSA
    (22), SHA-256/384/512, issued by the pinned key, with a creation time that is not in the future and not after the
    key's expiry, not itself expired, and no critical subpacket this does not understand. Anything else is refused."""
    now = time.time() if now is None else now
    try:
        pk = _ed25519_raw(key_packet)
        fpr = key_fingerprint(key_packet)
        pkts = _packets(dearmor(armored))
    except (ValueError, IndexError) as e:
        return False, f"the signature could not be read: {e}"
    sigs = [b for t, b in pkts if t == 2]
    if len(sigs) != 1 or len(pkts) != 1:
        return False, f"expected exactly one signature packet, found {len(pkts)} packet(s)"
    s = sigs[0]
    try:
        if s[0] != 4:
            return False, f"signature version {s[0]} is not supported"
        sigclass, algo, halgo = s[1], s[2], s[3]
        if sigclass != 0x00:
            return False, f"signature class {sigclass:#x} is not a binary-document signature"
        if algo != 22:
            return False, f"signature algorithm {algo} is not EdDSA"
        hname = {8: "sha256", 9: "sha384", 10: "sha512"}.get(halgo)
        if not hname:
            return False, f"hash algorithm {halgo} is not accepted"
        hlen = int.from_bytes(s[4:6], "big")
        hashed = s[6:6 + hlen]
        at = 6 + hlen
        ulen = int.from_bytes(s[at:at + 2], "big")
        unhashed = s[at + 2:at + 2 + ulen]
        at += 2 + ulen
        left16 = s[at:at + 2]
        r, at = _mpi(s, at + 2)
        sv, at = _mpi(s, at)
        if at != len(s):
            return False, f"{len(s) - at} unexpected byte(s) after the signature"
        issuers, created, expires_after = set(), None, 0
        hashed_sub = _subpackets(hashed)
        for where, subs in (("hashed", hashed_sub), ("unhashed", _subpackets(unhashed))):
            for t, critical, v in subs:
                if critical and t not in _KNOWN_SUBPACKETS:
                    return False, f"a critical {where} subpacket of type {t} is not understood"
                if t == 33 and len(v) == 21 and v[0] == 4:
                    issuers.add(v[1:].hex().upper())
                elif t == 16 and len(v) == 8:
                    issuers.add(v.hex().upper())
                elif t == 2 and where == "hashed" and len(v) == 4:
                    created = int.from_bytes(v, "big")
                elif t == 3 and where == "hashed" and len(v) == 4:
                    expires_after = int.from_bytes(v, "big")
        if not issuers:
            return False, "the signature names no issuer"
        if not all(fpr.endswith(x) for x in issuers):
            return False, f"signed by another key ({', '.join(sorted(issuers))}), not {fpr}"
        if created is None:
            return False, "the signature carries no creation time"
        if created > now + CLOCK_SKEW_S:
            return False, f"the signature is dated in the future ({time.strftime('%Y-%m-%d', time.gmtime(created))})"
        if created > KEY_EXPIRES:
            return False, "the signature was made after the maintainer key expired (2028-04-25)"
        if expires_after and created + expires_after < now:
            return False, "the signature has expired"
        trailer = s[:6 + hlen]
        h = hashlib.new(hname, data + trailer + b"\x04\xff" + len(trailer).to_bytes(4, "big")).digest()
        if h[:2] != left16:
            return False, "the signature is not over this file (digest prefix differs)"
        if len(r) > 32 or len(sv) > 32:
            return False, "malformed EdDSA signature"
        sig = r.rjust(32, b"\0") + sv.rjust(32, b"\0")
    except (ValueError, IndexError) as e:
        return False, f"the signature could not be read: {e}"
    try:
        from cryptography.exceptions import InvalidSignature
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    except ImportError:
        return False, "cannot check the signature: the cryptography package is missing"
    try:
        Ed25519PublicKey.from_public_bytes(pk).verify(sig, h)
    except InvalidSignature:
        return False, "BAD signature: SHA256SUMS.txt was not signed by the Hazync maintainer key"
    return True, f"good signature by {fpr}"


def manifest_hash(manifest_text, name):
    """The sha256 the manifest gives for `name`, or ""."""
    for line in (manifest_text or "").splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[1].lstrip("*") == name and re.fullmatch(r"[0-9a-f]{64}", parts[0]):
            return parts[0]
    return ""


def manifest_tag(manifest_text):
    """The tag the manifest's signed first line names (`# hazync <tag> commit ...`), or ""."""
    first = (manifest_text or "").splitlines()[:1]
    m = re.match(r"^# hazync (v\d+\.\d+\.\d+) ", first[0]) if first else None
    return m.group(1) if m else ""


def check_manifest(tag, manifest, asc, key_packet=MAINTAINER_KEY_PACKET, now=None):
    """(ok, why): `manifest` (bytes) is signed by the pinned key (`asc`, armored text or bytes) AND its signed first line
    names `tag`. What windows-installer.yml checks before it trusts a release's hashes (review R3-6 #8, 28.11.2026)."""
    ok, why = verify_signature(manifest, asc.decode("ascii", "replace") if isinstance(asc, bytes) else asc,
                               key_packet=key_packet, now=now)
    if not ok:
        return False, why
    named = manifest_tag(manifest.decode("utf-8", "replace"))
    return (True, why) if named == tag else (False, f"the signed manifest is for {named or 'no tag'}, not {tag}")


def check_download(tag, manifest, asc, installer, key_packet=MAINTAINER_KEY_PACKET, verify=None):
    """(ok, why) for three downloaded byte strings: signature, then the tag line, then the installer's hash."""
    ok, why = (verify or verify_signature)(manifest, asc.decode("ascii", "replace") if isinstance(asc, bytes) else asc,
                                           key_packet=key_packet)
    if not ok:
        return False, why
    text = manifest.decode("utf-8", "replace")
    if manifest_tag(text) != tag:
        return False, f"the signed manifest is for {manifest_tag(text) or 'no tag'}, not {tag}"
    want = manifest_hash(text, INSTALLER)
    if not want:
        return False, f"the signed manifest of {tag} lists no {INSTALLER}"
    got = hashlib.sha256(installer).hexdigest()
    if got != want:
        return False, f"{INSTALLER} has sha256 {got[:16]}..., the signed manifest says {want[:16]}..."
    return True, f"{why}; {INSTALLER} sha256 {got[:16]}... matches {tag}'s signed manifest"


def download_verified(tag, current, get, timeout=600, key_packet=MAINTAINER_KEY_PACKET, tmp_root=None):
    """Fetch the release's installer + manifest + signature, verify them, and only then write the installer, into a
    fresh private folder (review L5). (ok, path_or_why, sha256). `get` is nethttp.get (url, timeout=, max_bytes=).

    ⛔ (review M1) refused unless `tag` is strictly newer than `current`, this app's version: whatever releases/latest
    names is otherwise installed, so a signed OLDER release marked latest would downgrade the app, and an "update" to
    the same version would loop. A copy with no version (from source) never updates itself."""
    if not TAG_RE.fullmatch(tag or ""):
        return False, f"not a release tag: {tag!r}", ""
    if not parse_version(current):
        return False, "this copy of Hazync has no version (it runs from source), so it does not update itself", ""
    if not is_newer(tag, current):
        return False, f"{tag} is not newer than this Hazync (v{current}): there is no newer release to install yet", ""
    try:
        manifest = get(asset_url(tag, MANIFEST), timeout=60, max_bytes=MAX_SMALL)
        asc = get(asset_url(tag, MANIFEST + ".asc"), timeout=60, max_bytes=MAX_SMALL)
        installer = get(asset_url(tag, INSTALLER), timeout=timeout, max_bytes=MAX_INSTALLER)
    except Exception as e:     # noqa: BLE001 - nethttp.NetError and friends: say it, do not run anything
        return False, f"could not download {tag}: {e}", ""
    if len(manifest) > MAX_SMALL or len(asc) > MAX_SMALL or len(installer) > MAX_INSTALLER:
        return False, f"a download of {tag} is larger than any release file should be", ""
    ok, why = check_download(tag, manifest, asc, installer, key_packet=key_packet)
    if not ok:
        return False, why, ""
    root = Path(tmp_root or tempfile.gettempdir())
    try:
        d = Path(tempfile.mkdtemp(prefix=UPDATE_PREFIX, dir=root))
        dest = d / f"hazync-setup-{tag}.exe"
        dest.write_bytes(installer)
    except OSError as e:
        return False, f"verified, but could not save it: {e}", ""
    remove_old_installers(root, keep=d)
    return True, str(dest), hashlib.sha256(installer).hexdigest()


def remove_old_installers(root, keep=None):
    """Delete earlier updates' folders (hazync-update-*) under `root`, except `keep`. Best effort."""
    for p in Path(root).glob(UPDATE_PREFIX + "*"):
        if keep is not None and p.resolve() == Path(keep).resolve():
            continue
        if p.is_dir():
            shutil.rmtree(p, ignore_errors=True)


def installer_command(path):
    """The installer, run as a person would run it: its own wizard, which offers to open Hazync at the end."""
    return [str(path)]


def launch_installer(path, sha256):
    """Start the installer detached, so it outlives this app (which must close for its files to be replaced).

    The file is read and hashed again right before it is started (review L5): what runs is what was verified."""
    if hashlib.sha256(Path(path).read_bytes()).hexdigest() != sha256:
        raise OSError(f"{path} changed after it was verified; not started")
    flags = 0
    if sys.platform == "win32":
        flags = subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP
    return subprocess.Popen(installer_command(path), close_fds=True, creationflags=flags)


# ── after an upgrade: a remembered prover must not outlive it ─────────────────────────────────────────────────────
def upgrade_resets(remembered, installed, image_of, reference):
    """{kind: installed path} for each remembered prover to replace after the app's version changed.

    `remembered` / `installed`: {kind: path}; `image_of(path)` -> 64-hex id or None; `reference`: the board's image
    (or the app's own when the board could not be asked). A remembered prover is replaced only when it is not the
    installed one, does not make `reference`, and the installed one DOES: never swapped for something no better."""
    out = {}
    for kind, inst in (installed or {}).items():
        rem = (remembered or {}).get(kind) or ""
        if not rem or not inst:
            continue
        try:
            if Path(rem).resolve() == Path(inst).resolve():
                continue
        except OSError:
            pass
        if Path(rem).is_file() and image_of(rem) == reference:
            continue
        if image_of(inst) == reference:
            out[kind] = inst
    return out


def worker_reset(remembered, installed, facts):
    """The installed worker's path when the remembered one should be replaced after the app's version changed, else "".

    Replaced unless it IS the installed one, or it reads the board's input files and is stamped at least as new as
    the installed one (`facts` is supervisor.worker_facts). A checkout's "dev" worker has no version, so it is replaced
    too: the person can choose it again, and the feed names it."""
    if not remembered or not installed:
        return ""
    try:
        if Path(remembered).resolve() == Path(installed).resolve():
            return ""
    except OSError:
        pass
    fi = facts(installed)
    if not fi.get("lzleaf"):
        return ""                       # never swap for one that cannot read the board's files either
    if Path(remembered).is_file():
        fr = facts(remembered)
        a, b = parse_version(fr.get("version")), parse_version(fi.get("version"))
        if fr.get("lzleaf") and a and b and a >= b:
            return ""
    return installed
