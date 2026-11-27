/* zapp_hints.h: the C ABI of the SwiftSync hints zyncAPP (zapps/hints, docs/libzync/ZYNCAPPS.md §9), a module of its
   own on the core verifier. Link zapps/hints/ffi's libzapp_hints_ffi.a (and -lpthread -ldl -lm) on its own; beside the
   core verifier's libswiftsync_verify.a, link libzapp_hints_ffi.so instead: two Rust static archives in one binary each
   carry a Rust std and clash at link time (multiple definitions). ffi/test_with_core.c, run by ../test.sh, does that. */
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
/* receipt: an engine (OpenVM) proof file of the hints image image_hex (64 hex digits), resting on the engine core image
 * core_image_hex; vk/vk_len: the hints image's verifying key file and vk_pin its 32-byte SHA-256, which you hold as the
 * image's published pin (or your own build's); a key whose SHA-256 is not vk_pin is FAIL. Engine proofs only since
 * 27.11.2026; no engine image is published yet, so every one of these is required. */
int swiftsync_verify_hints(const uint8_t* receipt, size_t receipt_len, const uint8_t* file, size_t file_len,
                           const uint8_t* undo, size_t undo_len, const char* image_hex, const char* core_image_hex,
                           const uint8_t* vk, size_t vk_len, const uint8_t* vk_pin, uint64_t now, const char* expect,
                           SwiftsyncHints* out, char* msg, size_t msg_len);

#ifdef __cplusplus
}
#endif
#endif
