#!/usr/bin/env python3
"""Insert Hazync's batch hooks (same call sites and kinds as patches/libzync/0102) into a PRISTINE libsecp v0.5.1.
Difference from 0102: field elements reach the hook through libsecp's own secp256k1_fe_get_b32 (works on the
stock 10x26 backend), not as raw 8x32 limbs (which needed Hazync's risc0 field backend, patch 0012)."""
import sys
secp = sys.argv[1]
def patch(rel, anchor, ins, before=False):
    p = f"{secp}/{rel}"; s = open(p).read()
    if "HAZYNC_BATCH_ECDSA" in s and "hz_fe_limbs" in ins: pass
    assert s.count(anchor) == 1, (rel, anchor)
    s = s.replace(anchor, ins + anchor if before else anchor + ins)
    open(p, "w").write(s)
HELPER = """#ifdef HAZYNC_BATCH_ECDSA
static void hz_fe_limbs(const secp256k1_fe *a, uint32_t *out) {
    secp256k1_fe t = *a; unsigned char b[32]; int k;
    secp256k1_fe_normalize_var(&t);
    secp256k1_fe_get_b32(b, &t);
    for (k = 0; k < 8; k++) out[k] = ((uint32_t)b[28-4*k] << 24) | ((uint32_t)b[29-4*k] << 16) | ((uint32_t)b[30-4*k] << 8) | b[31-4*k];
}
extern int hazync_batch_defer(int kind, const uint32_t *u1, const uint32_t *u2, const uint32_t *qx, const uint32_t *qy, const uint32_t *r);
#endif
"""
patch("src/ecdsa_impl.h", "static int secp256k1_ecdsa_sig_verify(", HELPER, before=True)
patch("src/ecdsa_impl.h", """    if (secp256k1_scalar_is_zero(sigr) || secp256k1_scalar_is_zero(sigs)) {
        return 0;
    }
""", """#ifdef HAZYNC_BATCH_ECDSA
    {   /* kind 2: (e, s, Q, r) before libsecp's scalar inverse; the batch computes u1 = e/s, u2 = r/s */
        uint32_t hx[8], hy[8];
        hz_fe_limbs(&pubkey->x, hx); hz_fe_limbs(&pubkey->y, hy);
        if (hazync_batch_defer(2, message->d, sigs->d, hx, hy, sigr->d) == 1) return 1;
    }
#endif
""")
patch("src/modules/schnorrsig/main_impl.h", """    secp256k1_scalar_negate(&e, &e);
    secp256k1_gej_set_ge(&pkj, &pk);
""", """#ifdef HAZYNC_BATCH_ECDSA
    {   /* kind 1: (s, -e, P, r_x) */
        uint32_t hx[8], hy[8], hr[8];
        hz_fe_limbs(&pk.x, hx); hz_fe_limbs(&pk.y, hy); hz_fe_limbs(&rx, hr);
        if (hazync_batch_defer(1, s.d, e.d, hx, hy, hr) == 1) return 1;
    }
#endif
""")
patch("src/group_impl.h", "    ret = secp256k1_fe_sqrt(&r->y, &x3);\n", """#ifdef HAZYNC_LIFTX_OVM
    {   /* like Hazync's HAZYNC_LIFTX_ACCEL (patch 0007): ONLY the square root goes to the accelerator */
        extern int hz_fe_sqrt(const unsigned char *a_be, unsigned char *out_be);
        unsigned char ab[32], yb[32];
        secp256k1_fe t = x3; secp256k1_fe_normalize_var(&t); secp256k1_fe_get_b32(ab, &t);
        ret = hz_fe_sqrt(ab, yb);
        if (ret) { ret = secp256k1_fe_set_b32_limit(&r->y, yb); }
    }
#else
""", before=True)
patch("src/group_impl.h", "    ret = secp256k1_fe_sqrt(&r->y, &x3);\n", "#endif\n")
print("hooks inserted")
