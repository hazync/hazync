//! Check an engine (OpenVM) proof the way a Hazync verifier must (engine/VERIFIER.md): the code engine/wasm-verify
//! carried (FEASIBILITY Q11, tested natively, in Node and in a browser 26.11.2026), as a library for swiftsync-verify,
//! corecheck and the wasm build alike.
//!
//! `verify_full(vk, proof, journal, image_id)`: on top of OpenVM's STARK check (which also checks the user public
//! values' Merkle proof against the proven final memory, the program commitment, the initial memory and the exit code
//! against the vk's baseline):
//! - the image: SHA-256(exe || vm), computed from the vk's baseline as engine/host/src/engine.rs `image_of` does,
//!   must equal the published image id, so the vk file itself need not be trusted;
//! - the journal: the proof's 32 user public values (one byte per field element) must equal SHA-256(journal).
use openvm_stark_sdk::{
    config::baby_bear_poseidon2::{poseidon2_perm, CHUNK, F},
    openvm_stark_backend::{
        p3_field::{PrimeCharacteristicRing, PrimeField32},
        p3_symmetric::Permutation,
    },
};
use openvm_verify_stark_host::{verify_vm_stark_proof, verify_vm_stark_proof_decoded, vk::VmStarkVerifyingKey, VmStarkProof};
use sha2::{Digest as _, Sha256};

pub use openvm_verify_stark_host::vk::VmStarkVerifyingKey as VerifyingKey;

/// Poseidon2 state width (stark-sdk config/baby_bear_poseidon2.rs `WIDTH`, private there).
const WIDTH: usize = 16;

/// Why a proof was refused. The numbers are engine/wasm-verify's result codes (1 = valid).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The STARK does not verify against the key.
    Stark = 0,
    /// The verifying key's bytes do not decode.
    Vk = 2,
    /// The proof's bytes do not decode.
    Decode = 3,
    /// The proof's public values are not SHA-256(journal).
    Journal = 4,
    /// The key is not the published image's.
    Image = 5,
    /// The image id is not 32 bytes.
    IdLength = 6,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::Stark => "the STARK does not verify against the image's key",
            Refusal::Vk => "the verifying key does not decode",
            Refusal::Decode => "the proof does not decode",
            Refusal::Journal => "the proof's public values are not SHA-256 of this journal",
            Refusal::Image => "the verifying key is not the published image's (SHA-256(exe || vm) differs)",
            Refusal::IdLength => "the image id is not 32 bytes",
        })
    }
}

/// wasm-verify's code for a result: 1 valid, else the refusal's number.
pub fn code(r: Result<(), Refusal>) -> u32 {
    match r {
        Ok(()) => 1,
        Err(e) => e as u32,
    }
}

/// The verifying key from its bytes (bitcode of VmStarkVerifyingKey: agg.vk + baseline, `hzengine wasm-inputs`).
pub fn decode_vk(vk: &[u8]) -> Result<VmStarkVerifyingKey, Refusal> {
    bitcode::deserialize::<VmStarkVerifyingKey>(vk).map_err(|_| Refusal::Vk)
}

/// The STARK alone (no image or journal check): 1 valid, 0 invalid, 2 bad vk bytes.
pub fn verify_stark_bytes(vk: &[u8], proof: &[u8]) -> u32 {
    let Ok(vk) = decode_vk(vk) else { return 2 };
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

/// The full check (see the crate doc), on a decoded key.
pub fn verify_full(vk: &VmStarkVerifyingKey, proof: &[u8], journal: &[u8], image_id: &[u8]) -> Result<(), Refusal> {
    if image_id.len() != 32 {
        return Err(Refusal::IdLength);
    }
    if image_id_of(vk)[..] != image_id[..] {
        return Err(Refusal::Image);
    }
    let p = VmStarkProof::decode_compressed_using_vk(&vk.mvk, &vk.baseline, proof).map_err(|_| Refusal::Decode)?;
    verify_vm_stark_proof_decoded(vk, &p).map_err(|_| Refusal::Stark)?;
    // one byte per field element (the guest reveals bytes); anything above 255 is not a byte
    let pvs: Option<Vec<u8>> = p.user_pvs_proof.public_values.iter().map(|f| u8::try_from(f.as_canonical_u32()).ok()).collect();
    if pvs.as_deref() != Some(&Sha256::digest(journal)[..]) {
        return Err(Refusal::Journal);
    }
    Ok(())
}

/// The full check on the key's bytes.
pub fn verify_full_bytes(vk: &[u8], proof: &[u8], journal: &[u8], image_id: &[u8]) -> Result<(), Refusal> {
    verify_full(&decode_vk(vk)?, proof, journal, image_id)
}
