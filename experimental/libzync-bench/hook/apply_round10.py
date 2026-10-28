"""Round 10 Core-tree patch, behind its own #ifdef so a build without the define is unchanged.

Usage: python3 -I apply_round10.py <bitcoin-core dir>

HAZYNC_D64_STREAM (src/crypto/sha256.cpp)
    Merkle nodes go through SHA256D64 -> TransformD64. Under HAZYNC_SHA_D64_ACCEL that is Core's
    generic TransformD64Wrapper<sha256::Transform>, and patches/0002's sha256::Transform byte-swaps
    all 8 state words into and out of the risc0 accelerator on every one of its 3 calls per node,
    plus the wrapper's own WriteBE32 round trips. On block 962,000's aggregate that was ~29 M cycles
    for ~12.6k nodes, against ~4 M of actual sys_sha_compress.

    hz_d64 computes the same double-SHA256 of one 64-byte block with the state held in the
    accelerator's layout throughout (the layout in which the state IS the digest's bytes), so a node
    is 3 sys_sha_compress calls and two 32-byte copies. Same construction as Core's wrapper: hash the
    64 bytes plus one padding block, then hash the 32-byte digest plus its padding. Any mistake
    changes every merkle root, so the block's merkle_ok / witness checks catch it.
"""
import sys

path = sys.argv[1] + "/src/crypto/sha256.cpp"
src = open(path).read()
assert "HAZYNC_D64_STREAM" not in src, "already applied"

old = """#ifdef HAZYNC_SHA_D64_ACCEL
"""
new = """#if defined(HAZYNC_D64_STREAM)
extern "C" void sys_sha_compress(uint32_t*, const uint32_t*, const uint32_t*, const uint32_t*);
namespace {
inline uint32_t hz_d64_bswap(uint32_t x)
{
    return (x >> 24) | ((x >> 8) & 0x0000FF00u) | ((x << 8) & 0x00FF0000u) | (x << 24);
}
/* Double-SHA256 of one 64-byte block, state in accelerator layout throughout. */
void hz_d64(unsigned char* out, const unsigned char* in)
{
    static uint32_t iv[8];
    static bool iv_ready = false;
    if (!iv_ready) {
        sha256::Initialize(iv);
        for (int i = 0; i < 8; ++i) iv[i] = hz_d64_bswap(iv[i]);
        iv_ready = true;
    }
    /* Padding block for a 64-byte message: 0x80, zeros, then the bit length 512 big-endian. */
    alignas(4) static const unsigned char pad64b[64] = {
        0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0,    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0,    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0,    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0};
    const uint32_t* pad64 = reinterpret_cast<const uint32_t*>(pad64b);
    uint32_t blk[16];
    uint32_t s[8];
    __builtin_memcpy(blk, in, 64);
    sys_sha_compress(s, iv, blk, blk + 8);
    sys_sha_compress(s, s, pad64, pad64 + 8);
    /* Second hash: the 32-byte digest (accelerator layout = digest byte order), 0x80, zeros, then
     * the bit length 256 big-endian. */
    uint32_t b2[16] = {0};
    __builtin_memcpy(b2, s, 32);
    reinterpret_cast<unsigned char*>(b2)[32] = 0x80;
    reinterpret_cast<unsigned char*>(b2)[62] = 0x01;
    reinterpret_cast<unsigned char*>(b2)[63] = 0x00;
    sys_sha_compress(s, iv, b2, b2 + 8);
    __builtin_memcpy(out, s, 32);
}
} // namespace
TransformD64Type TransformD64 = hz_d64;
#elif defined(HAZYNC_SHA_D64_ACCEL)
"""
if src.count(old) == 1:                      # GHOST: patches/0010 already routes D64 to the accelerator
    src = src.replace(old, new)
else:                                         # CORE: no patches/0010; wrap Core's own default
    plain = "TransformD64Type TransformD64 = sha256::TransformD64;\n"
    assert src.count(plain) == 1, f"neither anchor found in {path}"
    src = src.replace(plain, new.replace("#elif defined(HAZYNC_SHA_D64_ACCEL)\n", "#else\n") + plain + "#endif\n")
open(path, "w").write(src)
print(f"HAZYNC_D64_STREAM added to {path}")
