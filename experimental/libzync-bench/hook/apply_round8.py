"""Round 8 Core-tree patch, behind its own #ifdef so a build without the define is unchanged.

Usage: python3 -I apply_round8.py <bitcoin-core dir>

HAZYNC_LEGACY_SIGHASH (src/script/interpreter.cpp)
    A legacy (pre-segwit) SIGHASH_ALL signature hash is double-SHA256 over the whole transaction
    re-serialised with every scriptSig blanked except the signed input's, which carries scriptCode:

        version | n | in[0] .. in[n-1] | outputs | locktime | hashtype

    Every blanked input is the same 41 bytes (prevout, 0x00, nSequence) whichever input is signed.
    So per transaction we serialise ONCE: the 41-byte blanked inputs and the outputs + locktime. Per
    signature we then hash slices of those bytes instead of re-serialising field by field, and the
    prefix (version | n | in[0..nIn) blanked) comes from a SHA-256 midstate that walks forward as
    nIn grows, so it is hashed once per transaction rather than once per input.

    On block 962,000 one 400-input legacy transaction re-serialised and hashed its 59 KB once per
    input (~24 MB). This halves the bytes hashed and removes the per-field serialisation.

    Fast path only for !ANYONECANPAY && !SINGLE && !NONE, the exact case where the serialiser emits
    the layout above; every other hash type takes Core's code unchanged. The signed input's scriptCode
    goes through Core's own SerializeScriptCode (OP_CODESEPARATOR removal included).

    The per-transaction cache is ONE static entry keyed by the transaction's vin/vout buffer
    addresses plus its sizes, version and locktime. The guest heap is a bump allocator that never
    frees, so a vin buffer address is never reused for a different transaction during a run. A miss
    just rebuilds; out-of-order nIn rewinds the midstate.
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


patch("script/interpreter.cpp", [
    ("""/** Compute the (single) SHA256 of the concatenation of all prevouts of a tx. */
template <class T>
uint256 GetPrevoutsSHA256(const T& txTo)""", """#ifdef HAZYNC_LEGACY_SIGHASH
/** Stream adaptors: one feeds a CSHA256, one appends to a byte vector. */
struct HzShaSink {
    CSHA256& h;
    void write(Span<const std::byte> src) { h.Write(UCharCast(src.data()), src.size()); }
    template <typename T> HzShaSink& operator<<(const T& obj) { ::Serialize(*this, obj); return *this; }
};
struct HzVecSink {
    std::vector<unsigned char>& v;
    void write(Span<const std::byte> src) { v.insert(v.end(), UCharCast(src.data()), UCharCast(src.data()) + src.size()); }
    template <typename T> HzVecSink& operator<<(const T& obj) { ::Serialize(*this, obj); return *this; }
};

struct HzLegacyCache {
    const void* vin_data{nullptr};
    const void* vout_data{nullptr};
    size_t n_in{0}, n_out{0};
    uint32_t version{0}, locktime{0};
    std::vector<unsigned char> blank; //!< 41 bytes per input: prevout | 0x00 | nSequence
    std::vector<unsigned char> tail;  //!< compactsize(n_out) | outputs | locktime
    CSHA256 mid;                      //!< state after version | n | blank[0 .. 41*mid_k)
    size_t mid_k{0};
};

/** SIGHASH_ALL legacy sighash from preserialised bytes; same digest as Core's serialiser. */
template <class T>
uint256 HzLegacySighashAll(const CScript& scriptCode, const T& txTo, unsigned int nIn, int nHashType)
{
    static HzLegacyCache c;
    const size_t n = txTo.vin.size();
    if (c.vin_data != (const void*)txTo.vin.data() || c.vout_data != (const void*)txTo.vout.data() ||
        c.n_in != n || c.n_out != txTo.vout.size() || c.version != uint32_t(txTo.version) ||
        c.locktime != uint32_t(txTo.nLockTime)) {
        c.vin_data = txTo.vin.data();
        c.vout_data = txTo.vout.data();
        c.n_in = n;
        c.n_out = txTo.vout.size();
        c.version = uint32_t(txTo.version);
        c.locktime = uint32_t(txTo.nLockTime);
        c.blank.clear();
        c.blank.reserve(41 * n);
        HzVecSink bs{c.blank};
        for (const auto& in : txTo.vin) {
            bs << in.prevout;
            bs << CScript();
            bs << in.nSequence;
        }
        assert(c.blank.size() == 41 * n);
        c.tail.clear();
        HzVecSink ts{c.tail};
        ::WriteCompactSize(ts, txTo.vout.size());
        for (const auto& out : txTo.vout) ts << out;
        ts << txTo.nLockTime;
        c.mid_k = n + 1; // force a rewind below
    }
    if (c.mid_k > nIn) {
        c.mid.Reset();
        HzShaSink ms{c.mid};
        ms << txTo.version;
        ::WriteCompactSize(ms, n);
        c.mid_k = 0;
    }
    if (c.mid_k < nIn) {
        c.mid.Write(c.blank.data() + 41 * c.mid_k, 41 * (nIn - c.mid_k));
        c.mid_k = nIn;
    }
    CSHA256 h = c.mid;
    HzShaSink hs{h};
    const unsigned char* own = c.blank.data() + 41 * nIn;
    h.Write(own, 36); // prevout
    CTransactionSignatureSerializer<T>(txTo, scriptCode, nIn, nHashType).SerializeScriptCode(hs);
    h.Write(own + 37, 4); // nSequence
    h.Write(own + 41, 41 * (n - nIn - 1));
    h.Write(c.tail.data(), c.tail.size());
    unsigned char ht[4];
    WriteLE32(ht, uint32_t(nHashType));
    h.Write(ht, 4);
    uint256 result;
    h.Finalize(result.begin());
    CSHA256().Write(result.begin(), CSHA256::OUTPUT_SIZE).Finalize(result.begin());
    return result;
}
#endif /* HAZYNC_LEGACY_SIGHASH */

/** Compute the (single) SHA256 of the concatenation of all prevouts of a tx. */
template <class T>
uint256 GetPrevoutsSHA256(const T& txTo)"""),
    ("""    // Wrapper to serialize only the necessary parts of the transaction being signed
    CTransactionSignatureSerializer<T> txTmp(txTo, scriptCode, nIn, nHashType);
""", """#ifdef HAZYNC_LEGACY_SIGHASH
    if (!(nHashType & SIGHASH_ANYONECANPAY) && (nHashType & 0x1f) != SIGHASH_SINGLE &&
        (nHashType & 0x1f) != SIGHASH_NONE) {
        return HzLegacySighashAll(scriptCode, txTo, nIn, nHashType);
    }
#endif
    // Wrapper to serialize only the necessary parts of the transaction being signed
    CTransactionSignatureSerializer<T> txTmp(txTo, scriptCode, nIn, nHashType);
"""),
])
