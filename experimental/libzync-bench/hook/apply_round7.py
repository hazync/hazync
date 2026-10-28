"""Round 7 Core-tree patches, each behind its own #ifdef so a build without the define is unchanged.

Usage: python3 -I apply_round7.py <bitcoin-core dir>

HAZYNC_HASHWRITER_BUF (src/hash.h)
    HashWriter forwards every serialised field -- 1 to 36 bytes -- to CSHA256::Write as a separate
    call. On block 962,000 that per-call work was 93 M cycles of CSHA256::Write SELF time against 26 M
    of actual accelerator. HashWriter now stages writes in a 512-byte inline buffer and hands the
    hasher whole slabs. A fixed buffer, not a vector: the guest heap is a bump allocator that never
    frees, so a growable buffer per HashWriter would leak a copy of every message hashed.
    Copying a HashWriter (Core does, for tagged-hash midstates) copies the staged bytes too, so the
    copy hashes exactly what it would have before.

HAZYNC_LAZY_TXID (src/primitives/transaction.{h,cpp})
    CTransaction computes txid AND wtxid in its constructor. Chunks need only the wtxid, and
    transactions without witnesses reuse the txid for it. Both become computed on first use. Same
    values, same accessors; operator== goes through GetHash().

HAZYNC_H160_MEMO (src/script/interpreter.cpp)
    OP_HASH160 over a 33-byte compressed public key is memoised for the guest run. Block 962,000 has
    6,986 signatures over 2,099 distinct keys, and each P2WPKH / P2PKH spend hashes its key (SHA-256 +
    software RIPEMD-160). A memo hit returns the same 20 bytes the hash would. A table slot keeps the
    full 33-byte key and is checked byte for byte.
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


patch("hash.h", [
    ("""class HashWriter
{
private:
    CSHA256 ctx;

public:
    void write(Span<const std::byte> src)
    {
        ctx.Write(UCharCast(src.data()), src.size());
    }
""", """class HashWriter
{
private:
    CSHA256 ctx;
#ifdef HAZYNC_HASHWRITER_BUF
    unsigned char m_hzbuf[512];
    size_t m_hzlen{0};
    void hz_flush()
    {
        if (m_hzlen) {
            ctx.Write(m_hzbuf, m_hzlen);
            m_hzlen = 0;
        }
    }
#endif

public:
    void write(Span<const std::byte> src)
    {
#ifdef HAZYNC_HASHWRITER_BUF
        const unsigned char* p = UCharCast(src.data());
        const size_t n = src.size();
        if (n > sizeof(m_hzbuf) - m_hzlen) {
            hz_flush();
            if (n >= sizeof(m_hzbuf)) {
                ctx.Write(p, n);
                return;
            }
        }
        for (size_t i = 0; i < n; ++i) m_hzbuf[m_hzlen + i] = p[i];
        m_hzlen += n;
#else
        ctx.Write(UCharCast(src.data()), src.size());
#endif
    }
"""),
    ("""    uint256 GetHash() {
        uint256 result;
        ctx.Finalize(result.begin());""", """    uint256 GetHash() {
        uint256 result;
#ifdef HAZYNC_HASHWRITER_BUF
        hz_flush();
#endif
        ctx.Finalize(result.begin());"""),
    ("""    uint256 GetSHA256() {
        uint256 result;
        ctx.Finalize(result.begin());""", """    uint256 GetSHA256() {
        uint256 result;
#ifdef HAZYNC_HASHWRITER_BUF
        hz_flush();
#endif
        ctx.Finalize(result.begin());"""),
])

patch("primitives/transaction.h", [
    ("""    const bool m_has_witness;
    const Txid hash;
    const Wtxid m_witness_hash;
""", """    const bool m_has_witness;
#ifdef HAZYNC_LAZY_TXID
    mutable Txid hash;
    mutable Wtxid m_witness_hash;
    mutable bool m_hash_done{false};
    mutable bool m_whash_done{false};
#else
    const Txid hash;
    const Wtxid m_witness_hash;
#endif
"""),
    ("""    const Txid& GetHash() const LIFETIMEBOUND { return hash; }
    const Wtxid& GetWitnessHash() const LIFETIMEBOUND { return m_witness_hash; };
""", """#ifdef HAZYNC_LAZY_TXID
    const Txid& GetHash() const LIFETIMEBOUND
    {
        if (!m_hash_done) { hash = ComputeHash(); m_hash_done = true; }
        return hash;
    }
    const Wtxid& GetWitnessHash() const LIFETIMEBOUND
    {
        if (!m_whash_done) { m_witness_hash = ComputeWitnessHash(); m_whash_done = true; }
        return m_witness_hash;
    }
#else
    const Txid& GetHash() const LIFETIMEBOUND { return hash; }
    const Wtxid& GetWitnessHash() const LIFETIMEBOUND { return m_witness_hash; };
#endif
"""),
    ("""        return a.hash == b.hash;""", """        return a.GetHash() == b.GetHash();"""),
    ("""        return a.hash != b.hash;""", """        return a.GetHash() != b.GetHash();"""),
])

patch("primitives/transaction.cpp", [
    ("""    if (!HasWitness()) {
        return Wtxid::FromUint256(hash.ToUint256());
    }""", """    if (!HasWitness()) {
        return Wtxid::FromUint256(GetHash().ToUint256());
    }"""),
    ("""CTransaction::CTransaction(const CMutableTransaction& tx) : vin(tx.vin), vout(tx.vout), version{tx.version}, nLockTime{tx.nLockTime}, m_has_witness{ComputeHasWitness()}, hash{ComputeHash()}, m_witness_hash{ComputeWitnessHash()} {}
CTransaction::CTransaction(CMutableTransaction&& tx) : vin(std::move(tx.vin)), vout(std::move(tx.vout)), version{tx.version}, nLockTime{tx.nLockTime}, m_has_witness{ComputeHasWitness()}, hash{ComputeHash()}, m_witness_hash{ComputeWitnessHash()} {}""",
     """#ifdef HAZYNC_LAZY_TXID
CTransaction::CTransaction(const CMutableTransaction& tx) : vin(tx.vin), vout(tx.vout), version{tx.version}, nLockTime{tx.nLockTime}, m_has_witness{ComputeHasWitness()} {}
CTransaction::CTransaction(CMutableTransaction&& tx) : vin(std::move(tx.vin)), vout(std::move(tx.vout)), version{tx.version}, nLockTime{tx.nLockTime}, m_has_witness{ComputeHasWitness()} {}
#else
CTransaction::CTransaction(const CMutableTransaction& tx) : vin(tx.vin), vout(tx.vout), version{tx.version}, nLockTime{tx.nLockTime}, m_has_witness{ComputeHasWitness()}, hash{ComputeHash()}, m_witness_hash{ComputeWitnessHash()} {}
CTransaction::CTransaction(CMutableTransaction&& tx) : vin(std::move(tx.vin)), vout(std::move(tx.vout)), version{tx.version}, nLockTime{tx.nLockTime}, m_has_witness{ComputeHasWitness()}, hash{ComputeHash()}, m_witness_hash{ComputeWitnessHash()} {}
#endif"""),
])

patch("script/interpreter.cpp", [
    ("""                    else if (opcode == OP_HASH160)
                        CHash160().Write(vch).Finalize(vchHash);""", """                    else if (opcode == OP_HASH160) {
#ifdef HAZYNC_H160_MEMO
                        /* Memo for 33-byte compressed public keys; 4096 slots, linear probing.
                         * Each slot keeps the full key and is compared byte for byte. */
                        struct HzSlot { bool used; unsigned char key[33]; unsigned char h[20]; };
                        static HzSlot hz_table[4096];
                        bool done = false;
                        if (vch.size() == 33) {
                            uint32_t i = (uint32_t(vch[1]) | uint32_t(vch[2]) << 8 | uint32_t(vch[3]) << 16) & 4095u;
                            for (int probe = 0; probe < 32; ++probe, i = (i + 1) & 4095u) {
                                HzSlot& sl = hz_table[i];
                                if (!sl.used) {
                                    CHash160().Write(vch).Finalize(vchHash);
                                    sl.used = true;
                                    std::memcpy(sl.key, vch.data(), 33);
                                    std::memcpy(sl.h, vchHash.data(), 20);
                                    done = true;
                                    break;
                                }
                                if (std::memcmp(sl.key, vch.data(), 33) == 0) {
                                    std::memcpy(vchHash.data(), sl.h, 20);
                                    done = true;
                                    break;
                                }
                            }
                        }
                        if (!done) CHash160().Write(vch).Finalize(vchHash);
#else
                        CHash160().Write(vch).Finalize(vchHash);
#endif
                    }"""),
])
