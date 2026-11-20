//! The tip mode's production witness source (prover/zyncapps/host/src/utreexo_forest.rs and utreexo_witness.rs,
//! docs/libzync/UTREEXO_ZAPP.md §10.5.1) against rustreexo's own MemForest and Stump, natively:
//!
//! - `forest_*`: ArenaForest is MemForest. Random adds and deletions (whole trees, roots, single leaves, any order),
//!   every proof compared with MemForest's (targets and hashes, byte for byte) and checked by `Stump::modify`, the
//!   roots compared after every batch, the arena checked from the leaves up, written and read back mid-run.
//!   Controls: a corrupted inner node makes the proofs differ, `Stump::modify` refuse, `check` fail and `read_from`
//!   refuse the written forest.
//! - `source_*`: ForestSource over a synthetic chain (coinbases, spends across blocks and inside one block,
//!   OP_RETURN): its witnesses equal a MemForest replay's, a lying node (a prevout's value, height or coinbase flag) and
//!   a lying store (inner nodes) are refused natively; a restart from a saved file gives the same witnesses; corrupt,
//!   truncated and lying files are refused at load; the test-only lies get past the source but not the guest's check
//!   (`block_changes` + `Stump::modify` on the emitted bytes).
use crate::step;
use crate::utreexo_forest::ArenaForest;
use crate::utreexo_witness::{ForestSource, Lie, PrevCoin};
use rustreexo::mem_forest::MemForest;
use rustreexo::node_hash::BitcoinNodeHash;
use rustreexo::proof::Proof;
use rustreexo::stump::Stump;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

type H = [u8; 32];

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn leaf(n: u64, collide: bool) -> BitcoinNodeHash {
    let mut h: H = Sha256::digest(n.to_le_bytes()).into();
    if collide {
        // the first 8 bytes (the leaf index's key) shared by every such leaf
        h[..8].copy_from_slice(&[0xab; 8]);
    }
    BitcoinNodeHash::Some(h)
}

fn same_proof(a: &Proof, b: &Proof) -> bool {
    a.targets == b.targets && a.hashes == b.hashes
}

/// One random history: `rounds` batches of adds and deletions on MemForest, ArenaForest and a Stump.
fn differential(seed: u64, rounds: usize, collide_every: u64) {
    let mut r = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let (mut m, mut a, mut s) = (MemForest::new(), ArenaForest::new(), Stump::new());
    let mut live: Vec<BitcoinNodeHash> = Vec::new();
    let mut n = seed << 32;
    for round in 0..rounds {
        // deletions: none, a few, a large share or (sometimes) everything, in a random order
        let k = match r.below(6) {
            0 => 0,
            1 | 2 => r.below(4) + 1,
            3 | 4 => r.below(live.len() as u64 / 2 + 1),
            _ => live.len() as u64,
        }
        .min(live.len() as u64) as usize;
        let mut dels = Vec::new();
        for _ in 0..k {
            let i = r.below(live.len() as u64) as usize;
            dels.push(live.swap_remove(i));
        }
        let adds: Vec<BitcoinNodeHash> = (0..r.below(40))
            .map(|_| {
                n += 1;
                leaf(n, collide_every > 0 && n % collide_every == 0)
            })
            .collect();
        let proof = if dels.is_empty() { Proof::default() } else {
            let pm = m.prove(&dels).unwrap();
            let pa = a.prove(&dels).unwrap();
            assert!(same_proof(&pm, &pa), "seed {seed} round {round}: the proofs differ");
            pa
        };
        s = s.modify(&adds, &dels, &proof).unwrap_or_else(|e| panic!("seed {seed} round {round}: Stump::modify refused: {e:?}"));
        m.modify(&adds, &dels).unwrap();
        a.modify(&adds, &dels).unwrap();
        let mr: Vec<BitcoinNodeHash> = m.get_roots().iter().map(|x| x.get_data()).collect();
        assert_eq!(mr, a.get_roots(), "seed {seed} round {round}: the roots differ from MemForest's");
        assert_eq!(mr, s.roots, "seed {seed} round {round}: the roots differ from the Stump's");
        assert_eq!(m.leaves, a.leaves);
        live.extend(adds);
        if round % 17 == 16 {
            a.check().unwrap();
            let mut b = Vec::new();
            a.write_to(&mut b).unwrap();
            a = ArenaForest::read_from(&mut &b[..]).unwrap_or_else(|e| panic!("seed {seed}: read back: {e}"));
            assert_eq!(a.get_roots(), mr);
        }
    }
    // every live leaf provable at the end, in one batch
    if !live.is_empty() {
        assert!(same_proof(&m.prove(&live).unwrap(), &a.prove(&live).unwrap()), "seed {seed}: the final proofs differ");
    }
    assert_eq!(a.counts().1, live.len());
}

#[test]
fn forest_equals_memforest_on_random_histories() {
    for seed in 0..120 {
        differential(seed, 60, 0);
    }
}

#[test]
fn forest_equals_memforest_with_leaf_index_collisions() {
    // every 3rd leaf shares its 8-byte index key with all the others: the overflow map and its promotion on delete
    for seed in 500..540 {
        differential(seed, 50, 3);
    }
}

#[test]
fn forest_edge_cases() {
    // one leaf; delete it (an Empty root); add again (the Empty root is skipped); delete everything twice over
    let (mut m, mut a) = (MemForest::new(), ArenaForest::new());
    let l: Vec<BitcoinNodeHash> = (0..9).map(|i| leaf(1000 + i, false)).collect();
    for (adds, dels) in [(&l[..1], &[][..]), (&[][..], &l[..1]), (&l[1..2], &[][..]), (&l[2..9], &[][..]), (&[][..], &l[1..9]),
                         (&l[..1], &[][..])] {
        if !dels.is_empty() {
            assert!(same_proof(&m.prove(dels).unwrap(), &a.prove(dels).unwrap()));
        }
        m.modify(adds, dels).unwrap();
        a.modify(adds, dels).unwrap();
        let mr: Vec<BitcoinNodeHash> = m.get_roots().iter().map(|x| x.get_data()).collect();
        assert_eq!(mr, a.get_roots());
    }
    // a repeated live leaf is refused (MemForest would silently re-map it); an unknown deletion is an error
    assert!(a.modify(&l[..1], &[]).is_err());
    assert!(a.prove(&[leaf(77, false)]).is_err());
}

#[test]
fn forest_control_a_corrupt_inner_node_is_caught() {
    let (mut m, mut a) = (MemForest::new(), ArenaForest::new());
    let l: Vec<BitcoinNodeHash> = (0..64).map(|i| leaf(2000 + i, false)).collect();
    m.modify(&l, &[]).unwrap();
    a.modify(&l, &[]).unwrap();
    let mut s = Stump::new();
    s = s.modify(&l, &[], &Proof::default()).unwrap();
    let dels = [l[5], l[40]];
    assert!(same_proof(&m.prove(&dels).unwrap(), &a.prove(&dels).unwrap()), "control: the healthy forest");
    assert!(a.corrupt_inner_nodes() > 0);
    let bad = a.prove(&dels).unwrap();
    assert!(!same_proof(&m.prove(&dels).unwrap(), &bad), "a corrupted inner node must change the proof");
    assert!(s.modify(&[], &dels, &bad).is_err(), "Stump::modify must refuse the corrupted proof");
    assert!(a.check().is_err(), "check must see the stale inner hash");
    let mut b = Vec::new();
    a.write_to(&mut b).unwrap();
    let e = ArenaForest::read_from(&mut &b[..]).err().expect("read_from must refuse a forest whose inner hashes lie");
    assert!(e.contains("not the hash of its children"), "{e}");
}

// ---------------------------------------------------------------------------------------------------------------
// A synthetic chain for ForestSource

fn put_tx(b: &mut Vec<u8>, script_sig: &[u8], inputs: &[(H, u32)], outputs: &[(u64, Vec<u8>)]) -> H {
    let s0 = b.len();
    b.extend_from_slice(&1u32.to_le_bytes());
    b.push(inputs.len() as u8);
    for (t, v) in inputs {
        b.extend_from_slice(t);
        b.extend_from_slice(&v.to_le_bytes());
        b.push(script_sig.len() as u8);
        b.extend_from_slice(script_sig);
        b.extend_from_slice(&u32::MAX.to_le_bytes());
    }
    b.push(outputs.len() as u8);
    for (v, spk) in outputs {
        b.extend_from_slice(&v.to_le_bytes());
        b.push(spk.len() as u8);
        b.extend_from_slice(spk);
    }
    b.extend_from_slice(&0u32.to_le_bytes());
    step::dsha(&b[s0..])
}

fn merkle(mut level: Vec<H>) -> H {
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            level.push(*level.last().unwrap());
        }
        level = level.chunks_exact(2).map(|c| {
            let mut p = [0u8; 64];
            p[..32].copy_from_slice(&c[0]);
            p[32..].copy_from_slice(&c[1]);
            step::dsha(&p)
        }).collect();
    }
    level[0]
}

#[derive(Clone)]
struct Coin {
    height: u32,
    coinbase: bool,
    value: u64,
    spk: Vec<u8>,
}

struct Chain {
    blocks: Vec<(Vec<u8>, Vec<Vec<PrevCoin>>)>,
    genesis: H,
}

/// `n` blocks after a fake genesis: each a coinbase (2 outputs, one OP_RETURN), then up to 6 transactions spending
/// random live coins (1-3 inputs) into 1-3 outputs, sometimes spending an output of the block's previous transaction.
fn chain(seed: u64, n: u32) -> Chain {
    let mut r = Rng(seed | 1);
    let genesis: H = Sha256::digest(b"fake genesis").into();
    let mut live: BTreeMap<(H, u32), Coin> = BTreeMap::new();
    let (mut prev, mut blocks) = (genesis, Vec::new());
    for h in 1..=n {
        let mut txs = Vec::new();
        let mut ids = Vec::new();
        let mut pv: Vec<Vec<PrevCoin>> = Vec::new();
        let cb_out = vec![(50 + h as u64, vec![0x51, h as u8, (h >> 8) as u8]), (0, vec![0x6a, 0x01, 0x02])];
        ids.push(put_tx(&mut txs, &[h as u8, (h >> 8) as u8, 0x51], &[([0; 32], u32::MAX)], &cb_out));
        let mut created: Vec<((H, u32), Coin)> = vec![((ids[0], 0), Coin { height: h, coinbase: true, value: cb_out[0].0, spk: cb_out[0].1.clone() })];
        for _ in 0..r.below(7) {
            let mut ins = Vec::new();
            let mut prevs = Vec::new();
            let same_block = r.below(4) == 0 && created.len() > 1;
            if same_block {
                // spend the newest output of this block (never a leaf: not a deletion)
                let (op, c) = created.pop().unwrap();
                ins.push(op);
                prevs.push(PrevCoin { height: h, coinbase: c.coinbase, value: c.value, spk: c.spk });
            }
            for _ in 0..r.below(3) + 1 {
                if live.is_empty() {
                    break;
                }
                let k = *live.keys().nth(r.below(live.len() as u64) as usize).unwrap();
                let c = live.remove(&k).unwrap();
                ins.push(k);
                prevs.push(PrevCoin { height: c.height, coinbase: c.coinbase, value: c.value, spk: c.spk });
            }
            if ins.is_empty() {
                continue;
            }
            let outs: Vec<(u64, Vec<u8>)> = (0..r.below(3) + 1).map(|i| (r.below(1000) + i, vec![0x51, r.below(250) as u8, 0x87])).collect();
            let id = put_tx(&mut txs, &[0x51], &ins, &outs);
            ids.push(id);
            pv.push(prevs);
            for (v, (val, spk)) in outs.iter().enumerate() {
                created.push(((id, v as u32), Coin { height: h, coinbase: false, value: *val, spk: spk.clone() }));
            }
        }
        live.extend(created);
        let mut b = Vec::new();
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&prev);
        b.extend_from_slice(&merkle(ids.clone()));
        b.extend_from_slice(&h.to_le_bytes());
        b.extend_from_slice(&[0u8; 8]);
        b.push(ids.len() as u8);
        b.extend_from_slice(&txs);
        prev = step::dsha(&b[..80]);
        blocks.push((b, pv));
    }
    Chain { blocks, genesis }
}

/// The reference witness: a MemForest replay of the same block rules (what utreexo_tip_host::Native does, minus its coin
/// map: the spent data comes from the same prevouts).
fn reference(c: &Chain) -> Vec<(Vec<u8>, Vec<u8>)> {
    let tag = crate::utreexo_roots::tag_state();
    let (mut f, mut s) = (MemForest::new(), Stump::new());
    let mut hashes = vec![c.genesis];
    let mut out = Vec::new();
    for (i, (b, pv)) in c.blocks.iter().enumerate() {
        let h = i as u32 + 1;
        let p = step::parse_block(b, hashes.last().unwrap()).unwrap();
        let mut by = std::collections::HashMap::new();
        for (t, x) in p.txs[1..].iter().zip(pv) {
            for (i, c) in t.inputs.iter().zip(x) {
                by.insert(*i, c.clone());
            }
        }
        let mut spent = Vec::new();
        for op in step::deletion_outpoints(&p).unwrap() {
            let c = &by[&op];
            step::put_spent(&mut spent, &hashes[c.height as usize], (c.height << 1) | c.coinbase as u32, c.value, &c.spk);
        }
        let ch = step::block_changes(&tag, b, hashes.last().unwrap(), h, &spent, &mut step::Counts::default()).unwrap();
        let proof = if ch.dels.is_empty() { Proof::default() } else { f.prove(&ch.dels).unwrap() };
        s = step::apply(&s, &ch, &proof, &mut step::Counts::default()).unwrap();
        f.modify(&ch.adds, &ch.dels).unwrap();
        hashes.push(ch.hash);
        out.push((spent, step::encode_proof(&proof)));
    }
    out
}

fn run(src: &mut ForestSource, c: &Chain, from: usize, to: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    (from..to).map(|i| {
        let w = src.block(&c.blocks[i].0, &c.blocks[i].1).unwrap_or_else(|e| panic!("{e}"));
        (w.spent, w.proof)
    }).collect()
}

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("hz-utreexo-witness-{}-{name}", std::process::id()));
    d.to_string_lossy().into_owned()
}

#[test]
fn source_witnesses_equal_a_memforest_replay() {
    for seed in [1u64, 2, 3, 99] {
        let c = chain(seed, 300);
        let want = reference(&c);
        let mut src = ForestSource::genesis(c.genesis);
        let got = run(&mut src, &c, 0, 300);
        let dels: usize = got.iter().filter(|w| w.0.len() > 0).count();
        assert!(dels > 100, "seed {seed}: the chain must spend ({dels} blocks with deletions)");
        assert!(got == want, "seed {seed}: the witnesses differ from the MemForest replay's");
        assert!(src.forest_agrees());
        src.forest.check().unwrap();
    }
}

#[test]
fn source_refuses_a_lying_node() {
    let c = chain(7, 120);
    // the first block with a deletion
    let k = reference(&c).iter().position(|w| !w.0.is_empty()).unwrap();
    for (what, f) in [
        ("value", Box::new(|p: &mut PrevCoin| p.value += 1) as Box<dyn Fn(&mut PrevCoin)>),
        ("height", Box::new(|p: &mut PrevCoin| p.height -= 1)),
        ("coinbase flag", Box::new(|p: &mut PrevCoin| p.coinbase = !p.coinbase)),
        ("script", Box::new(|p: &mut PrevCoin| p.spk.push(0))),
    ] {
        let mut src = ForestSource::genesis(c.genesis);
        run(&mut src, &c, 0, k);
        let mut pv = c.blocks[k].1.clone();
        // the first input whose prevout is below this block (a deletion)
        let (i, j) = pv.iter().enumerate().find_map(|(i, t)| t.iter().position(|p| p.height <= k as u32).map(|j| (i, j))).unwrap();
        f(&mut pv[i][j]);
        let e = src.block(&c.blocks[k].0, &pv).err().unwrap_or_else(|| panic!("a lying prevout {what} was accepted"));
        assert!(e.contains("is not a live leaf of the forest") || e.contains("claims height"), "{what}: {e}");
        // control: the source is untouched by the refused block and takes the honest one
        src.block(&c.blocks[k].0, &c.blocks[k].1).unwrap();
    }
    // a prevout at or above the block's own height is refused before any lookup
    let mut src = ForestSource::genesis(c.genesis);
    run(&mut src, &c, 0, k);
    let mut pv = c.blocks[k].1.clone();
    let (i, j) = pv.iter().enumerate().find_map(|(i, t)| t.iter().position(|p| p.height <= k as u32).map(|j| (i, j))).unwrap();
    pv[i][j].height = k as u32 + 1;
    assert!(src.block(&c.blocks[k].0, &pv).unwrap_err().contains("claims height"));
    // a prevout list that does not fit the block
    let mut pv = c.blocks[k].1.clone();
    pv[i].pop();
    assert!(src.block(&c.blocks[k].0, &pv).unwrap_err().contains("prevouts"));
}

#[test]
fn source_refuses_a_lying_store() {
    let c = chain(8, 150);
    let mut src = ForestSource::genesis(c.genesis);
    run(&mut src, &c, 0, 100);
    src.forest.corrupt_inner_nodes();
    let e = (100..150).find_map(|i| src.block(&c.blocks[i].0, &c.blocks[i].1).err()).expect("a corrupted store must be refused");
    assert!(e.contains("do not open"), "{e}");
}

#[test]
fn source_restart_from_a_saved_file() {
    let c = chain(9, 240);
    let want = reference(&c);
    let p = tmp("restart.bin");
    let mut a = ForestSource::genesis(c.genesis);
    let first = run(&mut a, &c, 0, 160);
    let n = a.save(&p).unwrap();
    assert_eq!(n, std::fs::metadata(&p).unwrap().len());
    let mut b = ForestSource::load(&p).unwrap();
    assert_eq!((b.height, b.last, b.roots()), (a.height, a.last, a.roots()));
    let rest = run(&mut b, &c, 160, 240);
    assert!(first.iter().chain(&rest).cloned().collect::<Vec<_>>() == want, "a restarted source gives other witnesses");
    // the file itself: a flipped byte anywhere, a truncation, a trailing byte, another magic: all refused
    let bytes = std::fs::read(&p).unwrap();
    for at in [9usize, 50, bytes.len() / 2, bytes.len() - 40, bytes.len() - 1] {
        let mut x = bytes.clone();
        x[at] ^= 1;
        std::fs::write(&p, &x).unwrap();
        assert!(ForestSource::load(&p).is_err(), "a flipped byte at {at} was accepted");
    }
    for x in [bytes[..bytes.len() - 1].to_vec(), [&bytes[..], &[0u8][..]].concat(), [&b"HZUTIP01"[..], &bytes[8..]].concat()] {
        std::fs::write(&p, &x).unwrap();
        assert!(ForestSource::load(&p).is_err());
    }
    // a store that LIES: inner nodes changed and the checksum made to match: refused by the recomputation at load
    let mut liar = ForestSource::load_or_genesis_for_test(&bytes, &p);
    liar.forest.corrupt_inner_nodes();
    liar.save(&p).unwrap();
    let e = ForestSource::load(&p).err().expect("a lying store with a valid checksum was accepted");
    assert!(e.contains("not the hash of its children"), "{e}");
    // control: the honest file loads
    std::fs::write(&p, &bytes).unwrap();
    assert!(ForestSource::load(&p).is_ok());
    let _ = std::fs::remove_file(&p);
}

impl ForestSource {
    fn load_or_genesis_for_test(bytes: &[u8], p: &str) -> ForestSource {
        std::fs::write(p, bytes).unwrap();
        ForestSource::load(p).unwrap()
    }
}

/// What the guest does with a frame, natively: rebuild the deletions from the bound block and the spent bytes, then
/// `Stump::modify` with the proof bytes.
fn guest_check(s: &Stump, block: &[u8], prev: &H, h: u32, spent: &[u8], proof: &[u8]) -> Result<Stump, String> {
    let tag = crate::utreexo_roots::tag_state();
    let ch = step::block_changes(&tag, block, prev, h, spent, &mut step::Counts::default())?;
    let p = step::decode_proof(proof)?;
    Ok(step::apply(s, &ch, &p, &mut step::Counts::default())?)
}

#[test]
fn source_lies_get_past_the_source_but_not_the_guest() {
    let c = chain(10, 160);
    for lie in [Lie::Node, Lie::Store] {
        let mut src = ForestSource::genesis(c.genesis);
        run(&mut src, &c, 0, 100);
        let (mut s, mut prev) = (src.stump.clone(), src.last);
        // control: the honest source's next frames pass the guest's check
        let mut honest = ForestSource::load_or_genesis_for_test(&{
            let p = tmp("ctl.bin");
            src.save(&p).unwrap();
            let b = std::fs::read(&p).unwrap();
            let _ = std::fs::remove_file(&p);
            b
        }, &tmp("ctl2.bin"));
        let (mut hs, mut hp) = (s.clone(), prev);
        for i in 100..160 {
            let w = honest.block(&c.blocks[i].0, &c.blocks[i].1).unwrap();
            hs = guest_check(&hs, &c.blocks[i].0, &hp, i as u32 + 1, &w.spent, &w.proof).unwrap();
            hp = w.hash;
        }
        // the lying source: no error from the source, a refusal from the guest's check at the first deletion
        src.set_lie(lie);
        let mut refused = None;
        for i in 100..160 {
            let w = src.block(&c.blocks[i].0, &c.blocks[i].1).unwrap_or_else(|e| panic!("{lie:?}: the lying source stopped itself: {e}"));
            match guest_check(&s, &c.blocks[i].0, &prev, i as u32 + 1, &w.spent, &w.proof) {
                Ok(n) => { s = n; prev = w.hash; }
                Err(e) => { refused = Some(e); break; }
            }
        }
        let e = refused.unwrap_or_else(|| panic!("{lie:?}: the guest's check accepted a lying source"));
        assert!(e.contains("do not open"), "{lie:?}: {e}");
        let _ = std::fs::remove_file(tmp("ctl2.bin"));
    }
}
