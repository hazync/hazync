// Bitcoin Core's own block rules, for the libzync guest.
//
// validation.cpp is the whole node and cannot be compiled in. These functions are lifted out of it, and out of
// pow.cpp, by the guest's build.rs at build time, into core_extract.inc, so they are Core's text and cannot drift
// from it. The build fails if one is not found. Each gets only what it reads:
//
//   CBlockIndex         a stand-in for the previous block: height, bits, time, and its median-time-past
//   ChainstateManager   a stand-in with the consensus params; checkpoints are off, as they are for a chain with no node
//   BlockManager        a stand-in that has no checkpoints to offer
//   NodeClock           a clock that never reads as past a block. time-too-new has no clock in a proof, so the verifier
//                       applies it to the committed max_time instead
//   CBlock              the real class, holding the real header and coinbase. The other transactions are in the chunks, so
//   BlockWitnessMerkleRoot   is answered from the wtxids those chunks committed, by Core's ComputeMerkleRoot
//   GetBlockWeight      is answered from the block total, summed across the chunks of Core's GetTransactionWeight
//   tx->HasWitness()    a transaction carrying a witness stands in for "some other transaction has one"

#include <arith_uint256.h>
#include <consensus/amount.h>
#include <consensus/consensus.h>
#include <consensus/merkle.h>
#include <consensus/params.h>
#include <consensus/tx_verify.h>
#include <consensus/validation.h>
#include <hash.h>
#include <logging.h>
#include <primitives/block.h>
#include <primitives/transaction.h>
#include <script/interpreter.h>
#include <script/script.h>
#include <sync.h>
#include <tinyformat.h>
#include <uint256.h>
#include <util/check.h>
#include <cassert>
#include <chrono>
#include <cstring>
#include <vector>

#define BITCOIN_CHAIN_H
#define CBlockIndex HzBlockIndexH
#include <pow.h>

const Consensus::Params& hz_net_params();
extern "C" uint32_t calc_next_bits(uint32_t prev_bits, int64_t first_time, int64_t last_time);

class HzBlockIndexH {
public:
    int nHeight{0};
    uint32_t nBits{0};
    int64_t m_time{0};
    int64_t m_mtp{0};
    const uint256* phashBlock{nullptr};
    const HzBlockIndexH* pprev{nullptr};
    const HzBlockIndexH* m_first{nullptr};   // the retarget period's first block
    int64_t GetBlockTime() const { return m_time; }
    int64_t GetMedianTimePast() const { return m_mtp; }
    const HzBlockIndexH* GetAncestor(int) const { return m_first; }   // GetNextWorkRequired's only use
};

class HzCheckpoints {};
struct HzParams { HzCheckpoints Checkpoints() const { return {}; } };
struct HzOptions { bool checkpoints_enabled{false}; };
class ChainstateManager {
public:
    HzOptions m_options;
    const Consensus::Params& GetConsensus() const { return hz_net_params(); }
    HzParams GetParams() const { return {}; }
};
class BlockManager {
public:
    const CBlockIndex* GetLastCheckpoint(const HzCheckpoints&) const { return nullptr; }
};
// Core's deploymentstatus.h for a buried deployment, the only kind these functions ask about: the same comparisons
// on Core's own height table, without the version-bits cache that header needs for signalled deployments.
static bool DeploymentActiveAt(const CBlockIndex& index, const ChainstateManager& chainman, Consensus::BuriedDeployment dep) {
    return index.nHeight >= chainman.GetConsensus().DeploymentHeight(dep);
}
static bool DeploymentActiveAfter(const CBlockIndex* prev, const ChainstateManager& chainman, Consensus::BuriedDeployment dep) {
    return (prev == nullptr ? 0 : prev->nHeight + 1) >= chainman.GetConsensus().DeploymentHeight(dep);
}
static constexpr int64_t MAX_FUTURE_BLOCK_TIME = 2 * 60 * 60;   // chain.h; read only by the never-firing clock test
struct HzNeverClock {
    static NodeSeconds now() { return NodeSeconds{std::chrono::seconds{int64_t{1} << 40}}; }
};
// pow.cpp's GetNextWorkRequired hands the period to this; the real one, on the real index, is behind calc_next_bits.
unsigned int CalculateNextWorkRequired(const CBlockIndex* last, int64_t first_time, const Consensus::Params&) {
    return calc_next_bits(last->nBits, first_time, last->GetBlockTime());
}

static const uint8_t* g_wtxids;
static uint32_t g_n;
static int64_t g_weight;
static uint256 hz_witness_root() {
    std::vector<uint256> leaves(g_n);
    for (uint32_t i = 0; i < g_n; i++) std::memcpy(leaves[i].begin(), g_wtxids + 32 * i, 32);
    leaves[0].SetNull();   // BlockWitnessMerkleRoot: the coinbase's wtxid is taken as zero
    return ComputeMerkleRoot(std::move(leaves), nullptr);
}

#define BlockWitnessMerkleRoot(block, mutated) hz_witness_root()
#define GetBlockWeight(block) g_weight
#define NodeClock HzNeverClock
#include "core_extract.inc"
#undef NodeClock
#undef GetBlockWeight
#undef BlockWitnessMerkleRoot

int64_t hz_block_subsidy(int height) { return GetBlockSubsidy(height, hz_net_params()); }

uint32_t hz_block_flags(int height, const uint8_t* block_hash) {
    uint256 h;
    std::memcpy(h.begin(), block_hash, 32);
    CBlockIndex index;
    index.nHeight = height;
    index.phashBlock = &h;
    return GetBlockScriptFlags(index, ChainstateManager{});
}

static CBlockHeader parse_header(const uint8_t* b) {
    CBlockHeader h;
    uint32_t v;
    std::memcpy(&h.nVersion, b, 4);
    std::memcpy(h.hashPrevBlock.begin(), b + 4, 32);
    std::memcpy(h.hashMerkleRoot.begin(), b + 36, 32);
    std::memcpy(&v, b + 68, 4); h.nTime = v;
    std::memcpy(&v, b + 72, 4); h.nBits = v;
    std::memcpy(&v, b + 76, 4); h.nNonce = v;
    return h;
}

// ContextualCheckBlockHeader: bad-diffbits, time-too-old, bad-version. `height` is the block's own.
bool hz_header_rules(const uint8_t* header, int height, uint32_t prev_bits, int64_t prev_time, int64_t prev_mtp, int64_t period_first_time) {
    if (height < 1) return false;
    const CBlockHeader h = parse_header(header);
    CBlockIndex first;
    first.m_time = period_first_time;
    CBlockIndex prev;
    prev.nHeight = height - 1;
    prev.nBits = prev_bits;
    prev.m_time = prev_time;
    prev.m_mtp = prev_mtp;
    prev.m_first = &first;
    BlockValidationState state;
    BlockManager blockman;
    return ContextualCheckBlockHeader(h, state, blockman, ChainstateManager{}, &prev);
}

// ContextualCheckBlock: bad-txns-nonfinal for the coinbase (each other transaction's IsFinalTx runs in its chunk), bad-cb-height,
// the witness rules, bad-blk-weight.
bool hz_block_rules(const uint8_t* header, const CMutableTransaction& coinbase, const uint8_t* wtxids, uint32_t n,
                    bool other_has_witness, int height, int64_t prev_mtp, int64_t weight) {
    if (height < 1 || n == 0) return false;
    CBlock block;
    static_cast<CBlockHeader&>(block) = parse_header(header);
    block.vtx.push_back(MakeTransactionRef(coinbase));
    if (block.vtx[0]->vin.empty()) return false;
    if (other_has_witness) {
        CMutableTransaction w;
        w.vin.resize(1);
        w.vin[0].scriptWitness.stack.push_back({1});
        block.vtx.push_back(MakeTransactionRef(w));
    }
    g_wtxids = wtxids;
    g_n = n;
    g_weight = weight;
    CBlockIndex prev;
    prev.nHeight = height - 1;
    prev.m_mtp = prev_mtp;
    BlockValidationState state;
    return ContextualCheckBlock(block, state, ChainstateManager{}, &prev);
}
