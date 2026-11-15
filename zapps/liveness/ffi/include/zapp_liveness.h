/* zapp_liveness.h: the C ABI of the coin-liveness zyncAPP (zapps/liveness, docs/libzync/LIVENESS_ZAPP.md and
   ZYNCAPPS.md §9), a module of its own on the core verifier. Link zapps/liveness/ffi's libzapp_liveness_ffi.a
   (and -lpthread -ldl -lm). Thread-safe; never aborts on bad input. */
#ifndef ZAPP_LIVENESS_H
#define ZAPP_LIVENESS_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

#define ZAPP_LIVENESS_OK 0
#define ZAPP_LIVENESS_FAIL 1
#define ZAPP_LIVENESS_DEMO 2

/* Verify a core receipt (or .sspf bundle) as swiftsync_verify does, then each liveness proof against it.
 * network: 0 mainnet, 1 signet; | 0x10 a coin-log image, | 0x20 the product image (as swiftsync_verify_facts).
 * image_hex: NULL for the built-in image, else 64 hex digits. anchor_json: a ChainState, or NULL for genesis.
 * facts_json: {"liveness": [{"proof": "<lower-case hex>", "ask": "<txid>:<vout>@<height|tip>=unspent|=spent[:<h>[:<tx>:<vin>]]
 *   [,value=<sat>][,script=<hex>]"}]}: one outpoint's state after a height each. The ask is YOURS and must claim its
 *   answer: the proof must be about exactly that outpoint and height, and the claimed state, amount and script must be
 *   the proof's. expect: NUL-terminated "<height>:<hash>", the network's block at the proof's height from a source YOU
 *   trust, or NULL (then at most DEMO); never inside facts_json (refused there).
 * Returns ZAPP_LIVENESS_OK, _DEMO, or _FAIL (reason in msg). */
int zapp_liveness_verify(const uint8_t* receipt, size_t receipt_len, uint8_t network, uint64_t now, const char* image_hex,
                         const char* anchor_json, const char* facts_json, const char* expect, char* msg, size_t msg_len);

#ifdef __cplusplus
}
#endif
#endif
