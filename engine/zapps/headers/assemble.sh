#!/bin/bash
# assemble.sh <out dir>: the header-chain zyncAPP's guest (prover/zyncapps/methods/headers, ROADMAP K1) for the engine
# (FEASIBILITY Q20). The zAPP's OWN source, copied; the only edits (each anchored, exactly once):
#   1) no `#![no_main]` / `risc0_zkvm::guest::entry!(main)`: an OpenVM std guest is an ordinary `fn main`;
#   2) `env::finalize()` after the last commit: the journal's sha256 becomes the proof's 32 public values (as the
#      product guest's edit 4).
# Dependencies: the engine platform (RISC Zero's guest API on OpenVM, linked as `risc0_zkvm`) and the engine's sha2 (on
# OpenVM's SHA-256 extension) in place of risc0-zkvm and RISC Zero's sha2 fork; serde and crypto-bigint as the zAPP's.
set -euo pipefail
OUT=$(readlink -m "$1"); HERE=$(cd "$(dirname "$0")" && pwd); ENGINE=$(cd "$HERE/../.." && pwd); REPO=$(cd "$ENGINE/.." && pwd)
SRC=$REPO/prover/zyncapps/methods/headers
rm -rf "$OUT"; mkdir -p "$OUT/src"
cp "$SRC/src/main.rs" "$SRC/src/mmr.rs" "$OUT/src/"
sed "s|@ENGINE@|$ENGINE|g" "$HERE/Cargo.toml.in" > "$OUT/Cargo.toml"
python3 - "$OUT/src/main.rs" <<'PY'
import sys
p = sys.argv[1]; s = open(p).read()
def rep(old, new):
    global s
    assert s.count(old) == 1, ("anchor not unique / missing", old[:60], s.count(old))
    s = s.replace(old, new)
rep("#![no_main]\n", "")
rep("risc0_zkvm::guest::entry!(main);\n", "")
rep("    env::commit(&(input_digest, s));\n}", "    env::commit(&(input_digest, s));\n    env::finalize();   // ENGINE: sha256(journal) as the public values\n}")
open(p, "w").write(s)
PY
# The pinned lock (reproduce/engine-locks, engine/ROADMAP.md 4.1): without it every build re-resolves crates.io and an
# id can move with a crate release. HZ_UNPINNED_LOCK=1 leaves it out (a manifest change that needs a new lock: make it,
# check the id, and pin it there).
[ "${HZ_UNPINNED_LOCK:-0}" = 1 ] || { cp "$REPO/reproduce/engine-locks/headers.Cargo.lock" "$OUT/Cargo.lock"; echo "  lock: reproduce/engine-locks/headers.Cargo.lock (pinned)"; }
echo "assembled $OUT from $SRC"
