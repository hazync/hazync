"""Make stdout survive the Windows console, which is cp1252 by default.

⛔⛔ THIS IS NOT COSMETIC — IT IS A CRASH. Measured on real Windows Python 3.14.2:

    UnicodeEncodeError: 'charmap' codec can't encode character '\\u2705'
      File "...\\encodings\\cp1252.py", line 19, in encode

Every ✅ ⛔ ⚠ ⭐ in this project's output is outside cp1252, so a single one of them aborts the
program mid-sentence. The checks print them constantly, which means on Windows the diagnostics
would crash before reporting anything — the exact opposite of their purpose.

⚠ IT ALSO AFFECTS READING. `Path.read_text()` with no encoding uses the same cp1252, so a UTF-8
file full of these markers cannot be read either. That one already bit this project once, in CI.

⇒ Call `fix()` at the top of every entry point that prints. It is a no-op where the stream is
already UTF-8, and it never raises: a program must not fail because it could not fix its own
console.
"""
import sys


def fix():
    """Reconfigure stdout/stderr to UTF-8. Safe to call anywhere, including twice."""
    for stream in (sys.stdout, sys.stderr):
        try:
            # errors="replace" so an exotic character degrades to '?' rather than aborting a run
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError, OSError):
            # ⚠ A stream that cannot be reconfigured (a pipe on an old Python, a captured buffer)
            # is not a reason to stop. The markers may come out wrong; the program still runs.
            pass


if __name__ == "__main__":
    fix()
    print("✅ ⛔ ⚠ ⭐ — if you can read these four symbols, the console is fine")
