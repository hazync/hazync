#!/usr/bin/env python3
"""Tests for nethttp.py — the HTTPS fallbacks, and the line they must never cross.

⛔⛔ THE FAILURE THIS EXISTS FOR, measured on Windows 11 2026-10-05:

    could not download https://github.com/.../hazync-worker:
    <urlopen error [SSL: CERTIFICATE_VERIFY_FAILED] certificate verify failed:
     unable to get local issuer certificate (_ssl.c:1028)>

while api.github.com had verified fine seconds earlier in the same process.

⛔⛔ THE ASSERTION THAT MATTERS MOST IS THE SECURITY ONE. `ssl._create_unverified_context()` would
have "fixed" this in one line, and would have made every download in this program spoofable —
including hazync-worker, the binary that signs and is trusted to sign this machine's claims. A
program that downloads executables must not be the one that stops checking who it is talking to.
Case 4 fails if verification is ever switched off anywhere in the module.

⚠ NO NETWORK IN THE DEFAULT RUN. Exceptions are built by hand in the shapes urllib really produces,
because the subtle case — a cert error arriving WRAPPED in URLError, so `isinstance` against
SSLCertVerificationError is False — is exactly the one a live test on a healthy machine never hits.

  python3 test_nethttp.py            # assertions; exit 0 on success
  python3 test_nethttp.py --control  # the naive isinstance check; MUST miss the real shape
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()   # ⛔ BEFORE anything prints: a ✅ on a cp1252 console raises, not degrades

import re
import ssl
import sys
import urllib.error

import nethttp

CONTROL = "--control" in sys.argv
fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)



def code_only(path):
    """The module's CODE, with comments and string literals removed.

    ⛔⛔ WITHOUT THIS, THE TESTS BELOW ASSERT ON PROSE. The first version searched the whole file for
    `_create_unverified_context` and failed -- on nethttp.py's own docstring, the paragraph
    explaining why that call is forbidden. A ban that trips on the sentence describing the ban is
    worse than no ban: it is a red light nobody can turn green except by deleting the explanation.
    """
    import io
    import tokenize
    out = []
    with open(path, "rb") as fh:
        for tok in tokenize.tokenize(fh.readline):
            if tok.type in (tokenize.COMMENT, tokenize.STRING):
                continue
            out.append(tok.string)
    return "\n".join(out)


def body_of(path, func):
    """Just one function's source, so 'A happens before B' means inside it, not file-wide."""
    import re as _re
    s = open(path).read()
    m = _re.search(rf"^def {func}\(.*?(?=\n(?:def |class )|\Z)", s, _re.S | _re.M)
    return m.group(0) if m else ""


is_cert_error = nethttp._is_cert_error
if CONTROL:
    # The naive version: the one that looks obviously correct and misses every real occurrence.
    is_cert_error = lambda e: isinstance(e, ssl.SSLCertVerificationError)  # noqa: E731

print("── 1. the shape urllib actually raises ──")
# ⛔ THIS IS THE WHOLE POINT. urlopen wraps the SSLError in a URLError, so the exception that
# reaches the caller is NOT an SSLCertVerificationError instance.
inner = ssl.SSLCertVerificationError(
    1, "[SSL: CERTIFICATE_VERIFY_FAILED] certificate verify failed: "
       "unable to get local issuer certificate (_ssl.c:1028)")
wrapped = urllib.error.URLError(inner)
check(not isinstance(wrapped, ssl.SSLCertVerificationError),
      "the wrapped error is NOT an SSLCertVerificationError instance — isinstance alone is a trap")
check(is_cert_error(wrapped), "it is recognised as a certificate failure anyway")
check(is_cert_error(inner), "and so is the bare SSLCertVerificationError")

print("── 2. a cert failure is distinguished from a real network failure ──")
# ⚠ Retrying a DNS failure through certifi and curl wastes a person's time and then blames
# certificates for something that was never about certificates.
for e, want, label in (
    (urllib.error.URLError(OSError("[Errno -2] Name or service not known")), False, "DNS failure"),
    (urllib.error.URLError(TimeoutError("timed out")), False, "timeout"),
    (ConnectionResetError("reset by peer"), False, "connection reset"),
):
    check(is_cert_error(e) is want, f"{label} is not treated as a certificate problem")

print("── 3. the fallback chain is in the right order and complete ──")
src = open(nethttp.__file__).read()
order = [m.group(1) for m in re.finditer(r"^\s*#\s*(\d) —", src, re.M)]
check(order == ["1", "2", "3", "4", "5"], f"five attempts, numbered in order (found {order})")
check(src.index("_certifi_context()") < src.index("_curl(url"),
      "a CA bundle is tried BEFORE shelling out to curl — cheapest first, and curl FAILED on the "
      "measured machine so it is the weakest link, not the strongest")
# ⭐ The insight that makes this work with nothing installed: pip vendors its own CA bundle, which
# is why `pip install cryptography` succeeded on the box where urllib could not verify anything.
check("pip._vendor" in src,
      "pip's vendored CA bundle is used, so a machine with no certifi is still fixed")
where_body = body_of(nethttp.__file__, "certifi_where")
check(where_body.index("import certifi") < where_body.index("pip._vendor"),
      "a real certifi installation is preferred over reaching into pip's internals")
check("-fsSL" in src, "curl uses -f, so an HTTP error is a failure and not a saved error page")

print("── 4. ⛔⛔ VERIFICATION IS NEVER DISABLED ──")
banned = [
    ("_create_unverified_context", "disables verification outright"),
    ("CERT_NONE", "accepts any certificate"),
    ("check_hostname = False", "accepts a certificate for the wrong host"),
    ("verify=False", "the requests-style spelling of the same thing"),
    ('"-k"', "curl's --insecure, short form, as an argv element"),
    ("--insecure", "curl's --insecure"),
]
# ⛔ CODE ONLY. See code_only() -- this previously matched nethttp.py's own explanation of the ban.
code = code_only(nethttp.__file__)
found = [(t, why) for t, why in banned if t in code]
check(not found,
      "no path turns verification off" + (f" — FOUND: {found}" if found else ""))
check("verifying" in src.lower() or "verify" in src.lower(),
      "and the module says so in writing, so the next person does not 'simplify' it")

print("── 5. an HTTP error is reported, not retried as a cert problem ──")
# A 404 is a definite answer from a server whose certificate verified. Retrying it through two more
# trust stores produces the same 404 three times and then blames certificates.
get_body = body_of(nethttp.__file__, "get")
check("HTTPError" in get_body and "answered HTTP" in get_body,
      "HTTPError raises NetError naming the status, before any fallback")
# ⚠ Compared INSIDE get(), not across the file: _certifi_context is DEFINED above get() and used
# below the HTTPError branch, so a file-wide index compares a definition with a call site.
check(get_body.index("answered HTTP") < get_body.index("_certifi_context()"),
      "and it does so BEFORE the certifi attempt")

print("── 6. the message tells a person what to do ──")
check("pip install --upgrade certifi" in src, "names the one command that fixes it on Windows")
check("could not verify the HTTPS certificate" in src, "says what actually went wrong")

print("── 7. diagnose() reports every path without raising ──")
try:
    rows = nethttp.diagnose(timeout=2)
    names = [r[0] for r in rows]
    check(len(rows) == 3, f"three rows (got {names})")
    check(all(isinstance(r[1], bool) for r in rows), "each row has a boolean verdict")
    check("certifi bundle" in names and "curl" in names, f"names the fallbacks: {names}")
except Exception as e:  # noqa: BLE001
    check(False, f"diagnose() raised {type(e).__name__}: {e} — a diagnostic must never crash")

print("── 8. ⛔⛔ the SHIPPED intermediates are verified, not trusted on sight ──")
# A certificate committed to a repository is a thing a reader will not read. If this file were ever
# swapped for someone else's certificate, every HTTPS check in the program would start accepting it.
# So its SIGNATURE is re-checked here against an anchor taken from certifi -- not from the network
# and not from the file itself -- which makes tampering a failing test rather than a silent widening.
pem = nethttp._extra_chain_pem()
check(pem is not None, "extra-chain.pem is present and contains certificates")
if pem:
    try:
        import re as _re
        from cryptography import x509
        from cryptography.hazmat.primitives import hashes
        from cryptography.hazmat.primitives.asymmetric import ec, padding, rsa
        import certifi as _certifi

        anchors = {}
        for b in _re.findall(r"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----",
                             open(_certifi.where()).read(), _re.S):
            try:
                c = x509.load_pem_x509_certificate(b.encode())
            except Exception:       # noqa: BLE001 - a bundle may carry one we cannot parse
                continue
            anchors[c.subject.rfc4514_string()] = c

        shipped = [x509.load_pem_x509_certificate(b.encode())
                   for b in _re.findall(r"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----",
                                        pem, _re.S)]
        check(bool(shipped), f"{len(shipped)} shipped certificate(s)")
        for c in shipped:
            name = c.subject.rfc4514_string()
            issuer = c.issuer.rfc4514_string()
            parent = anchors.get(issuer)
            check(parent is not None,
                  f"{name} chains to an anchor certifi ALREADY has ({issuer})")
            if parent is None:
                continue
            pub = parent.public_key()
            try:
                if isinstance(pub, rsa.RSAPublicKey):
                    pub.verify(c.signature, c.tbs_certificate_bytes,
                               padding.PKCS1v15(), c.signature_hash_algorithm)
                else:
                    pub.verify(c.signature, c.tbs_certificate_bytes,
                               ec.ECDSA(c.signature_hash_algorithm))
                check(True, f"{name}: signature verifies against {issuer}")
            except Exception as e:      # noqa: BLE001
                check(False, f"{name}: SIGNATURE DOES NOT VERIFY ({type(e).__name__}) — "
                             f"this file has been tampered with or replaced")
            # ⚠ An expired intermediate completes no chain, and would fail at the worst moment.
            #
            # ⛔⛔ AND THE ACCESSOR DIFFERS BY LIBRARY VERSION, WHICH IS HOW THIS FIRST FAILED IN CI
            # WHILE PASSING LOCALLY. `not_valid_before_utc` arrived in cryptography 42; before that
            # it is `not_valid_before`. They are not just different names: the old pair is NAIVE
            # UTC and the new pair is TIMEZONE-AWARE, so picking the wrong `now` raises TypeError
            # rather than comparing wrongly. A test that pins a library version it never declared
            # is a test that only runs on the machine that wrote it.
            import datetime
            nb = getattr(c, "not_valid_before_utc", None)
            na = getattr(c, "not_valid_after_utc", None)
            if nb is None or na is None:
                nb, na = c.not_valid_before, c.not_valid_after            # naive UTC, cryptography < 42
                now = datetime.datetime.utcnow()
            else:
                now = datetime.datetime.now(datetime.timezone.utc)        # aware, cryptography >= 42
            check(nb <= now <= na, f"{name} is in date (until {na:%Y-%m-%d})")
            # the fingerprint the header claims must be the fingerprint of what is actually here
            fp = c.fingerprint(hashes.SHA256()).hex()
            check(fp in open(nethttp.EXTRA_CHAIN, encoding="utf-8").read(),
                  f"{name}: the sha256 in the header matches the certificate ({fp[:16]}…)")
    except ImportError:
        check(False, "cryptography is needed to verify the shipped certificates")


print("── 9. ⛔⛔ the shipped file is TRACKED BY GIT, not just present on this disk ──")
# ⛔⛔ THIS EXACT BUG. extra-chain.pem matched `*.pem` in .gitignore -- a rule that is there for
# KEYS. `git add -A` skips an ignored path WITHOUT A WORD, so the commit looked complete, every
# test below passed against the copy sitting on the author's disk, the branch was pushed, and the
# person it was written for pulled 2 files and still could not download anything.
#
# ⚠ "the tests pass" and "the fix shipped" are different claims, and nothing here distinguished
# them. Every other assertion in this file reads the working tree, so every one of them was blind
# to a file that exists locally and nowhere else.
import shutil as _shutil
import subprocess as _sp

if _shutil.which("git") is None:
    print("       (git not available — skipped, and this is the one check that cannot be faked)")
else:
    inrepo = _sp.run(["git", "rev-parse", "--is-inside-work-tree"],
                     capture_output=True, text=True, cwd=_os.path.dirname(nethttp.EXTRA_CHAIN))
    if inrepo.returncode != 0 or inrepo.stdout.strip() != "true":
        print("       (not inside a git work tree — skipped)")
    else:
        r = _sp.run(["git", "ls-files", "--error-unmatch", _os.path.basename(nethttp.EXTRA_CHAIN)],
                    capture_output=True, text=True, cwd=_os.path.dirname(nethttp.EXTRA_CHAIN))
        check(r.returncode == 0,
              "extra-chain.pem is tracked by git, so it reaches the people who clone this")
        ig = _sp.run(["git", "check-ignore", _os.path.basename(nethttp.EXTRA_CHAIN)],
                     capture_output=True, text=True, cwd=_os.path.dirname(nethttp.EXTRA_CHAIN))
        # ⚠ check-ignore exits 0 when a pattern matches INCLUDING a negation, so its exit code
        # alone says nothing. Tracked-ness above is the claim that matters; this only reports.
        print(f"       (git check-ignore says: {ig.stdout.strip() or 'no pattern matched'})")


EXPECTED_CONTROL_FAILURES = {
    "it is recognised as a certificate failure anyway",
}

print()
if CONTROL:
    got = set(fails)
    if got == EXPECTED_CONTROL_FAILURES:
        print("CONTROL OK — the naive `isinstance` check missed the wrapped error, which is the "
              "only shape that actually occurs:")
        for f in sorted(got):
            print(f"  - {f}")
        sys.exit(0)
    print("CONTROL FAILED — the naive check did not produce the expected failure.")
    for f in sorted(EXPECTED_CONTROL_FAILURES - got):
        print(f"  should have failed and did not: {f}")
    for f in sorted(got - EXPECTED_CONTROL_FAILURES):
        print(f"  failed unexpectedly: {f}")
    sys.exit(1)

if fails:
    print(f"FAILED {len(fails)}: " + "; ".join(fails))
    sys.exit(1)
print("all good")
