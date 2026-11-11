#!/usr/bin/env bash
# The staged verifiers must ACCEPT a real libzync proof and REFUSE a tampered one, before a release.
#
#   scripts/check-release-verifiers.sh [dist]          # exit 0 only if every case below holds
#   scripts/check-release-verifiers.sh --cli <binary>  # the CLI cases only, on one binary
#
# --cli is how release-sign.yml checks the aarch64 verifier it cross-builds: CLI_WRAP names a runner
# to put in front of it (there: `qemu-aarch64 -L /usr/aarch64-linux-gnu`).
#
# An embedded id says which image a verifier is pinned to; it does not say the verifier works. This
# runs the artifacts that are about to ship on a REAL receipt: mainnet blocks 1..200 proved from
# genesis by the published image 8888d59c (experimental/libzync-bench/zapp/, one RTX PRO 6000,
# 2026-11-08). 1..200 is far below Core's minimum chainwork, so the best verdict a correct verifier
# can give it is DEMO (exit 2) with every check passed; anything else is a broken verifier.
#
# What is run, against the staged files only:
#   hazync-verify-x86_64-linux-gnu   real -> DEMO at height 200, the right tip, the published image;
#                                    one flipped byte in the seal, one in the journal, a wrong
#                                    --expect tip and RISC0_DEV_MODE=1 -> FAIL (exit 1)
#   hazync-verify.wasm + .js         the same receipt through the shipped glue under Node: real ->
#                                    'demo', tampered -> 'fail', imageId('mainnet') == the published id
#   hazync-host-x86_64-linux-gnu*    each host that can execute here: `libzync-verify` real -> DEMO,
#                                    tampered -> refused
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
CLI_ONLY=0
if [ "${1:-}" = "--cli" ]; then CLI_ONLY=1; V="${2:?usage: $0 --cli <binary>}"; DIST=$(dirname "$V"); else DIST="${1:-dist}"; V="$DIST/hazync-verify-x86_64-linux-gnu"; fi
read -ra WRAP <<< "${CLI_WRAP:-}"
RCPT=experimental/libzync-bench/zapp/mainnet_1_200_8888d59c.rcpt
RCPT_SHA=c58cf678c53d09990f21db15be1472aeaacd92c13d8565414b3d39037300ec45
TIP200=000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320
CANON=$(awk '$1=="libzync-product" && $2=="mainnet" {print $4}' reproduce/LIBZYNC_IDS)
fails=0; n=0
ok()  { n=$((n+1)); echo "  ok   $*"; }
bad() { n=$((n+1)); fails=$((fails+1)); echo "FAIL $*"; }

[[ "$CANON" =~ ^[0-9a-f]{64}$ ]] || { echo "FAIL no libzync-product mainnet id in reproduce/LIBZYNC_IDS"; exit 1; }
[ -f "$RCPT" ] || { echo "FAIL the reference receipt $RCPT is missing"; exit 1; }
[ "$(sha256sum "$RCPT" | cut -c1-64)" = "$RCPT_SHA" ] \
    || { echo "FAIL $RCPT is not the pinned receipt (sha256 $RCPT_SHA)"; exit 1; }
[ "$CLI_ONLY" = 1 ] || command -v node >/dev/null \
    || { echo "FAIL node is required to run the WebAssembly verifier (it is what a browser runs)"; exit 1; }

T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
# Flip one bit at a fraction of the file. The seal is the bulk of a receipt; its last bytes are the
# journal (the ChainState) and the metadata. Measured: every offset tried, either part, gives FAIL.
flip() { python3 -c "import sys
d=bytearray(open(sys.argv[1],'rb').read()); o=int(len(d)*float(sys.argv[3])); d[o]^=1
open(sys.argv[2],'wb').write(d)" "$RCPT" "$1" "$2"; }
flip "$T/seal.rcpt" 0.5
flip "$T/tail.rcpt" 0.995

echo "verifiers in $DIST against $RCPT (mainnet 1..200, image ${CANON:0:8})"

# ---- the CLI ----------------------------------------------------------------------------------
if [ -x "$V" ]; then
    out=$("${WRAP[@]}" "$V" --network mainnet --product "$RCPT" 2>&1); rc=$?
    if [ "$rc" = 2 ] && printf '%s\n' "$out" | grep -q '^SWIFTSYNC_VERIFY_DEMO' \
       && printf '%s\n' "$out" | grep -q "receipt verifies against image $CANON (mainnet)" \
       && printf '%s\n' "$out" | grep -q "tip $TIP200 at height 200"; then
        ok "CLI accepts the real receipt: DEMO, image ${CANON:0:8}, tip of block 200"
    else
        bad "CLI on the real receipt: exit $rc, want 2 with DEMO, image $CANON and tip $TIP200"
        printf '%s\n' "$out" | tail -3 | sed 's/^/       /'
    fi
    cli_refuses() {  # <what> <args...>
        local what=$1; shift
        out=$("$@" 2>&1); rc=$?
        if [ "$rc" = 1 ] && printf '%s\n' "$out" | grep -q '^SWIFTSYNC_VERIFY_FAIL'; then
            ok "CLI refuses $what"
        else
            bad "CLI did not refuse $what (exit $rc, want 1 with SWIFTSYNC_VERIFY_FAIL)"
        fi
    }
    cli_refuses "a flipped seal byte" "${WRAP[@]}" "$V" --network mainnet --product "$T/seal.rcpt"
    cli_refuses "a flipped journal byte" "${WRAP[@]}" "$V" --network mainnet --product "$T/tail.rcpt"
    cli_refuses "a wrong --expect tip" "${WRAP[@]}" "$V" --network mainnet --product --expect "200:$(printf '%064d' 1)" "$RCPT"
    cli_refuses "RISC0_DEV_MODE=1" env RISC0_DEV_MODE=1 "${WRAP[@]}" "$V" --network mainnet --product "$RCPT"
else
    bad "$V is missing or not executable"
fi

if [ "$CLI_ONLY" = 1 ]; then
    [ "$n" -ge 5 ] || { echo "FAIL only $n check(s) ran"; exit 1; }
    if [ "$fails" = 0 ]; then echo "all $n CLI check(s) passed"; exit 0; fi
    echo "$fails of $n CLI check(s) FAILED"; exit 1
fi

# ---- the WebAssembly module, through the glue that ships with it ------------------------------
if [ -f "$DIST/hazync-verify.wasm" ] && [ -f "$DIST/hazync-verify.js" ]; then
    # .mjs so Node reads the glue as the ES module it is, as a browser does with type="module".
    cp "$DIST/hazync-verify.js" "$T/glue.mjs"
    cat > "$T/run.mjs" <<'JS'
import fs from 'node:fs';
import { loadVerifier } from './glue.mjs';
const [wasm, real, seal, tail, canon] = process.argv.slice(2);
const v = await loadVerifier(fs.readFileSync(wasm));
const run = (f) => v.verify({ network: 'mainnet', image: 'product', receipt: fs.readFileSync(f) }).verdict;
console.log(`image ${v.imageId('mainnet') === canon ? 'ok' : 'WRONG ' + v.imageId('mainnet')}`);
console.log(`real ${run(real)}`);
console.log(`seal ${run(seal)}`);
console.log(`tail ${run(tail)}`);
JS
    out=$(node "$T/run.mjs" "$DIST/hazync-verify.wasm" "$RCPT" "$T/seal.rcpt" "$T/tail.rcpt" "$CANON" 2>&1); rc=$?
    want=$'image ok\nreal demo\nseal fail\ntail fail'
    if [ "$rc" = 0 ] && [ "$out" = "$want" ]; then
        ok "WASM (via hazync-verify.js): image ${CANON:0:8}, real receipt 'demo', both tampered 'fail'"
    else
        bad "WASM did not give image ok / demo / fail / fail (node exit $rc):"
        printf '%s\n' "$out" | tail -5 | sed 's/^/       /'
    fi
else
    bad "$DIST/hazync-verify.wasm or $DIST/hazync-verify.js is missing"
fi

# ---- the hosts that can run here --------------------------------------------------------------
for h in "$DIST/hazync-host-x86_64-linux-gnu" "$DIST/hazync-host-x86_64-linux-gnu-cuda"; do
    [ -x "$h" ] || continue
    b=$(basename "$h")
    timeout 120 "$h" method-id >/dev/null 2>&1 || { echo "  --   $b cannot execute here (release.sh step 4 requires its attestation)"; continue; }
    out=$(timeout 300 "$h" libzync-verify "$RCPT" 2>&1); rc=$?
    if [ "$rc" = 2 ] && printf '%s\n' "$out" | grep -q '^LIBZYNC_VERIFY_DEMO: a valid chain from genesis at height 200'; then
        ok "$b libzync-verify accepts the real receipt (DEMO at 200)"
    else
        bad "$b libzync-verify on the real receipt: exit $rc, want 2 with LIBZYNC_VERIFY_DEMO at height 200"
    fi
    out=$(timeout 300 "$h" libzync-verify "$T/seal.rcpt" 2>&1); rc=$?
    if [ "$rc" != 0 ] && [ "$rc" != 2 ] && ! printf '%s\n' "$out" | grep -q 'LIBZYNC_VERIFY_\(OK\|DEMO\)'; then
        ok "$b libzync-verify refuses a flipped seal byte (exit $rc)"
    else
        bad "$b libzync-verify did not refuse a tampered receipt (exit $rc)"
    fi
done

# Vacuity guard: a run that checked nothing must not read as a pass.
[ "$n" -ge 6 ] || { echo "FAIL only $n check(s) ran; the CLI and WASM cases alone are 6"; exit 1; }
if [ "$fails" = 0 ]; then echo "all $n verifier check(s) passed"; else echo "$fails of $n verifier check(s) FAILED — do not publish"; exit 1; fi
