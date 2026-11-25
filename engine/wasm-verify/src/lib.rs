//! Can OpenVM's host verifier run as wasm32? (engine/FEASIBILITY.md Q11) `verify(vk, proof)`: vk = bitcode of
//! VmStarkVerifyingKey, proof = VmStarkProof in its compressed encoding. 1 = valid, 0 = invalid, 2 = bad vk bytes.
use openvm_verify_stark_host::{verify_vm_stark_proof, vk::VmStarkVerifyingKey};

pub fn verify_bytes(vk: &[u8], proof: &[u8]) -> u32 {
    let Ok(vk) = bitcode::deserialize::<VmStarkVerifyingKey>(vk) else { return 2 };
    verify_vm_stark_proof(&vk, proof).is_ok() as u32
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


/// The same check through wasm-bindgen (byte arrays in, 1/0/2 out), for Node and the browser.
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn verify_proof(vk: &[u8], proof: &[u8]) -> u32 {
    verify_bytes(vk, proof)
}
