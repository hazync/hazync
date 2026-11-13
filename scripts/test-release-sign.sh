#!/usr/bin/env bash
# The release signature is bound to the tag and cannot be laundered (review 11-12, release #2).
#
# Runs the two key-holding steps of .github/workflows/release-sign.yml, taken from the YAML itself, on synthetic
# release assets with a throwaway GPG key and a stub `gh`:
#   - a correct set signs, and the manifest names the tag, the tag's commit and its image;
#   - a host or verifier that does not carry the tag's image (once) is refused;
#   - a re-sign over unchanged assets passes; a re-sign after an asset was swapped is refused; a release whose
#     existing manifest does not verify is refused.
#
#   scripts/test-release-sign.sh                      # must PASS
#   scripts/test-release-sign.sh --control            # the same steps with the three checks cut out: MUST FAIL
#   scripts/test-release-sign.sh --workflow <file>    # another copy of the workflow
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
REPO_ROOT=$(pwd)
WF=.github/workflows/release-sign.yml; CONTROL=0
case "${1:-}" in
  --control)  CONTROL=1 ;;
  --workflow) WF=$2 ;;
esac
command -v gpg >/dev/null || { echo "SKIP: no gpg"; exit 0; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
ID=$(awk '$1=="libzync-product" && $2=="mainnet" {print $4}' reproduce/LIBZYNC_IDS)
fails=0
check() { if [ "$1" = 1 ]; then echo "  ok   $2"; else echo "  FAIL $2"; fails=$((fails+1)); fi; }

step() {   # step <job> <name> -> the step's run script
    python3 - "$WF" "$1" "$2" <<'PY'
import sys, yaml
d = yaml.safe_load(open(sys.argv[1]))
for st in d["jobs"][sys.argv[2]]["steps"]:
    if st.get("name") == sys.argv[3]:
        print(st["run"]); break
else:
    sys.exit(f"no step {sys.argv[3]!r}")
PY
}
WRITE=$(step sign "Write SHA256SUMS over the binaries") || exit 1
SIGN=$(step sign "PGP-sign SHA256SUMS") || exit 1
if [ "$CONTROL" = 1 ]; then
    # the steps as they were before the review: the image check, the binding and the re-sign guard taken out
    WRITE=$(python3 -c '
import sys; s = sys.stdin.read()
i = s.index("# EVERY HOST AND VERIFIER"); j = s.index("sha256sum $bins; } > ../SHA256SUMS.txt") + len("sha256sum $bins; } > ../SHA256SUMS.txt")
print(s[:i] + "sha256sum $bins > ../SHA256SUMS.txt" + s[j:])' <<< "$WRITE") || exit 1
    SIGN=$(python3 -c '
import sys; s = sys.stdin.read()
i = s.index("if [ -s prev/SHA256SUMS.txt ]; then"); j = s.index("\nfi\n", i) + 4
print(s[:i] + s[j:])' <<< "$SIGN") || exit 1
    echo "CONTROL: the signing steps without the image check, the binding and the re-sign guard -- the checks below MUST fail"
fi

# a throwaway signing key
export GNUPGHOME=$T/gnupg; mkdir -m 700 "$GNUPGHOME"
gpg --batch --quiet --passphrase '' --quick-gen-key "test <test@invalid>" ed25519 sign never 2>/dev/null
export GPG_PRIVATE_KEY=$(gpg --batch --armor --export-secret-keys) GPG_PASSPHRASE=''
# a stub gh: `release view --json assets` lists dist/
mkdir -p "$T/bin"; cat > "$T/bin/gh" <<'SH'
#!/bin/bash
[ "$1 $2" = "release view" ] && { ls "$WS/dist" | grep '^hazync-'; exit 0; }
exit 0
SH
chmod +x "$T/bin/gh"; export PATH="$T/bin:$PATH" RELEASE_TAG=v9.9.9 REPO=test/test GITHUB_RUN_ID=1 GH_TOKEN=x

# workspace <name> [bad-host|bad-wasm-twice]: a release's assets, the tag tree, no previous manifest
workspace() {
    WS=$T/$1; export WS; rm -rf "$WS"; mkdir -p "$WS/dist" "$WS/prev" "$WS/tagtree/scripts" "$WS/tagtree/reproduce"
    cp scripts/release-assets.txt "$WS/tagtree/scripts/"; cp reproduce/LIBZYNC_IDS "$WS/tagtree/reproduce/"
    git -C "$WS/tagtree" init -q && git -C "$WS/tagtree" -c user.name=t -c user.email=t@t add -A \
        && git -C "$WS/tagtree" -c user.name=t -c user.email=t@t commit -qm tag
    python3 - "$WS/dist" "$ID" "${2:-}" "$WS/tagtree/scripts/release-assets.txt" <<'PY'
import os, sys, zipfile
d, idh, mode, lst = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
raw, hx = bytes.fromhex(idh), idh.encode()
names = [l.split()[1] for l in open(lst) if l.split()[:1] in (["release"], ["sign"]) and l.split()[1].startswith("hazync-")]
for n in names:
    body = os.urandom(64)
    if n.startswith("hazync-host-x86_64-linux"): body += raw + os.urandom(64)
    if n in ("hazync-verify-x86_64-linux-gnu", "hazync-verify-aarch64", "hazync-verify.wasm"): body += hx + os.urandom(16)
    if mode == "bad-host" and n == "hazync-host-x86_64-linux-gnu": body = os.urandom(200)
    if mode == "bad-wasm-twice" and n == "hazync-verify.wasm": body += hx
    p = os.path.join(d, n)
    if n.endswith(".zip"):
        with zipfile.ZipFile(p, "w") as z: z.writestr("host.exe", os.urandom(64) + raw); z.writestr("cudart64_12.dll", b"x")
    else:
        open(p, "wb").write(body)
PY
}
write() { (cd "$WS" && bash --noprofile --norc -eo pipefail -c "$WRITE") > "$WS/write.log" 2>&1; }
sign()  { (cd "$WS" && bash --noprofile --norc -eo pipefail -c "$SIGN")  > "$WS/sign.log" 2>&1; }

# 1. a correct release
workspace good
write; w=$?; sign; s=$?
check "$([ $w = 0 ] && [ $s = 0 ] && gpg --verify "$WS/SHA256SUMS.txt.asc" "$WS/SHA256SUMS.txt" 2>/dev/null && echo 1)" "a correct release is written and signed"
commit=$(git -C "$WS/tagtree" rev-parse HEAD)
check "$(grep -qx "# hazync v9.9.9 commit $commit image $ID" "$WS/SHA256SUMS.txt" 2>/dev/null && echo 1)" "the signed manifest names the tag, its commit and the image"
check "$( (cd "$WS/dist" && sha256sum --strict -c ../SHA256SUMS.txt >/dev/null 2>&1) && echo 1)" "sha256sum --strict -c still checks it (the binding lines are comments)"
GOOD=$WS

# 2. not this tag's image
workspace badhost bad-host; write
check "$([ $? != 0 ] && grep -q "not this tag's image" "$WS/write.log" && echo 1)" "a Linux host without the tag's image is refused"
workspace badwasm bad-wasm-twice; write
check "$([ $? != 0 ] && grep -q "hazync-verify.wasm carries it 2x" "$WS/write.log" && echo 1)" "a verifier carrying the image twice is refused"

# 3. re-signs
workspace resign; cp "$GOOD"/dist/* "$WS/dist/"; cp "$GOOD"/SHA256SUMS.txt "$GOOD"/SHA256SUMS.txt.asc "$WS/prev/"
write && sign; check "$([ $? = 0 ] && echo 1)" "a re-sign over unchanged assets passes"
workspace swap; cp "$GOOD"/dist/* "$WS/dist/"; cp "$GOOD"/SHA256SUMS.txt "$GOOD"/SHA256SUMS.txt.asc "$WS/prev/"
echo "#!/bin/sh  swapped" > "$WS/dist/hazync-worker"
write; sign; check "$([ $? != 0 ] && grep -q "differ from the hashes" "$WS/sign.log" && echo 1)" "a re-sign after swapping hazync-worker is refused"
workspace forged; cp "$GOOD"/dist/* "$WS/dist/"; cp "$GOOD"/SHA256SUMS.txt "$GOOD"/SHA256SUMS.txt.asc "$WS/prev/"
sed -i 's/^\([0-9a-f]\)/0/' "$WS/prev/SHA256SUMS.txt"
write; sign; check "$([ $? != 0 ] && grep -q "does not verify" "$WS/sign.log" && echo 1)" "a release whose existing manifest does not verify is refused"

if [ "$CONTROL" = 1 ]; then
    if [ "$fails" -ge 4 ]; then echo "CONTROL OK: $fails checks fail on the workflow without the fix, as they must"; exit 0; fi
    echo "CONTROL BROKEN: only $fails failed -- this test proves nothing"; exit 1
fi
[ "$fails" = 0 ] && echo PASS || { echo "FAIL ($fails)"; for f in "$T"/*/write.log "$T"/*/sign.log; do echo "== $f"; tail -5 "$f"; done; }
exit $([ "$fails" = 0 ] && echo 0 || echo 1)
