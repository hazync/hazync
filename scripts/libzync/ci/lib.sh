# shellcheck shell=bash disable=SC2034
# Shared by the libzync CI drivers. Every check must PRINT a positive marker and the driver greps for it: a step that
# crashed, was skipped or produced nothing fails, it never passes by saying nothing (memory: absence is not green).
set -u
Z=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)        # the repo
T=$Z/prover/testdata/libzync
B=$Z/experimental/libzync-bench
fails=0
step() { echo; echo "=== $*"; }
pass() { echo "PASS: $*"; }
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }
# want <log> <fixed string> <what>: the log must contain the string
want() { if grep -aqF -- "$2" "$1"; then pass "$3"; else fail "$3 (no '$2' in $1)"; tail -5 "$1" | sed 's/^/    /'; fi; }
# none <log> <fixed string> <what>: the log must NOT contain it (used only beside a want on the same log)
none() { if grep -aqF -- "$2" "$1"; then fail "$3 ('$2' in $1)"; grep -aF -- "$2" "$1" | head -3 | sed 's/^/    /'; else pass "$3"; fi; }
finish() { echo; if [ "$fails" = 0 ]; then echo "$1: ALL PASSED"; else echo "$1: $fails FAILED"; exit 1; fi; }
