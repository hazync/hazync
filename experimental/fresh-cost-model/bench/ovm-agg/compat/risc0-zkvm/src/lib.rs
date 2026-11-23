//! risc0-zkvm compatibility for Hazync's guest on OpenVM: the input stream, journal and serde word format.
extern crate alloc;
pub mod serde;
pub(crate) const fn align_up(addr: usize, align: usize) -> usize { (addr + align - 1) & !(align - 1) }
pub mod guest {
    pub mod env {
        use alloc::vec::Vec;
        use crate::serde::{Deserializer, WordRead};
        static mut INPUT: Option<Vec<u8>> = None;
        static mut POS: usize = 0;
        static mut JOURNAL: Vec<u8> = Vec::new();
        pub static mut VERIFIED: u32 = 0;
        fn input() -> &'static [u8] {
            unsafe {
                if (*core::ptr::addr_of!(INPUT)).is_none() { INPUT = Some(openvm::io::read_vec()); }
                (*core::ptr::addr_of!(INPUT)).as_ref().unwrap()
            }
        }
        fn take(n: usize) -> &'static [u8] {
            let i = input();
            unsafe { let s = &i[POS..POS + n]; POS += n; s }
        }
        struct Stdin;
        impl WordRead for Stdin {
            fn read_words(&mut self, words: &mut [u32]) -> crate::serde::Result<()> {
                for w in words.iter_mut() { *w = u32::from_le_bytes(take(4).try_into().unwrap()); }
                Ok(())
            }
            fn read_padded_bytes(&mut self, bytes: &mut [u8]) -> crate::serde::Result<()> {
                let n = bytes.len(); bytes.copy_from_slice(take(n)); let pad = crate::align_up(n, 4) - n; take(pad); Ok(())
            }
        }
        pub fn read<T: serde::de::DeserializeOwned>() -> T {
            T::deserialize(&mut Deserializer::new(&mut Stdin)).expect("env::read: deserialization failed")
        }
        pub fn read_slice<T: bytemuck::Pod>(buf: &mut [T]) {
            let b: &mut [u8] = bytemuck::cast_slice_mut(buf);
            let n = b.len(); b.copy_from_slice(take(n));
        }
        /// ⚠ NOT CHECKED HERE: on RISC Zero this resolves a receipt assumption. On OpenVM the same statement is the
        /// job of OpenVM's own aggregation/recursion; this port measures the block-validation work only.
        pub fn verify<A, T: bytemuck::Pod>(_image_id: A, _journal: &[T]) -> Result<(), core::convert::Infallible> {
            unsafe { VERIFIED += 1; } Ok(())
        }
        pub fn commit<T: serde::Serialize + ?Sized>(x: &T) {
            let w = crate::serde::to_vec(x).expect("env::commit: serialization failed");
            unsafe { for x in w { (*core::ptr::addr_of_mut!(JOURNAL)).extend_from_slice(&x.to_le_bytes()); } }
        }
        pub fn commit_slice<T: bytemuck::Pod>(x: &[T]) {
            unsafe { (*core::ptr::addr_of_mut!(JOURNAL)).extend_from_slice(bytemuck::cast_slice(x)); }
        }
        pub fn log(msg: &str) { openvm::io::println(msg); }
        pub fn cycle_count() -> u64 { 0 }
        pub fn exit(code: u8) -> ! { panic!("guest exit({code})") }
        /// OpenVM port: reveal SHA-256(journal bytes) — the "journal sha256" Hazync's host prints for RISC Zero.
        pub fn finalize_journal() {
            let j = unsafe { &*core::ptr::addr_of!(JOURNAL) };
            let mut h = openvm_sha2::Sha256::new(); h.update(j);
            openvm::io::reveal_bytes32(h.finalize());
            openvm::io::println(alloc::format!("journal {} bytes, env::verify calls (not checked) {}", j.len(), unsafe { VERIFIED }));
        }
    }
}
