"""Round 11 Core-tree patch: the round-8 legacy sighash cache follows the per-transaction generation.

Usage: python3 -I apply_round11.py <bitcoin-core dir>

HAZYNC_LEGACY_SIGHASH keys its one-entry cache on the transaction's vin/vout buffer addresses. That was
sound under risc0's bump allocator (no address is ever reused), but with a freeing allocator
(HAZYNC_TLSF) a later transaction can land at the same addresses with the same sizes, version and
locktime, and a stale hit would hash the WRONG bytes. That fails closed (the signature does not verify,
so a valid block is rejected), but it is wrong. Under HAZYNC_AGG_LITE the chunk bumps hz_tx_generation
(verify_input.cpp) before every transaction, and the cache now also keys on it.
"""
import sys

p = sys.argv[1] + "/src/script/interpreter.cpp"
s = open(p).read()
assert "hz_tx_generation" not in s, "already applied"
for old, new in [
    ("""    CSHA256 mid;                      //!< state after version | n | blank[0 .. 41*mid_k)
    size_t mid_k{0};
};""", """    CSHA256 mid;                      //!< state after version | n | blank[0 .. 41*mid_k)
    size_t mid_k{0};
    unsigned gen{~0u};                //!< hz_tx_generation when filled (HAZYNC_AGG_LITE)
};
#ifdef HAZYNC_AGG_LITE
extern "C" unsigned hz_tx_generation;
#endif"""),
    ("""        c.locktime != uint32_t(txTo.nLockTime)) {""", """        c.locktime != uint32_t(txTo.nLockTime)
#ifdef HAZYNC_AGG_LITE
        || c.gen != hz_tx_generation
#endif
        ) {
#ifdef HAZYNC_AGG_LITE
        c.gen = hz_tx_generation;
#endif"""),
]:
    assert s.count(old) == 1, f"{s.count(old)} matches for {old[:50]!r}"
    s = s.replace(old, new)
open(p, "w").write(s)
print(f"patched {p}")
