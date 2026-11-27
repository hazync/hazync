/* test.c: the C ABI on the vectors (../swiftsync-verify/vectors): the engine chain's proof of mainnet 1..200 under its
 * image id and pinned key (no engine image is published yet, so every call gives both), its one-change copies, bundles,
 * and refusals. ../swiftsync-verify/vectors/README.md describes every case.
 *   cc -O2 -Iinclude test.c target/release/libswiftsync_verify.a -lpthread -ldl -lm -o test && ./test */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "swiftsync_verify.h"

static const char* V = "../swiftsync-verify/vectors/";
static unsigned char* slurp(const char* f, size_t* n) {
    char p[1024]; snprintf(p, sizeof p, "%s%s", V, f);
    FILE* h = fopen(p, "rb"); if (!h) { perror(p); exit(2); }
    fseek(h, 0, SEEK_END); *n = (size_t)ftell(h); fseek(h, 0, SEEK_SET);
    unsigned char* b = malloc(*n + 1); if (fread(b, 1, *n, h) != *n) exit(2); fclose(h); b[*n] = 0; return b;
}

static int bad = 0;
static unsigned char *vk, pin[32];
static size_t vn;
static void run(const char* name, const char* file, uint8_t net, const char* image, const unsigned char* key, size_t kn,
                int want) {
    size_t rn; char msg[256] = {0};
    unsigned char* r = slurp(file, &rn);
    swiftsync_result out; memset(&out, 0, sizeof out);
    int got = swiftsync_verify_engine_facts(r, rn, net, (uint64_t)time(NULL), image, NULL, key, kn, pin, NULL, NULL,
                                            &out, msg, sizeof msg);
    printf("%s  %-24s verdict %d (want %d)  %.120s\n", got == want ? "PASS" : "WRONG", name, got, want, msg);
    bad += got != want; free(r);
}

int main(void) {
    size_t n;
    char* id = (char*)slurp("mainnet_1_200_engine.image_id", &n);
    for (size_t i = 0; i < n; i++) if (id[i] == '\n') id[i] = 0;
    char* pinhex = (char*)slurp("../tests/data/engine200/vk.sha256", &n);
    for (int i = 0; i < 32; i++) sscanf(pinhex + 2 * i, "%2hhx", &pin[i]);
    vk = slurp("mainnet_1_200_engine.vk", &vn);
    size_t kn; unsigned char* kf = slurp("mainnet_1_200_engine_key_flipped.vk", &kn);
    const uint8_t M = SWIFTSYNC_NET_MAINNET, S = SWIFTSYNC_NET_SIGNET;
    const char* OTHER = "abababababababababababababababababababababababababababababababab";
    run("mainnet_1_200_demo", "mainnet_1_200_engine.hze", M, id, vk, vn, SWIFTSYNC_VERIFY_DEMO);
    run("mainnet_1_200_as_signet", "mainnet_1_200_engine.hze", S, id, vk, vn, SWIFTSYNC_VERIFY_FAIL);
    run("other_image_id", "mainnet_1_200_engine.hze", M, OTHER, vk, vn, SWIFTSYNC_VERIFY_FAIL);
    run("no_image_given", "mainnet_1_200_engine.hze", M, NULL, vk, vn, SWIFTSYNC_VERIFY_FAIL);
    run("key_flipped", "mainnet_1_200_engine.hze", M, id, kf, kn, SWIFTSYNC_VERIFY_FAIL);
    run("no_key", "mainnet_1_200_engine.hze", M, id, NULL, 0, SWIFTSYNC_VERIFY_FAIL);
    run("proof_flipped", "mainnet_1_200_engine_proof_flipped.hze", M, id, vk, vn, SWIFTSYNC_VERIFY_FAIL);
    run("journal_flipped", "mainnet_1_200_engine_journal_flipped.hze", M, id, vk, vn, SWIFTSYNC_VERIFY_FAIL);
    run("trailing_byte", "mainnet_1_200_engine_trailing_byte.hze", M, id, vk, vn, SWIFTSYNC_VERIFY_FAIL);
    run("truncated", "mainnet_1_200_engine_truncated.hze", M, id, vk, vn, SWIFTSYNC_VERIFY_FAIL);
    run("length_past_end", "mainnet_1_200_engine_length_past_end.hze", M, id, vk, vn, SWIFTSYNC_VERIFY_FAIL);
    run("bundle", "mainnet_1_200_engine.sspf", M, id, vk, vn, SWIFTSYNC_VERIFY_DEMO);
    run("bundle_header_lies", "bundle_header_lies.sspf", M, id, vk, vn, SWIFTSYNC_VERIFY_FAIL);
    run("bundle_v1", "bundle_v1.sspf", M, id, vk, vn, SWIFTSYNC_VERIFY_FAIL);
    run("bad_image_hex", "mainnet_1_200_engine.hze", M, "xyz", vk, vn, SWIFTSYNC_VERIFY_FAIL);
    run("unknown_network", "mainnet_1_200_engine.hze", 7, id, vk, vn, SWIFTSYNC_VERIFY_FAIL);
    /* without a key (swiftsync_verify): no engine image is published, so refused with the reason */
    { size_t rn; char msg[256] = {0}; swiftsync_result out; unsigned char* r = slurp("mainnet_1_200_engine.hze", &rn);
      int got = swiftsync_verify(r, rn, M, (uint64_t)time(NULL), id, NULL, &out, msg, sizeof msg);
      int ok = got == SWIFTSYNC_VERIFY_FAIL && strstr(msg, "no engine image is published");
      printf("%s  %-24s verdict %d (want 1)  %.120s\n", ok ? "PASS" : "WRONG", "no_key_swiftsync_verify", got, msg); bad += !ok; free(r); }
    { int ok = swiftsync_image_id(0) == NULL && swiftsync_image_id(1) == NULL;
      printf("%s  %-24s no published engine image id yet\n", ok ? "PASS" : "WRONG", "image_id_null"); bad += !ok; }
    printf("%s\n", bad ? "SOME CASES WRONG" : "all cases as expected");
    return bad != 0;
}
