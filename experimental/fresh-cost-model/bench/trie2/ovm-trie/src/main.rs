//! fresh-trie: Hazync guest mode 17 (trie_update) on OpenVM, with measured levers / ablations behind features.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::vec::Vec;
use libzync::{coinlog, ttree};
use openvm_sha2::Sha256;
extern "C" { fn zkvm_sha256_impl(state: *const u8, input: *const u8, output: *mut u8); }
static mut NCOMP: u32 = 0;

#[inline(always)]
unsafe fn compress(st_native: &[u32; 8], b1: *const u32, b2: *const u32, o: &mut [u32; 8]) {
    NCOMP += 1;
    #[cfg(feature = "nohash")]
    { for i in 0..8 { o[i] = st_native[i] ^ *b1.add(i) ^ *b2.add(i); } return; }
    #[allow(unreachable_code)]
    {
        let mut blk = [0u32; 16];
        core::ptr::copy_nonoverlapping(b1, blk.as_mut_ptr(), 8);
        core::ptr::copy_nonoverlapping(b2, blk.as_mut_ptr().add(8), 8);
        zkvm_sha256_impl(st_native.as_ptr() as *const u8, blk.as_ptr() as *const u8, o.as_mut_ptr() as *mut u8);
    }
}
/// risc0 contract (baseline): state words byte-swapped from native, output = digest bytes in memory.
#[no_mangle]
pub unsafe extern "C" fn sys_sha_compress(out: *mut u32, inp: *const u32, b1: *const u32, b2: *const u32) {
    let mut st = [0u32; 8]; for i in 0..8 { st[i] = (*inp.add(i)).swap_bytes(); }
    let mut o = [0u32; 8];
    compress(&st, b1, b2, &mut o);
    for i in 0..8 { *out.add(i) = o[i].swap_bytes(); }
}
/// ovm_iv (same format): IV given in native state words; output still the digest bytes.
#[no_mangle]
pub unsafe extern "C" fn ovm_compress_iv(out: *mut [u32; 8], iv: *const [u32; 8], a: *const [u32; 8], b: *const [u32; 8]) {
    let mut o = [0u32; 8];
    compress(&*iv, a as *const u32, b as *const u32, &mut o);
    let out = &mut *out; for i in 0..8 { out[i] = o[i].swap_bytes(); }
}
/// ovm_nat (NEW format): native state words in, native state words out, no swap.
#[no_mangle]
pub unsafe extern "C" fn ovm_compress_nat(out: *mut [u32; 8], iv: *const [u32; 8], a: *const [u32; 8], b: *const [u32; 8]) {
    compress(&*iv, a as *const u32, b as *const u32, &mut *out);
}

openvm::entry!(main);
fn main() {
    let raw: Vec<u8> = openvm::io::read_vec();
    #[cfg(feature = "zerocopy")]
    let words: &[u32] = { assert!(raw.as_ptr() as usize % 4 == 0 && raw.len() % 4 == 0); unsafe { core::slice::from_raw_parts(raw.as_ptr() as *const u32, raw.len() / 4) } };
    #[cfg(not(feature = "zerocopy"))]
    let words_v: Vec<u32> = raw.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    #[cfg(not(feature = "zerocopy"))]
    let words: &[u32] = &words_v;
    let (nw, nm, ni, np) = (words[0] as usize, words[1] as usize, words[2] as usize, words[3] as usize);
    let mut o = 4;
    let wit = &words[o..o + nw]; o += nw;
    let mw = &words[o..o + nm * 24]; o += nm * 24;
    let iw = &words[o..o + ni * 27]; o += ni * 27;
    let perm = &words[o..o + np];
    let key = |w: &[u32]| -> ttree::Key {
        let mut k = [0u8; 64];
        for j in 0..16 { k[4 * j..4 * j + 4].copy_from_slice(&w[j].to_be_bytes()); }
        k
    };
    let marks: Vec<(ttree::Key, coinlog::W)> = mw.chunks_exact(24).map(|c| (key(&c[..16]), c[16..24].try_into().unwrap())).collect();
    let ins: Vec<(ttree::Key, Option<coinlog::W>, u64)> = iw.chunks_exact(27)
        .map(|c| (key(&c[..16]), (c[16] == 1).then(|| c[17..25].try_into().unwrap()), c[25] as u64 | (c[26] as u64) << 32)).collect();
    assert!(wit.first() == Some(&1), "trie: the witness is not in counts mode");
    let old: coinlog::W = wit.get(1..9).expect("trie: witness header").try_into().unwrap();
    #[cfg(feature = "decodeonly")]
    let new: coinlog::W = { let _ = (perm, &marks, &ins); old };
    #[cfg(not(feature = "decodeonly"))]
    let new = match ttree::apply_stream_perm(wit, &marks, &ins, Some(perm)) {
        Ok(n) => n,
        Err(_e) => { #[cfg(not(feature = "nohash"))] panic!("trie update"); #[allow(unreachable_code)] [0xdead; 8] }
    };
    let jn = ttree::trie_journal(&old, &new, &ttree::list_digest(&marks, &ins));
    let jb: Vec<u8> = jn.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut h = Sha256::new(); h.update(&jb);
    let mut out: [u8; 32] = h.finalize();
    out[28..].copy_from_slice(&unsafe { NCOMP }.to_le_bytes());
    openvm::io::reveal_bytes32(out);
}
