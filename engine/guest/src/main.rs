//! Hazync engine guest (OpenVM). One image, modes as in the product guest; see ../ENGINE.md.
//!
//! Every mode reveals sha256(journal) as its 32 public-value bytes; the journal travels beside the proof.
#![cfg_attr(target_os = "zkvm", no_main)]
#![cfg_attr(target_os = "zkvm", no_std)]

extern crate alloc;

mod recursion;
mod selftest;

openvm::entry!(main);

pub const MODE_SELFTEST_LEAF: u32 = 100;
pub const MODE_SELFTEST_NODE: u32 = 101;

static mut EMIT: bool = false;

pub fn main() {
    let mode: u32 = openvm::io::read();
    // 1 = an execute pass that prints the journal for the host; proving runs pass 0 (the journal is the same).
    let emit: u32 = openvm::io::read();
    unsafe { EMIT = emit == 1 };
    match mode {
        MODE_SELFTEST_LEAF => selftest::leaf(),
        MODE_SELFTEST_NODE => selftest::node(),
        _ => panic!("unknown mode {mode}"),
    }
}

/// Reveal sha256(journal) as the proof's public values.
pub fn commit_journal(journal: &[u8]) {
    openvm::io::reveal_bytes32(openvm_sha2::sha256(journal));
    if unsafe { EMIT } {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = alloc::string::String::with_capacity(5 + 2 * journal.len());
        s.push_str("HZJ:");
        for b in journal {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 15) as usize] as char);
        }
        s.push('\n');
        openvm::io::print(s);
    }
}
