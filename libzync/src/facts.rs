//! zyncAPP facts: the checks every zyncAPP and every native coin verifier shares (ZYNCAPPS.md §4, COINLOG.md §6).
//!
//! no_std and allocation-free on the check side, so a zyncAPP guest can `#[path]`-include this file (and coinlog.rs)
//! exactly as the core guest includes its libzync files. Every function returns a plain bool or an error string: the
//! caller decides whether a false is a refusal (a zyncAPP panics) or a FAIL line (a native verifier).
//!
//! What a fact proves, from ONE verified core receipt (its `ChainState` J, checked by [`check_core_statement`]):
//! - [`coin_unspent`]: coin C sits unspent at position p of the coin log at J's tip;
//! - [`coin_spent_by`]: C at p was spent by input `vin` of transaction `tx` (block position) at height `h`;
//! - [`block_in_chain`]: the block with hash B at height h is in J's chain, and the coin log after it had state S;
//! - with S's roots ([`utxo_state`]), the two coin facts at height h instead of the tip;
//! - [`tx_in_block`]: a transaction is in a block, by its Merkle branch against the block's header.
//!
//! ⚠ Positions, not keys: nothing here shows that an outpoint is ABSENT (COINLOG.md §6).

use crate::coinlog::{self, H};
use sha2::{Digest, Sha256};

/// UTREEXO_FORMAT.md §2: the coin leaf.
#[allow(clippy::too_many_arguments)]
pub fn coin_leaf(txid: &H, vout: u32, value: u64, spk: &[u8], height: u32, coinbase: bool, tracked: bool, creation_mtp: u32) -> H {
    let mut h = Sha256::new();
    h.update([0u8]);
    h.update(txid);
    h.update(vout.to_le_bytes());
    h.update(value.to_le_bytes());
    h.update((spk.len() as u32).to_le_bytes());
    h.update(spk);
    h.update(height.to_le_bytes());
    h.update([coinbase as u8 | (tracked as u8) << 1]);
    h.update(creation_mtp.to_le_bytes());
    h.finalize().into()
}

/// The perfect tree holding leaf `pos` in a forest of `n` leaves (largest tree first): (level, first leaf).
pub fn tree_of(n: u64, pos: u64) -> Option<(u32, u64)> {
    if pos >= n {
        return None;
    }
    let mut start = 0u64;
    for l in (0..64u32).rev() {
        if (n >> l) & 1 == 1 {
            let end = start + (1u64 << l);
            if pos < end {
                return Some((l, start));
            }
            start = end;
        }
    }
    None
}

/// Climb from `leaf` at `pos` with `sib` (bottom-up) using `parent(level of the parent, left, right)`. The path must
/// be exactly as long as the tree is high, and end at that tree's root in `roots` (index = level).
fn member(roots: &[Option<H>], n: u64, pos: u64, leaf: &H, sib: &[H], parent: impl Fn(u32, &H, &H) -> H) -> bool {
    // The roots present must be exactly the set bits of n (no root moved to another level; review 11-03).
    if roots.len() > 64 || (0..64).any(|l| roots.get(l).copied().flatten().is_some() != ((n >> l) & 1 == 1)) {
        return false;
    }
    let Some((l, _)) = tree_of(n, pos) else { return false };
    if sib.len() != l as usize {
        return false;
    }
    let mut node = *leaf;
    for (j, s) in sib.iter().enumerate() {
        node = if (pos >> j) & 1 == 0 { parent(j as u32 + 1, &node, s) } else { parent(j as u32 + 1, s, &node) };
    }
    roots.get(l as usize).copied().flatten() == Some(node)
}

/// `leaf` is at `pos` of the coin log with `n` leaves and these roots.
pub fn coinlog_member(roots: &[Option<H>], n: u64, pos: u64, leaf: &H, sib: &[H]) -> bool {
    member(roots, n, pos, leaf, sib, coinlog::parent)
}

/// Coin `coin` (its [`coin_leaf`]) is unspent at `pos`.
pub fn coin_unspent(roots: &[Option<H>], n: u64, pos: u64, coin: &H, sib: &[H]) -> bool {
    coinlog_member(roots, n, pos, coin, sib)
}

/// Coin `coin` at `pos` was spent by input `vin` of the transaction at block position `tx` (coinbase 0) at `height`.
#[allow(clippy::too_many_arguments)]
pub fn coin_spent_by(roots: &[Option<H>], n: u64, pos: u64, coin: &H, height: u32, tx: u32, vin: u32, sib: &[H]) -> bool {
    coinlog_member(roots, n, pos, &coinlog::spent(coin, &coinlog::spender(height, tx, vin)), sib)
}

pub const TAG_BLOCK_NODE: u8 = 0x01;
pub const TAG_BLOCK_LEAF: u8 = 0x02;
pub const TAG_UTXO_STATE: u8 = 0x04;

/// The block forest's interior hash (the guest's utreexo.rs `parent`; no level binding, as in Utreexo).
pub fn block_parent(_l: u32, a: &H, b: &H) -> H {
    let mut h = Sha256::new();
    h.update([TAG_BLOCK_NODE]);
    h.update(a);
    h.update(b);
    h.finalize().into()
}

/// HANDOVER.md §3: the coin state after a block.
pub fn utxo_state(leaves: u64, value: u64, roots: &[Option<H>]) -> H {
    let mut h = Sha256::new();
    h.update([TAG_UTXO_STATE]);
    h.update(leaves.to_le_bytes());
    h.update(value.to_le_bytes());
    for r in roots.iter().rev().flatten() {
        h.update(r);
    }
    h.finalize().into()
}

/// HANDOVER.md §3: a block's leaf in the block forest (`hash` internal byte order).
pub fn block_leaf(hash: &H, height: u32, state: &H) -> H {
    let mut h = Sha256::new();
    h.update([TAG_BLOCK_LEAF]);
    h.update(hash);
    h.update(height.to_le_bytes());
    h.update(state);
    h.finalize().into()
}

/// The block with `hash` at `height`, followed by coin state `state`, is in the chain whose block forest is
/// (`block_roots`, `block_leaves`). A block's position is its height (one leaf per height from genesis).
pub fn block_in_chain(block_roots: &[Option<H>], block_leaves: u64, hash: &H, height: u32, state: &H, sib: &[H]) -> bool {
    member(block_roots, block_leaves, height as u64, &block_leaf(hash, height, state), sib, block_parent)
}

fn dsha(b: &[u8]) -> H {
    Sha256::digest(Sha256::digest(b)).into()
}

/// A block header's hash (internal byte order).
pub fn header_hash(header: &[u8; 80]) -> H {
    dsha(header)
}

/// Climbs a Merkle branch from `leaf` at `index` to the root it implies.
fn merkle_climb(leaf: H, index: u32, branch: &[H]) -> Option<H> {
    if branch.len() >= 32 || (index as u64) >> branch.len() != 0 {
        return None;
    }
    let mut node = leaf;
    for (j, s) in branch.iter().enumerate() {
        let mut p = [0u8; 64];
        if (index >> j) & 1 == 0 {
            p[..32].copy_from_slice(&node);
            p[32..].copy_from_slice(s);
        } else {
            // the right child equal to the left would be Core's duplicated last node only when it IS the last node;
            // a left sibling equal to the node is the CVE-2012-2459 shape and never needed for a real branch
            if s == &node {
                return None;
            }
            p[..32].copy_from_slice(s);
            p[32..].copy_from_slice(&node);
        }
        node = dsha(&p);
    }
    Some(node)
}

/// A transaction (non-witness serialization, so its hash is the txid) is at `index` of the block with this header,
/// by its Merkle `branch` (bottom-up). The caller learns the txid from the bytes, never from the prover.
///
/// The tree's DEPTH is bound by the block's coinbase: its bytes and its branch, which must have the same length. Without
/// that, a 64-byte transaction T mined in the block could be read as an inner node, and a fake transaction whose txid
/// is one half of T would verify one level deeper (Lerner's attack; review 11-03). A fake coinbase one level deeper
/// would need a txid whose last 27 bytes are zero (the null prevout), out of reach. 64-byte transactions and
/// coinbases are refused outright.
pub fn tx_in_block(header: &[u8; 80], tx: &[u8], index: u32, branch: &[H], coinbase: &[u8], cb_branch: &[H]) -> Option<H> {
    if tx.len() == 64 || cb_branch.len() != branch.len() {
        return None;
    }
    let mut o = 0;
    let cb = parse_tx(coinbase, &mut o).filter(|_| o == coinbase.len())?;
    if cb.stripped.len() == 64 || cb.inputs.len() != 1 || cb.inputs[0] != ([0u8; 32], 0xffff_ffff) {
        return None;
    }
    let root = &header[36..68];
    if merkle_climb(dsha(&cb.stripped), 0, cb_branch)?[..] != *root {
        return None;
    }
    let txid = dsha(tx);
    (merkle_climb(txid, index, branch)?[..] == *root).then_some(txid)
}

/// A transaction's outpoints and outputs, from its serialization (with or without witness).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxView {
    /// The serialization without witness: its double SHA-256 is the txid.
    pub stripped: alloc::vec::Vec<u8>,
    pub inputs: alloc::vec::Vec<(H, u32)>,
    pub outputs: alloc::vec::Vec<(u64, alloc::vec::Vec<u8>)>,
}

fn rd<'a>(b: &'a [u8], o: &mut usize, n: usize) -> Option<&'a [u8]> {
    let e = o.checked_add(n)?;
    let r = b.get(*o..e)?;
    *o = e;
    Some(r)
}

fn varint(b: &[u8], o: &mut usize) -> Option<u64> {
    let f = rd(b, o, 1)?[0];
    Some(match f {
        0xfd => u16::from_le_bytes(rd(b, o, 2)?.try_into().ok()?) as u64,
        0xfe => u32::from_le_bytes(rd(b, o, 4)?.try_into().ok()?) as u64,
        0xff => u64::from_le_bytes(rd(b, o, 8)?.try_into().ok()?),
        x => x as u64,
    })
}

fn put_varint(v: &mut alloc::vec::Vec<u8>, n: u64) {
    match n {
        0..=0xfc => v.push(n as u8),
        0xfd..=0xffff => { v.push(0xfd); v.extend_from_slice(&(n as u16).to_le_bytes()); }
        0x1_0000..=0xffff_ffff => { v.push(0xfe); v.extend_from_slice(&(n as u32).to_le_bytes()); }
        _ => { v.push(0xff); v.extend_from_slice(&n.to_le_bytes()); }
    }
}

/// Parses one transaction at `*o` (BIP144 witness form or legacy), advancing `*o` past it.
pub fn parse_tx(b: &[u8], o: &mut usize) -> Option<TxView> {
    let start = *o;
    let version = rd(b, o, 4)?;
    let mut seg = false;
    if b.get(*o) == Some(&0) && b.get(*o + 1) == Some(&1) {
        seg = true;
        *o += 2;
    }
    let mut st = alloc::vec::Vec::new();
    st.extend_from_slice(version);
    let nin = varint(b, o)?;
    if nin == 0 || nin > 1 << 20 {
        return None;
    }
    put_varint(&mut st, nin);
    let mut inputs = alloc::vec::Vec::new();
    for _ in 0..nin {
        let s0 = *o;
        let txid: H = rd(b, o, 32)?.try_into().ok()?;
        let vout = u32::from_le_bytes(rd(b, o, 4)?.try_into().ok()?);
        let l = varint(b, o)? as usize;
        rd(b, o, l)?;
        rd(b, o, 4)?;
        st.extend_from_slice(&b[s0..*o]);
        inputs.push((txid, vout));
    }
    let nout = varint(b, o)?;
    if nout > 1 << 20 {
        return None;
    }
    put_varint(&mut st, nout);
    let mut outputs = alloc::vec::Vec::new();
    for _ in 0..nout {
        let s0 = *o;
        let value = u64::from_le_bytes(rd(b, o, 8)?.try_into().ok()?);
        let l = varint(b, o)? as usize;
        let spk = rd(b, o, l)?.to_vec();
        st.extend_from_slice(&b[s0..*o]);
        outputs.push((value, spk));
    }
    if seg {
        for _ in 0..nin {
            let k = varint(b, o)?;
            for _ in 0..k {
                let l = varint(b, o)? as usize;
                rd(b, o, l)?;
            }
        }
    }
    st.extend_from_slice(rd(b, o, 4)?);
    let _ = start;
    Some(TxView { stripped: st, inputs, outputs })
}

/// A whole block: its 80-byte header and every transaction.
pub fn parse_block(b: &[u8]) -> Option<([u8; 80], alloc::vec::Vec<TxView>)> {
    let header: [u8; 80] = b.get(..80)?.try_into().ok()?;
    let mut o = 80;
    let n = varint(b, &mut o)?;
    let mut txs = alloc::vec::Vec::new();
    for _ in 0..n {
        txs.push(parse_tx(b, &mut o)?);
    }
    (o == b.len()).then_some((header, txs))
}

/// The Merkle branch (bottom-up) of transaction `index` among `txids`, as Core builds the tree (an odd level
/// duplicates its last node).
pub fn merkle_branch(txids: &[H], index: usize) -> alloc::vec::Vec<H> {
    let mut level = txids.to_vec();
    let mut i = index;
    let mut br = alloc::vec::Vec::new();
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            level.push(*level.last().unwrap());
        }
        br.push(level[i ^ 1]);
        level = level.chunks_exact(2).map(|c| { let mut p = [0u8; 64]; p[..32].copy_from_slice(&c[0]); p[32..].copy_from_slice(&c[1]); dsha(&p) }).collect();
        i /= 2;
    }
    br
}

/// The txid of a [`TxView`].
pub fn txid(t: &TxView) -> H {
    dsha(&t.stripped)
}

/// The fields of a core `ChainState` a zyncAPP's statement rests on (SWIFTSYNC_PROOFS.md §3).
pub struct CoreStatement<'a> {
    pub kind: u32,
    pub version: u32,
    pub profile: u32,
    pub self_id: &'a [u32; 8],
    pub anchor_id: &'a H,
    pub ss_set: u64,
}

pub const KIND_CHAIN: u32 = 0xC4A1_0012;
pub const PROFILE_ASSUMEVALID: u32 = 2;
pub const PROFILE_SIGNET: u32 = 4;
pub const PROFILE_REGTEST: u32 = 8;
pub const PROFILE_COINLOG: u32 = 16;
/// The chain state's former hint hash is the counting trie's root (ttree.rs; absence, history and balance facts).
pub const PROFILE_TRIE: u32 = 32;
/// ss_set / ss_acc are the live-coin multiset hash (instant IBD), not SwiftSync's total.
pub const PROFILE_UTXOHASH: u32 = 64;

/// The network a zyncAPP answers for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Network {
    Mainnet,
    Signet,
    Regtest,
}

impl Network {
    /// The profile's network bits for this network (signet bit 2, regtest bit 3, mainnet neither).
    pub fn bits(self) -> u32 {
        match self { Network::Mainnet => 0, Network::Signet => PROFILE_SIGNET, Network::Regtest => PROFILE_REGTEST }
    }
}

/// ZYNCAPPS.md §4 step 3, the same in every zyncAPP: J is a core chain state of `core_id`, at least `min_version`,
/// on the expected network, with scripts checked unless `allow_assumevalid`, a coin log (coin facts hold at every
/// block), and anchored at the network's pinned genesis anchor (`genesis_anchor`).
pub fn check_core_statement(j: &CoreStatement, core_id: &[u32; 8], min_version: u32, net: Network, allow_assumevalid: bool, genesis_anchor: &H) -> Result<(), &'static str> {
    if j.kind != KIND_CHAIN {
        return Err("not a core ChainState");
    }
    if j.version < min_version {
        return Err("ChainState version too old for this question");
    }
    if j.self_id != core_id {
        return Err("self_id is not the core image id the receipt verified under");
    }
    if j.profile & (PROFILE_SIGNET | PROFILE_REGTEST) != net.bits() {
        return Err("wrong network");
    }
    if j.profile & PROFILE_ASSUMEVALID != 0 && !allow_assumevalid {
        return Err("scripts were not checked (assumevalid)");
    }
    // ss_set may be non-zero only as the live-coin hash's count; without that bit it is SwiftSync's total, unused.
    if j.profile & PROFILE_COINLOG == 0 || (j.ss_set != 0 && j.profile & PROFILE_UTXOHASH == 0) {
        return Err("not a coin-log chain: coin facts are not sound at every block");
    }
    if j.anchor_id != genesis_anchor {
        return Err("not anchored at the network's genesis");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coinlog::Store;
    use alloc::vec::Vec;

    fn h(i: u64) -> H {
        Sha256::digest(i.to_le_bytes()).into()
    }

    #[test]
    fn coin_paths() {
        let mut s = Store::default();
        for i in 0..13 {
            s.push(h(i));
        }
        let c5 = h(5);
        let sp = coinlog::spent(&c5, &coinlog::spender(9, 2, 1));
        s.set(5, sp);
        let (roots, n) = (s.roots(), s.len());
        for p in 0..n {
            let path = s.path(p);
            if p == 5 {
                assert!(coin_spent_by(&roots, n, p, &c5, 9, 2, 1, &path));
                assert!(!coin_unspent(&roots, n, p, &c5, &path));
                assert!(!coin_spent_by(&roots, n, p, &c5, 9, 2, 0, &path));
            } else {
                assert!(coin_unspent(&roots, n, p, &h(p), &path), "{p}");
                assert!(!coin_unspent(&roots, n, p ^ 1, &h(p), &path) || p ^ 1 >= n);
                let mut bad = path.clone();
                if let Some(x) = bad.first_mut() {
                    x[0] ^= 1;
                    assert!(!coin_unspent(&roots, n, p, &h(p), &bad));
                }
                if !path.is_empty() {
                    assert!(!coin_unspent(&roots, n, p, &h(p), &path[1..]), "a shortened path");
                }
            }
        }
        assert!(!coin_unspent(&roots, n, n, &h(0), &[]));
        assert_eq!(tree_of(13, 12), Some((0, 12)));
        assert_eq!(tree_of(13, 9), Some((2, 8)));
        assert_eq!(tree_of(13, 13), None);
    }

    #[test]
    fn block_forest_paths() {
        // the guest's Stump::add: carry with parent(existing, node)
        let mut roots: Vec<Option<H>> = Vec::new();
        let mut levels: Vec<Vec<H>> = Vec::new();
        let leaves: Vec<H> = (0..11u32).map(|i| block_leaf(&h(i as u64), i, &h(100 + i as u64))).collect();
        for (i, leaf) in leaves.iter().enumerate() {
            let mut node = *leaf;
            let mut l = 0;
            while roots.get(l).copied().flatten().is_some() {
                node = block_parent(0, &roots[l].unwrap(), &node);
                roots[l] = None;
                l += 1;
            }
            if roots.len() <= l {
                roots.resize(l + 1, None);
            }
            roots[l] = Some(node);
            let _ = i;
        }
        // all nodes, for paths
        levels.push(leaves.clone());
        while levels.last().unwrap().len() > 1 {
            let prev = levels.last().unwrap();
            let next: Vec<H> = prev.chunks_exact(2).map(|c| block_parent(0, &c[0], &c[1])).collect();
            levels.push(next);
        }
        let n = leaves.len() as u64;
        for p in 0..n {
            let (l, _) = tree_of(n, p).unwrap();
            let path: Vec<H> = (0..l as usize).map(|j| levels[j][((p >> j) ^ 1) as usize]).collect();
            assert!(block_in_chain(&roots, n, &h(p), p as u32, &h(100 + p), &path), "{p}");
            assert!(!block_in_chain(&roots, n, &h(p), p as u32, &h(101 + p), &path));
        }
        let st = utxo_state(3, 7, &[Some(h(1)), None, Some(h(2))]);
        let mut pre = Vec::new();
        pre.push(TAG_UTXO_STATE);
        pre.extend_from_slice(&3u64.to_le_bytes());
        pre.extend_from_slice(&7u64.to_le_bytes());
        pre.extend_from_slice(&h(2));
        pre.extend_from_slice(&h(1));
        assert_eq!(st, <[u8; 32]>::from(Sha256::digest(&pre)));
    }

    #[test]
    fn merkle_branch_check() {
        // a 3-tx block: Core duplicates the last node on an odd level. The coinbase binds the tree's depth.
        let mut cbv = alloc::vec![1u8, 0, 0, 0, 1];
        cbv.extend_from_slice(&[0u8; 32]);
        cbv.extend_from_slice(&[0xff; 4]);
        cbv.extend_from_slice(&[4, 1, 2, 3, 4]); // 65 bytes in all: a 64-byte coinbase is refused
        cbv.extend_from_slice(&[0xff; 4]);
        cbv.extend_from_slice(&[1, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0x51]); // one output: value 1, script OP_1
        cbv.extend_from_slice(&[0; 4]);
        let txs: [&[u8]; 3] = [&cbv, b"second tx bytes!", b"third transaction"];
        let ids: alloc::vec::Vec<H> = txs.iter().map(|t| dsha(t)).collect();
        let cat = |a: &H, b: &H| { let mut p = [0u8; 64]; p[..32].copy_from_slice(a); p[32..].copy_from_slice(b); dsha(&p) };
        let (l0, l1) = (cat(&ids[0], &ids[1]), cat(&ids[2], &ids[2]));
        let root = cat(&l0, &l1);
        let mut header = [0u8; 80];
        header[36..68].copy_from_slice(&root);
        let cbb = [ids[1], l1];
        assert_eq!(tx_in_block(&header, txs[1], 1, &[ids[0], l1], &cbv, &cbb), Some(ids[1]));
        assert_eq!(tx_in_block(&header, txs[2], 2, &[ids[2], l0], &cbv, &cbb), Some(ids[2]));
        assert_eq!(tx_in_block(&header, &cbv, 0, &cbb, &cbv, &cbb), Some(ids[0]));
        assert_eq!(tx_in_block(&header, txs[1], 0, &[ids[0], l1], &cbv, &cbb), None);
        assert_eq!(tx_in_block(&header, &[0u8; 64], 0, &[], &cbv, &[]), None);
        assert_eq!(tx_in_block(&header, txs[0], 4, &[ids[1], l1], &cbv, &cbb), None, "index past the branch");
        // a branch one level deeper than the coinbase's is refused (Lerner's 64-byte inner-node shape)
        assert_eq!(tx_in_block(&header, txs[1], 2, &[ids[0], ids[0], l1], &cbv, &cbb), None);
        // a non-coinbase in the coinbase slot is refused
        assert_eq!(tx_in_block(&header, txs[1], 1, &[ids[0], l1], txs[1], &cbb), None);
    }

    #[test]
    fn parse_real_txs() {
        // mainnet block 170's second tx (the first bitcoin payment, legacy) and a segwit tx shape
        let hexs = "0100000001c997a5e56e104102fa209c6a852dd90660a20b2d9c352423edce25857fcd3704000000004847304402204e45e16932b8af514961a1d3a1a25fdf3f4f7732e9d624c6c61548ab5fb8cd410220181522ec8eca07de4860a4acdd12909d831cc56cbbac4622082221a8768d1d0901ffffffff0200ca9a3b00000000434104ae1a62fe09c5f51b13905f07f06b99a2f7159b2225f374cd378d71302fa28414e7aab37397f554a7df5f142c21c1b7303b8a0626f1baded5c72a704f7e6cd84cac00286bee0000000043410411db93e1dcdb8a016b49840f8c53bc1eb68a382e97b1482ecad7b148a6909a5cb2e0eaddfb84ccf9744464f82e160bfa9b8b64f9d4c03f999b8643f656b412a3ac00000000";
        let raw: alloc::vec::Vec<u8> = (0..hexs.len()).step_by(2).map(|i| u8::from_str_radix(&hexs[i..i + 2], 16).unwrap()).collect();
        let mut o = 0;
        let t = parse_tx(&raw, &mut o).unwrap();
        assert_eq!(o, raw.len());
        let mut id = txid(&t);
        id.reverse();
        let shown: alloc::string::String = id.iter().map(|b| alloc::format!("{b:02x}")).collect();
        assert_eq!(shown, "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16");
        assert_eq!(t.outputs.len(), 2);
        assert_eq!(t.outputs[0].0, 1_000_000_000);
        // the same tx in witness form (one empty witness stack): same txid
        let mut w = raw[..4].to_vec();
        w.extend_from_slice(&[0, 1]);
        w.extend_from_slice(&raw[4..raw.len() - 4]);
        w.push(0);
        w.extend_from_slice(&raw[raw.len() - 4..]);
        let mut o = 0;
        let tw = parse_tx(&w, &mut o).unwrap();
        assert_eq!((o, txid(&tw)), (w.len(), txid(&t)));
        // branch round trip
        let ids: alloc::vec::Vec<H> = (0..5u64).map(h).collect();
        let mut header = [0u8; 80];
        let mut level = ids.clone();
        while level.len() > 1 {
            if level.len() % 2 == 1 { level.push(*level.last().unwrap()); }
            level = level.chunks_exact(2).map(|c| { let mut p = [0u8; 64]; p[..32].copy_from_slice(&c[0]); p[32..].copy_from_slice(&c[1]); dsha(&p) }).collect();
        }
        header[36..68].copy_from_slice(&level[0]);
        for i in 0..5 {
            let br = merkle_branch(&ids, i);
            let mut node = ids[i];
            for (j, s) in br.iter().enumerate() {
                let mut p = [0u8; 64];
                if (i >> j) & 1 == 0 { p[..32].copy_from_slice(&node); p[32..].copy_from_slice(s); } else { p[..32].copy_from_slice(s); p[32..].copy_from_slice(&node); }
                node = dsha(&p);
            }
            assert_eq!(node, level[0], "{i}");
        }
    }

    #[test]
    fn core_statement() {
        let id = [7u32; 8];
        let anchor = h(1);
        let ok = CoreStatement { kind: KIND_CHAIN, version: 2, profile: 1 | PROFILE_COINLOG, self_id: &id, anchor_id: &anchor, ss_set: 0 };
        assert!(check_core_statement(&ok, &id, 2, Network::Mainnet, false, &anchor).is_ok());
        assert!(check_core_statement(&ok, &[8; 8], 2, Network::Mainnet, false, &anchor).is_err());
        assert!(check_core_statement(&ok, &id, 3, Network::Mainnet, false, &anchor).is_err());
        assert!(check_core_statement(&ok, &id, 2, Network::Signet, false, &anchor).is_err());
        assert!(check_core_statement(&ok, &id, 2, Network::Mainnet, false, &h(2)).is_err());
        let av = CoreStatement { profile: 1 | PROFILE_COINLOG | PROFILE_ASSUMEVALID, ..ok };
        assert!(check_core_statement(&av, &id, 2, Network::Mainnet, false, &anchor).is_err());
        assert!(check_core_statement(&av, &id, 2, Network::Mainnet, true, &anchor).is_ok());
        let rt = CoreStatement { profile: 1 | PROFILE_COINLOG | PROFILE_REGTEST, ..ok };
        assert!(check_core_statement(&rt, &id, 2, Network::Mainnet, false, &anchor).is_err(), "a regtest image is not mainnet");
        assert!(check_core_statement(&rt, &id, 2, Network::Regtest, false, &anchor).is_ok());
        let b = CoreStatement { profile: 1, ..av };
        assert!(check_core_statement(&b, &id, 2, Network::Mainnet, true, &anchor).is_err());
        // the product: a non-zero ss_set is the live-coin hash's count, accepted only under that profile bit
        let p = CoreStatement { profile: 1 | PROFILE_COINLOG | PROFILE_TRIE | PROFILE_UTXOHASH, ss_set: 5, ..ok };
        assert!(check_core_statement(&p, &id, 2, Network::Mainnet, false, &anchor).is_ok());
        let q = CoreStatement { profile: 1 | PROFILE_COINLOG | PROFILE_TRIE, ..p };
        assert!(check_core_statement(&q, &id, 2, Network::Mainnet, false, &anchor).is_err());
    }
}
