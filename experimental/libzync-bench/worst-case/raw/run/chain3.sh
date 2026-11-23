cd /root/worst-case
HZ_ONLY=mem_k1140,unsplit HZ_CHUNKS="2 8" python3 -u worst.py build/host_regtest run guest > run/guest3.out 2>&1
HZ_ONLY=all_k8,none_k24 HZ_CHUNKS=2 python3 -u worst.py build/host_regtest run guest > run/guest4.out 2>&1
echo CHAIN3_DONE >> run/guest4.out
