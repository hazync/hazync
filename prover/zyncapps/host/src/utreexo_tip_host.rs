//! The utreexo zyncAPP's TIP mode, host side (docs/libzync/UTREEXO_ZAPP.md §10): a native Floresta replay (rustreexo
//! 0.7.0 `MemForest` for the batch proofs, `Stump` for the roots, step.rs for the block rules) that yields the step
//! guest's witness: per block the spent coins' data and the batch proof of its deletions. Nothing here is trusted by the
//! proof: the guest rebuilds every leaf and `Stump::modify` verifies every proof against the roots it holds.
//!
//! ⚠ The source of deletion proofs here is a MemForest replayed from genesis (or a cache of one, `ForestCache`): fine for
//! tests and low heights, NOT for mainnet's tip (the forest holds every live leaf in memory). At the tip a Utreexo
//! bridge node (utreexod with its proof index, the source Floresta itself syncs from) gives the same per-block proofs
//! and leaf data; that source is designed, not built (UTREEXO_ZAPP.md §10.5.1).
use crate::utreexo_tip_step as step;
use crate::utreexo_host;
use rustreexo::mem_forest::MemForest;
use rustreexo::node_hash::BitcoinNodeHash;
use rustreexo::proof::Proof;
use rustreexo::stump::Stump;
use std::collections::HashMap;

pub type H = [u8; 32];

/// A live coin's data, what a later deletion of it needs (step.rs `SpentCoin`).
#[derive(Clone)]
pub struct Coin {
    pub block_hash: H,
    pub header_code: u32,
    pub value: u64,
    pub spk: Vec<u8>,
}

/// One block's witness for the step guest.
pub struct TipFrame {
    pub stripped: Vec<u8>,
    pub spent: Vec<u8>,
    pub proof: Vec<u8>,
    pub hash: H,
}

/// The native accumulator: the full forest (for proofs), the Stump Floresta keeps (for the roots), the live coins.
pub struct Native {
    pub height: u32,
    pub last: H,
    pub forest: MemForest<BitcoinNodeHash>,
    pub stump: Stump,
    pub coins: HashMap<(H, u32), Coin>,
    pub tag: [u64; 8],
    pub counts: step::Counts,
}

impl Native {
    pub fn genesis(signet: bool) -> Self {
        let g = utreexo_host::genesis(signet);
        Native { height: 0, last: step::dsha(&g), forest: MemForest::new(), stump: Stump::new(), coins: HashMap::new(),
                 tag: crate::utreexo_roots::tag_state(), counts: step::Counts::default() }
    }

    /// Applies the next block (header ‖ count ‖ transactions without witness) as Floresta does; returns the step
    /// guest's witness for it. Every deletion's proof comes from the MemForest and is checked by the same
    /// `Stump::modify` the guest runs, so a witness that would fail in the guest fails here first.
    pub fn block(&mut self, stripped: &[u8]) -> Result<TipFrame, String> {
        let h = self.height + 1;
        let p = step::parse_block(stripped, &self.last).map_err(|e| format!("block {h}: {e}"))?;
        let dels = step::deletion_outpoints(&p).map_err(|e| format!("block {h}: {e}"))?;
        let mut spent = Vec::new();
        for op in &dels {
            let c = self.coins.get(op).ok_or_else(|| format!("block {h}: spends {}:{}, not a live coin of the replay", disp(&op.0), op.1))?;
            step::put_spent(&mut spent, &c.block_hash, c.header_code, c.value, &c.spk);
        }
        let ch = step::block_changes(&self.tag, stripped, &self.last, h, &spent, &mut self.counts).map_err(|e| format!("block {h}: {e}"))?;
        let proof: Proof = if ch.dels.is_empty() { Proof::default() } else { self.forest.prove(&ch.dels).map_err(|e| format!("block {h}: MemForest prove: {e}"))? };
        self.stump = step::apply(&self.stump, &ch, &proof, &mut self.counts).map_err(|e| format!("block {h}: {e}"))?;
        self.forest.modify(&ch.adds, &ch.dels).map_err(|e| format!("block {h}: MemForest modify: {e}"))?;
        for op in &dels {
            self.coins.remove(op);
        }
        for (ti, v) in step::add_outputs(&p) {
            let t = &p.txs[ti];
            let (value, spk) = t.outputs[v as usize];
            self.coins.insert((t.txid, v), Coin { block_hash: p.hash, header_code: (h << 1) | (ti == 0) as u32, value, spk: spk.to_vec() });
        }
        self.height = h;
        self.last = p.hash;
        Ok(TipFrame { stripped: stripped.to_vec(), spent, proof: step::encode_proof(&proof), hash: p.hash })
    }

    pub fn roots(&self) -> Vec<H> {
        step::roots_of(&self.stump)
    }

    /// The MemForest's own roots must be the Stump's (a cross-check of the two rustreexo structures).
    pub fn forest_agrees(&self) -> bool {
        let fr: Vec<H> = self.forest.get_roots().iter().map(|n| step::bytes(&n.get_data())).collect();
        fr == self.roots() && self.forest.leaves == self.stump.leaves
    }

    // ---- a cache of the replay, so a follower does not replay from genesis every run (test scale: see the module doc)

    pub fn save(&self, path: &str) -> Result<(), String> {
        let mut b = Vec::new();
        b.extend_from_slice(b"HZUTIP01");
        b.extend_from_slice(&self.height.to_le_bytes());
        b.extend_from_slice(&self.last);
        let mut coins: Vec<_> = self.coins.iter().collect();
        coins.sort_by(|a, b| a.0.cmp(b.0));
        b.extend_from_slice(&(coins.len() as u64).to_le_bytes());
        for ((t, v), c) in coins {
            b.extend_from_slice(t);
            b.extend_from_slice(&v.to_le_bytes());
            step::put_spent(&mut b, &c.block_hash, c.header_code, c.value, &c.spk);
        }
        b.extend_from_slice(&self.stump.leaves.to_le_bytes());
        b.extend_from_slice(&(self.stump.roots.len() as u32).to_le_bytes());
        for r in self.roots() {
            b.extend_from_slice(&r);
        }
        self.forest.serialize(&mut b).map_err(|e| e.to_string())?;
        let tmp = format!("{path}.tmp");
        std::fs::write(&tmp, &b).map_err(|e| format!("{tmp}: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{path}: {e}"))
    }

    pub fn load(path: &str) -> Result<Self, String> {
        let b = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let mut o = 0usize;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let r = b.get(o..o + n).ok_or("forest cache: truncated")?;
            o += n;
            Ok(r)
        };
        if take(8)? != b"HZUTIP01" { return Err("forest cache: not an HZUTIP01 file".into()); }
        let height = u32::from_le_bytes(take(4)?.try_into().unwrap());
        let last: H = take(32)?.try_into().unwrap();
        let n = u64::from_le_bytes(take(8)?.try_into().unwrap());
        let mut coins = HashMap::new();
        for _ in 0..n {
            let t: H = take(32)?.try_into().unwrap();
            let v = u32::from_le_bytes(take(4)?.try_into().unwrap());
            let block_hash: H = take(32)?.try_into().unwrap();
            let header_code = u32::from_le_bytes(take(4)?.try_into().unwrap());
            let value = u64::from_le_bytes(take(8)?.try_into().unwrap());
            let l = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
            let spk = take(l)?.to_vec();
            coins.insert((t, v), Coin { block_hash, header_code, value, spk });
        }
        let leaves = u64::from_le_bytes(take(8)?.try_into().unwrap());
        let nr = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let mut roots = Vec::with_capacity(nr);
        for _ in 0..nr {
            roots.push(take(32)?.try_into().unwrap());
        }
        let stump = step::stump(leaves, &roots).map_err(|e| format!("forest cache: {e}"))?;
        let forest = MemForest::deserialize(&b[o..]).map_err(|e| format!("forest cache: {e}"))?;
        Ok(Native { height, last, forest, stump, coins, tag: crate::utreexo_roots::tag_state(), counts: step::Counts::default() })
    }
}

pub fn disp(b: &H) -> String {
    b.iter().rev().map(|x| format!("{x:02x}")).collect()
}

/// The frames as the guest reads them: per block [L, S, Q] ‖ the three byte strings, each zero-padded to words.
pub fn frame_words(fr: &[TipFrame]) -> Vec<u32> {
    let mut w = Vec::new();
    let pad = |w: &mut Vec<u32>, b: &[u8]| {
        for c in b.chunks(4) {
            let mut x = [0u8; 4];
            x[..c.len()].copy_from_slice(c);
            w.push(u32::from_le_bytes(x));
        }
    };
    for f in fr {
        w.extend_from_slice(&[f.stripped.len() as u32, f.spent.len() as u32, f.proof.len() as u32]);
        pad(&mut w, &f.stripped);
        pad(&mut w, &f.spent);
        pad(&mut w, &f.proof);
    }
    w
}

/// Execute-only negative tests (UTREEXO_TIP_TAMPER): each must be refused by the guest.
pub fn tamper(case: &str, fr: &mut [TipFrame], link: &mut Vec<[u8; 80]>) -> Result<String, String> {
    let first_del = fr.iter().position(|f| !f.spent.is_empty()).ok_or("no block with a deletion in this step")?;
    match case {
        "spent_value" => { fr[first_del].spent[36] ^= 1; Ok(format!("frame {first_del}: the first spent coin's value +-1")) }
        "spent_block" => { fr[first_del].spent[0] ^= 1; Ok(format!("frame {first_del}: the first spent coin's creating block hash")) }
        "spent_code" => { fr[first_del].spent[32] ^= 2; Ok(format!("frame {first_del}: the first spent coin's height")) }
        "spent_coinbase" => { fr[first_del].spent[32] ^= 1; Ok(format!("frame {first_del}: the first spent coin's coinbase flag")) }
        "spent_drop" => { fr[first_del].spent.clear(); Ok(format!("frame {first_del}: its spent coins dropped")) }
        "spent_extra" => { let e = fr[first_del].spent.clone(); fr[first_del].spent.extend_from_slice(&e); Ok(format!("frame {first_del}: its spent coins twice")) }
        "proof_hash" => {
            let i = fr.iter().position(|f| step::decode_proof(&f.proof).map(|p| !p.hashes.is_empty()).unwrap_or(false)).ok_or("no proof with hashes in this step")?;
            let n = fr[i].proof.len();
            fr[i].proof[n - 1] ^= 1;
            Ok(format!("frame {i}: one byte of the last proof hash"))
        }
        "proof_target" => { fr[first_del].proof[4] ^= 1; Ok(format!("frame {first_del}: the first target's position")) }
        "proof_empty" => { fr[first_del].proof = step::encode_proof(&Proof::default()); Ok(format!("frame {first_del}: an empty proof")) }
        "tx_swap" => {
            // swap the last two transactions of the first block with three or more (the coinbase stays first)
            let k = fr.iter().position(|f| tx_count(&f.stripped) >= 3).ok_or("no block with 3 transactions")?;
            swap_last_two(&mut fr[k].stripped)?;
            Ok(format!("frame {k}: its last two transactions swapped"))
        }
        "link" => {
            let l = link.first_mut().ok_or("no link headers (H = T)")?;
            l[0] ^= 1;
            Ok("the first link header's version".into())
        }
        "link_last" => {
            // the LAST link header: the chain of prev fields still holds (no header follows it), so only the final
            // check (the link ends at the core journal's tip_hash) can refuse it
            let l = link.last_mut().ok_or("no link headers (H = T)")?;
            l[76] ^= 1;
            Ok("the last link header's nonce".into())
        }
        "link_drop" => {
            link.pop().ok_or("no link headers (H = T): nothing to drop")?;
            Ok("the last link header dropped".into())
        }
        "first_prev" => {
            fr.first_mut().ok_or("no blocks")?.stripped[4] ^= 1;
            Ok("the first block's prev-block field (it must follow the previous statement's block P)".into())
        }
        "prev_roots" => Ok("one byte of the previous statement's journal (its receipt does not match it)".into()),
        "prev_kind" => Ok("the previous statement given as the other kind (base vs step)".into()),
        "core_id" => Ok("another core image id (the core receipt does not verify under it)".into()),
        _ => Err(format!("unknown UTREEXO_TIP_TAMPER case {case}")),
    }
}

fn tx_count(stripped: &[u8]) -> u64 {
    match stripped[80] { 0xfd => u16::from_le_bytes([stripped[81], stripped[82]]) as u64, x => x as u64 }
}

/// Re-serialises a block with its last two transactions swapped (the Merkle root then differs).
fn swap_last_two(stripped: &mut Vec<u8>) -> Result<(), String> {
    let p = step::parse_block(stripped, &stripped[4..36].try_into().unwrap()).map_err(|e| e.to_string())?;
    let n = p.txs.len();
    // the offsets of each transaction: re-walk with the parser's own lengths
    let mut offs = Vec::new();
    let mut o = 80 + if stripped[80] == 0xfd { 3 } else { 1 };
    for _ in 0..n {
        let s = o;
        o = s + tx_len(&stripped[s..])?;
        offs.push((s, o));
    }
    let (a, b) = (offs[n - 2], offs[n - 1]);
    let mut out = stripped[..a.0].to_vec();
    out.extend_from_slice(&stripped[b.0..b.1]);
    out.extend_from_slice(&stripped[a.0..a.1]);
    out.extend_from_slice(&stripped[b.1..]);
    *stripped = out;
    Ok(())
}

fn tx_len(b: &[u8]) -> Result<usize, String> {
    fn vi(b: &[u8], o: &mut usize) -> u64 {
        let f = b[*o];
        *o += 1;
        match f {
            0xfd => { let x = u16::from_le_bytes([b[*o], b[*o + 1]]) as u64; *o += 2; x }
            0xfe => { let x = u32::from_le_bytes(b[*o..*o + 4].try_into().unwrap()) as u64; *o += 4; x }
            0xff => { let x = u64::from_le_bytes(b[*o..*o + 8].try_into().unwrap()); *o += 8; x }
            x => x as u64,
        }
    }
    let mut o = 4;
    for _ in 0..vi(b, &mut o) { o += 36; let l = vi(b, &mut o) as usize; o += l + 4; }
    for _ in 0..vi(b, &mut o) { o += 8; let l = vi(b, &mut o) as usize; o += l; }
    Ok(o + 4)
}
