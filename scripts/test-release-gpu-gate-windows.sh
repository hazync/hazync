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
#   scripts/test-release-gpu-gate-windows.sh --control-seg  # the VRAM table no longer sets HAZYNC_SEG_PO2: MUST FAIL
#
# The segment size (v0.27.1 gate, 21.11.2026: the host default 21 ran out of memory on a 4 GB GTX 1050 Ti): the
# script's VRAM table must equal the app's (tools/win-gui/supervisor.py _SEG_PO2_FOR_VRAM, checked everywhere), and under
# PS 5.1 a fake nvidia-smi.exe reporting 4,096 / 49,140 MiB must give the host 18 / 21, -SegPo2 and an
# HAZYNC_SEG_PO2 already set must win over the table.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
CONTROL=0; [ "${1:-}" = "--control" ] && CONTROL=1
CONTROL_SEG=0; [ "${1:-}" = "--control-seg" ] && { CONTROL=1; CONTROL_SEG=1; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
fails=0
check() { if [ "$1" = 1 ]; then echo "  ok   $2"; else echo "  FAIL $2"; fails=$((fails+1)); fi; }
S="$T/gate.ps1"; cp scripts/release-gpu-gate-windows.ps1 "$S"
if [ "$CONTROL_SEG" = 1 ]; then
    python3 - "$S" <<'PY'
import sys
p = sys.argv[1]; s = open(p).read()
o = '$env:HAZYNC_SEG_PO2 = "$($row[1])"; break'
assert o in s
s = s.replace(o, 'break')
open(p, "w").write(s)
PY
    echo "CONTROL-SEG: the VRAM table no longer sets HAZYNC_SEG_PO2 -- the segment-size checks below MUST fail"
elif [ "$CONTROL" = 1 ]; then
    python3 - "$S" <<'PY'
import sys
p = sys.argv[1]; s = open(p).read()
o = '$report.exit_code = Run $hostPath @("libzync-prove-range", "170", "170", "unused", "leaf", $Out) "prove.log" $ProveTimeoutS'
assert o in s
s = s.replace(o, '& $hostPath libzync-prove-range 170 170 unused leaf $Out *> prove.log\n$report.exit_code = $LASTEXITCODE')
open(p, "w").write(s)
PY
    echo "CONTROL: the old '& host.exe ... *> prove.log' is back -- the checks below MUST fail"
fi

PS=$(command -v powershell.exe || true)
# ---- the VRAM table equals the app's (everywhere)
tab=$(python3 - "$S" <<'PY'
import re, sys
ps = open(sys.argv[1]).read()
m = re.search(r'\$table = @\((.*?)\)\s+#', ps)
rows = [tuple(map(int, r)) for r in re.findall(r'@\((\d+),\s*(\d+)\)', m.group(1))] if m else []
app = open("tools/win-gui/supervisor.py").read()
a = re.search(r'^_SEG_PO2_FOR_VRAM = (\[.*\])$', app, re.M)
print(1 if a and rows and rows == [tuple(x) for x in eval(a.group(1))] else 0)
PY
)
check "$tab" "the script's VRAM -> HAZYNC_SEG_PO2 table equals tools/win-gui/supervisor.py _SEG_PO2_FOR_VRAM"
# ---- shape (everywhere; in a control run WITH PowerShell it is skipped, so the failures below are the 5.1 run's own)
code=$(grep -vE '^[[:space:]]*#' "$S")
if [ "$CONTROL" = 0 ] || [ "$CONTROL_SEG" = 1 ] || [ -z "$PS" ]; then
check "$(printf '%s' "$code" | grep -qE '&[[:space:]]+\$hostPath|\*>' || echo 1)" \
      "no native call runs as '& exe' or with a '*>' redirect"
check "$(printf '%s' "$code" | grep -q 'Start-Process -FilePath \$exe' && echo 1)" "native calls go through Start-Process"
check "$(printf '%s' "$code" | grep -qE 'Run \$hostPath @\("libzync-prove-range"' && echo 1)" "the prove itself goes through Run"
check "$(printf '%s' "$code" | grep -qE '\[Parameter\(Mandatory = \$true\)\]\[string\]\$Zip' && echo 1)" "-Zip is required"
check "$(printf '%s' "$code" | grep -q 'WaitForExit(\$timeoutS \* 1000)' && echo 1)" "every native call has a timeout"
fi

# ---- a real run under Windows PowerShell 5.1, where there is one ----
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
    Console.WriteLine("SEG_PO2_SEEN=" + (Environment.GetEnvironmentVariable("HAZYNC_SEG_PO2") ?? "unset"));
    if (Environment.GetEnvironmentVariable("FAKE_HANG") == "1") System.Threading.Thread.Sleep(120000);
    if (Environment.GetEnvironmentVariable("FAKE_PROVE_RC") == "3") return 3;
    Directory.CreateDirectory(a[5]);
    File.WriteAllText(Path.Combine(a[5], "block_170.rcpt"), "RCPT " + Guid.NewGuid());
    Console.WriteLine("LZ_PROVE_RANGE_DONE 170..=170"); return 0; }
  return 2; } }
CS
    (cd "$W" && "$PS" -NoProfile -Command "Add-Type -TypeDefinition (Get-Content -Raw fake.cs) -OutputAssembly host.exe -OutputType ConsoleApplication" ) >/dev/null 2>&1
    cat > "$W/smi.cs" <<'CS'
using System;
public static class P { public static int Main(string[] a) {
  Console.WriteLine("NVIDIA Fake Card, 6.1, 566.14, " + (Environment.GetEnvironmentVariable("FAKE_VRAM") ?? "4096")); return 0; } }
CS
    mkdir -p "$W/smi"
    (cd "$W" && "$PS" -NoProfile -Command "Add-Type -TypeDefinition (Get-Content -Raw smi.cs) -OutputAssembly smi\nvidia-smi.exe -OutputType ConsoleApplication" ) >/dev/null 2>&1
    printf 'leaf' > "$W/leaf_170.bin"; printf 'zip' > "$W/h.zip"
    if [ -f "$W/host.exe" ]; then
        echo STALE-LINE-FROM-AN-EARLIER-RUN > "$W/prove.log"
        (cd "$W" && "$PS" -NoProfile -ExecutionPolicy Bypass -File gate.ps1 -Leaf leaf_170.bin -Zip h.zip) > "$T/run.txt" 2>&1
        rc=$?
        j() { python3 -c "import json,sys; print(json.load(open(sys.argv[1], encoding='utf-8-sig')).get(sys.argv[2]))" "$W/gate-windows.json" "$1" 2>/dev/null; }
        check "$([ $rc = 0 ] && [ "$(j exit_code)" = 0 ] && echo 1)" "PS 5.1: a prove that writes to stderr finishes, exit 0 recorded (script rc $rc, exit_code $(j exit_code))"
        check "$(grep -q 'progress: segment 1 of 4' "$W/prove.log" 2>/dev/null && grep -q LZ_PROVE_RANGE_DONE "$W/prove.log" && ! grep -q STALE-LINE "$W/prove.log" && echo 1)" \
              "  ...prove.log holds both its stdout and its stderr, and nothing from an earlier run"
        check "$([ "$(j receipt_sha256)" = "$(sha256sum "$W/out/block_170.rcpt" 2>/dev/null | cut -c1-64)" ] && [ "$(j log_sha256)" = "$(sha256sum "$W/prove.log" | cut -c1-64)" ] && echo 1)" \
              "  ...the report names the receipt and the log by sha256"
        check "$([ "$(j host_exe_sha256)" = "$(sha256sum "$W/host.exe" | cut -c1-64)" ] && grep -q 46666b6f <<<"$(j method_id)" && echo 1)" \
              "  ...and host.exe's sha256 and its method-id"
        (cd "$W" && FAKE_PROVE_RC=3 WSLENV=FAKE_PROVE_RC "$PS" -NoProfile -ExecutionPolicy Bypass -File gate.ps1 -Leaf leaf_170.bin -Zip h.zip) > "$T/run3.txt" 2>&1
        rc=$?
        check "$([ $rc = 1 ] && [ "$(j exit_code)" = 3 ] && echo 1)" "PS 5.1: a prove that exits 3 is recorded as 3, and the script fails (rc $rc, exit_code $(j exit_code))"
        t0=$SECONDS
        (cd "$W" && FAKE_HANG=1 WSLENV=FAKE_HANG "$PS" -NoProfile -ExecutionPolicy Bypass -File gate.ps1 -Leaf leaf_170.bin -Zip h.zip -ProveTimeoutS 5) > "$T/runh.txt" 2>&1
        rc=$?
        check "$([ $rc = 1 ] && [ "$(j exit_code)" = -2 ] && grep -q 'KILLED: no exit within 5 s' "$W/prove.log" && [ $((SECONDS - t0)) -lt 60 ] && echo 1)" \
              "PS 5.1: a prove that hangs is killed at -ProveTimeoutS, recorded as -2 (rc $rc, exit_code $(j exit_code), $((SECONDS - t0)) s)"
        if [ -f "$W/smi/nvidia-smi.exe" ]; then
            # The fake nvidia-smi goes first on PATH; the gate itself still runs under `powershell -File` (5.1).
            seg() {   # seg <FAKE_VRAM> <HAZYNC_SEG_PO2 or ""> [gate args...] -> prints "<report seg_po2> <host saw>"
                local v=$1 e=$2; shift 2
                local pre="\$env:PATH = (Join-Path (Get-Location).Path 'smi') + ';' + \$env:PATH; \$env:FAKE_VRAM = '$v';"
                [ -n "$e" ] && pre="$pre \$env:HAZYNC_SEG_PO2 = '$e';" || pre="$pre Remove-Item Env:HAZYNC_SEG_PO2 -ErrorAction SilentlyContinue;"
                (cd "$W" && "$PS" -NoProfile -Command "$pre powershell -NoProfile -ExecutionPolicy Bypass -File gate.ps1 -Leaf leaf_170.bin -Zip h.zip $*") > "$T/seg.txt" 2>&1
                echo "$(j seg_po2) $(grep -o 'SEG_PO2_SEEN=[^[:space:]]*' "$W/prove.log" | head -1 | cut -d= -f2)"
            }
            got=$(seg 4096 ""); check "$([ "$got" = "18 18" ] && [ "$(j vram_mib)" = 4096 ] && echo 1)" "PS 5.1: a 4,096 MiB card proves at HAZYNC_SEG_PO2=18 (report/host: $got)"
            got=$(seg 49140 ""); check "$([ "$got" = "21 21" ] && echo 1)" "PS 5.1: a 49,140 MiB card proves at 21 (report/host: $got)"
            got=$(seg 4096 "" -SegPo2 19); check "$([ "$got" = "19 19" ] && [ "$(j seg_po2_source)" = "-SegPo2" ] && echo 1)" "PS 5.1: -SegPo2 19 wins over the table (report/host: $got)"
            got=$(seg 49140 20); check "$([ "$got" = "20 20" ] && [ "$(j seg_po2_source)" = "environment" ] && echo 1)" "PS 5.1: an HAZYNC_SEG_PO2 already set wins over the table (report/host: $got)"
        else
            echo "  --   could not compile a fake nvidia-smi.exe: the segment-size runs are not exercised"
        fi
    else
        echo "  --   could not compile a fake host.exe with Add-Type: the PowerShell run is not exercised"
    fi
else
    echo "  --   no Windows PowerShell here: only the script's shape is checked"
fi

if [ "$CONTROL" = 1 ]; then echo "CONTROL: $fails failure(s)"; else echo "$fails failure(s)"; fi
if [ "$CONTROL" = 1 ]; then [ "$fails" != 0 ]; else [ "$fails" = 0 ]; fi
