@echo off
cd /d C:\hz\blk
if not exist out mkdir out
echo START %DATE% %TIME% > C:\hz\blk\run_cpu741.txt
powershell -NoProfile -Command "$t=Measure-Command { & C:\hz\cpu\hzengine.exe prove-block --dir keys --export export\741000t --out out\cpu741 *> out\cpu741.log }; 'PROVE_S ' + [math]::Round($t.TotalSeconds,1) + ' RC ' + $LASTEXITCODE | Out-File -Append -Encoding ascii C:\hz\blk\run_cpu741.txt"
echo DONE %DATE% %TIME% >> C:\hz\blk\run_cpu741.txt
