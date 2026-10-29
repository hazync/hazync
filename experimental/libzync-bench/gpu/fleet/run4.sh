#!/bin/bash
# run4.sh: block 962,000 on the pods in fleet.txt (first line = A = aggregate coordinator).
#   1. push lane/serve/work scripts
#   2. reachability gate: every other pod must reach A's 9110 (public mapping, else private ip)
#   3. chunk lanes: pod k proves chunks i with i % N == k, all launched together
#   4. gather receipts onto A (via the laptop; ~0.4 MB each)
#   5. aggregate SPREAD: seg-serve on A, a worker on every pod (A's own over loopback)
cd ~/libzync-gpu/run; R=$PWD/${RUN:-r4}; mkdir -p $R
K="-i $HOME/.ssh/ghost_signet_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=15"
say(){ echo "[$(date +%T)] $*" | tee -a $R/run.log; }
mapfile -t F < fleet.txt; N=${#F[@]}
ssh_(){ local ip=$1 p=$2; shift 2; ssh $K -p $p root@$ip "$@"; }
read -r _ AIP AP A9IP A9P APRIV <<<"${F[0]}"
say "fleet: $N pods, A=${F[0]}"

for l in "${F[@]}"; do read -r nm ip p _ <<<"$l"; scp -q $K -P $p lane.sh serve.sh work.sh root@$ip:/workspace/ || { say "push to $nm failed"; exit 1; }
  ssh_ $ip $p 'grep -q PREP_OK /workspace/prep.out' || { say "$nm is not PREP_OK"; exit 1; }; done

# ---- 2. reachability gate ----
scp -q $K -P $AP listen.py root@$AIP:/workspace/ && ssh_ $AIP $AP '(setsid nohup python3 /workspace/listen.py > /dev/null 2>&1 < /dev/null &)'; sleep 2
declare -A ADDR; ADDR[0]="127.0.0.1:9110"
for k in $(seq 1 $((N-1))); do read -r nm ip p _ <<<"${F[$k]}"
  if ssh_ $ip $p "timeout 4 bash -c '</dev/tcp/$A9IP/$A9P'" 2>/dev/null; then ADDR[$k]="$A9IP:$A9P"
  elif ssh_ $ip $p "timeout 4 bash -c '</dev/tcp/$APRIV/9110'" 2>/dev/null; then ADDR[$k]="$APRIV:9110"
  else ADDR[$k]=""; fi
  say "reach $nm -> A: ${ADDR[$k]:-UNREACHABLE}"
done
ssh_ $AIP $AP 'kill $(cat /workspace/listen.pid)' ; true

# ---- 3. chunk lanes ----
if [ "${RESUME:-0}" != 1 ]; then
say "=== chunks: 16 over $N pods ==="
for k in $(seq 0 $((N-1))); do read -r nm ip p _ <<<"${F[$k]}"
  L=$(seq $k $N 15 | tr '\n' ' ')
  ssh_ $ip $p "rm -rf /workspace/lane /workspace/agg; (setsid nohup bash /workspace/lane.sh '$L' > /dev/null 2>&1 < /dev/null &)"
  say "  $nm: chunks $L"
done
fi
while true; do d=0; for l in "${F[@]}"; do read -r nm ip p _ <<<"$l"; s=$(ssh_ $ip $p 'tail -1 /workspace/lane/lane.tsv' 2>/dev/null)
  case "$s" in LANE_DONE) d=$((d+1));; LANE_FAIL) say "$nm LANE_FAIL"; exit 1;; esac; done; [ $d -eq $N ] && break; sleep 5; done
for l in "${F[@]}"; do read -r nm ip p _ <<<"$l"; ssh_ $ip $p 'cat /workspace/lane/lane.tsv' | sed "s/^/$nm\t/" >> $R/lanes.tsv; done
python3 - $R/lanes.tsv <<'PY' | tee -a $R/run.log
import sys
st, en, per = [], [], []
for l in open(sys.argv[1]):
    f = l.rstrip("\n").split("\t")
    if f[1].startswith("START"): st.append(float(f[1].split()[1]))
    elif f[1].startswith("END"): en.append(float(f[1].split()[1]))
    elif len(f) == 5: per.append(float(f[4]) - float(f[3]))
print(f"chunks: wall {max(en)-min(st):.1f} s (first start -> last end), per chunk mean {sum(per)/len(per):.1f} s max {max(per):.1f} s")
PY

# ---- 4. gather receipts onto A ----
T0=$(date +%s.%N); mkdir -p $R/rc
for l in "${F[@]:1}"; do read -r nm ip p _ <<<"$l"; scp -q $K -P $p "root@$ip:/workspace/lane/receipts/*.hzk" $R/rc/; done
[ -n "$(ls $R/rc 2>/dev/null)" ] && scp -q $K -P $AP $R/rc/*.hzk root@$AIP:/workspace/lane/receipts/
n=$(ssh_ $AIP $AP 'ls /workspace/lane/receipts/*.hzk | wc -l'); say "receipts on A: $n (gather $(python3 -c "print(round($(date +%s.%N)-$T0,1))") s, via the laptop)"
[ "$n" = 16 ] || { say "need 16 receipts on A"; exit 1; }

# ---- 5. aggregate spread ----
say "=== aggregate spread ==="
ssh_ $AIP $AP '(setsid nohup bash /workspace/serve.sh > /dev/null 2>&1 < /dev/null &)'
for t in $(seq 1 60); do ssh_ $AIP $AP 'grep -q "listening on" /workspace/agg/serve.log' 2>/dev/null && break; sleep 1; done
for k in $(seq 0 $((N-1))); do [ -n "${ADDR[$k]}" ] || continue; read -r nm ip p _ <<<"${F[$k]}"
  ssh_ $ip $p "mkdir -p /workspace/agg; (setsid nohup bash /workspace/work.sh ${ADDR[$k]} $nm > /dev/null 2>&1 < /dev/null &)"; done
say "  workers started: $(for k in $(seq 0 $((N-1))); do [ -n "${ADDR[$k]}" ] && echo -n "$k "; done)"
while ! ssh_ $AIP $AP 'grep -q SERVE_DONE /workspace/agg/times' 2>/dev/null; do sleep 5; done
ssh_ $AIP $AP 'cat /workspace/agg/times; grep -E "REAL_EXIT|RECEIPT VERIFIED|worker at|^  (prologue|segment phase|    of which|assembly|TOTAL|serial floor)" /workspace/agg/serve.log' | tee -a $R/run.log
python3 -c "
t=dict(l.split() for l in open('/dev/stdin') if l.startswith(('START','END')))
print(f'aggregate spread wall: {float(t[\"END\"])-float(t[\"START\"]):.1f} s')" < <(ssh_ $AIP $AP 'cat /workspace/agg/times') | tee -a $R/run.log
for l in "${F[@]}"; do read -r nm ip p _ <<<"$l"; ssh_ $ip $p 'cd /workspace && tar czf h.tgz lane/*.log lane/lane.tsv agg prep.out build.log exec1.log 2>/dev/null'; scp -q $K -P $p root@$ip:/workspace/h.tgz $R/harvest_$nm.tgz; done
say "RUN4_DONE (harvested $(ls $R/harvest_*.tgz | wc -l) pods)"
