#!/bin/bash
# M1 on CPU: keys, guest, two leaves, a node verifying both, a node verifying that node (nested deferral), verify all.
source ~/.cargo/env; cd /root/engine; H=host/target/release/hzengine; K=/root/engine/keys; mkdir -p p
t(){ local s=$(date +%s); "$@"; local rc=$?; echo "STEP rc=$rc secs=$(( $(date +%s)-s )) :: $*"; return $rc; }
[ -f $K/sdk.pk ] || t $H keygen --dir $K || exit 1
rm -f $K/image.json
t $H build --dir $K --guest guest || exit 1
t $H selftest-leaf --dir $K --value 7 --out p/l7 || exit 1
t $H selftest-leaf --dir $K --value 35 --out p/l35 || exit 1
t $H selftest-node --dir $K --left p/l7 --right p/l35 --out p/n42 || exit 1
t $H selftest-node --dir $K --left p/n42 --right p/l7 --out p/n49 || exit 1
for x in l7 l35 n42 n49; do t $H verify --dir $K --proof p/$x; done
ls -la p; echo M1_DONE
