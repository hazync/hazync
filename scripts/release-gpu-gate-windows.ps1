# The release's Windows CUDA check (docs/BUILD_AND_RELEASE.md, "The real-GPU gate"). Run it on a Windows PC with an
# NVIDIA card, in the folder where the STAGED hazync-host-x86_64-windows-cuda.zip was unzipped:
#
#   powershell -ExecutionPolicy Bypass -File release-gpu-gate-windows.ps1 -Leaf leaf_170.bin -Zip ..\hazync-host-x86_64-windows-cuda.zip
#
# It hashes the host.exe it is about to run and the zip it came from (which also carries cudart64_12.dll, so -Zip is
# required), asks nvidia-smi which card this is, runs `host.exe method-id`, proves block 170 from its real-state leaf
# bundle (prover/testdata/libzync/gpu-gate/leaf_170.bin), and writes ONE JSON report (gate-windows.json) with every
# sha256 and the timing. Bring the report, out\block_170.rcpt, prove.log and the zip back to the laptop for
# `scripts/release-gpu-gate.sh record-windows`, which checks host.exe's sha256 against the STAGED zip and refuses a
# receipt that was ever used as evidence before.
#
# ⛔ Native programs are run with Start-Process and file redirection, NEVER `& exe ... *> file`: under Windows
# PowerShell 5.1 (what `powershell -File` runs) with $ErrorActionPreference = "Stop", a native program's stderr
# behind a redirect becomes a terminating NativeCommandError -- the script dies at the first stderr line, prove.log
# is empty and $LASTEXITCODE is -1 (reproduced on 5.1.26100; scripts/test-release-gpu-gate-windows.sh).
#
# A Pascal card (GTX 10-series) needs -Pascal (HAZYNC_SPPARK_MIN_MAJOR=6, the app's opt-in floor, #727).
param(
    [string]$HostExe = ".\host.exe",
    [Parameter(Mandatory = $true)][string]$Leaf,
    [Parameter(Mandatory = $true)][string]$Zip,
    [string]$Out = "out",
    [switch]$Pascal
)
$ErrorActionPreference = "Stop"
function Sha([string]$p) { (Get-FileHash -Algorithm SHA256 -LiteralPath $p).Hash.ToLower() }
function Run([string]$exe, [string[]]$argv, [string]$log) {
    # stdout and stderr to files, then one log; returns the exit code. Never throws on the program's stderr.
    $o = "$log.out"; $e = "$log.err"
    $p = Start-Process -FilePath $exe -ArgumentList $argv -NoNewWindow -PassThru `
        -RedirectStandardOutput $o -RedirectStandardError $e
    $null = $p.Handle          # without this, ExitCode can read $null after the wait
    $p.WaitForExit()
    $text = @()
    if (Test-Path -LiteralPath $o) { $text += Get-Content -LiteralPath $o }
    if (Test-Path -LiteralPath $e) { $text += Get-Content -LiteralPath $e }
    $text | Set-Content -Encoding ascii -LiteralPath $log
    Remove-Item -Force -ErrorAction SilentlyContinue $o, $e
    return $p.ExitCode
}

$hostPath = (Resolve-Path -LiteralPath $HostExe).Path
$leafPath = (Resolve-Path -LiteralPath $Leaf).Path
$report = [ordered]@{
    schema          = 1
    kind            = "windows-cuda-run"
    host_exe        = $hostPath
    host_exe_sha256 = Sha $hostPath
    zip_sha256      = Sha (Resolve-Path -LiteralPath $Zip).Path
    leaf_sha256     = Sha $leafPath
    started         = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
}
$report.gpu_name = ""; $report.compute_cap = ""; $report.driver = ""
try {
    $null = Run "nvidia-smi" @("--query-gpu=name,compute_cap,driver_version", "--format=csv,noheader") "nvidia-smi.log"
    $line = "$(Get-Content -LiteralPath nvidia-smi.log -TotalCount 1)"
    if ($line -and $line -notmatch "(?i)error|failed|not found") {
        $parts = @($line.Split(",") | ForEach-Object { $_.Trim() })
        $report.gpu_name = $parts[0]
        if ($parts.Count -gt 1) { $report.compute_cap = $parts[1] }
        if ($parts.Count -gt 2) { $report.driver = $parts[2] }
    }
} catch { }                     # no nvidia-smi: gpu_name stays empty, and record-windows refuses the report
$null = Run $hostPath @("method-id") "method-id.log"
$report.method_id = ((Get-Content -LiteralPath method-id.log) -join "`n").Trim()

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $Out
Remove-Item Env:RISC0_DEV_MODE -ErrorAction SilentlyContinue
if ($Pascal) { $env:HAZYNC_SPPARK_MIN_MAJOR = "6" }
$env:HAZYNC_LEAF_BUNDLE = $leafPath
$t0 = Get-Date
$report.exit_code = Run $hostPath @("libzync-prove-range", "170", "170", "unused", "leaf", $Out) "prove.log"
$report.prove_s = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
$report.finished = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$report.log_sha256 = Sha (Resolve-Path -LiteralPath "prove.log").Path
$rc = Join-Path $Out "block_170.rcpt"
$report.receipt_sha256 = $(if (Test-Path -LiteralPath $rc) { Sha (Resolve-Path -LiteralPath $rc).Path } else { $null })
$report | ConvertTo-Json | Set-Content -Encoding ascii gate-windows.json
Write-Output ("gate-windows.json written: exit {0}, {1} s on '{2}'; host.exe {3}" -f `
    $report.exit_code, $report.prove_s, $report.gpu_name, $report.host_exe_sha256.Substring(0, 16))
if ($report.exit_code -ne 0 -or -not $report.receipt_sha256) { exit 1 }
