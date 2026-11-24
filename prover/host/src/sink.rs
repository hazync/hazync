//! Where a guest's input goes. The input writers (chunk, trie, aggregate) write to a `Sink`: RISC Zero's
//! `ExecutorEnvBuilder` for this host's own proving, or `Stream`, the same bytes collected for the OpenVM engine
//! (engine/, whose guest reads RISC Zero's serde word stream). Host-side only: no guest byte changes.
use risc0_zkvm::{ExecutorEnvBuilder, Receipt};

pub(crate) trait Sink {
    fn write<T: serde::Serialize>(&mut self, x: &T);
    fn write_slice<T: bytemuck::Pod>(&mut self, x: &[T]);
    fn segment_limit_po2(&mut self, po2: u32);
    fn add_assumption(&mut self, r: Receipt);
}

impl Sink for ExecutorEnvBuilder<'_> {
    fn write<T: serde::Serialize>(&mut self, x: &T) { ExecutorEnvBuilder::write(self, x).unwrap(); }
    fn write_slice<T: bytemuck::Pod>(&mut self, x: &[T]) { ExecutorEnvBuilder::write_slice(self, x); }
    fn segment_limit_po2(&mut self, po2: u32) { ExecutorEnvBuilder::segment_limit_po2(self, po2); }
    fn add_assumption(&mut self, r: Receipt) { ExecutorEnvBuilder::add_assumption(self, r); }
}

/// The input stream exactly as `ExecutorEnvBuilder` would hand it to the guest: serde words (little-endian) for
/// `write`, raw bytes for `write_slice`. Assumptions are not part of it (the engine passes proofs separately).
#[derive(Default)]
pub(crate) struct Stream(pub Vec<u8>);

impl Sink for Stream {
    fn write<T: serde::Serialize>(&mut self, x: &T) {
        for w in risc0_zkvm::serde::to_vec(x).unwrap() { self.0.extend_from_slice(&w.to_le_bytes()); }
    }
    fn write_slice<T: bytemuck::Pod>(&mut self, x: &[T]) { self.0.extend_from_slice(bytemuck::cast_slice(x)); }
    fn segment_limit_po2(&mut self, _: u32) {}
    fn add_assumption(&mut self, _: Receipt) {}
}
