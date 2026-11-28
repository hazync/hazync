#!/usr/bin/env bash
# scripts/release-gpu-gate-windows.ps1 (the ENGINE release's Windows check, hzengine.exe) must survive a native program
# that writes to stderr (every real prove does) under Windows PowerShell 5.1, which is what `powershell -File` runs.
#
# The bug (round-2 review, reproduced on 5.1.26100): with $ErrorActionPreference = "Stop", `& host.exe ... *> log`
# turns the first stderr line into a terminating NativeCommandError: the script dies, prove.log is empty, and
# $LASTEXITCODE reads -1 -- on exactly the card the maintainer is trying to attest.
#
# WHERE A WINDOWS POWERSHELL EXISTS (WSL: powershell.exe) this RUNS the script: a fake hzengine.exe compiled on the spot
# (Add-Type, C#) writes to stderr on every command; the run must unpack the key tarball and the export with tar.exe,
# finish, record exit 0, an HZE1 file from `pack` and its printed image, and a log that holds the stderr lines; a
# fake prove that exits 3 must be recorded as 3; a hang is killed at -ProveTimeoutS; -SegMemGb / HZ_SEG_MEM_GB reach
# the prover. ELSEWHERE (Linux CI) it checks the script's SHAPE: every native call goes through Start-Process with
# file redirection, none through `& ... *>`, and -Zip, -Export and -Keys are required. The shape check runs everywhere.
#
#   scripts/test-release-gpu-gate-windows.sh            # must PASS
#   scripts/test-release-gpu-gate-windows.sh --control  # the old `& exe ... *> prove.log` put back: MUST FAIL
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
o = '$report.exit_code = Run $enginePath @("prove-block", "--dir", "$work\\keys", "--export", "$work\\export", "--out", "$work\\p") "prove.log" $ProveTimeoutS'
assert o in s, "control: the prove line moved"
s = s.replace(o, '& $enginePath prove-block --dir "$work\\keys" --export "$work\\export" --out "$work\\p" *> prove.log\n    $report.exit_code = $LASTEXITCODE')
open(p, "w").write(s)
PY
    echo "CONTROL: the old '& exe ... *> prove.log' is back -- the checks below MUST fail"
fi

PS=$(command -v powershell.exe || true)
# ---- shape (everywhere; in a control run WITH PowerShell it is skipped, so the failures below are the 5.1 run's own)
code=$(grep -vE '^[[:space:]]*#' "$S")
if [ "$CONTROL" = 0 ] || [ -z "$PS" ]; then
check "$(printf '%s' "$code" | grep -qE '&[[:space:]]+\$enginePath|\*>' || echo 1)" \
      "no native call runs as '& exe' or with a '*>' redirect"
check "$(printf '%s' "$code" | grep -q 'Start-Process -FilePath \$exe' && echo 1)" "native calls go through Start-Process"
check "$(printf '%s' "$code" | grep -qE 'Run \$enginePath @\("prove-block"' && echo 1)" "the prove itself goes through Run"
check "$(printf '%s' "$code" | grep -qE 'Run \$enginePath @\("pack"' && echo 1)" "the pack goes through Run"
for prm in Zip Export Keys; do
check "$(printf '%s' "$code" | grep -qE "\\[Parameter\\(Mandatory = \\\$true\\)\\]\\[string\\]\\\$$prm" && echo 1)" "-$prm is required"
done
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
public static class P {
  static string Arg(string[] a, string k) { for (int i = 0; i + 1 < a.Length; i++) if (a[i] == k) return a[i + 1]; return null; }
  public static int Main(string[] a) {
  Console.Error.WriteLine("stderr line from " + (a.Length > 0 ? a[0] : "?"));
  if (a.Length > 0 && a[0] == "--help") { Console.WriteLine("Usage: hzengine <COMMAND>"); return 0; }
  if (a.Length > 0 && a[0] == "prove-block") {
    Console.Error.WriteLine("progress: segment 1 of 4 (stderr)");
    Console.WriteLine("SEG_MEM_SEEN=" + (Environment.GetEnvironmentVariable("HZ_SEG_MEM_GB") ?? "unset"));
    if (!File.Exists(Path.Combine(Arg(a, "--dir"), "vk.bin")) || !File.Exists(Path.Combine(Arg(a, "--export"), "manifest.txt"))) { Console.WriteLine("NOT UNPACKED"); return 4; }
    if (Environment.GetEnvironmentVariable("FAKE_HANG") == "1") System.Threading.Thread.Sleep(120000);
    if (Environment.GetEnvironmentVariable("FAKE_PROVE_RC") == "3") return 3;
    Directory.CreateDirectory(Arg(a, "--out"));
    File.WriteAllText(Path.Combine(Arg(a, "--out"), "block"), "PROOF " + Guid.NewGuid());
    Console.WriteLine("BLOCK_PROVED 170"); return 0; }
  if (a.Length > 0 && a[0] == "pack") {
    File.WriteAllText(Arg(a, "--out"), "HZE1" + File.ReadAllText(Arg(a, "--proof")));
    Console.WriteLine("image 7777777777777777777777777777777777777777777777777777777777777777 pin 8888"); return 0; }
  return 2; } }
CS
    (cd "$W" && "$PS" -NoProfile -Command "Add-Type -TypeDefinition (Get-Content -Raw fake.cs) -OutputAssembly hzengine.exe -OutputType ConsoleApplication" ) >/dev/null 2>&1
    mkdir -p "$T/k" "$T/x"; echo vk > "$T/k/vk.bin"; echo m > "$T/x/manifest.txt"
    tar czf "$W/keys.tar.gz" -C "$T/k" . && tar czf "$W/export_170.tgz" -C "$T/x" .
    printf 'zip' > "$W/h.zip"
    G="-Export export_170.tgz -Keys keys.tar.gz -Zip h.zip"
    if [ -f "$W/hzengine.exe" ]; then
        echo STALE-LINE-FROM-AN-EARLIER-RUN > "$W/prove.log"
        # shellcheck disable=SC2086  # $G is the gate's argument list, split on purpose
        (cd "$W" && "$PS" -NoProfile -ExecutionPolicy Bypass -File gate.ps1 $G) > "$T/run.txt" 2>&1
        rc=$?
        j() { python3 -c "import json,sys; print(json.load(open(sys.argv[1], encoding='utf-8-sig')).get(sys.argv[2]))" "$W/gate-windows.json" "$1" 2>/dev/null; }
        check "$([ $rc = 0 ] && [ "$(j exit_code)" = 0 ] && [ "$(j pack_exit)" = 0 ] && echo 1)" "PS 5.1: a prove that writes to stderr finishes, exit 0 and pack 0 recorded (script rc $rc, exit_code $(j exit_code))"
        check "$(grep -q 'progress: segment 1 of 4' "$W/prove.log" 2>/dev/null && grep -q BLOCK_PROVED "$W/prove.log" && ! grep -q STALE-LINE "$W/prove.log" && echo 1)" \
              "  ...prove.log holds both its stdout and its stderr (after tar.exe unpacked keys and export), nothing from an earlier run"
        check "$([ "$(j receipt_sha256)" = "$(sha256sum "$W/out/block_170.hze" 2>/dev/null | cut -c1-64)" ] && head -c 4 "$W/out/block_170.hze" | grep -q HZE1 && [ "$(j log_sha256)" = "$(sha256sum "$W/prove.log" | cut -c1-64)" ] && echo 1)" \
              "  ...the report names the packed HZE1 file and the log by sha256"
        check "$([ "$(j host_exe_sha256)" = "$(sha256sum "$W/hzengine.exe" | cut -c1-64)" ] && grep -q 77777777 <<<"$(j image)" && [ "$(j keys_sha256)" = "$(sha256sum "$W/keys.tar.gz" | cut -c1-64)" ] && echo 1)" \
              "  ...and hzengine.exe's sha256, the image pack printed, the key tarball's sha256"
        # shellcheck disable=SC2086
        (cd "$W" && FAKE_PROVE_RC=3 WSLENV=FAKE_PROVE_RC "$PS" -NoProfile -ExecutionPolicy Bypass -File gate.ps1 $G) > "$T/run3.txt" 2>&1
        rc=$?
        check "$([ $rc = 1 ] && [ "$(j exit_code)" = 3 ] && [ "$(j receipt_sha256)" = None ] && echo 1)" "PS 5.1: a prove that exits 3 is recorded as 3, no proof file, and the script fails (rc $rc, exit_code $(j exit_code))"
        t0=$SECONDS
        # shellcheck disable=SC2086
        (cd "$W" && FAKE_HANG=1 WSLENV=FAKE_HANG "$PS" -NoProfile -ExecutionPolicy Bypass -File gate.ps1 $G -ProveTimeoutS 5) > "$T/runh.txt" 2>&1
        rc=$?
        check "$([ $rc = 1 ] && [ "$(j exit_code)" = -2 ] && grep -q 'KILLED: no exit within 5 s' "$W/prove.log" && [ $((SECONDS - t0)) -lt 60 ] && echo 1)" \
              "PS 5.1: a prove that hangs is killed at -ProveTimeoutS, recorded as -2 (rc $rc, exit_code $(j exit_code), $((SECONDS - t0)) s)"
        seg() {   # seg <HZ_SEG_MEM_GB or ""> [gate args...] -> prints "<report seg_mem_gb> <prover saw>"
            local e=$1; shift
            local pre
            [ -n "$e" ] && pre="\$env:HZ_SEG_MEM_GB = '$e';" || pre="Remove-Item Env:HZ_SEG_MEM_GB -ErrorAction SilentlyContinue;"
            (cd "$W" && "$PS" -NoProfile -Command "$pre powershell -NoProfile -ExecutionPolicy Bypass -File gate.ps1 $G $*") > "$T/seg.txt" 2>&1
            echo "$(j seg_mem_gb) $(grep -o 'SEG_MEM_SEEN=[^[:space:]]*' "$W/prove.log" | head -1 | cut -d= -f2)"
        }
        got=$(seg ""); check "$([ "$got" = "default unset" ] && [ "$(j seg_mem_source)" = "hzengine default" ] && echo 1)" "PS 5.1: no -SegMemGb: hzengine chooses (report/prover: $got)"
        got=$(seg "" -SegMemGb 12); check "$([ "$got" = "12 12" ] && [ "$(j seg_mem_source)" = "-SegMemGb" ] && echo 1)" "PS 5.1: -SegMemGb 12 reaches the prover (report/prover: $got)"
        got=$(seg 20); check "$([ "$got" = "20 20" ] && [ "$(j seg_mem_source)" = "environment" ] && echo 1)" "PS 5.1: an HZ_SEG_MEM_GB already set is kept (report/prover: $got)"
    else
        echo "  --   could not compile a fake hzengine.exe with Add-Type: the PowerShell run is not exercised"
    fi
else
    echo "  --   no Windows PowerShell here: only the script's shape is checked"
fi

if [ "$CONTROL" = 1 ]; then echo "CONTROL: $fails failure(s)"; else echo "$fails failure(s)"; fi
if [ "$CONTROL" = 1 ]; then [ "$fails" != 0 ]; else [ "$fails" = 0 ]; fi
