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
        for w in hazync_journal::serde::to_vec(x).unwrap() { self.0.extend_from_slice(&w.to_le_bytes()); }
    }
    fn write_slice<T: bytemuck::Pod>(&mut self, x: &[T]) { self.0.extend_from_slice(bytemuck::cast_slice(x)); }
    fn segment_limit_po2(&mut self, _: u32) {}
    fn add_assumption(&mut self, _: Receipt) {}
}

/// C8 step 1 (engine/C8_PLAN.md): the hosts write the guests' words with hazync-journal instead of risc0_zkvm::serde.
/// The two must give the same bytes for everything the host writes: real blocks' chunk streams and chain states,
/// written through both. (Runs while risc0-zkvm is still in this graph; it goes with it.)
#[cfg(test)]
mod serde_identity {
    use super::*;
    struct R0(Vec<u8>);
    impl Sink for R0 {
        fn write<T: serde::Serialize>(&mut self, x: &T) {
            for w in risc0_zkvm::serde::to_vec(x).unwrap() { self.0.extend_from_slice(&w.to_le_bytes()); }
        }
        fn write_slice<T: bytemuck::Pod>(&mut self, x: &[T]) { self.0.extend_from_slice(bytemuck::cast_slice(x)); }
        fn segment_limit_po2(&mut self, _: u32) {}
        fn add_assumption(&mut self, _: Receipt) {}
    }

    #[test]
    fn hazync_journal_writes_risc0_serde_bytes() {
        let mut n = 0;
        for f in ["block_130000.json", "block_140000.json", "block_741000.json", "block_962000.json", "block_965500.json"] {
            let p = format!("{}/../{f}", env!("CARGO_MANIFEST_DIR"));
            let Ok(t) = std::fs::read_to_string(&p) else { continue };
            let (anchor, w) = crate::build_full_from(serde_json::from_str(&t).unwrap());
            let (mut a, mut b) = (Stream::default(), R0(Vec::new()));
            a.write(&anchor); b.write(&anchor);
            a.write(&w); b.write(&w);
            let k = w.inputs.len();
            for (lo, hi) in [(0, k), (0, k / 2), (k / 2, k)] {
                crate::chunks::write_chunk_inputs(&mut a, &w, lo, hi);
                crate::chunks::write_chunk_inputs(&mut b, &w, lo, hi);
            }
            assert!(!a.0.is_empty() && a.0 == b.0, "{f}: hazync-journal and risc0 serde differ ({} vs {} bytes)", a.0.len(), b.0.len());
            assert_eq!(crate::state_journal_bytes(&anchor), {
                risc0_zkvm::serde::to_vec(&anchor).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>()
            });
            n += 1;
        }
        assert!(n >= 3, "block fixtures missing: {n} read");
    }
}
