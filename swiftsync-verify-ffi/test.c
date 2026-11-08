/* test.c: the C ABI on the vectors (../swiftsync-verify/vectors): refusals only until the first GPU proofs of the frozen image.
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

/* The vectors are refusals only until the first GPU proofs of the frozen image (../swiftsync-verify/vectors/README.md). */
int main(void) {
    const char* V = "../swiftsync-verify/vectors/";
    const char* OLD = "c3501ac900743bbc911c8f6dedcd71f1ee53fe4bea4f1cbc9726324478e63dae";
    const uint8_t S = SWIFTSYNC_NET_SIGNET, SP = SWIFTSYNC_NET_SIGNET | SWIFTSYNC_IMG_PRODUCT;
    char a[512], c[512];
    #define P(x, f) (snprintf(x, sizeof x, "%s%s", V, f), x)
    run("retired_design_b", P(a, "r4_block_2600.rcpt"), S | SWIFTSYNC_IMG_COINLOG, OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("retired_design_b_product", P(a, "r4_block_2600.rcpt"), SP, OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("wrong_image", P(a, "r4_block_2600.rcpt"), SP, NULL, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("fake_receipt", P(a, "fake_block_2501.rcpt"), SP, NULL, NULL, SWIFTSYNC_VERIFY_FAIL);
    run("bundle_v1", P(a, "r4_2600.sspf"), SP, OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("bundle_header_lies", P(a, "bad_bundle_header.sspf"), SP, OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("bad_image_hex", P(a, "r4_block_2600.rcpt"), SP, "xyz", NULL, SWIFTSYNC_VERIFY_FAIL);
    run("unknown_network", P(a, "r4_block_2600.rcpt"), 7, NULL, NULL, SWIFTSYNC_VERIFY_FAIL);
    printf("%s\n", bad ? "SOME CASES WRONG" : "all cases as expected");
    return bad != 0;
}
