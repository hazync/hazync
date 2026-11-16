/* zapp_assume.h: the C ABI of the assumevalid and assumeutxo zyncAPPs (zapps/assume, docs/libzync/ASSUME_ZAPPS.md and
   ZYNCAPPS.md §9), a module of its own on the core verifier. Link zapps/assume/ffi's libzapp_assume_ffi.a (and
   -lpthread -ldl -lm) on its own; beside the core verifier's libswiftsync_verify.a, link libzapp_assume_ffi.so instead:
   two Rust static archives in one binary each carry a Rust std and clash at link time (multiple definitions).
   ffi/test_with_core.c, run by ../test.sh, does that. Thread-safe; never aborts on bad input. */
#ifndef ZAPP_ASSUME_H
#define ZAPP_ASSUME_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

#define ZAPP_ASSUME_OK 0
#define ZAPP_ASSUME_FAIL 1
#define ZAPP_ASSUME_DEMO 2
#define ZAPP_ASSUME_NET_MAINNET 0
#define ZAPP_ASSUME_NET_SIGNET 1

/* The verified chain: the core's swiftsync_result, field for field. */
typedef struct {
    uint32_t height;
    uint8_t  tip_hash[32];   /* internal byte order */
    uint8_t  chainwork[32];  /* little-endian */
    uint64_t utxo_leaves;
    uint32_t profile;
} zapp_assume_chain;

/* The assumevalid and assumeutxo checks (docs/libzync/ASSUME_ZAPPS.md), after the core verifier's receipt checks (as
 * swiftsync_verify_facts makes them) (genesis only: no anchor; a .rcpt receipt, not a bundle).
 *   expect:            as swiftsync_verify_facts; OK needs it to match (and enough work), else at most DEMO.
 *   assumevalid_block: NULL for no assumevalid check; 32 bytes, internal order: the block to name as -assumevalid,
 *                      or 32 zero bytes for the proven tip. Any block but the tip needs `headers` from it to the tip.
 *   headers:           raw 80-byte headers, oldest first, the last one the proven tip (NULL for none). The snapshot
 *                      check needs them from genesis: they give each coin's creation MTP.
 *   snapshot:          a Core `dumptxoutset` file (NULL for no assumeutxo check). Its base block must be the proven tip.
 *   tracked:           NUL-terminated "<txid>:<vout>" lines, the BIP30-tracked coins (scripts/libzync/assume_inputs.py), or NULL.
 *   aout:              under OK and DEMO: the block and height named, the snapshot's coin count and amount, and Core's
 *                      hash_serialized_3 of it (64 hex digits and a NUL; the value a node pins in m_assumeutxo_data).
 * A wrong headers file or tracked list can only make a true snapshot fail. Returns OK, DEMO or FAIL (reason in msg). */
typedef struct {
    uint8_t  assumevalid_block[32];  /* internal order */
    uint32_t assumevalid_height;
    uint64_t snapshot_coins;
    uint64_t snapshot_value;         /* satoshis */
    char     hash_serialized_3[65];
} swiftsync_assume_result;

int swiftsync_verify_assume(const uint8_t* receipt, size_t receipt_len, uint8_t network, uint64_t now,
                            const char* image_hex, const char* expect, const uint8_t* assumevalid_block,
                            const uint8_t* headers, size_t headers_len, const uint8_t* snapshot, size_t snapshot_len,
                            const char* tracked, zapp_assume_chain* out, swiftsync_assume_result* aout,
                            char* msg, size_t msg_len);

#ifdef __cplusplus
}
#endif
#endif
