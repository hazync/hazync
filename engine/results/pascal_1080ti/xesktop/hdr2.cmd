@echo off
cd /d C:\hz\hdr
if exist out2 rmdir /s /q out2
mkdir out2
set "CUDA_VISIBLE_DEVICES=0"
set "HZ_SEG_MEM_GB=7"
echo START %DATE% %TIME% > C:\hz\hdr\run2.txt
powershell -NoProfile -Command "$t=Measure-Command { & C:\hz\cuda2\hzengine.exe prove --dir keys --input export\stream.bin --out out2\one *> out2\one.log }; 'PROVE_S ' + [math]::Round($t.TotalSeconds,1) + ' RC ' + $LASTEXITCODE | Out-File -Append -Encoding ascii C:\hz\hdr\run2.txt"
powershell -NoProfile -Command "$t=Measure-Command { & C:\hz\cuda2\hzengine.exe verify --dir keys --proof out2\one *> out2\verify.log }; 'VERIFY_S ' + [math]::Round($t.TotalSeconds,1) + ' RC ' + $LASTEXITCODE | Out-File -Append -Encoding ascii C:\hz\hdr\run2.txt"
echo DONE %DATE% %TIME% >> C:\hz\hdr\run2.txt
