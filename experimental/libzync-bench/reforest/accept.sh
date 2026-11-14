#!/bin/bash
# libzync-reforest acceptance test (docs/libzync/UNDO.md §8), CPU: prep 1..2000 by the released v0.25.0 host and by the
# new host, the old run re-forested and compared byte for byte, a control, the refusals, an independent undo check and
# the phase-1 rate. Needs an unpruned node with getblock verbosity 3 (bitcoin-cli on PATH; fetch_block_rpc.py reads its
# cookie). usage: NEW=<new host_m> [W=<work dir>] accept.sh
set -u
W=${W:-/root/undocommit}; A=${A:-$W/accept}; S=${S:-$(cd "$(dirname "$0")/../../.." && pwd)}; mkdir -p $A; cd $A
OLD=${OLD:-$A/host_v025}; NEW=${NEW:?the new mainnet host (build-hosts.sh host_m)}
say() { echo "$*"; }
if [ ! -x $OLD ]; then
  curl -sfL -o $OLD.dl https://github.com/hazync/hazync/releases/download/v0.25.0/hazync-host-x86_64-linux-gnu
  curl -sfL -o SUMS https://github.com/hazync/hazync/releases/download/v0.25.0/SHA256SUMS.txt
  want=$(grep "hazync-host-x86_64-linux-gnu$" SUMS | cut -c1-64); have=$(sha256sum $OLD.dl | cut -c1-64)
  [ -n "$want" ] && [ "$want" = "$have" ] && mv $OLD.dl $OLD && chmod +x $OLD || { say "FATAL v0.25.0 host sha mismatch ($have vs $want)"; exit 1; }
fi
say "old: $($OLD method-id | head -1)"; say "new: $($NEW method-id | head -1)"
mkdir -p J
[ -f J/block_2000.json ] || python3 $S/prover/fetch_block_rpc.py range 1 2000 J --jobs 4 > fetch.log 2>&1
ls J | wc -l
export HAZYNC_PREP_LEAF_AT=171,1500,2000
rm -rf old new rf ctl oldl newl
t=$(date +%s); HAZYNC_PREP_LEAF_BUNDLES=$A/oldl $OLD libzync-prep 1 2000 J genesis old > prep_old.log 2>&1; say "PREP_OLD exit $? ($(( $(date +%s) - t )) s)"
t=$(date +%s); HAZYNC_PREP_LEAF_BUNDLES=$A/newl $NEW libzync-prep 1 2000 J genesis new > prep_new.log 2>&1; say "PREP_NEW exit $? ($(( $(date +%s) - t )) s)"
unset HAZYNC_PREP_LEAF_AT
d=0; for h in $(seq 1 2000); do cmp -s old/state_$h.json new/state_$h.json || d=$((d+1)); done; say "old vs new prep: $d of 2000 states differ (expected 2000: block_roots and the stamp)"
cp -a old rf; cp -a oldl rfl
t=$(date +%s); $NEW libzync-reforest rf 2000 --bitcoin-cli "bitcoin-cli" --all --jobs 4 --bundles $A/rfl > rf.log 2>&1; say "REFOREST exit $? ($(( $(date +%s) - t )) s)"; grep -a "REFOREST_FETCH\|LZ_REFOREST_DONE\|reforest:" rf.log | tail -4
d=0; for h in $(seq 1 2000); do cmp -s rf/state_$h.json new/state_$h.json || d=$((d+1)); done; say "ACCEPT re-forested old vs new prep: $((2000-d))/2000 state_<h>.json byte-identical"
for b in 171 1500 2000; do cmp -s rfl/leaf_$b.bin newl/leaf_$b.bin && say "ACCEPT bundle leaf_$b.bin identical to the new prep's" || say "WRONG bundle leaf_$b.bin differs"; done
for f in old new; do ls $f/*.coinlog $f/*.trie 2>/dev/null | wc -l; done
sha256sum old/state_2000.coinlog rf/state_2000.coinlog | cut -c1-16; sha256sum old/state_2000.trie rf/state_2000.trie 2>/dev/null | cut -c1-16
$NEW libzync-reforest rf 2000 --bitcoin-cli "bitcoin-cli" --all --bundles $A/rfl > rf2.log 2>&1; say "rerun exit $?: $(grep -a LZ_REFOREST_DONE rf2.log)"
# control: block 170's undo hash replaced by SHA-256 of nothing (as if its input were skipped)
cp -a old ctl; mkdir -p ctl/reforest
awk -F'\t' -v OFS='\t' 'NR==170{$3="e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"}1' rf/reforest/undo.tsv > ctl/reforest/undo.tsv
$NEW libzync-reforest ctl 2000 --bitcoin-cli "bitcoin-cli" --all > ctl.log 2>&1; say "control exit $?"
lo=0; hi=0; for h in $(seq 1 2000); do if cmp -s ctl/state_$h.json new/state_$h.json; then [ $h -lt 170 ] && lo=$((lo+1)); else [ $h -ge 170 ] && hi=$((hi+1)); fi; done
say "CONTROL below 170 identical: $lo/169; from 170 differing: $hi/1831"
# refusals: a held LOCK, a store directory set, a height that is not a save point
python3 - "$NEW" <<'P'
import fcntl, subprocess, sys
f = open("rf/LOCK", "a"); fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
r = subprocess.run([sys.argv[1], "libzync-reforest", "rf", "2000", "--bitcoin-cli", "bitcoin-cli"], capture_output=True, text=True)
print("REFUSE lock held: exit", r.returncode, "|", [l for l in (r.stderr + r.stdout).splitlines() if "LOCK" in l][:1])
P
HAZYNC_COINLOG_DIR=/tmp/x $NEW libzync-reforest rf 2000 --bitcoin-cli bitcoin-cli > r1.log 2>&1; say "REFUSE store dir set: exit $? | $(grep -ao 'touches no store[^"]*' r1.log | head -1)"
$NEW libzync-reforest rf 1999 --bitcoin-cli bitcoin-cli > r2.log 2>&1; say "REFUSE not a save point: exit $? | $(grep -ao 'S must be a save point' r2.log | head -1)"
# independent cross-check: the node's undo hashes vs undo_ref.py on the repo's block files
for h in 130000 140000 741000 962000 965500; do
  a=$($NEW libzync-undo-hashes $h $h --bitcoin-cli bitcoin-cli --jobs 1 | grep -ao "hash [0-9a-f]*" | head -1 | cut -c6-)
  b=$(python3 $S/scripts/libzync/undo_ref.py $S/prover/block_$h.json | grep -ao "hash [0-9a-f]*" | cut -c6-)
  [ -n "$a" ] && [ "$a" = "$b" ] && say "XCHECK $h node == undo_ref.py ($a)" || say "WRONG XCHECK $h node $a ref $b"
done
# throughput of phase 1 (getblock verbosity 3), samples of real heights
for r in "100000 100399" "300000 300199" "500000 500099" "590000 590099" "970000 970049"; do
  set -- $r; $NEW libzync-undo-hashes $1 $2 --bitcoin-cli bitcoin-cli --jobs 8 | grep -a "^UNDO_HASHES" | tail -1 | sed "s/^/RATE $1..$2 jobs=8 /"
done
say "ACCEPT_DONE"
