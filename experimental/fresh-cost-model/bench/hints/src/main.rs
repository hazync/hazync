//! Native hint generator for ovm-secp's batch: per deferred check, R (Hazync's HINT_BYTES = 68 layout:
//! u32 LE valid flag, R.x BE, R.y BE), in the guest's call order: every ECDSA then every BIP340 (mode 5),
//! or only one list (mode 1 / 2). Usage: hints <blob> <mode> <out>
use k256::elliptic_curve::{ops::Reduce, point::AffineCoordinates, sec1::{FromEncodedPoint, ToEncodedPoint}, PrimeField};
use k256::{AffinePoint, EncodedPoint, ProjectivePoint, Scalar, U256};
use sha2::{Digest, Sha256};
fn rd(b: &[u8], o: &mut usize) -> usize { let v = u32::from_le_bytes(b[*o..*o+4].try_into().unwrap()) as usize; *o += 4; v }
fn sc(b: &[u8]) -> Scalar { <Scalar as Reduce<U256>>::reduce_bytes(b.into()) }
fn push(out: &mut Vec<u8>, p: ProjectivePoint) {
    let e = p.to_affine().to_encoded_point(false);
    out.extend_from_slice(&1u32.to_le_bytes()); out.extend_from_slice(e.x().unwrap()); out.extend_from_slice(e.y().unwrap());
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let b = std::fs::read(&a[1]).unwrap(); let mode: u8 = a[2].parse().unwrap();
    let mut o = 0; let n_ec = rd(&b, &mut o); let ec_at = o; o += n_ec * 129; let n_sc = rd(&b, &mut o); let sc_at = o;
    let mut out = Vec::new();
    if mode == 1 || mode == 5 { for i in 0..n_ec {
        let r = &b[ec_at + i*129..ec_at + (i+1)*129];
        let q = AffinePoint::from_encoded_point(&EncodedPoint::from_bytes(&r[32..65]).unwrap()).unwrap();
        let (rs, ss) = (sc(&r[65..97]), sc(&r[97..129]));
        let w = ss.invert().unwrap();
        let rp = ProjectivePoint::GENERATOR * (sc(&r[..32]) * w) + ProjectivePoint::from(q) * (rs * w);
        assert_eq!(sc(&rp.to_affine().x()), rs, "ecdsa {i} does not verify");
        push(&mut out, rp);
    }}
    if mode == 2 || mode == 5 { for i in 0..n_sc {
        let r = &b[sc_at + i*128..sc_at + (i+1)*128];
        let mut enc = [0u8; 33]; enc[0] = 2; enc[1..].copy_from_slice(&r[32..64]);
        let p = AffinePoint::from_encoded_point(&EncodedPoint::from_bytes(&enc).unwrap()).unwrap();
        let tag = Sha256::digest(b"BIP0340/challenge");
        let e = sc(&Sha256::new().chain_update(tag).chain_update(tag).chain_update(&r[64..96]).chain_update(&r[32..64]).chain_update(&r[..32]).finalize());
        let s = Scalar::from_repr(*k256::FieldBytes::from_slice(&r[96..128])).unwrap();
        let rp = ProjectivePoint::GENERATOR * s - ProjectivePoint::from(p) * e;
        let ra = rp.to_affine(); assert_eq!(&ra.x()[..], &r[64..96], "schnorr {i}"); assert!(!bool::from(ra.y_is_odd()));
        push(&mut out, rp);
    }}
    std::fs::write(&a[3], &out).unwrap();
    println!("hints mode={mode} n={}", out.len() / 68);
}
