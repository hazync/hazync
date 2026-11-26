//! Check an engine (OpenVM) proof the way a Hazync verifier must (engine/VERIFIER.md): the code engine/wasm-verify
//! carried (FEASIBILITY Q11, tested natively, in Node and in a browser 26.11.2026), as a library for swiftsync-verify,
//! corecheck and the wasm build alike.
//!
//! `verify_full(vk, vk_sha256, proof, journal, image_id)`:
//! - **the key is the pinned one: SHA-256(vk bytes) == `vk_sha256`.** The key is NOT self-certifying: OpenVM verifies
//!   the STARK under `vk.mvk` (the internal-recursive layer's key) and then compares the proof's public values with
//!   the baseline, but nothing ties `mvk` itself to the baseline or to the image id, and those public values are
//!   whatever the circuit `mvk` describes. A forged `mvk` (a circuit that outputs the expected commitments) would pass
//!   every later check. So the whole key file is pinned per image (review 26.11.2026; the probe's earlier claim that
//!   "the vk file need not be trusted" was wrong);
//! - OpenVM's STARK check (with the user public values' Merkle proof against the proven final memory, the program
//!   commitment, the initial memory and the exit code against the key's baseline);
//! - the image: SHA-256(exe || vm), computed from the key's baseline as engine/host/src/engine.rs `image_of` does,
//!   must equal the published image id (a consistency check: the id the guest's env::verify uses);
//! - the journal: the proof's 32 user public values (one byte per field element) must equal SHA-256(journal).
use openvm_stark_sdk::{
    config::baby_bear_poseidon2::{poseidon2_perm, CHUNK, F},
    openvm_stark_backend::{
        p3_field::{PrimeCharacteristicRing, PrimeField32},
        p3_symmetric::Permutation,
    },
};
use openvm_verify_stark_host::{verify_vm_stark_proof_decoded, vk::VmStarkVerifyingKey, VmStarkProof};
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
    /// The verifying key's bytes are not the pinned key's (SHA-256 differs), or the pin is not 32 bytes.
    VkPin = 7,
    /// The file is not an engine proof container (magic, lengths).
    Container = 8,
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
            Refusal::VkPin => "the verifying key is not the image's pinned key (SHA-256 of the key file differs)",
            Refusal::Container => "not an engine proof file (HZE1 magic, proof length, journal)",
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

/// The engine proof file: `HZE1`, the proof's length (u32 LE), the proof in OpenVM's plain codec encoding (not zstd:
/// no C decoder in any verifier build; ~19% larger), then the journal (the rest of the file). The magic comes first so
/// that no decoder is ever handed bytes of another kind (a RISC Zero receipt starts with a bincode u32 0..3).
pub const MAGIC: [u8; 4] = *b"HZE1";

/// An engine proof file from its parts.
pub fn pack(proof: &[u8], journal: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + proof.len() + journal.len());
    v.extend_from_slice(&MAGIC);
    v.extend_from_slice(&(proof.len() as u32).to_le_bytes());
    v.extend_from_slice(proof);
    v.extend_from_slice(journal);
    v
}

/// (compressed proof, journal) from an engine proof file.
pub fn unpack(file: &[u8]) -> Result<(&[u8], &[u8]), Refusal> {
    if file.len() < 8 || file[..4] != MAGIC {
        return Err(Refusal::Container);
    }
    let n = u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize;
    let rest = &file[8..];
    if n == 0 || n > rest.len() {
        return Err(Refusal::Container);
    }
    Ok(rest.split_at(n))
}

/// Is this an engine proof file (by its magic alone)?
pub fn is_engine_proof(file: &[u8]) -> bool {
    file.len() >= 4 && file[..4] == MAGIC
}

/// The verifying key from its bytes (bitcode of VmStarkVerifyingKey: agg.vk + baseline, `hzengine wasm-inputs`).
pub fn decode_vk(vk: &[u8]) -> Result<VmStarkVerifyingKey, Refusal> {
    bitcode::deserialize::<VmStarkVerifyingKey>(vk).map_err(|_| Refusal::Vk)
}

/// The STARK alone (no image or journal check) on a plain-encoded proof: 1 valid, 0 invalid, 2 bad vk bytes, 3 the
/// proof does not decode.
pub fn verify_stark_bytes(vk: &[u8], proof: &[u8]) -> u32 {
    let Ok(vk) = decode_vk(vk) else { return 2 };
    let Ok(p) = VmStarkProof::decode_from_bytes_using_vk(&vk.mvk, &vk.baseline, proof) else { return 3 };
    verify_vm_stark_proof_decoded(&vk, &p).is_ok() as u32
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

/// The checks after the pin, on a decoded key: ONLY for a key the caller already holds as trusted (decoded from bytes
/// whose SHA-256 it checked against the pin, or built from its own keys). Use `verify_full_bytes` otherwise.
pub fn verify_full_pinned(vk: &VmStarkVerifyingKey, proof: &[u8], journal: &[u8], image_id: &[u8]) -> Result<(), Refusal> {
    if image_id.len() != 32 {
        return Err(Refusal::IdLength);
    }
    if image_id_of(vk)[..] != image_id[..] {
        return Err(Refusal::Image);
    }
    let p = VmStarkProof::decode_from_bytes_using_vk(&vk.mvk, &vk.baseline, proof).map_err(|_| Refusal::Decode)?;
    verify_vm_stark_proof_decoded(vk, &p).map_err(|_| Refusal::Stark)?;
    // one byte per field element (the guest reveals bytes); anything above 255 is not a byte
    let pvs: Option<Vec<u8>> = p.user_pvs_proof.public_values.iter().map(|f| u8::try_from(f.as_canonical_u32()).ok()).collect();
    if pvs.as_deref() != Some(&Sha256::digest(journal)[..]) {
        return Err(Refusal::Journal);
    }
    Ok(())
}

/// The full check on the key's bytes, against the key's pinned SHA-256 (see the crate doc: the key must be pinned).
pub fn verify_full_bytes(vk: &[u8], vk_sha256: &[u8], proof: &[u8], journal: &[u8], image_id: &[u8]) -> Result<(), Refusal> {
    if vk_sha256.len() != 32 || Sha256::digest(vk)[..] != vk_sha256[..] {
        return Err(Refusal::VkPin);
    }
    verify_full_pinned(&decode_vk(vk)?, proof, journal, image_id)
}
