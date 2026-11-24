//! The slice of risc0-crypto Hazync's live-coin hash uses (swiftsync.rs): secp256k1 Fq with 8x32 little-endian limbs.
#![no_std]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BigInt<const N: usize = 8>(pub [u32; 8]);
pub mod curves { pub mod secp256k1 {
    use crate::BigInt;
    use openvm_algebra_guest::IntMod;
    type C = k256::Secp256k1Coord;
    #[derive(Clone, PartialEq, Eq)]
    pub struct Fq(C);
    fn le(l: &[u32; 8]) -> [u8; 32] { let mut b = [0u8; 32]; for k in 0..8 { b[4*k..4*k+4].copy_from_slice(&l[k].to_le_bytes()); } b }
    impl Fq {
        pub fn from_bigint(b: BigInt) -> Option<Fq> { C::from_le_bytes(&le(&b.0)).map(Fq) }
        pub fn from_u32(v: u32) -> Fq { Fq(C::from_u32(v)) }
        pub fn as_bigint(&self) -> BigInt {
            let b = self.0.as_le_bytes(); let mut l = [0u32; 8];
            for k in 0..8 { l[k] = u32::from_le_bytes(b[4*k..4*k+4].try_into().unwrap()); } BigInt(l)
        }
    }
    impl<'a> core::ops::Mul<&'a Fq> for &'a Fq { type Output = Fq; fn mul(self, o: &Fq) -> Fq { Fq(&self.0 * &o.0) } }
    impl<'a> core::ops::Add<&'a Fq> for &'a Fq { type Output = Fq; fn add(self, o: &Fq) -> Fq { Fq(&self.0 + &o.0) } }
    impl<'a> core::ops::Neg for &'a Fq { type Output = Fq; fn neg(self) -> Fq { Fq(-&self.0) } }
}}
