#!/bin/bash
# hazync-utreexo-follow against a fake bitcoin-cli over REAL mainnet block files, with the real prover and checker.
#   test_follow.sh <zyncapp> <zapp-utreexo> <base receipt: mainnet H = 3> <core receipt: mainnet 1..T> <block dir 1..T>
# Real CPU proofs (three steps, minutes each). Every proof here is DEMO (the core proof is far below minimum work, and
# the images were local builds when it was written), so the follower runs with ALLOW_DEMO=1. Checks: base install (a wrong hash refused),
# the first step published, an unchanged tip proves nothing, a moved tip replaces the receipt (a step from a step), a
# reorg below the published height falls back to the kept receipt and steps forward again, a run that does not verify
# and a wrong ELF leave the published folder untouched, a reorg below the base stops.
set -uo pipefail
Z=$1; V=$2; BASE=$3; CORE=$4; export FAKE_BLOCKS=$5
HERE=$(cd "$(dirname "$0")" && pwd); F=$HERE/../hazync-utreexo-follow
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
# The follower's OWN block directory (it moves block files aside on a reorg): hard links to the shared files 1..T + 164,
# so a rename or delete inside $W never touches the source directory (which the fake node keeps reading)
UTREEXO_DIR=$W/d BLOCKS=$W/blocks "$F" --init >/dev/null 2>&1 || { echo "FAIL --init"; exit 1; }   # the ownership marker
# the core receipt's height T (any genesis-anchored chain proof: 3,836 when written, the spine's copy later) and a node
# tip 164 above it
T=$("$Z" core-info "$CORE" | sed -n 's/^CORE height=\([0-9]*\) .*/\1/p'); [[ "$T" =~ ^[0-9]+$ ]] || { echo "FAIL core-info"; exit 1; }
TIP=$((T + 164))
for h in $(seq 1 "$TIP"); do ln "$FAKE_BLOCKS/block_$h.json" "$W/blocks/" 2>/dev/null || cp "$FAKE_BLOCKS/block_$h.json" "$W/blocks/"; done
# fetch = copy from the source directory (the node's chain); a reorg's fall-back makes the follower fetch again
printf '#!/bin/bash\nfor h in $(seq "$1" "$2"); do cp "%s/block_$h.json" "$3/" || exit 1; done\n' "$FAKE_BLOCKS" > "$W/fetch"; chmod +x "$W/fetch"
export UTREEXO_DIR=$W/d ZYNCAPP=$Z VERIFY=$V CORE BLOCKS=$W/blocks BITCOIN_CLI=$HERE/fake-bitcoin-cli PUBLISH=$W/www/ \
       ALLOW_DEMO=1 MARGIN=6 FETCH=$W/fetch FAKE_TIP=$TIP
unset RISC0_DEV_MODE
mkdir -p "$W/www"
fails=0
t () { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (got '$3', want '$2')"; fails=$((fails + 1)); fi; }
js () { python3 -c "import json,sys; print(json.load(open('$W/www/latest.json'))[sys.argv[1]])" "$1"; }
cli () { "$HERE/fake-bitcoin-cli" "$@"; }
X3=$(cli getblockhash 3)
# The node's tip is T + 164, above the core proof's T (a node is never behind the chain proof), so H = min(tip - 6,
# T, P + MAX_STEP): MAX_STEP picks each step's H here; the margin itself is the min with tip - 6.

"$F" --set-base "$BASE" 3 00000000000000000000000000000000000000000000000000000000000000ff >/dev/null 2>&1
t "a base under the wrong hash is refused" 1 $?
"$F" --set-base "$BASE" 3 "$X3" > "$W/0.log" 2>&1; t "the base installs" 0 $?

MAX_STEP=100 "$F" > "$W/1.log" 2>&1; t "first step publishes" 0 $?
t "  latest.json height" 103 "$(js height)"
t "  latest.json mode" tip "$(js mode)"
t "  latest.json steps" 1 "$(js steps)"
X103=$(cli getblockhash 103)
"$V" verify "$W/www/latest.rcpt" --expect "103:$X103" > "$W/v1.log" 2>&1; t "  the published receipt verifies to block 103 (DEMO)" 2 $?
cmp -s "$W/www/latest.rcpt" "$W/www/utreexo-103.rcpt"; t "  latest.rcpt is utreexo-103.rcpt" 0 $?

MAX_STEP=0 "$F" > "$W/2.log" 2>&1; t "no new height: exit 0" 0 $?
t "  nothing proved" 1 "$(grep -c 'nothing to do' "$W/2.log")"
# the choice of H (DRY_RUN: no proving): the margin, the core tip, the step cap
h_of () { "$F" 2>&1 | sed -n 's/.*target H=\([0-9]*\) .*/\1/p'; }
t "the margin: node tip T + 4 -> H = T - 2" $((T - 2)) "$(FAKE_TIP=$((T + 4)) DRY_RUN=1 MAX_STEP=100000 h_of)"
t "the core tip: node tip T + 164 -> H = T" "$T" "$(DRY_RUN=1 MAX_STEP=100000 h_of)"
t "the step cap: 103 + 50" 153 "$(DRY_RUN=1 MAX_STEP=50 h_of)"

# H = 180: a step from the step at 103 (the first spends are at 170)
MAX_STEP=77 "$F" > "$W/3.log" 2>&1; t "moved tip publishes" 0 $?
t "  latest.json height" 180 "$(js height)"
t "  latest.json steps" 2 "$(js steps)"
t "  the old state is kept for reorgs" yes "$(ls -d "$W"/d/state/103-* >/dev/null 2>&1 && echo yes)"
t "  the step started from the saved forest of 103 (the production source, no replay)" 1 "$(grep -c 'UTREEXO_TIP_CACHE hit at 103 (forest' "$W/d/work/prove.log")"
t "  the forests of 103 and 180 are both kept" "2" "$(find "$W/d" -maxdepth 1 \( -name 'forest-103-*.bin' -o -name 'forest-180-*.bin' \) | wc -l)"
X180=$(cli getblockhash 180)
"$V" verify "$W/www/latest.rcpt" --expect "180:$X180" > "$W/v3.log" 2>&1; t "  the published receipt verifies to block 180 (DEMO)" 2 $?
grep -q 'tip mode: 2 steps from the base at 3, the last from 103' "$W/v3.log"; t "  and names its chain of steps" 0 $?
before=$(sha256sum "$W/www/latest.json")

printf '#!/bin/bash\n[ "$1" = core-info ] && exec %q "$@"\ncp %q "${6}"\n' "$Z" "$BASE" > "$W/lying-prover"; chmod +x "$W/lying-prover"   # "proves" by copying the base
ZYNCAPP=$W/lying-prover MAX_STEP=20 "$F" > "$W/4.log" 2>&1; t "a step that does not verify is not published" 1 $?
t "  and names it" 1 "$(grep -c 'does not verify' "$W/4.log")"
t "  and leaves the published folder alone" "$before" "$(sha256sum "$W/www/latest.json")"

cp "$Z" "$W/z.elf"; ELF=$W/z.elf WANT_ELF=$(printf '0%.0s' $(seq 64)) MAX_STEP=20 "$F" > "$W/5.log" 2>&1
t "a wrong ELF is refused" 1 $?
t "  and leaves the published folder alone" "$before" "$(sha256sum "$W/www/latest.json")"

# a reorg: the node's block 180 is another one. The follower falls back to the kept receipt of 103 and steps forward on
# the node's branch, here to 175 (MAX_STEP 72; the block files are the node's chain below 180).
MAX_STEP=72 FAKE_HASH_AT=180:$(printf 'e%.0s' $(seq 64)) "$F" > "$W/6.log" 2>&1
t "a reorg below the published height recovers" 0 $?
t "  and names it" 1 "$(grep -c 'REORG: the proven block 180' "$W/6.log")"
t "  fell back to 103" 1 "$(grep -c 'fell back to the kept proof of block 103' "$W/6.log")"
t "  latest.json height" 175 "$(js height)"
t "  latest.json steps (base, 103, 175)" 2 "$(js steps)"
t "  the step from 103 started from the KEPT forest of 103 (review F1)" 1 "$(grep -c 'UTREEXO_TIP_CACHE hit at 103 (forest' "$W/d/work/prove.log")"

MAX_STEP=20 FAKE_HASH_AT=3:$(printf 'e%.0s' $(seq 64)) "$F" > "$W/7.log" 2>&1
t "a reorg below the base stops" 1 $?
t "  and names it" 1 "$(grep -c 'no longer on the node' "$W/7.log")"

[ "$fails" -eq 0 ] && echo UTREEXO_FOLLOW_TESTS_OK || { echo "UTREEXO_FOLLOW_TESTS_FAILED ($fails)"; exit 1; }
