cd /root/worst-case
until grep -q CHAIN3_DONE run/guest4.out 2>/dev/null; do sleep 20; done
HZ_ONLY=none_small_k8 HZ_CHUNKS=2 python3 -u worst_next.py build/host_regtest run guest > run/guest5.out 2>&1
HZ_ONLY=unsplit_none HZ_CHUNKS=8 python3 -u worst_next.py build/host_regtest run guest > run/guest6.out 2>&1
HZ_ONLY=mem_k1140 HZ_CHUNKS="32 128" python3 -u worst_next.py build/host_regtest run guest > run/guest7.out 2>&1
echo CHAIN5_DONE >> run/guest7.out
