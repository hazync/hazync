/* test_hints.c: the C ABI's swiftsync_verify_hints on the engine's hints proof for H = 200 (tests/data/engine:
 * keys-z-hints over keys-ch's core proof of mainnet 1..200) and refusals.
 *   cc -O2 -Iinclude test_hints.c target/release/libzapp_hints_ffi.a -lpthread -ldl -lm -o test_hints
 *   ./test_hints <tests/data/engine> <hash of block 200> <core image id> */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "zapp_hints.h"

static unsigned char* slurp(const char* d, const char* f, size_t* n) {
    char p[4096]; snprintf(p, sizeof p, "%s/%s", d, f);
    FILE* fp = fopen(p, "rb"); if (!fp) { perror(p); exit(2); }
    fseek(fp, 0, SEEK_END); *n = (size_t)ftell(fp); fseek(fp, 0, SEEK_SET);
    unsigned char* b = malloc(*n + 1); if (fread(b, 1, *n, fp) != *n) exit(2); fclose(fp); b[*n] = 0; return b;
}

int main(int argc, char** argv) {
    if (argc != 4) return 2;
    size_t rn, fn, un, vn, n; char msg[512]; char exp[128]; int bad = 0;
    unsigned char *r = slurp(argv[1], "proof.hze", &rn), *f = slurp(argv[1], "mainnet-200.hints", &fn), *u = slurp(argv[1], "mainnet-200.undo", &un);
    unsigned char *vk = slurp(argv[1], "vk.bin", &vn), pin[32];
    char *id = (char*)slurp(argv[1], "image_id", &n), *ph = (char*)slurp(argv[1], "vk.sha256", &n);
    for (char* c = id; *c; c++) if (*c == '\n') *c = 0;
    for (int i = 0; i < 32; i++) sscanf(ph + 2 * i, "%2hhx", &pin[i]);
    const char* core = argv[3];
    snprintf(exp, sizeof exp, "200:%s", argv[2]);
    uint64_t now = (uint64_t)time(NULL);
    SwiftsyncHints out; memset(&out, 0, sizeof out);
    const char* net = NULL; uint32_t fmt = 0;  /* NULL: mainnet; 0: the format not compared */
    #define CALL(R, RN, F, FN, U, UN, ID, CORE, P, E) swiftsync_verify_hints(R, RN, F, FN, U, UN, ID, CORE, vk, vn, P, net, fmt, now, E, &out, msg, sizeof msg)
    int rc = CALL(r, rn, f, fn, u, un, id, core, pin, exp);
    printf("%s proven file: rc %d height %u unspent %llu (%s)\n", rc == 2 && out.height == 200 ? "PASS" : "WRONG", rc, out.height, (unsigned long long)out.n_unspent, msg); bad += !(rc == 2 && out.height == 200);
    /* review R3-4 L1, L2: the node's network and format are the caller's */
    uint32_t real_fmt = out.format;
    fmt = real_fmt; net = "mainnet";
    rc = CALL(r, rn, f, fn, u, un, id, core, pin, exp);
    printf("%s the node's network and format (still DEMO: below minimum work): rc %d (%.90s)\n", rc == 2 ? "PASS" : "WRONG", rc, msg); bad += rc != 2;
    fmt = real_fmt == 1 ? 2 : 1;
    rc = CALL(r, rn, f, fn, u, un, id, core, pin, exp);
    printf("%s a node that reads the other format: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    fmt = 0; net = "signet";
    rc = CALL(r, rn, f, fn, u, un, id, core, pin, exp);
    printf("%s a signet node: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    net = "testnet3";
    rc = CALL(r, rn, f, fn, u, un, id, core, pin, exp);
    printf("%s an unknown network: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    net = NULL; fmt = 7;
    rc = CALL(r, rn, f, fn, u, un, id, core, pin, exp);
    printf("%s an unknown format number: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    fmt = 0;
    f[fn - 1] ^= 1;
    rc = CALL(r, rn, f, fn, u, un, id, core, pin, exp);
    printf("%s flipped bit: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    f[fn - 1] ^= 1; u[0] ^= 1;
    rc = CALL(r, rn, f, fn, u, un, id, core, pin, exp);
    printf("%s tampered undo: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    u[0] ^= 1;
    rc = CALL(r, rn, f, fn, u, un, id, core, pin, "200:0000000000000000000000000000000000000000000000000000000000000001");
    printf("%s wrong hash at H: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    unsigned char tiny[9] = {0x55, 0x54, 0x58, 0x4f, 0, 0xff, 0xff, 0xff, 0xff};
    rc = CALL(r, rn, tiny, 9, NULL, 0, id, core, pin, exp);
    printf("%s 9-byte file claiming H = 2^32 - 1: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    unsigned char pin2[32]; memcpy(pin2, pin, 32); pin2[0] ^= 1;
    rc = CALL(r, rn, f, fn, u, un, id, core, pin2, exp);
    printf("%s another pin: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    rc = CALL(r, rn, f, fn, u, un, id, id, pin, exp);
    printf("%s another core image: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    rc = CALL(r, rn, f, fn, u, un, NULL, core, pin, exp);
    printf("%s no image id: rc %d (%.90s)\n", rc == 1 ? "PASS" : "WRONG", rc, msg); bad += rc != 1;
    printf("HINTS_C_ABI %s\n", bad ? "FAIL" : "OK");
    return bad != 0;
}
