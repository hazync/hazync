# The release's Windows CUDA check (docs/BUILD_AND_RELEASE.md, "The real-GPU gate"). Run it on a Windows PC with an
# NVIDIA card, in the folder where the STAGED hazync-host-x86_64-windows-cuda.zip was unzipped:
#
#   powershell -ExecutionPolicy Bypass -File release-gpu-gate-windows.ps1 -Leaf leaf_170.bin -Zip ..\hazync-host-x86_64-windows-cuda.zip
#
# It hashes the host.exe it is about to run (and the zip, if given), asks nvidia-smi which card this is, runs
# `host.exe method-id`, proves block 170 from its real-state leaf bundle (prover/testdata/libzync/gpu-gate/
# leaf_170.bin), and writes ONE JSON report (gate-windows.json) with every sha256 and the timing. Bring the report,
# out\block_170.rcpt, prove.log and the zip back to the laptop for `scripts/release-gpu-gate.sh record-windows`,
# which checks host.exe's sha256 against the STAGED zip and refuses a receipt that was ever used as evidence before.
#
# A Pascal card (GTX 10-series) needs -Pascal (HAZYNC_SPPARK_MIN_MAJOR=6, the app's opt-in floor, #727).
param(
    [string]$HostExe = ".\host.exe",
    [Parameter(Mandatory = $true)][string]$Leaf,
    [string]$Zip = "",
    [string]$Out = "out",
    [switch]$Pascal
)
$ErrorActionPreference = "Stop"
function Sha([string]$p) { (Get-FileHash -Algorithm SHA256 -LiteralPath $p).Hash.ToLower() }

$hostPath = (Resolve-Path -LiteralPath $HostExe).Path
$leafPath = (Resolve-Path -LiteralPath $Leaf).Path
$report = [ordered]@{
    schema          = 1
    kind            = "windows-cuda-run"
    host_exe        = $hostPath
    host_exe_sha256 = Sha $hostPath
    zip_sha256      = $(if ($Zip) { Sha (Resolve-Path -LiteralPath $Zip).Path } else { $null })
    leaf_sha256     = Sha $leafPath
    started         = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
}
$smi = (& nvidia-smi --query-gpu=name,compute_cap,driver_version --format=csv,noheader 2>&1 | Select-Object -First 1)
$parts = "$smi".Split(",") | ForEach-Object { $_.Trim() }
$report.gpu_name = $parts[0]
$report.compute_cap = $(if ($parts.Count -gt 1) { $parts[1] } else { "" })
$report.driver = $(if ($parts.Count -gt 2) { $parts[2] } else { "" })
$report.method_id = (& $hostPath method-id 2>&1 | Out-String).Trim()

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $Out
Remove-Item Env:RISC0_DEV_MODE -ErrorAction SilentlyContinue
if ($Pascal) { $env:HAZYNC_SPPARK_MIN_MAJOR = "6" }
$env:HAZYNC_LEAF_BUNDLE = $leafPath
$t0 = Get-Date
& $hostPath libzync-prove-range 170 170 unused leaf $Out *> prove.log
$report.exit_code = $LASTEXITCODE
$report.prove_s = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
$report.finished = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$report.log_sha256 = Sha (Resolve-Path -LiteralPath "prove.log").Path
$rc = Join-Path $Out "block_170.rcpt"
$report.receipt_sha256 = $(if (Test-Path -LiteralPath $rc) { Sha (Resolve-Path -LiteralPath $rc).Path } else { $null })
$report | ConvertTo-Json | Set-Content -Encoding ascii gate-windows.json
Write-Output ("gate-windows.json written: exit {0}, {1} s on {2}; host.exe {3}" -f `
    $report.exit_code, $report.prove_s, $report.gpu_name, $report.host_exe_sha256.Substring(0, 16))
if ($report.exit_code -ne 0 -or -not $report.receipt_sha256) { exit 1 }
