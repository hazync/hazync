// Core's side of the transaction-parser differential test (test-txparse.sh): Bitcoin Core v28.0's own
// deserialiser, from the tree the guest compiles (patches 0001 + 0101 applied; 0101's SHA-256 hunks left out, they
// only route the same hash through the zkVM's circuit), with the guest's defines for transaction.h and hash.h.
//
// Input lines: "<K> <hex>", K = T (a transaction as a block carries it) or P (a serialised std::vector<CTxOut>, a run's
// prevouts blob). For T it parses twice, TX_WITH_WITNESS (as every guest entry point does) and TX_NO_WITNESS (what the
// hints / utreexo zAPPs' stripped form is), and prints the fields the Rust parsers extract. A read past the end throws
// (the guest's MiniReader traps instead: either way no proof); trailing bytes are left unread, as in the guest.
#include <primitives/transaction.h>
#include <serialize.h>
#include <hash.h>
#include <span.h>
#include <uint256.h>
#include <cstdio>
#include <cstring>
#include <iostream>
#include <string>
#include <vector>

struct Reader {
    const std::byte* p;
    const std::byte* e;
    void read(Span<std::byte> dst) {
        if (dst.size() > static_cast<size_t>(e - p)) throw std::ios_base::failure("end of data");
        if (dst.size()) { std::memcpy(dst.data(), p, dst.size()); p += dst.size(); }
    }
    void ignore(size_t n) { if (n > static_cast<size_t>(e - p)) throw std::ios_base::failure("end of data"); p += n; }
    template <typename T> Reader& operator>>(T&& obj) { ::Unserialize(*this, obj); return *this; }
};

// The guest's txid / wtxid from the raw bytes (verify_input.cpp, HAZYNC_RAW_TXID), extracted verbatim by the driver.
#include "seed.inc"

static std::string hex(const unsigned char* b, size_t n) {
    static const char* d = "0123456789abcdef";
    std::string s; s.reserve(2 * n);
    for (size_t i = 0; i < n; i++) { s += d[b[i] >> 4]; s += d[b[i] & 15]; }
    return s;
}
static std::string hex(const uint256& h) { return hex(h.begin(), 32); }
static std::string hexs(const CScript& s) { return hex(s.data(), s.size()); }
static bool unhex(const std::string& s, std::vector<unsigned char>& out) {
    if (s.size() % 2) return false;
    auto v = [](char c) -> int { if (c >= '0' && c <= '9') return c - '0'; if (c >= 'a' && c <= 'f') return c - 'a' + 10; return -1; };
    out.resize(s.size() / 2);
    for (size_t i = 0; i < out.size(); i++) { int a = v(s[2 * i]), b = v(s[2 * i + 1]); if (a < 0 || b < 0) return false; out[i] = (unsigned char)(a * 16 + b); }
    return true;
}

// One parse: "ok <consumed> <txid> <wtxid> <seeded txid> <seeded wtxid> <has_witness> <nin> <outpoints> <nout> <outputs>"
// (seeded = the guest's raw-byte hashes, "-" unless every byte was consumed), or "err".
template <typename Wrap>
static std::string tx_line(const std::vector<unsigned char>& b, Wrap wrap, bool seed) {
    Reader r{reinterpret_cast<const std::byte*>(b.data()), reinterpret_cast<const std::byte*>(b.data()) + b.size()};
    CMutableTransaction m;
    try { r >> wrap(m); } catch (const std::exception&) { return "err"; }
    const size_t used = r.p - reinterpret_cast<const std::byte*>(b.data());
    CTransaction plain{m};
    std::string out = "ok " + std::to_string(used) + " " + hex(plain.GetHash().ToUint256()) + " " + hex(plain.GetWitnessHash().ToUint256());
    if (seed && used == b.size()) {
        CTransaction s{m};
        hz_seed_hashes(s, b.data(), (unsigned)b.size());
        out += " " + hex(s.GetHash().ToUint256()) + " " + hex(s.GetWitnessHash().ToUint256());
    } else {
        out += " - -";
    }
    out += std::string(" ") + (plain.HasWitness() ? "1" : "0") + " " + std::to_string(m.vin.size()) + " ";
    std::string ins;
    for (const auto& i : m.vin) { if (!ins.empty()) ins += ","; ins += hex(i.prevout.hash.ToUint256()) + ":" + std::to_string(i.prevout.n); }
    out += (ins.empty() ? "." : ins) + " " + std::to_string(m.vout.size()) + " ";
    std::string outs;
    for (const auto& o : m.vout) { if (!outs.empty()) outs += ","; outs += std::to_string((unsigned long long)(uint64_t)o.nValue) + ":" + (o.scriptPubKey.empty() ? "." : hexs(o.scriptPubKey)); }
    out += outs.empty() ? "." : outs;
    return out;
}

int main() {
    std::ios::sync_with_stdio(false);
    std::string line;
    std::vector<unsigned char> b;
    while (std::getline(std::cin, line)) {
        if (line.size() < 2) continue;
        const char k = line[0];
        if (!unhex(line.substr(2), b)) { std::cout << "bad\n"; continue; }
        if (k == 'T') {
            std::cout << "T W " << tx_line(b, [](CMutableTransaction& m) { return TX_WITH_WITNESS(m); }, true) << "\n";
            std::cout << "T N " << tx_line(b, [](CMutableTransaction& m) { return TX_NO_WITNESS(m); }, false) << "\n";
        } else if (k == 'P') {
            Reader r{reinterpret_cast<const std::byte*>(b.data()), reinterpret_cast<const std::byte*>(b.data()) + b.size()};
            std::vector<CTxOut> v;
            try { r >> v; } catch (const std::exception&) { std::cout << "P err\n"; continue; }
            std::string outs;
            for (const auto& o : v) { if (!outs.empty()) outs += ","; outs += std::to_string((unsigned long long)(uint64_t)o.nValue) + ":" + (o.scriptPubKey.empty() ? "." : hexs(o.scriptPubKey)); }
            std::cout << "P ok " << v.size() << " " << (outs.empty() ? "." : outs) << "\n";
        } else {
            std::cout << "bad\n";
        }
    }
}
