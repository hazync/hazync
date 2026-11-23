//! Hazync guest mode 17 (trie_update, prover/methods/guest/src/main.rs) on OpenVM, body unchanged; libzync's zkVM path
//! calls risc0's sys_sha_compress, supplied here on OpenVM's SHA-256 extension. Input: the mode-17 words (trieprep).
//! Reveals SHA-256 of the journal words (little-endian bytes), as trieprep prints it.
extern crate alloc;
use alloc::vec::Vec;
use libzync::{coinlog, ttree};
use openvm_sha2::Sha256;
openvm::init!();
extern "C" { fn zkvm_sha256_impl(state: *const u8, input: *const u8, output: *mut u8); }
#[no_mangle]
pub unsafe extern "C" fn sys_sha_compress(out: *mut u32, inp: *const u32, b1: *const u32, b2: *const u32) {
    let mut st = [0u32; 8]; for i in 0..8 { st[i] = (*inp.add(i)).swap_bytes(); }
    let mut blk = [0u8; 64];
    core::ptr::copy_nonoverlapping(b1 as *const u8, blk.as_mut_ptr(), 32);
    core::ptr::copy_nonoverlapping(b2 as *const u8, blk.as_mut_ptr().add(32), 32);
    let mut o = [0u32; 8];
    zkvm_sha256_impl(st.as_ptr() as *const u8, blk.as_ptr(), o.as_mut_ptr() as *mut u8);
    for i in 0..8 { *out.add(i) = o[i].swap_bytes(); }
}
fn main() {
    let raw: Vec<u8> = openvm::io::read_vec();
    let words: Vec<u32> = raw.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let (nw, nm, ni, np) = (words[0] as usize, words[1] as usize, words[2] as usize, words[3] as usize);
    let mut o = 4;
    let wit = &words[o..o + nw]; o += nw;
    let mw = &words[o..o + nm * 24]; o += nm * 24;
    let iw = &words[o..o + ni * 27]; o += ni * 27;
    let perm = &words[o..o + np];
    // ---- mode 17's body, as in Hazync's guest ----
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
    let new = ttree::apply_stream_perm(wit, &marks, &ins, Some(perm)).expect("trie update");
    let jn = ttree::trie_journal(&old, &new, &ttree::list_digest(&marks, &ins));
    // ----
    let jb: Vec<u8> = jn.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut h = Sha256::new(); h.update(&jb);
    openvm::io::reveal_bytes32(h.finalize());
}
