#!/usr/bin/env bash
# Differential test of the Rust transaction parsers on the soundness path against Bitcoin Core v28.0's own
# deserialiser (review round 2, R2-3 #2, engine/reviews/round2/r2_3_chunk_trie.md).
#
#   ./prover/methods/guest/test-txparse.sh            (a few minutes; needs g++, cargo, python3, git)
#   HAZYNC_CORE_SRC=<a clone of bitcoin/bitcoin at v28.0>  reuses that clone (read only); otherwise one is fetched
#                                                    into ${XDG_CACHE_HOME:-~/.cache}/hazync/core-v28.0 (shallow)
#
# Rust parsers (copied out of the shipped sources at build time, testsupport/txparse/rust/build.rs):
#   agg_lite.rs tx_io, prevout_fields   the core guest's trie keys, output script hashes and undo records
#   libzync facts::parse_tx             the verifiers' transaction facts
#   hints.rs / utreexo_roots.rs parse_tx the zAPP guests' stripped-transaction reader
# Core: testsupport/txparse/harness.cpp on Core v28.0 + patch 0101 (its SHA-256 hunks left out: they only route the
# same hash through the zkVM) with the guest's transaction / hash defines; plus the guest's raw-byte txid code
# (verify_input.cpp HAZYNC_RAW_TXID, extracted verbatim). The rules are in testsupport/txparse/compare.py.
#
# NOT COVERED: the guest is ILP32 (32-bit size_t), this runs LP64, so patch 0001 (int overloads for ILP32 Serialize,
# writing only) is not applied and size_t-width effects are not exercised; Core's ReadCompactSize returns uint64_t
# and checks MAX_SIZE before any narrowing, so the parse decisions compared here do not depend on it, but that is
# stated, not tested. Script semantics, signatures and amounts are out of scope (parsing only).
#
# POSITIVE CONTROLS (build.rs): `--nomarker` (tx_io without the BIP144 marker skip) must produce mismatches, or the
# run fails. `--lenient` (compact sizes in any encoding) must produce NONE: on the guest's path Core parses the same
# bytes first and traps on a non-canonical size, and the run records that claim by measurement.
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=$(cd "$HERE/../../.." && pwd)
T="$HERE/testsupport/txparse"
CORE_COMMIT=110183746150428e6385880c79f8c5733b1361ba
WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT

SRC=${HAZYNC_CORE_SRC:-${XDG_CACHE_HOME:-$HOME/.cache}/hazync/core-v28.0}
if [ ! -d "$SRC/.git" ]; then
    mkdir -p "$(dirname "$SRC")"
    git clone -q --depth 1 -b v28.0 https://github.com/bitcoin/bitcoin.git "$SRC"
fi
got=$(git -C "$SRC" rev-parse HEAD)
[ "$got" = "$CORE_COMMIT" ] || { echo "FATAL: $SRC is at $got, pinned $CORE_COMMIT" >&2; exit 1; }
# a clean copy of the committed tree (the clone may carry another build's edits in its working tree)
git -C "$SRC" archive --format=tar HEAD src | tar -x -C "$WORK"
git -C "$WORK" init -q
git -C "$WORK" apply --exclude=src/crypto/sha256.cpp "$REPO/patches/libzync/0101-core-libzync-levers.patch"
mkdir -p "$WORK/shim/config"; : > "$WORK/shim/config/bitcoin-config.h"; cp "$REPO"/coreshim/*.h "$WORK/shim/"
awk '/^#ifdef HAZYNC_RAW_TXID/{n++} n==1{print} n==1 && /^#endif/{exit}' "$HERE/verify_input.cpp" > "$WORK/seed.inc"
grep -q "hz_seed_hashes" "$WORK/seed.inc" || { echo "FATAL: the raw-txid code moved in verify_input.cpp" >&2; exit 1; }

C="$WORK/src"
echo "building Core's harness"
"${CXX:-g++}" -std=c++20 -O1 -w -DHAZYNC_HASHWRITER_BUF=1 -DHAZYNC_LAZY_TXID=1 -DHAZYNC_RAW_TXID=1 \
    -I "$WORK/shim" -I "$C" -I "$WORK" "$T/harness.cpp" \
    "$C/primitives/transaction.cpp" "$C/script/script.cpp" "$C/uint256.cpp" "$C/crypto/sha256.cpp" "$C/crypto/sha256_sse4.cpp" \
    "$C/crypto/ripemd160.cpp" "$C/crypto/sha512.cpp" "$C/crypto/hmac_sha512.cpp" "$C/util/strencodings.cpp" \
    "$C/crypto/hex_base.cpp" "$C/hash.cpp" -o "$WORK/harness"
echo "building the Rust parsers"
(cd "$T/rust" && CARGO_TARGET_DIR="$WORK/target" cargo build -q --release)
RS="$WORK/target/release/txparse-diff"

python3 "$T/gen_corpus.py" "$REPO" "$C/test/data" > "$WORK/corpus.txt"
"$WORK/harness" < "$WORK/corpus.txt" > "$WORK/core.out"
"$RS" < "$WORK/corpus.txt" > "$WORK/rust.out"
echo "the shipped parsers vs Core:"
rc=0
python3 "$T/compare.py" "$WORK/corpus.txt" "$WORK/core.out" "$WORK/rust.out" || rc=1

echo "control --nomarker (must be caught):"
"$RS" --nomarker < "$WORK/corpus.txt" > "$WORK/rust_nm.out"
if python3 "$T/compare.py" "$WORK/corpus.txt" "$WORK/core.out" "$WORK/rust_nm.out" --expect-mismatch | /usr/bin/grep "MISMATCHES"; then :
else echo "FAIL: the broken tx_io was not caught: the test cannot fail"; rc=1; fi
echo "control --lenient (non-canonical sizes; Core traps first on the guest's path: no mismatch expected):"
"$RS" --lenient < "$WORK/corpus.txt" > "$WORK/rust_len.out"
if python3 "$T/compare.py" "$WORK/corpus.txt" "$WORK/core.out" "$WORK/rust_len.out" | /usr/bin/grep "MISMATCHES"; then :
else echo "FAIL: a lenient compact-size reader changed a Core-accepted reading"; rc=1; fi

[ "$rc" = 0 ] && echo "TXPARSE OK" || echo "TXPARSE FAIL"
exit "$rc"
