//! OpenVM's host verifier as wasm32 (engine/FEASIBILITY.md Q11): engine/verify-core's checks behind wasm-bindgen.
//!
//! `verify(vk, proof)`: the STARK alone. vk = bitcode of VmStarkVerifyingKey (agg.vk + baseline), proof = VmStarkProof
//! in its compressed encoding. 1 = valid, 0 = invalid, 2 = bad vk bytes.
//!
//! `verify_full(vk, proof, journal, image_id)`: what a page needs to trust a Hazync result. On top of the STARK
//! (which already checks the user public values' Merkle proof against the proven final memory, the program
//! commitment, the initial memory and the exit code against the vk's baseline):
//! - the image: SHA-256(exe || vm), computed from the vk's baseline as engine/host/src/engine.rs `image_of` does,
//!   must equal the published image id (so the vk file itself need not be trusted);
//! - the journal: the proof's 32 user public values (one byte per field element) must equal SHA-256(journal).
//! Codes: 1 valid, 0 STARK invalid, 2 bad vk bytes, 3 proof does not decode, 4 journal mismatch, 5 image mismatch,
//! 6 image_id not 32 bytes.
pub use hazync_engine_verify::{image_id_of, image_of};

pub fn verify_bytes(vk: &[u8], proof: &[u8]) -> u32 {
    hazync_engine_verify::verify_stark_bytes(vk, proof)
}

pub fn verify_full_bytes(vk: &[u8], proof: &[u8], journal: &[u8], image_id: &[u8]) -> u32 {
    hazync_engine_verify::code(hazync_engine_verify::verify_full_bytes(vk, proof, journal, image_id))
}

/// wasm ABI: alloc(n) for the host to copy bytes in, then verify(vk_ptr, vk_len, proof_ptr, proof_len).
#[no_mangle]
pub extern "C" fn alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n);
    let p = v.as_mut_ptr();
    core::mem::forget(v);
    p
}
#[no_mangle]
pub unsafe extern "C" fn verify(vk: *const u8, vk_len: usize, proof: *const u8, proof_len: usize) -> u32 {
    verify_bytes(core::slice::from_raw_parts(vk, vk_len), core::slice::from_raw_parts(proof, proof_len))
}

/// The STARK check through wasm-bindgen (byte arrays in, 1/0/2 out), for Node and the browser.
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn verify_proof(vk: &[u8], proof: &[u8]) -> u32 {
    verify_bytes(vk, proof)
}

/// The full check through wasm-bindgen: codes as `verify_full_bytes`.
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn verify_full(vk: &[u8], proof: &[u8], journal: &[u8], image_id: &[u8]) -> u32 {
    verify_full_bytes(vk, proof, journal, image_id)
}

/// The image id the vk commits to, hex (for a page to show what it checked against).
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn image_id_hex(vk: &[u8]) -> String {
    match hazync_engine_verify::decode_vk(vk) {
        Ok(vk) => image_id_of(&vk).iter().map(|b| format!("{b:02x}")).collect(),
        Err(_) => String::new(),
    }
}
