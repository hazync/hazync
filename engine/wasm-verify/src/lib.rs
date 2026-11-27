//! OpenVM's host verifier as wasm32 (engine/FEASIBILITY.md Q11): engine/verify-core's checks behind wasm-bindgen.
//!
//! Only the PINNED check is exported (review R1-6 #2, 27.11.2026): a STARK-only `verify` / `verify_proof` taking an
//! unpinned key answered "valid" for a forged key and proof, so they are gone. vk = bitcode of VmStarkVerifyingKey
//! (agg.vk + baseline), proof = VmStarkProof in its PLAIN encoding (what the HZE1 file carries).
//!
//! `verify_full(vk, vk_sha256, proof, journal, image_id)`: what a page needs to trust a Hazync result (engine/verify-core:
//! the key must be the PINNED one, `vk_sha256` a constant the page is published with). On top of the STARK
//! (which already checks the user public values' Merkle proof against the proven final memory, the program
//! commitment, the initial memory and the exit code against the vk's baseline):
//! - the image: SHA-256(exe || vm), computed from the vk's baseline as engine/host/src/engine.rs `image_of` does,
//!   must equal the published image id (a consistency check; the KEY is trusted only through its pin);
//! - the journal: the proof's 32 user public values (one byte per field element) must equal SHA-256(journal).
//! Codes: 1 valid, 0 STARK invalid, 2 bad vk bytes, 3 proof does not decode, 4 journal mismatch, 5 image mismatch,
//! 6 image_id not 32 bytes, 7 the key is not the pinned one (or the pin is not 32 bytes).
pub use hazync_engine_verify::{image_id_of, image_of};

pub fn verify_full_bytes(vk: &[u8], vk_sha256: &[u8], proof: &[u8], journal: &[u8], image_id: &[u8]) -> u32 {
    hazync_engine_verify::code(hazync_engine_verify::verify_full_bytes(vk, vk_sha256, proof, journal, image_id))
}

/// The full check through wasm-bindgen: codes as `verify_full_bytes`.
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn verify_full(vk: &[u8], vk_sha256: &[u8], proof: &[u8], journal: &[u8], image_id: &[u8]) -> u32 {
    verify_full_bytes(vk, vk_sha256, proof, journal, image_id)
}

/// The image id the vk's baseline names, hex: for DISPLAY only. The key is unpinned here, and a forged key names any id
/// it likes, so this proves nothing; only `verify_full` (with the pin) does (review R1-6 #2).
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn unpinned_image_id_hex(vk: &[u8]) -> String {
    match hazync_engine_verify::decode_vk(vk) {
        Ok(vk) => image_id_of(&vk).iter().map(|b| format!("{b:02x}")).collect(),
        Err(_) => String::new(),
    }
}
