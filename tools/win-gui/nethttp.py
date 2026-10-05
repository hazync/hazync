"""HTTPS that works on a stock Windows machine, which `urllib` on its own does not.

⛔⛔ THE MEASUREMENT. On a real Windows 11 box, 2026-10-05, the GUI could not fetch the client:

    could not download https://github.com/hazync/hazync/releases/download/v0.22.1/hazync-worker:
    <urlopen error [SSL: CERTIFICATE_VERIFY_FAILED] certificate verify failed:
     unable to get local issuer certificate (_ssl.c:1028)>

⚠ AND api.github.com HAD JUST WORKED, in the same process, seconds earlier -- that is how the
version number got into the URL. So this is not "no internet" and not "GitHub is down", and any
message saying either would send someone looking in the wrong place.

WHY ONE HOST VERIFIES AND ANOTHER DOES NOT: THEY HAVE DIFFERENT ROOTS. Measured by reading the
chains directly:

    api.github.com, github.com        Sectigo Public Server Authentication Root E46   -> trusted
    objects.githubusercontent.com     ISRG Root YR  (Let's Encrypt)                   -> NOT trusted
    api.hazync.org                    ISRG Root YE / ISRG Root X2                     -> NOT trusted

The release download redirects from github.com to objects.githubusercontent.com, so the request
crosses from a root this machine trusts to one it does not, mid-download. ⚠ AND api.hazync.org IS IN
THE SAME FAMILY — so the dashboard and the block map were going to fail on that machine too, for a
reason nothing would have reported.

⛔ IT IS NOT A STALE LOCAL CACHE. curl.exe, which verifies through Schannel and Windows' own
CryptoAPI rather than OpenSSL, fails identically:

    curl: (60) schannel: SEC_E_UNTRUSTED_ROOT (0x80090325)
          - The certificate chain was issued by an authority that is not trusted.

Both of this machine's trust stores genuinely lack the newer ISRG roots. ⇒ Nothing that consults
Windows can fix this, which is why the fallback is a CA bundle of our own and not another OS call.

⭐ certifi HAS THEM, AND IS ALREADY ON THE MACHINE. Measured: certifi verifies all three hosts. And
it explains the thing that otherwise makes no sense — `pip install cryptography` SUCCEEDED on the
same box minutes earlier. pip vendors its own CA bundle and never asks Windows. So the bundle that
fixes this is sitting inside pip already, importable as `pip._vendor.certifi`, with nothing to
install and no network needed to obtain it.

FOUR ATTEMPTS, CHEAPEST FIRST:

    1. the default context          — correct on most machines, costs nothing to try
    2. certifi, or pip's vendored copy of it — the bundle that actually has the missing roots
    3. pip install certifi, then retry      — pip's own HTTPS works, so this can succeed
    4. curl                                 — last, because on the measured machine it failed too

⛔ WHAT THIS DELIBERATELY DOES NOT DO: disable verification. An `ssl._create_unverified_context()`
would have "fixed" this in one line and quietly made every download in this program spoofable --
including the client binary that signs, and is trusted to sign, this machine's claims. A tool that
downloads executables must not be the one that stops checking who it is talking to. Every path here
verifies; they differ only in WHICH trust store answers.
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()   # ⛔ BEFORE anything prints: a ✅ on a cp1252 console raises, not degrades

import json
import shutil
import ssl
import subprocess
import urllib.error
import urllib.request

IS_WINDOWS = _os.name == "nt"
UA = "hazync-win-gui"


class NetError(Exception):
    """Carries a sentence a person can act on, not a stack trace."""


def _is_cert_error(exc):
    """Is this the local-trust-store failure, as opposed to a real network or server problem?

    ⚠ Matched on the reason TEXT as well as the type. urllib wraps the SSLError in a URLError, so
    `isinstance(e, ssl.SSLCertVerificationError)` is False for exactly the case this module exists
    for -- which is how a first attempt at this missed every real occurrence.
    """
    if isinstance(exc, ssl.SSLCertVerificationError):
        return True
    reason = getattr(exc, "reason", None)
    if isinstance(reason, ssl.SSLCertVerificationError):
        return True
    return "CERTIFICATE_VERIFY_FAILED" in str(exc)


def certifi_where():
    """Path to a CA bundle that has the modern roots, or None.

    ⭐ pip's VENDORED COPY IS THE POINT. A machine that cannot verify these roots can still have a
    working pip, because pip ships its own bundle -- measured: `pip install cryptography` succeeded
    on the very box where this module's first attempt fails. So the bundle is already present with
    nothing to install, which turns "run this command and try again" into "it just works".
    """
    try:
        import certifi
        return certifi.where()
    except (ImportError, OSError):
        pass
    try:
        from pip._vendor import certifi as vendored   # noqa: PLC0415
        return vendored.where()
    except (ImportError, OSError, AttributeError):
        return None


def _certifi_context():
    """An SSL context using a real CA bundle, or None if none can be found."""
    where = certifi_where()
    if not where:
        return None
    try:
        return ssl.create_default_context(cafile=where)
    except OSError:
        return None


def _pip_install_certifi(timeout=180):
    """Last resort before curl: let pip fetch certifi, since pip's own HTTPS works regardless."""
    try:
        p = subprocess.run([_sys.executable, "-m", "pip", "install", "--upgrade", "certifi"],
                           capture_output=True, text=True, timeout=timeout)
    except (subprocess.TimeoutExpired, OSError):
        return None
    if p.returncode != 0:
        return None
    # ⚠ Re-import so the just-installed package is visible in THIS process.
    import importlib
    import site
    importlib.reload(site)
    for m in ("certifi",):
        _sys.modules.pop(m, None)
    return _certifi_context()


def _curl(url, timeout):
    """Fetch through curl.exe, which verifies via Schannel rather than OpenSSL.

    ⚠ -f so an HTTP error is a non-zero exit instead of a saved error page: a 404 body written to
    disk as if it were the file is the failure mode check_worker already had to be taught about.
    """
    exe = shutil.which("curl")
    if not exe:
        return None
    try:
        p = subprocess.run([exe, "-fsSL", "--max-time", str(int(timeout)),
                            "-A", UA, "-o", "-", url],
                           capture_output=True, timeout=timeout + 15)
    except (subprocess.TimeoutExpired, OSError):
        return None
    if p.returncode != 0:
        return None
    return p.stdout


def get(url, timeout=60, headers=None):
    """Fetch `url` and return its bytes, or raise NetError saying what to do about it."""
    hdrs = {"User-Agent": UA}
    hdrs.update(headers or {})
    attempts = []

    def _try(ctx, label):
        req = urllib.request.Request(url, headers=hdrs)
        with urllib.request.urlopen(req, timeout=timeout, context=ctx) as r:
            return r.read()

    # 1 — the default trust store
    try:
        return _try(None, "default")
    except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, OSError) as e:
        if isinstance(e, urllib.error.HTTPError):
            raise NetError(f"{url} answered HTTP {e.code} {e.reason}") from e
        if not _is_cert_error(e):
            raise NetError(f"could not reach {url}: {e}") from e
        attempts.append(f"system trust store: {e}")

    # 2 — a real CA bundle: certifi, or the copy vendored inside pip
    ctx = _certifi_context()
    if ctx is not None:
        try:
            return _try(ctx, "certifi")
        except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, OSError) as e:
            attempts.append(f"CA bundle ({certifi_where()}): {e}")
    else:
        attempts.append("CA bundle: neither certifi nor pip's vendored copy could be imported")

    # 3 — ask pip to fetch certifi, then retry. pip's HTTPS works even here.
    ctx = _pip_install_certifi()
    if ctx is not None:
        try:
            return _try(ctx, "certifi-installed")
        except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, OSError) as e:
            attempts.append(f"certifi after pip install: {e}")
    else:
        attempts.append("pip install certifi: did not help")

    # 4 — curl, verifying through the OS
    data = _curl(url, timeout)
    if data is not None:
        return data
    attempts.append("curl (Schannel): not available, or it failed too")

    raise NetError(
        f"could not verify the HTTPS certificate for {url}. This machine's certificate store "
        f"could not supply the issuer, and the fallbacks did not help.\n  "
        + "\n  ".join(attempts)
        + ("\n\nFix it with:  py -3 -m pip install --upgrade certifi"
           if IS_WINDOWS else ""))


def get_json(url, timeout=60, headers=None):
    hdrs = {"Accept": "application/json"}
    hdrs.update(headers or {})
    raw = get(url, timeout=timeout, headers=hdrs)
    try:
        return json.loads(raw.decode("utf-8", "replace"))
    except ValueError as e:
        raise NetError(f"{url} did not return JSON ({len(raw):,} bytes): {e}") from e


def diagnose(timeout=15):
    """Report which trust paths work on THIS machine. Used by the GUI and runnable by hand."""
    url = "https://objects.githubusercontent.com/"
    rows = []
    try:
        urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": UA}),
                               timeout=timeout)
        rows.append(("system trust store", True, "verifies"))
    except urllib.error.HTTPError:
        rows.append(("system trust store", True, "verifies (server answered an error, which is fine)"))
    except Exception as e:  # noqa: BLE001 - reporting, not handling
        rows.append(("system trust store", not _is_cert_error(e),
                     "CERTIFICATE_VERIFY_FAILED" if _is_cert_error(e) else str(e)[:60]))
    rows.append(("certifi bundle", _certifi_context() is not None,
                 "installed" if _certifi_context() is not None else "not installed"))
    rows.append(("curl", shutil.which("curl") is not None,
                 shutil.which("curl") or "not found"))
    return rows


if __name__ == "__main__":
    print("How this machine can verify HTTPS:")
    for name, ok, detail in diagnose():
        print(f"  {'ok  ' if ok else 'FAIL'} {name}: {detail}")
