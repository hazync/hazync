//! The SDK, keys, guest build, proving with child proofs, and verification.
use std::{path::Path, slice, sync::Arc};

use eyre::{ensure, eyre, Result, WrapErr};
use openvm_build::GuestOptions;
use openvm_sdk::{
    config::{AggregationSystemParams, AppConfig},
    fs::{read_from_file_json, read_object_from_file, write_object_to_file, write_to_file_json},
    keygen::SdkCachedProvingKey,
    openvm_circuit::arch::instructions::exe::VmExe,
    prover::DeferralAggProver,
    types::{VerificationBaselineJson, VersionedVmStarkProof},
    DeferralInput, Sdk, StdIn, F, SC,
};
use openvm_sdk_config::{deferral::SupportedDeferral, SdkVmConfig};
use openvm_stark_backend::keygen::types::MultiStarkVerifyingKey;
use openvm_stark_sdk::config::{
    app_params_with_100_bits_security, hook_params_with_100_bits_security, MAX_APP_LOG_STACKED_HEIGHT,
};
use openvm_verify_stark_circuit::extension::{get_deferral_state, get_raw_deferral_results};
use openvm_verify_stark_host::{
    vk::{VerificationBaseline, VmStarkVerifyingKey},
    VmStarkProof,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The extensions Hazync's consensus code needs: RV32IM, I/O, SHA-256, and secp256k1 (field p, order n, curve).
/// Deferral is added on top in `sdk_config`.
pub const BASE_CONFIG: &str = r#"
[app_vm_config.rv32i]
[app_vm_config.rv32m]
[app_vm_config.io]
[app_vm_config.sha2]
[app_vm_config.modular]
supported_moduli = ["115792089237316195423570985008687907853269984665640564039457584007908834671663", "115792089237316195423570985008687907852837564279074904382605163141518161494337"]
[[app_vm_config.ecc.supported_curves]]
struct_name = "Secp256k1Point"
modulus = "115792089237316195423570985008687907853269984665640564039457584007908834671663"
scalar = "115792089237316195423570985008687907852837564279074904382605163141518161494337"
a = "0"
b = "7"
"#;

/// Index of the verify-stark deferral circuit (the only one); the guest's `recursion::VERIFY_STARK`.
pub const VERIFY_STARK_DEF_IDX: usize = 0;

/// The image as the guest sees it: the (exe, vm) commitments a verify_stark call returns for its proofs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Image {
    pub exe: String,
    pub vm: String,
}

impl Image {
    pub fn exe_bytes(&self) -> [u8; 32] {
        hex::decode(&self.exe).unwrap().try_into().unwrap()
    }
    pub fn vm_bytes(&self) -> [u8; 32] {
        hex::decode(&self.vm).unwrap().try_into().unwrap()
    }
}

fn agg_params() -> AggregationSystemParams {
    AggregationSystemParams::default()
}

/// A fresh SDK: the self-recursive deferral setup and the VM config built on it.
fn new_sdk() -> Result<(Sdk, SdkVmConfig)> {
    let mut vm_config = SdkVmConfig::from_toml(BASE_CONFIG).wrap_err("base config")?;
    // The SDK's default system config, as OpenVM's own examples build it: it reserves the deferral address space's
    // cells, which a TOML without a [system] table leaves at zero (deferral's initial memory then has nowhere to go).
    vm_config.system = Default::default();
    let memory_dimensions = vm_config.system.config.memory_config.memory_dimensions();
    let num_user_pvs = vm_config.system.config.num_public_values;
    let agg = agg_params();
    let deferral_agg_prover =
        DeferralAggProver::verify_stark(&agg, hook_params_with_100_bits_security(), memory_dimensions, num_user_pvs);
    vm_config.deferral = Some(
        deferral_agg_prover
            .multi_deferral_circuit_prover
            .make_config(vec![SupportedDeferral::VerifyStark]),
    );
    let vm_config = vm_config.optimize();
    let app_params = app_params_with_100_bits_security(MAX_APP_LOG_STACKED_HEIGHT);
    let sdk = Sdk::builder()
        .app_config(AppConfig::new(vm_config.clone(), app_params))
        .agg_params(agg)
        .deferral_agg_prover(deferral_agg_prover)
        .build()?;
    Ok((sdk, vm_config))
}

pub fn keygen(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let t = std::time::Instant::now();
    let (sdk, vm_config) = new_sdk()?;
    let _ = sdk.app_keygen();
    let _ = sdk.agg_pk();
    let cached: SdkCachedProvingKey<SdkVmConfig> = sdk.cached_proving_key()?;
    write_object_to_file(dir.join("sdk.pk"), cached)?;
    std::fs::write(dir.join("openvm.toml"), vm_config.to_toml()?)?;
    write_object_to_file(dir.join("agg.vk"), sdk.agg_vk().as_ref().clone())?;
    eprintln!("keygen: {:.1} s", t.elapsed().as_secs_f64());
    Ok(())
}

/// The engine's loaded state: SDK from cached keys, plus the guest once built.
pub struct Engine {
    pub sdk: Sdk,
    pub agg_vk: Arc<MultiStarkVerifyingKey<SC>>,
}

impl Engine {
    pub fn load(dir: &Path) -> Result<Self> {
        let cached: SdkCachedProvingKey<SdkVmConfig> =
            read_object_from_file(dir.join("sdk.pk")).wrap_err("sdk.pk (run keygen)")?;
        let sdk = Sdk::from_deferral_cached_proving_key(cached)?;
        let agg_vk = sdk.agg_vk();
        Ok(Engine { sdk, agg_vk })
    }

    pub fn exe(dir: &Path) -> Result<Arc<VmExe<F>>> {
        read_object_from_file(dir.join("guest.vmexe")).wrap_err("guest.vmexe (run build)")
    }

    pub fn baseline(dir: &Path) -> Result<VerificationBaseline> {
        let b: VerificationBaselineJson = read_from_file_json(dir.join("baseline.json"))?;
        Ok(b.into())
    }

    pub fn image(dir: &Path) -> Result<Image> {
        Ok(serde_json::from_slice(&std::fs::read(dir.join("image.json"))?)?)
    }

    fn child_vk(&self, baseline: VerificationBaseline) -> VmStarkVerifyingKey {
        VmStarkVerifyingKey { mvk: self.agg_vk.as_ref().clone(), baseline }
    }

    fn verify_stark_cached_commit(&self) -> Result<openvm_stark_sdk::config::baby_bear_poseidon2::Digest> {
        let mut c = self.sdk.deferral_circuit_cached_commits(VERIFY_STARK_DEF_IDX)?;
        ensure!(c.len() == 1, "expected one verify-stark cached commit");
        Ok(c.pop().unwrap().into())
    }

    /// For each child proof: its deferral input commitment (what the guest passes to verify_stark) and the
    /// (exe, vm, user public values) the guest will get back.
    pub fn child_calls(&self, baseline: &VerificationBaseline, children: &[VmStarkProof]) -> Result<Vec<ChildCall>> {
        let vk = self.child_vk(baseline.clone());
        let cc = self.verify_stark_cached_commit()?;
        let raw = get_raw_deferral_results(&vk, children, cc)?;
        ensure!(raw.len() == children.len(), "one deferral result per child");
        raw.into_iter()
            .map(|r| {
                let input_commit: [u8; 32] = r.input.clone().try_into().map_err(|_| eyre!("input commit size"))?;
                let o = &r.output_raw;
                ensure!(o.len() >= 64, "deferral output too short");
                Ok(ChildCall {
                    input_commit,
                    exe: o[..32].try_into().unwrap(),
                    vm: o[32..64].try_into().unwrap(),
                    pvs: o[64..].chunks_exact(4).map(|b| b[0]).collect(),
                })
            })
            .collect()
    }

    /// Attach the deferral data for `children` (in the order the guest verifies them) to `stdin`; returns the
    /// prover's deferral inputs.
    pub fn attach_children(
        &self,
        baseline: &VerificationBaseline,
        stdin: &mut StdIn,
        children: &[VmStarkProof],
    ) -> Result<Vec<DeferralInput>> {
        if children.is_empty() {
            stdin.deferrals = vec![Default::default()];
            return Ok(vec![DeferralInput::default()]);
        }
        let vk = self.child_vk(baseline.clone());
        let cc = self.verify_stark_cached_commit()?;
        stdin.deferrals = vec![get_deferral_state(&vk, children, cc, VERIFY_STARK_DEF_IDX as u32)?];
        Ok(vec![DeferralInput::from_inputs(children)])
    }

    /// Execute with `emit` stdin (journal printed), then prove with `prove` stdin. Returns proof + journal.
    pub fn prove(
        &self,
        exe: Arc<VmExe<F>>,
        emit: StdIn,
        prove: StdIn,
        def_inputs: &[DeferralInput],
    ) -> Result<(VmStarkProof, Vec<u8>)> {
        let t = std::time::Instant::now();
        let (pvs, out) = crate::capture::stdout_of(|| self.sdk.execute(exe.clone(), emit));
        let pvs = pvs?;
        let journal = crate::capture::journal_of(&out);
        ensure!(pvs == Sha256::digest(&journal).as_slice(), "emitted journal does not match the public values");
        eprintln!("execute: {:.1} s, journal {} bytes", t.elapsed().as_secs_f64(), journal.len());
        let t = std::time::Instant::now();
        let (proof, _baseline) = self.sdk.prove(exe, prove, def_inputs)?;
        eprintln!("prove: {:.1} s", t.elapsed().as_secs_f64());
        Ok((proof, journal))
    }
}

pub struct ChildCall {
    pub input_commit: [u8; 32],
    pub exe: [u8; 32],
    pub vm: [u8; 32],
    pub pvs: Vec<u8>,
}

pub fn build(dir: &Path, guest: &Path) -> Result<()> {
    let e = Engine::load(dir)?;
    let t = std::time::Instant::now();
    let elf = e.sdk.build(GuestOptions::default(), guest, &None, None)?;
    let exe = e.sdk.convert_to_exe(elf)?;
    let prover = e.sdk.prover(exe.clone())?;
    let baseline = prover.generate_baseline();
    write_object_to_file(dir.join("guest.vmexe"), exe.clone())?;
    write_to_file_json(dir.join("baseline.json"), VerificationBaselineJson::from(baseline.clone()))?;
    let commit = e.sdk.app_commit(exe)?;
    eprintln!("build: {:.1} s; app_commit {:?}", t.elapsed().as_secs_f64(), commit);
    Ok(())
}

/// Record the image as the guest sees it, from a proof of it (the bytes verify_stark returns). Every later proof
/// is checked against this file, and `build` refuses a guest whose baseline moved without re-recording it.
pub fn record_image(dir: &Path, e: &Engine, proof: &VmStarkProof) -> Result<Image> {
    let baseline = Engine::baseline(dir)?;
    let c = e.child_calls(&baseline, slice::from_ref(proof))?.pop().unwrap();
    let img = Image { exe: hex::encode(c.exe), vm: hex::encode(c.vm) };
    std::fs::write(dir.join("image.json"), serde_json::to_vec_pretty(&img)?)?;
    eprintln!("image: exe {} vm {}", img.exe, img.vm);
    Ok(img)
}

pub fn write_proof(out: &Path, proof: VmStarkProof, journal: &[u8]) -> Result<()> {
    let v = VersionedVmStarkProof::new(proof)?;
    write_to_file_json(with_ext(out, "proof.json"), v)?;
    std::fs::write(with_ext(out, "journal"), journal)?;
    Ok(())
}

pub fn read_proof(e: &Engine, baseline: &VerificationBaseline, p: &Path) -> Result<(VmStarkProof, Vec<u8>)> {
    let v: VersionedVmStarkProof = read_from_file_json(with_ext(p, "proof.json"))?;
    let proof = v.decode_using_vk(&e.agg_vk, baseline)?;
    let journal = std::fs::read(with_ext(p, "journal"))?;
    Ok((proof, journal))
}

pub fn with_ext(p: &Path, ext: &str) -> std::path::PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".");
    s.push(ext);
    s.into()
}

pub fn verify(dir: &Path, proof_path: &Path) -> Result<()> {
    let e = Engine::load(dir)?;
    let baseline = Engine::baseline(dir)?;
    let (proof, journal) = read_proof(&e, &baseline, proof_path)?;
    let t = std::time::Instant::now();
    Sdk::verify_proof(e.agg_vk.as_ref().clone(), baseline.clone(), &proof)?;
    let c = e.child_calls(&baseline, slice::from_ref(&proof))?.pop().unwrap();
    ensure!(c.pvs == Sha256::digest(&journal).as_slice(), "public values are not sha256(journal)");
    if let Ok(img) = Engine::image(dir) {
        ensure!(c.exe == img.exe_bytes() && c.vm == img.vm_bytes(), "proof is not of the recorded image");
    }
    eprintln!("verify: OK ({:.2} s), journal {} bytes sha256 {}", t.elapsed().as_secs_f64(), journal.len(), hex::encode(&c.pvs));
    Ok(())
}
