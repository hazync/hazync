//! The coin log: libzync's UTXO commitment when built with `HAZYNC_COINLOG=1` (experimental; design B's SwiftSync
//! forest otherwise). Shared by the host (this crate) and the guest (included by path), so both run this code.
//!
//! Every coin that outlives the block that creates it is APPENDED, in birth order, to a forest of perfect Merkle trees
//! (one per set bit of the leaf count, largest and oldest first, as utreexo's stump). Spending a coin FLIPS its leaf to
//! `spent(leaf, spender)` in place and re-hashes its path. Nothing is moved or deleted, so a coin's position never changes.
//!
//! - leaf: the coin leaf of docs/libzync/UTREEXO_FORMAT.md §2 while unspent; `spent(c, s)` once spent;
//! - `parent(l, L, R)`: one SHA-256 compression of L ‖ R from `IV_NODE[l]` (a tree of 2^l leaves);
//! - `spent(c, s)`: one SHA-256 compression of c ‖ s from `IV_SPENT`, where s = [`spender`]: the spending block's height,
//!   the spending transaction's position in it (coinbase 0) and the input index, so the log proves who spent a coin.
//!
//! A block's update: the spends of coins from earlier blocks as (position, leaf, spender), sorted and strictly increasing, then
//! the appends. The witness is the hash of every untouched subtree met on the way down, in the order `update` reads
//! them; the positions alone fix the shape. An unspent leaf at the claimed position is the only way a touched tree's
//! old root can match: a coin that does not exist, or was spent before, cannot be spent.

use alloc::vec::Vec;

pub type H = [u8; 32];

/// IV_NODE[l] = SHA-256("libzync-coinlog-node" ‖ l as u8), as SHA-256 state words (big-endian). Generated; checked by the test `ivs`.
pub const IV_NODE: [[u32; 8]; 64] = [
    [0xe2e87bb0, 0x6b5520ee, 0xd2ab3fde, 0xbf3f2e71, 0x5571f7ff, 0xc1d6b542, 0x7b709e48, 0xc5ae7b8c],
    [0xb8e09545, 0xdf578bde, 0x5f94d7af, 0x743663e7, 0x05ab9a31, 0x01e03dda, 0xba870a2f, 0x8c222b73],
    [0xfd39b580, 0x759ddfa9, 0x560fb275, 0xf7d282c4, 0xbb39be9a, 0x6bf7ff22, 0x21697488, 0x0faf48ae],
    [0x459fd037, 0x2fdc3eb7, 0xb0c07987, 0xea6042a3, 0xbbe7a969, 0xca00fbbb, 0x22022233, 0x73d2be50],
    [0x2d8fd9e5, 0xa6e30b02, 0xdbd663f9, 0xa746cd3a, 0x1a5f2b52, 0xf0762d5b, 0xc3c35d32, 0xe347bd78],
    [0x62a0804c, 0x10b8876f, 0xcf071d2d, 0x2c0f89b4, 0x0f7a9b1f, 0x1b4e895f, 0x6bf9924e, 0xc3eed682],
    [0x9c5558fd, 0x38076df8, 0x9590068a, 0xbd2735bd, 0xc2333c3e, 0x6756f18d, 0x34dbd541, 0xc0e5adae],
    [0xd5af9212, 0x297f49c2, 0x91def2a5, 0x246b84eb, 0xd7c6fb88, 0x34a929f2, 0xe13b541e, 0x8358dfbd],
    [0xd7f90fc3, 0x6fad2056, 0x670d7515, 0x0545e2a4, 0x295aeab3, 0xaaac60ce, 0xdef8f1b1, 0xadeeed8e],
    [0x9633716b, 0xd1cfd7d3, 0xe2a85785, 0x9dcaec41, 0x15e0cb4b, 0xe05dde81, 0xe250cdf0, 0x9e07f00d],
    [0x7769df06, 0xd24f6525, 0x904cf59f, 0x59376be2, 0x638bc28a, 0x974668e4, 0x0a7dc74b, 0x324c813c],
    [0x683cced0, 0xd333490f, 0xda020e2d, 0x3abba9ed, 0xe65b7300, 0xbe817689, 0xafcac1cf, 0x78a40b9e],
    [0xdee9c625, 0x936fab4c, 0x280d007e, 0x2f91a9b0, 0x25a7f7fa, 0x3e39669b, 0xd31114fe, 0x8057ef64],
    [0x5f0c9dc7, 0x128b865a, 0xebb0348e, 0xea62a6dd, 0x5d37296d, 0xbf157420, 0xc70969e7, 0x4a6c797a],
    [0xf3831eb1, 0x77c385a8, 0x1ec1fe87, 0x6fc36475, 0xe02ee50c, 0x87a38791, 0xd06e2e38, 0x5049ff52],
    [0x4b8e5ed1, 0x0bcdeb0e, 0xc7b0b461, 0xc2d5dc92, 0x7cab2e83, 0x15a2f0d5, 0x258fa0d5, 0xcaa279dc],
    [0x18338288, 0x949e7901, 0xa2f955cf, 0xa3b4424f, 0xee35e3aa, 0x3e7048c5, 0x79bc696d, 0x1aa8ab59],
    [0x28ad2b33, 0xbcb43b27, 0xf9bc813b, 0x8c3ae1a2, 0x8b1ef1d5, 0x955f65c2, 0x6db5ee7e, 0xe96926b1],
    [0x6d8d4164, 0x2bf07c93, 0xc08dc2dd, 0x157670e7, 0x72e747b6, 0xa04c409d, 0x69bdf96d, 0x39daeb2d],
    [0x7bf933f4, 0x5aec7def, 0x3dd45765, 0xc925f76d, 0xa3d7e792, 0x5a422e48, 0x1c9cdf41, 0xd07eeed4],
    [0x709c33da, 0xe2a5500a, 0xeff77a8a, 0xa7cbf33e, 0xbf083af0, 0x56e0b4c6, 0x20225411, 0xc1603aa2],
    [0xc717c254, 0x2643b9e5, 0x208503c0, 0x5b28b53f, 0xd9a4763c, 0x1296d160, 0x058fe3fb, 0x4ec387a9],
    [0x94ddeffd, 0x3c553134, 0x3fd6dba4, 0x11c5fe6d, 0xab402070, 0x699d6f07, 0x3d4d5a82, 0x934d2b02],
    [0x75c8d0fa, 0xb0e64c8e, 0xb227bc15, 0x149f8939, 0xfda945d8, 0x2fb6c676, 0x381d7c84, 0x9cd055fe],
    [0x5d391e27, 0xe1dc52f9, 0x95dbac94, 0x37b6b3d2, 0xf6f04c73, 0xb8b955c1, 0x0063a248, 0xd4e7046a],
    [0x09f89b61, 0xf267de7e, 0xe0a516e2, 0x75a51a5c, 0xfe2b03b5, 0x87a18944, 0x032b6ba6, 0xcb4713c6],
    [0x66db32fc, 0xccff270c, 0xd8943b68, 0x318f8723, 0x0f443daf, 0xeaf8970f, 0x13cc7412, 0xb8e39d94],
    [0xd37d2f71, 0xae9086e3, 0x208f2f5c, 0x354857e8, 0x19e5820a, 0x219cbc0c, 0x0be5a41d, 0xc9a62b98],
    [0x5ecf8a0b, 0x1321c3f8, 0x2fab8db4, 0xeac00be8, 0x7a6e5287, 0x5d6721ac, 0xe4b9604d, 0x166be162],
    [0xa73f9fa4, 0x402ec8dd, 0x4a0ed6d2, 0x8884b000, 0x93bc2796, 0xae27ab7d, 0xc5051b11, 0x03641887],
    [0x2c1e06a2, 0x70701e69, 0x642b93fa, 0x5d501ed2, 0x6db60049, 0xe53debb2, 0x0ca7df4b, 0x66b8d11b],
    [0xa3d6a06b, 0xb4b0127e, 0x811c1db2, 0x9162311b, 0x7d564c60, 0x48570f3d, 0x4b1114c7, 0x25f46d2f],
    [0x479c4a12, 0xfaeeeefa, 0x88ab777f, 0xb456c7ef, 0x8d5d7ef4, 0x56d22bec, 0xd6134837, 0x07f19b75],
    [0x02a0b15e, 0xe6781cb3, 0x49dc3082, 0x06f82b7c, 0x76082aeb, 0x10e4d960, 0xc15dd649, 0x5e4ad306],
    [0x51b2f2c3, 0x73756858, 0xeeed6dd8, 0x06a976a7, 0x4fb39451, 0x3c9619e6, 0x23c7cc22, 0xfba6fc88],
    [0xeb277a26, 0x88a25106, 0x224afc1d, 0x18b2586c, 0x83fc7c81, 0x777f7ede, 0xf9dc58c2, 0x5262ebe2],
    [0x4bac6a74, 0x110d5321, 0xa87c9b5d, 0x0151c510, 0x153f387e, 0x2393f46b, 0x729dfec5, 0x409d65ca],
    [0x33ba7e5f, 0x0c1a5ad9, 0xfbbcd1f2, 0xebf66a08, 0x5264aac6, 0x2d6f5410, 0xf7ef7739, 0xb1009af9],
    [0x0312655f, 0xdf0a8b71, 0x87f5f50b, 0xd93f2fd5, 0xc68e359d, 0x4d55b4ee, 0x4a15831d, 0x8c4585d8],
    [0xcf4f2aef, 0xb58cbeff, 0x777e3b2d, 0x7af35ff7, 0x6317ee3a, 0x7c9d07a5, 0x5858ce45, 0x7fb23623],
    [0x1526b74c, 0xae87c589, 0x6cf3903c, 0x3cddac85, 0x361106cf, 0x4c2c3433, 0xccaf79e1, 0x7bcbd9ac],
    [0x6e9c52fe, 0x2e6d5ef7, 0x1385badc, 0x54f0546f, 0x988f6a7a, 0x1f074a57, 0xf41c4b3a, 0x186fe570],
    [0x87b5897c, 0x933844a5, 0x331ba343, 0x4bc661bf, 0x156d762f, 0x761c11f3, 0xd3e3a8ff, 0xc708841b],
    [0xd5613bda, 0xf667eceb, 0x6997ca3e, 0x682a730c, 0x29ed1826, 0x10fa01f3, 0xeb3d5df2, 0x8d403874],
    [0xf68a9dcb, 0x8a32e28a, 0x7f162baa, 0xe31d5ca5, 0x5f8c401e, 0x6c84d691, 0xd30b7a7a, 0xf7efba44],
    [0x14e476a2, 0xc568fa83, 0xf9a13eec, 0x567563da, 0x656c2540, 0xd9aaf432, 0xc1e716e4, 0x085ec8c6],
    [0x8e78b796, 0x8c33b566, 0x68862ddc, 0x8567a432, 0xb8ab8878, 0x9d0ffc9b, 0x13076d79, 0xd22f88a0],
    [0xf94a8a97, 0xca9207c1, 0xeb971b2e, 0x094db42d, 0xe1da5ce7, 0x7966600e, 0xa11cc121, 0x553f2167],
    [0x913d8a78, 0x51b73b02, 0x694ee3a2, 0xbbafdaad, 0x4c2441ee, 0x65409657, 0x6d47fe4b, 0xa6dd23f6],
    [0xf7ea0d08, 0xe77fa08d, 0xd1d6680d, 0x5bdb28cf, 0x5f4e2da9, 0x44949b4b, 0x1fdc7216, 0xc53b5564],
    [0x90a0d47b, 0x2f7d6afb, 0x6948cc02, 0x116cc8e5, 0xa09c740e, 0xc5b685f3, 0xedcf6e49, 0xfa756fff],
    [0x665a3fb5, 0x0775d964, 0x61ef0d75, 0xdac37465, 0x2de2661d, 0x1ff8bc4f, 0x8563b295, 0xbb1be777],
    [0xf3e2e9b6, 0xb60e1d55, 0x19e68a10, 0xcd81cc26, 0x958e14c2, 0x8db4f648, 0x7114efd4, 0x27a486d2],
    [0x70074702, 0xfdd7de21, 0x7efa2502, 0xd2a7fe1c, 0x1561adef, 0x293350b2, 0x5449bd6c, 0x4fcaf981],
    [0x576278f9, 0x29338604, 0x956d3fa4, 0xc1a3e306, 0xf04676d0, 0x89b4c772, 0xf07349dc, 0xcf2a86f0],
    [0x5b718525, 0x7edd2e03, 0x79a6f4c8, 0x5611863f, 0x791baa12, 0x9109708b, 0xc234a55c, 0xbe5cd4c2],
    [0x3f53be82, 0x565facb4, 0x62323669, 0x0020f0f7, 0x1606bc25, 0xb6712978, 0xca32b941, 0x69a26f9a],
    [0xf32b69d4, 0x2231ce9e, 0x020e617f, 0x8c8efa92, 0xe210e983, 0x7be286d2, 0xaa6a10f4, 0x426fc494],
    [0xb2e00522, 0x4a072009, 0x3bab39fd, 0x3b161e11, 0x4724daad, 0xc5d95cdb, 0xa27910cc, 0x07468ee6],
    [0xe455cea9, 0x6346abfe, 0x5d6b2acb, 0xb9871126, 0xf01503fc, 0x20494033, 0x853a9e2d, 0xd2c3e7d5],
    [0x6affe95f, 0x2fb31781, 0xc2563e64, 0x2b1cfec8, 0x6f6414c5, 0x9e98603c, 0x34f9c568, 0x7cc79deb],
    [0x5a727530, 0xe944ce7f, 0x8c27add0, 0x1cb40a41, 0x5dfb3052, 0x77d92726, 0x940d1f52, 0x6370a761],
    [0x5c8f19ca, 0x253d29c2, 0x76c1b787, 0x477ba5ae, 0x46f6b01d, 0x49fe72e5, 0x9e745b29, 0xce9d6738],
    [0x0ca750e5, 0x6a2d40df, 0x2355ca92, 0x2bd340f7, 0x453f2fdf, 0x797eef63, 0xc66b2e69, 0x23915cef],
];
/// IV_SPENT = SHA-256("libzync-coinlog-spent").
pub const IV_SPENT: [u32; 8] = [0x1c7d285c, 0x279c8667, 0xd7e930f4, 0x44bb4ba1, 0x521dbd9e, 0x0cbb90e9, 0xd8393e2f, 0x468d506b];

/// A hash as the eight little-endian words it occupies in memory: the form risc0's SHA accelerator reads and writes.
/// The update works on these throughout, so nothing is converted per node.
pub type W = [u32; 8];

pub fn w_of(h: &H) -> W {
    core::array::from_fn(|i| u32::from_le_bytes([h[4 * i], h[4 * i + 1], h[4 * i + 2], h[4 * i + 3]]))
}

pub fn h_of(w: &W) -> H {
    let mut o = [0u8; 32];
    for (i, x) in w.iter().enumerate() {
        o[4 * i..4 * i + 4].copy_from_slice(&x.to_le_bytes());
    }
    o
}

#[allow(dead_code)]
const fn swapped<const N: usize>(t: &[[u32; 8]; N]) -> [[u32; 8]; N] {
    let mut o = [[0u32; 8]; N];
    let mut i = 0;
    while i < N {
        let mut j = 0;
        while j < 8 {
            o[i][j] = t[i][j].swap_bytes();
            j += 1;
        }
        i += 1;
    }
    o
}
/// The IVs as the accelerator's state words (the state's big-endian bytes in memory order).
#[cfg(target_os = "zkvm")]
static IV_NODE_W: [[u32; 8]; 64] = swapped(&IV_NODE);
#[cfg(target_os = "zkvm")]
static IV_SPENT_W: [u32; 8] = swapped(&[IV_SPENT])[0];

// In the zkVM: one accelerated compression on words, written straight into `out` (risc0's sys_sha_compress, exported by
// the guest runtime). Outside the zkVM: sha2's compress256 on the same bytes. The host's roots are checked against the
// guest's after every block, so the two cannot drift apart unnoticed.
#[cfg(target_os = "zkvm")]
extern "C" {
    fn sys_sha_compress(out: *mut [u32; 8], in_state: *const [u32; 8], b1: *const [u32; 8], b2: *const [u32; 8]);
}

#[cfg(target_os = "zkvm")]
#[inline(always)]
fn compress_w(iv: &'static [u32; 8], a: &W, b: &W, out: &mut W) {
    unsafe { sys_sha_compress(out, iv, a, b) };
}

#[cfg(not(target_os = "zkvm"))]
fn compress_w(iv: &[u32; 8], a: &W, b: &W, out: &mut W) {
    let mut st = *iv;
    let mut blk = [0u8; 64];
    blk[..32].copy_from_slice(&h_of(a));
    blk[32..].copy_from_slice(&h_of(b));
    sha2::compress256(&mut st, &[blk.into()]);
    for (o, x) in out.iter_mut().zip(st.iter()) {
        *o = x.swap_bytes();
    }
}

#[inline(always)]
fn parent_w(l: u32, a: &W, b: &W, out: &mut W) {
    #[cfg(target_os = "zkvm")]
    compress_w(&IV_NODE_W[l as usize], a, b, out);
    #[cfg(not(target_os = "zkvm"))]
    compress_w(&IV_NODE[l as usize], a, b, out);
}

#[inline(always)]
fn spent_w(c: &W, s: &W, out: &mut W) {
    #[cfg(target_os = "zkvm")]
    compress_w(&IV_SPENT_W, c, s, out);
    #[cfg(not(target_os = "zkvm"))]
    compress_w(&IV_SPENT, c, s, out);
}

/// The spender record: height u32 LE ‖ transaction position in the block u32 LE ‖ input index u32 LE ‖ 20 zero bytes.
pub fn spender(height: u32, tx: u32, vin: u32) -> H {
    let mut s = [0u8; 32];
    s[0..4].copy_from_slice(&height.to_le_bytes());
    s[4..8].copy_from_slice(&tx.to_le_bytes());
    s[8..12].copy_from_slice(&vin.to_le_bytes());
    s
}

pub fn parent(l: u32, a: &H, b: &H) -> H {
    let mut o = [0u32; 8];
    parent_w(l, &w_of(a), &w_of(b), &mut o);
    h_of(&o)
}

pub fn spent(c: &H, s: &H) -> H {
    let mut o = [0u32; 8];
    spent_w(&w_of(c), &w_of(s), &mut o);
    h_of(&o)
}

/// One spend of an earlier coin: its position in the log, its unspent leaf and its spender record, as words.
pub type Spend = (u64, W, W);

#[derive(Debug, PartialEq, Eq)]
pub enum LogError {
    /// positions not strictly increasing (the same coin twice, or out of order)
    Order,
    /// a position at or past the end of the log
    PastEnd,
    /// a touched tree's old root does not match: the coin is not unspent at that position
    OldRoot,
    /// the roots do not have the shape the leaf count forces
    Shape,
    /// the witness ran out, or was longer than the update used
    Witness,
}

/// The old and new hash of the perfect tree of 2^l leaves starting at `start`, with the spends `e` inside it, written
/// into `old` / `new` (no tuples returned by value: in the guest those were memcpy calls). False: the witness ran out.
pub fn update(l: u32, start: u64, e: &[Spend], sib: &mut core::slice::Iter<W>, old: &mut W, new: &mut W) -> bool {
    if e.is_empty() {
        let Some(h) = sib.next() else { return false };
        *old = *h;
        *new = *h;
        return true;
    }
    if l == 0 {
        *old = e[0].1;
        spent_w(&e[0].1, &e[0].2, new);
        return true;
    }
    let mid = start + (1u64 << (l - 1));
    let m = e.partition_point(|x| x.0 < mid);
    let (mut a0, mut a1, mut b0, mut b1) = ([0u32; 8], [0u32; 8], [0u32; 8], [0u32; 8]);
    if !update(l - 1, start, &e[..m], sib, &mut a0, &mut a1) || !update(l - 1, mid, &e[m..], sib, &mut b0, &mut b1) {
        return false;
    }
    parent_w(l, &a0, &b0, old);
    parent_w(l, &a1, &b1, new);
    true
}

/// The roots have a tree exactly at each set bit of `n`.
pub fn shape_ok(roots: &[Option<H>], n: u64) -> bool {
    (0..64).all(|l| ((n >> l) & 1 == 1) == roots.get(l).copied().flatten().is_some()) && roots.len() <= 64
}

/// One block: `spends` (position, unspent leaf, spender), positions strictly increasing, against `roots`; then `appends`.
/// `roots` / `n` are updated only on success.
pub fn apply(roots: &mut Vec<Option<H>>, n: &mut u64, spends: &[Spend], appends: &[H], siblings: &[W]) -> Result<(), LogError> {
    if !shape_ok(roots, *n) {
        return Err(LogError::Shape);
    }
    if spends.windows(2).any(|p| p[0].0 >= p[1].0) {
        return Err(LogError::Order);
    }
    if spends.last().is_some_and(|x| x.0 >= *n) {
        return Err(LogError::PastEnd);
    }
    let mut r: Vec<Option<W>> = roots.iter().map(|x| x.as_ref().map(w_of)).collect();
    let mut sib = siblings.iter();
    let (mut start, mut k) = (0u64, 0usize);
    let (mut o, mut nw) = ([0u32; 8], [0u32; 8]);
    for l in (0..r.len()).rev() {
        let Some(root) = r[l] else { continue };
        let end = start + (1u64 << l);
        let m = k + spends[k..].partition_point(|x| x.0 < end);
        if m > k {
            if !update(l as u32, start, &spends[k..m], &mut sib, &mut o, &mut nw) {
                return Err(LogError::Witness);
            }
            if o != root {
                return Err(LogError::OldRoot);
            }
            r[l] = Some(nw);
        }
        k = m;
        start = end;
    }
    if sib.next().is_some() {
        return Err(LogError::Witness);
    }
    let mut c = *n;
    for leaf in appends {
        let (mut node, mut l) = (w_of(leaf), 0usize);
        while l < r.len() && r[l].is_some() {
            let left = r[l].take().unwrap();
            let mut up = [0u32; 8];
            parent_w(l as u32 + 1, &left, &node, &mut up);
            node = up;
            l += 1;
        }
        if l == r.len() {
            r.push(None);
        }
        r[l] = Some(node);
        c += 1;
    }
    while r.last() == Some(&None) {
        r.pop();
    }
    *roots = r.iter().map(|x| x.as_ref().map(h_of)).collect();
    *n = c;
    Ok(())
}

/// The witness a block's aggregate reads after its hint block: [k, positions as (lo, hi) words × k, 8 words per sibling].
/// Position i belongs to the i-th spend of an earlier coin in block order; the guest sorts them.
pub fn encode_witness(positions: &[u64], siblings: &[H]) -> Vec<u32> {
    let mut v = Vec::with_capacity(1 + 2 * positions.len() + 8 * siblings.len());
    v.push(positions.len() as u32);
    for p in positions {
        v.push(*p as u32);
        v.push((*p >> 32) as u32);
    }
    for s in siblings {
        for c in s.chunks_exact(4) {
            v.push(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        }
    }
    v
}

pub fn decode_witness(w: &[u32]) -> Option<(Vec<u64>, Vec<W>)> {
    let k = *w.first()? as usize;
    let rest = w.get(1 + 2 * k..)?;
    if rest.len() % 8 != 0 {
        return None;
    }
    let pos = (0..k).map(|i| w[1 + 2 * i] as u64 | (w[2 + 2 * i] as u64) << 32).collect();
    let sib = rest.chunks_exact(8).map(|c| c.try_into().unwrap()).collect();
    Some((pos, sib))
}

/// Host side: the whole log, every node kept, so any block's witness can be read off. Nodes at level l are aligned:
/// node i covers leaves [i·2^l, (i+1)·2^l), present once that whole range exists.
#[derive(Clone, Default)]
pub struct Store {
    pub nodes: Vec<Vec<H>>,
}

impl Store {
    pub fn len(&self) -> u64 {
        self.nodes.first().map(|v| v.len() as u64).unwrap_or(0)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn leaf(&self, p: u64) -> H {
        self.nodes[0][p as usize]
    }
    pub fn push(&mut self, leaf: H) {
        if self.nodes.is_empty() {
            self.nodes.push(Vec::new());
        }
        self.nodes[0].push(leaf);
        let mut l = 0;
        while self.nodes[l].len() % 2 == 0 {
            let n = self.nodes[l].len();
            let p = parent(l as u32 + 1, &self.nodes[l][n - 2], &self.nodes[l][n - 1]);
            if self.nodes.len() == l + 1 {
                self.nodes.push(Vec::new());
            }
            self.nodes[l + 1].push(p);
            l += 1;
        }
    }
    pub fn set(&mut self, p: u64, leaf: H) {
        self.nodes[0][p as usize] = leaf;
        let mut i = p as usize;
        for l in 0..self.nodes.len() - 1 {
            let j = i / 2;
            if j >= self.nodes[l + 1].len() {
                break;
            }
            self.nodes[l + 1][j] = parent(l as u32 + 1, &self.nodes[l][2 * j], &self.nodes[l][2 * j + 1]);
            i = j;
        }
    }
    /// The forest's roots (index = level), as the ChainState carries them.
    pub fn roots(&self) -> Vec<Option<H>> {
        let n = self.len();
        let mut r: Vec<Option<H>> = (0..64).map(|_| None).collect();
        let mut start = 0u64;
        for l in (0..64).rev() {
            if (n >> l) & 1 == 1 {
                r[l] = Some(self.nodes[l][(start >> l) as usize]);
                start += 1u64 << l;
            }
        }
        while r.last() == Some(&None) {
            r.pop();
        }
        r
    }
    fn wit(&self, l: u32, start: u64, e: &[(u64, H)], out: &mut Vec<H>) {
        if e.is_empty() {
            out.push(self.nodes[l as usize][(start >> l) as usize]);
            return;
        }
        if l == 0 {
            return;
        }
        let mid = start + (1u64 << (l - 1));
        let m = e.partition_point(|x| x.0 < mid);
        self.wit(l - 1, start, &e[..m], out);
        self.wit(l - 1, mid, &e[m..], out);
    }
    /// The siblings `apply` reads for these spends (sorted by position).
    pub fn witness(&self, spends: &[(u64, H)]) -> Vec<H> {
        let n = self.len();
        let mut out = Vec::new();
        let (mut start, mut k) = (0u64, 0usize);
        for l in (0..64u32).rev() {
            if (n >> l) & 1 == 0 {
                continue;
            }
            let end = start + (1u64 << l);
            let m = k + spends[k..].partition_point(|x| x.0 < end);
            if m > k {
                self.wit(l, start, &spends[k..m], &mut out);
            }
            k = m;
            start = end;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn ivs() {
        for l in 0..64u8 {
            let d = Sha256::digest([&b"libzync-coinlog-node"[..], &[l]].concat());
            let w: Vec<u32> = d.chunks_exact(4).map(|c| u32::from_be_bytes(c.try_into().unwrap())).collect();
            assert_eq!(&w[..], &IV_NODE[l as usize][..]);
        }
        let d = Sha256::digest(b"libzync-coinlog-spent");
        let w: Vec<u32> = d.chunks_exact(4).map(|c| u32::from_be_bytes(c.try_into().unwrap())).collect();
        assert_eq!(&w[..], &IV_SPENT[..]);
    }

    fn ws(sp: &[(u64, H)]) -> Vec<Spend> {
        sp.iter().map(|(p, h)| (*p, w_of(h), w_of(&spender(7, *p as u32, 1)))).collect()
    }
    fn wv(v: &[H]) -> Vec<W> {
        v.iter().map(w_of).collect()
    }

    fn h(a: u64, b: u64) -> H {
        Sha256::digest([a.to_le_bytes(), b.to_le_bytes()].concat()).into()
    }

    #[test]
    fn apply_matches_the_store() {
        // Grow a log block by block; each block spends some unspent coins and appends some. The guest-side `apply`
        // over the store's witness must give the store's own roots after every block.
        for t in 0..40u64 {
            let mut st = Store::default();
            let (mut roots, mut n): (Vec<Option<H>>, u64) = (Vec::new(), 0);
            let mut live: Vec<u64> = Vec::new();
            for b in 0..30u64 {
                let mut sp: Vec<(u64, H)> = live.iter().filter(|p| h(t * 1000 + b, **p)[0] < 50).map(|p| (*p, st.leaf(*p))).collect();
                sp.sort();
                let wit = st.witness(&sp);
                let app: Vec<H> = (0..(h(t, b)[0] % 23) as u64).map(|i| h(t * 7 + b, i + 1_000_000)).collect();
                apply(&mut roots, &mut n, &ws(&sp), &app, &wv(&wit)).unwrap();
                for (p, c) in &sp {
                    st.set(*p, spent(c, &spender(7, *p as u32, 1)));
                    live.retain(|x| x != p);
                }
                for a in &app {
                    live.push(st.len());
                    st.push(*a);
                }
                assert_eq!(roots, st.roots(), "trial {t} block {b}");
                assert_eq!(n, st.len());
            }
        }
    }

    #[test]
    fn refusals() {
        let mut st = Store::default();
        for i in 0..20u64 {
            st.push(h(9, i));
        }
        let (r0, n0) = (st.roots(), st.len());
        let one = |sp: &[(u64, H)], wit: &[H]| {
            let (mut r, mut n) = (r0.clone(), n0);
            apply(&mut r, &mut n, &ws(sp), &[], &wv(wit))
        };
        let sp = [(5u64, st.leaf(5))];
        assert!(one(&sp, &st.witness(&sp)).is_ok());
        // a coin that is not there
        let fake = [(5u64, h(1, 1))];
        assert_eq!(one(&fake, &st.witness(&fake)), Err(LogError::OldRoot));
        // the right coin at the wrong position
        let wrong = [(6u64, st.leaf(5))];
        assert_eq!(one(&wrong, &st.witness(&wrong)), Err(LogError::OldRoot));
        // the same coin twice
        let twice = [(5u64, st.leaf(5)), (5u64, st.leaf(5))];
        assert_eq!(one(&twice, &[]), Err(LogError::Order));
        // past the end
        let past = [(20u64, h(2, 2))];
        assert_eq!(one(&past, &[]), Err(LogError::PastEnd));
        // a coin already spent
        let mut st2 = st.clone();
        st2.set(5, spent(&st.leaf(5), &spender(7, 5, 1)));
        let (mut r, mut n) = (st2.roots(), st2.len());
        assert_eq!(apply(&mut r, &mut n, &ws(&sp), &[], &wv(&st2.witness(&sp))), Err(LogError::OldRoot));
        // a short or long witness
        let mut w = st.witness(&sp);
        w.pop();
        assert_eq!(one(&sp, &w), Err(LogError::Witness));
        let mut w = st.witness(&sp);
        w.push([0u8; 32]);
        assert_eq!(one(&sp, &w), Err(LogError::Witness));
        // the witness codec round-trips
        let enc = encode_witness(&[3, 1 << 40], &[h(1, 2), h(3, 4)]);
        assert_eq!(decode_witness(&enc), Some((alloc::vec![3, 1 << 40], alloc::vec![w_of(&h(1, 2)), w_of(&h(3, 4))])));
        // the spender is part of the new root: the same spend by another input gives another root
        let (mut r1, mut n1) = (r0.clone(), n0);
        apply(&mut r1, &mut n1, &ws(&sp), &[], &wv(&st.witness(&sp))).unwrap();
        let (mut r2, mut n2) = (r0.clone(), n0);
        let other = [(5u64, w_of(&st.leaf(5)), w_of(&spender(7, 5, 2)))];
        apply(&mut r2, &mut n2, &other, &[], &wv(&st.witness(&sp))).unwrap();
        assert_ne!(r1, r2);
        assert_eq!(spender(0x01020304, 5, 6)[..12], [4, 3, 2, 1, 5, 0, 0, 0, 6, 0, 0, 0]);
        // against scripts/libzync/coinlog_ref.py, which shares no code with this file
        let x: H = Sha256::digest(b"c").into();
        let y: H = Sha256::digest(b"d").into();
        let hx = |h: H| h.iter().map(|b| alloc::format!("{b:02x}")).collect::<alloc::string::String>();
        assert_eq!(hx(spent(&x, &spender(974000, 5, 1))), "cdcc3dda5f2c80be4780225cde93b2442e7d270052ab938d73be4107b388993e");
        assert_eq!(hx(parent(3, &x, &y)), "fcd6a12230b8fc914334039dfe3be49a9796fe16193c15e05de61a536c29f14f");
        // words and bytes round-trip
        assert_eq!(h_of(&w_of(&h(5, 6))), h(5, 6));
    }
}
