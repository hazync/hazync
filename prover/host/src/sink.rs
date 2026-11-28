//! Where a guest's input goes. The input writers (chunk, trie, aggregate, join) write to a `Sink`: `Stream`, the
//! engine's input file (engine/, whose guest reads the serde word stream RISC Zero's did: hazync-journal, byte-identical
//! to risc0-zkvm 3.1.0's, engine/journal/compat). Host-side only: no guest byte changes. RISC Zero's ExecutorEnvBuilder
//! was the other sink until CUTOVER C8 step 4; the byte identity of the two was checked then on five real blocks'
//! anchors, witnesses and chunk splits (C8 step 1, engine/C8_PLAN.md), and the engine has read these streams since.

pub(crate) trait Sink {
    fn write<T: serde::Serialize>(&mut self, x: &T);
    fn write_slice<T: bytemuck::Pod>(&mut self, x: &[T]);
}

/// The input stream as the guest reads it: serde words (little-endian) for `write`, raw bytes for `write_slice`.
/// Proofs a guest verifies are passed to the engine beside it, never in it.
#[derive(Default)]
pub(crate) struct Stream(pub Vec<u8>);

impl Sink for Stream {
    fn write<T: serde::Serialize>(&mut self, x: &T) {
        for w in hazync_journal::serde::to_vec(x).unwrap() { self.0.extend_from_slice(&w.to_le_bytes()); }
    }
    fn write_slice<T: bytemuck::Pod>(&mut self, x: &[T]) { self.0.extend_from_slice(bytemuck::cast_slice(x)); }
}

/// The streams of C8 step 1's identity test, pinned: five real blocks' anchors, witnesses and chunk splits (whole,
/// halves) as this host writes them, by SHA-256. At C8 step 1 (bd10e3cd) these bytes were checked equal to
/// risc0_zkvm::serde's; RISC Zero left this graph at C8 step 4, so the bytes are pinned instead (the anchor's self_id is
/// zero since step 4: build_full's states are proved by no image). A change here changes what every guest reads.
#[cfg(test)]
mod pinned_streams {
    use super::*;

    #[test]
    fn real_blocks_streams_are_pinned() {
        let want = [
            ("block_130000.json", "50f1d7f5906f0c3424f345ac1d7f344849083d69a59f1edac4dfb6d16f1ef969"),
            ("block_140000.json", "b1f359580e859529d9ee3f46030c94388ed1a9689fa5014b7502283b91935f27"),
            ("block_741000.json", "34b77066bfe093cd0767cd64ad7c5703fafeb36cd6150f61f108ade8125564c1"),
            ("block_962000.json", "fb9e61d6070527d3df2f3cd6c203418bfd3131d672f46c5a19bc5e5833780c22"),
            ("block_965500.json", "ae02cdbfed337be7d042a14ea55624c727fa6a4465c0d741ec2c49f75bc679b4"),
        ];
        for (f, pin) in want {
            let p = format!("{}/../{f}", env!("CARGO_MANIFEST_DIR"));
            let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"));
            let (anchor, w) = crate::build_full_from(serde_json::from_str(&t).unwrap());
            let mut a = Stream::default();
            a.write(&anchor);
            a.write(&w);
            let k = w.inputs.len();
            for (lo, hi) in [(0, k), (0, k / 2), (k / 2, k)] { crate::chunks::write_chunk_inputs(&mut a, &w, lo, hi); }
            assert_eq!(crate::state_journal_bytes(&anchor), { let mut s = Stream::default(); s.write(&anchor); s.0 });
            use sha2::Digest;
            assert_eq!(crate::hex(&sha2::Sha256::digest(&a.0)), pin, "{f}: the stream moved ({} bytes)", a.0.len());
        }
    }
}
