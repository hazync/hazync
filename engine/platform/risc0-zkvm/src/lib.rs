//! The RISC Zero guest API Hazync's guest uses, implemented on OpenVM 2.1, so the SAME guest source proves on the
//! engine (../../ENGINE.md). Linked under the crate name `risc0_zkvm`; nothing here is RISC Zero code except the
//! serde word format (src/serde, vendored from risc0-zkvm 3.1.0, Apache-2.0), which defines the input stream.
//!
//! Input (stdin chunks, in order; written by hzengine):
//!   0  control: flags u32 (bit 0 emit, bit 1 dry) | n u32 | n x 32-byte deferral input commitments, one per
//!      `env::verify` call, in call order
//!   1  the input stream, byte for byte what RISC Zero's ExecutorEnv would give this guest
//! Output: `env::commit*` append to the journal exactly as RISC Zero's would; `env::finalize` reveals sha256(journal)
//! as the 32 public-value bytes (and prints the journal when `emit` = 1, for the host's execute pass).
//!
//! `env::verify(image, journal)`: RISC Zero composition becomes an OpenVM deferred STARK verification. The proof
//! named by the next input commitment must exist and verify, its (exe, vm) commitments must hash to `image`
//! (image id = sha256(app_exe_commit || app_vm_commit)), and its public values must be sha256(journal). That is the
//! statement `env::verify` makes on RISC Zero: a valid proof of that image with that journal exists.
//!
//! `bench-noverify` (a METERING-ONLY build, never proved with): every `env::verify` accepts without a proof, so an
//! aggregate's cost and profile can be measured on CPU without its children's proofs. Its image id is its own.
//!
//! A DRY run (flag bit 1) skips every `env::verify` and can never commit: `finalize` refuses it. It exists for the
//! host's collect runs (which end in env::exit before committing anything), so a block's collect aggregate can run
//! before its chunk proofs exist. No proof of a dry run can carry a journal, so skipping the checks proves nothing.
#![no_std]
extern crate alloc;

pub mod serde;

pub(crate) const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

/// sha256 on OpenVM's SHA-256 extension.
pub fn sha256(b: &[u8]) -> [u8; 32] {
    let mut h = openvm_sha2::Sha256::new();
    h.update(b);
    h.finalize()
}

pub mod guest {
    pub mod env {
        use alloc::vec::Vec;

        use crate::serde::{Deserializer, WordRead};

        struct State {
            /// the input stream, 4-byte aligned (`read_slice_ref` lends words out of it), never freed
            input: &'static [u8],
            pos: usize,
            commits: Vec<[u8; 32]>,
            next_commit: usize,
            emit: bool,
            dry: bool,
            journal: Vec<u8>,
            finalized: bool,
        }

        static mut STATE: Option<State> = None;

        /// Metering builds only: the bump allocator's position (one 1-byte allocation), to report heap use.
        #[cfg(feature = "bench-noverify")]
        static mut HEAP0: usize = 0;
        #[cfg(feature = "bench-noverify")]
        fn heap_pos() -> usize {
            unsafe { alloc::alloc::alloc(core::alloc::Layout::from_size_align(1, 1).unwrap()) as usize }
        }

        #[allow(static_mut_refs)]
        fn st() -> &'static mut State {
            // SAFETY: the guest is single-threaded; this is the only access path.
            unsafe {
                if STATE.is_none() {
                    let ctl = openvm::io::read_vec();
                    assert!(ctl.len() >= 8, "engine: control chunk too short");
                    let flags = u32::from_le_bytes(ctl[0..4].try_into().unwrap());
                    assert!(flags & !3 == 0, "engine: unknown control flags {flags:#x}");
                    let (emit, dry) = (flags & 1 != 0, flags & 2 != 0);
                    let n = u32::from_le_bytes(ctl[4..8].try_into().unwrap()) as usize;
                    assert!(ctl.len() == 8 + 32 * n, "engine: control chunk length");
                    let commits = ctl[8..].chunks_exact(32).map(|c| c.try_into().unwrap()).collect();
                    #[cfg(feature = "bench-noverify")]
                    { HEAP0 = heap_pos(); }
                    let input = aligned(openvm::io::read_vec());
                    assert!(!dry || n == 0, "engine: a dry run takes no proofs");
                    STATE = Some(State { input, pos: 0, commits, next_commit: 0, emit, dry, journal: Vec::new(), finalized: false });
                }
                STATE.as_mut().unwrap()
            }
        }

        /// The stream as a 'static slice at a 4-byte aligned address (copied once if the allocator gave it less).
        fn aligned(v: Vec<u8>) -> &'static [u8] {
            if v.as_ptr() as usize % 4 == 0 {
                return v.leak();
            }
            let n = v.len();
            let w: &'static mut [u32] = alloc::vec![0u32; n.div_ceil(4)].leak();
            let b: &'static mut [u8] = bytemuck::cast_slice_mut(w);
            b[..n].copy_from_slice(&v);
            &b[..n]
        }

        fn take(n: usize) -> &'static [u8] {
            let s = st();
            assert!(s.pos + n <= s.input.len(), "env::read: input exhausted");
            let r = &s.input[s.pos..s.pos + n];
            s.pos += n;
            r
        }

        struct Stdin;
        impl WordRead for Stdin {
            fn read_words(&mut self, words: &mut [u32]) -> crate::serde::Result<()> {
                let b = take(4 * words.len());
                for (w, c) in words.iter_mut().zip(b.chunks_exact(4)) {
                    *w = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                }
                Ok(())
            }
            fn read_padded_bytes(&mut self, bytes: &mut [u8]) -> crate::serde::Result<()> {
                let n = bytes.len();
                bytes.copy_from_slice(take(n));
                take(crate::align_up(n, 4) - n);
                Ok(())
            }
        }

        pub fn read<T: serde::de::DeserializeOwned>() -> T {
            T::deserialize(&mut Deserializer::new(&mut Stdin)).expect("env::read: deserialization failed")
        }

        pub fn read_slice<T: bytemuck::Pod>(buf: &mut [T]) {
            let b: &mut [u8] = bytemuck::cast_slice_mut(buf);
            let n = b.len();
            b.copy_from_slice(take(n));
        }

        /// The next `n` values of the stream, lent out of the input buffer instead of copied: the same bytes
        /// `read_slice` would write (engine-only, used by engine/guest/edits.py where a mode reads a large word array
        /// once). Copies only if the position is not aligned for T.
        pub fn read_slice_ref<T: bytemuck::Pod>(n: usize) -> &'static [T] {
            let b = take(n * core::mem::size_of::<T>());
            match bytemuck::try_cast_slice(b) {
                Ok(t) => t,
                Err(_) => {
                    let v: &'static mut [T] = alloc::vec![T::zeroed(); n].leak();
                    bytemuck::cast_slice_mut(v).copy_from_slice(b);
                    v
                }
            }
        }

        #[derive(Debug)]
        pub struct VerifyError;

        /// See the crate doc. Panics rather than returning Err on a mismatch: no caller continues after a failure.
        pub fn verify<T: bytemuck::Pod>(image_id: [u32; 8], journal: &[T]) -> Result<(), VerifyError> {
            let s = st();
            if s.dry || cfg!(feature = "bench-noverify") {
                let _ = (image_id, journal);
                return Ok(());
            }
            assert!(s.next_commit < s.commits.len(), "env::verify: no proof supplied for this call");
            let commit = s.commits[s.next_commit];
            s.next_commit += 1;
            let out = openvm_verify_stark_guest::verify_stark_unchecked::<0>(&commit);
            let mut id_pre = [0u8; 64];
            id_pre[..32].copy_from_slice(&out.app_exe_commit);
            id_pre[32..].copy_from_slice(&out.app_vm_commit);
            let id = crate::sha256(&id_pre);
            let mut want = [0u8; 32];
            for (k, w) in image_id.iter().enumerate() {
                want[4 * k..4 * k + 4].copy_from_slice(&w.to_le_bytes());
            }
            assert!(id == want, "env::verify: the proof is not of this image");
            let jb: &[u8] = bytemuck::cast_slice(journal);
            assert!(out.user_public_values.as_slice() == crate::sha256(jb).as_slice(), "env::verify: journal does not match the proof");
            Ok(())
        }

        pub fn commit<T: serde::Serialize + ?Sized>(x: &T) {
            let w = crate::serde::to_vec(x).expect("env::commit: serialization failed");
            let j = &mut st().journal;
            for x in w {
                j.extend_from_slice(&x.to_le_bytes());
            }
        }

        pub fn commit_slice<T: bytemuck::Pod>(x: &[T]) {
            st().journal.extend_from_slice(bytemuck::cast_slice(x));
        }

        pub fn log(msg: &str) {
            openvm::io::println(msg);
        }

        /// No cycle counter on OpenVM; the guest only prints differences of it (TXCYC lines).
        pub fn cycle_count() -> u64 {
            0
        }

        /// RISC Zero's exit with a non-zero code (the collect run's COLLECT_EXIT) means "no proof"; so does a panic.
        /// The code goes to the host as a `HZEXIT:<code>` line (hzengine execute reports it), then the guest stops.
        pub fn exit(code: u8) -> ! {
            openvm::io::println(alloc::format!("HZEXIT:{code}"));
            panic!("guest exit({code})")
        }

        /// Reveal sha256(journal) as the public values (call once, at the end of main). Every env::verify input must
        /// have been used: a supplied proof that was never checked would be a host error worth stopping on.
        pub fn finalize() {
            let s = st();
            assert!(!s.dry, "engine: a dry run cannot commit a journal");
            assert!(!s.finalized, "env::finalize twice");
            s.finalized = true;
            assert!(s.next_commit == s.commits.len(), "engine: {} proofs supplied, {} verified", s.commits.len(), s.next_commit);
            openvm::io::reveal_bytes32(crate::sha256(&s.journal));
            #[cfg(feature = "bench-noverify")]
            openvm::io::println(alloc::format!("HZHEAP: {} bytes allocated (input {} bytes)", heap_pos() - unsafe { HEAP0 }, s.input.len()));
            if s.emit {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                let mut o = alloc::string::String::with_capacity(5 + 2 * s.journal.len());
                o.push_str("HZJ:");
                for b in &s.journal {
                    o.push(HEX[(b >> 4) as usize] as char);
                    o.push(HEX[(b & 15) as usize] as char);
                }
                o.push('\n');
                openvm::io::print(o);
            }
        }
    }
}
