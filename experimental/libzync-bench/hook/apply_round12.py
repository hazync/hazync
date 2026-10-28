"""Round 12 Core-tree patches, each behind its own #ifdef so a build without the define is unchanged.

Usage: python3 -I apply_round12.py <bitcoin-core dir>

HAZYNC_RAW_TXID (src/primitives/transaction.h; needs HAZYNC_LAZY_TXID)
    Adds CTransaction::HzSetHashes, so the guest's tx cache can seed the lazily computed txid and
    wtxid from the RAW bytes it just parsed instead of re-serialising the parsed transaction field by
    field (ComputeHash + ComputeWitnessHash measured ~37 M on block 962,000). See cached_tx in
    verify_input.cpp for when that is exact.

HAZYNC_PUBKEY_MEMO (src/pubkey.cpp)
    CPubKey::Verify parses the public key (secp256k1_ec_pubkey_parse, with its square root for a
    compressed key) on every call. Block 962,000 verifies 6,986 signatures over 2,099 distinct keys.
    A 4,096-slot memo keeps the parsed secp256k1_pubkey keyed by the FULL serialised key (compared byte
    for byte); only successful parses are stored, so a failure is recomputed every time.
"""
import sys

base = sys.argv[1] + "/src/"


def patch(rel, pairs):
    p = base + rel
    s = open(p).read()
    for old, new in pairs:
        assert s.count(old) == 1, f"{rel}: {s.count(old)} matches for {old[:60]!r}"
        s = s.replace(old, new)
    open(p, "w").write(s)
    print(f"patched {rel}")


patch("primitives/transaction.h", [
    ("""    const Wtxid& GetWitnessHash() const LIFETIMEBOUND
    {
        if (!m_whash_done) { m_witness_hash = ComputeWitnessHash(); m_whash_done = true; }
        return m_witness_hash;
    }
""", """    const Wtxid& GetWitnessHash() const LIFETIMEBOUND
    {
        if (!m_whash_done) { m_witness_hash = ComputeWitnessHash(); m_whash_done = true; }
        return m_witness_hash;
    }
#ifdef HAZYNC_RAW_TXID
    /** Seed both lazy hashes with values the caller computed from this transaction's exact bytes. */
    void HzSetHashes(const uint256& txid, const uint256& wtxid) const
    {
        hash = Txid::FromUint256(txid); m_hash_done = true;
        m_witness_hash = Wtxid::FromUint256(wtxid); m_whash_done = true;
    }
#endif
"""),
])

patch("pubkey.cpp", [
    ("#include <pubkey.h>\n", "#include <pubkey.h>\n#ifdef HAZYNC_PUBKEY_MEMO\n#include <cstring>\n#endif\n"),
    ("""    secp256k1_pubkey pubkey;
    secp256k1_ecdsa_signature sig;
    if (!secp256k1_ec_pubkey_parse(secp256k1_context_static, &pubkey, vch, size())) {
        return false;
    }
    if (!ecdsa_signature_parse_der_lax(&sig, vchSig.data(), vchSig.size())) {""",
     """    secp256k1_pubkey pubkey;
    secp256k1_ecdsa_signature sig;
#ifdef HAZYNC_PUBKEY_MEMO
    {
        struct HzKeySlot { unsigned char len; unsigned char key[65]; secp256k1_pubkey pk; };
        static HzKeySlot hz_keys[4096];
        const unsigned n = size();
        uint32_t i = (uint32_t(vch[1]) | uint32_t(vch[2]) << 8 | uint32_t(vch[3]) << 16) & 4095u;
        bool hit = false;
        HzKeySlot* empty = nullptr;
        for (int probe = 0; probe < 32; ++probe, i = (i + 1) & 4095u) {
            HzKeySlot& sl = hz_keys[i];
            if (sl.len == 0) { empty = &sl; break; }
            if (sl.len == n && std::memcmp(sl.key, vch, n) == 0) { pubkey = sl.pk; hit = true; break; }
        }
        if (!hit) {
            if (!secp256k1_ec_pubkey_parse(secp256k1_context_static, &pubkey, vch, n)) {
                return false;
            }
            if (empty) { empty->len = (unsigned char)n; std::memcpy(empty->key, vch, n); empty->pk = pubkey; }
        }
    }
#else
    if (!secp256k1_ec_pubkey_parse(secp256k1_context_static, &pubkey, vch, size())) {
        return false;
    }
#endif
    if (!ecdsa_signature_parse_der_lax(&sig, vchSig.data(), vchSig.size())) {"""),
])
