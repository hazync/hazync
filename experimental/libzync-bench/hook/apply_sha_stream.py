"""Add a #ifdef HAZYNC_SHA_STREAM implementation of CSHA256 to a Bitcoin Core tree.

Usage: python3 -I apply_sha_stream.py <bitcoin-core dir>

Core's CSHA256 keeps its state in native word order and, under patches/0002, byte-swaps all 8 state
words in and out of the RISC0 accelerator on EVERY Transform call. It also copies every Write --
most are 4- or 8-byte serialisation fields -- through a memcpy call into its 64-byte buffer. On
block 962,000 that plumbing measured ~190 M cycles around 28 M of actual sys_sha_compress.

Under HAZYNC_SHA_STREAM, CSHA256 holds its state in the accelerator's layout (the digest's bytes in
memory order) from construction to Finalize:
- the constructor and Reset swap the IV once, Finalize copies the state out once;
- short writes copy inline instead of calling memcpy.
The hash is unchanged by construction, and every txid, wtxid and sighash depends on it, so a
mistake shows up as a different journal or a failed block, not a silent difference.

Only CSHA256's own methods change. The free sha256::Transform / TransformD64 paths, which other code
calls with native-order state, are untouched.
"""
import sys

path = sys.argv[1] + "/src/crypto/sha256.cpp"
src = open(path).read()
assert "HAZYNC_SHA_STREAM" not in src, "already applied"
start = src.index("CSHA256::CSHA256()\n")
end_marker = "CSHA256& CSHA256::Reset()\n{\n    bytes = 0;\n    sha256::Initialize(s);\n    return *this;\n}\n"
end = src.index(end_marker) + len(end_marker)
original = src[start:end]

stream = r'''#ifdef HAZYNC_SHA_STREAM
extern "C" void sys_sha_compress(uint32_t*, const uint32_t*, const uint32_t*, const uint32_t*);
namespace {
inline uint32_t hz_bswap(uint32_t x)
{
    return (x >> 24) | ((x >> 8) & 0x0000FF00u) | ((x << 8) & 0x00FF0000u) | (x << 24);
}
/* s is in the accelerator's layout throughout, so a block is ONE syscall and no swaps. */
inline void hz_compress(uint32_t* s, const unsigned char* chunk, size_t blocks)
{
    uint32_t blk[16];
    while (blocks--) {
        if ((((uintptr_t)chunk) & 3u) == 0u) {
            const uint32_t* w = (const uint32_t*)(const void*)chunk;
            sys_sha_compress(s, s, w, w + 8);
        } else {
            __builtin_memcpy(blk, chunk, 64);
            sys_sha_compress(s, s, blk, blk + 8);
        }
        chunk += 64;
    }
}
inline void hz_init(uint32_t* s)
{
    sha256::Initialize(s);
    for (int i = 0; i < 8; ++i) s[i] = hz_bswap(s[i]);
}
} // namespace

CSHA256::CSHA256()
{
    hz_init(s);
}

CSHA256& CSHA256::Write(const unsigned char* data, size_t len)
{
    size_t bufsize = bytes % 64;
    if (bufsize + len < 64) {
        /* The common case: a serialisation field that fits in the buffer. */
        unsigned char* d = buf + bufsize;
        for (size_t i = 0; i < len; ++i) d[i] = data[i];
        bytes += len;
        return *this;
    }
    const unsigned char* end = data + len;
    if (bufsize) {
        const size_t take = 64 - bufsize;
        for (size_t i = 0; i < take; ++i) buf[bufsize + i] = data[i];
        bytes += take;
        data += take;
        hz_compress(s, buf, 1);
    }
    if (end - data >= 64) {
        const size_t blocks = (end - data) / 64;
        hz_compress(s, data, blocks);
        data += 64 * blocks;
        bytes += 64 * blocks;
    }
    if (end > data) {
        const size_t r = end - data;
        for (size_t i = 0; i < r; ++i) buf[i] = data[i];
        bytes += r;
    }
    return *this;
}

void CSHA256::Finalize(unsigned char hash[OUTPUT_SIZE])
{
    static const unsigned char pad[64] = {0x80};
    unsigned char sizedesc[8];
    WriteBE64(sizedesc, bytes << 3);
    Write(pad, 1 + ((119 - (bytes % 64)) % 64));
    Write(sizedesc, 8);
    /* Accelerator layout IS the digest's byte order. */
    __builtin_memcpy(hash, s, 32);
}

CSHA256& CSHA256::Reset()
{
    bytes = 0;
    hz_init(s);
    return *this;
}
#else
'''
open(path, "w").write(src[:start] + stream + original + "#endif /* HAZYNC_SHA_STREAM */\n" + src[end:])
print(f"HAZYNC_SHA_STREAM added to {path}")
