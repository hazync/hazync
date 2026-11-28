@echo off
cd /d C:\hz\hdr
set "CUDA_VISIBLE_DEVICES=0"
set "VPMM_VA_SIZE=68719476736"
set "HZ_SEG_MEM_GB=7"
echo START %DATE% %TIME% > C:\hz\hdr\run.txt
powershell -NoProfile -Command "$t=Measure-Command { & C:\hz\cuda\hzengine.exe prove --dir keys --input export\stream.bin --out out\one *> out\one.log }; 'PROVE_S ' + [math]::Round($t.TotalSeconds,1) + ' RC ' + $LASTEXITCODE | Out-File -Append -Encoding ascii C:\hz\hdr\run.txt"
powershell -NoProfile -Command "$t=Measure-Command { & C:\hz\cuda\hzengine.exe verify --dir keys --proof out\one *> out\verify.log }; 'VERIFY_S ' + [math]::Round($t.TotalSeconds,1) + ' RC ' + $LASTEXITCODE | Out-File -Append -Encoding ascii C:\hz\hdr\run.txt"
echo DONE %DATE% %TIME% >> C:\hz\hdr\run.txt
