/* test_engine.c <swiftsync-verify/tests/data/engine200>: swiftsync_verify_engine_facts on the engine chain's proof of
 * block 200, a coin fact on it, and refusals (engine/VERIFIER.md).
 *   cargo build --release && cc -O2 -Iinclude test_engine.c target/release/libswiftsync_verify.a \
 *     -lpthread -ldl -lm -o test_engine && ./test_engine ../swiftsync-verify/tests/data/engine200 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "swiftsync_verify.h"

static uint8_t* rd(const char* dir, const char* name, size_t* n) {
    char p[4096]; snprintf(p, sizeof p, "%s/%s", dir, name);
    FILE* f = fopen(p, "rb"); if (!f) { perror(p); exit(2); }
    fseek(f, 0, SEEK_END); long len = ftell(f); fseek(f, 0, SEEK_SET);
    uint8_t* b = malloc(len + 1); if (fread(b, 1, len, f) != (size_t)len) exit(2); fclose(f);
    b[len] = 0; *n = len; return b;
}
static int bad = 0;
static void want(const char* name, int got, int want_rc, const char* msg) {
    int ok = got == want_rc; bad += !ok;
    printf("%s %-36s rc %d (want %d) %.110s\n", ok ? "ok  " : "BAD ", name, got, want_rc, msg);
}
int main(int argc, char** argv) {
    if (argc != 2) { fprintf(stderr, "usage: %s <engine200 dir>\n", argv[0]); return 2; }
    size_t pn, vn, hn, cn, idn;
    uint8_t *proof = rd(argv[1], "proof.hze", &pn), *vk = rd(argv[1], "vk.bin", &vn);
    char *pinhex = (char*)rd(argv[1], "vk.sha256", &hn), *coin = (char*)rd(argv[1], "coin_spent_8.json", &cn);
    char *id = (char*)rd(argv[1], "image_id", &idn);
    for (size_t i = 0; i < idn; i++) if (id[i] == '\n') id[i] = 0;
    uint8_t pin[32]; for (int i = 0; i < 32; i++) sscanf(pinhex + 2 * i, "%2hhx", &pin[i]);
    char facts[16384]; snprintf(facts, sizeof facts, "{\"coins\": [%s]}", coin);
    char msg[512]; swiftsync_result out; uint64_t now = 1790000000;
    msg[0] = 0; int rc = swiftsync_verify_engine_facts(proof, pn, SWIFTSYNC_IMG_PRODUCT, now, id, NULL, vk, vn, pin, NULL, NULL, &out, msg, sizeof msg);
    want("block 200 (demo: low work)", rc, SWIFTSYNC_VERIFY_DEMO, msg);
    if (rc == SWIFTSYNC_VERIFY_DEMO) printf("     height %u profile 0x%x coins %llu\n", out.height, out.profile, (unsigned long long)out.utxo_leaves);
    msg[0] = 0; rc = swiftsync_verify_engine_facts(proof, pn, SWIFTSYNC_IMG_PRODUCT, now, id, NULL, vk, vn, pin, facts, NULL, &out, msg, sizeof msg);
    want("+ coin fact: block 9's coinbase spent", rc, SWIFTSYNC_VERIFY_DEMO, msg);
    /* the same fact with its spender changed (block 170 -> 171): must FAIL, so the fact is really checked */
    char* sp = strstr(facts, "170"); if (!sp) { printf("BAD  no 170 in the fact\n"); return 1; }
    sp[2] = '1';
    msg[0] = 0; want("+ coin fact with another spender", swiftsync_verify_engine_facts(proof, pn, SWIFTSYNC_IMG_PRODUCT, now, id, NULL, vk, vn, pin, facts, NULL, &out, msg, sizeof msg), SWIFTSYNC_VERIFY_FAIL, msg);
    uint8_t pin2[32]; memcpy(pin2, pin, 32); pin2[0] ^= 1;
    msg[0] = 0; want("another pin", swiftsync_verify_engine_facts(proof, pn, SWIFTSYNC_IMG_PRODUCT, now, id, NULL, vk, vn, pin2, NULL, NULL, &out, msg, sizeof msg), SWIFTSYNC_VERIFY_FAIL, msg);
    proof[pn - 40] ^= 1;
    msg[0] = 0; want("journal bit flipped", swiftsync_verify_engine_facts(proof, pn, SWIFTSYNC_IMG_PRODUCT, now, id, NULL, vk, vn, pin, NULL, NULL, &out, msg, sizeof msg), SWIFTSYNC_VERIFY_FAIL, msg);
    proof[pn - 40] ^= 1;
    msg[0] = 0; want("no key", swiftsync_verify_engine_facts(proof, pn, SWIFTSYNC_IMG_PRODUCT, now, id, NULL, NULL, 0, pin, NULL, NULL, &out, msg, sizeof msg), SWIFTSYNC_VERIFY_FAIL, msg);
    msg[0] = 0; want("engine proof to swiftsync_verify", swiftsync_verify(proof, pn, SWIFTSYNC_IMG_PRODUCT, now, id, NULL, &out, msg, sizeof msg), SWIFTSYNC_VERIFY_FAIL, msg);
    printf(bad ? "ENGINE_C_FAILED\n" : "ENGINE_C_OK\n");
    return bad != 0;
}
