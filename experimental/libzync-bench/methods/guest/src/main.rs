#![no_main]
//! libzync-bench guest: measures, in cycles, the building blocks of a proving-native
//! signature/set-hash library on the RISC Zero bigint2 coprocessor.
//!
//! Every section is bracketed by `env::cycle_count()` and the deltas are committed to the
//! journal, so the host reports what the GUEST paid for that section alone — set-up (point
//! generation, input deserialisation) is outside the brackets and reported separately.
//!
//! ⛔ EXPERIMENT ONLY. Nothing here is consensus code; batch verification in particular is a
//! design under test, not a reviewed protocol (see docs/history/MSM_BATCH_VERIFY.md).

use risc0_crypto::curves::secp256k1::{Affine, Fq, Fr};
use risc0_crypto::ecdsa::Signature;
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256, Sha512_256};

mod msm;

risc0_zkvm::guest::entry!(main);

/// One signature as the host hands it over. All 32-byte values are big-endian.
#[derive(serde::Deserialize)]
struct Sig {
    qx: [u8; 32],
    qy: [u8; 32],
    e: [u8; 32],
    r: [u8; 32],
    s: [u8; 32],
    /// The hinted nonce point R = u1*G + u2*Q. Only the batch path reads it.
    rx: [u8; 32],
    ry: [u8; 32],
}

fn fq(b: &[u8; 32]) -> Fq {
    Fq::from_be_bytes_mod_order(b)
}
fn fr(b: &[u8; 32]) -> Fr {
    Fr::from_be_bytes_mod_order(b)
}

/// Results travel as (label, value) pairs so the host never has to know the section order.
struct Out(Vec<(String, u64)>);
impl Out {
    fn put(&mut self, k: &str, v: u64) {
        self.0.push((k.to_string(), v));
    }
}

fn main() {
    let mode: u32 = env::read();
    let mut out = Out(Vec::new());
    match mode {
        1 => ops(&mut out),
        2 => msm_mode(&mut out),
        3 => batch_mode(&mut out),
        4 => sethash_mode(&mut out),
        5 => utreexo_mode(&mut out),
        _ => panic!("unknown mode {mode}"),
    }
    env::commit(&out.0);
}

/// Mode 1: unit costs. n repetitions of each primitive, chained so nothing is dead code.
fn ops(out: &mut Out) {
    let n: u32 = env::read();
    let sigs: Vec<Sig> = env::read();

    let mut a = fq(&[3u8; 32]);
    let b = fq(&[5u8; 32]);
    let t = env::cycle_count();
    for _ in 0..n {
        a = &a * &b;
    }
    out.put("fq_mul", (env::cycle_count() - t) / n as u64);

    let t = env::cycle_count();
    for _ in 0..n {
        a = &a + &b;
    }
    out.put("fq_add", (env::cycle_count() - t) / n as u64);

    let t = env::cycle_count();
    for _ in 0..n {
        a = a.inverse();
    }
    out.put("fq_inv", (env::cycle_count() - t) / n as u64);

    let mut c = fr(&[7u8; 32]);
    let d = fr(&[9u8; 32]);
    let t = env::cycle_count();
    for _ in 0..n {
        c = &c * &d;
    }
    out.put("fr_mul", (env::cycle_count() - t) / n as u64);
    let t = env::cycle_count();
    for _ in 0..n {
        c = c.inverse();
    }
    out.put("fr_inv", (env::cycle_count() - t) / n as u64);

    // Point ops on distinct points (a walk), so add never degenerates into a double.
    let mut p = Affine::GENERATOR;
    let g = Affine::GENERATOR;
    let t = env::cycle_count();
    for _ in 0..n {
        let q = p;
        p.add_into(&q, &g);
    }
    out.put("ec_add", (env::cycle_count() - t) / n as u64);
    let t = env::cycle_count();
    for _ in 0..n {
        p = p.double();
    }
    out.put("ec_double", (env::cycle_count() - t) / n as u64);

    // SHA-256 over a 32-byte and a 64-byte message (1 and 2 compressions after padding).
    let mut h = [0u8; 32];
    let t = env::cycle_count();
    for _ in 0..n {
        h = Sha256::digest(h).into();
    }
    out.put("sha256_32B", (env::cycle_count() - t) / n as u64);
    let t = env::cycle_count();
    for _ in 0..n {
        let mut m = [0u8; 64];
        m[..32].copy_from_slice(&h);
        h = Sha256::digest(m).into();
    }
    out.put("sha256_64B", (env::cycle_count() - t) / n as u64);

    // y from x: software sqrt (pow) vs a hinted y checked with y^2 == x^3 + 7.
    let x = fq(&sigs[0].qx);
    let y_hint = fq(&sigs[0].qy);
    let seven = Fq::from_u32(7);
    let m = (n / 10).max(1);
    let t = env::cycle_count();
    let mut ok = true;
    for _ in 0..m {
        let rhs = &(&(&x * &x) * &x) + &seven;
        ok &= rhs.sqrt().is_some();
    }
    out.put("sqrt_pow", (env::cycle_count() - t) / m as u64);
    let t = env::cycle_count();
    for _ in 0..n {
        let rhs = &(&(&x * &x) * &x) + &seven;
        ok &= &y_hint * &y_hint == rhs;
    }
    out.put("sqrt_hint_check", (env::cycle_count() - t) / n as u64);

    // Single ECDSA verify with risc0-crypto (Shamir double-scalar-mul, affine coprocessor ops).
    let t = env::cycle_count();
    let mut good = 0u64;
    for s in &sigs {
        let q = Affine::new_in_subgroup(fq(&s.qx), fq(&s.qy)).expect("pubkey");
        let sig = Signature::new(fr(&s.r), fr(&s.s)).expect("sig");
        good += sig.verify(&q, &s.e) as u64;
    }
    out.put("ecdsa_verify_single", (env::cycle_count() - t) / sigs.len() as u64);
    out.put("ecdsa_single_valid", good);
    out.put("guard", ok as u64 + a.to_bigint().as_le_bytes()[0] as u64 + c.to_bigint().as_le_bytes()[0] as u64
        + h[0] as u64 + p.is_identity() as u64);
}

/// Mode 2: Pippenger alone at size n, with op counts, for a list of windows.
fn msm_mode(out: &mut Out) {
    let n: u32 = env::read();
    let windows: Vec<u32> = env::read();
    let variant: u32 = env::read();
    let mut pts = Vec::with_capacity(n as usize);
    let mut scs = Vec::with_capacity(n as usize);
    let mut p = Affine::GENERATOR;
    let mut seed = [0u8; 32];
    for i in 0..n {
        seed = Sha256::digest([&seed[..], &i.to_le_bytes()].concat()).into();
        scs.push(fr(&seed));
        let q = p;
        p.add_into(&q, &Affine::GENERATOR);
        pts.push(p);
    }
    let raw_pts: Vec<msm::Pt> = if variant == 2 { pts.iter().map(msm::to_pt).collect() } else { Vec::new() };
    let mut first: Option<Affine> = None;
    for w in windows {
        msm::reset_counts();
        let t = env::cycle_count();
        let r = match variant {
            0 => msm::msm(&pts, &scs, w as usize),
            1 => msm::msm_signed(&pts, &scs, w as usize),
            _ => msm::acc_to_affine(&msm::msm_raw(&raw_pts, &scs, w as usize)),
        };
        let cyc = env::cycle_count() - t;
        let (adds, dbls) = msm::counts();
        out.put(&format!("w{w}_cycles"), cyc);
        out.put(&format!("w{w}_adds"), adds);
        out.put(&format!("w{w}_doubles"), dbls);
        // Every window must give the same point, or the comparison is meaningless.
        match &first {
            None => first = Some(r),
            Some(f) => assert!(f.xy() == r.xy(), "msm windows disagree"),
        }
    }
    // The host recomputes Σ s_i·(i+2)·G independently (k256) and compares this x coordinate.
    let (x, _) = first.expect("no window").xy().expect("identity");
    let mut xb = [0u8; 32];
    x.to_bigint().write_be_bytes(&mut xb);
    for k in 0..4 {
        out.put(&format!("res_x{k}"), u64::from_be_bytes(xb[8 * k..8 * k + 8].try_into().unwrap()));
    }
}

/// Mode 3: verify n real signatures two ways — one at a time, and as ONE batch with hinted R.
///
/// Batch check (docs/history/MSM_BATCH_VERIFY.md §3):
///   per sig: Q, R on curve; r,s != 0; x(R) mod n == r; u1 = e/s, u2 = r/s;
///   z_i = 128-bit Fiat-Shamir weight from a hash over EVERY input including R;
///   Σ z_i·u2_i·Q_i − Σ z_i·R_i + (Σ z_i·u1_i)·G == O, as one MSM.
fn batch_mode(out: &mut Out) {
    let sigs: Vec<Sig> = env::read();
    let window: u32 = env::read();
    let also_single: bool = env::read();
    let n = sigs.len() as u64;

    if also_single {
        let t = env::cycle_count();
        let mut all = true;
        for s in &sigs {
            let q = Affine::new_in_subgroup(fq(&s.qx), fq(&s.qy)).expect("pubkey");
            all &= Signature::new(fr(&s.r), fr(&s.s)).map_or(false, |sig| sig.verify(&q, &s.e));
        }
        out.put("single_total", env::cycle_count() - t);
        out.put("single_all_valid", all as u64);
    }

    let t0 = env::cycle_count();
    // 1. Transcript: one hash over every byte of every input, R included.
    let mut tr = Sha256::new();
    tr.update(b"libzync/batch-ecdsa/v0");
    for s in &sigs {
        tr.update(s.qx);
        tr.update(s.qy);
        tr.update(s.e);
        tr.update(s.r);
        tr.update(s.s);
        tr.update(s.rx);
        tr.update(s.ry);
    }
    let seed: [u8; 32] = tr.finalize().into();
    let t1 = env::cycle_count();

    // 2. Per-signature checks and scalars.
    let mut pts = Vec::with_capacity(2 * sigs.len() + 1);
    let mut scs = Vec::with_capacity(2 * sigs.len() + 1);
    let mut g_coeff = Fr::ZERO;
    let mut ok = true;
    for (i, s) in sigs.iter().enumerate() {
        let q = Affine::new_in_subgroup(fq(&s.qx), fq(&s.qy));
        let rpt = Affine::new_in_subgroup(fq(&s.rx), fq(&s.ry));
        let (r, sc) = (fr(&s.r), fr(&s.s));
        let (Some(q), Some(rpt)) = (q, rpt) else {
            ok = false;
            continue;
        };
        if r.is_zero() || sc.is_zero() {
            ok = false;
            continue;
        }
        // The actual ECDSA acceptance test, on the hinted point: x(R) mod n == r.
        ok &= Fr::from_be_bytes_mod_order(&s.rx) == r;
        let w = sc.inverse();
        let u1 = &fr(&s.e) * &w;
        let u2 = &r * &w;
        // z_i: 128 bits of H(seed || i). Top 16 bytes zero => a 128-bit scalar.
        let zh: [u8; 32] = Sha256::digest([&seed[..], &(i as u32).to_le_bytes()].concat()).into();
        let mut zb = [0u8; 32];
        zb[16..].copy_from_slice(&zh[..16]);
        let z = fr(&zb);
        g_coeff = &g_coeff + &(&z * &u1);
        pts.push(q);
        scs.push(&z * &u2);
        pts.push(-&rpt);
        scs.push(z);
    }
    pts.push(Affine::GENERATOR);
    scs.push(g_coeff);
    let t2 = env::cycle_count();

    // 3. One MSM; the batch holds iff it is the identity.
    msm::reset_counts();
    let raw: Vec<msm::Pt> = pts.iter().map(msm::to_pt).collect();
    let acc = msm::msm_raw(&raw, &scs, window as usize);
    let t3 = env::cycle_count();
    let (adds, dbls) = msm::counts();
    let batch_ok = ok && acc.get().is_none();

    out.put("n", n);
    out.put("batch_transcript", t1 - t0);
    out.put("batch_per_sig_prep", t2 - t1);
    out.put("batch_msm", t3 - t2);
    out.put("batch_total", t3 - t0);
    out.put("batch_msm_adds", adds);
    out.put("batch_msm_doubles", dbls);
    out.put("batch_valid", batch_ok as u64);
}

/// Mode 4: multiset-hash cost per element, for SwiftSync-style add/subtract aggregates.
fn sethash_mode(out: &mut Out) {
    let n: u32 = env::read();
    // ECMH hints: for element i, (ctr, y) with x = H(elem||ctr) on the curve, plus for every
    // smaller ctr a witness w with w^2 == -(x^3+7) proving that x was NOT on the curve
    // (p ≡ 3 mod 4, so -1 is a non-residue). That pins the map: one point per element.
    let ecmh_hints: Vec<(u32, [u8; 32], Vec<[u8; 32]>)> = env::read();
    let elems: Vec<[u8; 32]> = (0..n).map(|i| Sha256::digest(i.to_le_bytes()).into()).collect();

    // (a) SwiftSync original: salted SHA-256, summed mod 2^256 (needs a SECRET salt to be safe).
    let salt = [0x5au8; 32];
    let t = env::cycle_count();
    let mut acc = [0u32; 8];
    for e in &elems {
        let h: [u8; 32] = Sha256::digest([&salt[..], &e[..]].concat()).into();
        let mut carry = 0u64;
        for k in 0..8 {
            let v = u32::from_le_bytes(h[4 * k..4 * k + 4].try_into().unwrap()) as u64;
            let s = acc[k] as u64 + v + carry;
            acc[k] = s as u32;
            carry = s >> 32;
        }
    }
    out.put("sum256_per_elem", (env::cycle_count() - t) / n as u64);

    // (b) ECMH on secp256k1 with hinted y and pinned counter.
    let seven = Fq::from_u32(7);
    let t = env::cycle_count();
    let mut pacc = Affine::IDENTITY;
    let mut tries = 0u64;
    for (e, (ctr, y, nonres)) in elems.iter().zip(ecmh_hints.iter()) {
        assert_eq!(nonres.len() as u32, *ctr, "need one non-residue witness per skipped ctr");
        for (c, w) in nonres.iter().enumerate() {
            let xb: [u8; 32] = Sha256::digest([&e[..], &(c as u32).to_le_bytes()].concat()).into();
            let x = fq(&xb);
            let rhs = &(&(&x * &x) * &x) + &seven;
            let w = fq(w);
            assert!(&w * &w == -&rhs, "bad non-residue witness");
            tries += 1;
        }
        let xb: [u8; 32] = Sha256::digest([&e[..], &ctr.to_le_bytes()].concat()).into();
        let pt = Affine::new_in_subgroup(fq(&xb), fq(y)).expect("ecmh point off curve");
        tries += 1;
        let a = pacc;
        pacc.add_into(&a, &pt);
    }
    out.put("ecmh_per_elem", (env::cycle_count() - t) / n as u64);
    out.put("ecmh_tries_x1000_per_elem", tries * 1000 / n as u64);

    // (c) MuHash-style: SHA-expand to 3072 bits, multiply mod 2^3072 - 1103717 on the 4096-bit
    // coprocessor. (Core's MuHash3072 expands with ChaCha20; SHA keeps it on the accelerator.)
    use risc0_bigint2::field::{modmul_4096, FIELD_4096_WIDTH_WORDS as W};
    let mut modulus = [0u32; W];
    for k in 0..96 {
        modulus[k] = u32::MAX;
    }
    // 2^3072 - 1103717 = 0xFFFF...FFFF - 1103716 in the low word.
    modulus[0] = u32::MAX - 1103716;
    let t = env::cycle_count();
    let mut macc = [0u32; W];
    macc[0] = 1;
    for e in &elems {
        let mut num = [0u32; W];
        for blk in 0..12u32 {
            let h: [u8; 32] = Sha256::digest([&e[..], &blk.to_le_bytes()].concat()).into();
            for k in 0..8 {
                num[(blk * 8) as usize + k] = u32::from_le_bytes(h[4 * k..4 * k + 4].try_into().unwrap());
            }
        }
        // Values >= modulus occur with probability ~2^-3052; an honest benchmark can ignore them.
        let mut r = [0u32; W];
        modmul_4096(&macc, &num, &modulus, &mut r);
        macc = r;
    }
    out.put("muhash3072_per_elem", (env::cycle_count() - t) / n as u64);

    // (d) Reference: SHA-256 alone per element (the floor any of these pays).
    let t = env::cycle_count();
    let mut h = [0u8; 32];
    for e in &elems {
        h = Sha256::digest([&h[..], &e[..]].concat()).into();
    }
    out.put("sha_only_per_elem", (env::cycle_count() - t) / n as u64);
    out.put("guard", acc[0] as u64 + pacc.is_identity() as u64 + macc[0] as u64 + h[0] as u64);
}

/// Mode 5: Utreexo hashing, ours vs the standard (rustreexo / utreexod) format.
/// Ours: parent = SHA-256(0x01 || l || r), the accelerated SHA-256. Standard: parent =
/// SHA-512/256(l || r) and leaf = SHA-512/256(tag || tag || leaf data), software SHA-512 (risc0 has no
/// SHA-512 accelerator). Each section chains its output into the next input so nothing is dead, and
/// the first 8 bytes of every chain are committed for the host to recompute natively.
fn utreexo_mode(out: &mut Out) {
    let n: u32 = env::read();
    let leaf_lens: Vec<u32> = env::read();
    let first8 = |h: &[u8]| u64::from_le_bytes(h[..8].try_into().unwrap());

    let mut h = [0u8; 32];
    let t = env::cycle_count();
    for _ in 0..n {
        let mut s = Sha256::new();
        s.update([0x01u8]);
        s.update(h);
        s.update(h);
        h = s.finalize().into();
    }
    out.put("ours_parent", (env::cycle_count() - t) / n as u64);
    out.put("ours_parent_chk", first8(&h));

    let mut h = [0u8; 32];
    let t = env::cycle_count();
    for _ in 0..n {
        let mut m = [0u8; 64];
        m[..32].copy_from_slice(&h);
        m[32..].copy_from_slice(&h);
        h = Sha512_256::digest(m).into();
    }
    out.put("std_parent", (env::cycle_count() - t) / n as u64);
    out.put("std_parent_chk", first8(&h));

    // Leaf data sizes: tag||tag (64) + blockhash 32 + outpoint 36 + header code 4 + amount 8 + script
    // (compact size + bytes). The host passes the total preimage lengths.
    for len in &leaf_lens {
        let mut m = vec![0u8; *len as usize];
        let mut h = [0u8; 32];
        let t = env::cycle_count();
        for _ in 0..n {
            m[64..96].copy_from_slice(&h);
            h = Sha512_256::digest(&m).into();
        }
        out.put(&format!("std_leaf_{len}"), (env::cycle_count() - t) / n as u64);
        out.put(&format!("std_leaf_{len}_chk"), first8(&h));
    }
}
