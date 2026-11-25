//! The slice of RustCrypto sha2 Hazync's guest uses, on OpenVM's SHA-256 compression (state = 8 native-LE words,
//! the same layout as RustCrypto's compress256 state). Outputs are plain [u8; 32].
#![no_std]
extern "C" { fn zkvm_sha256_impl(state: *const u8, input: *const u8, output: *mut u8); }
const H0: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
#[repr(C, align(4))]
pub struct Block(pub [u8; 64]);
impl From<[u8; 64]> for Block { fn from(b: [u8; 64]) -> Block { Block(b) } }
pub fn compress256(state: &mut [u32; 8], blocks: &[Block]) {
    for b in blocks { compress(state, &b.0); }
}
/// One compression in place (the intrinsic writes a separate output; no zeroed temporary). The intrinsic reads its
/// input as words: `block` must be 4-byte aligned (`Sha256::buf` is, at offset 32 of a repr(C) struct; input blocks are
/// passed only when their address is).
#[inline(always)]
fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut o = core::mem::MaybeUninit::<[u32; 8]>::uninit();
    unsafe {
        zkvm_sha256_impl(state.as_ptr() as *const u8, block.as_ptr(), o.as_mut_ptr() as *mut u8);
        *state = o.assume_init();
    }
}
#[derive(Clone)]
#[repr(C)]
pub struct Sha256 { st: [u32; 8], buf: [u8; 64], n: usize, len: u64 }
pub trait Digest: Sized {
    fn new() -> Self;
    fn update(&mut self, data: impl AsRef<[u8]>);
    fn finalize(self) -> [u8; 32];
    fn chain_update(mut self, data: impl AsRef<[u8]>) -> Self { self.update(data); self }
    fn digest(data: impl AsRef<[u8]>) -> [u8; 32] { let mut h = Self::new(); h.update(data); h.finalize() }
}
impl Digest for Sha256 {
    fn new() -> Self { Sha256 { st: H0, buf: [0; 64], n: 0, len: 0 } }
    fn update(&mut self, data: impl AsRef<[u8]>) {
        let mut d = data.as_ref(); self.len += d.len() as u64;
        if self.n > 0 {
            let k = core::cmp::min(64 - self.n, d.len());
            self.buf[self.n..self.n + k].copy_from_slice(&d[..k]); self.n += k; d = &d[k..];
            if self.n < 64 { return; }
            compress(&mut self.st, &self.buf); self.n = 0;
        }
        // whole blocks straight from the input when it is word-aligned, else through the buffer
        while d.len() >= 64 {
            let (blk, rest) = d.split_at(64);
            if blk.as_ptr() as usize % 4 == 0 { compress(&mut self.st, blk.try_into().unwrap()); }
            else { self.buf.copy_from_slice(blk); compress(&mut self.st, &self.buf); }
            d = rest;
        }
        self.buf[..d.len()].copy_from_slice(d); self.n = d.len();
    }
    fn finalize(mut self) -> [u8; 32] {
        // padding written in place: 0x80, zeros, the bit length (one more block when it does not fit)
        let (n, bits) = (self.n, self.len * 8);
        self.buf[n] = 0x80;
        if n >= 56 {
            self.buf[n + 1..].fill(0);
            compress(&mut self.st, &self.buf);
            self.buf[..56].fill(0);
        } else {
            self.buf[n + 1..56].fill(0);
        }
        self.buf[56..].copy_from_slice(&bits.to_be_bytes());
        compress(&mut self.st, &self.buf);
        let w: [u32; 8] = self.st.map(u32::to_be);
        unsafe { core::mem::transmute::<[u32; 8], [u8; 32]>(w) }
    }
}
