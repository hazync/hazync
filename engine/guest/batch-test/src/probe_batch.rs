#[cfg(test)]
mod r1_3 {
    use super::*;
    use openvm_algebra_guest::IntMod as _;
    use rk256::elliptic_curve::{sec1::ToEncodedPoint, Field, PrimeField};
    use rk256::{AffinePoint, ProjectivePoint, Scalar};

    const OLD: bool = cfg!(feature = "old");
    fn tag() -> &'static str { if OLD { "OLD (0382bb3f^)" } else { "NEW (0382bb3f)" } }

    fn le_limbs(be: &[u8]) -> [u32; 8] {
        let mut le = [0u8; 32];
        for k in 0..32 { le[k] = be[31 - k]; }
        core::array::from_fn(|i| u32::from_le_bytes(le[4 * i..4 * i + 4].try_into().unwrap()))
    }
    fn sc(s: &Scalar) -> [u32; 8] { le_limbs(&s.to_repr()) }
    fn xy(p: &ProjectivePoint) -> ([u8; 32], [u8; 32]) {
        let e = p.to_affine().to_encoded_point(false);
        (e.x().unwrap().as_slice().try_into().unwrap(), e.y().unwrap().as_slice().try_into().unwrap())
    }
    fn hint(valid: u32, x: &[u8; 32], y: &[u8; 32]) -> Vec<u8> {
        let mut h = valid.to_le_bytes().to_vec(); h.extend_from_slice(x); h.extend_from_slice(y); h
    }
    /// One call of the libsecp hook: u1·G + u2·Q == R, R's x given as r (LE limbs, as the hook passes them).
    fn defer(kind: i32, u1: &Scalar, u2: &Scalar, q: &ProjectivePoint, r_be: &[u8; 32]) -> i32 {
        let (qx, qy) = xy(q);
        let (a, b, x, y, r) = (sc(u1), sc(u2), le_limbs(&qx), le_limbs(&qy), le_limbs(r_be));
        unsafe { hazync_batch_defer(kind, a.as_ptr(), b.as_ptr(), x.as_ptr(), y.as_ptr(), r.as_ptr()) }
    }
    fn fallback() -> u32 { unsafe { FALLBACK } }
    fn deferred_raw(i: usize) -> Vec<u8> { unsafe { let d: &Vec<Deferred> = &*core::ptr::addr_of!(DEFERRED); d[i].raw.to_vec() } }
    fn even_y_point() -> (Scalar, ProjectivePoint) {
        loop {
            let k = Scalar::random(&mut rand::thread_rng());
            let p = ProjectivePoint::GENERATOR * k;
            if xy(&p).1[31] & 1 == 0 { return (k, p); }
        }
    }

    /// (a) BIP340 (kind 1) with r = 0 and s = e·d: libsecp computes s·G − e·P = infinity and rejects. The prover's hint
    /// {valid=1, x=0, y=0}.
    #[test]
    fn a_bip340_r_zero_hint_identity() {
        let (d, p) = even_y_point();
        let e = Scalar::random(&mut rand::thread_rng());
        let s = e * d;
        // control first: a VALID BIP340 item is deferred and the batch accepts
        let (k, r) = even_y_point();
        let sv = k + e * d;
        let (rx, ry) = xy(&r);
        install(false, hint(1, &rx, &ry));
        let rc = defer(1, &sv, &(-e), &p, &rx);
        let ok = finish();
        eprintln!("[{}] (a) control, a valid BIP340 item: defer -> {rc}, fallback {}, finish -> {ok}", tag(), fallback());
        assert!(rc == 1 && ok);
        // the forgery: u1·G + u2·Q = s·G − e·P = O, r = 0, hint R = (0,0)
        let zero = [0u8; 32];
        install(false, hint(1, &zero, &zero));
        let rc = defer(1, &s, &(-e), &p, &zero);
        let ok = finish();
        eprintln!("[{}] (a) forged BIP340 r=0, hint (0,0): defer -> {rc} (1 = deferred, 0 = left to libsecp), fallback {}, finish -> {ok}", tag(), fallback());
        if OLD { assert!(rc == 1 && ok, "OLD: the forged item is deferred and the batch ACCEPTS it"); }
        else { assert!(rc == 0 && fallback() == 1, "NEW: the item goes to libsecp's own check"); }
        // and with no usable hint the guest's own direct check runs: infinity is invalid
        install(false, hint(0, &zero, &zero));
        let rc = defer(1, &s, &(-e), &p, &zero);
        eprintln!("[{}] (a) the same item with no hint: defer -> {rc} (2 = invalid)", tag());
        assert_eq!(rc, 2);
    }

    /// (b) ECDSA (kind 0): R and −R both pass accept_r (x(R) mod n == r). Does the transcript depend on the choice?
    #[test]
    fn b_ecdsa_plus_minus_r() {
        let mut rng = rand::thread_rng();
        let items: Vec<(Scalar, Scalar, ProjectivePoint, ProjectivePoint)> = (0..6).map(|_| {
            let (u1, u2) = (Scalar::random(&mut rng), Scalar::random(&mut rng));
            let q = ProjectivePoint::GENERATOR * Scalar::random(&mut rng);
            (u1, u2, q, ProjectivePoint::GENERATOR * u1 + q * u2)
        }).collect();
        let run = |flip: Option<usize>| -> (Vec<Vec<u8>>, bool) {
            let mut h = Vec::new();
            for (i, it) in items.iter().enumerate() {
                let r = if flip == Some(i) { -it.3 } else { it.3 };
                let (x, y) = xy(&r);
                h.extend(hint(1, &x, &y));
            }
            install(false, h);
            for it in &items {
                let (x, _) = xy(&it.3);
                // r = x(R) mod n: these random x are below n with overwhelming probability; the hook passes r reduced
                assert_eq!(defer(0, &it.0, &it.1, &it.2, &x), 1);
            }
            let raws = (0..items.len()).map(deferred_raw).collect();
            (raws, finish())
        };
        let (honest, ok) = run(None);
        let (flipped, ok_f) = run(Some(2));
        let same = honest[2] == flipped[2];
        eprintln!("[{}] (b) all valid: finish -> {ok}; item 2's R negated: transcript bytes of item 2 {} ({} bytes each), finish -> {ok_f}",
                  tag(), if same { "IDENTICAL (the weights do not depend on the sign)" } else { "DIFFER (R.y is bound)" }, honest[2].len());
        assert!(ok);
        assert!(!ok_f, "one flipped R alone always fails: the attack needs a subset-sum over many flips, which needs fixed weights");
        if OLD { assert!(same, "OLD: the transcript is blind to R.y") } else { assert!(!same, "NEW: R.y is in the transcript") }
    }

    /// (c) the MSM: the window it picks (c = 16 stored a digit of +32768 as i16 -32768), and its sums vs a naive sum.
    #[test]
    fn c_msm_window_and_sums() {
        // the pick at sizes up to 2^21 points of 128-bit (the batch weights) and 256-bit scalars
        let mut max_pick = 0;
        for lg in [4usize, 8, 12, 16, 18, 19, 20, 21] {
            for b in [128usize, 256] {
                let c = msm::probe::pick(&vec![b; 1 << lg]);
                max_pick = max_pick.max(c);
                if lg >= 18 { eprintln!("[{}] (c) {} points of {b} bits: window c = {c}", tag(), 1usize << lg); }
            }
        }
        eprintln!("[{}] (c) largest window picked: {max_pick} (top of the range: {})", tag(), env!("HZ_MSM_MAX_C"));
        // a scalar whose low 16-bit window is 0x8000 has the digit +32768 at c = 16
        let mut s = [0u8; 32]; s[1] = 0x80;
        let d16 = msm::probe::stored_digits(&s, 16);
        eprintln!("[{}] (c) scalar 0x8000 at c = 16: digit {} stored as {}", tag(), d16[0].0, d16[0].1);
        assert_eq!(d16[0].0 as i32 != d16[0].1 as i32, true, "c = 16 wraps (the reason the range must stop at 15)");
        if !OLD { assert!(max_pick <= 15); }
        // sums vs naive, incl. the identity base, zero scalars, equal and opposite points; 3..=40 points (Straus and
        // Pippenger)
        let mut rng = rand::thread_rng();
        for n in [3usize, 4, 5, 9, 17, 40] {
            let mut pts: Vec<ProjectivePoint> = (0..n).map(|_| ProjectivePoint::GENERATOR * Scalar::random(&mut rng)).collect();
            let mut scs: Vec<Scalar> = (0..n).map(|_| Scalar::random(&mut rng)).collect();
            pts[0] = ProjectivePoint::IDENTITY; scs[1] = Scalar::ZERO;
            if n > 3 { pts[3] = pts[2]; pts[n - 1] = -pts[2]; }
            let naive = pts.iter().zip(&scs).fold(ProjectivePoint::IDENTITY, |a, (p, s)| a + *p * s);
            let to_p = |p: &ProjectivePoint| -> P {
                if *p == ProjectivePoint::IDENTITY { return <P as openvm_ecc_guest::weierstrass::WeierstrassPoint>::IDENTITY; }
                let (x, y) = xy(p);
                let (mut xl, mut yl) = (x, y); xl.reverse(); yl.reverse();
                unsafe { P::from_xy_unchecked(Fq::from_le_bytes(&xl).unwrap(), Fq::from_le_bytes(&yl).unwrap()) }
            };
            let to_s = |s: &Scalar| -> Fr { let mut b: [u8; 32] = s.to_repr().into(); b.reverse(); Fr::from_le_bytes(&b).unwrap() };
            let got = msm::probe::run(&scs.iter().map(to_s).collect::<Vec<_>>(), &pts.iter().map(to_p).collect::<Vec<_>>());
            let want = to_p(&naive);
            assert!(got == want, "msm != naive at n = {n}");
        }
        eprintln!("[{}] (c) msm == naive sum at n = 3, 4, 5, 9, 17, 40 (identity base, zero scalar, equal and opposite points)", tag());
    }
}
