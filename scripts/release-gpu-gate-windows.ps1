# The release's Windows CUDA check for the ENGINE (OpenVM) release (docs/BUILD_AND_RELEASE.md, "The real-GPU gate";
# engine/RELEASE_TOOLING.md). Run it on a Windows PC with an NVIDIA card, in the folder where the STAGED
# hazync-engine-x86_64-windows-cuda.zip was unzipped (hzengine.exe and its CUDA runtime):
#
#   powershell -ExecutionPolicy Bypass -File release-gpu-gate-windows.ps1 -Export export_170.tgz `
#       -Keys ..\hazync-keys-core.tar.gz -Zip ..\hazync-engine-x86_64-windows-cuda.zip
#
# It hashes the hzengine.exe it is about to run, the zip it came from (-Zip is required), the block export
# (engine/gpu-gate's b170 entry) and the key tarball, asks nvidia-smi which card this is, unpacks both with Windows'
# own tar.exe, proves block 170 (`hzengine prove-block`), packs the proof into the HZE1 file a reader verifies
# (`hzengine pack`, which prints the image id), and writes ONE JSON report (gate-windows.json) with every sha256 and
# the timing. Bring the report, out\block_170.hze, prove.log and the zip back to the laptop for
# `scripts/release-gpu-gate.sh record-windows`, which checks hzengine.exe's sha256 against the STAGED zip, verifies the
# proof under the published record and refuses a proof that was ever used as evidence before.
#
# ⛔ Native programs are run with Start-Process and file redirection, NEVER `& exe ... *> file`: under Windows
# PowerShell 5.1 (what `powershell -File` runs) with $ErrorActionPreference = "Stop", a native program's stderr
# behind a redirect becomes a terminating NativeCommandError -- the script dies at the first stderr line, prove.log
# is empty and $LASTEXITCODE is -1 (reproduced on 5.1.26100; scripts/test-release-gpu-gate-windows.sh).
#
# The segment memory: hzengine sizes segments itself unless HZ_SEG_MEM_GB is set (engine/ENGINE.md). -SegMemGb N sets
# it, or an HZ_SEG_MEM_GB already in the environment is kept; the report records the value and where it came from.
# Small cards (4 GB, the GTX 1050 Ti) are ROADMAP 2.8's experimental option: not measured on the engine yet.
param(
    [string]$EngineExe = ".\hzengine.exe",
    [Parameter(Mandatory = $true)][string]$Export,
    [Parameter(Mandatory = $true)][string]$Keys,
    [Parameter(Mandatory = $true)][string]$Zip,
    [string]$Out = "out",
    [int]$SegMemGb = 0,             # 0 = hzengine's own choice (or HZ_SEG_MEM_GB from the environment)
    [int]$ToolTimeoutS = 120,       # nvidia-smi, tar, --help, pack
    [int]$ProveTimeoutS = 2400      # the prove: 40 minutes
)
$ErrorActionPreference = "Stop"
function Sha([string]$p) { (Get-FileHash -Algorithm SHA256 -LiteralPath $p).Hash.ToLower() }
if ($Out -match "\s") { throw "-Out must not contain whitespace (it is passed to hzengine.exe as one argument)" }
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

$enginePath = (Resolve-Path -LiteralPath $EngineExe).Path
$exportPath = (Resolve-Path -LiteralPath $Export).Path
$keysPath = (Resolve-Path -LiteralPath $Keys).Path
$report = [ordered]@{
    schema          = 1
    kind            = "windows-cuda-run"
    engine_exe      = $enginePath
    host_exe_sha256 = Sha $enginePath           # the prover's sha256 (record-windows reads this field name)
    zip_sha256      = Sha (Resolve-Path -LiteralPath $Zip).Path
    export_sha256   = Sha $exportPath
    keys_sha256     = Sha $keysPath
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
$report.help_exit = Run $enginePath @("--help") "help.log" $ToolTimeoutS

# the work folder: the key tarball and the export, unpacked fresh by Windows' own tar.exe
$work = "gate-work"
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $Out, $work
New-Item -ItemType Directory -Force -Path $Out, "$work\keys", "$work\export" | Out-Null
$report.untar_keys_exit = Run "tar" @("-xzf", $keysPath, "-C", "$work\keys") "untar-keys.log" $ToolTimeoutS
$report.untar_export_exit = Run "tar" @("-xzf", $exportPath, "-C", "$work\export") "untar-export.log" $ToolTimeoutS

# The segment memory: -SegMemGb, else an HZ_SEG_MEM_GB the caller set, else hzengine's own choice.
if ($SegMemGb -ne 0) {
    if ($SegMemGb -lt 1 -or $SegMemGb -gt 64) { throw "-SegMemGb must be 1..64 (got $SegMemGb)" }
    $env:HZ_SEG_MEM_GB = "$SegMemGb"; $report.seg_mem_source = "-SegMemGb"
} elseif ($env:HZ_SEG_MEM_GB) {
    $report.seg_mem_source = "environment"
} else {
    $report.seg_mem_source = "hzengine default"
}
$report.seg_mem_gb = $(if ($env:HZ_SEG_MEM_GB) { $env:HZ_SEG_MEM_GB } else { "default" })
# A prove writes segment files as it goes; 1.4 GB free was not enough on the v0.27.1 gate's laptop (os error 112).
$drive = (Get-Item -LiteralPath $PWD.Path).PSDrive
if ($drive.Free -ne $null) {
    $report.disk_free_gb = [math]::Round($drive.Free / 1GB, 1)
    if ($drive.Free -lt 3GB) { Write-Warning ("only {0} GB free on {1}: the prove may stop with 'not enough space on the disk'" -f $report.disk_free_gb, $drive.Name) }
}
$t0 = Get-Date
$report.exit_code = -3          # not run: the unpacking failed
if ($report.untar_keys_exit -eq 0 -and $report.untar_export_exit -eq 0) {
    $report.exit_code = Run $enginePath @("prove-block", "--dir", "$work\keys", "--export", "$work\export", "--out", "$work\p") "prove.log" $ProveTimeoutS
} else {
    "not proved: unpacking failed (keys $($report.untar_keys_exit), export $($report.untar_export_exit))" | Set-Content -Encoding ascii -LiteralPath "prove.log"
}
$report.prove_s = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
$hze = Join-Path $Out "block_170.hze"
$report.pack_exit = $null; $report.image = ""
if ($report.exit_code -eq 0) {
    $report.pack_exit = Run $enginePath @("pack", "--dir", "$work\keys", "--proof", "$work\p\block", "--out", $hze) "pack.log" $ToolTimeoutS
    $report.image = ((Get-Content -LiteralPath pack.log) -join "`n").Trim()     # pack prints the image id and the pin
}
$report.finished = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$report.log_sha256 = Sha (Resolve-Path -LiteralPath "prove.log").Path
$report.receipt_sha256 = $(if (Test-Path -LiteralPath $hze) { Sha (Resolve-Path -LiteralPath $hze).Path } else { $null })
$report | ConvertTo-Json | Set-Content -Encoding ascii gate-windows.json
Write-Output ("gate-windows.json written: exit {0}, {1} s on '{2}' at HZ_SEG_MEM_GB={3} ({4}); hzengine.exe {5}" -f `
    $report.exit_code, $report.prove_s, $report.gpu_name, $report.seg_mem_gb, $report.seg_mem_source, $report.host_exe_sha256.Substring(0, 16))
if ($report.exit_code -ne 0 -or $report.pack_exit -ne 0 -or -not $report.receipt_sha256) { exit 1 }
