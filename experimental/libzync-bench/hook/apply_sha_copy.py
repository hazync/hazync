"""Round 6: replace the byte loops in the HAZYNC_SHA_STREAM CSHA256::Write with a word copy.

Usage: python3 -I apply_sha_copy.py <bitcoin-core dir>   (after apply_sha_stream.py)

Most writes land at a buffer offset whose alignment differs from the source's, so a plain word copy
rarely applies. hz_copy aligns the DESTINATION, then assembles each destination word from two aligned
source words with shifts (rv32im is little-endian). It may read up to 3 bytes past the end of the
source, but never past the aligned word that holds its last byte. Bytes written are unchanged.
"""
import sys

path = sys.argv[1] + "/src/crypto/sha256.cpp"
src = open(path).read()
assert "HAZYNC_SHA_STREAM" in src and "hz_copy" not in src
helper = '''inline void hz_copy(unsigned char* d, const unsigned char* s, size_t n)
{
    if (n >= 12) {
        while (((uintptr_t)d) & 3u) { *d++ = *s++; --n; }
        uint32_t* dw = (uint32_t*)(void*)d;
        const size_t words = n / 4;
        const unsigned k = (unsigned)(((uintptr_t)s) & 3u);
        if (k == 0) {
            const uint32_t* sw = (const uint32_t*)(const void*)s;
            for (size_t i = 0; i < words; ++i) dw[i] = sw[i];
        } else {
            const uint32_t* sw = (const uint32_t*)(const void*)(s - k);
            const unsigned lo = 8u * k, hi = 32u - lo;
            uint32_t a = sw[0];
            for (size_t i = 0; i < words; ++i) {
                const uint32_t b = sw[i + 1];
                dw[i] = (a >> lo) | (b << hi);
                a = b;
            }
        }
        d += 4 * words; s += 4 * words; n -= 4 * words;
    }
    while (n--) *d++ = *s++;
}
inline void hz_init(uint32_t* s)'''
reps = [
    ("inline void hz_init(uint32_t* s)", helper),
    ("        unsigned char* d = buf + bufsize;\n        for (size_t i = 0; i < len; ++i) d[i] = data[i];\n",
     "        hz_copy(buf + bufsize, data, len);\n"),
    ("        for (size_t i = 0; i < take; ++i) buf[bufsize + i] = data[i];\n",
     "        hz_copy(buf + bufsize, data, take);\n"),
    ("        for (size_t i = 0; i < r; ++i) buf[i] = data[i];\n",
     "        hz_copy(buf, data, r);\n"),
]
for old, new in reps:
    assert src.count(old) == 1, old
    src = src.replace(old, new)
open(path, "w").write(src)
print(f"hz_copy installed in {path}")
