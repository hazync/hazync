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
#
# The segment size (HAZYNC_SEG_PO2) follows the card's memory, from the app's own table
# (tools/win-gui/supervisor.py _SEG_PO2_FOR_VRAM): the host's default, 21, peaks near 22 GB and ran out of memory on a
# 4 GB GTX 1050 Ti in the v0.27.1 gate (21.11.2026); 18 proved the same block there. -SegPo2 N, or HAZYNC_SEG_PO2
# already set in the environment, overrides the table. The report records the value and where it came from.
param(
    [string]$HostExe = ".\host.exe",
    [Parameter(Mandatory = $true)][string]$Leaf,
    [Parameter(Mandatory = $true)][string]$Zip,
    [string]$Out = "out",
    [switch]$Pascal,
    [int]$SegPo2 = 0,               # 0 = from the card's memory (the app's table); else 17..21
    [int]$ToolTimeoutS = 60,        # nvidia-smi and method-id
    [int]$ProveTimeoutS = 2400      # the prove: 40 minutes (a GTX 1050 Ti took 197.7 s for block 1)
)
$ErrorActionPreference = "Stop"
function Sha([string]$p) { (Get-FileHash -Algorithm SHA256 -LiteralPath $p).Hash.ToLower() }
if ($Out -match "\s") { throw "-Out must not contain whitespace (it is passed to host.exe as one argument)" }
function Run([string]$exe, [string[]]$argv, [string]$log, [int]$timeoutS) {
    # stdout and stderr to files, then one log; returns the exit code (-2 when killed at the timeout). Never throws
    # on the program's stderr. Any argument with a space is quoted (Start-Process joins them with spaces).
    $o = "$log.out"; $e = "$log.err"
    Remove-Item -Force -ErrorAction SilentlyContinue $log, $o, $e     # never report a stale log from an earlier run
    $q = @($argv | ForEach-Object { if ($_ -match "\s") { '"' + $_ + '"' } else { $_ } })
    $p = Start-Process -FilePath $exe -ArgumentList $q -NoNewWindow -PassThru `
        -RedirectStandardOutput $o -RedirectStandardError $e
    $null = $p.Handle          # without this, ExitCode can read $null after the wait
    if (-not $p.WaitForExit($timeoutS * 1000)) {
        try { $p.Kill() } catch { }
        $p.WaitForExit()
        "KILLED: no exit within $timeoutS s" | Set-Content -Encoding ascii -LiteralPath "$log.timeout"
    }
    $text = @()
    if (Test-Path -LiteralPath $o) { $text += Get-Content -LiteralPath $o }
    if (Test-Path -LiteralPath $e) { $text += Get-Content -LiteralPath $e }
    $text | Set-Content -Encoding ascii -LiteralPath $log
    if (Test-Path -LiteralPath "$log.timeout") {
        Get-Content -LiteralPath "$log.timeout" | Add-Content -Encoding ascii -LiteralPath $log
        Remove-Item -Force "$log.timeout"
        Remove-Item -Force -ErrorAction SilentlyContinue $o, $e
        return -2
    }
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
$report.gpu_name = ""; $report.compute_cap = ""; $report.driver = ""; $report.vram_mib = $null
try {
    $null = Run "nvidia-smi" @("--query-gpu=name,compute_cap,driver_version,memory.total", "--format=csv,noheader,nounits") "nvidia-smi.log" $ToolTimeoutS
    $line = "$(Get-Content -LiteralPath nvidia-smi.log -TotalCount 1)"
    if ($line -and $line -notmatch "(?i)error|failed|not found") {
        $parts = @($line.Split(",") | ForEach-Object { $_.Trim() })
        $report.gpu_name = $parts[0]
        if ($parts.Count -gt 1) { $report.compute_cap = $parts[1] }
        if ($parts.Count -gt 2) { $report.driver = $parts[2] }
        $mib = 0
        if ($parts.Count -gt 3 -and [int]::TryParse($parts[3], [ref]$mib)) { $report.vram_mib = $mib }
    }
} catch { }                     # no nvidia-smi: gpu_name stays empty, and record-windows refuses the report
$null = Run $hostPath @("method-id") "method-id.log" $ToolTimeoutS
$report.method_id = ((Get-Content -LiteralPath method-id.log) -join "`n").Trim()

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $Out
Remove-Item Env:RISC0_DEV_MODE -ErrorAction SilentlyContinue
if ($Pascal) { $env:HAZYNC_SPPARK_MIN_MAJOR = "6" }
# The segment size: -SegPo2, else an HAZYNC_SEG_PO2 the caller set, else the app's VRAM table, else the host default.
$table = @(@(40000, 21), @(20000, 20), @(10000, 19), @(4000, 18), @(0, 17))     # = supervisor._SEG_PO2_FOR_VRAM
if ($SegPo2 -ne 0) {
    if ($SegPo2 -lt 17 -or $SegPo2 -gt 21) { throw "-SegPo2 must be 17..21 (got $SegPo2)" }
    $env:HAZYNC_SEG_PO2 = "$SegPo2"; $report.seg_po2_source = "-SegPo2"
} elseif ($env:HAZYNC_SEG_PO2) {
    $report.seg_po2_source = "environment"
} elseif ($report.vram_mib) {
    foreach ($row in $table) { if ($report.vram_mib -ge $row[0]) { $env:HAZYNC_SEG_PO2 = "$($row[1])"; break } }
    $report.seg_po2_source = "vram $($report.vram_mib) MiB"
} else {
    $report.seg_po2_source = "host default (nvidia-smi gave no memory size)"
}
$report.seg_po2 = $(if ($env:HAZYNC_SEG_PO2) { $env:HAZYNC_SEG_PO2 } else { "default" })
# A prove writes segment files as it goes; 1.4 GB free was not enough on the v0.27.1 gate's laptop (os error 112).
$drive = (Get-Item -LiteralPath $PWD.Path).PSDrive
if ($drive.Free -ne $null) {
    $report.disk_free_gb = [math]::Round($drive.Free / 1GB, 1)
    if ($drive.Free -lt 3GB) { Write-Warning ("only {0} GB free on {1}: the prove may stop with 'not enough space on the disk'" -f $report.disk_free_gb, $drive.Name) }
}
$env:HAZYNC_LEAF_BUNDLE = $leafPath
$t0 = Get-Date
$report.exit_code = Run $hostPath @("libzync-prove-range", "170", "170", "unused", "leaf", $Out) "prove.log" $ProveTimeoutS
$report.prove_s = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
$report.finished = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$report.log_sha256 = Sha (Resolve-Path -LiteralPath "prove.log").Path
$rc = Join-Path $Out "block_170.rcpt"
$report.receipt_sha256 = $(if (Test-Path -LiteralPath $rc) { Sha (Resolve-Path -LiteralPath $rc).Path } else { $null })
$report | ConvertTo-Json | Set-Content -Encoding ascii gate-windows.json
Write-Output ("gate-windows.json written: exit {0}, {1} s on '{2}' at HAZYNC_SEG_PO2={3} ({4}); host.exe {5}" -f `
    $report.exit_code, $report.prove_s, $report.gpu_name, $report.seg_po2, $report.seg_po2_source, $report.host_exe_sha256.Substring(0, 16))
if ($report.exit_code -ne 0 -or -not $report.receipt_sha256) { exit 1 }
