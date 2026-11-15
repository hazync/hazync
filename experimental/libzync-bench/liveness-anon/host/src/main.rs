//! liveanon: EXECUTES (no proof) the liveness-anon guest on the real mainnet 1..200 proof and prints its cycles
//! (docs/libzync/LIVENESS_ZAPP.md §8).
//!
//!   liveanon set <core receipt> <liveness proof hex> <set depth>   policy 0: the coin's script in a set of 2^depth
//!   liveanon key                                                    policy 1's own work (sk·G + nullifier), alone
//!   liveanon wrong <core receipt> <liveness proof hex>              must FAIL: min_value above the coin's amount
use risc0_zkvm::{default_executor, ExecutorEnv, Receipt};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize)]
struct Coin { txid: [u8; 32], vout: u32, value: u64, script_pubkey: Vec<u8>, height: u32, coinbase: bool, tracked: bool, creation_mtp: u32 }

#[derive(Serialize)]
struct Input {
    core_id: [u32; 8], journal: Vec<u8>, signet: bool, policy: u32, min_value: u64, set_root: [u8; 32], set_depth: u32, context: [u8; 32],
    coin: Coin, pos: u64, path: Vec<[u8; 32]>, set_index: u64, set_path: Vec<[u8; 32]>, sk: [u8; 32], bench_key_only: bool,
}

fn node(l: &[u8; 32], r: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([1u8]);
    h.update(l);
    h.update(r);
    h.finalize().into()
}

/// A set leaf: SHA-256(0x00 ‖ len u32 LE ‖ spk).
fn leaf(spk: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([0u8]);
    h.update((spk.len() as u32).to_le_bytes());
    h.update(spk);
    h.finalize().into()
}

/// A set of 2^depth scripts with `spk` at `index` (the others are a counter's bytes): (root, path).
fn set_of(spk: &[u8], depth: u32, index: u64) -> ([u8; 32], Vec<[u8; 32]>) {
    let mut level: Vec<[u8; 32]> = (0..1u64 << depth).map(|i| if i == index { leaf(spk) } else { leaf(&i.to_le_bytes()) }).collect();
    let mut path = Vec::new();
    let mut i = index as usize;
    while level.len() > 1 {
        path.push(level[i ^ 1]);
        level = level.chunks_exact(2).map(|c| node(&c[0], &c[1])).collect();
        i /= 2;
    }
    (level[0], path)
}

fn run(inp: &Input, assumption: Option<&Receipt>) -> Result<(u64, u64, usize), String> {
    run_j(inp, assumption).map(|(c, t, n, _)| (c, t, n))
}

fn run_j(inp: &Input, assumption: Option<&Receipt>) -> Result<(u64, u64, usize, Vec<u8>), String> {
    let mut b = ExecutorEnv::builder();
    if let Some(r) = assumption {
        b.add_assumption(r.clone());
    }
    let env = b.write(inp).map_err(|e| e.to_string())?.build().map_err(|e| e.to_string())?;
    let t = std::time::Instant::now();
    let s = default_executor().execute(env, liveanon_methods::LIVEANON_ELF).map_err(|e| e.to_string())?;
    let total: u64 = s.segments.iter().map(|g| 1u64 << g.po2).sum();
    eprintln!("execute: {:.2} s wall", t.elapsed().as_secs_f64());
    Ok((s.cycles(), total, s.segments.len(), s.journal.bytes.clone()))
}

/// A refusal counts only as the GUEST's own refusal for the stated reason (a host or r0vm error is not one).
fn refuse(name: &str, r: Result<(u64, u64, usize), String>, want: &str) {
    match r {
        Ok(_) => { println!("LIVEANON {name}: ACCEPTED (bad)"); std::process::exit(1) }
        Err(e) if e.contains("Guest panicked") && e.contains(want) => println!("LIVEANON {name}: refused ({})", e.lines().next().unwrap_or("")),
        Err(e) => { println!("LIVEANON {name}: failed for another reason, not a refusal: {}", e.lines().next().unwrap_or("")); std::process::exit(1) }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let core_id: [u32; 8] = {
        let h = "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4";
        let b: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap()).collect();
        core::array::from_fn(|i| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap()))
    };
    let load = |rc: &str, lv: &str| -> (Receipt, libzync::liveness::Proof) {
        let r: Receipt = bincode::deserialize(&std::fs::read(rc).unwrap()).unwrap();
        let t = std::fs::read_to_string(lv).unwrap();
        let t = t.trim();
        let b: Vec<u8> = (0..t.len()).step_by(2).map(|i| u8::from_str_radix(&t[i..i + 2], 16).unwrap()).collect();
        (r, libzync::liveness::decode(&b).unwrap())
    };
    let input = |r: &Receipt, p: &libzync::liveness::Proof, policy: u32, min_value: u64, set_root, set_index, set_path: Vec<[u8; 32]>| Input {
        core_id, journal: r.journal.bytes.clone(), signet: false, policy, min_value, set_root, set_depth: set_path.len() as u32, context: [7; 32],
        coin: Coin { txid: p.coin.txid, vout: p.coin.vout, value: p.coin.value, script_pubkey: p.coin.script_pubkey.clone(), height: p.coin.height,
                     coinbase: p.coin.coinbase, tracked: p.coin.tracked, creation_mtp: p.coin.creation_mtp },
        pos: p.pos, path: p.path.clone(), set_index, set_path, sk: [0; 32], bench_key_only: false,
    };
    match a.first().map(|s| s.as_str()) {
        Some("set") => {
            let (r, p) = load(&a[1], &a[2]);
            assert!(p.opening.is_none() && p.state == libzync::liveness::State::Unspent, "want an unspent-at-the-tip proof");
            let depth: u32 = a[3].parse().unwrap();
            let idx = (1u64 << depth) / 3;
            let (root, path) = set_of(&p.coin.script_pubkey, depth, idx);
            let inp = input(&r, &p, 0, p.coin.value / 2, root, idx, path);
            let (u, t, n) = run(&inp, Some(&r)).unwrap();
            println!("LIVEANON set depth {depth}: {u} user cycles, {n} segment(s), {t} total cycles (padded to 2^po2), log path {} hashes, core journal {} bytes", p.path.len(), r.journal.bytes.len());
        }
        Some("key") => {
            let mut sk = [0u8; 32];
            sk[31] = 1;
            sk[0] = 0x3c;
            let mk = |sk: [u8; 32]| Input { core_id, journal: vec![], signet: false, policy: 1, min_value: 0, set_root: [0; 32], set_depth: 0, context: [7; 32],
                              coin: Coin { txid: [9; 32], vout: 0, value: 0, script_pubkey: vec![], height: 0, coinbase: false, tracked: false, creation_mtp: 0 },
                              pos: 0, path: vec![], set_index: 0, set_path: vec![], sk, bench_key_only: true };
            let (u, t, n, j1) = run_j(&mk(sk), None).unwrap();
            println!("LIVEANON key work alone: {u} user cycles, {n} segment(s), {t} total cycles");
            // n − sk meets the same x-only key: the nullifier must not change (one per coin)
            use k256::elliptic_curve::PrimeField;
            let d = Option::<k256::Scalar>::from(k256::Scalar::from_repr(sk.into())).unwrap();
            let neg: [u8; 32] = (-d).to_repr().into();
            let (_, _, _, j2) = run_j(&mk(neg), None).unwrap();
            if j1 == j2 { println!("LIVEANON key: sk and n-sk give the same x-only key and the same nullifier"); }
            else { println!("LIVEANON key: sk and n-sk DIFFER (bad)"); std::process::exit(1) }
        }
        Some("wrong") => {
            let (r, p) = load(&a[1], &a[2]);
            let (root, path) = set_of(&p.coin.script_pubkey, 4, 5);
            let inp = input(&r, &p, 0, p.coin.value + 1, root, 5, path);
            refuse("wrong", run(&inp, Some(&r)), "worth less than min_value");
            let (root, path) = set_of(b"another script", 4, 5);
            let inp = input(&r, &p, 0, 1, root, 5, path);
            refuse("not-in-set", run(&inp, Some(&r)), "not in the set");
            // the coin's script in the set, but the path one level shorter than the committed depth
            let (root, path) = set_of(&p.coin.script_pubkey, 4, 5);
            let mut inp = input(&r, &p, 0, 1, root, 5, path);
            inp.set_path.pop();
            refuse("short-path", run(&inp, Some(&r)), "not the committed depth");
            // another network's anchor: the mainnet receipt asked as signet is refused (profile and pinned anchor)
            let (root, path) = set_of(&p.coin.script_pubkey, 4, 5);
            let mut inp = input(&r, &p, 0, 1, root, 5, path);
            inp.signet = true;
            refuse("as-signet", run(&inp, Some(&r)), "core statement");
        }
        _ => { eprintln!("usage: see the source header"); std::process::exit(1) }
    }
}
