// EXPERIMENTAL Trusted State chunked ingest: guest helpers shared by the chunk and combine guests (#[path]).
use alloc::vec::Vec;
use risc0_zkvm::guest::env;

pub fn read_u32() -> u32 { env::read() }
pub fn read_bytes(n: usize) -> Vec<u8> {
    let mut v = alloc::vec![0u8; n.div_ceil(4) * 4];
    if n > 0 { env::read_slice(&mut v); }
    v.truncate(n);
    v
}
pub fn read_words(n: usize) -> Vec<u32> {
    let mut v = alloc::vec![0u32; n];
    if n > 0 { env::read_slice(&mut v); }
    v
}
/// A length-prefixed byte section (u32 byte length, then the bytes padded to words).
pub fn read_section() -> Vec<u8> { let n = read_u32() as usize; read_bytes(n) }

pub struct Src<'a> { pub b: &'a [u8], pub i: usize }
impl<'a> Src<'a> {
    pub fn take(&mut self, n: usize) -> &'a [u8] { let s = &self.b[self.i..self.i + n]; self.i += n; s }
    pub fn u8(&mut self) -> u8 { self.take(1)[0] }
    pub fn compact(&mut self) -> u64 {
        match self.u8() {
            c @ 0..=252 => c as u64,
            253 => u16::from_le_bytes(self.take(2).try_into().unwrap()) as u64,
            254 => u32::from_le_bytes(self.take(4).try_into().unwrap()) as u64,
            _ => u64::from_le_bytes(self.take(8).try_into().unwrap()),
        }
    }
    /// Core's VARINT (VarIntMode::DEFAULT)
    pub fn varint(&mut self) -> u64 {
        let mut n: u64 = 0;
        loop {
            let ch = self.u8();
            assert!(n <= u64::MAX >> 7, "VARINT too large");
            n = (n << 7) | (ch & 0x7f) as u64;
            if ch & 0x80 != 0 { n += 1; } else { return n; }
        }
    }
}

/// Core's WriteVarInt bytes (the coins-database key's vout encoding).
pub fn varint_bytes(mut n: u64) -> Vec<u8> {
    let mut tmp = [0u8; 10];
    let mut len = 0;
    loop {
        tmp[len] = (n & 0x7f) as u8 | if len > 0 { 0x80 } else { 0 };
        if n <= 0x7f { break; }
        n = (n >> 7) - 1;
        len += 1;
    }
    tmp[..=len].iter().rev().copied().collect()
}

/// Core's DecompressAmount.
pub fn decompress_amount(mut x: u64) -> u64 {
    if x == 0 { return 0; }
    x -= 1;
    let mut e = x % 10;
    x /= 10;
    let mut n;
    if e < 9 { let d = (x % 9) + 1; x /= 9; n = x * 10 + d; } else { n = x + 1; }
    while e > 0 { n *= 10; e -= 1; }
    n
}

pub fn be_limbs(b: &[u8]) -> [u32; 8] {
    let mut l = [0u32; 8];
    for k in 0..8 { l[k] = u32::from_be_bytes(b[28 - 4 * k..32 - 4 * k].try_into().unwrap()); }
    l
}

pub fn median(v: &[u32]) -> u32 { let mut s = v.to_vec(); s.sort_unstable(); s[s.len() / 2] }
