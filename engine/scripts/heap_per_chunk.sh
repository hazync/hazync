#!/bin/bash
# Roadmap 1.8: the guest's peak heap per chunk at OpenVM's 512 MiB ceiling, on the final image's code (C heap, format 2).
# Metering images (bench-noverify: HZHEAP = the bump allocator's final position minus its start, i.e. the heap's high-water
# mark: Rust never gives memory back to the bump, freed C blocks are reused from their size class, so the bump only grows
# when the live set does). Regtest: keys-rtchb (keys-rtch + bench-noverify, same guest tree and base). Mainnet: keys-chb.
# Attack blocks: chunk streams from `engine-export` (the cost-based chunking a proof uses) by the regtest host.
# shellcheck source=/dev/null
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$HOME/.risc0/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
E=/root/engine-kit/engine/host/target/release/hzengine; G=/root/engine-guestch/prover/methods/guest
HR=/root/engine-src-rt/prover/target/release/host; W=/root/engine/heap; mkdir -p $W; cd /root/engine || exit 1
if [ ! -f keys-rtchb_build.log ] || ! grep -aq '^build:' keys-rtchb_build.log; then
  mkdir -p keys-rtchb && cp keys-rt29/{sdk.pk,agg.vk,openvm.toml} keys-rtchb/
  HAZYNC_NETWORK=regtest HZ_GUEST_FEATURES=regtest,bench-noverify HZ_BASE=/root/engine-base6 nice $E build --dir keys-rtchb --guest $G > keys-rtchb_build.log 2>&1 || { echo BUILD_FAIL; grep -a "^error\|panicked" -A6 keys-rtchb_build.log | head; exit 1; }
fi
echo "keys-rtchb: $(grep -a '^build:' keys-rtchb_build.log | cut -c1-110)"
meas() { # <keys> <stream> -> "heap MiB, cost, rc"
  o=$($E execute --dir $1 --input $2 --log $2.guest.log 2>&1 | sed 's/\x1b\[[0-9;]*m//g'); o="$o
$(cat $2.guest.log 2>/dev/null)"
  h=$(echo "$o" | grep -a "HZHEAP:" | tail -1 | sed -E 's/.*HZHEAP: ([0-9]+) bytes allocated \(input ([0-9]+).*/\1 \2/')
  c=$(echo "$o" | grep -a "^execute:" | sed -E 's/.*cost ([0-9]+).*/\1/')
  set -- $h; echo "heap $(python3 -c "print(f'{$1/2**20:.1f}')" 2>/dev/null) MiB, input $(python3 -c "print(f'{$2/2**20:.2f}')" 2>/dev/null) MiB, cost $c$(echo "$o" | grep -aq 'panicked\|rror' && echo ' ERROR')"; }
for x in light_only:126:/root/worst-case/run/blocks mem_k1140:126:/root/worst-case/run/blocks tap_small:127:/root/oomfix/tap/blocks tap_full:127:/root/oomfix/tap/blocks; do
  IFS=: read n h d <<< "$x"; B=$d/$n/block_$h.json; X=$W/$n
  [ -d $X ] || { HAZYNC_NETWORK=regtest HAZYNC_PROFILE_LAYOUT=cost HAZYNC_HINT_THREADS=8 HAZYNC_BLOCK=$B $HR libzync-hints $W/$n.lzb > $W/$n.hints.log 2>&1
  HAZYNC_NETWORK=regtest HAZYNC_PROFILE_LAYOUT=cost HAZYNC_HINT_THREADS=8 HAZYNC_BLOCK=$B HAZYNC_LZ_HINTS=$W/$n.lzb $HR engine-export $X > $W/$n.export.log 2>&1; }
  echo "== $n (block $h): $(ls $X/chunk_*.in 2>/dev/null | wc -l) chunks exported; $(grep -a 'predicted cost' $W/$n.export.log | head -1)"
  for f in $X/chunk_*.in; do echo "  $(basename $f): $(meas keys-rtchb $f)"; done
done
for d in edgef/*/ negf2/x_base/; do f=$(ls -S $d/chunk_*.in 2>/dev/null | head -1); [ -n "$f" ] || continue
  echo "== mainnet $(basename $d) largest chunk $(basename $f) of $(ls $d/chunk_*.in | wc -l): $(meas keys-chb $f)"; done
echo HEAP_DONE
