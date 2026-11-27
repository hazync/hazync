/* test_with_core.c: the hints zAPP's C ABI and the core verifier's (swiftsync-verify-ffi) in ONE program, as a node
 * patch would use them: each answers. Built by ../test.sh against the core's libswiftsync_verify.a and this zAPP's
 * libzapp_hints_ffi.so (two Rust staticlibs in one binary each carry a Rust std and clash: link the zAPP's cdylib). */
#include <stdio.h>
#include <string.h>
#include "swiftsync_verify.h"
#include "zapp_hints.h"

int main(void) {
    const char* id = swiftsync_image_id(SWIFTSYNC_NET_MAINNET);
    char msg[256]; SwiftsyncHints out; memset(&out, 0, sizeof out);
    unsigned char tiny[9] = {0x55, 0x54, 0x58, 0x4f, 0, 0xff, 0xff, 0xff, 0xff};
    int rc = swiftsync_verify_hints(tiny, sizeof tiny, tiny, sizeof tiny, NULL, 0, NULL, NULL, NULL, 0, NULL, 0, NULL, &out, msg, sizeof msg);
    /* no engine image is published yet: the core's id is NULL until one is */
    int ok = (id == NULL || strlen(id) == 64) && rc == 1;
    printf("%s core image id %.8s..., zAPP refuses junk: rc %d (%.60s)\n", ok ? "PASS" : "WRONG", id ? id : "(null)", rc, msg);
    printf("CORE_AND_ZAPP_IN_ONE_BINARY %s\n", ok ? "OK" : "FAIL");
    return !ok;
}
