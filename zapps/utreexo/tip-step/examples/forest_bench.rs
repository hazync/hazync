//! A synthetic measurement of the tip mode's forest (UTREEXO_ZAPP.md §10.5.1), not a test: grow a forest block by
//! block to `live` leaves out of `adds` ever added (each block adds `per_block` leaves and deletes uniformly random live
//! ones at the rate that ends at `live`), then measure, over 20 blocks of `dels` deletions and as many adds: the batch
//! proof (time, hashes per deletion), the modify, and the memory (resident set, the arena's own count), then the
//! snapshot (write and read-back time, bytes). Uniform deletions share fewer proof paths than real blocks do.
//!   cargo run --release --example forest_bench -- <arena|memforest> <adds> <live> <per_block> <dels> [<snapshot file>]
use rustreexo::mem_forest::MemForest;
use rustreexo::node_hash::BitcoinNodeHash;
use rustreexo::proof::Proof;
use rustreexo::stump::Stump;
use sha2::{Digest, Sha256};
use std::time::Instant;
use zapp_utreexo_tip_step::utreexo_forest::ArenaForest;

fn leaf(n: u64) -> BitcoinNodeHash {
    BitcoinNodeHash::Some(Sha256::digest(n.to_le_bytes()).into())
}

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status").ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).map(|l| l.split_whitespace().nth(1).unwrap().parse().unwrap()))
        .unwrap_or(0)
}

enum F {
    A(ArenaForest),
    M(MemForest),
}

impl F {
    fn prove(&self, d: &[BitcoinNodeHash]) -> Proof {
        match self { F::A(a) => a.prove(d).unwrap(), F::M(m) => m.prove(d).unwrap() }
    }
    fn modify(&mut self, a: &[BitcoinNodeHash], d: &[BitcoinNodeHash]) {
        match self { F::A(f) => f.modify(a, d).unwrap(), F::M(m) => m.modify(a, d).unwrap() }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (mode, adds, live, per_block, dels): (&str, u64, u64, u64, usize) =
        (&a[1], a[2].parse().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap(), a[5].parse().unwrap());
    let mut f = if mode == "arena" { F::A(ArenaForest::new()) } else { F::M(MemForest::new()) };
    let mut s = Stump::new();
    let mut pool: Vec<BitcoinNodeHash> = Vec::with_capacity(live as usize + per_block as usize * 2);
    let mut rng = 0x9E3779B97F4A7C15u64;
    let mut next = || { rng ^= rng >> 12; rng ^= rng << 25; rng ^= rng >> 27; rng.wrapping_mul(0x2545F4914F6CDD1D) };
    let rss0 = rss_kib();
    let t = Instant::now();
    let (mut n, mut owed) = (0u64, 0f64);
    let del_rate = (adds - live) as f64 / adds as f64;
    while n < adds {
        let k = per_block.min(adds - n);
        owed += k as f64 * del_rate;
        let nd = (owed as usize).min(pool.len());
        owed -= nd as f64;
        let d: Vec<BitcoinNodeHash> = (0..nd).map(|_| { let i = (next() % pool.len() as u64) as usize; pool.swap_remove(i) }).collect();
        let ad: Vec<BitcoinNodeHash> = (n..n + k).map(leaf).collect();
        f.modify(&ad, &d);
        pool.extend_from_slice(&ad);
        n += k;
    }
    let build_s = t.elapsed().as_secs_f64();
    // the Stump of the grown forest (its roots), so the measured blocks are checked as the source checks them
    let roots = match &f { F::A(x) => x.get_roots(), F::M(m) => m.get_roots().iter().map(|r| r.get_data()).collect() };
    s.leaves = n;
    s.roots = roots;
    let rss1 = rss_kib();
    let (mut tp, mut tc, mut tm, mut hashes, mut nd) = (0f64, 0f64, 0f64, 0usize, 0usize);
    for b in 0..20u64 {
        let d: Vec<BitcoinNodeHash> = (0..dels.min(pool.len())).map(|_| { let i = (next() % pool.len() as u64) as usize; pool.swap_remove(i) }).collect();
        let ad: Vec<BitcoinNodeHash> = (0..dels as u64).map(|i| leaf(1 << 62 | b << 32 | i)).collect();
        let t0 = Instant::now();
        let p = f.prove(&d);
        let t1 = Instant::now();
        s = s.modify(&ad, &d, &p).expect("Stump::modify refused a bench proof");
        let t2 = Instant::now();
        f.modify(&ad, &d);
        tp += (t1 - t0).as_secs_f64();
        tc += (t2 - t1).as_secs_f64();
        tm += t2.elapsed().as_secs_f64();
        hashes += p.hashes.len();
        nd += d.len();
        pool.extend_from_slice(&ad);
    }
    println!("FOREST_BENCH mode={mode} adds={n} live={} rows={} build_s={build_s:.1} rss_mib={} rss_per_live_leaf={:.1} \
              per_block: dels={} prove_ms={:.1} stump_check_ms={:.1} modify_ms={:.1} proof_hashes_per_del={:.2} proof_bytes={}",
             pool.len(), 64 - (n - 1).leading_zeros(), (rss1 - rss0) / 1024, (rss1 - rss0) as f64 * 1024.0 / pool.len() as f64,
             nd / 20, tp * 50.0, tc * 50.0, tm * 50.0, hashes as f64 / nd as f64, 8 + hashes / 20 * 32 + nd / 20 * 8);
    if let (F::A(x), Some(path)) = (&f, a.get(6)) {
        let (nodes, leaves) = x.counts();
        println!("FOREST_BENCH arena nodes={nodes} leaves={leaves} heap_bytes={} heap_per_live_leaf={:.1}", x.heap_bytes(), x.heap_bytes() as f64 / leaves as f64);
        let t = Instant::now();
        let mut w = std::io::BufWriter::with_capacity(1 << 22, std::fs::File::create(path).unwrap());
        x.write_to(&mut w).unwrap();
        drop(w);
        let ws = t.elapsed().as_secs_f64();
        let bytes = std::fs::metadata(path).unwrap().len();
        let t = Instant::now();
        let y = ArenaForest::read_from(&mut std::io::BufReader::with_capacity(1 << 22, std::fs::File::open(path).unwrap()), bytes).unwrap();
        let rs = t.elapsed().as_secs_f64();
        assert_eq!(y.get_roots(), x.get_roots());
        println!("FOREST_BENCH snapshot bytes={bytes} bytes_per_live_leaf={:.1} write_s={ws:.1} read_and_check_s={rs:.1} \
                  heap_after_load_per_live_leaf={:.1}", bytes as f64 / leaves as f64, y.heap_bytes() as f64 / leaves as f64);
        let _ = std::fs::remove_file(path);
    }
}
