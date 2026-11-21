//! The utreexo zyncAPP's TIP mode (docs/libzync/UTREEXO_ZAPP.md §10): one step moves Floresta's accumulator from the
//! roots after block P (proven by the previous receipt) to the roots after block H > P, block by block, exactly as a
//! Floresta node does: `Stump::modify(adds, dels, proof)` of rustreexo 0.7.0, the crate Floresta pins, with the adds of
//! floresta-chain's `get_block_adds` and the deletions of its `process_proof` (inputs that do not spend an output of the
//! same block), each deleted leaf rebuilt here from the spending input's outpoint and the spent coin's data, and opened
//! against the current roots by the batch proof (an untrusted host gives both; a wrong leaf or proof does not open).
//!
//! Shared by the tip guest (`main.rs`) and the host (`zyncapp`, #[path]-included), so the host runs the same code
//! natively before proving. The includer provides `crate::utreexo_roots` (libzync/src/utreexo_roots.rs: the leaf hash)
//! and `crate::facts` (libzync/src/facts.rs: the compact-size reader).
//! Not part of any other image: editing this file moves the utreexo TIP image only (never the base image 27bea7da,
//! never the core).

use crate::utreexo_roots as ur;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use rustreexo::node_hash::BitcoinNodeHash;
use rustreexo::proof::Proof;
use rustreexo::stump::Stump;
use sha2::{Digest, Sha256};

pub type H = [u8; 32];

/// The tip statement's kind word ('ZU', version 2). Version 1 is the base statement (a walk from genesis).
pub const KIND_UTREEXO_TIP: u32 = 0x5A55_0002;

/// Floresta's UNSPENDABLE_BIP30_UTXO_91722 / _91812 (floresta-chain src/pruned_utreexo/consensus.rs): the leaves of
/// the two BIP30-overwritten coinbases. Floresta's `update_acc` refuses a block that deletes either.
pub const UNSPENDABLE_BIP30: [H; 2] = [
    [0x84, 0xb3, 0xaf, 0x07, 0x83, 0xb4, 0x10, 0xb4, 0x56, 0x4c, 0x5d, 0x1f, 0x36, 0x18, 0x68, 0x55, 0x9f, 0x7c, 0xf7, 0x7c,
     0xfc, 0x65, 0xce, 0x2b, 0xe9, 0x51, 0x21, 0x03, 0x57, 0x02, 0x2f, 0xe3],
    [0xbc, 0x6b, 0x4b, 0xf7, 0xce, 0xbb, 0xd3, 0x3a, 0x18, 0xd6, 0xb0, 0xfe, 0x1f, 0x8e, 0xcc, 0x7a, 0xa5, 0x40, 0x30, 0x83,
     0xc3, 0x9e, 0xe3, 0x43, 0xb9, 0x85, 0xd5, 0x1f, 0xd0, 0x29, 0x5a, 0xd8],
];

pub fn dsha(b: &[u8]) -> H {
    Sha256::digest(Sha256::digest(b)).into()
}

/// A statement root (32 bytes; 32 zero bytes = Empty, the base statement's encoding) as rustreexo's node.
pub fn node(r: &H) -> BitcoinNodeHash {
    if *r == [0u8; 32] { BitcoinNodeHash::Empty } else { BitcoinNodeHash::Some(*r) }
}

/// rustreexo's node as a statement root (Empty as 32 zero bytes). A Placeholder never occurs in a Stump.
pub fn bytes(n: &BitcoinNodeHash) -> H {
    match n {
        BitcoinNodeHash::Some(h) => *h,
        _ => [0u8; 32],
    }
}

pub fn stump(leaves: u64, roots: &[H]) -> Result<Stump, &'static str> {
    if roots.len() as u32 != leaves.count_ones() {
        return Err("roots: not one per set bit of the leaf count");
    }
    Ok(Stump { leaves, roots: roots.iter().map(node).collect() })
}

pub fn roots_of(s: &Stump) -> Vec<H> {
    s.roots.iter().map(bytes).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Block parsing (the same rules as utreexo_roots.rs's walk, copied so neither image moves the other; the compact
// sizes are read by libzync/src/facts.rs's strict reader, which every zyncAPP image includes).

/// A compact size as Core reads it (facts::varint: the shortest form, at most MAX_SIZE).
fn varint(b: &[u8], o: &mut usize) -> Result<u64, &'static str> {
    crate::facts::varint(b, o).ok_or("a compact size cut short or not as Core reads it (shortest form, at most MAX_SIZE)")
}

/// A compact size that counts bytes still to come (facts::len_varint: also within the bytes left, as a usize).
fn len_varint(b: &[u8], o: &mut usize) -> Result<usize, &'static str> {
    crate::facts::len_varint(b, o).ok_or("a length cut short, not as Core reads it, or past the end")
}

fn take<'a>(b: &'a [u8], o: &mut usize, n: usize) -> Result<&'a [u8], &'static str> {
    let e = o.checked_add(n).ok_or("truncated")?;
    let r = b.get(*o..e).ok_or("truncated")?;
    *o = e;
    Ok(r)
}

pub struct Tx<'a> {
    pub inputs: Vec<(H, u32)>,
    pub outputs: Vec<(u64, &'a [u8])>,
    pub txid: H,
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
        let t: H = take(b, o, 32)?.try_into().unwrap();
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

/// A parsed block bound to its header: `block` = 80-byte header ‖ CompactSize count ‖ every transaction without
/// witness. Checks: the header follows `prev`, the transactions hash to its Merkle root (mutation refused), transaction
/// 0 is the only coinbase (it fixes the tree's depth).
pub struct Parsed<'a> {
    pub hash: H,
    pub txs: Vec<Tx<'a>>,
}

pub fn parse_block<'a>(block: &'a [u8], prev: &H) -> Result<Parsed<'a>, &'static str> {
    let header: &[u8; 80] = block.get(..80).ok_or("truncated header")?.try_into().unwrap();
    if header[4..36] != *prev {
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
    Ok(Parsed { hash, txs })
}

/// The spent coin's data a deletion needs besides the input's outpoint (Floresta's CompactLeafData plus the creating
/// block's hash, which Floresta looks up in its own header chain by height): block hash ‖ header code u32 LE (height·2
/// + coinbase) ‖ value u64 LE ‖ script length u32 LE ‖ script. Untrusted: the leaf rebuilt from it must open in the
/// accumulator.
pub struct SpentCoin<'a> {
    pub block_hash: H,
    pub header_code: u32,
    pub value: u64,
    pub spk: &'a [u8],
}

pub fn put_spent(out: &mut Vec<u8>, block_hash: &H, header_code: u32, value: u64, spk: &[u8]) {
    out.extend_from_slice(block_hash);
    out.extend_from_slice(&header_code.to_le_bytes());
    out.extend_from_slice(&value.to_le_bytes());
    out.extend_from_slice(&(spk.len() as u32).to_le_bytes());
    out.extend_from_slice(spk);
}

fn read_spent<'a>(b: &'a [u8], o: &mut usize) -> Result<SpentCoin<'a>, &'static str> {
    let block_hash: H = take(b, o, 32)?.try_into().unwrap();
    let header_code = u32::from_le_bytes(take(b, o, 4)?.try_into().unwrap());
    let value = u64::from_le_bytes(take(b, o, 8)?.try_into().unwrap());
    let n = u32::from_le_bytes(take(b, o, 4)?.try_into().unwrap()) as usize;
    if n > 4_000_000 {
        return Err("spent coin: script too long");
    }
    Ok(SpentCoin { block_hash, header_code, value, spk: take(b, o, n)? })
}

/// Profile counters of a step (not part of any statement).
#[derive(Clone, Default, Debug)]
pub struct Counts {
    pub blocks: u64,
    pub txs: u64,
    pub adds: u64,
    pub dels: u64,
    pub proof_hashes: u64,
    /// SHA-512 compressions of the adds' and deletions' leaf hashes (the tag block excluded)
    pub leaf_blocks: u64,
    pub bytes: u64,
}

/// floresta-chain `get_block_adds`: (transaction index, vout) of every output that is not unspendable
/// (`Consensus::is_unspendable`) and whose outpoint no input of the block spends (`get_input_outpoints`: EVERY input,
/// the coinbase's null outpoint included), in block order.
pub fn add_outputs(p: &Parsed) -> Vec<(usize, u32)> {
    let mut inputs: BTreeSet<(H, u32)> = BTreeSet::new();
    for t in &p.txs {
        for i in &t.inputs {
            inputs.insert(*i);
        }
    }
    let mut v = Vec::new();
    for (ti, t) in p.txs.iter().enumerate() {
        for (k, (_, spk)) in t.outputs.iter().enumerate() {
            if !ur::unspendable_script(spk) && !inputs.contains(&(t.txid, k as u32)) {
                v.push((ti, k as u32));
            }
        }
    }
    v
}

/// floresta-chain `process_proof`: the outpoints a block deletes, in input order. Every input of a non-coinbase
/// transaction, except one whose txid is a transaction of this block and whose vout is below that transaction's output
/// count (it spends an output of this block, which never became a leaf). An outpoint twice is refused.
pub fn deletion_outpoints(p: &Parsed) -> Result<Vec<(H, u32)>, &'static str> {
    let mut outs: alloc::collections::BTreeMap<H, usize> = alloc::collections::BTreeMap::new();
    for t in &p.txs {
        outs.insert(t.txid, t.outputs.len());
    }
    let mut seen: BTreeSet<(H, u32)> = BTreeSet::new();
    let mut v = Vec::new();
    for t in &p.txs[1..] {
        for (pt, pv) in &t.inputs {
            if outs.get(pt).is_some_and(|n| (*pv as usize) < *n) {
                continue;
            }
            if !seen.insert((*pt, *pv)) {
                return Err("an outpoint spent twice in one block");
            }
            v.push((*pt, *pv));
        }
    }
    Ok(v)
}

/// What one block does to the accumulator, as Floresta computes it: `adds` in block order (floresta-chain
/// `get_block_adds`: every output that is not unspendable and whose outpoint no input of the block spends) and `dels`,
/// one per input of a non-coinbase transaction that does not spend an output of this block (`process_proof`), in input
/// order. `spent` = the spent coins' data in that order (from the host; see [`SpentCoin`]).
pub struct Changes {
    pub hash: H,
    pub adds: Vec<BitcoinNodeHash>,
    pub dels: Vec<BitcoinNodeHash>,
    /// the deleted outpoints, in the same order (the host uses them to find each leaf's proof)
    pub del_outpoints: Vec<(H, u32)>,
}

pub fn block_changes(tag: &[u64; 8], block: &[u8], prev: &H, height: u32, spent: &[u8], c: &mut Counts) -> Result<Changes, &'static str> {
    let p = parse_block(block, prev)?;
    let mut adds = Vec::new();
    for (ti, v) in add_outputs(&p) {
        let t = &p.txs[ti];
        let (value, spk) = t.outputs[v as usize];
        c.leaf_blocks += ur::leaf_blocks(spk.len());
        adds.push(BitcoinNodeHash::Some(ur::utreexo_leaf(tag, &p.hash, &t.txid, v, height, ti == 0, value, spk)));
    }
    let del_outpoints = deletion_outpoints(&p)?;
    let mut dels = Vec::with_capacity(del_outpoints.len());
    let mut o = 0usize;
    for (pt, pv) in &del_outpoints {
        let s = read_spent(spent, &mut o)?;
        let leaf = ur::utreexo_leaf(tag, &s.block_hash, pt, *pv, s.header_code >> 1, s.header_code & 1 == 1, s.value, s.spk);
        if UNSPENDABLE_BIP30.contains(&leaf) {
            return Err("a deletion of a BIP30-unspendable leaf (Floresta refuses the block)");
        }
        c.leaf_blocks += ur::leaf_blocks(s.spk.len());
        dels.push(BitcoinNodeHash::Some(leaf));
    }
    if o != spent.len() {
        return Err("spent coins: more records than deletions");
    }
    c.blocks += 1;
    c.txs += p.txs.len() as u64;
    c.adds += adds.len() as u64;
    c.dels += dels.len() as u64;
    c.bytes += block.len() as u64;
    Ok(Changes { hash: p.hash, adds, dels, del_outpoints })
}

/// Floresta's accumulator update (`Consensus::update_acc` minus its BIP30 check, which [`block_changes`] makes):
/// rustreexo's `Stump::modify`, which verifies the batch proof of the deletions against the current roots.
pub fn apply(s: &Stump, ch: &Changes, proof: &Proof, c: &mut Counts) -> Result<Stump, &'static str> {
    c.proof_hashes += proof.hashes.len() as u64;
    s.modify(&ch.adds, &ch.dels, proof).map_err(|_| "the deletions do not open in the accumulator (rustreexo Stump::modify refused the proof)")
}

/// A batch proof as the guest reads it: [n targets] ‖ targets (u64 LE each) ‖ [n hashes] ‖ hashes (32 bytes; 32 zero
/// bytes = Empty).
pub fn encode_proof(p: &Proof) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&(p.targets.len() as u32).to_le_bytes());
    for t in &p.targets {
        b.extend_from_slice(&t.to_le_bytes());
    }
    b.extend_from_slice(&(p.hashes.len() as u32).to_le_bytes());
    for h in &p.hashes {
        b.extend_from_slice(&bytes(h));
    }
    b
}

pub fn decode_proof(b: &[u8]) -> Result<Proof, &'static str> {
    let mut o = 0;
    let nt = u32::from_le_bytes(take(b, &mut o, 4)?.try_into().unwrap()) as usize;
    if nt > 1 << 22 {
        return Err("proof: too many targets");
    }
    let mut targets = Vec::with_capacity(nt);
    for _ in 0..nt {
        targets.push(u64::from_le_bytes(take(b, &mut o, 8)?.try_into().unwrap()));
    }
    let nh = u32::from_le_bytes(take(b, &mut o, 4)?.try_into().unwrap()) as usize;
    if nh > 1 << 24 {
        return Err("proof: too many hashes");
    }
    let mut hashes = Vec::with_capacity(nh);
    for _ in 0..nh {
        hashes.push(node(&take(b, &mut o, 32)?.try_into().unwrap()));
    }
    if o != b.len() {
        return Err("proof: trailing bytes");
    }
    Ok(Proof { targets, hashes })
}

/// One header of the link from block H to the core tip: it must follow `prev`; returns its hash.
pub fn link_header(header: &[u8], prev: &H) -> Result<H, &'static str> {
    if header.len() != 80 {
        return Err("a link header is not 80 bytes");
    }
    if header[4..36] != *prev {
        return Err("the link from block H does not reach the core tip (a header does not follow the previous)");
    }
    Ok(dsha(header))
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
}
