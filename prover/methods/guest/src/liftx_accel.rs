//! The ECDSA scalar inverse on the bigint2 coprocessor (patches/0008, feature `scalar-inv-accel`).
//!
//! This module once also held G1, a coprocessor lift-x that REPLACED libsecp's square root (feature `liftx-accel`,
//! patches/0007). It was superseded by the host-hinted lift that libsecp verifies (liftx_hint.rs, patches/0013) and
//! removed 11-04 (E2c). The file name is kept so history stays readable.

/// hazync#205 / GHOST_GAINS — the ECDSA scalar inverse, on the coprocessor.
///
/// `patches/0005` deliberately KEEPS `secp256k1_scalar_inverse_var` as literal libsecp: the middle
/// path moves only the group arithmetic. That was invisible when an ECDSA verify cost 1.72 M cycles.
/// After bigint2 and G1 it is the fourth-largest term in the chunk work — a profile of the current
/// stack puts `secp256k1_modinv32_var` + `modinv32_update_de_30` at **146.7 M, 7.0%**.
///
/// `Fr::inverse()` is the same value over bigint2 arithmetic. libsecp has already rejected a zero
/// `s` by this point (the r/s zero checks run before the inversion), but this stays total anyway and
/// returns 0 on a zero input, which is what `secp256k1_scalar_inverse_var` does.
///
/// # Safety
/// `inb` valid for 32 bytes, `outb` writable for 32. Called only from `secp256k1_ecdsa_sig_verify`
/// under `patches/0008`.
#[cfg(feature = "scalar-inv-accel")]
#[no_mangle]
pub unsafe extern "C" fn hazync_scalar_inverse(inb: *const u8, outb: *mut u8) -> i32 {
    use risc0_crypto::curves::secp256k1::Fr;
    let mut a = [0u8; 32];
    core::ptr::copy_nonoverlapping(inb, a.as_mut_ptr(), 32);
    let x = Fr::from_be_bytes_mod_order(&a);
    if x.is_zero() {
        core::ptr::write_bytes(outb, 0, 32);
        return 1;
    }
    x.inverse().to_bigint().write_be_bytes(core::slice::from_raw_parts_mut(outb, 32));
    1
}
