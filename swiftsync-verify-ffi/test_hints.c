/* test_hints.c: the C ABI's swiftsync_verify_hints on a real hints receipt (H = 3) and refusals.
 *   cc -O2 -Iinclude test_hints.c target/release/libswiftsync_verify.a -lpthread -ldl -lm -o test_hints
 *   ./test_hints <dir with mainnet-3.{receipt,hints,undo}> <hash of block 3> */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "swiftsync_verify.h"

static unsigned char* slurp(const char* d, const char* f, size_t* n) {
    char p[4096]; snprintf(p, sizeof p, "%s/%s", d, f);
    FILE* fp = fopen(p, "rb"); if (!fp) { perror(p); exit(2); }
    fseek(fp, 0, SEEK_END); *n = (size_t)ftell(fp); fseek(fp, 0, SEEK_SET);
    unsigned char* b = malloc(*n); if (fread(b, 1, *n, fp) != *n) exit(2); fclose(fp); return b;
}

int main(int argc, char** argv) {
    if (argc != 3) return 2;
    size_t rn, fn, un; char msg[512]; char exp[128]; int bad = 0;
    unsigned char *r = slurp(argv[1], "mainnet-3.receipt", &rn), *f = slurp(argv[1], "mainnet-3.hints", &fn), *u = slurp(argv[1], "mainnet-3.undo", &un);
    snprintf(exp, sizeof exp, "3:%s", argv[2]);
    SwiftsyncHints out; memset(&out, 0, sizeof out);
    int rc = swiftsync_verify_hints(r, rn, f, fn, u, un, (uint64_t)time(NULL), exp, &out, msg, sizeof msg);
    printf("%s proven file: rc %d height %u unspent %llu (%s)\n", rc == 2 && out.height == 3 ? "PASS" : "WRONG", rc, out.height, (unsigned long long)out.n_unspent, msg); bad += !(rc == 2 && out.height == 3);
    f[fn - 1] ^= 1;
    rc = swiftsync_verify_hints(r, rn, f, fn, u, un, (uint64_t)time(NULL), exp, &out, msg, sizeof msg);
    printf("%s flipped bit: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    f[fn - 1] ^= 1; u[0] ^= 1;
    rc = swiftsync_verify_hints(r, rn, f, fn, u, un, (uint64_t)time(NULL), exp, &out, msg, sizeof msg);
    printf("%s tampered undo: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    u[0] ^= 1;
    rc = swiftsync_verify_hints(r, rn, f, fn, u, un, (uint64_t)time(NULL), "3:0000000000000000000000000000000000000000000000000000000000000001", &out, msg, sizeof msg);
    printf("%s wrong hash at H: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    printf("HINTS_C_ABI %s\n", bad ? "FAIL" : "OK");
    return bad != 0;
}
