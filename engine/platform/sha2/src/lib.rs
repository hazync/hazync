//! The slice of RustCrypto sha2 Hazync's guest uses, on OpenVM's SHA-256 compression (state = 8 native-LE words,
//! the same layout as RustCrypto's compress256 state). Outputs are plain [u8; 32].
#![no_std]
extern "C" { fn zkvm_sha256_impl(state: *const u8, input: *const u8, output: *mut u8); }
const H0: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
pub struct Block(pub [u8; 64]);
impl From<[u8; 64]> for Block { fn from(b: [u8; 64]) -> Block { Block(b) } }
pub fn compress256(state: &mut [u32; 8], blocks: &[Block]) {
    for b in blocks { let mut o = [0u32; 8]; unsafe { zkvm_sha256_impl(state.as_ptr() as *const u8, b.0.as_ptr(), o.as_mut_ptr() as *mut u8) }; *state = o; }
}
#[derive(Clone)]
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
        while !d.is_empty() {
            let k = core::cmp::min(64 - self.n, d.len());
            self.buf[self.n..self.n + k].copy_from_slice(&d[..k]); self.n += k; d = &d[k..];
            if self.n == 64 { compress256(&mut self.st, &[Block(self.buf)]); self.n = 0; }
        }
    }
    fn finalize(mut self) -> [u8; 32] {
        let bits = self.len * 8; let mut pad = [0u8; 72]; pad[0] = 0x80;
        let padlen = if self.n < 56 { 56 - self.n } else { 120 - self.n };
        self.update(&pad[..padlen]); self.update(bits.to_be_bytes());
        let mut o = [0u8; 32]; for k in 0..8 { o[4*k..4*k+4].copy_from_slice(&self.st[k].to_be_bytes()); } o
    }
}
