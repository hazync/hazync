//! The tip mode's PRODUCTION witness source (docs/libzync/UTREEXO_ZAPP.md §10.5.1, option B): Hazync's own full
//! forest, kept on disk between runs, that yields per block what the step guest reads: the spent coins' data
//! (step.rs `put_spent`) and the batch proof of the block's deletions (step.rs `encode_proof`).
//!
//! - The forest is [`ArenaForest`] (utreexo_forest.rs): rustreexo 0.7.0's MemForest, compact (44-byte nodes in one
//!   arena, a leaf index by 8 bytes of the leaf hash).
//! - The spent coins' data comes from the BLOCK FILES, not from a map of live coins: `fetch_block_rpc.py` writes each
//!   input's prevout (height, coinbase flag, value, script) from the node's undo data (`getblock` verbosity 3), and the
//!   creating block's hash is this source's own table of the hashes of the blocks it applied (one per height). So the
//!   source holds no UTXO set, only the forest and 32 bytes per height.
//! - Every block runs through the guest's own step.rs (`block_changes`: the adds, the deletions and every deleted leaf
//!   rebuilt from the outpoint and the prevout) and rustreexo's `Stump::modify` (the guest's check) before the forest
//!   moves. A wrong prevout (a lying node) gives a leaf that is not in the forest; a wrong inner node (a lying or
//!   corrupt store) gives a proof `Stump::modify` refuses; either stops the source before anything is proved.
//!
//! Nothing here is trusted by the proof: the guest rebuilds every leaf from the bound block and checks every proof
//! against the roots the previous receipt proves.
use crate::step;
use crate::utreexo_forest::ArenaForest;
use rustreexo::node_hash::BitcoinNodeHash;
use rustreexo::proof::Proof;
use rustreexo::stump::Stump;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};

pub type H = [u8; 32];

/// A spent coin as the block file carries it (an input's `prevout`, from the node's undo data). Untrusted.
#[derive(Clone, Debug, PartialEq)]
pub struct PrevCoin {
    pub height: u32,
    pub coinbase: bool,
    pub value: u64,
    pub spk: Vec<u8>,
}

/// One block's witness for the step guest (the frame's S and Q byte strings).
#[derive(Debug)]
pub struct Witness {
    pub hash: H,
    pub spent: Vec<u8>,
    pub proof: Vec<u8>,
}

/// Test-only lies (UTREEXO_TIP_SOURCE_LIE, execute only): the source emits them WITHOUT its native checks, so the guest
/// must refuse. `Node`: the node's prevout value of the first deletion off by one (the forest still deletes the true
/// leaf, so the source can go on). `Store`: every inner node of the forest corrupted (see `corrupt_inner_nodes`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lie {
    Node,
    Store,
}

pub struct ForestSource {
    pub height: u32,
    pub last: H,
    pub forest: ArenaForest,
    pub stump: Stump,
    /// hashes[h] = the hash of block h this source applied (0 = genesis)
    pub hashes: Vec<H>,
    pub tag: [u64; 8],
    pub counts: step::Counts,
    lie: Option<Lie>,
    lied: bool,
}

const MAGIC: &[u8; 8] = b"HZUFOR01";

impl ForestSource {
    pub fn genesis(genesis_hash: H) -> Self {
        ForestSource { height: 0, last: genesis_hash, forest: ArenaForest::new(), stump: Stump::new(), hashes: vec![genesis_hash],
                       tag: crate::utreexo_roots::tag_state(), counts: step::Counts::default(), lie: None, lied: false }
    }

    /// TEST ONLY: from now on this source lies (see [`Lie`]) and skips its native checks.
    pub fn set_lie(&mut self, l: Lie) {
        self.lie = Some(l);
        if l == Lie::Store {
            self.forest.corrupt_inner_nodes();
        }
    }

    pub fn roots(&self) -> Vec<H> {
        step::roots_of(&self.stump)
    }

    pub fn forest_agrees(&self) -> bool {
        let fr: Vec<H> = self.forest.get_roots().iter().map(step::bytes).collect();
        fr == self.roots() && self.forest.leaves == self.stump.leaves
    }

    /// Applies the next block: `stripped` = header ‖ count ‖ transactions without witness; `prevouts[i]` = the
    /// prevouts of transaction i + 1's inputs, in input order (the block file's). Returns the guest's witness for it.
    pub fn block(&mut self, stripped: &[u8], prevouts: &[Vec<PrevCoin>]) -> Result<Witness, String> {
        let h = self.height + 1;
        let e = |m: &str| format!("block {h}: {m}");
        let p = step::parse_block(stripped, &self.last).map_err(e)?;
        if prevouts.len() + 1 != p.txs.len() {
            return Err(e(&format!("{} transactions but prevouts for {}", p.txs.len(), prevouts.len())));
        }
        let mut by_outpoint: HashMap<(H, u32), &PrevCoin> = HashMap::new();
        for (t, pv) in p.txs[1..].iter().zip(prevouts) {
            if t.inputs.len() != pv.len() {
                return Err(e(&format!("a transaction with {} inputs but {} prevouts", t.inputs.len(), pv.len())));
            }
            for (i, c) in t.inputs.iter().zip(pv) {
                by_outpoint.insert(*i, c);
            }
        }
        let dels = step::deletion_outpoints(&p).map_err(e)?;
        let mut spent = Vec::new();
        let mut first_honest: Option<Vec<u8>> = None;
        for (k, op) in dels.iter().enumerate() {
            let c = by_outpoint.get(op).ok_or_else(|| e("an input without its prevout"))?;
            if c.height >= h {
                return Err(e(&format!("the prevout of {}:{} claims height {} (not below {h})", disp(&op.0), op.1, c.height)));
            }
            let bh = self.hashes[c.height as usize];
            let code = (c.height << 1) | c.coinbase as u32;
            if k == 0 && self.lie == Some(Lie::Node) && !self.lied {
                let mut honest = Vec::new();
                step::put_spent(&mut honest, &bh, code, c.value, &c.spk);
                first_honest = Some(honest);
                step::put_spent(&mut spent, &bh, code, c.value ^ 1, &c.spk);
                continue;
            }
            step::put_spent(&mut spent, &bh, code, c.value, &c.spk);
        }
        let lying = self.lie.is_some();
        // the deletions as the guest computes them: from the honest data when this block's spent bytes carry a lie
        let ch = match &first_honest {
            Some(honest) => {
                let mut s = honest.clone();
                let n0 = spent_len(&spent, 0)?;
                s.extend_from_slice(&spent[n0..]);
                self.lied = true;
                step::block_changes(&self.tag, stripped, &self.last, h, &s, &mut self.counts).map_err(e)?
            }
            None => step::block_changes(&self.tag, stripped, &self.last, h, &spent, &mut self.counts).map_err(e)?,
        };
        let proof: Proof = if ch.dels.is_empty() {
            Proof::default()
        } else {
            self.forest.prove(&ch.dels).map_err(|x| {
                e(&format!("{x}: a spent coin's leaf (rebuilt from the node's prevout data) is not a live leaf of the forest, so the node's \
                            prevouts or the forest are wrong (first deletion {}:{})", disp(&ch.del_outpoints[0].0), ch.del_outpoints[0].1))
            })?
        };
        if lying {
            // no native check: the stump follows the forest's own roots (the guest is the check)
            self.forest.modify(&ch.adds, &ch.dels).map_err(|x| e(&format!("forest modify: {x}")))?;
            let roots: Vec<BitcoinNodeHash> = self.forest.get_roots();
            self.stump = Stump { leaves: self.forest.leaves, roots };
        } else {
            self.stump = step::apply(&self.stump, &ch, &proof, &mut self.counts).map_err(e)?;
            self.forest.modify(&ch.adds, &ch.dels).map_err(|x| e(&format!("forest modify: {x}")))?;
        }
        self.height = h;
        self.last = ch.hash;
        self.hashes.push(ch.hash);
        Ok(Witness { hash: ch.hash, spent, proof: step::encode_proof(&proof) })
    }

    // ---- persistence: one file, written to <path>.tmp, synced and renamed; a sha256 of everything before it at the end

    pub fn save(&self, path: &str) -> Result<u64, String> {
        // never save a forest that is not the Stump's accumulator (review F2)
        if !self.forest_agrees() {
            return Err("forest source: the forest's roots are not the Stump's; not saving".into());
        }
        let tmp = format!("{path}.tmp");
        let f = std::fs::File::create(&tmp).map_err(|e| format!("{tmp}: {e}"))?;
        let mut w = HashingWriter { inner: BufWriter::with_capacity(1 << 22, f), sha: Sha256::new(), n: 0 };
        let io = |e: std::io::Error| format!("{tmp}: {e}");
        w.write_all(MAGIC).map_err(io)?;
        w.write_all(&self.height.to_le_bytes()).map_err(io)?;
        w.write_all(&self.last).map_err(io)?;
        w.write_all(&(self.hashes.len() as u32).to_le_bytes()).map_err(io)?;
        for x in &self.hashes {
            w.write_all(x).map_err(io)?;
        }
        w.write_all(&self.stump.leaves.to_le_bytes()).map_err(io)?;
        w.write_all(&(self.stump.roots.len() as u32).to_le_bytes()).map_err(io)?;
        for r in self.roots() {
            w.write_all(&r).map_err(io)?;
        }
        self.forest.write_to(&mut w).map_err(io)?;
        let d: [u8; 32] = w.sha.clone().finalize().into();
        let n = w.n + 32;
        let mut inner = w.inner;
        inner.write_all(&d).map_err(io)?;
        let f = inner.into_inner().map_err(|e| format!("{tmp}: {e}"))?;
        f.sync_all().map_err(|e| format!("{tmp}: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{path}: {e}"))?;
        if let Some(dir) = std::path::Path::new(path).parent() {
            if let Ok(d) = std::fs::File::open(if dir.as_os_str().is_empty() { std::path::Path::new(".") } else { dir }) {
                let _ = d.sync_all();
            }
        }
        Ok(n)
    }

    /// Reads a [`save`]d source. Refused: another format, a bad checksum, a truncated or overlong file, a forest whose
    /// inner hashes or roots are not the stored ones (ArenaForest::read_from recomputes every inner node), a hash table
    /// that does not end at the stored block.
    pub fn load(path: &str) -> Result<Self, String> {
        let f = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
        let len = f.metadata().map_err(|e| e.to_string())?.len();
        if len < 8 + 32 {
            return Err("forest source: too short".into());
        }
        let mut r = HashingReader { inner: BufReader::with_capacity(1 << 22, f).take(len - 32), sha: Sha256::new() };
        let mut m = [0u8; 8];
        r.read_exact(&mut m).map_err(|e| e.to_string())?;
        if &m != MAGIC {
            return Err("forest source: not an HZUFOR01 file".into());
        }
        let u32_ = |r: &mut HashingReader<_>| -> Result<u32, String> {
            let mut b = [0u8; 4];
            r.read_exact(&mut b).map_err(|e| e.to_string())?;
            Ok(u32::from_le_bytes(b))
        };
        let height = u32_(&mut r)?;
        let mut last = [0u8; 32];
        r.read_exact(&mut last).map_err(|e| e.to_string())?;
        let n = u32_(&mut r)?;
        // bounded by the file before anything is allocated (review F5)
        if (n as u64).saturating_mul(32) > len {
            return Err("forest source: the hash table is longer than the file".into());
        }
        if n != height + 1 {
            return Err("forest source: the hash table is not one per height".into());
        }
        let mut hashes = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let mut x = [0u8; 32];
            r.read_exact(&mut x).map_err(|e| e.to_string())?;
            hashes.push(x);
        }
        if *hashes.last().unwrap() != last {
            return Err("forest source: the hash table does not end at the stored block".into());
        }
        let mut b8 = [0u8; 8];
        r.read_exact(&mut b8).map_err(|e| e.to_string())?;
        let leaves = u64::from_le_bytes(b8);
        let nr = u32_(&mut r)? as usize;
        if nr > 64 {
            return Err("forest source: too many roots".into());
        }
        let mut roots = Vec::with_capacity(nr);
        for _ in 0..nr {
            let mut x = [0u8; 32];
            r.read_exact(&mut x).map_err(|e| e.to_string())?;
            roots.push(x);
        }
        let stump = step::stump(leaves, &roots).map_err(|e| format!("forest source: {e}"))?;
        let forest = ArenaForest::read_from(&mut r, len).map_err(|e| format!("forest source: {e}"))?;
        let mut rest = [0u8; 1];
        if r.read(&mut rest).map_err(|e| e.to_string())? != 0 {
            return Err("forest source: trailing bytes".into());
        }
        let want: [u8; 32] = r.sha.finalize().into();
        let mut got = [0u8; 32];
        r.inner.into_inner().read_exact(&mut got).map_err(|e| e.to_string())?;
        if got != want {
            return Err("forest source: the checksum does not match (a corrupt file)".into());
        }
        let s = ForestSource { height, last, forest, stump, hashes, tag: crate::utreexo_roots::tag_state(), counts: step::Counts::default(),
                               lie: None, lied: false };
        if !s.forest_agrees() {
            return Err("forest source: the forest's roots are not the stored roots".into());
        }
        Ok(s)
    }
}

/// The byte length of the first spent-coin record in `s` (block hash 32 ‖ code 4 ‖ value 8 ‖ script length 4 ‖ script).
fn spent_len(s: &[u8], at: usize) -> Result<usize, String> {
    let l = u32::from_le_bytes(s.get(at + 44..at + 48).ok_or("spent: truncated")?.try_into().unwrap()) as usize;
    Ok(at + 48 + l)
}

pub fn disp(b: &H) -> String {
    b.iter().rev().map(|x| format!("{x:02x}")).collect()
}

struct HashingWriter<W: Write> {
    inner: W,
    sha: Sha256,
    n: u64,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let k = self.inner.write(b)?;
        self.sha.update(&b[..k]);
        self.n += k as u64;
        Ok(k)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

struct HashingReader<R: Read> {
    inner: R,
    sha: Sha256,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let k = self.inner.read(b)?;
        self.sha.update(&b[..k]);
        Ok(k)
    }
}
