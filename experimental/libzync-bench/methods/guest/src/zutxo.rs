//! Zutxo prototype: Bitcoin Core's chainstate (coins keyed by outpoint) as a compact Merkle tree, updated once per
//! block. Shared, byte for byte, by the bench guest and the bench host (`#[path]`), so the guest's result and the
//! host's reference come from the same code. ⛔ EXPERIMENT ONLY, not consensus code.
//!
//! Tree (the root depends only on the SET of coins, never on order or history):
//! - a coin's position is `key = SHA-256(txid ‖ vout_le)`, read as 256 bits, most significant first;
//! - hash(S, d) = 0^32 if S is empty; the coin's leaf hash if |S| = 1; if every coin in S has the same bit d,
//!   hash(S, d + 1) (no node for a one-sided split); otherwise node_hash(d, hash(L), hash(R)): one SHA-256
//!   compression of hash(L) ‖ hash(R) from a per-depth starting state, so a proof cannot place a node at another depth.
//!
//! Batch update: `entries` are the block's touched coins sorted by key, each with its old leaf (zero = absent) and
//! new leaf (zero = absent). The witness describes everything untouched, in the order `node` reads it:
//! - at a subtree with no touched coin: a SUMMARY, (hash, class) with class = min(number of coins, 2);
//! - at a subtree with touched coins: tag 1 = split here and recurse; tag 0 = the untouched coins here are at most
//!   one, given explicitly as (count, [key, leaf]).

use sha2::{Digest, Sha256};

pub type H = [u8; 32];
pub const ZERO: H = [0u8; 32];

#[derive(Clone, Copy)]
pub struct Entry {
    pub key: H,
    pub old: H,
    pub new: H,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sum {
    pub h: H,
    pub c: u8, // 0 empty, 1 one coin, 2 two or more
}
pub const EMPTY: Sum = Sum { h: ZERO, c: 0 };

pub fn key_of(outpoint: &[u8]) -> H {
    Sha256::digest(outpoint).into()
}

pub fn bit(k: &H, d: u32) -> bool {
    (k[(d / 8) as usize] >> (7 - d % 8)) & 1 == 1
}

pub static mut HASHES: u64 = 0;

static mut IVS: Option<Vec<risc0_zkvm::sha::Digest>> = None;

/// The node hash is ONE SHA-256 compression of the 64-byte block L ‖ R, from a starting state that depends on the
/// split depth: IV_d = SHA-256("zutxo-node" ‖ d as u16 LE), used as the chaining value (as BIP340 tagged hashes use a
/// midstate). One accelerated compression in the guest instead of a padded two-block hash.
pub fn node_hash(d: u32, l: &H, r: &H) -> H {
    use risc0_zkvm::sha::{Digest as D, Impl, Sha256 as _};
    unsafe { HASHES += 1 };
    let ivs = unsafe {
        #[allow(static_mut_refs)]
        IVS.get_or_insert_with(|| (0..=256u16).map(|d| {
            let h: H = Sha256::digest([&b"zutxo-node"[..], &d.to_le_bytes()].concat()).into();
            D::from(h)
        }).collect())
    };
    let out = Impl::compress(&ivs[d as usize], &D::from(*l), &D::from(*r));
    let mut o = [0u8; 32];
    o.copy_from_slice(out.as_bytes());
    o
}

/// Two sibling subtrees at split depth d.
pub fn comb(d: u32, a: Sum, b: Sum) -> Sum {
    if a.c == 0 {
        return b;
    }
    if b.c == 0 {
        return a;
    }
    Sum { h: node_hash(d, &a.h, &b.h), c: 2 }
}

/// The hash of an explicit set of (key, leaf), sorted by key, below depth d.
pub fn mini(d: u32, s: &[(H, H)]) -> Sum {
    match s.len() {
        0 => EMPTY,
        1 => Sum { h: s[0].1, c: 1 },
        _ => {
            if d >= 256 {
                panic!("two coins with one key");
            }
            let m = s.partition_point(|e| !bit(&e.0, d));
            comb(d, mini(d + 1, &s[..m]), mini(d + 1, &s[m..]))
        }
    }
}

pub struct Rd<'a> {
    pub b: &'a [u8],
    pub i: usize,
}
impl<'a> Rd<'a> {
    pub fn u8(&mut self) -> u8 {
        self.i += 1;
        self.b[self.i - 1]
    }
    pub fn h(&mut self) -> H {
        self.i += 32;
        self.b[self.i - 32..self.i].try_into().unwrap()
    }
}

/// (old, new) summaries of the subtree at depth d holding `entries`.
pub fn node(d: u32, e: &[Entry], w: &mut Rd) -> (Sum, Sum) {
    if e.is_empty() {
        let h = w.h();
        let c = w.u8();
        let s = Sum { h, c };
        return (s, s);
    }
    if w.u8() == 1 {
        let m = e.partition_point(|x| !bit(&x.key, d));
        let (lo, hi) = (node(d + 1, &e[..m], w), node(d + 1, &e[m..], w));
        return (comb(d, lo.0, hi.0), comb(d, lo.1, hi.1));
    }
    let mut un: Option<(H, H)> = None;
    if w.u8() == 1 {
        un = Some((w.h(), w.h()));
    }
    let set = |pick: fn(&Entry) -> H| {
        let mut v: Vec<(H, H)> = e.iter().filter(|x| pick(x) != ZERO).map(|x| (x.key, pick(x))).collect();
        if let Some(u) = un {
            v.push(u);
        }
        v.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        mini(d, &v)
    };
    (set(|x| x.old), set(|x| x.new))
}
