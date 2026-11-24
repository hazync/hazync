#![no_std]
extern crate alloc;
#[path = "../../msm_ovm.rs"]
pub mod msm;

#[cfg(test)]
mod tests {
    extern crate std;
    use alloc::vec::Vec;
    use k256::{Secp256k1Point as P, Secp256k1Scalar as Fr};
    use openvm_algebra_guest::IntMod;
    use openvm_ecc_guest::{weierstrass::{IntrinsicCurve, WeierstrassPoint}, CyclicGroup};

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 }
        fn bytes(&mut self, n: usize) -> [u8; 32] {
            let mut b = [0u8; 32];
            for k in 0..n { b[k] = self.next() as u8; }
            b
        }
    }
    fn fr(b: &[u8; 32]) -> Fr { let mut x = Fr::from_le_bytes_unchecked(b); x.assert_reduced(); x }
    fn fr_reduced(b: &[u8; 32]) -> Fr { use openvm_algebra_guest::Reduce; Fr::reduce_le_bytes(b) }
    fn reference(s: &[Fr], p: &[P]) -> P { <k256::Secp256k1 as IntrinsicCurve>::msm(s, p) }
    fn mul_g(k: &Fr) -> P { reference(core::slice::from_ref(k), &[<P as CyclicGroup>::GENERATOR]) }

    fn check(s: &[Fr], p: &[P], what: &str) {
        let a = crate::msm::msm(s, p);
        let b = reference(s, p);
        assert!(a == b, "{what}: n={} differs", s.len());
    }

    #[test]
    fn random_and_edges() {
        let mut r = Rng(0x9e3779b97f4a7c15);
        let mut pool: Vec<P> = Vec::new();
        for _ in 0..64 { pool.push(mul_g(&fr_reduced(&r.bytes(32)))); }
        let n_minus_1 = { let mut x = Fr::ZERO; x = &x - &Fr::ONE; x };
        let special = [Fr::ZERO, Fr::ONE, n_minus_1.clone(), fr(&r.bytes(16))];
        // small n with every combination of special scalars and repeated / opposite / identity bases
        for n in 0..=6usize {
            for t in 0..200 {
                let mut s = Vec::new(); let mut p = Vec::new();
                for i in 0..n {
                    let v = r.next();
                    s.push(match v % 5 { 0..=3 => special[(v / 5 % 4) as usize].clone(), _ => fr_reduced(&r.bytes((v as usize / 7) % 33)) });
                    let q = &pool[(r.next() % 4) as usize]; // few distinct points: many equal pairs
                    p.push(match r.next() % 4 { 0 => -q, 1 => <P as WeierstrassPoint>::IDENTITY, _ => q.clone() });
                    if i > 0 && t % 3 == 0 { p[i] = -&p[i - 1]; s[i] = s[i - 1].clone(); } // exact cancellation
                }
                check(&s, &p, "small");
            }
        }
        // the batch check's shape: 128-bit z on R points, full scalars on keys, one full scalar on G; and a sum
        // that is exactly zero (k·P + (n-k)·P)
        for &n in &[3usize, 31, 32, 100, 700, 3000] {
            let mut s = Vec::new(); let mut p = Vec::new();
            for i in 0..n {
                s.push(if i % 3 == 0 { fr_reduced(&r.bytes(32)) } else { fr(&r.bytes(16)) });
                p.push(pool[(r.next() % 64) as usize].clone());
            }
            check(&s, &p, "batch");
            let k = fr_reduced(&r.bytes(32)); let nk = &Fr::ZERO - &k;
            let mut s2 = s.clone(); let mut p2 = p.clone();
            s2.push(k); p2.push(pool[0].clone()); s2.push(nk); p2.push(pool[0].clone());
            check(&s2, &p2, "batch+cancel");
            let z = crate::msm::msm(&[s2[n].clone(), s2[n + 1].clone()], &[pool[0].clone(), pool[0].clone()]);
            assert!(z == <P as WeierstrassPoint>::IDENTITY, "k·P + (n-k)·P != O");
        }
    }
}
