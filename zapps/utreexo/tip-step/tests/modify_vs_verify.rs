//! Review R3-4 I1: the utreexo tip step (prover/zyncapps/methods/utreexo-tip/src/step.rs) calls rustreexo 0.7.0's
//! `Stump::modify(adds, dels, proof)` alone, where Floresta calls `Stump::verify` first. Differential: on random
//! accumulators and honest batch proofs, then many mutations of the proof and of the deletions, `modify` alone must
//! never accept a (proof, deletions) pair that `verify` refuses. A pair `modify` accepts must also give the honest
//! result's roots whenever it deletes the same leaves. Either failing is a soundness finding (the tip image would accept
//! a deletion that does not open in the accumulator).
use rustreexo::mem_forest::MemForest;
use rustreexo::node_hash::BitcoinNodeHash;
use rustreexo::proof::Proof;
use rustreexo::stump::Stump;
use sha2::{Digest, Sha256};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn leaf(n: u64) -> BitcoinNodeHash {
    BitcoinNodeHash::Some(Sha256::digest(n.to_le_bytes()).into())
}

fn flip(h: &BitcoinNodeHash, bit: u64) -> BitcoinNodeHash {
    let mut b = **h;
    b[(bit / 8 % 32) as usize] ^= 1 << (bit % 8);
    BitcoinNodeHash::Some(b)
}

/// The mutations of an honest (proof, dels): each a different way to lie about which leaves are deleted, or where.
fn mutations(r: &mut Rng, p: &Proof, dels: &[BitcoinNodeHash], live: &[BitcoinNodeHash], leaves: u64) -> Vec<(String, Proof, Vec<BitcoinNodeHash>)> {
    let mut out = Vec::new();
    let mk = |t: Vec<u64>, h: Vec<BitcoinNodeHash>| Proof::new(t, h);
    if !p.hashes.is_empty() {
        let i = r.below(p.hashes.len() as u64) as usize;
        let mut h = p.hashes.clone();
        h[i] = flip(&h[i], r.next());
        out.push(("a proof hash with one bit flipped".into(), mk(p.targets.clone(), h), dels.to_vec()));
        let mut h = p.hashes.clone();
        h.remove(i);
        out.push(("a proof hash dropped".into(), mk(p.targets.clone(), h), dels.to_vec()));
        let mut h = p.hashes.clone();
        h.push(p.hashes[i]);
        out.push(("a proof hash repeated at the end".into(), mk(p.targets.clone(), h), dels.to_vec()));
        if p.hashes.len() > 1 {
            let mut h = p.hashes.clone();
            h.swap(0, p.hashes.len() - 1);
            out.push(("two proof hashes swapped".into(), mk(p.targets.clone(), h), dels.to_vec()));
        }
    }
    let i = r.below(dels.len() as u64) as usize;
    let mut d = dels.to_vec();
    d[i] = flip(&d[i], r.next());
    out.push(("a deleted leaf's hash with one bit flipped (not in the accumulator)".into(), p.clone(), d));
    let mut d = dels.to_vec();
    d[i] = leaf(u64::MAX - r.below(1000));
    out.push(("a leaf that was never added".into(), p.clone(), d));
    if let Some(other) = live.iter().find(|x| !dels.contains(x)) {
        let mut d = dels.to_vec();
        d[i] = *other;
        out.push(("another live leaf in a deleted leaf's place".into(), p.clone(), d));
    }
    if dels.len() > 1 {
        let mut d = dels.to_vec();
        d.swap(0, dels.len() - 1);
        out.push(("the deleted leaves in another order".into(), p.clone(), d));
        let mut t = p.targets.clone();
        t.swap(0, p.targets.len() - 1);
        out.push(("the targets in another order".into(), mk(t, p.hashes.clone()), dels.to_vec()));
        let mut t = p.targets.clone();
        t[1] = t[0];
        out.push(("a target repeated".into(), mk(t, p.hashes.clone()), dels.to_vec()));
    }
    let mut t = p.targets.clone();
    t[i] ^= 1;
    out.push(("a target moved to its sibling".into(), mk(t, p.hashes.clone()), dels.to_vec()));
    let mut t = p.targets.clone();
    t[i] = t[i].wrapping_add(1 + r.below(leaves.max(2)));
    out.push(("a target moved elsewhere".into(), mk(t, p.hashes.clone()), dels.to_vec()));
    let mut t = p.targets.clone();
    t[i] = leaves + r.below(64);
    out.push(("a target past the leaf count".into(), mk(t, p.hashes.clone()), dels.to_vec()));
    let mut t = p.targets.clone();
    t.pop();
    out.push(("a target dropped (more deletions than targets)".into(), mk(t, p.hashes.clone()), dels.to_vec()));
    let mut d = dels.to_vec();
    d.pop();
    if !d.is_empty() {
        out.push(("a deletion dropped (more targets than deletions)".into(), p.clone(), d));
    }
    out
}

#[test]
fn modify_alone_never_accepts_what_verify_refuses() {
    let (mut tried, mut both_refuse, mut agreed_ok) = (0u64, 0u64, 0u64);
    for seed in 1..=60u64 {
        let mut r = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
        let (mut m, mut s) = (MemForest::new(), Stump::new());
        let mut live: Vec<BitcoinNodeHash> = Vec::new();
        let mut n = seed << 32;
        for round in 0..12 {
            let adds: Vec<BitcoinNodeHash> = (0..1 + r.below(60)).map(|_| { n += 1; leaf(n) }).collect();
            // a batch to test against: 1..k of the live leaves
            if !live.is_empty() {
                let k = 1 + r.below(live.len().min(12) as u64) as usize;
                let mut pool = live.clone();
                let mut dels = Vec::new();
                for _ in 0..k {
                    let i = r.below(pool.len() as u64) as usize;
                    dels.push(pool.swap_remove(i));
                }
                let p = m.prove(&dels).unwrap();
                assert_eq!(s.verify(&p, &dels), Ok(true), "seed {seed} round {round}: the honest proof");
                let honest = s.modify(&adds, &dels, &p).expect("the honest proof");
                for (what, q, d) in mutations(&mut r, &p, &dels, &live, s.leaves) {
                    tried += 1;
                    let v = s.verify(&q, &d);
                    let md = s.modify(&adds, &d, &q);
                    match (v, md) {
                        (Ok(true), Ok(st)) => {
                            agreed_ok += 1;
                            // both accept: the same leaves deleted must give the same accumulator
                            let mut sd = d.clone();
                            let mut sh = dels.clone();
                            sd.sort_by_key(|h| **h);
                            sh.sort_by_key(|h| **h);
                            if sd == sh {
                                assert_eq!(st.roots, honest.roots, "seed {seed} round {round}: {what}: accepted with other roots");
                            }
                        }
                        (Ok(true), Err(_)) => {}
                        (_, Ok(st)) => panic!(
                            "SOUNDNESS: seed {seed} round {round}: {what}: Stump::modify ACCEPTED what Stump::verify refuses \
                             (roots equal the honest result's: {})", st.roots == honest.roots),
                        (_, Err(_)) => both_refuse += 1,
                    }
                }
            }
            // move on: apply an honest random batch so later rounds see other shapes
            let k = r.below(live.len() as u64 / 2 + 1) as usize;
            let mut dels = Vec::new();
            for _ in 0..k.min(live.len()) {
                let i = r.below(live.len() as u64) as usize;
                dels.push(live.swap_remove(i));
            }
            let p = if dels.is_empty() { Proof::default() } else { m.prove(&dels).unwrap() };
            s = s.modify(&adds, &dels, &p).unwrap();
            m.modify(&adds, &dels).unwrap();
            live.extend(adds);
        }
    }
    eprintln!("modify vs verify: {tried} mutated cases; both refused {both_refuse}; both accepted {agreed_ok}; modify alone accepted 0");
    assert!(both_refuse > tried / 2, "most mutations must be refusals, or the mutations test nothing");
}
