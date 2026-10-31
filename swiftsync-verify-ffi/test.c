/* test.c: the C ABI on the real round-4 signet receipt and corrupted inputs (../swiftsync-verify/vectors).
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
static void run(const char* name, const char* rc, const char* hf, const char* image, const char* anchor_path, int want) {
    size_t rn, hn, an; char msg[256];
    unsigned char *r = slurp(rc, &rn), *h = slurp(hf, &hn);
    char* anchor = NULL;
    if (anchor_path) { unsigned char* a = slurp(anchor_path, &an); anchor = calloc(an + 1, 1); memcpy(anchor, a, an); free(a); }
    swiftsync_result out; memset(&out, 0, sizeof out);
    int got = swiftsync_verify(r, rn, h, hn, SWIFTSYNC_NET_SIGNET, (uint64_t)time(NULL), image, anchor, &out, msg, sizeof msg);
    printf("%s  %-20s verdict %d (want %d)  height %u leaves %llu  %s\n", got == want ? "PASS" : "WRONG", name, got, want,
           out.height, (unsigned long long)out.utxo_leaves, msg);
    bad += got != want; free(r); free(h); free(anchor);
}

int main(void) {
    const char* V = "../swiftsync-verify/vectors/";
    const char* OLD = "c3501ac900743bbc911c8f6dedcd71f1ee53fe4bea4f1cbc9726324478e63dae";
    char a[512], b[512], c[512];
    #define P(x, f) (snprintf(x, sizeof x, "%s%s", V, f), x)
    printf("built-in signet image: %s\n", swiftsync_image_id(SWIFTSYNC_NET_SIGNET));
    run("demo_receipt", P(a, "r4_block_2600.rcpt"), P(b, "signet2600.hints"), OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_DEMO);
    run("demo_bundle", P(a, "r4_2600.sspf"), P(b, "signet2600.hints"), OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_DEMO);
    run("no_anchor", P(a, "r4_block_2600.rcpt"), P(b, "signet2600.hints"), OLD, NULL, SWIFTSYNC_VERIFY_FAIL);
    run("wrong_image", P(a, "r4_block_2600.rcpt"), P(b, "signet2600.hints"), NULL, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("flipped_bit", P(a, "r4_block_2600.rcpt"), P(b, "bad_flipped_bit.hints"), OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("early_stop", P(a, "r4_block_2530.rcpt"), P(b, "signet2600.hints"), OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("fake_receipt", P(a, "fake_block_2501.rcpt"), P(b, "signet2600.hints"), NULL, NULL, SWIFTSYNC_VERIFY_FAIL);
    run("bundle_header_lies", P(a, "bad_bundle_header.sspf"), P(b, "signet2600.hints"), OLD, P(c, "r4_anchor_2500.json"), SWIFTSYNC_VERIFY_FAIL);
    run("bad_image_hex", P(a, "r4_block_2600.rcpt"), P(b, "signet2600.hints"), "xyz", NULL, SWIFTSYNC_VERIFY_FAIL);
    printf("%s\n", bad ? "FFI_TEST_FAIL" : "FFI_TEST_OK");
    return bad != 0;
}
