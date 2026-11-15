/* zapp_hints.h: the C ABI of the SwiftSync hints zyncAPP (zapps/hints, docs/libzync/ZYNCAPPS.md §9), a module of its
   own on the core verifier. Link zapps/hints/ffi's libzapp_hints_ffi.a. */
#ifndef ZAPP_HINTS_H
#define ZAPP_HINTS_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

/* The SwiftSync hints zyncAPP (docs/libzync/HINTS_ZAPP.md): `receipt` proves that `file` is the hints file for block H
   of a chain a trusted core image proved from genesis. `undo` (may be NULL) is the per-block undo-hash file 1..H. `expect`
   (NUL-terminated "H:hash", may be NULL) is block H from YOUR node's header chain: without it the verdict is at most DEMO.
   Returns 0 OK, 2 DEMO, 1 FAIL (reason in msg); `out` is filled unless FAIL. A core tip above H is DEMO here (use the CLI). */
typedef struct {
    uint32_t height;
    uint8_t block_hash[32];   /* internal byte order */
    uint32_t format;          /* 1: hintsfile.md, the BIP vectors' bit order; 2: the hintsfile crate 0.2.0's */
    uint64_t n_unspent;
    uint64_t unspent_value;
} SwiftsyncHints;
int swiftsync_verify_hints(const uint8_t* receipt, size_t receipt_len, const uint8_t* file, size_t file_len,
                           const uint8_t* undo, size_t undo_len, uint64_t now, const char* expect,
                           SwiftsyncHints* out, char* msg, size_t msg_len);

#ifdef __cplusplus
}
#endif
#endif
