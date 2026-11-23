/* Thin calls into REAL libsecp256k1 v0.5.1, the way Core's CPubKey::Verify / XOnlyPubKey::VerifySchnorr do. */
#include <stddef.h>
#include <stdint.h>
#include "secp256k1.h"
#include "secp256k1_extrakeys.h"
#include "secp256k1_schnorrsig.h"
extern void hz_fatal(int code);
void secp256k1_default_illegal_callback_fn(const char *str, void *data) { (void)str; (void)data; hz_fatal(1); }
void secp256k1_default_error_callback_fn(const char *str, void *data) { (void)str; (void)data; hz_fatal(2); }
void abort(void) { hz_fatal(3); for (;;) {} }

int hz_ecdsa(const unsigned char *msg32, const unsigned char *pub33, const unsigned char *sig64) {
    secp256k1_pubkey pk; secp256k1_ecdsa_signature s;
    if (!secp256k1_ec_pubkey_parse(secp256k1_context_static, &pk, pub33, 33)) return -1;
    if (!secp256k1_ecdsa_signature_parse_compact(secp256k1_context_static, &s, sig64)) return -2;
    secp256k1_ecdsa_signature_normalize(secp256k1_context_static, &s, &s);   /* Core: lower-S before verify */
    return secp256k1_ecdsa_verify(secp256k1_context_static, &s, msg32, &pk);
}
int hz_schnorr(const unsigned char *msg32, const unsigned char *x32, const unsigned char *sig64) {
    secp256k1_xonly_pubkey xp;
    if (!secp256k1_xonly_pubkey_parse(secp256k1_context_static, &xp, x32)) return -1;
    return secp256k1_schnorrsig_verify(secp256k1_context_static, sig64, msg32, 32, &xp);
}
