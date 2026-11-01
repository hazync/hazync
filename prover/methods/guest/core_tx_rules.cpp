// Bitcoin Core's own consensus/tx_verify.cpp, unmodified, compiled into the libzync guest.
//
// IsFinalTx, Consensus::CheckTxInputs, SequenceLocks and GetTransactionSigOpCost below are Core's code,
// not copies of it. Core's file needs two things the guest does not have: a coin database and a block
// index. This file gives it minimal stand-ins, under Core's names, and nothing else.
//
// How: Core's headers are guarded out by their include guards, the stand-ins take their place, and
// the .cpp is #included. The macros rename the stand-ins so that nothing here can collide with the real
// CBlockIndex in chain.cpp, and they apply to this translation unit only.
//
// A chunk holds a run of a transaction's inputs, not always all of them. Each input belongs to exactly one
// run, so each run applies Core's per-coin rules to its own inputs and the runs together cover the
// transaction. Coin existence is not checked here: it is what the ECMH total establishes at H.

#include <consensus/amount.h>
#include <consensus/consensus.h>
#include <consensus/validation.h>
#include <consensus/tx_check.h>
#include <primitives/transaction.h>
#include <script/interpreter.h>
#include <tinyformat.h>
#include <util/check.h>
#include <cassert>
#include <cstdint>
#include <string>
#include <vector>

// The reject reason of the last failed rule, Core's own string, for the log. The verdict never reads it.
std::string hz_last_reject;

#define BITCOIN_CHAIN_H
#define BITCOIN_COINS_H
#define CBlockIndex HzBlockIndex
#define CCoinsViewCache HzCoinsView
#define Coin HzCoin

// The fields of Core's Coin that tx_verify.cpp reads, declared as in coins.h. `out` refers to the prevout
// the caller already parsed, so a spent script is not copied per input.
class HzCoin {
public:
    const CTxOut& out;
    unsigned int fCoinBase : 1;
    uint32_t nHeight : 31;
    HzCoin(const CTxOut& o, bool cb, uint32_t h) : out(o), fCoinBase(cb), nHeight(h) {}
    bool IsCoinBase() const { return fCoinBase; }
    bool IsSpent() const { return false; }
};

// Core's lookup is by outpoint, and tx_verify.cpp only ever passes tx.vin[i].prevout, so the index is the
// reference's position in vin. Anything else fails closed.
class HzCoinsView {
    const CTransaction& m_tx;
    const std::vector<HzCoin>& m_coins;
public:
    HzCoinsView(const CTransaction& tx, const std::vector<HzCoin>& coins) : m_tx(tx), m_coins(coins) {}
    bool HaveInputs(const CTransaction& tx) const { return &tx == &m_tx && tx.vin.size() == m_coins.size(); }
    const HzCoin& AccessCoin(const COutPoint& o) const {
        const char* p = reinterpret_cast<const char*>(&o);
        const char* base = reinterpret_cast<const char*>(&m_tx.vin[0].prevout);
        size_t i = static_cast<size_t>(p - base) / sizeof(CTxIn);
        assert(i < m_coins.size() && &m_tx.vin[i].prevout == &o);
        return m_coins[i];
    }
};

// What SequenceLocks reads from the block index. GetAncestor(h) answers only "the block before a coin"
// for this run's coins, from the coin's leaf-committed creation MTP. `pprev` carries the spending block's
// MTP. Two coins of one height must carry one MTP: a disagreement cannot come from real coins, so it fails.
class HzBlockIndex {
public:
    int nHeight{0};
    const HzBlockIndex* pprev{nullptr};
    int64_t m_mtp{0};
    unsigned m_k{0};
    const uint32_t* m_ch{nullptr};
    const uint32_t* m_cmtp{nullptr};
    mutable HzBlockIndex* m_scratch{nullptr};
    int64_t GetMedianTimePast() const { return m_mtp; }
    const HzBlockIndex* GetAncestor(int height) const {
        const HzBlockIndex* found = nullptr;
        for (unsigned j = 0; j < m_k; j++) {
            const int key = m_ch[j] == 0 ? 0 : static_cast<int>(m_ch[j]) - 1;
            if (key != height) continue;
            if (found && found->m_mtp != static_cast<int64_t>(m_cmtp[j])) return nullptr;
            m_scratch->m_mtp = m_cmtp[j];
            found = m_scratch;
        }
        return found;
    }
};

#include <consensus/tx_verify.cpp>

// Everything Core checks of one transaction from its coins, for the run of inputs [start, start + k).
// Returns 1, or a negative code. `first`: this run owns input 0, so it also does the whole-transaction rules
// once and reports fee, weight and sigop cost.
int hz_tx_rules(const CTransaction& tx, const std::vector<CTxOut>& spent, unsigned start, unsigned k,
                const uint32_t* ch, const uint32_t* cb, const uint32_t* cmtp,
                int height, int64_t mtp, int64_t lock_cutoff, uint32_t script_flags, bool csv_active,
                int64_t* out_fee, int64_t* out_weight, int64_t* out_sigops) {
    if (tx.IsCoinBase() || spent.size() != tx.vin.size() || static_cast<size_t>(start) + k > tx.vin.size()) return -24;
    std::vector<HzCoin> coins;
    coins.reserve(spent.size());
    for (unsigned i = 0; i < spent.size(); i++) {
        const bool mine = i >= start && i - start < k;
        coins.emplace_back(spent[i], mine && (cb[i - start] & 1) == 1, mine ? ch[i - start] : 0);
    }
    const HzCoinsView view(tx, coins);
    const bool first = start == 0;

    if (first) {
        TxValidationState st;
        if (!CheckTransaction(tx, st)) { hz_last_reject = st.GetRejectReason(); return -20; }
        if (!IsFinalTx(tx, height, lock_cutoff)) { hz_last_reject = "bad-txns-nonfinal"; return -25; }
    }
    // Core: Consensus::CheckTxInputs in ConnectBlock. Values, overflow, fee, and maturity of this run's coins.
    TxValidationState state;
    CAmount fee = 0;
    if (!Consensus::CheckTxInputs(tx, state, view, height, fee)) { hz_last_reject = state.GetRejectReason(); return -21; }

    // Core: SequenceLocks in ConnectBlock. Inputs outside this run are their own run's: in a split transaction
    // they are given SEQUENCE_FINAL here, which Core's code skips, so only this run's locks are evaluated.
    HzBlockIndex prev, scratch, block;
    prev.m_mtp = mtp;
    block.nHeight = height;
    block.pprev = &prev;
    block.m_k = k;
    block.m_ch = ch;
    block.m_cmtp = cmtp;
    block.m_scratch = &scratch;
    const int lock_flags = csv_active ? LOCKTIME_VERIFY_SEQUENCE : 0;
    std::vector<int> prev_heights(tx.vin.size(), 0);
    for (unsigned j = 0; j < k; j++) prev_heights[start + j] = static_cast<int>(ch[j]);
    if (k == tx.vin.size()) {
        if (!SequenceLocks(tx, lock_flags, prev_heights, block)) { hz_last_reject = "bad-txns-nonfinal"; return -41; }
    } else {
        CMutableTransaction part;
        part.version = tx.version;
        part.vin.resize(tx.vin.size());
        for (unsigned j = 0; j < k; j++) part.vin[start + j].nSequence = tx.vin[start + j].nSequence;
        const CTransaction part_tx{part};
        if (!SequenceLocks(part_tx, lock_flags, prev_heights, block)) { hz_last_reject = "bad-txns-nonfinal"; return -41; }
    }

    if (first) {
        *out_fee = fee;
        *out_weight = GetTransactionWeight(tx);
        *out_sigops = GetTransactionSigOpCost(tx, view, script_flags);
    }
    return 1;
}

// The coinbase's share of the same rules. Core's GetTransactionSigOpCost returns before reading a coin.
int64_t hz_coinbase_sigops(const CTransaction& tx, uint32_t script_flags) {
    const std::vector<HzCoin> none;
    return GetTransactionSigOpCost(tx, HzCoinsView(tx, none), script_flags);
}
int64_t hz_weight(const CTransaction& tx) { return GetTransactionWeight(tx); }

// Core's Assert() handler lives in util/check.cpp, which is logging glue. A failed assertion halts the guest,
// so no proof exists for that block.
void assertion_fail(std::string_view, int, std::string_view, std::string_view) { __builtin_trap(); }
