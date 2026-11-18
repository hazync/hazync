#!/bin/bash
# hazync-utreexo-follow's own logic with STUB prover and checker (no proofs, seconds): the review fixes of 18.11.2026.
#   test_follow_logic.sh <block dir with block_1..block_300.json (mainnet)>
# The stub prover writes "STUB <H> <T> <steps> <hash>" receipts; the stub checker accepts one only for its own H and hash
# and, when T > H, only with --expect-tip <its own T> (so a republish must name the RECEIPT's core tip, review L1).
# Cases: M1 (an RPC failure or a node below the proven height is an error, never a reorg; nothing is moved), M2 (a
# fall-back moves the orphaned states and published receipts aside; retention keeps the most recently published, never
# the current one), L1, L2 (latest is a symlink to a state directory holding the receipt and its meta), L3 (latest.json
# reaches the publish target), L4 (only the step's bodies are fetched, the link is headers; PRUNE_BLOCKS).
set -uo pipefail
SRC=$(cd "$1" && pwd)
HERE=$(cd "$(dirname "$0")" && pwd); F=${FOLLOW:-$HERE/../hazync-utreexo-follow}   # FOLLOW: a modified copy (the controls)
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
mkdir -p "$W/bin" "$W/www" "$W/blocks"
fails=0
t () { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (got '$3', want '$2')"; fails=$((fails + 1)); fi; }
export FAKE_BLOCKS=$SRC FAKE_TIP=300 STUB_T=290 W
cli () { "$HERE/fake-bitcoin-cli" "$@"; }

cat > "$W/bin/zyncapp" <<'EOF'
#!/bin/bash
# stub prover: core-info, utreexo-tip-prove (writes the --cache file and counts the --headers lines)
case "$1" in
  core-info) echo "CORE height=$STUB_T tip=$("$CLI" getblockhash "$STUB_T") image=$(printf '0%.0s' $(seq 64))";;
  utreexo-tip-prove)
    read -r _ _ _ steps _ < "$3"; h=$4; out=$6
    shift 6; while [ $# -gt 0 ]; do case "$1" in --cache) touch "$2"; shift;; --headers) wc -l < "$2" > "$W/headers.count"; shift;; esac; shift; done
    echo "STUB $h $STUB_T $((steps + 1)) $("$CLI" getblockhash "$h")" > "$out";;
  *) exit 1;;
esac
EOF
cat > "$W/bin/zapp-utreexo" <<'EOF'
#!/usr/bin/env python3
# stub checker: verify <rcpt> --network n --expect H:X [--expect-tip T:TX] [--json] [--allow-demo] [--image ..] [--base-image ..]
import sys, json
a = sys.argv[1:]
f = open(a[1]).read().split()
h, t, steps, x = int(f[1]), int(f[2]), int(f[3]), f[4]
opt = lambda k: a[a.index(k) + 1] if k in a else None
eh, ex = opt('--expect').split(':')
ok = int(eh) == h and ex == x
tip = opt('--expect-tip')
if tip: ok = ok and int(tip.split(':')[0]) == t
import os
if os.path.exists(os.environ['W'] + '/vfail') and str(h) == open(os.environ['W'] + '/vfail').read().strip(): ok = False
if not ok: print('ZAPP_UTREEXO_FAIL: stub'); sys.exit(1)
if '--json' in a:
    j = {'height': h, 'block_hash': x, 'leaves': h, 'roots': [], 'core_height': t}
    if steps: j.update(mode='tip', steps=steps, base_height=3)
    print(json.dumps(j)); sys.exit(2)
print(f"ZAPP_UTREEXO_DEMO: mainnet Utreexo accumulator (Floresta's) after block {h} {x} is stub"); sys.exit(2)
EOF
cat > "$W/bin/fetch" <<'EOF'
#!/bin/bash
# stub fetch: copies block files lo..hi from FAKE_BLOCKS and logs the range
echo "$1 $2" >> "$W/fetch.log"; for h in $(seq "$1" "$2"); do cp "$FAKE_BLOCKS/block_$h.json" "$3/"; done
EOF
chmod +x "$W/bin"/*
export CLI=$HERE/fake-bitcoin-cli UTREEXO_DIR=$W/d ZYNCAPP=$W/bin/zyncapp VERIFY=$W/bin/zapp-utreexo CORE=/dev/null \
       BLOCKS=$W/blocks FETCH=$W/bin/fetch BITCOIN_CLI=$HERE/fake-bitcoin-cli PUBLISH=$W/www/ PUBLISH_KEY=/dev/null \
       ALLOW_DEMO=1 MARGIN=6
X3=$(cli getblockhash 3)
echo "STUB 3 3 0 $X3" > "$W/base.rcpt"
"$F" --set-base "$W/base.rcpt" 3 "$X3" > "$W/0.log" 2>&1; t "the base installs" 0 $?
[ -n "${DEBUG:-}" ] && cat "$W/0.log"
t "  latest is a symlink to a state directory (L2)" "yes 3 $X3" "$([ -L "$W/d/latest" ] && echo yes) $(cat "$W/d/latest/meta")"

MAX_STEP=47 "$F" > "$W/1.log" 2>&1; t "step 3 -> 50" 0 $?
t "  no forest cache yet: bodies 1..50 fetched (L4)" "1 50" "$(tail -1 "$W/fetch.log")"
t "  the link 51..290 as headers only" 240 "$(cat "$W/headers.count")"
MAX_STEP=30 "$F" > "$W/2.log" 2>&1; t "step 50 -> 80" 0 $?
t "  with the cache: bodies 51..80 only (L4)" "51 80" "$(tail -1 "$W/fetch.log")"
MAX_STEP=20 "$F" > "$W/3.log" 2>&1; t "step 80 -> 100" 0 $?
t "  latest is 100, its receipt and meta agree (L2)" "100 100" "$(cut -d' ' -f1 "$W/d/latest/meta") $(cut -d' ' -f2 "$W/d/latest/rcpt")"
t "  published: 50, 80, 100" "utreexo-100.rcpt utreexo-50.rcpt utreexo-80.rcpt" "$(cd "$W/d/pub" && ls utreexo-*.rcpt | sort | tr '\n' ' ' | sed 's/ $//')"
t "  latest.json reached the publish target (L3)" 100 "$(python3 -c "import json;print(json.load(open('$W/www/latest.json'))['height'])")"

snap () { (cd "$W/d" && find state pub -type f | sort | xargs sha256sum; ls -l forest.bin; readlink latest) 2>&1 | sha256sum; }
before=$(snap)
FAKE_RPC_FAIL=1 MAX_STEP=20 "$F" > "$W/4.log" 2>&1; t "an RPC failure is an error (M1)" 1 $?
t "  not read as a reorg" 0 "$(grep -c REORG "$W/4.log")"
t "  nothing moved" "$before" "$(snap)"
FAKE_TIP=90 STUB_T=90 MAX_STEP=20 "$F" > "$W/5.log" 2>&1; t "a node below the proven height is an error (M1)" 1 $?
t "  named as such" 1 "$(grep -c 'below the proven height' "$W/5.log")"
t "  nothing moved" "$before" "$(snap)"

# a reorg at 100: the node's block 100 is another one. Fall back to 80, move 100's state and receipt aside, step to 90
FAKE_HASH_AT=100:$(printf 'e%.0s' $(seq 64)) MAX_STEP=10 "$F" > "$W/6.log" 2>&1; t "a reorg at 100 recovers (M2)" 0 $?
[ -n "${DEBUG:-}" ] && cat "$W/6.log"
t "  fell back to 80" 1 "$(grep -c 'fell back to the kept proof of block 80' "$W/6.log")"
t "  latest is 90" 90 "$(cut -d' ' -f1 "$W/d/latest/meta")"
t "  the orphaned receipt 100 is no longer published" "utreexo-50.rcpt utreexo-80.rcpt utreexo-90.rcpt" "$(cd "$W/d/pub" && ls utreexo-*.rcpt | sort | tr '\n' ' ' | sed 's/ $//')"
t "  and is kept aside, with its state" "2" "$(find "$W/d/orphaned" \( -name 'utreexo-100.rcpt' -o -name '100-*' \) | wc -l)"
t "  the cache moved aside too (the replay restarts from genesis)" 1 "$(find "$W/d/orphaned" -name forest.bin | wc -l)"
t "  the old branch's block files above 80 moved aside (M3)" 20 "$(find "$W/d/orphaned" -path '*/blocks/block_*.json' | wc -l)"
t "  so the new branch's 81..90 are fetched (M3)" "81 90" "$(tail -1 "$W/fetch.log")"

# retention by publish order, never the current one
sleep 1; MAX_STEP=5 PUBLISHED_KEEP=2 "$F" > "$W/7.log" 2>&1; t "step 90 -> 95 with PUBLISHED_KEEP=2" 0 $?
t "  keeps the current and the one published before it" "utreexo-90.rcpt utreexo-95.rcpt" "$(cd "$W/d/pub" && ls utreexo-*.rcpt | sort | tr '\n' ' ' | sed 's/ $//')"
sleep 1; rm -f "$W/d/published"; MAX_STEP=0 PUBLISHED_KEEP=1 "$F" > "$W/8.log" 2>&1; t "a republish with PUBLISHED_KEEP=1" 0 $?
t "  keeps only the current receipt" "utreexo-95.rcpt" "$(cd "$W/d/pub" && ls utreexo-*.rcpt | tr '\n' ' ' | sed 's/ $//')"

# L1: the core proof moved to 296, the kept receipt rests on 290: a republish names the receipt's own tip
rm -f "$W/d/published"; STUB_T=296 MAX_STEP=0 "$F" > "$W/9.log" 2>&1; t "a republish under a newer core proof checks the receipt's own core tip (L1)" 0 $?

# PRUNE_BLOCKS: the bodies at or below H go once the cache is there
PRUNE_BLOCKS=1 MAX_STEP=5 "$F" > "$W/10.log" 2>&1; t "step 95 -> 100 with PRUNE_BLOCKS=1" 0 $?
t "  no body at or below 100 is left" 0 "$(find "$W/blocks" -name 'block_*.json' | sed 's/.*block_\([0-9]*\)\.json/\1/' | awk '$1 <= 100' | wc -l)"

# review of f99c301 M2: a step that fails after the prove (here: its receipt does not verify) with PRUNE_BLOCKS=1 must not
# leave the host's forest cache (saved at H) behind while latest stays at P
echo 105 > "$W/vfail"
PRUNE_BLOCKS=1 MAX_STEP=5 "$F" > "$W/11.log" 2>&1; t "a step that fails after the prove (M2)" 1 $?
t "  drops the forest cache, latest stays 100" "no 100" "$([ -e "$W/d/forest.bin" ] && echo yes || echo no) $(cut -d' ' -f1 "$W/d/latest/meta")"
rm -f "$W/vfail"
PRUNE_BLOCKS=1 MAX_STEP=5 "$F" > "$W/12.log" 2>&1; t "  the next run recovers" 0 $?
t "  replaying from genesis: bodies 1..105 fetched again" "1 105" "$(tail -1 "$W/fetch.log")"

# M1: a state that cannot be written (disk full) is never installed; latest stays complete; the next run recovers
mkdir -p "$W/shim"; printf '#!/bin/bash\ncase "$*" in *"/state/."*) echo "cp: write error: No space left on device" >&2; exit 1;; esac\nexec /usr/bin/cp "$@"\n' > "$W/shim/cp"; chmod +x "$W/shim/cp"
before=$(readlink "$W/d/latest")
PATH=$W/shim:$PATH MAX_STEP=3 "$F" > "$W/13.log" 2>&1; t "a state write that fails (M1)" 1 $?
t "  latest unchanged and complete" "$before yes" "$(readlink "$W/d/latest") $([ -s "$W/d/latest/rcpt" ] && [ -s "$W/d/latest/meta" ] && echo yes)"
MAX_STEP=3 "$F" > "$W/14.log" 2>&1; t "  the next run recovers" 0 $?
t "  latest is 108" 108 "$(cut -d' ' -f1 "$W/d/latest/meta")"

# L1: a getblockhash failure (getblockcount fine) is logged, not silent
printf '#!/bin/bash\n[ "$1" = getblockhash ] && { echo "error code: -28" >&2; exit 28; }\nexec %q "$@"\n' "$HERE/fake-bitcoin-cli" > "$W/bin/cli1"; chmod +x "$W/bin/cli1"
BITCOIN_CLI=$W/bin/cli1 MAX_STEP=3 "$F" > "$W/15.log" 2>&1; t "a getblockhash failure is an error (L1)" 1 $?
t "  and says so" 1 "$(grep -c 'did not answer getblockhash' "$W/15.log")"

[ "$fails" -eq 0 ] && echo UTREEXO_FOLLOW_LOGIC_OK || { echo "UTREEXO_FOLLOW_LOGIC_FAILED ($fails)"; exit 1; }
