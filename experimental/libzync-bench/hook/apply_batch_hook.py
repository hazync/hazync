"""Insert the libzync batch-ECDSA hook into a libsecp tree (stock or patch-0005'd).

Usage: python3 -I apply_batch_hook.py <secp256k1 dir>

The hook sits after libsecp has computed u1 = e/s and u2 = r/s, and before the ecmult. It hands
(u1, u2, Q, r) to the guest's Rust side. A return of 1 means "deferred into the end-of-chunk batch,
treat as valid". Anything else falls through to libsecp's own verification, unchanged.
Everything is #ifdef HAZYNC_BATCH_ECDSA, so a build without the define compiles as before.
"""
import sys

path = sys.argv[1] + "/src/ecdsa_impl.h"
src = open(path).read()
anchor = "    secp256k1_gej_set_ge(&pubkeyj, pubkey);\n"
hook = anchor + """#ifdef HAZYNC_BATCH_ECDSA
    {
        unsigned char bu1[32], bu2[32], bqx[32], bqy[32], br[32];
        secp256k1_ge q = *pubkey;
        extern int hazync_batch_ecdsa(const unsigned char *u1, const unsigned char *u2,
                                      const unsigned char *qx, const unsigned char *qy,
                                      const unsigned char *r);
        secp256k1_fe_normalize_var(&q.x);
        secp256k1_fe_normalize_var(&q.y);
        secp256k1_scalar_get_b32(bu1, &u1);
        secp256k1_scalar_get_b32(bu2, &u2);
        secp256k1_fe_get_b32(bqx, &q.x);
        secp256k1_fe_get_b32(bqy, &q.y);
        secp256k1_scalar_get_b32(br, sigr);
        if (hazync_batch_ecdsa(bu1, bu2, bqx, bqy, br) == 1) {
            return 1;
        }
    }
#endif
"""
assert src.count(anchor) == 1, f"anchor found {src.count(anchor)} times in {path}"
assert "HAZYNC_BATCH_ECDSA" not in src, "hook already applied"
open(path, "w").write(src.replace(anchor, hook))
print(f"batch hook inserted into {path}")
