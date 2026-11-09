//! Counting-trie FACTS (E2 step 4): what a zyncAPP or a wallet checks against the trie root a verified chain state
//! commits (the state's `trie_root` field under profile bit 5). Shared by the CLI, C and WASM verifiers; no_std.
//!
//! A proof is the trie pruned to the keys it answers for: the same preorder format the guest's update reads
//! (`trie_model::encode`), served by the host's store (`TrieStore::prove`). The checks: the proof is in counts mode, its
//! stated root is the given root and its hash is that root; then the key's path is walked. Each node is fixed by the
//! root (one SHA-256 compression per node), so a walk that ends at
//!   - an empty slot, or an address / coin leaf with a different key: the key is ABSENT (a subtree holding one leaf IS
//!     that leaf, so no other key can be under it);
//!   - the address leaf: its unspent count and value (the BALANCE), and its coin trie;
//!   - the coin leaf: UNSPENT (value), or SPENT by (height, transaction, input), with its value.
//!
//! A path that meets a pruned node is "not covered": the proof does not answer for that key (never a verdict).
//! A HISTORY proof expands an address's whole coin trie: every coin ever paid to that script, each with its state.
//!
//! Which coins: those Core keeps (no OP_RETURN / over-10,000-byte scripts, no genesis coinbase; the BIP30 pairs 91,722 /
//! 91,880 and 91,812 / 91,842 filed once, at the later block). ABSENT is per (script, outpoint): ZYNCAPPS.md §2.

use crate::coinlog::W;
use crate::trie_model::{decode, hash_top, Sub, Top};
use crate::ttree::{bit, split, Key};
use alloc::vec::Vec;

/// The trie key of a coin: SHA-256(scriptPubKey) ‖ the txid (internal byte order) with the output index XORed, little-
/// endian, into its first 4 bytes. As the guest's `shash` / `okey` (agg_lite.rs).
pub fn coin_key(script_pubkey: &[u8], txid: &[u8; 32], vout: u32) -> Key {
    let mut k = [0u8; 64];
    k[..32].copy_from_slice(&script_hash(script_pubkey));
    k[32..].copy_from_slice(txid);
    for (b, v) in k[32..36].iter_mut().zip(vout.to_le_bytes()) { *b ^= v; }
    k
}

/// The address key: SHA-256(scriptPubKey).
pub fn script_hash(script_pubkey: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(script_pubkey).into()
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum FactErr {
    /// not a well-formed proof
    Decode,
    /// a proof without values and counts (not the product trie)
    NotCounts,
    /// the proof is not of the given root
    Root,
    /// the key's path meets a pruned node: the proof does not answer for it
    NotCovered,
}

/// A spender record, decoded: the spending block's height, the transaction's position in it, the input index.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Spender { pub height: u32, pub tx: u32, pub vin: u32 }

fn spender_of(w: &W) -> Spender { Spender { height: w[0], tx: w[1], vin: w[2] } }

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum CoinState {
    Absent,
    Unspent { value: u64 },
    Spent { value: u64, by: Spender },
}

/// An address leaf: its unspent coins' count and total value, and its coin trie's root.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Balance { pub count: u64, pub value: u64, pub coins_root: W }

fn open(proof: &[u32], root: &W) -> Result<Top, FactErr> {
    let (counts, r, t) = decode(proof).ok_or(FactErr::Decode)?;
    if !counts { return Err(FactErr::NotCounts); }
    if r != *root || hash_top(&t, true) != *root { return Err(FactErr::Root); }
    Ok(t)
}

/// The address leaf for S: Ok(None) = absent, Ok(Some(leaf)) = present.
fn find_addr<'a>(t: &'a Top, s: &W) -> Result<Option<(u64, u64, &'a Sub)>, FactErr> {
    let mut n = t;
    let mut d = 0;
    loop {
        match n {
            Top::Empty => return Ok(None),
            Top::Pruned(_) => return Err(FactErr::NotCovered),
            Top::Inner(l, r) => {
                if d >= 256 { return Err(FactErr::Decode); }
                n = if bit(s, d) { r } else { l };
                d += 1;
            }
            Top::Addr { s: a, count, value, sub } => return Ok((a == s).then_some((*count, *value, &**sub))),
        }
    }
}

/// The coin at `key`: absent, unspent with its value, or spent with its spender.
pub fn coin(proof: &[u32], root: &W, key: &Key) -> Result<CoinState, FactErr> {
    let t = open(proof, root)?;
    let (s, o) = split(key);
    let Some((_, _, sub)) = find_addr(&t, &s)? else { return Ok(CoinState::Absent) };
    let mut n = sub;
    let mut d = 0;
    loop {
        match n {
            Sub::Empty => return Ok(CoinState::Absent),
            Sub::Pruned(_) => return Err(FactErr::NotCovered),
            Sub::Inner(l, r) => {
                if d >= 256 { return Err(FactErr::Decode); }
                n = if bit(&o, d) { r } else { l };
                d += 1;
            }
            Sub::Coin { o: c, sp, value } => return Ok(if *c != o { CoinState::Absent } else {
                match sp { None => CoinState::Unspent { value: *value }, Some(w) => CoinState::Spent { value: *value, by: spender_of(w) } }
            }),
        }
    }
}

/// The balance of the script with hash `s`: None = no coin has ever been paid to it.
pub fn balance(proof: &[u32], root: &W, s: &[u8; 32]) -> Result<Option<Balance>, FactErr> {
    let t = open(proof, root)?;
    let s = words_be(s);
    Ok(find_addr(&t, &s)?.map(|(count, value, sub)| Balance { count, value, coins_root: crate::trie_model::hash_sub(sub, true) }))
}

/// One coin in an address's history: its coin key (the txid with the output index XORed in), state and value.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct HistCoin { pub okey: [u8; 32], pub state: CoinState }

/// Every coin ever paid to the script with hash `s`, in coin-key order, with the balance. The proof must expand the
/// whole coin trie (no pruned node in it). None = no coin has ever been paid to it.
pub fn history(proof: &[u32], root: &W, s: &[u8; 32]) -> Result<Option<(Balance, Vec<HistCoin>)>, FactErr> {
    let t = open(proof, root)?;
    let sw = words_be(s);
    let Some((count, value, sub)) = find_addr(&t, &sw)? else { return Ok(None) };
    let mut out = Vec::new();
    fn walk(n: &Sub, out: &mut Vec<HistCoin>) -> Result<(), FactErr> {
        match n {
            Sub::Empty => Ok(()),
            Sub::Pruned(_) => Err(FactErr::NotCovered),
            Sub::Inner(l, r) => { walk(l, out)?; walk(r, out) }
            Sub::Coin { o, sp, value } => {
                let mut k = [0u8; 32];
                for (c, x) in k.chunks_exact_mut(4).zip(o) { c.copy_from_slice(&x.to_be_bytes()); }
                let state = match sp { None => CoinState::Unspent { value: *value }, Some(w) => CoinState::Spent { value: *value, by: spender_of(w) } };
                out.push(HistCoin { okey: k, state });
                Ok(())
            }
        }
    }
    walk(sub, &mut out)?;
    // the address leaf's count and value are the sum of its unspent coins (the guest keeps them so; checked here too)
    let (c, v) = out.iter().fold((0u64, 0u64), |a, x| match x.state { CoinState::Unspent { value } => (a.0 + 1, a.1.saturating_add(value)), _ => a });
    if (c, v) != (count, value) { return Err(FactErr::Decode); }
    Ok(Some((Balance { count, value, coins_root: crate::trie_model::hash_sub(sub, true) }, out)))
}

fn words_be(b: &[u8; 32]) -> W { core::array::from_fn(|i| u32::from_be_bytes(b[4 * i..4 * i + 4].try_into().unwrap())) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trie_store::TrieStore;

    fn sp(h: u32, t: u32, v: u32) -> W { [h, t, v, 0, 0, 0, 0, 0] }

    #[test]
    fn coin_balance_history_and_absence() {
        let a = b"\x00\x14aaaaaaaaaaaaaaaaaaaa".to_vec();
        let b = b"\x51\x20bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_vec();
        let tx1 = [1u8; 32];
        let tx2 = [2u8; 32];
        let mut st = TrieStore::new(true);
        // block 1: three coins to A (one spent in its own block), one to B
        st.apply(&[], &[(coin_key(&a, &tx1, 0), None, 500), (coin_key(&a, &tx1, 1), Some(sp(1, 2, 0)), 70),
                        (coin_key(&a, &tx2, 0), None, 30), (coin_key(&b, &tx2, 1), None, 9)]).unwrap();
        // block 2: A's first coin spent
        st.apply(&[(coin_key(&a, &tx1, 0), sp(2, 1, 3))], &[]).unwrap();
        let root = st.root();

        let k = coin_key(&a, &tx1, 0);
        let p = st.prove_coins(&[k]);
        assert_eq!(coin(&p, &root, &k), Ok(CoinState::Spent { value: 500, by: Spender { height: 2, tx: 1, vin: 3 } }));
        let k2 = coin_key(&a, &tx2, 0);
        assert_eq!(coin(&st.prove_coins(&[k2]), &root, &k2), Ok(CoinState::Unspent { value: 30 }));
        // absent: a coin never made at an existing address, and at an address never paid
        let k3 = coin_key(&a, &tx2, 7);
        assert_eq!(coin(&st.prove_coins(&[k3]), &root, &k3), Ok(CoinState::Absent));
        let k4 = coin_key(b"\x6a", &tx1, 0);
        assert_eq!(coin(&st.prove_coins(&[k4]), &root, &k4), Ok(CoinState::Absent));
        // a proof for one key does not answer for another under a pruned node
        assert_eq!(coin(&p, &root, &k2), Err(FactErr::NotCovered));

        let sa = script_hash(&a);
        let bal = balance(&st.prove_addr(&sa), &root, &sa).unwrap().unwrap();
        assert_eq!((bal.count, bal.value), (1, 30));
        assert_eq!(balance(&st.prove_addr(&script_hash(b"\x6a")), &root, &script_hash(b"\x6a")), Ok(None));
        let (hb, coins) = history(&st.prove_history(&sa), &root, &sa).unwrap().unwrap();
        assert_eq!(hb, bal);
        assert_eq!(coins.len(), 3);
        assert_eq!(coins.iter().filter(|c| matches!(c.state, CoinState::Unspent { .. })).count(), 1);
        // a balance proof prunes the coin trie: not a history proof
        assert_eq!(history(&st.prove_addr(&sa), &root, &sa).err(), Some(FactErr::NotCovered));

        // a proof of another root, a tampered proof, a counts-off proof: refused
        assert_eq!(coin(&p, &[9; 8], &k), Err(FactErr::Root));
        let mut bad = p.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(coin(&bad, &root, &k).is_err());
        let mut nc = p.clone();
        nc[0] = 0;
        assert_eq!(coin(&nc, &root, &k), Err(FactErr::NotCounts));
    }

    /// Fixture: an address with an unspent coin, a coin spent later, a coin spent in its own block, and a neighbour.
    fn fixture() -> (TrieStore, W, [u8; 32], Vec<Key>) {
        let a = b"\x00\x14aaaaaaaaaaaaaaaaaaaa".to_vec();
        let b = b"\x51\x20bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_vec();
        let (tx1, tx2) = ([1u8; 32], [2u8; 32]);
        let mut st = TrieStore::new(true);
        st.apply(&[], &[(coin_key(&a, &tx1, 0), None, 500), (coin_key(&a, &tx1, 1), Some(sp(1, 2, 0)), 70),
                        (coin_key(&a, &tx2, 0), None, 30), (coin_key(&b, &tx2, 1), None, 9)]).unwrap();
        st.apply(&[(coin_key(&a, &tx1, 0), sp(2, 1, 3))], &[]).unwrap();
        let root = st.root();
        (st, root, script_hash(&a), alloc::vec![coin_key(&a, &tx1, 0), coin_key(&a, &tx2, 0), coin_key(&a, &tx2, 7)])
    }

    /// What the pre-11-09 reader made of a witness, re-encoded: the one canonical form of its MEANING (an independent
    /// re-implementation of the old lenient grammar). A mutation whose canonical form verifies is one the old decoder
    /// accepted with the same answer; the tests below show each such mutation is refused now.
    fn old_reader_accepts(w: &[u32], ok: impl Fn(&[u32]) -> bool) -> bool {
        let c = canonicalise(w);
        !c.is_empty() && c != w && ok(&c)
    }

    fn canonicalise(w: &[u32]) -> Vec<u32> {
        // parse leniently with the same grammar as trie_model, then re-encode
        fn sub(w: &[u32], i: &mut usize, out: &mut Vec<u32>) -> Option<()> {
            let tag = *w.get(*i)?; *i += 1;
            match tag {
                0 => out.push(0),
                1 => { out.push(1); out.extend_from_slice(w.get(*i..*i + 8)?); *i += 8; }
                2 | 5 | 6 => {
                    // tag 2 whose one child is pruned is the one-sided 5 / 6 the encoder writes
                    if tag != 2 { out.push(tag); out.extend_from_slice(w.get(*i..*i + 8)?); *i += 8; return sub(w, i, out); }
                    let (mut l, mut r) = (Vec::new(), Vec::new());
                    sub(w, i, &mut l)?; sub(w, i, &mut r)?;
                    let pruned = |x: &[u32]| x.first() == Some(&1);
                    if pruned(&l) && !pruned(&r) { out.push(5); out.extend_from_slice(&l[1..9]); out.extend(r); }
                    else if pruned(&r) && !pruned(&l) { out.push(6); out.extend_from_slice(&r[1..9]); out.extend(l); }
                    else { out.push(2); out.extend(l); out.extend(r); }
                }
                4 => {
                    out.push(4); out.extend_from_slice(w.get(*i..*i + 8)?); *i += 8;
                    let f = *w.get(*i)?; *i += 1;
                    let s = w.get(*i..*i + 8)?; *i += 8;
                    out.push((f == 1) as u32);
                    // spent: the hash binds words 0-4 (5-7 are overwritten); unspent: nothing
                    let mut sp = [0u32; 8];
                    if f == 1 { sp[..5].copy_from_slice(&s[..5]); }
                    out.extend_from_slice(&sp);
                    out.extend_from_slice(w.get(*i..*i + 2)?); *i += 2;
                }
                _ => return None,
            }
            Some(())
        }
        fn top(w: &[u32], i: &mut usize, out: &mut Vec<u32>) -> Option<()> {
            let tag = *w.get(*i)?;
            if tag == 3 {
                *i += 1; out.push(3); out.extend_from_slice(w.get(*i..*i + 12)?); *i += 12;
                return sub(w, i, out);
            }
            if tag == 2 {
                *i += 1;
                let (mut l, mut r) = (Vec::new(), Vec::new());
                top(w, i, &mut l)?; top(w, i, &mut r)?;
                let pruned = |x: &[u32]| x.first() == Some(&1);
                if pruned(&l) && !pruned(&r) { out.push(5); out.extend_from_slice(&l[1..9]); out.extend(r); }
                else if pruned(&r) && !pruned(&l) { out.push(6); out.extend_from_slice(&r[1..9]); out.extend(l); }
                else { out.push(2); out.extend(l); out.extend(r); }
                return Some(());
            }
            if tag == 5 || tag == 6 { *i += 1; out.push(tag); out.extend_from_slice(w.get(*i..*i + 8)?); *i += 8; return top(w, i, out); }
            sub(w, i, out)
        }
        let mut out = w[..9].to_vec();
        out[0] = (w[0] == 1) as u32;
        let mut i = 9;
        if top(w, &mut i, &mut out).is_none() || i != w.len() { return alloc::vec![] }
        out
    }

    /// Each kind of redundant encoding: decoded before 11-09 (same meaning), refused now. Root cause: the fact check
    /// hashes the DECODED trie, so bytes the decoder discards or reads leniently were never bound.
    #[test]
    fn non_canonical_witnesses_are_refused() {
        let (st, root, sa, keys) = fixture();
        let h = st.prove_history(&sa);
        assert!(history(&h, &root, &sa).is_ok(), "the honest history verifies");
        // find each unspent coin record (tag 4 ... flag 0) in the history witness
        let coins: Vec<usize> = (0..h.len().saturating_sub(19))
            .filter(|&i| h[i] == 4 && h[i + 9] == 0 && h[i + 10..i + 18].iter().all(|&x| x == 0)).collect();
        assert!(!coins.is_empty(), "the fixture's history has an unspent coin");
        let c = coins[0];
        let mut cases: Vec<(&str, Vec<u32>)> = Vec::new();
        let mut m = h.clone(); m[c + 10] = 0xdead_beef; cases.push(("an unspent coin's spender slot carries data", m));
        let mut m = h.clone(); m[c + 17] = 1; cases.push(("an unspent coin's spender slot, its last word", m));
        let mut m = h.clone(); m[c + 9] = 2; cases.push(("a spent flag of 2", m));
        let mut m = h.clone(); m[c + 9] = 0xffff_ffff; cases.push(("a spent flag of all ones", m));
        // (a counts header other than 1 was never a hole: the old reader read it as "counts off", which facts refuse)
        let mut m = h.clone(); m[0] = 7;
        assert!(history(&m, &root, &sa).is_err(), "a counts header of 7: refused");
        // a SPENT coin's spender words 5-7: the leaf hash overwrites them with the value and the flag
        let spent: Vec<usize> = (0..h.len().saturating_sub(19)).filter(|&i| h[i] == 4 && h[i + 9] == 1).collect();
        assert!(!spent.is_empty(), "the fixture's history has a spent coin");
        for w in [5, 6, 7] {
            let mut m = h.clone(); m[spent[0] + 10 + w] = 0x5a5a_5a5a;
            cases.push((["a spent coin's spender word 5", "a spent coin's spender word 6", "a spent coin's spender word 7"][w - 5], m));
        }
        for (what, m) in &cases {
            assert!(old_reader_accepts(m, |c| history(c, &root, &sa).is_ok()),
                    "{what}: the old reader accepted it with the same answer (the test exercises the class)");
            assert_eq!(history(m, &root, &sa).err(), Some(FactErr::Decode), "{what}: refused");
        }
        // tag 2 with a pruned child, which the encoder writes as one-sided 5: same meaning, refused
        let p = st.prove_coins(&[keys[1]]);
        assert!(coin(&p, &root, &keys[1]).is_ok());
        let i5 = (9..p.len()).find(|&i| p[i] == 5).expect("a one-sided node in the coin proof");
        let mut two = p[..i5].to_vec();
        two.push(2); two.push(1); two.extend_from_slice(&p[i5 + 1..i5 + 9]); two.extend_from_slice(&p[i5 + 9..]);
        assert!(old_reader_accepts(&two, |c| coin(c, &root, &keys[1]).is_ok()), "tag 2 + pruned child: the old reader accepted it");
        assert_eq!(coin(&two, &root, &keys[1]).err(), Some(FactErr::Decode), "tag 2 + pruned child: refused");
    }

    /// The property the fix gives: no single changed bit anywhere in an honest fact verifies (each kind of fact).
    #[test]
    fn every_single_bit_change_to_a_fact_is_refused() {
        let (st, root, sa, keys) = fixture();
        let proofs: Vec<(&str, Vec<u32>)> = alloc::vec![("history", st.prove_history(&sa)), ("balance", st.prove_addr(&sa)),
            ("coin spent", st.prove_coins(&[keys[0]])), ("coin unspent", st.prove_coins(&[keys[1]])),
            ("coin absent", st.prove_coins(&[keys[2]]))];
        let check = |p: &[u32]| -> bool {
            history(p, &root, &sa).is_ok() || balance(p, &root, &sa).is_ok()
                || keys.iter().any(|k| coin(p, &root, k).is_ok())
        };
        for (what, p) in &proofs {
            assert!(check(p), "{what}: the honest proof verifies");
            for i in 0..p.len() {
                for b in [0, 1, 7, 15, 31] {
                    let mut m = p.clone();
                    m[i] ^= 1 << b;
                    assert!(!check(&m), "{what}: word {i} bit {b} flipped and it still verified");
                }
            }
        }
    }
}
