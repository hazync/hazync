/* test_with_core.c: the assume zAPP's C ABI and the core verifier's (swiftsync-verify-ffi) in ONE program, as a node
 * patch would use them: each answers. Built by ../test.sh against the core's libswiftsync_verify.a and this zAPP's
 * libzapp_assume_ffi.so (two Rust staticlibs in one binary each carry a Rust std and clash: link the zAPP's cdylib). */
#include <stdio.h>
#include <string.h>
#include "swiftsync_verify.h"
#include "zapp_assume.h"

int main(void) {
    const char* id = swiftsync_image_id(SWIFTSYNC_NET_MAINNET);
    char msg[256]; zapp_assume_chain out; swiftsync_assume_result a; memset(&out, 0, sizeof out); memset(&a, 0, sizeof a);
    unsigned char junk[9] = {1, 2, 3, 4, 5, 6, 7, 8, 9};
    const uint8_t tip[32] = {0};
    int rc = swiftsync_verify_assume(junk, sizeof junk, ZAPP_ASSUME_NET_MAINNET, 0, NULL, NULL, 0, NULL, NULL, tip, NULL, 0, NULL, 0, NULL,
                                     &out, &a, msg, sizeof msg);
    /* no engine image is published yet: the core's id is NULL until one is */
    int ok = (id == NULL || strlen(id) == 64) && rc == ZAPP_ASSUME_FAIL;
    printf("%s core image id %.8s..., zAPP refuses junk: rc %d (%.60s)\n", ok ? "PASS" : "WRONG", id ? id : "(null)", rc, msg);
    printf("CORE_AND_ZAPP_IN_ONE_BINARY %s\n", ok ? "OK" : "FAIL");
    return !ok;
}
