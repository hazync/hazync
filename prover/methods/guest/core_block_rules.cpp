// Bitcoin Core's own block-level functions from validation.cpp, for the libzync guest.
//
// validation.cpp is the whole node and cannot be compiled in. These functions are lifted out of it by the
// guest's build.rs at build time, into core_extract.inc, so they are Core's text and cannot drift from it. The
// build fails if one is not found. Each gets only what it reads:
//
//   ChainstateManager   a stand-in with GetConsensus(), which is all these functions call on it
//   CBlock              the real class, holding the real coinbase. The other transactions are in the chunks, so
//   BlockWitnessMerkleRoot   is answered from the wtxids those chunks committed, by Core's ComputeMerkleRoot
//   tx->HasWitness()    a transaction carrying a witness stands in for "some other transaction has one"

#include <chain.h>
#include <consensus/amount.h>
#include <consensus/merkle.h>
#include <consensus/params.h>
#include <consensus/validation.h>
#include <hash.h>
#include <primitives/block.h>
#include <primitives/transaction.h>
#include <script/interpreter.h>
#include <tinyformat.h>
#include <uint256.h>
#include <util/check.h>
#include <cassert>
#include <cstring>
#include <vector>

const Consensus::Params& hz_net_params();

class ChainstateManager {
public:
    const Consensus::Params& GetConsensus() const { return hz_net_params(); }
};
// Core's deploymentstatus.h for a buried deployment, the only kind these functions ask about: the same comparison
// on Core's own height table, without the version-bits cache that header needs for signalled deployments.
static bool DeploymentActiveAt(const CBlockIndex& index, const ChainstateManager& chainman, Consensus::BuriedDeployment dep) {
    return index.nHeight >= chainman.GetConsensus().DeploymentHeight(dep);
}

static const uint8_t* g_wtxids;
static uint32_t g_n;
static uint256 hz_witness_root() {
    std::vector<uint256> leaves(g_n);
    for (uint32_t i = 0; i < g_n; i++) std::memcpy(leaves[i].begin(), g_wtxids + 32 * i, 32);
    leaves[0].SetNull();   // BlockWitnessMerkleRoot: the coinbase's wtxid is taken as zero
    return ComputeMerkleRoot(std::move(leaves), nullptr);
}

#define BlockWitnessMerkleRoot(block, mutated) hz_witness_root()
#include "core_extract.inc"
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

bool hz_witness_rule(const CMutableTransaction& coinbase, const uint8_t* wtxids, uint32_t n, bool other_has_witness, int height) {
    if (n == 0) return false;
    CBlock block;
    block.vtx.push_back(MakeTransactionRef(coinbase));
    if (other_has_witness) {
        CMutableTransaction w;
        w.vin.resize(1);
        w.vin[0].scriptWitness.stack.push_back({1});
        block.vtx.push_back(MakeTransactionRef(w));
    }
    g_wtxids = wtxids;
    g_n = n;
    CBlockIndex index;
    index.nHeight = height;
    BlockValidationState state;
    return CheckWitnessMalleation(block, DeploymentActiveAt(index, ChainstateManager{}, Consensus::DEPLOYMENT_SEGWIT), state);
}
