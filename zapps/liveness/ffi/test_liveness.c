/* test_liveness.c: the liveness zAPP's C ABI (zapp_liveness_verify) on the real mainnet 1..200 proof (8888d59c,
 * risc0 3.1.0; the core's ../../../swiftsync-verify/vectors) and this zAPP's proofs (../vectors). Honest claims are
 * DEMO (below minimum work), refusals FAIL. Moved from swiftsync-verify-ffi/test.c's 7 liveness cases, 15.11.2026.
 *   cc -O2 -Iinclude test_liveness.c target/release/libzapp_liveness_ffi.a -lpthread -ldl -lm -o test_liveness && ./test_liveness */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "zapp_liveness.h"

static unsigned char* slurp(const char* p, size_t* n) {
    FILE* f = fopen(p, "rb"); if (!f) { perror(p); exit(2); }
    fseek(f, 0, SEEK_END); *n = (size_t)ftell(f); fseek(f, 0, SEEK_SET);
    unsigned char* b = malloc(*n); if (fread(b, 1, *n, f) != *n) exit(2); fclose(f); return b;
}

static int bad = 0, n = 0;
static const char* IMG = "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4";
static const char* CORE = "../../../swiftsync-verify/vectors/";

/* A liveness proof: `flip` >= 0 flips that hex digit. */
static void live(const char* name, const char* rc, const char* proof, const char* ask, int flip, const char* expect, int want) {
    size_t rn, pn; char msg[512], path[512];
    snprintf(path, sizeof path, "%s%s", CORE, rc);
    unsigned char* r = slurp(path, &rn);
    snprintf(path, sizeof path, "../vectors/%s", proof);
    unsigned char* p = slurp(path, &pn);
    while (pn && (p[pn - 1] == '\n' || p[pn - 1] == '\r')) pn--;
    if (flip >= 0) p[flip] = p[flip] == '0' ? '1' : '0';
    char* facts = malloc(pn + strlen(ask) + 64);
    sprintf(facts, "{\"liveness\": [{\"proof\": \"%.*s\", \"ask\": \"%s\"}]}", (int)pn, (const char*)p, ask);
    int got = zapp_liveness_verify(r, rn, 0x20, (uint64_t)time(NULL), IMG, NULL, facts, expect, msg, sizeof msg);
    printf("%s  %-24s verdict %d (want %d)  %.150s\n", got == want ? "PASS" : "WRONG", name, got, want, msg);
    bad += got != want; n++; free(r); free(p); free(facts);
}

/* Review 11-18: the facts JSON is {"liveness": [...]} and nothing else. `fmt` holds %s where the one honest fact goes
 * (up to 3 times); `why`: a word the refusal names. Before facts_from_json a key beside "liveness" was dropped unread. */
static void facts(const char* name, const char* fmt, int want, const char* why) {
    size_t rn, pn; char msg[512], path[512];
    snprintf(path, sizeof path, "%smainnet_1_200_8888d59c.rcpt", CORE);
    unsigned char* r = slurp(path, &rn);
    unsigned char* p = slurp("../vectors/b170_0_at200.hex", &pn);
    while (pn && (p[pn - 1] == '\n' || p[pn - 1] == '\r')) pn--;
    char* f = malloc(pn + 256);
    sprintf(f, "{\"proof\": \"%.*s\", \"ask\": \"f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16:0@200=unspent\"}", (int)pn, (const char*)p);
    char* j = malloc(3 * strlen(f) + strlen(fmt) + 1);
    sprintf(j, fmt, f, f, f);
    int got = zapp_liveness_verify(r, rn, 0x20, (uint64_t)time(NULL), IMG, NULL, j, NULL, msg, sizeof msg);
    int ok = got == want && (!why || strstr(msg, why));
    printf("%s  %-24s verdict %d (want %d)  %.150s\n", ok ? "PASS" : "WRONG", name, got, want, msg);
    bad += !ok; n++; free(r); free(p); free(f); free(j);
}

int main(void) {
    const char* H = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16:0@200";
    const char* R = "mainnet_1_200_8888d59c.rcpt";
    const char* TIP = "200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320";
    char q[256];
    snprintf(q, sizeof q, "%s=unspent,value=1000000000", H);
    live("liveness_unspent", R, "b170_0_at200.hex", q, -1, NULL, ZAPP_LIVENESS_DEMO); /* low work: DEMO */
    live("liveness_unspent_expect", R, "b170_0_at200.hex", q, -1, TIP, ZAPP_LIVENESS_DEMO); /* tip matches; still low work */
    live("liveness_spent", R, "b9_0_at200.hex", "0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9:0@200=spent:170:1:0", -1, NULL, ZAPP_LIVENESS_DEMO);
    snprintf(q, sizeof q, "%s=spent", H);
    live("liveness_false_claim", R, "b170_0_at200.hex", q, -1, NULL, ZAPP_LIVENESS_FAIL);
    live("liveness_no_claim", R, "b170_0_at200.hex", H, -1, NULL, ZAPP_LIVENESS_FAIL);
    snprintf(q, sizeof q, "%s=unspent,value=999", H);
    live("liveness_wrong_amount", R, "b170_0_at200.hex", q, -1, NULL, ZAPP_LIVENESS_FAIL);
    snprintf(q, sizeof q, "%s=unspent", H);
    live("liveness_tampered", R, "b170_0_at200.hex", q, 200, NULL, ZAPP_LIVENESS_FAIL);
    live("liveness_wrong_tip", R, "b170_0_at200.hex", q, -1, "200:0000000000000000000000000000000000000000000000000000000000000001", ZAPP_LIVENESS_FAIL);
    snprintf(q, sizeof q, "%s=unspent,created=170,coinbase=0", H);
    live("liveness_created_pinned", R, "b170_0_at200.hex", q, -1, NULL, ZAPP_LIVENESS_DEMO);
    snprintf(q, sizeof q, "%s=unspent,created=171", H);
    live("liveness_wrong_created", R, "b170_0_at200.hex", q, -1, NULL, ZAPP_LIVENESS_FAIL);
    live("liveness_upper_case_txid", R, "b170_0_at200.hex", "F4184FC596403B9D638783CF57ADFE4C75C605F6356FBC91338530E9831E9E16:0@200=unspent", -1, NULL, ZAPP_LIVENESS_FAIL);
    live("liveness_fake_receipt", "fake_block_2501.rcpt", "b170_0_at200.hex", q, -1, NULL, ZAPP_LIVENESS_FAIL);
    /* positive control: the zAPP regtest harness's real receipt (image eb2cd424) and its real liveness proof are refused */
    live("regtest_receipt_refused", "../../scripts/zapp-regtest/receipts/a/block_112.rcpt", "regtest_x_spent_a112.hex",
         "e0b6ab75f1ce5f1dcf1a77ded4bf03eb962f50365ee8f77a4b269802dadd5a9f:1@112=spent:111:2:0", -1,
         "112:67c1d2e718067bba4ffd07a10f2ce27f5e6b55702eb2c766fafbf36db6160574", ZAPP_LIVENESS_FAIL);
    facts("facts_documented_key", "{\"liveness\": [%s]}", ZAPP_LIVENESS_DEMO, NULL);
    facts("facts_misspelt_beside", "{\"liveness\": [%s], \"livenes\": [%s]}", ZAPP_LIVENESS_FAIL, "unknown field `livenes`");
    facts("facts_misspelt_alone", "{\"livenes\": [%s]}", ZAPP_LIVENESS_FAIL, "unknown field `livenes`");
    facts("facts_key_twice", "{\"liveness\": [%s], \"liveness\": [%s]}", ZAPP_LIVENESS_FAIL, "duplicate field `liveness`");
    facts("facts_expect_inside", "{\"liveness\": [%s], \"expect\": \"200:00\"}", ZAPP_LIVENESS_FAIL, "\"expect\" inside");
    facts("facts_expect_null", "{\"liveness\": [%s], \"expect\": null}", ZAPP_LIVENESS_FAIL, "\"expect\" inside");
    facts("facts_not_an_object", "[[%s]]", ZAPP_LIVENESS_FAIL, "not an object");
    facts("facts_fact_extra_key", "{\"liveness\": [{\"x\": 1, \"proof\": \"00\", \"ask\": \"q\"}]}", ZAPP_LIVENESS_FAIL, "unknown field `x`");
    printf("LIVENESS_C_ABI %d/%d %s\n", n - bad, n, bad ? "SOME CASES WRONG" : "all cases as expected");
    return bad != 0;
}
