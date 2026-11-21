//! The Utreexo roots zyncAPP's derivation (docs/libzync/UTREEXO_ZAPP.md), shared by its guest (#[path]-included, like
//! coinlog.rs and facts.rs) and the host, which runs the same code natively before proving.
//!
//! [`Deriver`] walks blocks 1..=T (each header and its transactions without witnesses). For each block up to H it
//! rebuilds every coin-log leaf from the outputs and the host's claimed spender records, as the hints zyncAPP does
//! (HINTS_ZAPP.md §3), and feeds Floresta's Utreexo accumulator the same outputs in the same order: a coin unspent at H
//! as its Utreexo leaf, a coin spent at or below H as an Empty leaf (§2 of the doc: rustreexo's deletion is exactly
//! that). The caller then compares the coin log at H and the block forest with the core proof's, as hints does: if they
//! match, every spent/unspent claim was the chain's, so the Utreexo roots are the ones Floresta would hold after H.
//!
//! The walk is resumable: [`Deriver::encode`] / [`Deriver::decode`] carry its whole state between runs, and a run's
//! statement commits SHA-256 of the encoded state it starts from and ends at.
//!
//! Not guest-compiled by the CORE, nor by the hints zyncAPP: editing this file moves the utreexo zyncAPP's id only.

use crate::coinlog::{self, H};
use crate::facts;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use sha2::digest::generic_array::GenericArray;
use sha2::{Digest, Sha256};

/// The statement's kind word ('ZU', version 1).
pub const KIND_UTREEXO: u32 = 0x5A55_0001;

/// Floresta's `UTREEXO_TAG_V1` (floresta-chain src/pruned_utreexo/consensus.rs): SHA-512("UtreexoV1"), 64 bytes. A leaf
/// hashes it twice, so the first 128-byte SHA-512 block of every leaf is the same and its state is computed once.
pub const UTREEXO_TAG_V1: [u8; 64] = [
    0x5b, 0x83, 0x2d, 0xb8, 0xca, 0x26, 0xc2, 0x5b, 0xe1, 0xc5, 0x42, 0xd6, 0xcc, 0xed, 0xdd, 0xa8, 0xc1, 0x45, 0x61, 0x5c, 0xff, 0x5c,
    0x35, 0x72, 0x7f, 0xb3, 0x46, 0x26, 0x10, 0x80, 0x7e, 0x20, 0xae, 0x53, 0x4d, 0xc3, 0xf6, 0x42, 0x99, 0x19, 0x99, 0x31, 0x77, 0x2e,
    0x03, 0x78, 0x7d, 0x18, 0x15, 0x6e, 0xb3, 0x15, 0x1e, 0x0e, 0xd1, 0xb3, 0x09, 0x8b, 0xdc, 0x84, 0x45, 0x86, 0x18, 0x85,
];

/// Mainnet's two BIP30-overwritten coinbases (height, block hash in internal byte order). The coin log skips their
/// outputs; Utreexo adds them as leaves that can never be deleted (Floresta's UNSPENDABLE_BIP30_UTXO_91722 / _91812).
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

// ---------------------------------------------------------------------------------------------------------------
// SHA-512/256 (FIPS 180-4 §5.3.6.2), by blocks, so the tag block's state is reused. rustreexo and Floresta use
// bitcoin_hashes' sha512_256, the same function.

const IV_512_256: [u64; 8] = [
    0x22312194FC2BF72C, 0x9F555FA3C84C64C2, 0x2393B86B6F53B151, 0x963877195940EABD,
    0x96283EE2A88EFFE3, 0xBE5E1E2553863992, 0x2B0199FC2C85B8AA, 0x0EB72DDC81C52CA2,
];

fn compress(state: &mut [u64; 8], blocks: &[u8]) {
    debug_assert!(blocks.len() % 128 == 0);
    for b in blocks.chunks_exact(128) {
        sha2::compress512(state, core::slice::from_ref(GenericArray::from_slice(b)));
    }
}

/// Finishes a SHA-512/256 whose first `done` bytes (a multiple of 128) are already in `state`.
fn finish512(mut state: [u64; 8], done: u64, tail: &[u8]) -> H {
    let total = done + tail.len() as u64;
    let mut buf = Vec::with_capacity(tail.len() + 145);
    buf.extend_from_slice(tail);
    buf.push(0x80);
    while buf.len() % 128 != 112 {
        buf.push(0);
    }
    buf.extend_from_slice(&((total as u128) * 8).to_be_bytes());
    compress(&mut state, &buf);
    let mut o = [0u8; 32];
    for i in 0..4 {
        o[8 * i..8 * i + 8].copy_from_slice(&state[i].to_be_bytes());
    }
    o
}

/// The SHA-512 state after the leaf's first block, UTREEXO_TAG_V1 ‖ UTREEXO_TAG_V1.
pub fn tag_state() -> [u64; 8] {
    let mut s = IV_512_256;
    let mut b = [0u8; 128];
    b[..64].copy_from_slice(&UTREEXO_TAG_V1);
    b[64..].copy_from_slice(&UTREEXO_TAG_V1);
    compress(&mut s, &b);
    s
}

fn put_compact(out: &mut Vec<u8>, v: u64) {
    match v {
        0..=0xfc => out.push(v as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend_from_slice(&(v as u16).to_le_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0xfe);
            out.extend_from_slice(&(v as u32).to_le_bytes());
        }
        _ => {
            out.push(0xff);
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

/// Floresta's leaf hash (floresta-chain src/pruned_utreexo/udata.rs `get_leaf_hashes`):
/// SHA-512/256(TAG ‖ TAG ‖ block hash ‖ txid ‖ vout u32 LE ‖ header code u32 LE ‖ TxOut), header code = height·2 +
/// coinbase, TxOut = value u64 LE ‖ CompactSize(len) ‖ scriptPubKey. Hashes and txid in internal byte order.
#[allow(clippy::too_many_arguments)]
pub fn utreexo_leaf(tag: &[u64; 8], block_hash: &H, txid: &H, vout: u32, height: u32, coinbase: bool, value: u64, spk: &[u8]) -> H {
    let mut m = Vec::with_capacity(81 + 9 + spk.len());
    m.extend_from_slice(block_hash);
    m.extend_from_slice(txid);
    m.extend_from_slice(&vout.to_le_bytes());
    m.extend_from_slice(&((height << 1) | coinbase as u32).to_le_bytes());
    m.extend_from_slice(&value.to_le_bytes());
    put_compact(&mut m, spk.len() as u64);
    m.extend_from_slice(spk);
    finish512(*tag, 128, &m)
}

/// The SHA-512 blocks a leaf costs after the cached tag block: 81 fixed bytes, the CompactSize, the script, 17 of padding.
pub fn leaf_blocks(spk_len: usize) -> u64 {
    let cs = if spk_len < 0xfd { 1 } else { 3 };
    (81 + cs + spk_len + 17).div_ceil(128) as u64
}

/// rustreexo's `parent_hash`: SHA-512/256(left ‖ right).
pub fn utreexo_parent(l: &H, r: &H) -> H {
    let mut b = [0u8; 64];
    b[..32].copy_from_slice(l);
    b[32..].copy_from_slice(r);
    finish512(IV_512_256, 0, &b)
}

/// A Utreexo forest under rustreexo's rules (src/stump/mod.rs `add`, src/proof/mod.rs `calculate_hashes_delete`): a
/// node is a hash or Empty; Empty is the identity of the parent rule; the older subtree is the left child.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct UForest {
    /// index = tree level; `Some(node)` exactly when bit k of `leaves` is set; a node `None` is Empty.
    pub levels: Vec<Option<Option<H>>>,
    pub leaves: u64,
}

impl UForest {
    /// Adds one leaf (`None` = Empty); returns how many parent hashes it computed.
    pub fn add(&mut self, leaf: Option<H>) -> u32 {
        let mut node = leaf;
        let mut k = 0;
        let mut n = 0;
        loop {
            if k == self.levels.len() {
                self.levels.push(None);
            }
            match self.levels[k].take() {
                Some(root) => {
                    node = match (root, node) {
                        (None, x) => x,
                        (x, None) => x,
                        (Some(a), Some(b)) => {
                            n += 1;
                            Some(utreexo_parent(&a, &b))
                        }
                    };
                    k += 1;
                }
                None => {
                    self.levels[k] = Some(node);
                    break;
                }
            }
        }
        self.leaves += 1;
        n
    }

    /// The roots as rustreexo's `Stump.roots` lists them: the largest tree first, one per set bit of `leaves`; an Empty
    /// root as 32 zero bytes (rustreexo's own test encoding of Empty).
    pub fn roots(&self) -> Vec<H> {
        self.levels.iter().rev().flatten().map(|n| n.unwrap_or([0u8; 32])).collect()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The walk (HINTS_ZAPP.md §3, the same rules; kept apart from hints.rs so neither image moves the other; the compact
// sizes are read by facts.rs's strict reader, which both images include).

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

/// Core's IsUnspendable (Floresta's `Consensus::is_unspendable`): OP_RETURN first, or a script over 10,000 bytes.
pub fn unspendable_script(spk: &[u8]) -> bool {
    spk.first() == Some(&0x6a) || spk.len() > 10_000
}

/// The median of up to the last 11 block times (Core's GetMedianTimePast: sort, take element n / 2).
pub fn median_time(times: &[u32]) -> u32 {
    let mut t: Vec<u32> = times[times.len().saturating_sub(11)..].to_vec();
    t.sort_unstable();
    t[t.len() / 2]
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

/// What [`Deriver::finish`] returns: the statement's fields and the forests to compare with the core proof.
pub struct Derived {
    pub block_hash: H,
    pub utreexo: UForest,
    pub n_live: u64,
    pub utxo_roots: Vec<Option<H>>,
    pub utxo_leaves: u64,
    pub block_roots: Vec<Option<H>>,
    pub block_leaves: u64,
}

/// Profiling counters of one run (not part of the state; zero after [`Deriver::decode`]).
#[derive(Clone, Default, Debug)]
pub struct Prof {
    /// a cycle counter (the guest passes `env::cycle_count`); None: no timing
    pub clock: Option<fn() -> u64>,
    /// cycles spent in Utreexo leaf hashing and the forest add (only with a clock)
    pub utreexo_cycles: u64,
    pub leaf_hashes: u64,
    /// SHA-512 compressions in those leaf hashes, the tag block excluded (one per 128 bytes of the rest, padded)
    pub leaf_blocks: u64,
    pub parent_hashes: u64,
    pub empty_adds: u64,
    pub log_leaves: u64,
    pub outputs: u64,
    pub inputs: u64,
    pub txs: u64,
    pub bytes: u64,
}

/// The walk's whole state (everything [`Deriver::encode`] carries from one run to the next).
#[derive(Clone, Debug)]
pub struct Deriver {
    stop: u32,
    bip30: bool,
    h: u32,
    last_hash: H,
    times: Vec<u32>,
    log: Vec<Option<H>>,
    log_n: u64,
    value: u64,
    blk: Vec<Option<H>>,
    blk_n: u64,
    utx: UForest,
    n_live: u64,
    stop_hash: H,
    tag: [u64; 8],
    pub prof: Prof,
}

impl Deriver {
    /// From genesis (block 0 has the 0x02 leaf over the empty coin state, HANDOVER.md §3; the genesis coinbase is
    /// neither a coin-log leaf nor a Utreexo leaf). `bip30`: mainnet's overwritten coinbases ([`MAINNET_BIP30`]).
    pub fn new(stop: u32, genesis: &[u8; 80], bip30: bool) -> Result<Self, &'static str> {
        if stop == 0 {
            return Err("H must be at least 1");
        }
        let g = dsha(genesis);
        let mut blk = Vec::new();
        push_forest(&mut blk, facts::block_leaf(&g, 0, &facts::utxo_state(0, 0, &[])), facts::block_parent);
        Ok(Deriver {
            stop, bip30, h: 0, last_hash: g,
            times: alloc::vec![u32::from_le_bytes(genesis[68..72].try_into().unwrap())],
            log: Vec::new(), log_n: 0, value: 0, blk, blk_n: 1, utx: UForest::default(), n_live: 0, stop_hash: [0; 32],
            tag: tag_state(), prof: Prof::default(),
        })
    }

    pub fn height(&self) -> u32 {
        self.h
    }

    /// One Utreexo add: a live coin's leaf (block hash, txid, vout, height, coinbase, value, script) or Empty.
    #[allow(clippy::type_complexity)]
    fn uadd(&mut self, c: Option<(&H, &H, u32, u32, bool, u64, &[u8])>) {
        let t0 = self.prof.clock.map(|f| f());
        if let Some((.., spk)) = &c {
            self.prof.leaf_blocks += leaf_blocks(spk.len());
        }
        let leaf = c.map(|(bh, txid, v, h, cb, value, spk)| utreexo_leaf(&self.tag, bh, txid, v, h, cb, value, spk));
        if leaf.is_some() {
            self.n_live += 1;
            self.prof.leaf_hashes += 1;
        } else {
            self.prof.empty_adds += 1;
        }
        self.prof.parent_hashes += self.utx.add(leaf) as u64;
        if let (Some(t0), Some(f)) = (t0, self.prof.clock) {
            self.prof.utreexo_cycles += f() - t0;
        }
    }

    pub fn stop(&self) -> u32 {
        self.stop
    }

    /// Block `h` = the next one, at or below H. `block` = 80-byte header ‖ CompactSize count ‖ every transaction
    /// without witness; `tracked` = the positions of its transactions whose outputs carry flag bit 1 (strictly
    /// increasing); `status` = for each output that enters the coin log, in order, `0` (unspent at H) or
    /// `height, tx, vin` (its spender, at or below H); `state`, `undo` = the coin state after the block and its undo
    /// hash, as the block's leaf commits them. At H, `state` must equal the state this walk computes.
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
        let overwritten = self.bip30 && MAINNET_BIP30.iter().any(|(bh, bx)| *bh == h && *bx == hash);
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
        // pass 2: the coin-log leaves and the Utreexo leaves, in block order
        let mut si = 0usize;
        let mut tk = 0usize;
        for (ti, t) in txs.iter().enumerate() {
            let is_tracked = tk < tracked.len() && tracked[tk] as usize == ti;
            if is_tracked {
                tk += 1;
            }
            for (v, (value, spk)) in t.outputs.iter().enumerate() {
                if unspendable_script(spk) || spent_here[first[ti] + v] {
                    continue; // neither a coin-log leaf nor a Utreexo add (get_block_adds)
                }
                if ti == 0 && overwritten {
                    // a Utreexo leaf the coin log does not have; Floresta refuses to ever delete it
                    self.uadd(Some((&hash, &t.txid, v as u32, h, true, *value, spk)));
                    continue;
                }
                let coin = facts::coin_leaf(&t.txid, v as u32, *value, spk, h, ti == 0, is_tracked, mtp);
                let w = *status.get(si).ok_or("status: too short")?;
                si += 1;
                let leaf = if w == 0 {
                    self.value = self.value.checked_add(*value).ok_or("value overflow")?;
                    self.uadd(Some((&hash, &t.txid, v as u32, h, ti == 0, *value, spk)));
                    coin
                } else {
                    let (tx, vin) = (*status.get(si).ok_or("status: too short")?, *status.get(si + 1).ok_or("status: too short")?);
                    si += 2;
                    if w <= h || w > self.stop {
                        return Err("status: a spend outside (creation, H]");
                    }
                    self.uadd(None);
                    coinlog::spent(&coin, &coinlog::spender(w, tx, vin))
                };
                push_forest(&mut self.log, leaf, coinlog::parent);
                self.log_n += 1;
                self.prof.log_leaves += 1;
            }
        }
        self.prof.txs += txs.len() as u64;
        self.prof.outputs += nout as u64;
        self.prof.inputs += txs.iter().map(|t| t.inputs.len() as u64).sum::<u64>();
        self.prof.bytes += block.len() as u64;
        if si != status.len() {
            return Err("status: too long");
        }
        self.times.push(u32::from_le_bytes(header[68..72].try_into().unwrap()));
        if self.times.len() > 11 {
            self.times.remove(0);
        }
        let leaf_state = if h == self.stop {
            let own = facts::utxo_state(self.log_n, self.value, &self.log);
            if own != *state {
                return Err("the coin state at H is not the one the claimed spends imply");
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

    /// The number of block-forest leaves so far (one per height from genesis, then the tail).
    pub fn block_leaves(&self) -> u64 {
        self.blk_n
    }

    pub fn finish(self) -> Result<Derived, &'static str> {
        if self.h != self.stop {
            return Err("fewer blocks than H");
        }
        Ok(Derived {
            block_hash: self.stop_hash,
            utreexo: self.utx,
            n_live: self.n_live,
            utxo_roots: trimmed(&self.log),
            utxo_leaves: self.log_n,
            block_roots: trimmed(&self.blk),
            block_leaves: self.blk_n,
        })
    }

    /// The state as bytes: what one run ends at and the next starts from. Fixed field order, every length explicit.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        let opt = |b: &mut Vec<u8>, x: &Option<H>| match x {
            None => b.push(0),
            Some(h) => {
                b.push(1);
                b.extend_from_slice(h);
            }
        };
        b.extend_from_slice(&KIND_UTREEXO.to_le_bytes());
        b.extend_from_slice(&self.stop.to_le_bytes());
        b.push(self.bip30 as u8);
        b.extend_from_slice(&self.h.to_le_bytes());
        b.extend_from_slice(&self.last_hash);
        b.push(self.times.len() as u8);
        for t in &self.times {
            b.extend_from_slice(&t.to_le_bytes());
        }
        b.push(self.log.len() as u8);
        for x in &self.log {
            opt(&mut b, x);
        }
        b.extend_from_slice(&self.log_n.to_le_bytes());
        b.extend_from_slice(&self.value.to_le_bytes());
        b.push(self.blk.len() as u8);
        for x in &self.blk {
            opt(&mut b, x);
        }
        b.extend_from_slice(&self.blk_n.to_le_bytes());
        b.push(self.utx.levels.len() as u8);
        for x in &self.utx.levels {
            match x {
                None => b.push(0),
                Some(None) => b.push(2),
                Some(Some(h)) => {
                    b.push(1);
                    b.extend_from_slice(h);
                }
            }
        }
        b.extend_from_slice(&self.utx.leaves.to_le_bytes());
        b.extend_from_slice(&self.n_live.to_le_bytes());
        b.extend_from_slice(&self.stop_hash);
        b
    }

    /// The inverse of [`Deriver::encode`]; refuses anything `encode` would not have written.
    pub fn decode(b: &[u8]) -> Result<Self, &'static str> {
        struct Rd<'a> { b: &'a [u8], o: usize }
        impl<'a> Rd<'a> {
            fn n(&mut self, n: usize) -> Result<&'a [u8], &'static str> { take(self.b, &mut self.o, n) }
            fn u8_(&mut self) -> Result<u8, &'static str> { Ok(self.n(1)?[0]) }
            fn u32_(&mut self) -> Result<u32, &'static str> { Ok(u32::from_le_bytes(self.n(4)?.try_into().unwrap())) }
            fn u64_(&mut self) -> Result<u64, &'static str> { Ok(u64::from_le_bytes(self.n(8)?.try_into().unwrap())) }
            fn h(&mut self) -> Result<H, &'static str> { Ok(self.n(32)?.try_into().unwrap()) }
            fn forest(&mut self) -> Result<Vec<Option<H>>, &'static str> {
                let n = self.u8_()? as usize;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(match self.u8_()? { 0 => None, 1 => Some(self.h()?), _ => return Err("state: forest node tag") });
                }
                Ok(v)
            }
        }
        let mut r = Rd { b, o: 0 };
        if r.u32_()? != KIND_UTREEXO {
            return Err("state: not a utreexo walk state");
        }
        let stop = r.u32_()?;
        let bip30 = match r.u8_()? { 0 => false, 1 => true, _ => return Err("state: bip30 flag") };
        let h = r.u32_()?;
        let last_hash = r.h()?;
        let nt = r.u8_()? as usize;
        if nt == 0 || nt > 11 {
            return Err("state: times");
        }
        let mut times = Vec::with_capacity(nt);
        for _ in 0..nt {
            times.push(r.u32_()?);
        }
        let log = r.forest()?;
        let log_n = r.u64_()?;
        let value = r.u64_()?;
        let blk = r.forest()?;
        let blk_n = r.u64_()?;
        let nl = r.u8_()? as usize;
        let mut levels = Vec::with_capacity(nl);
        for _ in 0..nl {
            levels.push(match r.u8_()? { 0 => None, 2 => Some(None), 1 => Some(Some(r.h()?)), _ => return Err("state: utreexo node tag") });
        }
        let leaves = r.u64_()?;
        let n_live = r.u64_()?;
        let stop_hash = r.h()?;
        if r.o != b.len() {
            return Err("state: trailing bytes");
        }
        let d = Deriver {
            stop, bip30, h, last_hash, times, log, log_n, value, blk, blk_n, utx: UForest { levels, leaves }, n_live, stop_hash,
            tag: tag_state(), prof: Prof::default(),
        };
        if d.encode() != b {
            return Err("state: not canonical");
        }
        Ok(d)
    }

    /// SHA-256 of [`Deriver::encode`]: the digest a run's statement commits for the state it starts from or ends at.
    pub fn digest(&self) -> H {
        Sha256::digest(self.encode()).into()
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
    }

    fn hx(s: &str) -> H {
        let v: Vec<u8> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect();
        v.try_into().unwrap()
    }

    /// rustreexo's own vector (src/node_hash/mod.rs `parent_hash` doc test): parent([0; 32], [1; 32]).
    #[test]
    fn parent_vector() {
        assert_eq!(utreexo_parent(&[0; 32], &[1; 32]), hx("34e33ca0c40b7bd33d28932ca9e35170def7309a3bf91ecda5e1ceb067548a12"));
    }

    /// The tag block's state, then the same leaf by one-shot SHA-512/256.
    #[test]
    fn leaf_by_blocks_equals_one_shot() {
        let spk = [0x51u8; 300];
        for n in [0usize, 1, 25, 34, 47, 48, 100, 300] {
            let a = utreexo_leaf(&tag_state(), &[7; 32], &[9; 32], 3, 170, false, 5000, &spk[..n]);
            let mut m = Vec::new();
            m.extend_from_slice(&UTREEXO_TAG_V1);
            m.extend_from_slice(&UTREEXO_TAG_V1);
            m.extend_from_slice(&[7; 32]);
            m.extend_from_slice(&[9; 32]);
            m.extend_from_slice(&3u32.to_le_bytes());
            m.extend_from_slice(&340u32.to_le_bytes());
            m.extend_from_slice(&5000u64.to_le_bytes());
            put_compact(&mut m, n as u64);
            m.extend_from_slice(&spk[..n]);
            let b: H = sha2::Sha512_256::digest(&m).into();
            assert_eq!(a, b, "script length {n}");
        }
    }

    /// The add rule on a small forest with an Empty leaf, and an Empty root kept in the list.
    #[test]
    fn forest_shape_rules() {
        let mut f = UForest::default();
        for i in 0..7u8 {
            f.add(if i == 2 { None } else { Some([i; 32]) });
        }
        // 7 = 4 + 2 + 1: (0,1 | -,3) , (4,5), (6)
        let p01 = utreexo_parent(&[0; 32], &[1; 32]);
        assert_eq!(f.roots(), alloc::vec![utreexo_parent(&p01, &[3; 32]), utreexo_parent(&[4; 32], &[5; 32]), [6; 32]]);
        let mut e = UForest::default();
        e.add(None);
        e.add(None);
        e.add(Some([1; 32]));
        assert_eq!(e.roots(), alloc::vec![[0; 32], [1; 32]], "an Empty root is listed, as rustreexo keeps it");
    }

    #[test]
    fn state_round_trip() {
        let g: [u8; 80] = [0; 80];
        let mut d = Deriver::new(5, &g, true).unwrap();
        d.utx.add(Some([3; 32]));
        d.utx.add(None);
        d.utx.add(None);
        let e = d.encode();
        assert_eq!(Deriver::decode(&e).unwrap().encode(), e);
        let mut bad = e.clone();
        bad.push(0);
        assert!(Deriver::decode(&bad).is_err());
    }
}
