#!/bin/bash
# hazync-headers-follow against a fake bitcoin-cli serving REAL mainnet headers, with the real prover and verifier.
#   test_follow.sh <zyncapp> <swiftsync-verify> <zyncapp-headers.bin> <base 0..975278 receipt> <headers file(s), colon-separated>
# Two real CPU proofs (~1 min each). Checks: base install (wrong hash refused), publish, unchanged tip, a moved tip
# replaces the run, a reorg below the base and a wrong ELF are refused with the published folder untouched.
set -uo pipefail
Z=$1; V=$2; E=$3; BASE=$4; export FAKE_HEADERS=$5
HERE=$(cd "$(dirname "$0")" && pwd); F=$HERE/../hazync-headers-follow
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
export HEADERS_DIR=$W/d ZYNCAPP=$Z VERIFY=$V ELF=$E BITCOIN_CLI=$HERE/fake-bitcoin-cli PUBLISH=$W/www/
mkdir -p "$W/www"
fails=0
t () { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (got '$3', want '$2')"; fails=$((fails + 1)); fi; }
B=975278; BX=00000000000000000001d951592f7a06f0d24697e4f9c362ae173fced55583e3

FAKE_TIP=975300 "$F" --set-base "$BASE" $B 00000000000000000000000000000000000000000000000000000000000000ff >/dev/null 2>&1
t "a base under the wrong hash is refused" 1 $?
FAKE_TIP=975300 "$F" --set-base "$BASE" $B $BX >/dev/null 2>&1
t "the base installs" 0 $?

FAKE_TIP=975300 "$F" > "$W/1.log" 2>&1; t "first run publishes" 0 $?
t "published runs" '["base_0_975278.rcpt", "run_975279_975300.rcpt"]' "$(python3 -c "import json;print(json.dumps(json.load(open('$W/www/runs.json'))['runs']))")"
X300=$("$HERE/fake-bitcoin-cli" getblockhash 975300)
"$V" headers --expect 975300:$X300 "$W/www/base_0_975278.rcpt" "$W/www/run_975279_975300.rcpt" | grep -q '^SWIFTSYNC_VERIFY_OK'
t "the published pair verifies to the tip 975300" 0 $?

FAKE_TIP=975300 "$F" > "$W/2.log" 2>&1; t "same tip: exit 0" 0 $?
t "same tip: nothing proved" 1 "$(grep -c 'unchanged' "$W/2.log")"

FAKE_TIP=975315 "$F" > "$W/3.log" 2>&1; t "moved tip publishes" 0 $?
t "the old run is gone, the new one is in" "base_0_975278.rcpt run_975279_975315.rcpt runs.json" "$(ls "$W/www" | tr '\n' ' ' | sed 's/ $//')"
t "runs.json tip" 975315 "$(python3 -c "import json;print(json.load(open('$W/www/runs.json'))['tip'])")"

before=$(sha256sum "$W/www/runs.json")
FAKE_TIP=975315 FAKE_HASH_AT=$B:00000000000000000000000000000000000000000000000000000000000000ee "$F" > "$W/4.log" 2>&1
t "a reorg below the base is refused" 1 $?
t "  and names it" 1 "$(grep -c 'no longer on the node' "$W/4.log")"
t "  and leaves the published folder alone" "$before" "$(sha256sum "$W/www/runs.json")"

printf '#!/bin/bash\ncp "%s" "$3"\n' "$BASE" > "$W/lying-prover"; chmod +x "$W/lying-prover"   # "proves" by copying the base
ZYNCAPP=$W/lying-prover FAKE_TIP=975310 "$F" > "$W/6.log" 2>&1
t "a run that does not verify is not published" 1 $?
t "  and names it" 1 "$(grep -c 'does not verify' "$W/6.log")"
t "  and leaves the published folder alone" "$before" "$(sha256sum "$W/www/runs.json")"

cp "$E" "$W/bad.elf"; printf '\x00' >> "$W/bad.elf"
ELF=$W/bad.elf FAKE_TIP=975315 "$F" > "$W/5.log" 2>&1; t "a wrong ELF is refused" 1 $?

[ "$fails" -eq 0 ] && echo HEADERS_FOLLOW_TESTS_OK || { echo "HEADERS_FOLLOW_TESTS_FAILED ($fails)"; exit 1; }
