//! The SwiftSync hints zyncAPP's derivation (docs/libzync/HINTS_ZAPP.md), shared by its guest (#[path]-included,
//! like coinlog.rs and facts.rs) and the host, which runs the same code natively before proving.
//!
//! [`Deriver`] walks blocks 1..=H (each header and its transactions without witnesses), rebuilds every coin-log leaf
//! from the block's outputs and the host's claimed spender records, folds the coin log forest and the block forest,
//! and streams the hints file (hintsfile.md, file version 0x00) into SHA-256. The caller then compares the forests with
//! the core proof's: the coin state at H opens block H's leaf, and the block forest equals the core's `block_roots`.
//! Nothing the host gives is trusted: a wrong spender, tracked bit, state or transaction list breaks a root.
//!
//! Not guest-compiled by the CORE: editing this file moves the hints zyncAPP's id only.

use crate::coinlog::{self, H};
use crate::facts;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use sha2::{Digest, Sha256};

/// hintsfile.md file version 0x00, the bit order of its test vectors (`test_vectors/hintsfile/elias_fano.json`): each
/// byte filled from its most significant bit, each low value least significant bit first.
pub const FORMAT_BIP: u32 = 1;
/// hintsfile.md file version 0x00, the bit order of the `hintsfile` crate 0.2.0 (each byte filled from bit 0).
pub const FORMAT_CRATE: u32 = 2;
pub const MAGIC: [u8; 4] = [0x55, 0x54, 0x58, 0x4f];
pub const FILE_VERSION: u8 = 0x00;
/// The statement's kind word ('ZH', version 1).
pub const KIND_HINTS: u32 = 0x5A48_0001;

/// Mainnet's two BIP30-overwritten coinbases (height, block hash in internal byte order): their outputs get no index.
pub const MAINNET_BIP30: [(u32, H); 2] = [
    (91722, rev(*b"\x00\x00\x00\x00\x00\x02\x71\xa2\xdc\x26\xe7\x66\x7f\x84\x19\xf2\xe1\x54\x16\xdc\x69\x55\xe5\xa6\xc6\xcd\xf3\xf2\x57\x4d\xd0\x8e")),
    (91812, rev(*b"\x00\x00\x00\x00\x00\x0a\xf0\xae\xd4\x79\x2b\x1a\xce\xe3\xd9\x66\xaf\x36\xcf\x5d\xef\x14\x93\x5d\xb8\xde\x83\xd6\xf9\x30\x6f\x2f")),
];

const fn rev(mut b: H) -> H {
    let mut i = 0;
    while i < 16 {
        let t = b[i];
        b[i] = b[31 - i];
        b[31 - i] = t;
        i += 1;
    }
    b
}

fn dsha(b: &[u8]) -> H {
    Sha256::digest(Sha256::digest(b)).into()
}

struct Bits {
    out: Vec<u8>,
    cur: u8,
    k: u8,
    msb: bool,
}

impl Bits {
    fn new(msb: bool) -> Self {
        Bits { out: Vec::new(), cur: 0, k: 0, msb }
    }
    fn push(&mut self, bit: bool) {
        if bit {
            self.cur |= if self.msb { 0x80 >> self.k } else { 1 << self.k };
        }
        self.k += 1;
        if self.k == 8 {
            self.out.push(self.cur);
            self.cur = 0;
            self.k = 0;
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.k > 0 {
            self.out.push(self.cur);
        }
        self.out
    }
}

fn put_compact(out: &mut Vec<u8>, v: u32) {
    match v {
        0..=0xfc => out.push(v as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend_from_slice(&(v as u16).to_le_bytes());
        }
        _ => {
            out.push(0xfe);
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

/// The number of low bits: floor(log2((m + 1) / n)).
fn low_bits(m: u32, n: u32) -> u32 {
    ((m as u64 + 1) / n as u64).ilog2()
}

/// One block's Elias-Fano list (hintsfile.md): `CompactSize n`, then for n > 0 `CompactSize m`, the low bits, the
/// high bits in unary. `idx` strictly increasing. The canonical encoding: minimal CompactSize, zero padding bits.
pub fn ef_encode(idx: &[u32], format: u32, out: &mut Vec<u8>) {
    debug_assert!(idx.windows(2).all(|w| w[0] < w[1]));
    if idx.is_empty() {
        out.push(0);
        return;
    }
    let msb = format == FORMAT_BIP;
    let n = idx.len() as u32;
    let m = *idx.last().unwrap();
    let l = low_bits(m, n);
    put_compact(out, n);
    put_compact(out, m);
    let mut low = Bits::new(msb);
    for &x in idx {
        for s in 0..l {
            low.push((x >> s) & 1 == 1);
        }
    }
    let mut high = Bits::new(msb);
    let mut prev = 0u32;
    for &x in idx {
        let hx = x >> l;
        for _ in prev..hx {
            high.push(false);
        }
        high.push(true);
        prev = hx;
    }
    out.extend_from_slice(&low.finish());
    out.extend_from_slice(&high.finish());
}

fn get_compact(b: &[u8], o: &mut usize) -> Option<u32> {
    let f = *b.get(*o)?;
    *o += 1;
    let (v, w) = match f {
        0xfd => (u16::from_le_bytes(b.get(*o..*o + 2)?.try_into().ok()?) as u32, 2),
        0xfe => (u32::from_le_bytes(b.get(*o..*o + 4)?.try_into().ok()?), 4),
        0xff => return None,
        x => (x as u32, 0),
    };
    *o += w;
    // minimal only (the zAPP writes nothing else)
    let min_ok = match w { 2 => v >= 0xfd, 4 => v > 0xffff, _ => true };
    min_ok.then_some(v)
}

/// Decodes one block's list at `*o` (the verifier's and the tests' reader). Strict: refuses a list that is not the
/// canonical encoding of its indices (non-minimal sizes, nonzero padding, a last element other than m).
pub fn ef_decode(b: &[u8], o: &mut usize, format: u32) -> Option<Vec<u32>> {
    let start = *o;
    let n = get_compact(b, o)?;
    if n == 0 {
        return Some(Vec::new());
    }
    let m = get_compact(b, o)?;
    if m == u32::MAX || n as u64 > m as u64 + 1 {
        return None;
    }
    let l = low_bits(m, n);
    // sizes in u64 and bounded by the bytes left BEFORE anything is reserved or indexed (a hostile n or m must not
    // reserve memory or overflow a 32-bit usize: review H1)
    let left = b.len().checked_sub(*o)? as u64;
    let lo_bytes = (n as u64 * l as u64).div_ceil(8);
    let hi_bytes = (n as u64 + (m >> l) as u64).div_ceil(8);
    if lo_bytes.checked_add(hi_bytes)? > left {
        return None;
    }
    let (lo_bytes, hi_bytes) = (lo_bytes as usize, hi_bytes as usize);
    let lo = b.get(*o..*o + lo_bytes)?;
    let hi = b.get(*o + lo_bytes..*o + lo_bytes + hi_bytes)?;
    let msb = format == FORMAT_BIP;
    let bit = |s: &[u8], i: usize| -> bool { if msb { s[i / 8] >> (7 - i % 8) & 1 == 1 } else { s[i / 8] >> (i % 8) & 1 == 1 } };
    let mut out = Vec::with_capacity(n as usize); // n <= 8 * hi_bytes <= the bytes left: bounded above
    let (mut hp, mut hb) = (0u32, 0usize);
    for k in 0..n as usize {
        let mut lv = 0u32;
        for s in 0..l as usize {
            if bit(lo, k * l as usize + s) {
                lv |= 1 << s;
            }
        }
        loop {
            if hb >= hi_bytes * 8 {
                return None;
            }
            let x = bit(hi, hb);
            hb += 1;
            if x {
                break;
            }
            hp += 1;
        }
        out.push((hp << l) | lv);
    }
    *o += lo_bytes + hi_bytes;
    // canonical: re-encoding gives these exact bytes
    let mut re = Vec::new();
    ef_encode(&out, format, &mut re);
    (out.windows(2).all(|w| w[0] < w[1]) && *out.last().unwrap() == m && re[..] == b[start..*o]).then_some(out)
}

/// Decodes a whole hints file: (stop height, the list of every block 1..=H). Strict as [`ef_decode`].
pub fn decode_file(b: &[u8], format: u32) -> Option<(u32, Vec<Vec<u32>>)> {
    if b.len() < 9 || b[..4] != MAGIC || b[4] != FILE_VERSION {
        return None;
    }
    let stop = u32::from_le_bytes(b[5..9].try_into().ok()?);
    let mut o = 9;
    // every block's list is at least one byte: a header naming more blocks than bytes left is refused before any
    // reservation (a 9-byte file claiming H = 0xFFFFFFFF reserved ~103 GB and aborted: review H1)
    if stop as u64 > (b.len() - 9) as u64 {
        return None;
    }
    let mut v = Vec::with_capacity(stop as usize);
    for _ in 0..stop {
        v.push(ef_decode(b, &mut o, format)?);
    }
    (o == b.len()).then_some((stop, v))
}

fn push_forest(roots: &mut Vec<Option<H>>, mut node: H, parent: fn(u32, &H, &H) -> H) {
    let mut k = 0usize;
    loop {
        if k == roots.len() {
            roots.push(None);
        }
        match roots[k].take() {
            Some(r) => {
                node = parent(k as u32 + 1, &r, &node);
                k += 1;
            }
            None => {
                roots[k] = Some(node);
                return;
            }
        }
    }
}

/// A forest's roots with trailing empty levels removed (ChainState's form).
pub fn trimmed(r: &[Option<H>]) -> Vec<Option<H>> {
    let mut v = r.to_vec();
    while v.last() == Some(&None) {
        v.pop();
    }
    v
}

/// SHA-256(0x05 ‖ hash ‖ height ‖ state ‖ undo): the block leaf with the undo-data commitment (UNDO.md §3).
pub fn block_leaf_undo(hash: &H, height: u32, state: &H, undo: &H) -> H {
    let mut h = Sha256::new();
    h.update([0x05u8]);
    h.update(hash);
    h.update(height.to_le_bytes());
    h.update(state);
    h.update(undo);
    h.finalize().into()
}

/// A compact size as Core reads it (facts::varint: the shortest form, at most MAX_SIZE).
fn varint(b: &[u8], o: &mut usize) -> Result<u64, &'static str> {
    facts::varint(b, o).ok_or("a compact size cut short or not as Core reads it (shortest form, at most MAX_SIZE)")
}

/// A compact size that counts bytes still to come (facts::len_varint: also within the bytes left, as a usize).
fn len_varint(b: &[u8], o: &mut usize) -> Result<usize, &'static str> {
    facts::len_varint(b, o).ok_or("a length cut short, not as Core reads it, or past the end")
}

fn take<'a>(b: &'a [u8], o: &mut usize, n: usize) -> Result<&'a [u8], &'static str> {
    let e = o.checked_add(n).ok_or("truncated")?;
    let r = b.get(*o..e).ok_or("truncated")?;
    *o = e;
    Ok(r)
}

struct Tx<'a> {
    inputs: Vec<(&'a [u8], u32)>,
    outputs: Vec<(u64, &'a [u8])>,
    txid: H,
}

/// One transaction in its serialisation WITHOUT witness (what its txid hashes). A witness marker is refused.
fn parse_tx<'a>(b: &'a [u8], o: &mut usize) -> Result<Tx<'a>, &'static str> {
    let s0 = *o;
    take(b, o, 4)?;
    let nin = varint(b, o)?;
    if nin == 0 || nin > 1 << 20 {
        return Err("a transaction with no inputs (or a witness marker): give transactions without witnesses");
    }
    let mut inputs = Vec::with_capacity(nin as usize);
    for _ in 0..nin {
        let t = take(b, o, 32)?;
        let v = u32::from_le_bytes(take(b, o, 4)?.try_into().unwrap());
        let l = len_varint(b, o)?;
        take(b, o, l)?;
        take(b, o, 4)?;
        inputs.push((t, v));
    }
    let nout = varint(b, o)?;
    if nout > 1 << 20 {
        return Err("too many outputs");
    }
    let mut outputs = Vec::with_capacity(nout as usize);
    for _ in 0..nout {
        let v = u64::from_le_bytes(take(b, o, 8)?.try_into().unwrap());
        let l = len_varint(b, o)?;
        outputs.push((v, take(b, o, l)?));
    }
    take(b, o, 4)?;
    Ok(Tx { inputs, outputs, txid: dsha(&b[s0..*o]) })
}

/// Core's ComputeMerkleRoot with its mutation flag (CVE-2012-2459): refuses a list with an equal adjacent pair.
fn merkle_root(mut level: Vec<H>) -> Result<H, &'static str> {
    while level.len() > 1 {
        let mut i = 0;
        while i + 1 < level.len() {
            if level[i] == level[i + 1] {
                return Err("mutated transaction list (duplicate pair, CVE-2012-2459)");
            }
            i += 2;
        }
        if level.len() % 2 == 1 {
            level.push(*level.last().unwrap());
        }
        level = level
            .chunks_exact(2)
            .map(|c| {
                let mut p = [0u8; 64];
                p[..32].copy_from_slice(&c[0]);
                p[32..].copy_from_slice(&c[1]);
                dsha(&p)
            })
            .collect();
    }
    Ok(level[0])
}

/// Core's IsUnspendable plus the hintsfile's rule: OP_RETURN first, or a script over 10,000 bytes.
pub fn unspendable_script(spk: &[u8]) -> bool {
    spk.first() == Some(&0x6a) || spk.len() > 10_000
}

/// The median of up to the last 11 block times (Core's GetMedianTimePast: sort, take element n / 2).
pub fn median_time(times: &[u32]) -> u32 {
    let mut t: Vec<u32> = times[times.len().saturating_sub(11)..].to_vec();
    t.sort_unstable();
    t[t.len() / 2]
}

/// What [`Deriver::finish`] returns: the statement's derived fields and the forests to compare with the core proof.
pub struct Derived {
    pub hints_sha256: H,
    pub hints_len: u64,
    pub n_counted: u64,
    pub n_unspent: u64,
    pub unspent_value: u64,
    pub undo_sha256: H,
    pub block_hash: H,
    pub utxo_roots: Vec<Option<H>>,
    pub utxo_leaves: u64,
    pub block_roots: Vec<Option<H>>,
    pub block_leaves: u64,
}

pub struct Deriver {
    format: u32,
    stop: u32,
    bip30: &'static [(u32, H)],
    h: u32,
    last_hash: H,
    times: Vec<u32>,
    log: Vec<Option<H>>,
    log_n: u64,
    value: u64,
    blk: Vec<Option<H>>,
    blk_n: u64,
    file: Sha256,
    file_len: u64,
    undo: Sha256,
    n_counted: u64,
    n_unspent: u64,
    stop_hash: H,
    /// with `keep`: the file's bytes (the host writes them; the guest only hashes)
    pub bytes: Option<Vec<u8>>,
}

impl Deriver {
    /// From genesis (block 0 has the 0x02 leaf over the empty coin state, HANDOVER.md §3). `bip30`: the network's
    /// overwritten coinbases ([`MAINNET_BIP30`], none on signet).
    pub fn new(format: u32, stop: u32, genesis: &[u8; 80], bip30: &'static [(u32, H)], keep: bool) -> Result<Self, &'static str> {
        if format != FORMAT_BIP && format != FORMAT_CRATE {
            return Err("unknown hints format");
        }
        if stop == 0 {
            return Err("H must be at least 1");
        }
        let g = dsha(genesis);
        let mut blk = Vec::new();
        push_forest(&mut blk, facts::block_leaf(&g, 0, &facts::utxo_state(0, 0, &[])), facts::block_parent);
        let mut head = Vec::with_capacity(9);
        head.extend_from_slice(&MAGIC);
        head.push(FILE_VERSION);
        head.extend_from_slice(&stop.to_le_bytes());
        let mut file = Sha256::new();
        file.update(&head);
        Ok(Deriver {
            format, stop, bip30, h: 0, last_hash: g,
            times: alloc::vec![u32::from_le_bytes(genesis[68..72].try_into().unwrap())],
            log: Vec::new(), log_n: 0, value: 0, blk, blk_n: 1, file, file_len: 9, undo: Sha256::new(),
            n_counted: 0, n_unspent: 0, stop_hash: [0; 32], bytes: keep.then_some(head),
        })
    }

    /// Block `h` = the next one: `block` = 80-byte header ‖ CompactSize count ‖ every transaction without witness;
    /// `tracked` = the positions of its transactions whose outputs carry flag bit 1 (strictly increasing);
    /// `status` = for each output that enters the coin log, in order, `0` (unspent at H) or `height, tx, vin` (its
    /// spender, at or below H); `state`, `undo` = the coin state after the block and its undo hash, as the block's leaf
    /// commits them. At H, `state` must equal the state this walk computes.
    pub fn block(&mut self, block: &[u8], tracked: &[u32], status: &[u32], state: &H, undo: &H) -> Result<(), &'static str> {
        let h = self.h + 1;
        if h > self.stop {
            return Err("a block above H");
        }
        let header: &[u8; 80] = block.get(..80).ok_or("truncated header")?.try_into().unwrap();
        if header[4..36] != self.last_hash {
            return Err("header does not follow the previous block");
        }
        let hash = dsha(header);
        let mut o = 80;
        let ntx = varint(block, &mut o)?;
        if ntx == 0 || ntx > 1 << 20 {
            return Err("bad transaction count");
        }
        let mut txs = Vec::with_capacity(ntx as usize);
        for _ in 0..ntx {
            txs.push(parse_tx(block, &mut o)?);
        }
        if o != block.len() {
            return Err("trailing bytes after the transactions");
        }
        // the coinbase fixes the tree's depth: an inner node (64 bytes of hashes) cannot stand in for it
        let cb = &txs[0];
        if cb.inputs.len() != 1 || cb.inputs[0].0 != [0u8; 32] || cb.inputs[0].1 != u32::MAX {
            return Err("transaction 0 is not a coinbase");
        }
        if txs[1..].iter().any(|t| t.inputs.iter().any(|i| i.0 == [0u8; 32] && i.1 == u32::MAX)) {
            return Err("a second coinbase");
        }
        if merkle_root(txs.iter().map(|t| t.txid).collect())? != header[36..68] {
            return Err("transactions do not hash to the header's Merkle root");
        }
        if !tracked.windows(2).all(|w| w[0] < w[1]) || tracked.last().is_some_and(|&t| t as u64 >= ntx) {
            return Err("tracked positions not increasing within the block");
        }
        let overwritten = self.bip30.iter().any(|(bh, bx)| *bh == h && *bx == hash);
        let mtp = median_time(&self.times);
        // pass 1: which outputs are spent in this block (inputs only spend outputs of EARLIER transactions)
        let mut made: BTreeMap<(H, u32), usize> = BTreeMap::new();
        let mut first = Vec::with_capacity(txs.len());
        let mut nout = 0usize;
        for t in &txs {
            first.push(nout);
            nout += t.outputs.len();
        }
        let mut spent_here = alloc::vec![false; nout];
        for (ti, t) in txs.iter().enumerate() {
            if ti > 0 {
                for (pt, pv) in &t.inputs {
                    let k: H = (*pt).try_into().unwrap();
                    if let Some(&ord) = made.get(&(k, *pv)) {
                        if spent_here[ord] {
                            return Err("an output spent twice in one block");
                        }
                        spent_here[ord] = true;
                    }
                }
            }
            for v in 0..t.outputs.len() {
                made.insert((t.txid, v as u32), first[ti] + v);
            }
        }
        // pass 2: indices, leaves, hints
        let mut list: Vec<u32> = Vec::new();
        let mut idx = 0u32;
        let mut si = 0usize;
        let mut tk = 0usize;
        for (ti, t) in txs.iter().enumerate() {
            let is_tracked = tk < tracked.len() && tracked[tk] as usize == ti;
            if is_tracked {
                tk += 1;
            }
            if ti == 0 && overwritten {
                continue;
            }
            for (v, (value, spk)) in t.outputs.iter().enumerate() {
                if unspendable_script(spk) {
                    continue;
                }
                let i = idx;
                idx = idx.checked_add(1).ok_or("too many outputs")?;
                if spent_here[first[ti] + v] {
                    continue;
                }
                let coin = facts::coin_leaf(&t.txid, v as u32, *value, spk, h, ti == 0, is_tracked, mtp);
                let w = *status.get(si).ok_or("status: too short")?;
                si += 1;
                let leaf = if w == 0 {
                    list.push(i);
                    self.n_unspent += 1;
                    self.value = self.value.checked_add(*value).ok_or("value overflow")?;
                    coin
                } else {
                    let (tx, vin) = (*status.get(si).ok_or("status: too short")?, *status.get(si + 1).ok_or("status: too short")?);
                    si += 2;
                    if w <= h || w > self.stop {
                        return Err("status: a spend outside (creation, H]");
                    }
                    coinlog::spent(&coin, &coinlog::spender(w, tx, vin))
                };
                push_forest(&mut self.log, leaf, coinlog::parent);
                self.log_n += 1;
            }
        }
        if si != status.len() {
            return Err("status: too long");
        }
        self.n_counted += idx as u64;
        let mut enc = Vec::new();
        ef_encode(&list, self.format, &mut enc);
        self.file.update(&enc);
        self.file_len += enc.len() as u64;
        if let Some(b) = &mut self.bytes {
            b.extend_from_slice(&enc);
        }
        self.undo.update(undo);
        self.times.push(u32::from_le_bytes(header[68..72].try_into().unwrap()));
        if self.times.len() > 11 {
            self.times.remove(0);
        }
        let leaf_state = if h == self.stop {
            let own = facts::utxo_state(self.log_n, self.value, &self.log);
            if own != *state {
                return Err("the coin state at H is not the one the hints imply");
            }
            self.stop_hash = hash;
            own
        } else {
            *state
        };
        push_forest(&mut self.blk, block_leaf_undo(&hash, h, &leaf_state, undo), facts::block_parent);
        self.blk_n += 1;
        self.last_hash = hash;
        self.h = h;
        Ok(())
    }

    /// A block-forest leaf above H (the core tip may be higher than H).
    pub fn tail_leaf(&mut self, leaf: &H) -> Result<(), &'static str> {
        if self.h != self.stop {
            return Err("tail leaves before H");
        }
        push_forest(&mut self.blk, *leaf, facts::block_parent);
        self.blk_n += 1;
        Ok(())
    }

    pub fn height(&self) -> u32 {
        self.h
    }

    pub fn finish(self) -> Result<Derived, &'static str> {
        if self.h != self.stop {
            return Err("fewer blocks than H");
        }
        Ok(Derived {
            hints_sha256: self.file.finalize().into(),
            hints_len: self.file_len,
            n_counted: self.n_counted,
            n_unspent: self.n_unspent,
            unspent_value: self.value,
            undo_sha256: self.undo.finalize().into(),
            block_hash: self.stop_hash,
            utxo_roots: trimmed(&self.log),
            utxo_leaves: self.log_n,
            block_roots: trimmed(&self.blk),
            block_leaves: self.blk_n,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compact sizes as Core reads them (facts::varint and facts::len_varint; review 21.11.2026): block 170's payment with
    /// a non-canonical count or length, a length past the end, and a size above MAX_SIZE, each refused by name.
    #[test]
    fn compact_sizes_are_cores() {
        let hexs = "0100000001c997a5e56e104102fa209c6a852dd90660a20b2d9c352423edce25857fcd3704000000004847304402204e45e16932b8af514961a1d3a1a25fdf3f4f7732e9d624c6c61548ab5fb8cd410220181522ec8eca07de4860a4acdd12909d831cc56cbbac4622082221a8768d1d0901ffffffff0200ca9a3b00000000434104ae1a62fe09c5f51b13905f07f06b99a2f7159b2225f374cd378d71302fa28414e7aab37397f554a7df5f142c21c1b7303b8a0626f1baded5c72a704f7e6cd84cac00286bee0000000043410411db93e1dcdb8a016b49840f8c53bc1eb68a382e97b1482ecad7b148a6909a5cb2e0eaddfb84ccf9744464f82e160bfa9b8b64f9d4c03f999b8643f656b412a3ac00000000";
        let raw: Vec<u8> = (0..hexs.len()).step_by(2).map(|i| u8::from_str_radix(&hexs[i..i + 2], 16).unwrap()).collect();
        let mut o = 0;
        assert!(parse_tx(&raw, &mut o).is_ok());
        assert_eq!(o, raw.len());
        let with = |at: usize, rep: &[u8]| { let mut v = raw[..at].to_vec(); v.extend_from_slice(rep); v.extend_from_slice(&raw[at + 1..]); v };
        const C: &str = "a compact size cut short or not as Core reads it (shortest form, at most MAX_SIZE)";
        const L: &str = "a length cut short, not as Core reads it, or past the end";
        assert_eq!(raw[4], 0x01);
        assert_eq!(parse_tx(&with(4, &[0xfd, 0x01, 0x00]), &mut 0).err(), Some(C), "a non-canonical input count");
        assert_eq!(raw[41], 0x48);
        assert_eq!(parse_tx(&with(41, &[0xfd, 0x48, 0x00]), &mut 0).err(), Some(L), "a non-canonical scriptSig length");
        let nout = 4 + 1 + 32 + 4 + 1 + 0x48 + 4;
        assert_eq!(raw[nout], 0x02);
        assert_eq!(parse_tx(&with(nout, &[0xfd, 0x02, 0x00]), &mut 0).err(), Some(C), "a non-canonical output count");
        let last = raw.len() - 4 - 0x43 - 1;
        assert_eq!(raw[last], 0x43);
        assert_eq!(parse_tx(&with(last, &[0x48]), &mut 0).err(), Some(L), "a scriptPubKey length past the end");
        assert_eq!(parse_tx(&with(last, &[0xfe, 0x00, 0x00, 0x00, 0x02]), &mut 0).err(), Some(L), "MAX_SIZE, past the end");
        assert_eq!(parse_tx(&with(last, &[0xfe, 0x01, 0x00, 0x00, 0x02]), &mut 0).err(), Some(L), "above MAX_SIZE");
        assert_eq!(parse_tx(&with(last, &[0xff, 0x43, 0, 0, 0, 1, 0, 0, 0]), &mut 0).err(), Some(L), "2^32 + 0x43: as usize on wasm32 read 0x43");
    // MAX_SIZE itself, on a count (a length past MAX_SIZE is also past the end, so only a count can show it): an input
    // count of MAX_SIZE + 1 in its shortest form is Core's "size too large", not this parser's own input cap
    assert_eq!(parse_tx(&with(4, &[0xfe, 0x01, 0x00, 0x00, 0x02]), &mut 0).err(), Some(C), "an input count above MAX_SIZE");
    assert_eq!(parse_tx(&with(nout, &[0xfe, 0x01, 0x00, 0x00, 0x02]), &mut 0).err(), Some(C), "an output count above MAX_SIZE");
    }
    use alloc::vec;

    fn hx(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// hintsfile.md's test vectors (2140-dev/swiftsync-bips test_vectors/hintsfile/elias_fano.json, 2026-11-14).
    #[test]
    fn bip_vectors() {
        let v: [(Vec<u32>, &str); 3] = [
            (vec![13, 16, 19, 22, 25, 28, 31, 34, 37, 40], "0a288d8d8016ad50"),
            ((0..17).map(|i| 5 + 7 * i).collect(), "11758d8d8d8d804a4949292524"),
            ((0..50).map(|i| 17 + 3 * i).collect(), "32a4aaaaaaaaaaaa800094a5294a5294a5294a5294a5294a5290"),
        ];
        for (idx, want) in v.iter() {
            let mut b = Vec::new();
            ef_encode(idx, FORMAT_BIP, &mut b);
            assert_eq!(b, hx(want));
            let mut o = 0;
            assert_eq!(ef_decode(&b, &mut o, FORMAT_BIP).as_ref(), Some(idx));
        }
    }

    /// The crate 0.2.0's own test (`lsb_first_bit_order`): [3, 7, 12] -> 03 0c 0f 25.
    #[test]
    fn crate_order() {
        let mut b = Vec::new();
        ef_encode(&[3, 7, 12], FORMAT_CRATE, &mut b);
        assert_eq!(b, vec![0x03, 0x0c, 0x0f, 0x25]);
        let mut c = Vec::new();
        ef_encode(&[3, 7, 12], FORMAT_BIP, &mut c);
        assert_ne!(b, c, "the two bit orders differ");
    }

    #[test]
    fn decode_strict() {
        let mut b = Vec::new();
        ef_encode(&[0, 5, 9, 1000, 70000], FORMAT_BIP, &mut b);
        let mut o = 0;
        assert!(ef_decode(&b, &mut o, FORMAT_BIP).is_some() && o == b.len());
        let mut bad = b.clone();
        *bad.last_mut().unwrap() ^= 1; // a padding or data bit
        let mut o = 0;
        assert!(ef_decode(&bad, &mut o, FORMAT_BIP).map(|v| v != vec![0, 5, 9, 1000, 70000]).unwrap_or(true));
        // a non-minimal CompactSize for n
        let mut nm = vec![0xfd, 5, 0];
        nm.extend_from_slice(&b[1..]);
        let mut o = 0;
        assert!(ef_decode(&nm, &mut o, FORMAT_BIP).is_none());
        let mut o = 0;
        assert_eq!(ef_decode(&[0], &mut o, FORMAT_BIP), Some(vec![]));
    }

    /// Review H1: hostile headers and sizes are refused without reserving memory.
    #[test]
    fn hostile_sizes() {
        let mut f = vec![0x55, 0x54, 0x58, 0x4f, 0x00];
        f.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_file(&f, FORMAT_BIP).is_none(), "a 9-byte file claiming H = 2^32 - 1");
        // n = m + 1 = 2^32 - 1 (as large as allowed) with nothing after it
        let mut l = vec![0xfe];
        l.extend_from_slice(&(u32::MAX - 1).to_le_bytes());
        l.push(0xfe);
        l.extend_from_slice(&(u32::MAX - 2).to_le_bytes());
        let mut o = 0;
        assert!(ef_decode(&l, &mut o, FORMAT_BIP).is_none());
        let mut l = vec![0xfe];
        l.extend_from_slice(&1000u32.to_le_bytes());
        l.push(0xfe);
        l.extend_from_slice(&(u32::MAX - 1).to_le_bytes());
        let mut o = 0;
        assert!(ef_decode(&l, &mut o, FORMAT_BIP).is_none(), "m near 2^32: sizes past the input");
    }

    #[test]
    fn bip30_hashes() {
        let d = |b: &H| -> alloc::string::String { b.iter().rev().map(|x| alloc::format!("{x:02x}")).collect() };
        assert_eq!(d(&MAINNET_BIP30[0].1), "00000000000271a2dc26e7667f8419f2e15416dc6955e5a6c6cdf3f2574dd08e");
        assert_eq!(d(&MAINNET_BIP30[1].1), "00000000000af0aed4792b1acee3d966af36cf5def14935db8de83d6f9306f2f");
    }
}
