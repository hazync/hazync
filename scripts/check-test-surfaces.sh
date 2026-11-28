#!/usr/bin/env bash
# Guard against the failure mode that actually bit us: a test that reports success while measuring
# nothing. Every instance below is something that really happened in this repo, silently, for weeks:
#
#   * guest-pure-fuzz DID NOT COMPILE — its build.rs extracts guest items verbatim and they had moved
#     to another file. docs/history/FUZZING.md still advertised it as clean at 700k+ execs. Nothing ran it.
#   * prover/test_bip68_real.sh printed "expect REJECT", got VALID, and exited 0. It had no assertion
#     at all, so it could not fail. Nothing ran it either.
#   * prover/test_cov_negatives.sh duplicated coverage that ci_negative_tests.sh already had, and was
#     stale — two scripts claiming one guarantee, one of them wrong.
#   * the checked-in block fixtures predated coin_height/coin_mtp, so they could not express an
#     in-block spend and quietly carried a stray UTXO leaf each.
#
# The common shape is a green signal from an instrument that is not connected to anything. These
# checks make that state fail the build instead.
set -uo pipefail
# || exit matters here: a failed cd would run every check against the WRONG directory, find no test
# surfaces, and pass — a false green from the very script whose job is to prevent false greens.
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
WF=.github/workflows
fail=0
bad() { printf 'FAIL %s\n' "$*"; fail=1; }
ok()  { printf '  ok   %s\n' "$*"; }

# Harnesses deliberately NOT in CI. Each needs a reason — an unexplained entry here is how a test
# quietly leaves the build. Keep this list short and justified.
declare -A EXCLUDED=(
  [build-release.sh]="release build tool, not a test"
)

# tests_run_via_path_include <crate dir>: prints the including crates and succeeds when every .rs file under
# <dir>/src that holds a #[test] is #[path]-included (the path resolved, not matched by name) by a crate under
# zapps/<name>/ whose zapps/<name>/test.sh runs `cargo test --release --manifest-path ./<crate>/Cargo.toml`.
tests_run_via_path_include() {
  local d=$1 f inc p crate z m found all=""
  while IFS= read -r f; do
    found=""
    while IFS= read -r inc; do
      while IFS= read -r p; do
        [ "$(realpath -m "$(dirname "$inc")/$p")" = "$(realpath -m "$f")" ] || continue
        crate=$(dirname "$inc"); while [ "$crate" != . ] && [ ! -f "$crate/Cargo.toml" ]; do crate=$(dirname "$crate"); done
        z=zapps/$(cut -d/ -f2 <<<"$crate"); m=".${crate#"$z"}/Cargo.toml"
        if [ "$crate" != . ] && grep -qsF -- "cargo test --release --manifest-path $m" "$z/test.sh" \
           && grep -rqsF 'zapps/*/test.sh' "$WF"; then found="$crate"; fi
      done < <(sed -n 's/^[[:space:]]*#\[path *= *"\([^"]*\)"\].*/\1/p' "$inc")
    done < <(git ls-files 'zapps/*.rs' | xargs grep -lsF -- "$(basename "$f")" 2>/dev/null)
    [ -n "$found" ] || return 1
    all="$all $found"
  done < <(grep -rls --include='*.rs' '#\[test\]' "$d/src")
  [ -n "$all" ] || return 1
  tr ' ' '\n' <<<"$all" | sed '/^$/d' | sort -u | paste -sd' ' -
}

echo "== 1. every Rust crate that has tests is run by CI =="
while read -r manifest; do
  d=$(dirname "$manifest")
  grep -rqs --include='*.rs' '#\[test\]' "$d/src" 2>/dev/null || continue
  # VENDORED UPSTREAM IS NOT OUR TEST SURFACE. vendor/risc0-zkvm is risc0 3.1.0 verbatim apart from
  # two files we changed — the recursion fold, rebalanced from linear to a tree, and the split that
  # lets assembly run from externally proved segment receipts. Its #[test]s test risc0. Running them
  # here would report on upstream's health and say nothing about this repo, and a failure would not
  # be actionable in it.
  #
  # The parts we DID change are covered where it counts: the gates in prover/ assert that the
  # rebalanced fold and the split assembly produce a byte-identical receipt, across eight execution
  # paths. That is a stronger check on our two edits than upstream's unit tests would be.
  case "$d" in
    vendor/*) ok "$d skipped — vendored upstream, not our test surface (our edits are gated by receipt equality)"; continue ;;
    # history/ is the record of the retired CORE design (history/README.md): kept, not built or run.
    history/*) ok "$d skipped — history/, the retired CORE design's record, not built"; continue ;;
    # The engine guests' platform crate builds for the zkVM target only (natively its sha2 shim does not compile). Its
    # #[test]s are upstream's serde tests: serializer.rs and deserializer.rs are byte-identical to engine/journal's,
    # whose tests CI runs (engine-tests), and the round trips of its serde/mod.rs are run there too.
    engine/platform/risc0-zkvm)
      if cmp -s "$d/src/serde/serializer.rs" engine/journal/src/serde/serializer.rs && cmp -s "$d/src/serde/deserializer.rs" engine/journal/src/serde/deserializer.rs \
         && grep -rqs -- "--manifest-path engine/journal/Cargo.toml" "$WF"; then
        ok "$d skipped — a zkVM-target crate; its serde tests run as engine/journal's (the same source)"
      else bad "$d: its serde source is no longer engine/journal's, or no workflow runs engine/journal's tests"; fi
      continue ;;
    # A zAPP is a module of its own (zapps/<name>/, docs/libzync/ZYNCAPPS.md §9) and runs its tests from its own
    # directory: the zapps job runs every zapps/<name>/test.sh, which must test this crate by its manifest.
    zapps/*)
      z=zapps/$(cut -d/ -f2 <<<"$d"); m=".${d#"$z"}/Cargo.toml"
      # wasm_crate_test <dir> (scripts/zapp-checks.sh) runs `cargo test --release --locked` on that crate's engine-graph lock
      if { grep -qsF -- "cargo test --release --manifest-path $m" "$z/test.sh" || grep -qsxE -- "\s*wasm_crate_test ${m%/Cargo.toml}\s*" "$z/test.sh"; } \
         && grep -rqsF 'zapps/*/test.sh' "$WF"; then
        ok "$d has tests and CI runs them ($z/test.sh, the zapps job)"
      else bad "$d contains #[test] but $z/test.sh does not run them (cargo test --release --manifest-path $m), or no workflow runs zapps/*/test.sh"; fi
      continue ;;
  esac
  # A GUEST'S SHARED FILE IS TESTED WHERE IT IS INCLUDED. A guest crate cannot run `cargo test` natively, so a rules
  # file such as prover/zyncapps/methods/utreexo-tip/src/step.rs is #[path]-included by a native crate in the zAPP's
  # own module (zapps/utreexo/tip-step), whose test.sh runs it; the file's own #[cfg(test)] module runs THERE. Accepted
  # only when EVERY file of $d that holds a #[test] is so included by a zapps/ crate that its zAPP's test.sh runs.
  if covered=$(tests_run_via_path_include "$d"); then ok "$d has tests and CI runs them (#[path]-included by $covered)"
  elif grep -rqs -- "--manifest-path $d/Cargo.toml" "$WF"; then ok "$d has tests and CI runs them"
  elif grep -rqsxE -- "\s*wasm_crate_test $d\s*" "$WF"; then ok "$d has tests and CI runs them (wasm_crate_test, its engine-graph lock)"
  # prover/host's tests need a libzync guest, so they run inside the reproducible container, from
  # scripts/libzync/ci/build-hosts.sh (named by the libzync-hosts job), as `cargo test -p host`.
  elif [ "$d" = prover/host ] && grep -qs 'cargo test .*-p host' scripts/libzync/ci/build-hosts.sh \
       && grep -rqs 'scripts/libzync/ci/build-hosts.sh' "$WF"; then ok "$d has tests and CI runs them (build-hosts.sh, libzync mode)"
  else bad "$d contains #[test] but no workflow runs it — a rotting test surface"; fi
done < <(git ls-files '*/Cargo.toml' | grep -v '/fuzz/Cargo.toml$')

echo "== 2. every test harness is either in CI or explicitly excluded with a reason =="
# A harness counts as "in CI" if a workflow names it, OR if a shell harness that a workflow names
# invokes it. The second case is real: chainparams_check.cpp is compiled and run by
# chainparams_check.sh, which CI calls — the .cpp itself appears in no workflow.
in_ci() {
  grep -rqs "$1" "$WF" && return 0
  while read -r sh; do
    grep -qs "$1" "$sh" || continue
    grep -rqs "$(basename "$sh")" "$WF" && return 0
  done < <(git ls-files '*.sh')
  return 1
}
while read -r h; do
  b=$(basename "$h")
  case "$b" in make_negative_tests.py|fetch_block*.py) continue ;; esac   # generators, not tests
  if in_ci "$b"; then ok "$b runs in CI"
  elif [ -n "${EXCLUDED[$b]:-}" ]; then ok "$b excluded — ${EXCLUDED[$b]}"
  else bad "$b is neither run by CI nor listed as excluded — nothing would notice if it broke"; fi
# C++ harnesses are enumerated too. They were not, which made this section blind to them: the
# EXCLUDED entry for the FFI suite claimed it was "invoked directly by the FFI CI step" and NOTHING
# checked that claim — the basename never reached this loop, so the entry was never even consulted.
# Deleting the CI step would have gone unnoticed while the exclusion list went on asserting it ran.
done < <(git ls-files 'prover/*.sh' 'coordinator/*fuzz*.py' 'prover/testdata/*.cpp')

echo "== 3. every CI-run harness can actually FAIL =="
# A script with no failure path is decoration. test_bip68_real.sh printed a contradiction and exited 0.
while read -r h; do
  b=$(basename "$h")
  grep -rqs "$b" "$WF" || continue
  # Match conditional exits too: `sys.exit(0 if fails == 0 else 1)` is a failure path, and an earlier
  # version of this check called it decoration — a guard that cries wolf is a guard people switch off.
  if grep -qE 'exit 1|exit \$|fail=1|set -e|sys\.exit\([^)]*1' "$h"; then ok "$b has a failure path"
  else bad "$b runs in CI but contains no way to exit non-zero — it cannot fail"; fi
done < <(git ls-files 'prover/*.sh' 'coordinator/*fuzz*.py' 'scripts/*.sh')

echo "== 4. fuzz positive controls are still able to detect their bug class =="
# A fuzzer whose control has gone quiet is blind, and its clean runs mean nothing. seam_fuzz --control
# already exits 1 when it finds NOTHING; assert CI actually runs it in that mode.
if grep -rqs 'seam_fuzz.py --control' "$WF"; then ok "seam_fuzz control runs in CI"
else bad "seam_fuzz.py --control is not in CI — nothing proves the seam fuzzer can still catch H9"; fi

echo "== 5. block fixtures carry the fields the host needs =="
# Without coin_height a fixture cannot express an in-block spend, and the block still proves VALID —
# it just silently strands a UTXO leaf. Exactly the drift that hid for weeks.
for f in $(git ls-files 'prover/block_*.json' | grep -v badwit); do
  python3 - "$f" <<'PY' || fail=1
import json,sys
f=sys.argv[1]; d=json.load(open(f))
missing=[k for k in ("recent_times","txs") if k not in d]
for t in d.get("txs",[]):
    for p in t.get("prevouts",[]):
        missing += [k for k in ("coin_height","coin_is_coinbase","coin_mtp") if k not in p]
        break
    break
if missing: print(f"FAIL {f} missing {sorted(set(missing))} — regenerate with prover/fetch_block_rpc.py"); sys.exit(1)
print(f"  ok   {f} carries coin_height/coin_mtp/recent_times")
PY
done

echo "== 6. every zAPP module has its own tests, run by CI and able to fail =="
# zapps/<name>/test.sh is the one place a zAPP's tests are run from (ZYNCAPPS.md §9); a zAPP without one is untested.
for z in $(git ls-files 'zapps/*' | cut -d/ -f2 | sort -u); do
  t=zapps/$z/test.sh
  if [ ! -f "$t" ]; then bad "zapps/$z has no test.sh — a zAPP module whose tests nothing runs"
  elif ! grep -qE 'set -e|exit 1' "$t"; then bad "$t contains no way to exit non-zero — it cannot fail"
  elif ! grep -rqsF 'zapps/*/test.sh' "$WF"; then bad "$t exists but no workflow runs zapps/*/test.sh"
  else ok "$t runs in CI (the zapps job) and has a failure path"; fi
done

echo
[ "$fail" -eq 0 ] && echo "test surfaces are all connected to something." \
                  || { echo "A test surface is disconnected — it would report green while measuring nothing."; exit 1; }
