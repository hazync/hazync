/* test_assume.c: the assume zAPP's C ABI (swiftsync_verify_assume) on the real Core v31.1.0 snapshot at 200 against
 * the engine chain's mainnet 1..200 proof (the core's ../../../swiftsync-verify/vectors, under its image id and pinned
 * key) and refusals. Moved from swiftsync-verify-ffi/test.c's 5 assume cases, 15.11.2026; engine only since 27.11.2026.
 *   cc -O2 -Iinclude test_assume.c target/release/libzapp_assume_ffi.a -lpthread -ldl -lm -o test_assume && ./test_assume */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "zapp_assume.h"

static unsigned char* slurp(const char* p, size_t* n) {
    FILE* f = fopen(p, "rb"); if (!f) { perror(p); exit(2); }
    fseek(f, 0, SEEK_END); *n = (size_t)ftell(f); fseek(f, 0, SEEK_SET);
    unsigned char* b = malloc(*n); if (fread(b, 1, *n, f) != *n) exit(2); fclose(f); return b;
}

static int bad = 0, n = 0;
static char id[65]; static unsigned char *vk, pin[32]; static size_t vn;
static void run_assume(const char* name, const char* rc, const char* snap, int flip, const uint8_t* block, const char* expect,
                       int want, const char* want_h3) {
    size_t rn, sn = 0, hn; char msg[512];
    unsigned char *r = slurp(rc, &rn), *s = snap ? slurp(snap, &sn) : NULL;
    unsigned char *h = slurp("../vectors/headers_0_200.bin", &hn);
    if (s && flip >= 0) s[flip] ^= 1;
    zapp_assume_chain out; swiftsync_assume_result a; memset(&out, 0, sizeof out); memset(&a, 0, sizeof a);
    int got = swiftsync_verify_assume(r, rn, ZAPP_ASSUME_NET_MAINNET, (uint64_t)time(NULL), id, vk, vn, pin, expect, block, h, hn, s, sn, NULL,
                                      &out, &a, msg, sizeof msg);
    int ok = got == want && (!want_h3 || (strcmp(a.hash_serialized_3, want_h3) == 0 && a.snapshot_coins == 204 && out.height == 200));
    printf("%s  %-24s verdict %d (want %d)  coins %llu h3 %s height %u  %s\n", ok ? "PASS" : "WRONG", name, got, want,
           (unsigned long long)a.snapshot_coins, a.hash_serialized_3, a.assumevalid_height, msg);
    bad += !ok; n++; free(r); free(s); free(h);
}

int main(void) {
    const char* V = "../../../swiftsync-verify/";
    char p[512]; size_t k;
    snprintf(p, sizeof p, "%svectors/mainnet_1_200_engine.image_id", V); unsigned char* i = slurp(p, &k); memcpy(id, i, 64); id[64] = 0;
    snprintf(p, sizeof p, "%svectors/mainnet_1_200_engine.vk", V); vk = slurp(p, &vn);
    snprintf(p, sizeof p, "%stests/data/engine200/vk.sha256", V); unsigned char* ph = slurp(p, &k);
    for (int j = 0; j < 32; j++) sscanf((char*)ph + 2 * j, "%2hhx", &pin[j]);
    const char* R = "../../../swiftsync-verify/vectors/mainnet_1_200_engine.hze";
    const char* F = "../../../swiftsync-verify/vectors/mainnet_1_200_engine_proof_flipped.hze";
    const char* TIP = "200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";
    const char* H3 = "a51f51aea7cf42fc9f1ab491532de766675e983fde7166fa6273a6d6a6439d67";
    const uint8_t ZERO[32] = {0};
    run_assume("assumeutxo_200", R, "../vectors/utxo_200.dat", -1, ZERO, TIP, ZAPP_ASSUME_DEMO, H3);
    run_assume("assumeutxo_tampered", R, "../vectors/utxo_200.dat", 51, NULL, TIP, ZAPP_ASSUME_FAIL, NULL);
    run_assume("assumeutxo_199", R, "../vectors/utxo_199.dat", -1, NULL, NULL, ZAPP_ASSUME_FAIL, NULL);
    run_assume("assumeutxo_flipped_proof", F, "../vectors/utxo_200.dat", -1, NULL, NULL, ZAPP_ASSUME_FAIL, NULL);
    run_assume("assumeutxo_wrong_tip", R, "../vectors/utxo_200.dat", -1, NULL, "200:00000000d1145790a8694403d4063f323d499e655c83426834d4ce2f8dd4a2ee", ZAPP_ASSUME_FAIL, NULL);
    const uint8_t NOT_IN[32] = {7};
    run_assume("assumevalid_not_in_chain", R, NULL, -1, NOT_IN, NULL, ZAPP_ASSUME_FAIL, NULL);
    printf("ASSUME_C_ABI %d/%d %s\n", n - bad, n, bad ? "SOME CASES WRONG" : "all cases as expected");
    return bad != 0;
}
