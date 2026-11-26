//! Handover step 1 (engine/HANDOVER.md): OpenVM's own STARK verifier, as SOFTWARE inside an engine guest. Input (RISC
//! Zero's serde word stream, through the engine platform): the verifying key (bitcode of VmStarkVerifyingKey: agg.vk +
//! baseline) and a proof in its UNCOMPRESSED encoding. Commits (verified, the image id SHA-256(exe || vm) derived from
//! the key's baseline, the proof's 32 public-value bytes), the facts a handover checks against its pin and the old
//! journal. Verification failure is committed, not a panic, so a negative control has a journal to compare.
use openvm_stark_sdk::{
    config::baby_bear_poseidon2::{poseidon2_perm, CHUNK, F},
    openvm_stark_backend::{p3_field::PrimeCharacteristicRing, p3_field::PrimeField32, p3_symmetric::Permutation},
};
use openvm_verify_stark_host::{verify_vm_stark_proof_decoded, vk::VmStarkVerifyingKey, VmStarkProof};
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};

/// Poseidon2 state width (stark-sdk config/baby_bear_poseidon2.rs `WIDTH`, private there).
const WIDTH: usize = 16;

/// openvm-recursion's poseidon2_hash_slice (as engine/wasm-verify/src/lib.rs).
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

/// The image id the key's baseline names (engine/host/src/engine.rs `image_of`, r0.rs `image_id`).
fn image_id_of(vk: &VmStarkVerifyingKey) -> [u8; 32] {
    let b = &vk.baseline;
    let exe = b.app_exe_commit();
    let parts = [
        b.app_vk_commit.cached_commit, b.app_vk_commit.vk_pre_hash,
        b.leaf_vk_commit.cached_commit, b.leaf_vk_commit.vk_pre_hash,
        b.internal_for_leaf_vk_commit.cached_commit, b.internal_for_leaf_vk_commit.vk_pre_hash,
    ];
    let vm = poseidon2_hash_slice(&parts.into_iter().flatten().collect::<Vec<_>>());
    let mut h = Sha256::new();
    h.update(le_bytes(&exe));
    h.update(le_bytes(&vm));
    h.finalize().into()
}

fn main() {
    let vk_bytes: Vec<u8> = env::read();
    let proof_bytes: Vec<u8> = env::read();
    let vk: VmStarkVerifyingKey = bitcode::deserialize(&vk_bytes).expect("vk: not a VmStarkVerifyingKey");
    let image = image_id_of(&vk);
    let (ok, pvs) = match VmStarkProof::decode_from_bytes_using_vk(&vk.mvk, &vk.baseline, &proof_bytes) {
        Err(_) => (false, [0u8; 32]),
        Ok(p) => {
            let ok = verify_vm_stark_proof_decoded(&vk, &p).is_ok();
            let mut pvs = [0u8; 32];
            for (o, f) in pvs.iter_mut().zip(p.user_pvs_proof.public_values.iter()) {
                *o = u8::try_from(f.as_canonical_u32()).unwrap_or(0xff);
            }
            (ok, pvs)
        }
    };
    env::commit(&(ok as u32, image, pvs));
    env::finalize();
}
