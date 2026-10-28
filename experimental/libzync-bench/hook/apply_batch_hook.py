"""Insert the libzync batch hooks into a libsecp tree (stock, or with GHOST's patches applied).

Usage: python3 -I apply_batch_hook.py <secp256k1 dir>

Two hooks, both #ifdef HAZYNC_BATCH_ECDSA, so a build without the define compiles as before:

- ECDSA (src/ecdsa_impl.h, secp256k1_ecdsa_sig_verify): right after libsecp has checked r,s != 0,
  hand (e, s, Q, r) over as libsecp's own 8x32 little-endian limbs (round 6; round 4-5 handed over
  u1, u2 after libsecp's own scalar arithmetic).
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
        /* kind 2: (e, s, Q, r) BEFORE libsecp's scalar inverse and the two scalar products --
         * the batch computes u1 = e/s and u2 = r/s on the coprocessor, for a few hundred cycles
         * against ~2.7k of libsecp's software scalar arithmetic. On fallback libsecp does it all. */
        if (hazync_batch_defer(2, message->d, sigs->d, q.x.n, q.y.n, sigr->d) == 1) {
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

TWEAK = BEGIN + """    {
        /* kind 3 / 4: BIP341 tweak check Q == P + t*G with y(Q) even / odd. The same linear shape as
         * the signature equations (u1 = t, u2 = 1, the internal key P as "Q", the output key as R),
         * so it joins the same end-of-chunk batch. Any value libsecp would reject (tweak >= n,
         * output x >= p) falls through to libsecp unchanged. */
        secp256k1_scalar hz_t;
        secp256k1_fe hz_qx;
        int hz_overflow = 0;
        secp256k1_scalar_set_b32(&hz_t, tweak32, &hz_overflow);
        if (!hz_overflow && secp256k1_fe_set_b32_limit(&hz_qx, tweaked_pubkey32)
            && (tweaked_pk_parity == 0 || tweaked_pk_parity == 1)) {
            extern int hazync_batch_defer(int kind, const uint32_t *u1, const uint32_t *u2,
                                          const uint32_t *qx, const uint32_t *qy, const uint32_t *r);
            secp256k1_ge hz_p = pk;
            secp256k1_fe_normalize_var(&hz_p.x);
            secp256k1_fe_normalize_var(&hz_p.y);
            secp256k1_fe_normalize_var(&hz_qx);
            if (hazync_batch_defer(3 + tweaked_pk_parity, hz_t.d, secp256k1_scalar_one.d,
                                   hz_p.x.n, hz_p.y.n, hz_qx.n) == 1) {
                return 1;
            }
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
install(base + "/src/ecdsa_impl.h",
        "    if (secp256k1_scalar_is_zero(sigr) || secp256k1_scalar_is_zero(sigs)) {\n        return 0;\n    }\n",
        ECDSA)
install(base + "/src/modules/schnorrsig/main_impl.h", "    secp256k1_gej_set_ge(&pkj, &pk);\n", SCHNORR)
# libzync round 13: the BIP341 tweak check (taproot SCRIPT-path spends). libsecp loads the internal key
# and applies the tweak in ONE condition, so it is split: load, then the hook, then libsecp's own tweak.
def install_tweak(path):
    src = open(path).read()
    src = re.sub(re.escape(BEGIN) + r".*?" + re.escape(END), "", src, flags=re.S)
    joined = """    if (!secp256k1_xonly_pubkey_load(ctx, &pk, internal_pubkey)
        || !secp256k1_ec_pubkey_tweak_add_helper(&pk, tweak32)) {
        return 0;
    }
"""
    split = """    if (!secp256k1_xonly_pubkey_load(ctx, &pk, internal_pubkey)) {
        return 0;
    }
    if (!secp256k1_ec_pubkey_tweak_add_helper(&pk, tweak32)) {
        return 0;
    }
"""
    # The same joined condition also opens secp256k1_xonly_pubkey_tweak_add; only touch _check.
    fn = src.index("int secp256k1_xonly_pubkey_tweak_add_check(")
    body_end = src.index("\n}\n", fn)
    body = src[fn:body_end]
    if joined in body:
        body = body.replace(joined, split)
    assert body.count(split) == 1, f"tweak-check anchor found {body.count(split)} times in {path}"
    load = "    if (!secp256k1_xonly_pubkey_load(ctx, &pk, internal_pubkey)) {\n        return 0;\n    }\n"
    body = body.replace(split, load + TWEAK + split[len(load):])
    src = src[:fn] + body + src[body_end:]
    open(path, "w").write(src)
    assert open(path).read().count("hazync_batch_defer") == 2  # declaration + call
    print(f"hook installed in {path}")


install_tweak(base + "/src/modules/extrakeys/main_impl.h")
