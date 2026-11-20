#!/usr/bin/env bash
# A header change in a vendored C/C++/CUDA dependency must rebuild every -sys crate compiled against it,
# even on a WARM target (the v0.26.1 / v0.27.0 Linux CUDA host segfault, 20.11.2026).
#
# The bug: vendor/sppark/build.rs emitted its rerun-if-changed lines only in sppark's OWN repo layout, and
# in the vendored layout declared nothing but rerun-if-env-changed -- which switches off cargo's default
# "re-run on any file in the package". So #727's edit of sppark/util/gpu_t.cuh (stream_t gained members) and
# of all_gpus.cpp rebuilt neither libsppark_cuda.a nor anything downstream that watched only its own
# kernels/ (risc0-circuit-rv32im-sys, -keccak-sys). A release box that reuses prover/target linked objects
# compiled against two layouts of stream_t: segfault in stream_t::Dmalloc on the first GPU step.
#
# Two fixes, each tested on its own with REAL cargo, offline, no nvcc (NVCC=off: sppark then compiles
# nothing, but its build script still runs and still decides what cargo watches):
#   A. sppark's build.rs watches its header tree in the vendored layout. Cargo then re-runs every build
#      script that depends on sppark, because their `links` dependency's build-script output is newer.
#      Tested with the REAL vendor/sppark and a dependent that watches only itself (as the -sys crates did).
#   B. each vendored -sys build.rs watches every DEP_*_ROOT it includes (watch_dep), so it rebuilds even if
#      the dependency's own script forgets to. Tested with sppark's PRE-FIX build.rs (the bug put back) and
#      a dependent using the watch_dep function copied out of vendor/risc0-circuit-rv32im-sys/build.rs.
#   The same workspace also shows the bug itself: pre-fix sppark + a dependent watching only itself does
#   NOT rebuild on the header change. Then a static check: no vendor/*/build.rs reads a DEP_* path
#   except through watch_dep, and risc0-build-kernel watches include dirs for every kernel type.
#
#   scripts/test-build-rerun.sh            # must PASS
#   scripts/test-build-rerun.sh --control  # both fixes taken out of the COPIES; must FAIL
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
CONTROL=0; [ "${1:-}" = "--control" ] && CONTROL=1
command -v cargo >/dev/null || { echo "FAIL cargo is required (this test drives cargo's real rerun rules)"; exit 1; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
fails=0
check() { if [ "$1" = 1 ]; then echo "  ok   $2"; else echo "  FAIL $2"; fails=$((fails+1)); fi; }
export NVCC=off HIPCC=off
unset RUSTC_WRAPPER CARGO_TARGET_DIR

# sppark as vendored, and the same with its HAZYNC_RERUN_ON_HEADERS block cut out (= the pre-fix file).
strip_fix() {   # $1 = build.rs to edit in place
    python3 - "$1" <<'PY'
import sys
p = sys.argv[1]; s = open(p).read()
a = s.index("    // HAZYNC_RERUN_ON_HEADERS"); b = s.index("    // pass DEP_SPPARK_* variables to dependents")
open(p, "w").write(s[:a] + s[b:])
PY
}
cp -r vendor/sppark "$T/sppark-fixed"; rm -rf "$T/sppark-fixed/target"
cp -r vendor/sppark "$T/sppark-prefix"; rm -rf "$T/sppark-prefix/target"
strip_fix "$T/sppark-prefix/build.rs"
grep -q 'HAZYNC_RERUN_ON_HEADERS' "$T/sppark-prefix/build.rs" && { echo "FAIL could not cut the fix out of the pre-fix copy"; exit 1; }
grep -q 'HAZYNC_RERUN_ON_HEADERS' "$T/sppark-fixed/build.rs" || { echo "FAIL vendor/sppark/build.rs has no HAZYNC_RERUN_ON_HEADERS block"; exit 1; }

# The watch_dep helper exactly as the vendored -sys crates carry it.
WATCH=$(python3 - vendor/risc0-circuit-rv32im-sys/build.rs <<'PY'
import sys, re
s = open(sys.argv[1]).read()
m = re.search(r"^fn watch_dep\(var: &str\) -> String \{\n.*?^\}\n", s, re.S | re.M)
print(m.group(0) if m else "")
PY
)
[ -n "$WATCH" ] || { echo "FAIL no watch_dep in vendor/risc0-circuit-rv32im-sys/build.rs"; exit 1; }

if [ "$CONTROL" = 1 ]; then
    echo "CONTROL: both fixes cut out of the copies -- the rebuild checks below MUST fail"
    strip_fix "$T/sppark-fixed/build.rs"
    WATCH=$(printf '%s\n' "$WATCH" | grep -v 'rerun-if-changed')
fi

# A dependent -sys crate: links nothing, depends on sppark, and logs every run of its build script.
mkdep() {   # mkdep <dir> <sppark dir> <naive|watch>
    mkdir -p "$1/src"; : > "$1/src/lib.rs"
    cat > "$1/Cargo.toml" <<EOF
[package]
name = "$(basename "$1")"
version = "0.0.0"
edition = "2021"
publish = false
[dependencies]
sppark = { path = "$2" }
EOF
    if [ "$3" = naive ]; then
        cat > "$1/build.rs" <<'EOF'
use std::{env, io::Write};
fn main() {
    // as the -sys crates did before the fix: watch only our own files
    println!("cargo:rerun-if-changed=build.rs");
    let root = env::var("DEP_SPPARK_ROOT").unwrap();
    // one log per crate: two build scripts run in parallel, and interleaved appends would garble a shared one
    let log = format!("{}/../runs-{}.log", env!("CARGO_MANIFEST_DIR"), env!("CARGO_PKG_NAME"));
    writeln!(std::fs::OpenOptions::new().create(true).append(true).open(log).unwrap(), "ran {root}").unwrap();
}
EOF
    else
        { cat <<'EOF'
use std::{env, io::Write};
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let root = watch_dep("DEP_SPPARK_ROOT");
    // one log per crate: two build scripts run in parallel, and interleaved appends would garble a shared one
    let log = format!("{}/../runs-{}.log", env!("CARGO_MANIFEST_DIR"), env!("CARGO_PKG_NAME"));
    writeln!(std::fs::OpenOptions::new().create(true).append(true).open(log).unwrap(), "ran {root}").unwrap();
}
EOF
          printf '%s\n' "$WATCH"; } > "$1/build.rs"
    fi
}
mkws() {   # mkws <ws> <sppark dir> <member:kind>...
    local ws=$1 sp=$2; shift 2
    mkdir -p "$ws"; local members=""
    for m in "$@"; do mkdep "$ws/${m%%:*}" "$sp" "${m##*:}"; members="$members\"${m%%:*}\", "; done
    printf '[workspace]\nresolver = "2"\nmembers = [%s]\n' "${members%, }" > "$ws/Cargo.toml"
}
runs() { grep -c '^ran ' "$1/runs-$2.log" 2>/dev/null || true; }
# Offline when ~/.cargo already has sppark's build deps (cc, which); otherwise (a fresh CI runner) fetch them once.
# cargo's own reason for every re-run goes to $T/fingerprint.log (printed when a check fails).
build() { (cd "$1" && export CARGO_LOG=cargo::core::compiler::fingerprint=info &&
            { cargo build --offline -v 2>&1 || cargo build -v 2>&1; } | tee -a "$T/fingerprint.log"); }

# Workspace A: the FIXED sppark (fix A) and a dependent that watches only itself.
mkws "$T/wsA" "$T/sppark-fixed" depnaive:naive
# Workspace B: the PRE-FIX sppark, one naive dependent (the bug) and one using watch_dep (fix B).
mkws "$T/wsB" "$T/sppark-prefix" depnaive:naive depwatch:watch

# Back-date every input an hour, so no copied file sits next to the first build's start time.
find "$T" -exec touch -h -d '-1 hour' {} +
for ws in wsA wsB; do
    out=$(build "$T/$ws") || { printf '%s\n' "$out" | grep -E '^error' -A3 | head -12; echo "FAIL $ws did not build"; exit 1; }
done
check "$([ "$(runs "$T/wsA" depnaive)" -ge 1 ] && [ "$(runs "$T/wsB" depnaive)" -ge 1 ] && [ "$(runs "$T/wsB" depwatch)" -ge 1 ] && echo 1)" \
      "first build: every dependent build script ran"
# Settle: a fresh target can re-run once more on cargo's own mtime race between a just-built registry dependency
# (`which`) and the build script compiled after it (seen 2 in 20: "the dependency `which` was rebuilt"). That is
# cargo, not the watch under test, so build until one pass re-runs nothing (at most 3), then count from there.
for _ in 1 2 3; do
    o=$( { build "$T/wsA"; build "$T/wsB"; } ); printf '%s' "$o" | grep -q 'Dirty' || break
done
nA=$(runs "$T/wsA" depnaive); nB=$(runs "$T/wsB" depnaive); nW=$(runs "$T/wsB" depwatch)
build "$T/wsA" >/dev/null; build "$T/wsB" >/dev/null
check "$([ "$(runs "$T/wsA" depnaive)" = "$nA" ] && [ "$(runs "$T/wsB" depnaive)" = "$nB" ] && [ "$(runs "$T/wsB" depwatch)" = "$nW" ] && echo 1)" \
      "a build with nothing changed re-runs nothing (the watch is not 'always rebuild')"

sleep 1.1   # past any coarse mtime granularity
echo '// touched by test-build-rerun.sh: a stream_t layout change' >> "$T/sppark-fixed/sppark/util/gpu_t.cuh"
echo '// touched by test-build-rerun.sh: a stream_t layout change' >> "$T/sppark-prefix/sppark/util/gpu_t.cuh"
outA=$(build "$T/wsA"); outB=$(build "$T/wsB")
check "$(printf '%s' "$outA" | grep -q 'Running `.*sppark-.*build-script-build' && echo 1)" \
      "A: the vendored sppark re-runs its own build script (libsppark_cuda.a) when gpu_t.cuh changes"
check "$([ "$(runs "$T/wsA" depnaive)" = $((nA + 1)) ] && echo 1)" \
      "A: ...and that re-runs a dependent -sys build script that watches only itself"
check "$([ "$(runs "$T/wsB" depwatch)" = $((nW + 1)) ] && echo 1)" \
      "B: a dependent using watch_dep re-runs even under the PRE-FIX sppark"
# The bug itself, reproduced -- asserted in both modes, so the test also shows it was real.
check "$([ "$(runs "$T/wsB" depnaive)" = "$nB" ] && ! printf '%s' "$outB" | grep -q 'Running `.*sppark-.*build-script-build' && echo 1)" \
      "the bug, reproduced: PRE-FIX sppark + a self-watching dependent rebuild NOTHING on the header change"

sleep 1.1
echo '// touched' >> "$T/sppark-fixed/sppark/util/all_gpus.cpp"
build "$T/wsA" >/dev/null
check "$([ "$(runs "$T/wsA" depnaive)" = $((nA + 2)) ] && echo 1)" \
      "A: a change to sppark's own all_gpus.cpp also rebuilds downstream"

# ---- static: every vendored build script reads DEP_* paths only through watch_dep ---------------------
raw=$(grep -n 'env::var("DEP_\|env::var_os("DEP_' vendor/*/build.rs 2>/dev/null)
check "$([ -z "$raw" ] && echo 1)" "no vendor/*/build.rs reads a DEP_* include path without watching it${raw:+: $raw}"
for f in vendor/risc0-circuit-rv32im-sys/build.rs vendor/risc0-circuit-keccak-sys/build.rs vendor/risc0-groth16-sys/build.rs; do
    check "$(grep -q 'println!("cargo:rerun-if-changed={p}")' "$f" && echo 1)" "$f: watch_dep emits rerun-if-changed"
done
check "$(python3 - vendor/risc0-build-kernel/src/lib.rs <<'PY'
import sys
s = open(sys.argv[1]).read()
c = s[s.index("pub fn compile(&mut self, output: &str)"):]
c = c[:c.index("match &self.kernel_type")]
print(1 if "for inc_dir in self.inc_dirs.iter()" in c else 0)
PY
)" "risc0-build-kernel watches include dirs for EVERY kernel type (C++ too), before dispatching"

if [ "$CONTROL" = 0 ] && [ "$fails" != 0 ]; then
    echo "cargo's stale reasons:"; grep -E 'Dirty|stale' "$T/fingerprint.log" | sed 's/^/    /' | head -40
fi
if [ "$CONTROL" = 1 ]; then echo "CONTROL: $fails failure(s)"; else echo "$fails failure(s)"; fi
if [ "$CONTROL" = 1 ]; then [ "$fails" != 0 ]; else [ "$fails" = 0 ]; fi
