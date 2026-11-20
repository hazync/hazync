#!/usr/bin/env bash
# release.sh will not publish a CUDA host that has not proved real blocks on real GPUs (20.11.2026).
#
# v0.26.1 and v0.27.0 shipped a Linux CUDA host that segfaulted on its first GPU step; every release check was a
# BYTE check on a box with no GPU. Step 4b now requires the real-GPU gate's markers (scripts/release_gpu_gate.py)
# for EXACTLY the staged files. This runs release.sh's OWN step 4b (cut out of the script, as test-release-attest.sh
# does for step 4) against a fake dist/ -- no network, no GPU, no build:
#   refused: no marker; a marker for another host sha256 (incl. a host rebuilt after the gate); another verifier;
#            another image; result FAIL; no Blackwell card; no pre-Blackwell card; no Windows marker; a Windows
#            marker for another zip.   proceeds: both markers right.   dry run: reports, does not die.
# Plus the clean-build stamp: host_is_current (release.sh's own function) treats a host without
# .clean-build-<asset> = HEAD as stale, so it is rebuilt from an empty target.
#
#   scripts/test-release-gpu-gate.sh            # must PASS
#   scripts/test-release-gpu-gate.sh --control  # step 4b's refusal made non-fatal in a COPY: must FAIL
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
CONTROL=0; [ "${1:-}" = "--control" ] && CONTROL=1
CANON=$(awk '$1=="libzync-product" && $2=="mainnet" {print $4}' reproduce/LIBZYNC_IDS)
[[ "$CANON" =~ ^[0-9a-f]{64}$ ]] || { echo "FAIL no canonical id"; exit 1; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
fails=0
check() { if [ "$1" = 1 ]; then echo "  ok   $2"; else echo "  FAIL $2"; fails=$((fails+1)); fi; }

R="$T/release.sh"; cp scripts/release.sh "$R"
if [ "$CONTROL" = 1 ]; then
    python3 - "$R" <<'PY'
import sys
p = sys.argv[1]; s = open(p).read()
a = s.index('step "4b.'); b = s.index('DRY RUN COMPLETE')
blk = s[a:b].replace('    die "no real-GPU evidence', '    echo "no real-GPU evidence')
open(p, "w").write(s[:a] + blk + s[b:])
PY
    echo "CONTROL: step 4b's refusal made non-fatal -- the refusal checks below MUST fail"
fi

# ---- structure: 4b sits between the artifact checks and publishing, on the path every publish takes --------------
lno() { grep -n -m1 -F -- "$1" "$R" | cut -d: -f1; }
l_ver=$(lno './scripts/check-release-verifiers.sh "$DIST"'); l_4b=$(lno 'step "4b.'); l_dry=$(lno 'DRY RUN COMPLETE')
l_5=$(lno 'step "5. tag and publish"'); l_create=$(lno 'gh release create')
check "$([ -n "$l_4b" ] && [ "$l_ver" -lt "$l_4b" ] && [ "$l_4b" -lt "$l_dry" ] && [ "$l_dry" -lt "$l_5" ] && [ "$l_5" -lt "$l_create" ] && echo 1)" \
      "step 4b runs after the verifier checks and before the dry-run exit, the tag and the upload"
check "$(sed -n "${l_4b},${l_dry}p" "$R" | grep -q 'release_gpu_gate.py check --dist "$DIST" --image "$CANON"' && echo 1)" \
      "step 4b asks release_gpu_gate.py check about \$DIST and the release's image"

# ---- release.sh's own step 4b, against a fake dist ----------------------------------------------------------------
BLOCK=$(sed -n "/^step \"4b\./,/DRY RUN COMPLETE/p" "$R" | sed '$d')
D="$T/dist"; mkdir -p "$D"
head -c 8192 /dev/urandom > "$D/hazync-host-x86_64-linux-gnu-cuda"
head -c 4096 /dev/urandom > "$D/hazync-verify-x86_64-linux-gnu"
python3 -c "import zipfile,os,sys; z=zipfile.ZipFile(sys.argv[1],'w'); z.writestr('host.exe', os.urandom(2048)); z.writestr('cudart64_12.dll', b'd'); z.close()" "$D/hazync-host-x86_64-windows-cuda.zip"
HEAD_SHA=$(printf '%040d' 7)
run4b() {   # run4b <dry 0|1> -> output; REACHED_PUBLISH printed only if 4b let it through
    {   echo 'die() { echo "FAIL  $*" >&2; exit 1; }; ok() { echo "  ok   $*"; }; step() { echo "=== $* ==="; }'
        echo "DIST='$D'; CANON='$CANON'; DRY=$1; TAG=v0.0.0-test; HEAD_SHA='$HEAD_SHA'; CU=\"\$DIST/hazync-host-x86_64-linux-gnu-cuda\""
        printf '%s\n' "$BLOCK"
        echo 'echo REACHED_PUBLISH'
    } > "$T/step4b.sh"
    bash "$T/step4b.sh" 2>&1
}
mk() {   # mk <linux|windows> <python dict edits...>: write a marker that is right for the fake dist, then edit it
    python3 - "$D" "$CANON" "$HEAD_SHA" "$@" <<'PY'
import hashlib, json, os, sys
sys.path.insert(0, "scripts")
import release_gpu_gate as g
d, canon, commit, kind, *edits = sys.argv[1:]
sha = lambda n: hashlib.sha256(open(os.path.join(d, n), "rb").read()).hexdigest()
blocks = [{"height": b[0], "verified": True, "receipt_sha256": "ab" * 32} for b in g.gate_blocks()]
run = lambda name, cc: {"gpu_name": name, "compute_cap": cc, "host_sha256_on_pod": sha(g.CUDA_ASSET),
                        "terminated": True, "blocks": blocks}
if kind == "linux":
    m = {"schema": 1, "kind": "linux-cuda", "result": "PASS", "asset": g.CUDA_ASSET, "host_sha256": sha(g.CUDA_ASSET),
         "verifier_sha256": sha(g.VERIFIER_ASSET), "image": canon, "finished": "test", "repo_commit": commit, "repo_dirty": False,
         "fixtures_sha256": g.sha256_file(os.path.join(g.FIXTURES, "SHA256SUMS")),
         "runs": [run("NVIDIA A40", "8.6"), run("NVIDIA RTX PRO 4500 Blackwell Server Edition", "12.0")]}
    path = os.path.join(d, g.MARKER_LINUX)
else:
    m = {"schema": 1, "kind": "windows-cuda", "result": "ATTESTED", "asset": g.WIN_ASSET, "zip_sha256": sha(g.WIN_ASSET),
         "host_exe_sha256": g.zip_host_exe_sha(os.path.join(d, g.WIN_ASSET)), "repo_commit": commit, "repo_dirty": False, "recorded_by": "test",
         "verifier_sha256": sha(g.VERIFIER_ASSET), "image": canon, "gpu_name": "NVIDIA GeForce GTX 1050 Ti",
         "recorded_at": "test", "blocks": [{"height": 170, "verified": True, "receipt_sha256": "cd" * 32}]}
    path = os.path.join(d, g.MARKER_WIN)
for e in edits:
    exec(e)
json.dump(m, open(path, "w"))
PY
}
clear() { rm -f "$D/.gpu-gate-linux-cuda.json" "$D/.gpu-gate-windows-cuda.json"; }
refused() {   # refused <title>
    local out; out=$(run4b 0)
    check "$(! printf '%s' "$out" | grep -q REACHED_PUBLISH && printf '%s' "$out" | grep -q 'no real-GPU evidence' && echo 1)" "refused: $1"
}

clear;                                                      refused "no markers at all"
clear; mk windows;                                          refused "no Linux CUDA marker"
clear; mk linux;                                            refused "no Windows CUDA marker"
clear; mk linux 'm["host_sha256"] = "0" * 64'; mk windows;  refused "a Linux marker for a different host sha256"
clear; mk linux 'm["verifier_sha256"] = "0" * 64'; mk windows; refused "receipts verified by a different verifier"
clear; mk linux 'm["image"] = "1" * 64'; mk windows;        refused "a marker for another image"
clear; mk linux 'm["result"] = "FAIL"'; mk windows;         refused "a marker whose result is FAIL"
clear; mk linux 'm["runs"] = m["runs"][:1]'; mk windows;    refused "no Blackwell card in the marker"
clear; mk linux 'm["runs"] = m["runs"][1:]'; mk windows;    refused "no pre-Blackwell card in the marker"
clear; mk linux 'm["runs"][1]["blocks"] = m["runs"][1]["blocks"][:1]'; mk windows; refused "a card that did not verify every gate block"
clear; mk linux 'm["runs"][0]["terminated"] = False'; mk windows; refused "a run whose pod was not confirmed terminated"
clear; mk linux 'm["repo_commit"] = "0" * 40'; mk windows;  refused "a Linux marker made from another commit than the release's"
clear; mk linux 'm["fixtures_sha256"] = "0" * 64'; mk windows; refused "a Linux marker that proved other fixtures"
clear; mk linux; mk windows 'm["host_exe_sha256"] = "0" * 64'; refused "a Windows run of another host.exe than the staged zip's"
clear; mk linux; mk windows 'm["result"] = "PASS"';         refused "a Windows marker that claims more than ATTESTED"
clear; mk linux; mk windows 'm["repo_commit"] = "0" * 40';  refused "a Windows marker recorded from another commit"
clear; mk linux 'm["repo_dirty"] = True'; mk windows;        refused "a Linux marker from a checkout with uncommitted changes"
clear; mk linux; mk windows 'm["repo_dirty"] = True';        refused "a Windows marker from a checkout with uncommitted changes"
clear; mk linux; mk windows 'm["blocks"][0]["receipt_sha256"] = "ab" * 32'; refused "a Windows marker whose receipt is the Linux gate's"
clear; mk linux; mk windows 'm["zip_sha256"] = "0" * 64';   refused "a Windows marker for a different zip"
clear; mk linux; mk windows 'm["blocks"] = []';             refused "a Windows marker with no verified block 170"
clear; mk linux; mk windows
out=$(run4b 0)
check "$(printf '%s' "$out" | grep -q REACHED_PUBLISH && echo 1)" "proceeds: both markers name exactly the staged files"
check "$(printf '%s' "$out" | grep -q 'Linux CUDA host .* PROVED' && printf '%s' "$out" | grep -q 'Windows CUDA host .* is ATTESTED' && echo 1)" \
      "  ...and says PROVED for Linux, ATTESTED (not proved) for Windows"
head -c 8192 /dev/urandom > "$D/hazync-host-x86_64-linux-gnu-cuda"   # the CUDA host rebuilt after the gate ran
refused "the CUDA host was rebuilt after the gate (its marker names the old sha256)"
clear
out=$(run4b 1)
check "$(printf '%s' "$out" | grep -q REACHED_PUBLISH && printf '%s' "$out" | grep -q 'would REFUSE' && echo 1)" \
      "dry run: reports that the publish would be refused, and does not die"

# ---- the clean-build stamp, through release.sh's own host_is_current ----------------------------------------------
FN=$(sed -n '/^host_is_current() {/,/^}/p' "$R")
H="$T/hd"; mkdir -p "$H"
python3 -c "import sys;open(sys.argv[1],'wb').write(b'\x7fELFnot-runnable'+bytes.fromhex(sys.argv[2])+b'tail')" "$H/hazync-host-x86_64-linux-gnu-cuda" "$CANON"
echo deadbeef > "$H/.built-from-hazync-host-x86_64-linux-gnu-cuda"
# shellcheck disable=SC2034  # DIST and SRC_REV are read by the eval'd host_is_current
hic() { ( DIST="$H"; SRC_REV=deadbeef; eval "$FN"; host_is_current "$H/hazync-host-x86_64-linux-gnu-cuda" hazync-host-x86_64-linux-gnu-cuda ); }
hic; check "$([ $? = 1 ] && echo 1)" "a host with no .clean-build stamp is stale (rebuilt from an empty target)"
echo "deadbeef $(sha256sum "$H/hazync-host-x86_64-linux-gnu-cuda" | cut -c1-64)" > "$H/.clean-build-hazync-host-x86_64-linux-gnu-cuda"
hic; check "$([ $? = 0 ] && echo 1)" "  ...with the stamp for this commit AND this binary's sha256 it is current"
echo "deadbeef $(printf '%064d' 0)" > "$H/.clean-build-hazync-host-x86_64-linux-gnu-cuda"
hic; check "$([ $? = 1 ] && echo 1)" "  ...a stamp for another binary (swapped after the clean build) is stale"
check "$(grep -q 'HAZYNC_REUSE_TARGET=0 ./prover/build-release.sh' "$R" && echo 1)" "release.sh builds with HAZYNC_REUSE_TARGET=0"
check "$(awk '/rm -f "\$OUT\/.clean-build-\$asset"/{r=NR} /mv -f "\$OUT\/.\$asset.tmp/{m=NR} /> "\$OUT\/.clean-build-\$asset"/{w=NR} END{exit !(r && m && w && r<m && m<w)}' prover/build-release.sh && echo 1)" \
      "build-release.sh removes the stamp BEFORE moving the binary in and writes it after"
check "$(grep -q 'flock -n 9' prover/build-release.sh && echo 1)" "build-release.sh refuses a second same-mode build in one checkout"

# ---- the pod-side dead-man (scripts/release-gpu-gate-pod.sh --deadman <s> <pod id>), stub runpodctl/curl ----------
mkdir -p "$T/bin" "$T/wd"
printf '#!/bin/sh\necho "runpodctl $*" >> "%s"\nexit "${STUB_RC:-0}"\n' "$T/calls" > "$T/bin/runpodctl"
printf '#!/bin/sh\necho "curl $*" >> "%s"\nexit 22\n' "$T/calls" > "$T/bin/curl"
chmod +x "$T/bin/runpodctl" "$T/bin/curl"
printf 'PATH=/usr/bin\0RUNPOD_POD_ID=abc123xyz\0RUNPOD_API_KEY=podscopedkey\0HOME=/root\0' > "$T/environ"
printf 'PATH=/usr/bin\0HOME=/root\0' > "$T/environ-nopod"
dm() {   # dm <environ> <expected id> [STUB_RC]: one dead-man run; calls and log are fresh each time
    rm -f "$T/calls" "$T/deadman.log"
    PATH="$T/bin:$PATH" STUB_RC="${3:-0}" DEADMAN_ENVIRON="$1" DEADMAN_WAIT=0 DEADMAN_LOG="$T/deadman.log" \
        DEADMAN_WORKDIR="$T/wd" bash scripts/release-gpu-gate-pod.sh --deadman 0 "$2"
}
dm "$T/environ" abc123xyz
check "$(grep -q '^runpodctl pod delete abc123xyz$' "$T/calls" && grep -q 'runpodctl pod delete answered OK' "$T/deadman.log" && echo 1)" \
      "dead-man: deletes its OWN pod with runpodctl (id from PID 1's environment) and logs which step answered"
check "$(grep -q 'curl .*-X DELETE .*Bearer podscopedkey.*/pods/abc123xyz' "$T/calls" && echo 1)" \
      "dead-man: and through the REST API with the pod-scoped key"
dm "$T/environ" otherpod
check "$([ ! -s "$T/calls" ] && grep -q REFUSING "$T/deadman.log" && echo 1)" "dead-man: armed for another pod id: refuses, calls nothing"
dm "$T/environ-nopod" abc123xyz
check "$([ ! -s "$T/calls" ] && grep -q REFUSING "$T/deadman.log" && echo 1)" "dead-man: no RUNPOD_POD_ID (not on a pod): refuses, calls nothing"
setsid sleep 300 & spid=$!
sleep 0.2; echo "$spid" > "$T/wd/script.pid"
dm "$T/environ" abc123xyz 1
sleep 0.2
check "$(grep -q 'COULD NOT REMOVE THIS POD' "$T/deadman.log" && ! kill -0 "$spid" 2>/dev/null && echo 1)" \
      "dead-man: when nothing removes the pod it says so and kills the gate's workload (not PID 1)"
kill "$spid" 2>/dev/null

if [ "$CONTROL" = 1 ]; then echo "CONTROL: $fails failure(s)"; else echo "$fails failure(s)"; fi
if [ "$CONTROL" = 1 ]; then [ "$fails" != 0 ]; else [ "$fails" = 0 ]; fi
