//! The shim's padding and buffering against RustCrypto, the compression in software (state: 8 native words).
#[no_mangle]
pub unsafe extern "C" fn zkvm_sha256_impl(state: *const u8, input: *const u8, output: *mut u8) {
    let mut st: [u32; 8] = core::ptr::read_unaligned(state as *const [u32; 8]);
    let blk: [u8; 64] = core::ptr::read_unaligned(input as *const [u8; 64]);
    sha2::compress256(&mut st, &[blk.into()]);
    core::ptr::write_unaligned(output as *mut [u32; 8], st);
}

#[cfg(test)]
mod tests {
    use shim::Digest as _;
    use sha2::Digest as _;
    #[test]
    fn digests_match() {
        let data: Vec<u8> = (0..700u32).map(|i| (i * 131 + 7) as u8).collect();
        for off in 0..4 {
            for n in 0..300 {
                let m = &data[off..off + n];
                let want: [u8; 32] = sha2::Sha256::digest(m).into();
                assert_eq!(<shim::Sha256 as shim::Digest>::digest(m), want, "len {n} off {off}");
                for cut in [0, 1, 3, 55, 56, 63, 64, 65, 129].into_iter().filter(|c| *c <= n) {
                    let mut h = <shim::Sha256 as shim::Digest>::new();
                    shim::Digest::update(&mut h, &m[..cut]);
                    shim::Digest::update(&mut h, &m[cut..]);
                    assert_eq!(shim::Digest::finalize(h), want, "len {n} off {off} cut {cut}");
                }
            }
        }
    }
}
