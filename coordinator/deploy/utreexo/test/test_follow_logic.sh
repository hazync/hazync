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
# FAIL_FAST=1 (run_ci.sh's controls): stop at the first failing check; its label says which guard was missed
t () { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (got '$3', want '$2')"; fails=$((fails + 1)); [ "${FAIL_FAST:-0}" = 1 ] && exit 1; fi; }
# STUB_T: the core proof's height. 160 (above every H the cases reach) keeps each step's link of headers short: the
# fake node is a process per call, and the link was most of the run's cost (review round 2 #1)
export FAKE_BLOCKS=$SRC FAKE_TIP=300 STUB_T=160 W
# fwrite <file> <h>: a saved source's header for block h (as zyncapp writes it); fhh <file>: the height in one
cat > "$W/bin/fhelp" <<'EOF'
fwrite () { python3 - "$1" "$2" "${3:-$("$CLI" getblockhash "$2")}" <<'PY'
import sys, struct
open(sys.argv[1], 'wb').write(b'HZUFOR01' + struct.pack('<I', int(sys.argv[2])) + bytes.fromhex(sys.argv[3])[::-1] + b'stub')
PY
}
fhh () { python3 -c 'import sys,struct; print(struct.unpack("<I", open(sys.argv[1],"rb").read()[8:12])[0])' "$1"; }
EOF
. "$W/bin/fhelp"
cli () { "$HERE/fake-bitcoin-cli" "$@"; }

cat > "$W/bin/zyncapp" <<'EOF'
#!/bin/bash
. "$W/bin/fhelp"
# stub prover: core-info, utreexo-tip-prove (writes the --cache file and counts the --headers lines)
case "$1" in
  core-info) echo "CORE height=$STUB_T tip=$("$CLI" getblockhash "$STUB_T") image=$(printf '0%.0s' $(seq 64))";;
  utreexo-tip-prove)
    read -r _ _ _ steps _ < "$3"; h=$4; out=$6
    # a saved source's header as zyncapp writes it (magic, height u32 LE, block hash in internal order), the forest
    # source's heap line, and the --cache it was given (its own header's height: "hit <h>", or "miss")
    req=; c=
    shift 6; while [ $# -gt 0 ]; do case "$1" in --cache) c=$2; if [ -f "$2" ]; then echo "hit $(fhh "$2")"; else echo miss; fi > "$W/cache.state"; shift;;
      --cache-required) req=1;;
      --source) echo "$2" > "$W/source.arg"; shift;; --cache-out) fwrite "$2" "$h"; shift;; --headers) wc -l < "$2" > "$W/headers.count"; shift;; esac; shift; done
    # zyncapp's --cache-required: a cache it cannot load is exit 4 (here: a file without the forest's magic)
    if [ -n "$req" ] && [ "$(head -c 8 "$c" 2>/dev/null)" != HZUFOR01 ]; then echo "UTREEXO_TIP_CACHE_REFUSED unusable (stub)"; echo refused > "$W/cache.state"; exit 4; fi
    echo "UTREEXO_TIP_WITNESS source=forest blocks=1 dels=0 forest_nodes=1 forest_leaves=1 forest_heap_bytes=${STUB_HEAP:-1000}"
    echo "STUB $h $STUB_T $((steps + 1)) $("$CLI" getblockhash "$h") $("$CLI" getblockhash "$STUB_T")" > "$out";;
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
if tip: ok = ok and int(tip.split(':')[0]) == t and (len(f) < 6 or tip.split(':')[1] == f[5])
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
# LOW 3: the block directory must carry the marker --init writes; --init refuses a non-empty unmarked directory
mkdir -p "$W/shared"; echo keep > "$W/shared/block_5.json"
BLOCKS=$W/shared "$F" --set-base "$W/base.rcpt" 3 "$X3" > "$W/m1.log" 2>&1; t "an unmarked block directory is refused (LOW 3)" "1 1" "$? $(grep -c 'has no .hazync-utreexo-blocks' "$W/m1.log")"
BLOCKS=$W/shared "$F" --init > "$W/m2.log" 2>&1; t "  --init refuses a non-empty unmarked directory" 1 $?
t "  and its files are untouched" "keep" "$(cat "$W/shared/block_5.json")"
"$F" --init > "$W/m3.log" 2>&1; t "--init marks the follower's own block directory" "0 yes" "$? $([ -f "$W/blocks/.hazync-utreexo-blocks" ] && echo yes)"
# (review of 7806f4b, 2) --init never overwrites a marker naming another follower
mkdir -p "$W/other"; echo "owned by the hazync-utreexo-follow of /somewhere/else" > "$W/other/.hazync-utreexo-blocks"
BLOCKS=$W/other "$F" --init > "$W/m4.log" 2>&1; t "--init refuses a directory another follower marked" 1 $?
t "  and leaves its marker alone" "owned by the hazync-utreexo-follow of /somewhere/else" "$(cat "$W/other/.hazync-utreexo-blocks")"
"$F" --init > "$W/m5.log" 2>&1; t "  --init again on our own directory is fine" 0 $?
"$F" --set-base "$W/base.rcpt" 3 "$X3" > "$W/0.log" 2>&1; t "the base installs" 0 $?
[ -n "${DEBUG:-}" ] && cat "$W/0.log"
[ -n "${DEBUG:-}" ] && cat "$W/0.log"
t "  latest is a symlink to a state directory (L2)" "yes 3 $X3" "$([ -L "$W/d/latest" ] && echo yes) $(cat "$W/d/latest/meta")"

MAX_STEP=47 "$F" > "$W/1.log" 2>&1; t "step 3 -> 50" 0 $?
t "  no forest cache yet: bodies 1..50 fetched (L4)" "1 50" "$(tail -1 "$W/fetch.log")"
t "  the link 51..160 as headers only" 110 "$(cat "$W/headers.count")"
MAX_STEP=30 "$F" > "$W/2.log" 2>&1; t "step 50 -> 80" 0 $?
t "  with the cache: bodies 51..80 only (L4)" "51 80" "$(tail -1 "$W/fetch.log")"
MAX_STEP=20 "$F" > "$W/3.log" 2>&1; t "step 80 -> 100" 0 $?
t "  the forest source is asked for" forest "$(cat "$W/source.arg")"
t "  the FOREST_KEEP (3) newest forest caches are kept: 50, 80, 100" "50 80 100" "$(cd "$W/d" && ls forest-*.bin | sed 's/forest-\([0-9]*\)-.*/\1/' | sort -n | tr '\n' ' ' | sed 's/ $//')"
t "  latest is 100, its receipt and meta agree (L2)" "100 100" "$(cut -d' ' -f1 "$W/d/latest/meta") $(cut -d' ' -f2 "$W/d/latest/rcpt")"
t "  published: 50, 80, 100" "utreexo-100.rcpt utreexo-50.rcpt utreexo-80.rcpt" "$(cd "$W/d/pub" && ls utreexo-*.rcpt | sort | tr '\n' ' ' | sed 's/ $//')"
t "  latest.json reached the publish target (L3)" 100 "$(python3 -c "import json;print(json.load(open('$W/www/latest.json'))['height'])")"

snap () { (cd "$W/d" && find state pub -type f | sort | xargs sha256sum; ls -l forest-*.bin; readlink latest) 2>&1 | sha256sum; }
before=$(snap)
FAKE_RPC_FAIL=1 MAX_STEP=20 "$F" > "$W/4.log" 2>&1; t "an RPC failure is an error (M1)" 1 $?
t "  not read as a reorg" 0 "$(grep -c REORG "$W/4.log")"
t "  nothing moved" "$before" "$(snap)"
FAKE_TIP=90 STUB_T=90 MAX_STEP=20 "$F" > "$W/5.log" 2>&1; t "a node below the proven height is an error (M1)" 1 $?
t "  named as such" 1 "$(grep -c 'below the proven height' "$W/5.log")"
t "  nothing moved" "$before" "$(snap)"

# a reorg at 100: the node's block 100 is another one. Fall back to 80, move 100's state and receipt aside, step to 90
# (LOW 4) the first run's step fails after the fall-back: pub/latest.json already names 80, not the orphaned 100
echo 90 > "$W/vfail"
FAKE_HASH_AT=100:$(printf 'e%.0s' $(seq 64)) MAX_STEP=10 "$F" > "$W/6.log" 2>&1; t "a reorg at 100 whose next step fails" 1 $?
t "  the fall-back state 80 is published at once (LOW 4)" "80 80" "$(python3 -c "import json;print(json.load(open('$W/d/pub/latest.json'))['height'])") $(cut -d' ' -f1 "$W/d/published")"
rm -f "$W/vfail"
FAKE_HASH_AT=100:$(printf 'e%.0s' $(seq 64)) MAX_STEP=10 "$F" > "$W/6b.log" 2>&1; t "a reorg at 100 recovers (M2)" 0 $?
[ -n "${DEBUG:-}" ] && cat "$W/6.log" "$W/6b.log"
t "  fell back to 80" 1 "$(grep -c 'fell back to the kept proof of block 80' "$W/6.log")"
t "  latest is 90" 90 "$(cut -d' ' -f1 "$W/d/latest/meta")"
t "  the orphaned receipt 100 is no longer published" "utreexo-50.rcpt utreexo-80.rcpt utreexo-90.rcpt" "$(cd "$W/d/pub" && ls utreexo-*.rcpt | sort | tr '\n' ' ' | sed 's/ $//')"
t "  and is kept aside, with its state" "2" "$(find "$W/d/orphaned" \( -name 'utreexo-100.rcpt' -o -name '100-*' \) | wc -l)"
t "  the orphaned branch's forest cache (100) moved aside" "1 1" "$(find "$W/d/orphaned" -name 'forest-*.bin' | wc -l) $(find "$W/d/orphaned" -name 'forest-100-*.bin' | wc -l)"
t "  the step from 80 started from the KEPT forest cache of 80 (no replay from genesis)" "hit 80" "$(cat "$W/cache.state")"
t "  the old branch's block files above 80 moved aside (M3)" 20 "$(find "$W/d/orphaned" -path '*/blocks/block_*.json' | wc -l)"
t "  so the new branch's 81..90 are fetched (M3)" "81 90" "$(tail -1 "$W/fetch.log")"

# retention by publish order, never the current one
sleep 1; MAX_STEP=5 PUBLISHED_KEEP=2 "$F" > "$W/7.log" 2>&1; t "step 90 -> 95 with PUBLISHED_KEEP=2" 0 $?
t "  keeps the current and the one published before it" "utreexo-90.rcpt utreexo-95.rcpt" "$(cd "$W/d/pub" && ls utreexo-*.rcpt | sort | tr '\n' ' ' | sed 's/ $//')"
sleep 1; rm -f "$W/d/published"; MAX_STEP=0 PUBLISHED_KEEP=1 "$F" > "$W/8.log" 2>&1; t "a republish with PUBLISHED_KEEP=1" 0 $?
t "  keeps only the current receipt" "utreexo-95.rcpt" "$(cd "$W/d/pub" && ls utreexo-*.rcpt | tr '\n' ' ' | sed 's/ $//')"

# L1: the core proof moved to 166, the kept receipt rests on 160: a republish names the receipt's own tip
rm -f "$W/d/published"; STUB_T=166 MAX_STEP=0 "$F" > "$W/9.log" 2>&1; t "a republish under a newer core proof checks the receipt's own core tip (L1)" 0 $?

# PRUNE_BLOCKS: the bodies at or below H go once the cache is there
PRUNE_BLOCKS=1 MAX_STEP=5 "$F" > "$W/10.log" 2>&1; t "step 95 -> 100 with PRUNE_BLOCKS=1" 0 $?
t "  no body at or below 100 is left" 0 "$(find "$W/blocks" -name 'block_*.json' | sed 's/.*block_\([0-9]*\)\.json/\1/' | awk '$1 <= 100' | wc -l)"

# review of f99c301 M2 / 0d9683a LOW 2: a step that fails after the prove (here: its receipt does not verify) with
# PRUNE_BLOCKS=1: the host wrote the cache at H to work/forest.next, never over the cache for P, which stays valid
echo 105 > "$W/vfail"
PRUNE_BLOCKS=1 MAX_STEP=5 "$F" > "$W/11.log" 2>&1; t "a step that fails after the prove (M2)" 1 $?
t "  keeps the forest cache of 100 (the one at 105 is not promoted), latest stays 100 (LOW 2)" "100 100" "$(fhh "$W"/d/forest-100-*.bin) $(cut -d' ' -f1 "$W/d/latest/meta")"
rm -f "$W/vfail"
FOREST_KEEP=1 PRUNE_BLOCKS=1 MAX_STEP=5 "$F" > "$W/12.log" 2>&1; t "  the next run recovers (FOREST_KEEP=1)" 0 $?
t "  from the cache: bodies 101..105 only" "101 105" "$(tail -1 "$W/fetch.log")"
t "  the cache now is forest-105-<hash>.bin, alone" "105 1" "$(fhh "$W"/d/forest-105-*.bin) $(find "$W/d" -maxdepth 1 -name 'forest-*.bin' | wc -l)"

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

# MED: a cached block file of another branch inside the step's range (left by an interrupted fall-back, a failed step
# before a reorg, ...) is checked against the node, moved aside and fetched again
python3 - "$SRC/block_111.json" "$W/blocks/block_111.json" <<'PY'
import json, sys
v = json.load(open(sys.argv[1])); v['nonce'] ^= 1; json.dump(v, open(sys.argv[2], 'w'))
PY
MAX_STEP=3 "$F" > "$W/16.log" 2>&1; t "a stale cached block in the step's range (MED)" 0 $?
t "  moved aside" 1 "$(find "$W/d/orphaned" -path '*stale-step-*/blocks/block_111.json' | wc -l)"
t "  and fetched again from the node's chain" yes "$(cmp -s "$SRC/block_111.json" "$W/blocks/block_111.json" && echo yes)"

# LOW 1: the checker must not run on when the node does not answer for the receipt's own core tip (160, while the
# current core proof is at 166): an empty hash once went on into --expect-tip
printf '#!/bin/bash\n[ "$1 $2" = "getblockhash 160" ] && { echo "error code: -1" >&2; exit 1; }\nexec %q "$@"\n' "$HERE/fake-bitcoin-cli" > "$W/bin/cli290"; chmod +x "$W/bin/cli290"
P0=$(cat "$W/d/published"); rm -f "$W/d/published"
BITCOIN_CLI=$W/bin/cli290 STUB_T=166 MAX_STEP=0 "$F" > "$W/17.log" 2>&1; t "a node that does not answer for the receipt's core tip (LOW 1)" 1 $?
t "  nothing published" no "$([ -e "$W/d/published" ] && echo yes || echo no)"
echo "$P0" > "$W/d/published"

# MED (c): --set-base checks the block files above the new base the same way
python3 - "$SRC/block_150.json" "$W/blocks/block_150.json" <<'PY'
import json, sys
v = json.load(open(sys.argv[1])); v['nonce'] ^= 1; json.dump(v, open(sys.argv[2], 'w'))
PY
"$F" --set-base "$W/base.rcpt" 3 "$X3" > "$W/18.log" 2>&1; t "--set-base again" 0 $?
t "  moves a stale block file above the base aside (MED)" 1 "$(find "$W/d/orphaned" -path '*stale-set-base-*/blocks/block_150.json' | wc -l)"

# (review of 7806f4b, 4) the node's stdin must not be the loop's: a BITCOIN_CLI that reads stdin (as ssh does) must not
# swallow the rest of check_blocks' list. A stale file at the END of the range must still be found
printf '#!/bin/bash\ncat > /dev/null\nexec %q "$@"\n' "$HERE/fake-bitcoin-cli" > "$W/bin/cli_stdin"; chmod +x "$W/bin/cli_stdin"
for h in 1 2 3 4 5; do cp "$SRC/block_$h.json" "$W/blocks/"; done   # earlier lines in the list for a reader to swallow
python3 - "$SRC/block_6.json" "$W/blocks/block_6.json" <<'PY'
import json, sys
v = json.load(open(sys.argv[1])); v['nonce'] ^= 1; json.dump(v, open(sys.argv[2], 'w'))
PY
BITCOIN_CLI=$W/bin/cli_stdin MAX_STEP=3 "$F" > "$W/19.log" 2>&1; t "a stdin-reading BITCOIN_CLI (step 3 -> 6)" 0 $?
t "  check_blocks still sees the whole range: the stale block 6 is moved aside" 1 "$(find "$W/d/orphaned" -path '*stale-step-*/blocks/block_6.json' | wc -l)"

# (4) a failing listing (python) is an error, not "nothing to check"
mkdir -p "$W/pyshim"; printf '#!/bin/bash\n[ "$2" = %q ] && exit 1\nexec /usr/bin/python3 "$@"\n' "$W/blocks" > "$W/pyshim/python3"; chmod +x "$W/pyshim/python3"
PATH=$W/pyshim:$PATH MAX_STEP=3 "$F" > "$W/20.log" 2>&1; t "a block listing that fails" "1 1" "$? $(grep -c 'listing the cached block files failed' "$W/20.log")"

# review F1 (a): a run killed between `point latest` and the forest's rename leaves latest's forest in work/forest.next.
# It is ADOPTED (its header names latest), so the next step starts there, not from genesis, under PRUNE_BLOCKS too
PRUNE_BLOCKS=1 MAX_STEP=3 "$F" > "$W/21.log" 2>&1; t "step 6 -> 9 with PRUNE_BLOCKS=1" 0 $?
f9=$(ls "$W"/d/forest-9-*.bin); mv "$f9" "$W/d/work/forest.next"   # as if the run died before the rename
PRUNE_BLOCKS=1 MAX_STEP=3 "$F" > "$W/22.log" 2>&1; t "a run after a kill between point latest and the forest's rename (F1 a)" 0 $?
t "  work/forest.next adopted as the forest of 9" 1 "$(grep -c 'adopted work/forest.next as the forest of latest (9 ' "$W/22.log")"
t "  the step started from it: bodies 10..12 only, not from 1" "10 12 hit 9" "$(tail -1 "$W/fetch.log") $(cat "$W/cache.state")"
# a work/forest.next whose header names another block is never adopted
fwrite "$W/d/work/forest.next" 7
PRUNE_BLOCKS=1 MAX_STEP=3 "$F" > "$W/22b.log" 2>&1; t "  a forest.next for another height (7) is not adopted" "0 0 hit 12" "$? $(grep -c adopted "$W/22b.log") $(cat "$W/cache.state")"

# (4) --set-base with a garbage getblockcount: refused, no file moved
printf '#!/bin/bash\n[ "$1" = getblockcount ] && { echo garbage; exit 0; }\nexec %q "$@"\n' "$HERE/fake-bitcoin-cli" > "$W/bin/cli_garbage"; chmod +x "$W/bin/cli_garbage"
nb=$(find "$W/blocks" -name 'block_*.json' | wc -l)
BITCOIN_CLI=$W/bin/cli_garbage "$F" --set-base "$W/base.rcpt" 3 "$X3" > "$W/23.log" 2>&1; t "--set-base with a garbage getblockcount is refused" 1 $?
t "  no block file moved" "$nb" "$(find "$W/blocks" -name 'block_*.json' | wc -l)"

# (1) a deep reorg that orphans the kept receipt's OWN core tip too: the fall-back state cannot be checked against this
# node, so it is not published, and the run steps on (it used to die before the step)
"$F" --set-base "$W/base.rcpt" 3 "$X3" > "$W/24.log" 2>&1
MAX_STEP=47 "$F" > "$W/25.log" 2>&1; MAX_STEP=30 "$F" > "$W/26.log" 2>&1     # 3 -> 50 -> 80, both on core tip 160
t "  (setup: latest 80)" 80 "$(cut -d' ' -f1 "$W/d/latest/meta")"
STUB_T=166 FAKE_HASH_AT=80:$(printf 'e%.0s' $(seq 64)),160:$(printf 'd%.0s' $(seq 64)) MAX_STEP=10 "$F" > "$W/27.log" 2>&1
t "a reorg that orphans the kept receipt's core tip too: the run steps on (review of 7806f4b, 1)" 0 $?
t "  the fall-back is not published, and says so" 1 "$(grep -c 'not published; stepping on' "$W/27.log")"
t "  the new step is published" 60 "$(python3 -c "import json;print(json.load(open('$W/d/pub/latest.json'))['height'])")"

# review F1 (b): a step starts from the newest kept forest AT OR BELOW its previous height. A new base, steps to 120
# with three forests (80, 100, 120); the forest of 100 lost; a reorg at 120 falls back to the state at 100, whose
# forest is gone: the step starts from 80's and replays 81..100, fetching only from 81 (PRUNE_BLOCKS removed 1..120)
"$F" --set-base "$W/base.rcpt" 3 "$X3" > "$W/30.log" 2>&1; t "--set-base again (F1 c)" 0 $?
t "  the old base's forests moved aside, not deleted" "0 yes" "$(find "$W/d" -maxdepth 1 -name 'forest-*.bin' | wc -l) $([ "$(find "$W/d/orphaned" -path '*base-*' -name 'forest-*.bin' | wc -l)" -gt 0 ] && echo yes)"
for m in 47 30 20 20; do FOREST_KEEP=3 PRUNE_BLOCKS=1 MAX_STEP=$m "$F" > "$W/31.log" 2>&1 || { echo "FAIL setup step"; fails=$((fails + 1)); }; done
t "  (setup: forests 80 100 120, latest 120)" "80 100 120 120" "$(cd "$W/d" && ls forest-*.bin | sed 's/forest-\([0-9]*\)-.*/\1/' | sort -n | tr '\n' ' ')$(cut -d' ' -f1 "$W/d/latest/meta")"
mv "$W"/d/forest-100-*.bin "$W/lost-forest.bin"
FAKE_HASH_AT=120:$(printf 'e%.0s' $(seq 64)) FOREST_KEEP=3 PRUNE_BLOCKS=1 MAX_STEP=10 "$F" > "$W/32.log" 2>&1
t "a fall-back to a state without a forest (F1 b)" 0 $?
t "  started from the newest kept forest below it (80)" "hit 80" "$(cat "$W/cache.state")"
t "  and fetched from 81, not from 1" "81 110" "$(tail -1 "$W/fetch.log")"
# no forest at all: a replay from genesis above GENESIS_REPLAY_MAX is refused (nothing moves), allowed with the switch
mkdir -p "$W/lost"; mv "$W"/d/forest-*.bin "$W/lost/"
before=$(snap)
GENESIS_REPLAY_MAX=50 MAX_STEP=5 "$F" > "$W/33.log" 2>&1; t "a genesis replay above GENESIS_REPLAY_MAX is refused (F1)" "1 1" "$? $(grep -c 'needs ALLOW_GENESIS_REPLAY=1' "$W/33.log")"
t "  nothing moved" "$before" "$(snap)"
ALLOW_GENESIS_REPLAY=1 GENESIS_REPLAY_MAX=50 MAX_STEP=5 "$F" > "$W/34.log" 2>&1; t "  and runs with ALLOW_GENESIS_REPLAY=1, from block 1" "0 1" "$? $(tail -1 "$W/fetch.log" | cut -d' ' -f1)"
# review F3: the forest's heap is recorded for the check, and a warning logged above FOREST_HEAP_WARN_BYTES
STUB_HEAP=5000 FOREST_HEAP_WARN_BYTES=4000 MAX_STEP=5 "$F" > "$W/35.log" 2>&1; t "a step whose forest heap passes the warning line (F3)" 0 $?
t "  recorded in forest-heap, warned in the log" "5000 120 1" "$(cat "$W/d/forest-heap") $(grep -c 'WARNING: the forest holds 5000 bytes' "$W/35.log")"
# review F7: orphaned forests go after ORPHANED_FOREST_MIN minutes; other orphaned files stay
n_or=$(find "$W/d/orphaned" -type f ! -name 'forest*' | wc -l)
MAX_STEP=5 "$F" > "$W/36.log" 2>&1; t "  (default: orphaned forests kept)" "0 yes" "$? $([ "$(find "$W/d/orphaned" -name 'forest*' | wc -l)" -gt 0 ] && echo yes)"
sleep 1; ORPHANED_FOREST_MIN=0 MAX_STEP=5 "$F" > "$W/37.log" 2>&1
t "orphaned forests pruned after ORPHANED_FOREST_MIN (F7)" "0 0" "$? $(find "$W/d/orphaned" -name 'forest*' | wc -l)"
t "  the rest of orphaned/ untouched" "$n_or" "$(find "$W/d/orphaned" -type f ! -name 'forest*' | wc -l)"

# review round 2 #5: a forest ABOVE the new H (left by a kill mid-fall-back or mid---set-base) goes to orphaned/ and does
# not take a retention slot
lat () { cut -d' ' -f1 "$W/d/latest/meta"; }
hts () { (cd "$W/d" && ls forest-*.bin 2>/dev/null | sed 's/forest-\([0-9]*\)-.*/\1/' | sort -n | tr '\n' ' ' | sed 's/ $//'); }
L=$(lat)
fwrite "$W/d/forest-150-$(cli getblockhash 150).bin" 150
FOREST_KEEP=2 MAX_STEP=5 "$F" > "$W/40.log" 2>&1; t "a step with a forest above H left behind (round 2 #5)" 0 $?
t "  the forest above H moved aside, the two newest at or below H kept" "$L $((L + 5)) 1" "$(hts) $(find "$W/d/orphaned" -path '*above-*' -name 'forest-150-*' | wc -l)"
# review round 2 #2: a forest the host cannot use (corrupt) is exit 4 under --cache-required; the follower moves it to
# orphaned/ and starts from the next older one, never a silent replay from genesis
P2=$(lat)
printf 'CORRUPT' > "$W/d/forest-$P2-$(cli getblockhash "$P2").bin"
MAX_STEP=5 "$F" > "$W/41.log" 2>&1; t "an unusable forest at P (round 2 #2)" 0 $?
t "  moved to orphaned/unusable-*, and the step started from the next older forest" "1 hit $L" "$(find "$W/d/orphaned" -path '*unusable-*' -name "forest-$P2-*" | wc -l) $(cat "$W/cache.state")"
# every kept forest unusable: they all move aside, and the genesis arm still obeys GENESIS_REPLAY_MAX
for f in "$W"/d/forest-*.bin; do printf 'CORRUPT' > "$f"; done
GENESIS_REPLAY_MAX=50 MAX_STEP=5 "$F" > "$W/42.log" 2>&1
t "  every forest unusable: refused at the genesis limit, not replayed" "1 1 0" "$? $(grep -c 'needs ALLOW_GENESIS_REPLAY=1' "$W/42.log") $(grep -c 'replays from genesis' "$W/42.log")"
ALLOW_GENESIS_REPLAY=1 MAX_STEP=5 "$F" > "$W/43.log" 2>&1; t "  (recovered with ALLOW_GENESIS_REPLAY=1)" 0 $?
# review round 2 #7: a work/forest.next with latest's height but another block hash is not adopted
P3=$(lat)
fwrite "$W/d/work/forest.next" "$P3" "$(printf 'ab%.0s' $(seq 32))"
MAX_STEP=0 "$F" > "$W/44.log" 2>&1
t "a forest.next with latest's height and another hash is not adopted" "0 0 yes" "$? $(grep -c adopted "$W/44.log") $([ -f "$W/d/work/forest.next" ] && echo yes)"
# review round 2 #8: DRY_RUN changes nothing: no adoption, no pruning of orphaned forests
mv "$W/d/forest-$P3-"*.bin "$W/d/work/forest.next"
mkdir -p "$W/d/orphaned/manual"; echo x > "$W/d/orphaned/manual/forest-1-x.bin"; sleep 1
DRY_RUN=1 ORPHANED_FOREST_MIN=0 "$F" > "$W/45.log" 2>&1
t "DRY_RUN=1 adopts nothing and prunes nothing (round 2 #8)" "0 yes yes" "$? $([ -f "$W/d/work/forest.next" ] && echo yes) $([ -f "$W/d/orphaned/manual/forest-1-x.bin" ] && echo yes)"
MAX_STEP=0 "$F" > "$W/46.log" 2>&1; t "  the next real run adopts it" "0 1" "$? $(grep -c "adopted work/forest.next as the forest of latest ($P3 " "$W/46.log")"
# review round 2 #4: an RPC failure while choosing the forest stops the run; it is not "another branch" (which fell
# through to a replay from genesis)
MAX_STEP=5 "$F" > "$W/47.log" 2>&1; P4=$(lat)
mv "$W/d/forest-$P4-"*.bin "$W/lost-$P4.bin"
printf '#!/bin/bash\n[ "$1 $2" = "getblockhash %s" ] && { echo "error code: -28" >&2; exit 28; }\nexec %q "$@"\n' "$P3" "$HERE/fake-bitcoin-cli" > "$W/bin/cli_sel"; chmod +x "$W/bin/cli_sel"
BITCOIN_CLI=$W/bin/cli_sel MAX_STEP=5 "$F" > "$W/48.log" 2>&1
t "an RPC failure while choosing the forest is an error (round 2 #4)" "1 0" "$? $(grep -c 'replays from genesis' "$W/48.log")"

# settings: a bad FOREST_KEEP or WITNESS_SOURCE is refused before anything runs
FOREST_KEEP=0 "$F" > "$W/28.log" 2>&1; t "FOREST_KEEP=0 is refused" "1 1" "$? $(grep -c 'FOREST_KEEP must be' "$W/28.log")"
WITNESS_SOURCE=bridge "$F" > "$W/29.log" 2>&1; t "an unknown WITNESS_SOURCE is refused" "1 1" "$? $(grep -c 'WITNESS_SOURCE: forest or native' "$W/29.log")"

[ "$fails" -eq 0 ] && echo UTREEXO_FOLLOW_LOGIC_OK || { echo "UTREEXO_FOLLOW_LOGIC_FAILED ($fails)"; exit 1; }
