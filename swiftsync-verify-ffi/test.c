/* test.c: the C ABI on the vectors (../swiftsync-verify/vectors): a real mainnet 1..200 proof of the frozen image (8888d59c, risc0 3.1.0) and refusals.
 *   cc -O2 -Iinclude test.c target/release/libswiftsync_verify.a -lpthread -ldl -lm -o test && ./test */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "swiftsync_verify.h"

static unsigned char* slurp(const char* p, size_t* n) {
    FILE* f = fopen(p, "rb"); if (!f) { perror(p); exit(2); }
    fseek(f, 0, SEEK_END); *n = (size_t)ftell(f); fseek(f, 0, SEEK_SET);
    unsigned char* b = malloc(*n); if (fread(b, 1, *n, f) != *n) exit(2); fclose(f); return b;
}

static int bad = 0;
static void run(const char* name, const char* rc, uint8_t net, const char* image, const char* anchor_path, int want) {
    size_t rn, an; char msg[256];
    unsigned char *r = slurp(rc, &rn);
    char* anchor = NULL;
    if (anchor_path) { unsigned char* a = slurp(anchor_path, &an); anchor = calloc(an + 1, 1); memcpy(anchor, a, an); free(a); }
    swiftsync_result out; memset(&out, 0, sizeof out);
    int got = swiftsync_verify(r, rn, net, (uint64_t)time(NULL), image, anchor, &out, msg, sizeof msg);
    printf("%s  %-24s verdict %d (want %d)  %s\n", got == want ? "PASS" : "WRONG", name, got, want, msg);
    bad += got != want; free(r); free(anchor);
}

/* A liveness proof (docs/libzync/LIVENESS_ZAPP.md) through swiftsync_verify_facts: `flip` >= 0 flips that hex digit. */
static void live(const char* name, const char* rc, const char* proof_path, const char* ask, int flip, int want) {
    size_t rn, pn; char msg[512];
    unsigned char *r = slurp(rc, &rn), *p = slurp(proof_path, &pn);
    while (pn && (p[pn - 1] == '\n' || p[pn - 1] == '\r')) pn--;
    if (flip >= 0) p[flip] = p[flip] == '0' ? '1' : '0';
    char* facts = malloc(pn + strlen(ask) + 64);
    sprintf(facts, "{\"liveness\": [{\"proof\": \"%.*s\", \"ask\": \"%s\"}]}", (int)pn, (const char*)p, ask);
    swiftsync_result out; memset(&out, 0, sizeof out);
    int got = swiftsync_verify_facts(r, rn, SWIFTSYNC_NET_MAINNET | SWIFTSYNC_IMG_PRODUCT, (uint64_t)time(NULL),
                                     "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4", NULL, facts, NULL, &out, msg, sizeof msg);
    printf("%s  %-24s verdict %d (want %d)  %.150s\n", got == want ? "PASS" : "WRONG", name, got, want, msg);
    bad += got != want; free(r); free(p); free(facts);
}

/* ../swiftsync-verify/vectors/README.md describes every case. */
int main(void) {
    const char* V = "../swiftsync-verify/vectors/";
    const char* OLD = "c3501ac900743bbc911c8f6dedcd71f1ee53fe4bea4f1cbc9726324478e63dae";
    const char* OLD305 = "863c9ce35104740d8b6fa2972f0dd1b29289e3e67e5679400f0e8ecdbef69af5"; /* the same guest on risc0 3.0.5 */
    const uint8_t MP = SWIFTSYNC_NET_MAINNET | SWIFTSYNC_IMG_PRODUCT;
    const uint8_t S = SWIFTSYNC_NET_SIGNET, SP = SWIFTSYNC_NET_SIGNET | SWIFTSYNC_IMG_PRODUCT;
    char a[512], c[512];
    #define P(x, f) (snprintf(x, sizeof x, "%s%s", V, f), x)
    run("retired_design_b", P(a, "r4_block_2600.rcpt"), S | SWIFTSYNC_IMG_COINLOG, OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("retired_design_b_product", P(a, "r4_block_2600.rcpt"), SP, OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("wrong_image", P(a, "r4_block_2600.rcpt"), SP, NULL, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("fake_receipt", P(a, "fake_block_2501.rcpt"), SP, NULL, NULL, SWIFTSYNC_VERIFY_FAIL);
    run("bundle_v1", P(a, "r4_2600.sspf"), SP, OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("bundle_header_lies", P(a, "bad_bundle_header.sspf"), SP, OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("mainnet_1_200_demo", P(a, "mainnet_1_200_8888d59c.rcpt"), MP, NULL, NULL, SWIFTSYNC_VERIFY_DEMO);
    run("mainnet_1_200_as_signet", P(a, "mainnet_1_200_8888d59c.rcpt"), SP, NULL, NULL, SWIFTSYNC_VERIFY_FAIL);
    run("mainnet_1_200_old_image", P(a, "mainnet_1_200_8888d59c.rcpt"), MP, OLD305, NULL, SWIFTSYNC_VERIFY_FAIL);
    run("mainnet_1_200_flipped", P(a, "mainnet_1_200_8888d59c_flipped.rcpt"), MP, NULL, NULL, SWIFTSYNC_VERIFY_FAIL);
    run("risc0_305_receipt", P(a, "mainnet_1_200_863c9ce3_risc0_305.rcpt"), MP, OLD305, NULL, SWIFTSYNC_VERIFY_FAIL);
    run("bad_image_hex", P(a, "r4_block_2600.rcpt"), SP, "xyz", NULL, SWIFTSYNC_VERIFY_FAIL);
    run("unknown_network", P(a, "r4_block_2600.rcpt"), 7, NULL, NULL, SWIFTSYNC_VERIFY_FAIL);
    const char* H = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16:0@200";
    char q[256], l[512], l9[512];
    P(l, "liveness/b170_0_at200.hex"); P(l9, "liveness/b9_0_at200.hex");
    snprintf(q, sizeof q, "%s=unspent,value=1000000000", H);
    live("liveness_unspent", P(a, "mainnet_1_200_8888d59c.rcpt"), l, q, -1, SWIFTSYNC_VERIFY_DEMO); /* low work: DEMO */
    live("liveness_spent", P(a, "mainnet_1_200_8888d59c.rcpt"), l9, "0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9:0@200=spent:170:1:0", -1, SWIFTSYNC_VERIFY_DEMO);
    snprintf(q, sizeof q, "%s=spent", H);
    live("liveness_false_claim", P(a, "mainnet_1_200_8888d59c.rcpt"), l, q, -1, SWIFTSYNC_VERIFY_FAIL);
    live("liveness_no_claim", P(a, "mainnet_1_200_8888d59c.rcpt"), l, H, -1, SWIFTSYNC_VERIFY_FAIL);
    snprintf(q, sizeof q, "%s=unspent,value=999", H);
    live("liveness_wrong_amount", P(a, "mainnet_1_200_8888d59c.rcpt"), l, q, -1, SWIFTSYNC_VERIFY_FAIL);
    snprintf(q, sizeof q, "%s=unspent", H);
    live("liveness_tampered", P(a, "mainnet_1_200_8888d59c.rcpt"), l, q, 200, SWIFTSYNC_VERIFY_FAIL);
    live("liveness_fake_receipt", P(a, "fake_block_2501.rcpt"), l, q, -1, SWIFTSYNC_VERIFY_FAIL);
    printf("%s\n", bad ? "SOME CASES WRONG" : "all cases as expected");
    return bad != 0;
}
