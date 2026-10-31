/* swiftsync_verify.h: check a libzync proof that a SwiftSync hints file is correct (docs/libzync/SWIFTSYNC_PROOFS.md
 * §5), from C or C++. The same checks as the swiftsync-verify CLI; link libswiftsync_verify.a (and -lpthread -ldl -lm).
 * Thread-safe; no global state; never aborts on bad input. */
#ifndef SWIFTSYNC_VERIFY_H
#define SWIFTSYNC_VERIFY_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

#define SWIFTSYNC_VERIFY_OK   0  /* genesis-anchored: the file is correct for the chain in `out` */
#define SWIFTSYNC_VERIFY_FAIL 1
#define SWIFTSYNC_VERIFY_DEMO 2  /* passed against a supplied anchor only: never a proof of the file */

#define SWIFTSYNC_NET_MAINNET 0
#define SWIFTSYNC_NET_SIGNET  1

typedef struct {
    uint32_t height;         /* H */
    uint8_t  tip_hash[32];   /* the proven block at H, internal byte order (as uint256 holds it) */
    uint8_t  chainwork[32];  /* cumulative work at H, 32 bytes little-endian (as arith_uint256 serialises) */
    uint64_t utxo_leaves;    /* coins in the UTXO set at H */
    uint32_t profile;        /* bit 0 libzync, bit 1 scripts NOT checked (assumevalid), bit 2 signet */
} swiftsync_result;

/* Verify `receipt` (a .rcpt receipt or a .sspf bundle) against the hints file bytes.
 *   network:     SWIFTSYNC_NET_*.
 *   now:         the caller's clock, unix seconds (for time-too-new, §5 step 11).
 *   image_hex:   NULL for the built-in image id, or 64 hex digits (an id you rebuilt, §5 step 9).
 *   anchor_json: NULL for genesis, or a ChainState JSON; the result is then at best SWIFTSYNC_VERIFY_DEMO.
 *   out:         filled when the receipt decoded (any verdict); may be NULL.
 *   msg/msg_len: optional buffer for a NUL-terminated description of the first failed check.
 * Step 7 of §5 is the caller's: `out->tip_hash` must be the block at `out->height` on its own most-work chain,
 * with `out->chainwork`. Returns SWIFTSYNC_VERIFY_OK, _DEMO or _FAIL. */
int swiftsync_verify(const uint8_t* receipt, size_t receipt_len, const uint8_t* hints, size_t hints_len,
                     uint8_t network, uint64_t now, const char* image_hex, const char* anchor_json,
                     swiftsync_result* out, char* msg, size_t msg_len);

/* The built-in image id for `network`, as 64 hex digits and a NUL; NULL for an unknown network. Static storage. */
const char* swiftsync_image_id(uint8_t network);

#ifdef __cplusplus
}
#endif
#endif
