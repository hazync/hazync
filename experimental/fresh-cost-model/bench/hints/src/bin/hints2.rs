//! hints2 <collect log> <out>: BATCHREC lines (kind u1 u2 qx qy r, big-endian hex, u1/u2 already = e/s, r/s for ECDSA)
//! -> 68-byte hints in call order. R = u1*G + u2*Q natively; VALID = 1 only if the scheme's acceptance test passes
//! (CHECKMULTISIG's expected mismatches get VALID 0 and fall back to libsecp, exactly as in Hazync).
use k256::elliptic_curve::{ops::Reduce, point::AffineCoordinates, sec1::{FromEncodedPoint, ToEncodedPoint}};
use k256::{AffinePoint, EncodedPoint, ProjectivePoint, Scalar, U256};
fn sc(h: &str) -> Scalar { let b = hex(h); <Scalar as Reduce<U256>>::reduce_bytes((&b[..]).into()) }
fn hex(h: &str) -> Vec<u8> { (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i+2], 16).unwrap()).collect() }
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let log = std::fs::read_to_string(&a[1]).unwrap();
    let (mut out, mut ok, mut no) = (Vec::new(), 0, 0);
    for l in log.lines().filter_map(|l| l.find("BATCHREC ").map(|i| &l[i + 9..])) {
        let f: Vec<&str> = l.split_whitespace().collect();
        let kind: u32 = f[0].parse().unwrap();
        let mut enc = vec![4u8]; enc.extend(hex(f[3])); enc.extend(hex(f[4]));
        let q = AffinePoint::from_encoded_point(&EncodedPoint::from_bytes(&enc).unwrap()).unwrap();
        let r = (ProjectivePoint::GENERATOR * sc(f[1]) + ProjectivePoint::from(q) * sc(f[2])).to_affine();
        let e = r.to_encoded_point(false);
        let valid = match (kind, e.x()) {
            (0, Some(x)) => sc(&x.iter().map(|b| format!("{b:02x}")).collect::<String>()) == sc(f[5]),
            (1, Some(x)) => x.as_slice() == &hex(f[5])[..] && !bool::from(r.y_is_odd()),
            (3 | 4, Some(x)) => x.as_slice() == &hex(f[5])[..] && bool::from(r.y_is_odd()) == (kind == 4),
            _ => false,
        };
        out.extend_from_slice(&(valid as u32).to_le_bytes());
        if valid { ok += 1; out.extend_from_slice(e.x().unwrap()); out.extend_from_slice(e.y().unwrap()); }
        else { no += 1; out.extend_from_slice(&[0u8; 64]); }
    }
    std::fs::write(&a[2], &out).unwrap();
    println!("hints2 valid={ok} invalid={no}");
}
