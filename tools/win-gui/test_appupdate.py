#!/usr/bin/env python3
"""Updating the installed Windows app (appupdate.py), headless: no display, no network, no Windows.

    python3 test_appupdate.py             # the real logic
    python3 test_appupdate.py --control   # naive versions, each of which must break

What is pinned:
  * the maintainer key pinned in appupdate.py IS the one SECURITY.md publishes (fingerprint computed from the packet);
  * the real v0.26.1 SHA256SUMS.txt + .asc (copied below, as published) verify against it in pure Python, and one
    flipped byte in either is refused;
  * download_verified runs nothing it has not verified: a forged manifest, a wrong installer hash, an older signed
    manifest replayed under a newer tag and a download error each leave no installer on disk;
  * versions: a newer release is offered, the same or an older one never; a dev build reads as its release;
  * after an upgrade, a remembered prover/worker that is an old copy is replaced by the installed one, and one that is
    fine (or better) is left alone.

The controls: check_download with the signature check stubbed out accepts a forged release whose hashes all match; an upgrade reset that
replaces every non-installed prover throws away a good one the person chose.
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()

import base64
import hashlib
import os
import re
import sys
import tempfile
import time
from pathlib import Path

import appupdate  # noqa: E402

CONTROL = "--control" in sys.argv
HERE = Path(__file__).resolve().parent
fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


# The release's own files, as published on v0.26.1 (gh release download v0.26.1 -p 'SHA256SUMS.txt*').
REAL_MANIFEST = (
    "# hazync v0.26.1 commit 3b9a90a228feb5e24ad3705b2ee9695744ad5a05 image "
    "46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c\n"
    "# signed by .github/workflows/release-sign.yml (run 44167720461); check with: sha256sum -c SHA256SUMS.txt\n"
    "1fb5848ab64b6603b27079d4a09dd4a90c8d7d86ee8883c834eeb3dd69eaa795  hazync-coordinator.py\n"
    "a051ccf5607b17712d4a858b26b109e6c679c8de40800563276a9fa0e4904050  hazync-host-x86_64-linux-gnu\n"
    "c02087f0b72d6c688ef3ade9b74e458bfe34cd234c9e3ee5a34bae4130f0690f  hazync-host-x86_64-linux-gnu-cuda\n"
    "31c2b17e5d993e4d29cb445b405805e94f9710b09d6ec7a95657ea9bfdec0d05  hazync-host-x86_64-windows-cpu.zip\n"
    "23f5b58e0c5613c3a73c9ef39f7e737c2f73dac92f5a7ca0ab7d6c26d910a8ec  hazync-host-x86_64-windows-cuda.zip\n"
    "2613ac09a1a662ec5847ca2b58b38b58dc8fdb2b3599290fc142aef14729da40  hazync-run-workers.sh\n"
    "ef4210f10cc7696ea8f77a6f79eba928f6adcc99fea321f0900cf9af67ff0054  hazync-setup-x86_64-windows.exe\n"
    "d654d2f7f9869e42000cd9691284b35a42dde40cd25a721965441d40757b5383  hazync-verify-aarch64\n"
    "a3272c6f3364a35e83829cc38619955542717a2eb0f39f7bb4b685441c5b009a  hazync-verify-x86_64-linux-gnu\n"
    "f24777b693f9667218e0f6267e29490f36baba9604a26079a30932c45070ec7a  hazync-verify.js\n"
    "996f1ed661c2b68d4e5bcc6b56f53a5fbfdf910ef59e7ba57bd9a436f19081a9  hazync-verify.wasm\n"
    "db052bf01e50fea5e7e68ef7f58461ee91f5535fbc648017abcbe255fafd6437  hazync-worker\n").encode()
REAL_ASC = """-----BEGIN PGP SIGNATURE-----

iHUEABYKAB0WIQR3f+gfjMB3/T0IBV6FLCsxkPW5KAUCavy3/QAKCRCFLCsxkPW5
KE0/AQC/0LD3WWE9uXjXEUl8Q1O3cMtWhlODlPGISfY4pKp4dwEApsHntRpiszyj
qdE2LtU9475aAVGqjlliemB5/pWd6gE=
=9djT
-----END PGP SIGNATURE-----
"""


# ── a throwaway Ed25519 OpenPGP key, to sign test releases the way release-sign.yml signs real ones ─────────────
def test_key():
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
    sk = Ed25519PrivateKey.generate()
    raw = sk.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    packet = (b"\x04" + int(time.time()).to_bytes(4, "big") + b"\x16" + b"\x09" + bytes.fromhex("2b06010401da470f01")
              + (263).to_bytes(2, "big") + b"\x40" + raw)
    return sk, packet


def _mpi(b):
    b = b.lstrip(b"\0") or b"\0"
    return (b[0].bit_length() + 8 * (len(b) - 1)).to_bytes(2, "big") + b


def sign_packet(sk, packet, data, sigclass=0x00, halgo=10, created=None, extra_hashed=b"", issuer=True,
                trailing=b"", text_canon=False):
    """One v4 EdDSA signature packet body over `data`, the way gpg makes them, with knobs to make it wrong."""
    fpr = bytes.fromhex(appupdate.key_fingerprint(packet))
    created = int(time.time()) if created is None else created
    hashed = (bytes([22, 33, 4]) + fpr if issuer else b"") + bytes([5, 2]) + created.to_bytes(4, "big") + extra_hashed
    trailer = bytes([4, sigclass, 22, halgo]) + len(hashed).to_bytes(2, "big") + hashed
    hname = {2: "sha1", 8: "sha256", 10: "sha512"}[halgo]
    body_data = data.replace(b"\n", b"\r\n") if text_canon else data
    digest = hashlib.new(hname, body_data + trailer + b"\x04\xff" + len(trailer).to_bytes(4, "big")).digest()
    sig = sk.sign(digest)
    unhashed = (bytes([9, 16]) + fpr[-8:]) if issuer else b""
    return trailer + len(unhashed).to_bytes(2, "big") + unhashed + digest[:2] + _mpi(sig[:32]) + _mpi(sig[32:]) + trailing


def armor(pkt):
    b64 = base64.b64encode(pkt).decode()
    return ("-----BEGIN PGP SIGNATURE-----\n\n" + "\n".join(b64[i:i + 64] for i in range(0, len(b64), 64))
            + "\n=AAAA\n-----END PGP SIGNATURE-----\n")


def new_hdr(body):
    return bytes([0xC2, len(body)]) + body


def sign(sk, packet, data, **kw):
    return armor(new_hdr(sign_packet(sk, packet, data, **kw)))


def release(tag, installer, sk, packet, manifest_tag=None):
    m = (f"# hazync {manifest_tag or tag} commit {'0' * 40} image {'1' * 64}\n"
         f"{hashlib.sha256(installer).hexdigest()}  {appupdate.INSTALLER}\n").encode()
    return {appupdate.asset_url(tag, appupdate.MANIFEST): m,
            appupdate.asset_url(tag, appupdate.MANIFEST + ".asc"): sign(sk, packet, m).encode(),
            appupdate.asset_url(tag, appupdate.INSTALLER): installer}


def getter(files, asked=None):
    def get(url, timeout=60, max_bytes=None):
        if asked is not None:
            asked.append(url)
        if url not in files:
            raise OSError(f"404 {url}")
        if max_bytes and len(files[url]) > max_bytes:
            raise OSError(f"{url} is over {max_bytes} bytes")
        return files[url]
    return get


def main():
    print("── the pinned key is the one SECURITY.md publishes ──")
    sec = (HERE.parent.parent / "SECURITY.md").read_text(encoding="utf-8")
    published = {re.sub(r"\s", "", m) for m in re.findall(r"^([0-9A-F]{8}(?: [0-9A-F]{8}){4})$", sec, re.M)}
    check(published == {appupdate.key_fingerprint()},
          f"fingerprint of the pinned packet {appupdate.key_fingerprint()} = SECURITY.md's {sorted(published)}")

    print("── the real v0.26.1 release signature ──")
    ok, why = appupdate.verify_signature(REAL_MANIFEST, REAL_ASC)
    check(ok, f"verifies in pure Python against the pinned key: {why}")
    ok, why = appupdate.verify_signature(REAL_MANIFEST.replace(b"ef4210f1", b"ef4210f2"), REAL_ASC)
    check(not ok, f"one changed hash in the manifest is refused: {why}")
    bad = REAL_ASC.replace("qdE2LtU9", "qdE2LtU8")
    ok, why = appupdate.verify_signature(REAL_MANIFEST, bad)
    check(not ok, f"one changed byte of the signature is refused: {why}")
    check(appupdate.manifest_tag(REAL_MANIFEST.decode()) == "v0.26.1", "the signed first line names its tag")
    check(appupdate.manifest_hash(REAL_MANIFEST.decode(), appupdate.INSTALLER).startswith("ef4210f1"),
          "the installer's hash is read from the manifest")

    print("── the signature's own rules (review L1, L2, L10) ──")
    sk, pkt = test_key()
    data = b"# hazync v0.27.0 commit x\nabc  file\n"
    V = lambda asc, **k: appupdate.verify_signature(data, asc, key_packet=pkt, **k)   # noqa: E731
    ok, why = V(sign(sk, pkt, data))
    check(ok, f"a well-formed signature by the key verifies: {why}")
    for label, asc in (
            ("text mode (class 0x01)", sign(sk, pkt, data, sigclass=0x01, text_canon=True)),
            ("SHA-1", sign(sk, pkt, data, halgo=2)),
            ("an unknown critical subpacket", sign(sk, pkt, data, extra_hashed=bytes([2, 0x80 | 25, 1]))),
            ("an expired signature (type 3)", sign(sk, pkt, data, created=int(time.time()) - 7200,
                                                    extra_hashed=bytes([5, 3]) + (60).to_bytes(4, "big"))),
            ("one dated next week", sign(sk, pkt, data, created=int(time.time()) + 7 * 86400)),
            ("one made after the key expired", sign(sk, pkt, data, created=appupdate.KEY_EXPIRES + 1)),
            ("no issuer", sign(sk, pkt, data, issuer=False)),
            ("bytes after the S value", sign(sk, pkt, data, trailing=b"\x00")),
            ("two signature packets", armor(new_hdr(sign_packet(sk, pkt, data)) * 2)),
            ("a partial-length packet", armor(bytes([0xC2, 0xE5]) + sign_packet(sk, pkt, data)))):
        ok, why = V(asc, now=appupdate.KEY_EXPIRES + 10 if "after the key" in label else None)
        check(not ok, f"{label} is refused: {why}")
    ok, why = V(sign(sk, pkt, data, extra_hashed=bytes([2, 25, 1])))
    check(ok, f"an unknown NON-critical subpacket is ignored, as OpenPGP says: {why}")

    print("── download_verified: nothing unverified reaches the disk ──")
    inst = b"MZ" + os.urandom(4096)
    tmp = Path(tempfile.mkdtemp())
    files = release("v0.27.0", inst, sk, pkt)
    stale = tmp / (appupdate.UPDATE_PREFIX + "old")
    stale.mkdir()
    (stale / "hazync-setup-v0.26.0.exe").write_bytes(b"old")
    ok, path, sha = appupdate.download_verified("v0.27.0", "0.26.1", getter(files), key_packet=pkt, tmp_root=tmp)
    check(ok and Path(path).read_bytes() == inst and sha == hashlib.sha256(inst).hexdigest(),
          f"a signed release is saved: {path}")
    check(Path(path).parent.name.startswith(appupdate.UPDATE_PREFIX) and Path(path).parent != stale
          and not stale.exists(), "into a fresh folder, and an earlier update's installer is deleted (L5)")
    ran = []
    real_popen = appupdate.subprocess.Popen
    appupdate.subprocess.Popen = lambda *a, **k: ran.append(a[0])
    try:
        appupdate.launch_installer(path, sha)
        check(ran == [[path]], "the verified file is started")
        Path(path).write_bytes(inst + b"swapped")
        try:
            appupdate.launch_installer(path, sha)
            check(False, "a file changed after verification is not started")
        except OSError as e:
            check(len(ran) == 1, f"a file changed after verification is not started: {e}")
    finally:
        appupdate.subprocess.Popen = real_popen

    def refused(label, tag, current, get, key=pkt, want=""):
        before = set(tmp.iterdir())
        ok, why, _ = appupdate.download_verified(tag, current, get, key_packet=key, tmp_root=tmp)
        check(not ok and set(tmp.iterdir()) == before and want in why, f"{label}: refused, nothing saved: {why}")

    refused("signed by another key (not the maintainer's)", "v0.27.0", "0.26.1", getter(files),
            key=appupdate.MAINTAINER_KEY_PACKET)
    other_sk, _ = test_key()
    forged = dict(files)
    forged[appupdate.asset_url("v0.27.0", appupdate.MANIFEST + ".asc")] = sign(
        other_sk, pkt, files[appupdate.asset_url("v0.27.0", appupdate.MANIFEST)]).encode()
    refused("the right key id but a wrong secret", "v0.27.0", "0.26.1", getter(forged))
    swapped = dict(files)
    swapped[appupdate.asset_url("v0.27.0", appupdate.INSTALLER)] = inst + b"x"
    refused("an installer that is not the signed one", "v0.27.0", "0.26.1", getter(swapped))
    old = release("v0.26.0", inst, sk, pkt)
    replay = {u.replace("v0.26.0", "v0.27.0"): d for u, d in old.items()}
    refused("an older signed manifest under a newer tag", "v0.27.0", "0.26.1", getter(replay), want="v0.26.0")
    refused("a failed download", "v0.27.0", "0.26.1", getter({}))
    refused("a tag that is not vX.Y.Z", "../x", "0.26.1", getter(files))
    refused("a tag with a trailing newline (fullmatch, L3)", "v0.27.0\n", "0.26.1", getter(files))

    print("── never a downgrade, never a loop (review M1) ──")
    for tag, current, label in (("v0.26.0", "0.26.1", "an older signed release marked latest"),
                                ("v0.26.1", "0.26.1", "the same version (a reinstall loop)"),
                                ("v0.27.0", "", "a copy with no version (from source)")):
        asked = []
        files_t = release(tag, inst, sk, pkt)
        refused(label, tag, current, getter(files_t, asked))
        check(not asked, f"...without downloading anything ({len(asked)} requests)")

    print("── download size caps (review L6) ──")
    big = dict(files)
    big[appupdate.asset_url("v0.27.0", appupdate.MANIFEST + ".asc")] = b"x" * (appupdate.MAX_SMALL + 1)
    refused("a signature file over 1 MB", "v0.27.0", "0.26.1", getter(big))
    import http.server
    import threading
    import nethttp

    class H(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.end_headers()
            try:
                for _ in range(64):
                    self.wfile.write(b"x" * 65536)
            except OSError:
                pass

        def log_message(self, *a):
            pass
    srv = http.server.HTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{srv.server_address[1]}/big"
    try:
        nethttp.get(url, timeout=10, max_bytes=1 << 20)
        check(False, "nethttp.get stops reading past max_bytes")
    except nethttp.TooLarge as e:
        check(True, f"nethttp.get stops reading past max_bytes: {e}")
    check(len(nethttp.get(url, timeout=10)) == 64 * 65536, "without a cap it reads everything, as before")
    srv.shutdown()

    print("── versions ──")
    check(appupdate.is_newer("v0.27.0", "0.26.1"), "v0.27.0 is newer than 0.26.1")
    check(not appupdate.is_newer("v0.26.1", "0.26.1"), "the same version is not offered")
    check(not appupdate.is_newer("v0.26.0", "0.26.1"), "an older release is never offered")
    check(not appupdate.is_newer("v0.26.1", "0.26.1-dev.42"), "a dev build of a release reads as that release")
    check(appupdate.is_newer("v0.26.2", "0.26.1-dev.42"), "...and is offered the next one")
    check(not appupdate.is_newer("", "0.26.1") and not appupdate.is_newer("v0.27.0", ""), "unknown: never offered")
    d = Path(tempfile.mkdtemp())
    (d / "Hazync.exe").write_bytes(b"")
    check(appupdate.app_version(d / "Hazync.exe") == "", "no version.ini: no version (no update check)")
    (d / "version.ini").write_text("[hazync]\r\nversion=0.26.1\r\n", encoding="utf-8")
    check(appupdate.app_version(d / "Hazync.exe") == "0.26.1", "version.ini as Inno Setup writes it")
    try:
        appupdate.latest_tag(lambda *a, **k: {"tag_name": "v0.27.0; rm -rf"})
        check(False, "a malformed tag from the API is refused")
    except ValueError:
        check(True, "a malformed tag from the API is refused")
    check(appupdate.latest_tag(lambda *a, **k: {"tag_name": "v0.27.0"}) == "v0.27.0", "a good tag is read")

    print("── after an upgrade: the remembered prover ──")
    BOARD, OLD = "4" * 64, "3" * 64
    d = Path(tempfile.mkdtemp())
    paths = {n: d / n for n in ("old.exe", "inst.exe", "inst-cpu.exe", "mine.exe")}
    for p in paths.values():
        p.write_bytes(b"x")
    images = {str(paths["old.exe"]): OLD, str(paths["inst.exe"]): BOARD, str(paths["inst-cpu.exe"]): BOARD,
              str(paths["mine.exe"]): BOARD}
    image_of = images.get
    resets = (lambda rem, inst: {k: inst[k] for k in inst if rem.get(k) and rem[k] != inst[k]}) if CONTROL \
        else (lambda rem, inst: appupdate.upgrade_resets(rem, inst, image_of, BOARD))
    inst = {"cuda": str(paths["inst.exe"]), "cpu": str(paths["inst-cpu.exe"])}
    got = resets({"cuda": str(paths["old.exe"]), "cpu": ""}, inst)
    check(got == {"cuda": str(paths["inst.exe"])}, f"an old prover making image 33333333 is replaced: {got}")
    got = resets({"cuda": str(paths["mine.exe"])}, inst)
    check(got == {}, f"a prover the person chose that makes the board's image is kept: {got}")
    got = resets({"cuda": str(paths["inst.exe"])}, inst)
    check(got == {}, "the installed one stays")
    images[str(paths["inst.exe"])] = OLD
    got = resets({"cuda": str(paths["old.exe"])}, inst)
    check(got == {}, f"never swapped for an installed one that is no better: {got}")

    print("── after an upgrade: the remembered worker ──")
    facts = {"old": {"version": "0.21.0", "lzleaf": False}, "inst": {"version": "0.26.1", "lzleaf": True},
             "dev": {"version": "dev", "lzleaf": True}, "newer": {"version": "0.27.0", "lzleaf": True}}
    for n in facts:
        (d / n).write_bytes(b"x")
    f = lambda p: facts[Path(p).name]   # noqa: E731
    check(appupdate.worker_reset(str(d / "old"), str(d / "inst"), f) == str(d / "inst"),
          "a worker that cannot read the board's files is replaced")
    check(appupdate.worker_reset(str(d / "newer"), str(d / "inst"), f) == "", "a newer worker is kept")
    check(appupdate.worker_reset(str(d / "inst"), str(d / "inst"), f) == "", "the installed one stays")
    check(appupdate.worker_reset(str(d / "dev"), str(d / "inst"), f) == str(d / "inst"),
          "an unversioned (dev) worker is replaced by the installed one")
    check(appupdate.worker_reset(str(d / "inst"), str(d / "old"), f) == "",
          "never swapped for an installed one that cannot read the board's files")

    print("── the signature is what refuses a forged release (review L9) ──")
    evil = b"MZ evil"
    m = f"# hazync v0.27.0 x\n{hashlib.sha256(evil).hexdigest()}  {appupdate.INSTALLER}\n".encode()
    verify = (lambda *a, **k: (True, "stubbed")) if CONTROL else None
    ok, why = appupdate.check_download("v0.27.0", m, "not a signature", evil, verify=verify)
    check(not ok, f"an unsigned release whose hashes all match is refused: {why}")

    print()
    if CONTROL:
        print("CONTROL:", "FAILED as it must" if fails else "PASSED (the test is inert!)")
        return 0 if fails else 1
    print("PASS" if not fails else f"FAIL ({len(fails)})")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
