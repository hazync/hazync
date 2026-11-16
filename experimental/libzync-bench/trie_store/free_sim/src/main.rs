// free_sim (PREP_MEMORY.md §5): usage `free_sim <warm-up coins> <updates per flush interval, comma list> [address reuse share]`.
// Counts, per flush interval, how many slots the trie store frees and how many of those were written since the last
// flush (could be reused at once without breaking the "flushed trie intact" rule). Mirrors DiskArena's policy.
use libzync::trie_store::{Arena, Node, TrieStore};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Stats { frees: u64, fresh_frees: u64, puts: u64 }
struct Cnt { nodes: Vec<Node>, free: Vec<u64>, pending: Vec<u64>, since: Vec<u64>, st: Arc<Mutex<Stats>> }
impl Cnt {
    fn mark(&mut self, r: u64) { let i = r as usize; if self.since.len() <= i / 64 { self.since.resize(i / 64 + 1024, 0); } self.since[i / 64] |= 1 << (i % 64); }
    fn is(&self, r: u64) -> bool { let i = r as usize; self.since.get(i / 64).map_or(false, |w| w >> (i % 64) & 1 == 1) }
}
impl Arena for Cnt {
    fn get(&self, r: u64) -> Node { self.nodes[(r - 1) as usize] }
    fn put(&mut self, n: Node) -> u64 {
        self.st.lock().unwrap().puts += 1;
        let r = if let Some(r) = self.free.pop() { self.nodes[(r - 1) as usize] = n; r } else { self.nodes.push(n); self.nodes.len() as u64 };
        self.mark(r); r
    }
    fn free(&mut self, r: u64) {
        let f = self.is(r);
        { let mut s = self.st.lock().unwrap(); s.frees += 1; if f { s.fresh_frees += 1; } }
        self.pending.push(r);
    }
    fn live(&self) -> u64 { (self.nodes.len() - self.free.len() - self.pending.len()) as u64 }
    fn durable(&mut self, _: u64, _: bool, _: u64, _: &[u8; 32]) -> Result<(), String> {
        let p = std::mem::take(&mut self.pending); self.free.extend(p);
        for w in self.since.iter_mut() { *w = 0; }
        Ok(())
    }
}

struct Rng(u64);
impl Rng { fn n(&mut self) -> u64 { self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15); let mut z = self.0; z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9); z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB); z ^ (z >> 31) }
  fn b32(&mut self) -> [u8; 32] { let mut o = [0u8; 32]; for c in o.chunks_mut(8) { c.copy_from_slice(&self.n().to_le_bytes()); } o } }

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let warm: usize = a[1].parse().unwrap();           // coins inserted before measuring
    let uints: Vec<usize> = a[2].split(',').map(|x| x.parse().unwrap()).collect(); // updates per flush interval
    let reuse: f64 = a.get(3).map_or(0.5, |x| x.parse().unwrap()); // share of inserts to an existing address
    eprintln!("size_of Node = {}", std::mem::size_of::<Node>());
    let st = Arc::new(Mutex::new(Stats::default()));
    let arena = Cnt { nodes: Vec::new(), free: Vec::new(), pending: Vec::new(), since: Vec::new(), st: st.clone() };
    let mut ts = TrieStore::from_bytes_into(&TrieStore::new(true).to_bytes(), Box::new(arena)).unwrap();
    let mut r = Rng(7);
    let mut addrs: Vec<[u8; 32]> = Vec::new();
    let mut live: Vec<[u8; 64]> = Vec::new();
    let mut coins = 0u64; // inserted so far (the warm-up target)
    let ins = |r: &mut Rng, addrs: &mut Vec<[u8; 32]>, n: usize| -> Vec<([u8; 64], Option<[u32; 8]>, u64)> {
        (0..n).map(|_| {
            let s = if !addrs.is_empty() && (r.n() % 1000) as f64 / 1000.0 < reuse { addrs[(r.n() % addrs.len() as u64) as usize] } else { let s = r.b32(); addrs.push(s); s };
            let mut k = [0u8; 64]; k[..32].copy_from_slice(&s); k[32..].copy_from_slice(&r.b32());
            (k, None, 1000)
        }).collect()
    };
    // warm-up: insert-only blocks of 50k, a spend share afterwards
    while (coins as usize) < warm {
        let v = ins(&mut r, &mut addrs, 50_000);
        for x in &v { live.push(x.0); }
        coins += v.len() as u64;
        ts.apply(&[], &v).unwrap();
        ts.flush().unwrap();
    }
    println!("warm coins={coins} addrs={} nodes={}", addrs.len(), ts.nodes());
    for &u in &uints {
        for rep in 0..3 {
            *st.lock().unwrap() = Stats::default();
            // one interval = 10 blocks of u/10 updates: half spends of live coins, half new coins
            for _ in 0..10 {
                let nb = u / 10;
                let mut marks = Vec::new();
                for _ in 0..nb / 2 { let i = (r.n() % live.len() as u64) as usize; let k = live.swap_remove(i); marks.push((k, [1u32, 2, 3, 4, 5, 0, 0, 0])); }
                let v = ins(&mut r, &mut addrs, nb - nb / 2);
                for x in &v { live.push(x.0); }
                ts.apply(&marks, &v).unwrap();
            }
            ts.flush().unwrap();
            let s = st.lock().unwrap();
            println!("U_int={u} rep={rep} nodes={} addrs={} frees/update={:.2} fresh/update={:.2} held/update={:.2} held_share={:.3} log2U={:.2} log2addrs={:.2}",
                     ts.nodes(), addrs.len(), s.frees as f64 / u as f64, s.fresh_frees as f64 / u as f64, (s.frees - s.fresh_frees) as f64 / u as f64,
                     (s.frees - s.fresh_frees) as f64 / s.frees as f64, (u as f64).log2(), (addrs.len() as f64).log2());
        }
    }
}
