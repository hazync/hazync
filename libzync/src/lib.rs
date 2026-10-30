//! libzync: the formats and rules the Hazync host and the libzync guest share. See Cargo.toml for
//! why this crate exists and why the guest includes its modules by path.
#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod ss_hints;
pub mod wire;
pub mod net;
#[cfg(feature = "std")]
pub mod host;

include!("profile.rs");

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn chunk_hints_round_trip() {
        let liftx = vec![7u8; 128];
        let sig = vec![9u8; 68 * 3];
        let h = wire::ChunkHints { mtp: 5, block_time: 6, liftx: &liftx, sig_collect: false, sig: &sig };
        let w = h.encode();
        let d = wire::ChunkHints::decode(&w).unwrap();
        assert_eq!((d.mtp, d.block_time, d.liftx, d.sig_collect, d.sig), (5, 6, &liftx[..], false, &sig[..]));
        // collect mode carries no records
        let c = wire::ChunkHints { mtp: 1, block_time: 2, liftx: &[], sig_collect: true, sig: &[] }.encode();
        assert!(wire::ChunkHints::decode(&c).unwrap().sig_collect);
        // malformed: truncated, a partial record, collect with records
        assert!(wire::ChunkHints::decode(&w[..w.len() - 1]).is_err());
        let mut bad = w.clone();
        bad[5] = 67;
        assert!(wire::ChunkHints::decode(&bad).is_err());
        let mut bad = w.clone();
        bad[4] = 1;
        assert!(wire::ChunkHints::decode(&bad).is_err());
    }

    #[test]
    fn agg_hints_round_trip() {
        let ecmh = vec![3u8; 101]; // not a multiple of 4: padding must be dropped on decode
        let bits = vec![0b1011u32, 0x8000_0000, 0x1];
        let h = wire::AggHints { ecmh_collect: false, ecmh: &ecmh, ss: Some((65, &bits)) };
        let w = h.encode();
        let d = wire::AggHints::decode(&w).unwrap();
        assert_eq!(d.ecmh, &ecmh[..]);
        assert_eq!(d.ss, Some((65, &bits[..])));
        let none = wire::AggHints { ecmh_collect: true, ecmh: &[], ss: None }.encode();
        let d = wire::AggHints::decode(&none).unwrap();
        assert!(d.ecmh_collect && d.ss.is_none());
        // a set padding bit (index 65 of nbits 65) is refused
        let pad = vec![0u32, 0, 0b10];
        let w = wire::AggHints { ecmh_collect: false, ecmh: &[], ss: Some((65, &pad)) }.encode();
        assert!(wire::AggHints::decode(&w).is_err());
        // nonzero padding after a 101-byte ECMH blob is refused (one encoding per value)
        let mut w = wire::AggHints { ecmh_collect: false, ecmh: &ecmh, ss: None }.encode();
        w[5 + 25] |= 0x0100_0000;
        assert!(wire::AggHints::decode(&w).is_err());
    }

    #[test]
    fn hint_hash_two_ways_agree() {
        let words = vec![0x8000_0001u32, 0, 0x10];
        let idx = ss_hints::indices(&words);
        assert_eq!(idx, vec![0, 31, 68]);
        let p = [1u8; 32];
        assert_eq!(ss_hints::step(&p, 9, &idx), ss_hints::step_bits(&p, 9, &words));
        assert_eq!(ss_hints::bits_from_indices(69, &idx).unwrap(), words);
        assert!(ss_hints::bits_from_indices(68, &idx).is_none());
        assert!(ss_hints::bits_canonical(69, &words));
        assert!(!ss_hints::bits_canonical(68, &words));
        assert!(!ss_hints::bits_canonical(69, &words[..2]));
        assert!(ss_hints::bits_canonical(0, &[]));
    }

    #[test]
    fn profile_parses() {
        assert_eq!(libzync_profile(None), (false, false));
        assert_eq!(libzync_profile(Some("1")), (true, false));
        assert_eq!(libzync_profile(Some("assumevalid")), (true, true));
        assert!(!libzync_signet(Some("mainnet")) && libzync_signet(Some("signet")));
        // every lever is distinct
        let mut l: Vec<&str> = LIBZYNC_LEVERS.to_vec();
        l.sort();
        l.dedup();
        assert_eq!(l.len(), LIBZYNC_LEVERS.len());
    }

    #[cfg(feature = "std")]
    #[test]
    fn bundle_round_trip() {
        let b = host::BlockBundle {
            height: 7,
            block_hash: [2; 32],
            chunks: vec![(0, 10, vec![1; 68]), (10, 20, vec![])],
            ecmh: vec![5; 37],
        };
        let back = host::BlockBundle::from_bytes(&b.to_bytes()).unwrap();
        assert_eq!(back, b);
        assert_eq!(back.chunk_sig(10, 20), Some(&[][..]));
        assert_eq!(back.chunk_sig(0, 20), None);
        let mut t = b.to_bytes();
        t.push(0);
        assert!(host::BlockBundle::from_bytes(&t).is_err());
    }

    /// A real ECDSA signature: the hint the producer makes satisfies the guest's acceptance test.
    #[cfg(feature = "std")]
    #[test]
    fn sig_hint_is_r() {
        use secp256k1::{Message, SecretKey, SECP256K1};
        use num_bigint::BigUint;
        let sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let pk = sk.public_key(SECP256K1).serialize_uncompressed();
        let msg = [0x22u8; 32];
        let sig = SECP256K1.sign_ecdsa(&Message::from_digest(msg), &sk).serialize_compact();
        let n = BigUint::from_bytes_be(&net::hex_be("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141"));
        let (r, s, e) = (BigUint::from_bytes_be(&sig[..32]), BigUint::from_bytes_be(&sig[32..]), BigUint::from_bytes_be(&msg));
        let w = s.modpow(&(&n - 2u32), &n);
        let be = |v: BigUint| { let b = v.to_bytes_be(); let mut o = [0u8; 32]; o[32 - b.len()..].copy_from_slice(&b); o };
        let mut q = [0u8; 64];
        q.copy_from_slice(&pk[1..]);
        let rec = host::SigRecord { kind: 0, u1: be(&e * &w % &n), u2: be(&r * &w % &n), q, r: be(r.clone()) };
        let h = host::sig_hint(&rec);
        assert_eq!(&h[..4], &1u32.to_le_bytes());
        // a wrong r is marked invalid (the guest then asks libsecp)
        let bad = host::SigRecord { r: be(&r + 1u32), ..rec };
        assert_eq!(&host::sig_hint(&bad)[..4], &0u32.to_le_bytes());
    }

    /// The ECMH hint satisfies the curve equation the guest checks, and every skipped counter's
    /// witness proves a non-residue.
    #[cfg(feature = "std")]
    #[test]
    fn ecmh_hint_checks() {
        use num_bigint::BigUint;
        use sha2::{Digest, Sha256};
        let p = BigUint::parse_bytes(b"fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f", 16).unwrap();
        for seed in 0u8..40 {
            let leaf = [seed; 32];
            let h = host::ecmh_hint(&leaf);
            let ctr = u32::from_le_bytes(h[..4].try_into().unwrap());
            assert_eq!(h.len(), 36 + 32 * ctr as usize);
            let x_for = |c: u32| { let mut d = Sha256::new(); d.update(leaf); d.update(c.to_le_bytes()); BigUint::from_bytes_be(&d.finalize()) };
            let rhs = |x: &BigUint| (x * x * x + 7u32) % &p;
            let y = BigUint::from_bytes_be(&h[4..36]);
            assert_eq!(&y * &y % &p, rhs(&x_for(ctr)));
            assert!(!y.bit(0), "the guest accepts only the even root");
            for k in 0..ctr {
                let w = BigUint::from_bytes_be(&h[36 + 32 * k as usize..68 + 32 * k as usize]);
                assert_eq!(&w * &w % &p, (&p - rhs(&x_for(k))) % &p);
            }
        }
    }
}
