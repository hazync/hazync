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
    shift 6; while [ $# -gt 0 ]; do case "$1" in --cache) shift;; --cache-out) echo "$h" > "$2"; shift;; --headers) wc -l < "$2" > "$W/headers.count"; shift;; esac; shift; done
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
t "  the link 51..290 as headers only" 240 "$(cat "$W/headers.count")"
MAX_STEP=30 "$F" > "$W/2.log" 2>&1; t "step 50 -> 80" 0 $?
t "  with the cache: bodies 51..80 only (L4)" "51 80" "$(tail -1 "$W/fetch.log")"
MAX_STEP=20 "$F" > "$W/3.log" 2>&1; t "step 80 -> 100" 0 $?
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
t "  the cache moved aside too (the replay restarts from genesis)" 1 "$(find "$W/d/orphaned" -name 'forest-*.bin' | wc -l)"
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

# review of f99c301 M2 / 0d9683a LOW 2: a step that fails after the prove (here: its receipt does not verify) with
# PRUNE_BLOCKS=1: the host wrote the cache at H to work/forest.next, never over the cache for P, which stays valid
echo 105 > "$W/vfail"
PRUNE_BLOCKS=1 MAX_STEP=5 "$F" > "$W/11.log" 2>&1; t "a step that fails after the prove (M2)" 1 $?
t "  keeps the forest cache of 100 (the one at 105 is not promoted), latest stays 100 (LOW 2)" "100 100" "$(cat "$W"/d/forest-100-*.bin) $(cut -d' ' -f1 "$W/d/latest/meta")"
rm -f "$W/vfail"
PRUNE_BLOCKS=1 MAX_STEP=5 "$F" > "$W/12.log" 2>&1; t "  the next run recovers" 0 $?
t "  from the cache: bodies 101..105 only" "101 105" "$(tail -1 "$W/fetch.log")"
t "  the cache now is forest-105-<hash>.bin, alone" "105 1" "$(cat "$W"/d/forest-105-*.bin) $(find "$W/d" -maxdepth 1 -name 'forest-*.bin' | wc -l)"

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

# LOW 1: the checker must not run on when the node does not answer for the receipt's own core tip (290, while the
# current core proof is at 296): an empty hash once went on into --expect-tip
printf '#!/bin/bash\n[ "$1 $2" = "getblockhash 290" ] && { echo "error code: -1" >&2; exit 1; }\nexec %q "$@"\n' "$HERE/fake-bitcoin-cli" > "$W/bin/cli290"; chmod +x "$W/bin/cli290"
P0=$(cat "$W/d/published"); rm -f "$W/d/published"
BITCOIN_CLI=$W/bin/cli290 STUB_T=296 MAX_STEP=0 "$F" > "$W/17.log" 2>&1; t "a node that does not answer for the receipt's core tip (LOW 1)" 1 $?
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
python3 - "$SRC/block_6.json" "$W/blocks/block_6.json" <<'PY'
import json, sys
v = json.load(open(sys.argv[1])); v['nonce'] ^= 1; json.dump(v, open(sys.argv[2], 'w'))
PY
BITCOIN_CLI=$W/bin/cli_stdin MAX_STEP=3 "$F" > "$W/19.log" 2>&1; t "a stdin-reading BITCOIN_CLI (step 3 -> 6)" 0 $?
t "  check_blocks still sees the whole range: the stale block 6 is moved aside" 1 "$(find "$W/d/orphaned" -path '*stale-step-*/blocks/block_6.json' | wc -l)"

# (4) a failing listing (python) is an error, not "nothing to check"
mkdir -p "$W/pyshim"; printf '#!/bin/bash\n[ "$2" = %q ] && exit 1\nexec /usr/bin/python3 "$@"\n' "$W/blocks" > "$W/pyshim/python3"; chmod +x "$W/pyshim/python3"
PATH=$W/pyshim:$PATH MAX_STEP=3 "$F" > "$W/20.log" 2>&1; t "a block listing that fails" "1 1" "$? $(grep -c 'listing the cached block files failed' "$W/20.log")"

# (3) the cache is tied to (P, hash): a cache left for an older height (a kill between `point latest` and its rename)
# means a replay from genesis, so the bodies from 1 are checked and fetched, under PRUNE_BLOCKS too
PRUNE_BLOCKS=1 MAX_STEP=3 "$F" > "$W/21.log" 2>&1; t "step 6 -> 9 with PRUNE_BLOCKS=1" 0 $?
f9=$(ls "$W"/d/forest-9-*.bin); mv "$f9" "$W/d/forest-6-$(cli getblockhash 6).bin"   # as if the rename never happened
PRUNE_BLOCKS=1 MAX_STEP=3 "$F" > "$W/22.log" 2>&1; t "a cache for the wrong height (review of 7806f4b, 3)" 0 $?
t "  bodies fetched from 1, not 10" "1 12" "$(tail -1 "$W/fetch.log")"

# (4) --set-base with a garbage getblockcount: refused, no file moved
printf '#!/bin/bash\n[ "$1" = getblockcount ] && { echo garbage; exit 0; }\nexec %q "$@"\n' "$HERE/fake-bitcoin-cli" > "$W/bin/cli_garbage"; chmod +x "$W/bin/cli_garbage"
nb=$(find "$W/blocks" -name 'block_*.json' | wc -l)
BITCOIN_CLI=$W/bin/cli_garbage "$F" --set-base "$W/base.rcpt" 3 "$X3" > "$W/23.log" 2>&1; t "--set-base with a garbage getblockcount is refused" 1 $?
t "  no block file moved" "$nb" "$(find "$W/blocks" -name 'block_*.json' | wc -l)"

# (1) a deep reorg that orphans the kept receipt's OWN core tip too: the fall-back state cannot be checked against this
# node, so it is not published, and the run steps on (it used to die before the step)
"$F" --set-base "$W/base.rcpt" 3 "$X3" > "$W/24.log" 2>&1
MAX_STEP=47 "$F" > "$W/25.log" 2>&1; MAX_STEP=30 "$F" > "$W/26.log" 2>&1     # 3 -> 50 -> 80, both on core tip 290
t "  (setup: latest 80)" 80 "$(cut -d' ' -f1 "$W/d/latest/meta")"
STUB_T=296 FAKE_HASH_AT=80:$(printf 'e%.0s' $(seq 64)),290:$(printf 'd%.0s' $(seq 64)) MAX_STEP=10 "$F" > "$W/27.log" 2>&1
t "a reorg that orphans the kept receipt's core tip too: the run steps on (review of 7806f4b, 1)" 0 $?
t "  the fall-back is not published, and says so" 1 "$(grep -c 'not published; stepping on' "$W/27.log")"
t "  the new step is published" 60 "$(python3 -c "import json;print(json.load(open('$W/d/pub/latest.json'))['height'])")"

[ "$fails" -eq 0 ] && echo UTREEXO_FOLLOW_LOGIC_OK || { echo "UTREEXO_FOLLOW_LOGIC_FAILED ($fails)"; exit 1; }
