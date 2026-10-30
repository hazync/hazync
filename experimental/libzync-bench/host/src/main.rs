//! libzync-bench host. Builds inputs (real secp256k1 signatures via k256), runs the guest in the
//! r0vm EXECUTOR (no proving), and prints the guest's own cycle measurements.
//!
//!   zbench-host ops <n> <nsigs>
//!   zbench-host msm <n> <variant: 0 unsigned | 1 signed | 2 raw> <w1,w2,...>
//!   zbench-host batch <nsigs> <window> <single:0|1> <corrupt:0|1|2>
//!   zbench-host sethash <n>
//!   zbench-host utreexo <n> <leaf preimage lengths, comma separated>
//!
//! corrupt=1 flips a bit of one s and keeps the honest R hint  -> the MSM must not vanish.
//! corrupt=2 flips a bit of one s and re-derives R from it     -> x(R) == r must fail.
//! Either way the guest must report batch_valid = 0. That is the negative control.

use k256::elliptic_curve::ops::Reduce;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::{FieldElement, ProjectivePoint, Scalar, U256};
use methods::{ZBENCH_ELF as GUEST_ELF, ZBENCH_ID as GUEST_ID};
use risc0_zkvm::{default_executor, ExecutorEnv};
use serde::Serialize;
use sha2::{Digest, Sha256, Sha512_256};
use std::time::Instant;

#[derive(Serialize, Clone)]
struct Sig {
    qx: [u8; 32],
    qy: [u8; 32],
    e: [u8; 32],
    r: [u8; 32],
    s: [u8; 32],
    rx: [u8; 32],
    ry: [u8; 32],
}

fn h(tag: &str, i: u64) -> [u8; 32] {
    Sha256::digest([tag.as_bytes(), &i.to_le_bytes()].concat()).into()
}
fn sc(b: &[u8; 32]) -> Scalar {
    <Scalar as Reduce<U256>>::reduce_bytes(b.into())
}
fn xy(p: &ProjectivePoint) -> ([u8; 32], [u8; 32]) {
    let e = p.to_affine().to_encoded_point(false);
    (e.x().unwrap().as_slice().try_into().unwrap(), e.y().unwrap().as_slice().try_into().unwrap())
}

fn make_sigs(n: usize, corrupt: u32) -> Vec<Sig> {
    let mut v: Vec<Sig> = (0..n as u64)
        .map(|i| {
            let d = sc(&h("d", i));
            let k = sc(&h("k", i));
            let e = h("e", i);
            let q = ProjectivePoint::GENERATOR * d;
            let r_pt = ProjectivePoint::GENERATOR * k;
            let (rx0, _) = xy(&r_pt);
            let r = sc(&rx0);
            let s = k.invert().unwrap() * (sc(&e) + r * d);
            // The hint is what the VERIFIER equation gives, so low-s or not it is consistent.
            let w = s.invert().unwrap();
            let rh = ProjectivePoint::GENERATOR * (sc(&e) * w) + q * (r * w);
            let (qx, qy) = xy(&q);
            let (rx, ry) = xy(&rh);
            Sig { qx, qy, e, r: r.to_bytes().into(), s: s.to_bytes().into(), rx, ry }
        })
        .collect();
    if corrupt > 0 && !v.is_empty() {
        let j = v.len() / 2;
        let mut s = v[j].s;
        s[31] ^= 1;
        v[j].s = s;
        if corrupt == 2 {
            let (q, e, r) = (&v[j], sc(&v[j].e), sc(&v[j].r));
            let qp = ProjectivePoint::from(
                k256::AffinePoint::from_encoded_point(&k256::EncodedPoint::from_affine_coordinates(
                    (&q.qx).into(), (&q.qy).into(), false)).unwrap());
            let w = sc(&s).invert().unwrap();
            let (rx, ry) = xy(&(ProjectivePoint::GENERATOR * (e * w) + qp * (r * w)));
            v[j].rx = rx;
            v[j].ry = ry;
        }
    }
    v
}
use k256::elliptic_curve::sec1::FromEncodedPoint;

fn run(env: ExecutorEnv) -> Vec<(String, u64)> {
    let t = Instant::now();
    let info = default_executor().execute(env, GUEST_ELF).expect("guest execution failed");
    let out: Vec<(String, u64)> = info.journal.decode().expect("journal");
    let segs = info.segments.len();
    let padded: u64 = info.segments.iter().map(|s| 1u64 << s.po2).sum();
    println!(
        "# session: user cycles {}  segments {}  padded {}  exec {:.1}s",
        info.cycles(), segs, padded, t.elapsed().as_secs_f64()
    );
    for (k, v) in &out {
        println!("{k:>28} {v}");
    }
    out
}

fn get(out: &[(String, u64)], k: &str) -> u64 {
    out.iter().find(|(a, _)| a == k).map(|x| x.1).unwrap_or_else(|| panic!("missing {k}"))
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let arg = |i: usize, d: &str| a.get(i).cloned().unwrap_or_else(|| d.to_string());
    eprintln!("# guest id {}", hex(&bytemuck_id()));
    match a.get(1).map(String::as_str) {
        Some("ops") => {
            let n: u32 = arg(2, "2000").parse().unwrap();
            let ns: usize = arg(3, "50").parse().unwrap();
            let env = ExecutorEnv::builder().write(&1u32).unwrap().write(&n).unwrap()
                .write(&make_sigs(ns, 0)).unwrap().build().unwrap();
            let out = run(env);
            assert_eq!(get(&out, "ecdsa_single_valid"), ns as u64, "a valid signature failed single verify");
        }
        Some("msm") => {
            let n: u32 = arg(2, "1000").parse().unwrap();
            let variant: u32 = arg(3, "2").parse().unwrap();
            let ws: Vec<u32> = arg(4, "8").split(',').map(|x| x.parse().unwrap()).collect();
            let env = ExecutorEnv::builder().write(&2u32).unwrap().write(&n).unwrap().write(&ws).unwrap()
                .write(&variant).unwrap().build().unwrap();
            let out = run(env);
            // Independent reference: points are (i+2)·G, so the MSM is (Σ s_i·(i+2))·G.
            let mut seed = [0u8; 32];
            let mut total = Scalar::ZERO;
            for i in 0..n {
                seed = Sha256::digest([&seed[..], &i.to_le_bytes()].concat()).into();
                total += sc(&seed) * Scalar::from(i as u64 + 2);
            }
            let (ex, _) = xy(&(ProjectivePoint::GENERATOR * total));
            let mut got = [0u8; 32];
            for k in 0..4 {
                got[8 * k..8 * k + 8].copy_from_slice(&get(&out, &format!("res_x{k}")).to_be_bytes());
            }
            assert_eq!(got, ex, "MSM DISAGREES WITH THE k256 REFERENCE");
            println!("# msm result matches k256 reference");
            for w in &ws {
                println!("# w={w}: {:.0} cycles/point, {:.1} cycles/add-equivalent",
                    get(&out, &format!("w{w}_cycles")) as f64 / n as f64,
                    get(&out, &format!("w{w}_cycles")) as f64
                        / (get(&out, &format!("w{w}_adds")) + get(&out, &format!("w{w}_doubles"))) as f64);
            }
        }
        Some("batch") => {
            let n: usize = arg(2, "438").parse().unwrap();
            let w: u32 = arg(3, "8").parse().unwrap();
            let single = arg(4, "1") == "1";
            let corrupt: u32 = arg(5, "0").parse().unwrap();
            let env = ExecutorEnv::builder().write(&3u32).unwrap().write(&make_sigs(n, corrupt)).unwrap()
                .write(&w).unwrap().write(&single).unwrap().build().unwrap();
            let out = run(env);
            let valid = get(&out, "batch_valid");
            if corrupt == 0 {
                assert_eq!(valid, 1, "an all-valid batch was REJECTED");
            } else {
                assert_eq!(valid, 0, "NEGATIVE CONTROL FAILED: a corrupted batch was ACCEPTED");
                println!("# negative control (corrupt={corrupt}) correctly rejected");
            }
            let bt = get(&out, "batch_total") as f64 / n as f64;
            println!("# batch: {bt:.0} cycles/sig");
            if single {
                let st = get(&out, "single_total") as f64 / n as f64;
                println!("# single: {st:.0} cycles/sig  => batch is {:.2}x cheaper", st / bt);
            }
        }
        Some("sethash") => {
            let n: u32 = arg(2, "2000").parse().unwrap();
            let hints: Vec<(u32, [u8; 32], Vec<[u8; 32]>)> = (0..n).map(|i| ecmh_hint(i)).collect();
            let env = ExecutorEnv::builder().write(&4u32).unwrap().write(&n).unwrap().write(&hints).unwrap()
                .build().unwrap();
            run(env);
        }
        Some("utreexo") => {
            let n: u32 = arg(2, "2000").parse().unwrap();
            let lens: Vec<u32> = arg(3, "167,179").split(',').map(|x| x.parse().unwrap()).collect();
            let env = ExecutorEnv::builder().write(&5u32).unwrap().write(&n).unwrap().write(&lens).unwrap()
                .build().unwrap();
            let out = run(env);
            // Native recomputation of every chain: the guest's hashes must be the real ones.
            let f8 = |h: &[u8]| u64::from_le_bytes(h[..8].try_into().unwrap());
            let mut h = [0u8; 32];
            for _ in 0..n {
                h = Sha256::digest([&[0x01u8][..], &h, &h].concat()).into();
            }
            assert_eq!(get(&out, "ours_parent_chk"), f8(&h), "ours_parent mismatch");
            let mut h = [0u8; 32];
            for _ in 0..n {
                h = Sha512_256::digest([h, h].concat()).into();
            }
            assert_eq!(get(&out, "std_parent_chk"), f8(&h), "std_parent mismatch");
            for len in &lens {
                let mut m = vec![0u8; *len as usize];
                let mut h = [0u8; 32];
                for _ in 0..n {
                    m[64..96].copy_from_slice(&h);
                    h = Sha512_256::digest(&m).into();
                }
                assert_eq!(get(&out, &format!("std_leaf_{len}_chk")), f8(&h), "std_leaf_{len} mismatch");
            }
            println!("# all chains match native sha2");
            println!("# std parent / ours parent = {:.1}x",
                get(&out, "std_parent") as f64 / get(&out, "ours_parent") as f64);
        }
        _ => eprintln!("usage: zbench-host ops|msm|batch|sethash|utreexo ..."),
    }
}

/// For element i (= SHA256(i_le)), find the first ctr with x = SHA256(e||ctr) on the curve.
/// Returns (ctr, y, [witness w with w^2 = -(x^3+7) for every skipped ctr]).
fn ecmh_hint(i: u32) -> (u32, [u8; 32], Vec<[u8; 32]>) {
    let e: [u8; 32] = Sha256::digest(i.to_le_bytes()).into();
    let mut skipped = Vec::new();
    for ctr in 0u32.. {
        let xb: [u8; 32] = Sha256::digest([&e[..], &ctr.to_le_bytes()].concat()).into();
        let Some(x) = Option::<FieldElement>::from(FieldElement::from_bytes(&xb.into())) else {
            panic!("x >= p (p ~ 2^-224); pick another n");
        };
        let rhs = (x * x * x + FieldElement::from(7u64)).normalize();
        if let Some(y) = Option::<FieldElement>::from(rhs.sqrt()) {
            return (ctr, y.normalize().to_bytes().into(), skipped);
        }
        let w = Option::<FieldElement>::from((-rhs).normalize().sqrt()).expect("p = 3 mod 4: -rhs is a QR");
        skipped.push(w.normalize().to_bytes().into());
    }
    unreachable!()
}

fn bytemuck_id() -> [u8; 32] {
    let mut b = [0u8; 32];
    for (k, w) in GUEST_ID.iter().enumerate() {
        b[4 * k..4 * k + 4].copy_from_slice(&w.to_le_bytes());
    }
    b
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
