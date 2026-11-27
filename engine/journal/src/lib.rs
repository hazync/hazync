//! The journal word encoding Hazync's guests commit (risc0's serde format; `serde`, Apache-2.0), the [`Journal`] that
//! carries it, and nothing of a zkVM (engine/CUTOVER.md C1). Every verifier decodes statements with it.
#![no_std]
extern crate alloc;

use alloc::vec::Vec;

pub mod serde;

pub(crate) const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

/// A proof's journal: the bytes its public values commit to (the engine proof file carries them next to the proof).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Journal {
    pub bytes: Vec<u8>,
}

impl Journal {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
    /// Decodes the journal with the word deserializer.
    pub fn decode<T: ::serde::de::DeserializeOwned>(&self) -> serde::Result<T> {
        serde::from_slice(&self.bytes)
    }
}

impl AsRef<[u8]> for Journal {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

/// An image id as the guests commit it: 8 little-endian words of the 32 bytes (risc0's `Digest` layout).
pub fn id_words(id: &[u8; 32]) -> [u32; 8] {
    core::array::from_fn(|i| u32::from_le_bytes(id[4 * i..4 * i + 4].try_into().unwrap()))
}

/// The bytes of an id committed as 8 words.
pub fn id_bytes(w: &[u32; 8]) -> [u8; 32] {
    let mut b = [0u8; 32];
    for (i, x) in w.iter().enumerate() {
        b[4 * i..4 * i + 4].copy_from_slice(&x.to_le_bytes());
    }
    b
}
