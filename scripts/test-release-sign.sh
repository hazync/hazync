#!/usr/bin/env bash
# The release signature is bound to the tag and cannot be laundered (review 11-12, release #2).
#
# Runs the two key-holding steps of .github/workflows/release-sign.yml, taken from the YAML itself, on synthetic
# release assets with a throwaway GPG key and a stub `gh`:
#   - a correct set signs, and the manifest names the tag, the tag's commit and its image;
#   - (the engine release, 28.11.2026) a key tarball that is not its record (image.json's id, vk.bin's pin), a core
#     verifier without the core image, a Windows zip without exactly one hzengine.exe, an installer that is not one, and
#     a tag with no reproduce/engine-release.tsv are refused. The records here are synthetic (the real ones come with
#     the Stage 4 freeze);
#   - a re-sign over unchanged assets passes; a re-sign after an asset was swapped is refused; a release whose
#     existing manifest does not verify is refused.
#
#   scripts/test-release-sign.sh                      # must PASS
#   scripts/test-release-sign.sh --control            # the same steps with the three checks cut out: MUST FAIL
#   scripts/test-release-sign.sh --workflow <file>    # another copy of the workflow
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
WF=.github/workflows/release-sign.yml; CONTROL=0
case "${1:-}" in
  --control)  CONTROL=1 ;;
  --workflow) WF=$2 ;;
esac
command -v gpg >/dev/null || { echo "SKIP: no gpg"; exit 0; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
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
GPG_PRIVATE_KEY=$(gpg --batch --armor --export-secret-keys); export GPG_PRIVATE_KEY GPG_PASSPHRASE=''
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
    # the assets and the tag's records, made together (a record = the key tarball's image id and vk.bin pin)
    python3 - "$WS/dist" "${2:-}" "$WS/tagtree/scripts/release-assets.txt" "$WS/tagtree/reproduce/engine-release.tsv" <<'PY'
import hashlib, io, json, os, sys, tarfile, zipfile
d, mode, lst, rec = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
names = [l.split()[1] for l in open(lst) if l.split()[:1] in (["release"], ["sign"]) and l.split()[1].startswith("hazync-")]
records = {}
for n in names:
    if n.startswith("hazync-keys-") and n.endswith(".tar.gz"):
        name = n[len("hazync-keys-"):-len(".tar.gz")]
        exe, vm, vk = os.urandom(32).hex(), os.urandom(32).hex(), os.urandom(256)
        records[name] = (hashlib.sha256(bytes.fromhex(exe + vm)).hexdigest(), hashlib.sha256(vk).hexdigest())
        if mode == "bad-keys" and name == "headers":
            vk = os.urandom(256)                       # a key that is not the record's pinned one
        members = [("vk.bin", vk), ("image.json", json.dumps({"exe": exe, "vm": vm}).encode())] + \
            [(fn, fn.encode()) for fn in ("sdk.pk", "agg.vk", "openvm.toml", "guest.vmexe", "baseline.json")]
        if name == "headers" and mode == "dotslash":   # R3-6 #3: names the old check normalised with lstrip("./")
            members = [("./" + fn, data) for fn, data in members]
        if name == "headers" and mode == "dup-member":  # a second vk.bin: `tar -x` keeps the LAST one
            members.append(("vk.bin", os.urandom(256)))
        if name == "headers" and mode == "dotdot":
            members.append(("../vk.bin", os.urandom(256)))
        with tarfile.open(os.path.join(d, n), "w:gz") as t:
            for fn, data in members:
                ti = tarfile.TarInfo(fn); ti.size = len(data); t.addfile(ti, io.BytesIO(data))
idh = records["core"][0]
raw, hx = bytes.fromhex(idh), idh.encode()
for n in names:
    if n.startswith("hazync-keys-"):
        continue
    body = os.urandom(64)
    if n in ("hazync-verify-x86_64-linux-gnu", "hazync-verify-aarch64", "hazync-verify.wasm"): body += hx + os.urandom(16)
    if mode == "bad-verifier" and n == "hazync-verify.wasm": body = os.urandom(200)
    if n == "hazync-setup-x86_64-windows.exe": body = b"MZ" + os.urandom(64) + b"Inno Setup Setup Data (6.4.0)" + os.urandom(64)
    if mode == "not-installer" and n == "hazync-setup-x86_64-windows.exe": body = b"MZ" + os.urandom(200)
    p = os.path.join(d, n)
    if n.endswith(".zip"):
        with zipfile.ZipFile(p, "w") as z:
            z.writestr("host.exe" if mode == "bad-zip" else "hzengine.exe", os.urandom(64)); z.writestr("cudart64_12.dll", b"x")
    else:
        open(p, "wb").write(body)
if mode != "no-records":
    with open(rec, "w") as f:
        f.write("# name network image pin\n")
        for name, (img, pin) in sorted(records.items()):
            if mode == "missing-record" and name == "headers":
                continue                               # R3-6 #3: a key tarball on the release with no record
            f.write(f"{name} {'signet' if name.endswith('-signet') else 'mainnet'} {img} {pin}\n")
        if mode == "dup-record":                       # a second core line: the old dict kept the LAST
            f.write(f"core mainnet {os.urandom(32).hex()} {os.urandom(32).hex()}\n")
PY
    git -C "$WS/tagtree" init -q && git -C "$WS/tagtree" -c user.name=t -c user.email=t@t add -A \
        && git -C "$WS/tagtree" -c user.name=t -c user.email=t@t commit -qm tag
    ID=$(awk '$1=="core" {print $3}' "$WS/tagtree/reproduce/engine-release.tsv" 2>/dev/null)
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

# 2. not this tag's images
workspace badkeys bad-keys; write
check "$([ $? != 0 ] && grep -q "hazync-keys-headers.tar.gz's vk.bin is not the headers record's pinned key" "$WS/write.log" && echo 1)" "a key tarball whose vk.bin is not its record's pinned key is refused"
workspace badver bad-verifier; write
check "$([ $? != 0 ] && grep -q "hazync-verify.wasm does not carry the core image" "$WS/write.log" && echo 1)" "a core verifier without the core image is refused"
workspace badzip bad-zip; write
check "$([ $? != 0 ] && grep -q "does not hold exactly one hzengine.exe" "$WS/write.log" && echo 1)" "a Windows zip without hzengine.exe is refused"
workspace norec no-records; write
check "$([ $? != 0 ] && grep -q "no engine image is published at this tag" "$WS/write.log" && echo 1)" "a tag with no reproduce/engine-release.tsv is refused"
workspace notinst not-installer; write
check "$([ $? != 0 ] && grep -q "is not an Inno Setup installer" "$WS/write.log" && echo 1)" "a Windows installer that is not an installer is refused"
# review R3-6 #3 (28.11.2026)
workspace norecord missing-record; write
check "$([ $? != 0 ] && grep -q "hazync-keys-headers.tar.gz has no record" "$WS/write.log" && echo 1)" "a key tarball with no record is refused (it was signed unchecked)"
workspace duprec dup-record; write
check "$([ $? != 0 ] && grep -q "names 'core' twice" "$WS/write.log" && echo 1)" "a record name given twice is refused (the last one silently won)"
workspace dotslash dotslash; write
check "$([ $? != 0 ] && grep -q "hazync-keys-headers.tar.gz: members must be exactly the key files" "$WS/write.log" && echo 1)" "tar members named ./vk.bin are refused (no name normalising)"
workspace dupmem dup-member; write
check "$([ $? != 0 ] && grep -q "hazync-keys-headers.tar.gz: members must be exactly the key files" "$WS/write.log" && echo 1)" "a tarball with two vk.bin members is refused (tar -x keeps the last)"
workspace dotdot dotdot; write
check "$([ $? != 0 ] && grep -q "hazync-keys-headers.tar.gz: members must be exactly the key files" "$WS/write.log" && echo 1)" "a tarball member ../vk.bin is refused"

# 3. re-signs: the good release's assets AND its tag's records (each workspace makes fresh ones)
goodtag() { cp "$GOOD"/dist/* "$WS/dist/"; cp "$GOOD"/SHA256SUMS.txt "$GOOD"/SHA256SUMS.txt.asc "$WS/prev/"
            cp "$GOOD/tagtree/reproduce/engine-release.tsv" "$WS/tagtree/reproduce/"
            git -C "$WS/tagtree" -c user.name=t -c user.email=t@t commit -qam "the good tag's records"; }
workspace resign; goodtag
write && sign; check "$([ $? = 0 ] && echo 1)" "a re-sign over unchanged assets passes"
workspace swap; goodtag
echo "#!/bin/sh  swapped" > "$WS/dist/hazync-worker"
write; sign; check "$([ $? != 0 ] && grep -q "differ from the hashes" "$WS/sign.log" && echo 1)" "a re-sign after swapping hazync-worker is refused"
workspace forged; goodtag
sed -i 's/^\([0-9a-f]\)/0/' "$WS/prev/SHA256SUMS.txt"
write; sign; check "$([ $? != 0 ] && grep -q "does not verify" "$WS/sign.log" && echo 1)" "a release whose existing manifest does not verify is refused"

if [ "$CONTROL" = 1 ]; then
    if [ "$fails" -ge 4 ]; then echo "CONTROL OK: $fails checks fail on the workflow without the fix, as they must"; exit 0; fi
    echo "CONTROL BROKEN: only $fails failed -- this test proves nothing"; exit 1
fi
[ "$fails" = 0 ] && echo PASS || { echo "FAIL ($fails)"; for f in "$T"/*/write.log "$T"/*/sign.log; do echo "== $f"; tail -5 "$f"; done; }
[ "$fails" = 0 ] || exit 1
exit 0
