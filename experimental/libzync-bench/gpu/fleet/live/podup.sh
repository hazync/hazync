#!/bin/bash
# podup.sh <name>: wait for the pod's ports, record them in fleet.txt, push files, start prep, upload bundle.
cd ~/libzync-gpu/live; N=$1
for i in $(seq 1 40); do
  L=$(./pod.sh status | python3 -c '
import json,sys
for p in json.load(sys.stdin)["data"]["myself"]["pods"]:
  if p["name"]!=sys.argv[1]: continue
  r=p.get("runtime") or {}; ports={x["privatePort"]:(x["ip"],x["publicPort"]) for x in (r.get("ports") or []) if x["isIpPublic"]}
  priv=[x["ip"] for x in (r.get("ports") or []) if not x["isIpPublic"]]
  if 22 in ports and 9110 in ports: print(sys.argv[1], ports[22][0], ports[22][1], ports[9110][0], ports[9110][1], (priv or ["-"])[0])' "$N")
  [ -n "$L" ] && break; sleep 10; done
[ -n "$L" ] || { echo "$N NO PORTS"; exit 1; }
grep -v "^$N " fleet.txt 2>/dev/null > fleet.tmp; echo "$L" >> fleet.tmp; mv fleet.tmp fleet.txt
set -- $L; K="-i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=15"
for t in $(seq 1 12); do ssh $K -p $3 root@$2 true && break; sleep 10; done
scp -q $K -P $3 setup.sh prep.sh lzenv.sh lanew.sh blockA.sh eph eph.pub root@$2:/workspace/ || { echo "$N scp failed"; exit 1; }
ssh $K -p $3 root@$2 'mkdir -p /root/.ssh && install -m 600 /workspace/eph /root/.ssh/eph && cat /workspace/eph.pub >> /root/.ssh/authorized_keys && cd /workspace && (setsid nohup bash prep.sh > prep.out 2>&1 < /dev/null &)'
scp -q $K -P $3 ~/libzync-gpu/libzync-bundle.tgz ~/libzync-gpu/libzync-bundle.tgz.sha256 root@$2:/workspace/ && echo "$N up: $L, bundle sent"
