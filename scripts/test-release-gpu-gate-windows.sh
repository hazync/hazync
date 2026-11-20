#!/usr/bin/env bash
# scripts/release-gpu-gate-windows.ps1 must survive a native program that writes to stderr (every real prove does)
# under Windows PowerShell 5.1, which is what `powershell -File` runs.
#
# The bug (round-2 review, reproduced on 5.1.26100): with $ErrorActionPreference = "Stop", `& host.exe ... *> log`
# turns the first stderr line into a terminating NativeCommandError: the script dies, prove.log is empty, and
# $LASTEXITCODE reads -1 -- on exactly the card the maintainer is trying to attest.
#
# WHERE A WINDOWS POWERSHELL EXISTS (WSL: powershell.exe) this RUNS the script: a fake host.exe compiled on the spot
# (Add-Type, C#) writes to stderr on every command; the run must finish, record exit 0, a receipt, and a log that
# holds the stderr lines; a fake prove that exits 3 must be recorded as 3. ELSEWHERE (Linux CI) it checks the
# script's SHAPE: every native call goes through Start-Process with file redirection, none through `& ... *>`, and
# -Zip is required. The shape check runs everywhere.
#
#   scripts/test-release-gpu-gate-windows.sh            # must PASS
#   scripts/test-release-gpu-gate-windows.sh --control  # the old `& host.exe ... *> prove.log` put back: MUST FAIL
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
CONTROL=0; [ "${1:-}" = "--control" ] && CONTROL=1
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
fails=0
check() { if [ "$1" = 1 ]; then echo "  ok   $2"; else echo "  FAIL $2"; fails=$((fails+1)); fi; }
S="$T/gate.ps1"; cp scripts/release-gpu-gate-windows.ps1 "$S"
if [ "$CONTROL" = 1 ]; then
    python3 - "$S" <<'PY'
import sys
p = sys.argv[1]; s = open(p).read()
o = '$report.exit_code = Run $hostPath @("libzync-prove-range", "170", "170", "unused", "leaf", $Out) "prove.log"'
assert o in s
s = s.replace(o, '& $hostPath libzync-prove-range 170 170 unused leaf $Out *> prove.log\n$report.exit_code = $LASTEXITCODE')
open(p, "w").write(s)
PY
    echo "CONTROL: the old '& host.exe ... *> prove.log' is back -- the checks below MUST fail"
fi

# ---- shape (everywhere) ----
code=$(grep -vE '^[[:space:]]*#' "$S")
check "$(printf '%s' "$code" | grep -qE '&[[:space:]]+\$hostPath|\*>' || echo 1)" \
      "no native call runs as '& exe' or with a '*>' redirect"
check "$(printf '%s' "$code" | grep -q 'Start-Process -FilePath \$exe' && echo 1)" "native calls go through Start-Process"
check "$(printf '%s' "$code" | grep -qE 'Run \$hostPath @\("libzync-prove-range"' && echo 1)" "the prove itself goes through Run"
check "$(printf '%s' "$code" | grep -qE '\[Parameter\(Mandatory = \$true\)\]\[string\]\$Zip' && echo 1)" "-Zip is required"

# ---- a real run under Windows PowerShell 5.1, where there is one ----
PS=$(command -v powershell.exe || true)
if [ -n "$PS" ] && command -v wslpath >/dev/null; then
    WT=$(cmd.exe /c 'echo %TEMP%' 2>/dev/null | tr -d '\r')
    W="$(wslpath -u "$WT")/hz-gate-ps-$$"
    mkdir -p "$W"; trap 'rm -rf "$T" "$W"' EXIT
    cp "$S" "$W/gate.ps1"
    cat > "$W/fake.cs" <<'CS'
using System; using System.IO;
public static class P { public static int Main(string[] a) {
  Console.Error.WriteLine("stderr line from " + (a.Length > 0 ? a[0] : "?"));
  if (a.Length > 0 && a[0] == "method-id") { Console.WriteLine("METHOD_ID 46666b6f"); return 0; }
  if (a.Length > 5 && a[0] == "libzync-prove-range") {
    Console.Error.WriteLine("progress: segment 1 of 4 (stderr)");
    if (Environment.GetEnvironmentVariable("FAKE_PROVE_RC") == "3") return 3;
    Directory.CreateDirectory(a[5]);
    File.WriteAllText(Path.Combine(a[5], "block_170.rcpt"), "RCPT " + Guid.NewGuid());
    Console.WriteLine("LZ_PROVE_RANGE_DONE 170..=170"); return 0; }
  return 2; } }
CS
    (cd "$W" && "$PS" -NoProfile -Command "Add-Type -TypeDefinition (Get-Content -Raw fake.cs) -OutputAssembly host.exe -OutputType ConsoleApplication" ) >/dev/null 2>&1
    printf 'leaf' > "$W/leaf_170.bin"; printf 'zip' > "$W/h.zip"
    if [ -f "$W/host.exe" ]; then
        (cd "$W" && "$PS" -NoProfile -ExecutionPolicy Bypass -File gate.ps1 -Leaf leaf_170.bin -Zip h.zip) > "$T/run.txt" 2>&1
        rc=$?
        j() { python3 -c "import json,sys; print(json.load(open(sys.argv[1], encoding='utf-8-sig')).get(sys.argv[2]))" "$W/gate-windows.json" "$1" 2>/dev/null; }
        check "$([ $rc = 0 ] && [ "$(j exit_code)" = 0 ] && echo 1)" "PS 5.1: a prove that writes to stderr finishes, exit 0 recorded (script rc $rc, exit_code $(j exit_code))"
        check "$(grep -q 'progress: segment 1 of 4' "$W/prove.log" 2>/dev/null && grep -q LZ_PROVE_RANGE_DONE "$W/prove.log" && echo 1)" \
              "  ...prove.log holds both its stdout and its stderr"
        check "$([ "$(j receipt_sha256)" = "$(sha256sum "$W/out/block_170.rcpt" 2>/dev/null | cut -c1-64)" ] && [ "$(j log_sha256)" = "$(sha256sum "$W/prove.log" | cut -c1-64)" ] && echo 1)" \
              "  ...the report names the receipt and the log by sha256"
        check "$([ "$(j host_exe_sha256)" = "$(sha256sum "$W/host.exe" | cut -c1-64)" ] && grep -q 46666b6f <<<"$(j method_id)" && echo 1)" \
              "  ...and host.exe's sha256 and its method-id"
        (cd "$W" && FAKE_PROVE_RC=3 WSLENV=FAKE_PROVE_RC "$PS" -NoProfile -ExecutionPolicy Bypass -File gate.ps1 -Leaf leaf_170.bin -Zip h.zip) > "$T/run3.txt" 2>&1
        rc=$?
        check "$([ $rc = 1 ] && [ "$(j exit_code)" = 3 ] && echo 1)" "PS 5.1: a prove that exits 3 is recorded as 3, and the script fails (rc $rc, exit_code $(j exit_code))"
    else
        echo "  --   could not compile a fake host.exe with Add-Type: the PowerShell run is not exercised"
    fi
else
    echo "  --   no Windows PowerShell here: only the script's shape is checked"
fi

if [ "$CONTROL" = 1 ]; then echo "CONTROL: $fails failure(s)"; else echo "$fails failure(s)"; fi
if [ "$CONTROL" = 1 ]; then [ "$fails" != 0 ]; else [ "$fails" = 0 ]; fi
