//! OpenVM's host verifier as wasm32 (engine/FEASIBILITY.md Q11).
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
use openvm_stark_sdk::{
    config::baby_bear_poseidon2::{poseidon2_perm, CHUNK, F},
    openvm_stark_backend::{
        p3_field::{PrimeCharacteristicRing, PrimeField32},
        p3_symmetric::Permutation,
    },
};
use openvm_verify_stark_host::{verify_vm_stark_proof, verify_vm_stark_proof_decoded, vk::VmStarkVerifyingKey, VmStarkProof};

/// Poseidon2 state width (stark-sdk config/baby_bear_poseidon2.rs `WIDTH`, private there).
const WIDTH: usize = 16;
use sha2::{Digest as _, Sha256};

pub fn verify_bytes(vk: &[u8], proof: &[u8]) -> u32 {
    let Ok(vk) = bitcode::deserialize::<VmStarkVerifyingKey>(vk) else { return 2 };
    verify_vm_stark_proof(&vk, proof).is_ok() as u32
}

/// openvm-recursion's poseidon2_hash_slice (crates/recursion/src/utils.rs): absorb CHUNK field elements at a time
/// into the first CHUNK lanes (overwrite), permute after each full chunk and after a partial last one.
fn poseidon2_hash_slice(vals: &[F]) -> [F; CHUNK] {
    let perm = poseidon2_perm();
    let mut state = [F::ZERO; WIDTH];
    let mut i = 0;
    for &v in vals {
        state[i] = v;
        i += 1;
        if i == CHUNK {
            perm.permute_mut(&mut state);
            i = 0;
        }
    }
    if i != 0 {
        perm.permute_mut(&mut state);
    }
    state[..CHUNK].try_into().unwrap()
}

fn le_bytes(d: &[F]) -> Vec<u8> {
    d.iter().flat_map(|f| f.as_canonical_u32().to_le_bytes()).collect()
}

/// (exe, vm) as the guest sees them, from the baseline alone: engine.rs `image_of`.
pub fn image_of(vk: &VmStarkVerifyingKey) -> (Vec<u8>, Vec<u8>) {
    let b = &vk.baseline;
    let exe = b.app_exe_commit();
    let parts = [
        b.app_vk_commit.cached_commit, b.app_vk_commit.vk_pre_hash,
        b.leaf_vk_commit.cached_commit, b.leaf_vk_commit.vk_pre_hash,
        b.internal_for_leaf_vk_commit.cached_commit, b.internal_for_leaf_vk_commit.vk_pre_hash,
    ];
    let vm = poseidon2_hash_slice(&parts.into_iter().flatten().collect::<Vec<_>>());
    (le_bytes(&exe), le_bytes(&vm))
}

/// The image id Hazync publishes: SHA-256(exe || vm) (engine/host/src/r0.rs `image_id`).
pub fn image_id_of(vk: &VmStarkVerifyingKey) -> [u8; 32] {
    let (exe, vm) = image_of(vk);
    let mut h = Sha256::new();
    h.update(&exe);
    h.update(&vm);
    h.finalize().into()
}

pub fn verify_full_bytes(vk: &[u8], proof: &[u8], journal: &[u8], image_id: &[u8]) -> u32 {
    let Ok(vk) = bitcode::deserialize::<VmStarkVerifyingKey>(vk) else { return 2 };
    if image_id.len() != 32 {
        return 6;
    }
    if image_id_of(&vk)[..] != image_id[..] {
        return 5;
    }
    let Ok(p) = VmStarkProof::decode_compressed_using_vk(&vk.mvk, &vk.baseline, proof) else { return 3 };
    if verify_vm_stark_proof_decoded(&vk, &p).is_err() {
        return 0;
    }
    // one byte per field element (the guest reveals bytes); anything above 255 is not a byte
    let pvs: Option<Vec<u8>> = p.user_pvs_proof.public_values.iter().map(|f| u8::try_from(f.as_canonical_u32()).ok()).collect();
    if pvs.as_deref() != Some(&Sha256::digest(journal)[..]) {
        return 4;
    }
    1
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
    match bitcode::deserialize::<VmStarkVerifyingKey>(vk) {
        Ok(vk) => image_id_of(&vk).iter().map(|b| format!("{b:02x}")).collect(),
        Err(_) => String::new(),
    }
}
