"""Insert the libzync batch hooks into a libsecp tree (stock, or with GHOST's patches applied).

Usage: python3 -I apply_batch_hook.py <secp256k1 dir>

Two hooks, both #ifdef HAZYNC_BATCH_ECDSA, so a build without the define compiles as before:

- ECDSA (src/ecdsa_impl.h, secp256k1_ecdsa_sig_verify): after libsecp has checked r,s != 0 and
  computed u1 = e/s, u2 = r/s, hand (u1, u2, Q, r) over as libsecp's own 8x32 little-endian limbs.
- Schnorr (src/modules/schnorrsig/main_impl.h, secp256k1_schnorrsig_verify): after libsecp has
  parsed s, loaded P and computed (and negated) e, hand (s, -e, P, r_x) over. BIP340's equation
  R = s*G + (-e)*P has the same linear shape as ECDSA's, so both share one batch.

A return of 1 means "deferred into the end-of-chunk batch, treat as valid"; anything else falls
through to libsecp's own verification, unchanged. Re-running replaces an earlier version of a hook.

⚠ Passing limbs relies on the field_bigint2 backend (patches/0012: 8x32 canonical limbs after
normalize) and on the 8x32 scalar libsecp selects on rv32. Both hold for CORE and GHOST.
"""
import re
import sys

BEGIN, END = "#ifdef HAZYNC_BATCH_ECDSA\n", "#endif /* HAZYNC_BATCH_ECDSA */\n"

ECDSA = BEGIN + """    {
        secp256k1_ge q = *pubkey;
        extern int hazync_batch_defer(int kind, const uint32_t *u1, const uint32_t *u2,
                                      const uint32_t *qx, const uint32_t *qy, const uint32_t *r);
        secp256k1_fe_normalize_var(&q.x);
        secp256k1_fe_normalize_var(&q.y);
        if (hazync_batch_defer(0, u1.d, u2.d, q.x.n, q.y.n, sigr->d) == 1) {
            return 1;
        }
    }
""" + END

SCHNORR = BEGIN + """    {
        secp256k1_ge pk_n = pk;
        secp256k1_fe rx_n = rx;
        extern int hazync_batch_defer(int kind, const uint32_t *u1, const uint32_t *u2,
                                      const uint32_t *qx, const uint32_t *qy, const uint32_t *r);
        secp256k1_fe_normalize_var(&pk_n.x);
        secp256k1_fe_normalize_var(&pk_n.y);
        secp256k1_fe_normalize_var(&rx_n);
        if (hazync_batch_defer(1, s.d, e.d, pk_n.x.n, pk_n.y.n, rx_n.n) == 1) {
            return 1;
        }
    }
""" + END


def install(path, anchor, hook):
    src = open(path).read()
    # Remove any earlier version: the round-1 hook ended with a bare "#endif", so match it by body.
    src = re.sub(r"#ifdef HAZYNC_BATCH_ECDSA\n.*?hazync_batch_ecdsa\(bu1.*?\n    \}\n#endif\n", "", src,
                 flags=re.S)
    src = re.sub(re.escape(BEGIN) + r".*?" + re.escape(END), "", src, flags=re.S)
    assert src.count(anchor) == 1, f"anchor found {src.count(anchor)} times in {path}"
    open(path, "w").write(src.replace(anchor, anchor + hook))
    assert open(path).read().count("hazync_batch_defer") == 2  # declaration + call
    print(f"hook installed in {path}")


base = sys.argv[1]
install(base + "/src/ecdsa_impl.h", "    secp256k1_gej_set_ge(&pubkeyj, pubkey);\n", ECDSA)
install(base + "/src/modules/schnorrsig/main_impl.h", "    secp256k1_gej_set_ge(&pkj, &pk);\n", SCHNORR)
