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
use openvm_verify_stark_circuit::extension::get_raw_deferral_results;
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

/// HZ_LEAF_CHILDREN (app segment proofs per leaf proof, OpenVM's default and maximum 4) and HZ_INTERNAL_CHILDREN
/// (proofs per internal proof, default and maximum 3): prover-side only. Each layer's GPU memory peak grows with the
/// proofs it verifies at once (results/gpu_a40_memory), so a small card uses fewer. None: both unset.
fn tree_config_from_env() -> Result<Option<openvm_sdk::config::AggregationTreeConfig>> {
    let get = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<usize>().ok());
    let (leaf, internal) = (get("HZ_LEAF_CHILDREN"), get("HZ_INTERNAL_CHILDREN"));
    if leaf.is_none() && internal.is_none() { return Ok(None); }
    let mut t = openvm_sdk::config::AggregationTreeConfig::default();
    if let Some(n) = leaf {
        ensure!((1..=openvm_sdk::config::MAX_NUM_CHILDREN_LEAF).contains(&n), "HZ_LEAF_CHILDREN {n} outside 1..=4");
        t.num_children_leaf = n;
    }
    if let Some(n) = internal {
        ensure!((2..=openvm_sdk::config::MAX_NUM_CHILDREN_INTERNAL).contains(&n), "HZ_INTERNAL_CHILDREN {n} outside 2..=3");
        t.num_children_internal = n;
    }
    Ok(Some(t))
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
    // The guest's address space (OpenVM's 2^29 by default). HZ_GUEST_MEM_BITS (keygen only; it is in the keys, so it
    // changes the image id): every segment proof puts the whole space on the GPU as a dense image with its Merkle tree
    // (+4.5 GiB at 29, "set initial memory", results/gpu_a40_memory), so a smaller space lowers the memory floor. The
    // guest's openvm-platform MEM_BITS must be the same, so its allocator stops at the same limit.
    if let Some(bits) = std::env::var("HZ_GUEST_MEM_BITS").ok().and_then(|v| v.parse::<usize>().ok()) {
        ensure!((24..=29).contains(&bits), "HZ_GUEST_MEM_BITS {bits} outside 24..=29");
        let mc = &mut vm_config.system.config.memory_config;
        mc.pointer_max_bits = bits;
        mc.addr_spaces[openvm_sdk::openvm_circuit::arch::instructions::riscv::RV32_MEMORY_AS as usize].num_cells = 1 << bits;
        eprintln!("guest address space: 2^{bits} bytes");
    }
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
    // The app proofs' stacked height (all traces are committed in columns of 2^h rows): OpenVM's 24 by default.
    // HZ_APP_LOG_STACKED_HEIGHT (keygen only; it is in the keys, so it changes the image id): a smaller height lowers
    // the GPU memory floor (~6.6 GB at 24, results/gpu_a40_memory) at the cost of more, smaller segments.
    let h = std::env::var("HZ_APP_LOG_STACKED_HEIGHT").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(MAX_APP_LOG_STACKED_HEIGHT);
    ensure!((16..=MAX_APP_LOG_STACKED_HEIGHT).contains(&h), "HZ_APP_LOG_STACKED_HEIGHT {h} outside 16..={MAX_APP_LOG_STACKED_HEIGHT}");
    if h != MAX_APP_LOG_STACKED_HEIGHT { eprintln!("app log stacked height: {h}"); }
    let app_params = app_params_with_100_bits_security(h);
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

/// Memory of the GPU this process proves on, GiB (the first of CUDA_VISIBLE_DEVICES, via nvidia-smi). None on a CPU
/// build or without nvidia-smi.
pub fn gpu_mem_gib() -> Option<f64> {
    if !cfg!(feature = "cuda") {
        return None;
    }
    let dev = std::env::var("CUDA_VISIBLE_DEVICES").ok().and_then(|v| v.split(',').next().map(str::to_string)).unwrap_or_else(|| "0".into());
    let o = std::process::Command::new("nvidia-smi").args(["--query-gpu=memory.total", "--format=csv,noheader,nounits", "-i", &dev]).output().ok()?;
    let mib: f64 = String::from_utf8_lossy(&o.stdout).trim().lines().next()?.trim().parse().ok()?;
    Some(mib / 1024.0)
}

/// GPUs visible to this process: HZ_GPUS, else nvidia-smi's count (within CUDA_VISIBLE_DEVICES when set). 1 on CPU.
pub fn gpu_count() -> usize {
    if let Some(n) = std::env::var("HZ_GPUS").ok().and_then(|v| v.parse::<usize>().ok()) {
        return n.max(1);
    }
    if !cfg!(feature = "cuda") {
        return 1;
    }
    if let Ok(v) = std::env::var("CUDA_VISIBLE_DEVICES") {
        return v.split(',').filter(|x| !x.trim().is_empty()).count().max(1);
    }
    std::process::Command::new("nvidia-smi").arg("-L").output().ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.starts_with("GPU ")).count())
        .unwrap_or(1).max(1)
}

/// The cards the defaults below were measured on (A6000, 48 GB: results/gpu_a6000_tuning) and anything larger.
const BIG_CARD_GIB: f64 = 40.0;

/// How many provers share one GPU by default: 3 on a card of 40 GiB or more (976,379: 240 s with 1, 194 s with 2,
/// 183 s with 3 on an A6000; one prover's host work overlaps another's GPU work), else 1 (unmeasured).
pub fn default_gpu_share() -> usize {
    match gpu_mem_gib() { Some(m) if m >= BIG_CARD_GIB => 3, _ => 1 }
}

/// HZ_GPU_SHARE: how many provers this process shares its GPU with, itself included (set by whoever starts them:
/// prove-block --workers, the product host's engine pool). Default 1.
pub fn gpu_share() -> usize {
    std::env::var("HZ_GPU_SHARE").ok().and_then(|v| v.parse().ok()).unwrap_or(1).max(1)
}

/// The segment cap, GiB. HZ_SEG_MEM_GB when set. Else, on a card of 40 GiB or more: memory / share - 4, at most 24
/// (A6000: 24 alone, 20 for two, 12 for three — each the measured setting; 32 makes the prover refuse its own trace,
/// "nonzero root sum"). Smaller cards and CPU: OpenVM's default.
pub fn seg_cap_gib(share: usize) -> Option<usize> {
    if let Some(gb) = std::env::var("HZ_SEG_MEM_GB").ok().and_then(|v| v.parse::<usize>().ok()) {
        return Some(gb);
    }
    let m = gpu_mem_gib().filter(|&m| m >= BIG_CARD_GIB)?;
    // nvidia-smi gives an A6000 as 49,140 MiB (47.99 GiB): round, so 3 provers get the measured 12, not 11
    Some(((m / share as f64).round() as usize).saturating_sub(4).clamp(4, 24))
}

/// The engine's loaded state: SDK from cached keys, plus the guest once built.
pub struct Engine {
    pub sdk: Sdk,
    pub agg_vk: Arc<MultiStarkVerifyingKey<SC>>,
    /// The keys dir holds a METERING image (built with the guest feature `bench-noverify`: env::verify accepts without
    /// a proof). Its public values are the domain-separated [`bench_pvs`], never sha256(journal), so no proof of it can
    /// pass as a reader's proof of any journal (review R1-1 #1). Marked by the file BENCH_NOVERIFY in the keys dir.
    pub bench: bool,
}

/// The guest's tag for a metering image's public values (engine/platform/risc0-zkvm BENCH_TAG; the same bytes).
pub const BENCH_TAG: &[u8] = b"hazync/bench-noverify/v1";
pub const BENCH_MARK: &str = "BENCH_NOVERIFY";

/// A metering image's public values: sha256(BENCH_TAG || journal).
pub fn bench_pvs(journal: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(BENCH_TAG);
    h.update(journal);
    h.finalize().into()
}

impl Engine {
    /// The public values a proof of this keys dir's image commits for `journal`.
    pub fn pvs_of(&self, journal: &[u8]) -> [u8; 32] {
        if self.bench { bench_pvs(journal) } else { Sha256::digest(journal).into() }
    }
}

impl Engine {
    pub fn load(dir: &Path) -> Result<Self> {
        let mut cached: SdkCachedProvingKey<SdkVmConfig> =
            read_object_from_file(dir.join("sdk.pk")).wrap_err("sdk.pk (run keygen)")?;
        // The prover's per-segment trace memory cap (OpenVM default 15 GiB, sized for 24 GB cards). Fewer, larger
        // segments hash each touched memory page fewer times (Poseidon2 was 39% of a chunk's trace cells,
        // results/profile_chunk). Prover-side only: not serialized, not in any key or commitment. See seg_cap_gib.
        // HZ_SEG_MEM_MB (MiB, finer than the GiB setting; for small cards, results/gpu_a40_memory) wins when set.
        let mb = std::env::var("HZ_SEG_MEM_MB").ok().and_then(|v| v.parse::<usize>().ok());
        let cap = match mb {
            Some(m) => { ensure!(m >= 256, "HZ_SEG_MEM_MB {m} below 256"); Some((m << 20, format!("{m} MiB"))) }
            None => seg_cap_gib(gpu_share()).map(|gb| (gb << 30, format!("{gb} GiB"))),
        };
        if let Some((bytes, shown)) = cap {
            std::sync::Arc::get_mut(&mut cached.app_pk.app_vm_pk).expect("sdk.pk: unshared key").vm_config.system.config.segmentation_max_memory = bytes;
            eprintln!("segmentation max memory: {shown}");
        }
        let sdk = match tree_config_from_env()? {
            None => Sdk::from_deferral_cached_proving_key(cached)?,
            // A proving-time aggregation tree (HZ_LEAF_CHILDREN / HZ_INTERNAL_CHILDREN; in no key): the SDK from the cached
            // app and aggregation keys plus that tree. OpenVM's cached-key constructor takes no tree config and the
            // function it uses for the cached deferral keys is crate-private, so the deferral prover is rebuilt with
            // the public constructor keygen uses, and must give exactly the deferral config the app key carries.
            Some(tree) => {
                let SdkCachedProvingKey { app_pk, agg_pk, .. } = cached;
                let vm = &app_pk.app_vm_pk.vm_config;
                let dap = DeferralAggProver::verify_stark(&agg_params(), hook_params_with_100_bits_security(),
                    vm.system.config.memory_config.memory_dimensions(), vm.system.config.num_public_values);
                let regenerated = dap.multi_deferral_circuit_prover.make_config(vec![SupportedDeferral::VerifyStark]);
                let want = vm.deferral.as_ref().ok_or_else(|| eyre!("sdk.pk: the app key has no deferral config"))?;
                ensure!(bitcode::serialize(&regenerated)? == bitcode::serialize(want)?,
                        "the rebuilt deferral prover does not match the app key's deferral config");
                eprintln!("aggregation tree: {} children per leaf, {} per internal", tree.num_children_leaf, tree.num_children_internal);
                Sdk::builder().app_pk(app_pk).agg_pk(agg_pk).agg_tree_config(tree).deferral_agg_prover(dap).build()?
            }
        };
        let agg_vk = sdk.agg_vk();
        Ok(Engine { sdk, agg_vk, bench: dir.join(BENCH_MARK).exists() })
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
        let bs: Vec<&VerificationBaseline> = children.iter().map(|_| baseline).collect();
        self.child_calls_of(&bs, children)
    }

    /// Each child's raw deferral result, checked under ITS image's baseline (`baselines[i]` for `children[i]`).
    /// get_raw_deferral_results works proof by proof: the baseline enters only its host-side check of the proof's
    /// public values (verify_vm_stark_proof_pvs: the proof is of that executable); the STARK is verified under the VM's
    /// key and the input commit is the VM-level cached commit's. So children of several images on the SAME VM can be
    /// verified in one run (a utreexo tip step: the previous utreexo statement and the core); each result carries its
    /// own exe and vm commitments, which the guest binds to the id it asks env::verify for.
    fn raw_results_of(&self, baselines: &[&VerificationBaseline], children: &[VmStarkProof])
        -> Result<Vec<openvm_deferral_circuit::RawDeferralResult>> {
        ensure!(baselines.len() == children.len(), "one baseline per child");
        let cc = self.verify_stark_cached_commit()?;
        let mut raw = Vec::with_capacity(children.len());
        for (b, p) in baselines.iter().zip(children) {
            let r = get_raw_deferral_results(&self.child_vk((*b).clone()), slice::from_ref(p), cc)?;
            ensure!(r.len() == 1, "one deferral result per child");
            raw.extend(r);
        }
        Ok(raw)
    }

    /// As `child_calls`, each child under its own image's baseline.
    pub fn child_calls_of(&self, baselines: &[&VerificationBaseline], children: &[VmStarkProof]) -> Result<Vec<ChildCall>> {
        let raw = self.raw_results_of(baselines, children)?;
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
        let bs: Vec<&VerificationBaseline> = children.iter().map(|_| baseline).collect();
        self.attach_children_of(&bs, stdin, children)
    }

    /// As `attach_children`, each child under its own image's baseline: the deferral state is OpenVM's
    /// get_deferral_state, built from the per-child raw results (`raw_results_of`) instead of one baseline's.
    pub fn attach_children_of(
        &self,
        baselines: &[&VerificationBaseline],
        stdin: &mut StdIn,
        children: &[VmStarkProof],
    ) -> Result<Vec<DeferralInput>> {
        if children.is_empty() {
            stdin.deferrals = vec![Default::default()];
            return Ok(vec![DeferralInput::default()]);
        }
        let raw = self.raw_results_of(baselines, children)?;
        let results = openvm_deferral_circuit::generate_deferral_results(
            raw, VERIFY_STARK_DEF_IDX as u32, &openvm_deferral_circuit::poseidon2::deferral_poseidon2_chip::<F>());
        stdin.deferrals = vec![openvm_sdk::openvm_circuit::arch::deferral::DeferralState::new(results)];
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
        self.prove_known(exe, Some(emit), None, prove, def_inputs)
    }

    /// As `prove`; with `known` (a journal from an earlier execution of the same input) the emit pass is skipped and
    /// the proof's public values are checked against it instead.
    pub fn prove_known(
        &self,
        exe: Arc<VmExe<F>>,
        emit: Option<StdIn>,
        known: Option<Vec<u8>>,
        prove: StdIn,
        def_inputs: &[DeferralInput],
    ) -> Result<(VmStarkProof, Vec<u8>)> {
        let t = std::time::Instant::now();
        let journal = match (known, emit) {
            (Some(j), _) => j,
            (None, Some(emit)) => {
                let (pvs, out) = crate::capture::stdout_of(|| self.sdk.execute(exe.clone(), emit));
                let pvs = pvs?;
                let journal = crate::capture::journal_of(&out);
                ensure!(pvs == self.pvs_of(&journal).as_slice(), "emitted journal does not match the public values");
                eprintln!("execute: {:.1} s, journal {} bytes", t.elapsed().as_secs_f64(), journal.len());
                journal
            }
            (None, None) => eyre::bail!("prove: neither a known journal nor an emit pass"),
        };
        let t = std::time::Instant::now();
        // segpar's chain with no workers (it releases the app VM's device key before aggregation, the GPU memory
        // peak); HZ_SDK_PROVE=1: the SDK's own StarkProver::prove
        let (proof, baseline) = if std::env::var("HZ_SDK_PROVE").as_deref() == Ok("1") {
            self.sdk.prove(exe, prove, def_inputs)?
        } else {
            crate::segpar::prove(self, exe, prove, def_inputs, &std::env::temp_dir().join(format!("hz-prove-{}.stdin", std::process::id())), &[])?
        };
        eprintln!("prove: {:.1} s", t.elapsed().as_secs_f64());
        let c = self.child_calls(&baseline, slice::from_ref(&proof))?.pop().unwrap();
        ensure!(c.pvs == self.pvs_of(&journal).as_slice(), "the proof's public values are not sha256(journal)");
        Ok((proof, journal))
    }
}

impl Engine {
    /// As `prove`, with the app proof's segments spread over `workers` and this process (segpar).
    pub fn prove_split(
        &self,
        exe: Arc<VmExe<F>>,
        emit: StdIn,
        prove: StdIn,
        def_inputs: &[DeferralInput],
        stdin_file: &Path,
        workers: &[&crate::block::Worker],
    ) -> Result<(VmStarkProof, Vec<u8>)> {
        let t = std::time::Instant::now();
        let (pvs, out) = crate::capture::stdout_of(|| self.sdk.execute(exe.clone(), emit));
        let pvs = pvs?;
        let journal = crate::capture::journal_of(&out);
        ensure!(pvs == self.pvs_of(&journal).as_slice(), "emitted journal does not match the public values");
        eprintln!("execute: {:.1} s, journal {} bytes", t.elapsed().as_secs_f64(), journal.len());
        let t = std::time::Instant::now();
        let (proof, baseline) = crate::segpar::prove(self, exe, prove, def_inputs, stdin_file, workers)?;
        eprintln!("prove (segpar): {:.1} s", t.elapsed().as_secs_f64());
        let c = self.child_calls(&baseline, slice::from_ref(&proof))?.pop().unwrap();
        ensure!(c.pvs == self.pvs_of(&journal).as_slice(), "the proof's public values are not sha256(journal)");
        Ok((proof, journal))
    }
}

pub struct ChildCall {
    pub input_commit: [u8; 32],
    pub exe: [u8; 32],
    pub vm: [u8; 32],
    pub pvs: Vec<u8>,
}

/// Reproducible ids (engine/FEASIBILITY.md Q15): two builds of the same source on different paths gave different exe
/// commitments, because file paths reach the program (panic locations, std's sources). Map every machine-specific root
/// to a fixed name: CARGO_HOME, RUSTUP_HOME, the assembled tree (guest/../../..) and the engine kit the guest's
/// Cargo.toml points its platform crates at. (The C/C++ half is in engine/guest/build.rs.)
fn remap_flags(guest: &Path) -> Result<Vec<String>> {
    let home = std::env::var("HOME").unwrap_or_default();
    let canon = |p: String| std::fs::canonicalize(&p).map(|c| c.to_string_lossy().into_owned()).unwrap_or(p);
    let cargo = canon(std::env::var("CARGO_HOME").unwrap_or(format!("{home}/.cargo")));
    let rustup = canon(std::env::var("RUSTUP_HOME").unwrap_or(format!("{home}/.rustup")));
    let tree = canon(guest.join("../../..").to_string_lossy().into_owned());
    let toml = std::fs::read_to_string(guest.join("Cargo.toml"))?;
    let engine = toml.lines().find_map(|l| l.split("path = \"").nth(1).and_then(|r| r.split("/platform/").next()).filter(|p| p.starts_with('/')))
        .map(|p| canon(p.to_string())).ok_or_else(|| eyre!("guest Cargo.toml: no platform path"))?;
    Ok([(cargo, "/hz-cargo"), (rustup, "/hz-rustup"), (tree, "/hz-tree"), (engine, "/hz-engine")]
        .iter().map(|(from, to)| format!("--remap-path-prefix={from}={to}")).collect())
}

pub fn build(dir: &Path, guest: &Path) -> Result<()> {
    let e = Engine::load(dir)?;
    let t = std::time::Instant::now();
    // HZ_GUEST_FEATURES (comma-separated): extra guest features, e.g. the metering-only `bench-noverify`
    let feats: Vec<String> = std::env::var("HZ_GUEST_FEATURES").map(|v| v.split(',').filter(|f| !f.is_empty()).map(str::to_string).collect()).unwrap_or_default();
    if !feats.is_empty() {
        eprintln!("guest features: {}", feats.join(","));
    }
    let bench = feats.iter().any(|f| f == "bench-noverify");
    let elf = e.sdk.build(GuestOptions { rustc_flags: remap_flags(guest)?, ..GuestOptions::default() }.with_features(feats), guest, &None, None)?;
    let exe = e.sdk.convert_to_exe(elf)?;
    let prover = e.sdk.prover(exe.clone())?;
    let baseline = prover.generate_baseline();
    write_object_to_file(dir.join("guest.vmexe"), exe.clone())?;
    // the metering marker follows the build (review R1-1 #1): present exactly when this image has bench-noverify
    if bench {
        std::fs::write(dir.join(BENCH_MARK), "metering image (guest feature bench-noverify): never a reader's image\n")?;
    } else if dir.join(BENCH_MARK).exists() {
        std::fs::remove_file(dir.join(BENCH_MARK))?;
    }
    write_to_file_json(dir.join("baseline.json"), VerificationBaselineJson::from(baseline.clone()))?;
    let img = image_of(&baseline);
    std::fs::write(dir.join("image.json"), serde_json::to_vec_pretty(&img)?)?;
    eprintln!("build: {:.1} s; image exe {} vm {}", t.elapsed().as_secs_f64(), img.exe, img.vm);
    Ok(())
}

/// The image as the guest sees it (the bytes a verify_stark call returns for a proof of it), from the baseline alone:
/// exe = the executable commitment (program, initial memory, entry pc), vm = poseidon2 over the app, leaf and
/// internal-for-leaf vk commitments; each field element as 4 little-endian bytes. The same computation as
/// openvm-verify-stark-circuit's output_raw_from_proof; `record_image` checks it against a real proof.
pub fn image_of(baseline: &VerificationBaseline) -> Image {
    use openvm_stark_backend::p3_field::PrimeField32;
    let exe = baseline.app_exe_commit();
    let parts = [
        baseline.app_vk_commit.cached_commit, baseline.app_vk_commit.vk_pre_hash,
        baseline.leaf_vk_commit.cached_commit, baseline.leaf_vk_commit.vk_pre_hash,
        baseline.internal_for_leaf_vk_commit.cached_commit, baseline.internal_for_leaf_vk_commit.vk_pre_hash,
    ];
    let vm = openvm_recursion_circuit::utils::poseidon2_hash_slice(&parts.into_iter().flatten().collect::<Vec<_>>()).0;
    let bytes = |d: &[F]| -> String { hex::encode(d.iter().flat_map(|f| f.as_canonical_u32().to_le_bytes()).collect::<Vec<u8>>()) };
    Image { exe: bytes(&exe), vm: bytes(&vm) }
}

/// Check a real proof's (exe, vm) against image.json (written by `build`), or write it if absent.
pub fn record_image(dir: &Path, e: &Engine, proof: &VmStarkProof) -> Result<Image> {
    let baseline = Engine::baseline(dir)?;
    let c = e.child_calls(&baseline, slice::from_ref(proof))?.pop().unwrap();
    let img = Image { exe: hex::encode(c.exe), vm: hex::encode(c.vm) };
    if let Ok(have) = Engine::image(dir) {
        ensure!(have == img, "image.json ({have:?}) is not what a proof of this guest carries ({img:?})");
        return Ok(img);
    }
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

/// A proof as P.proof.json + P.journal, or, when `p` is itself a file starting with the HZE1 magic, that one file (the
/// board's and the readers' proof file: `pack`), decoded under this engine's key.
pub fn read_proof(e: &Engine, baseline: &VerificationBaseline, p: &Path) -> Result<(VmStarkProof, Vec<u8>)> {
    if p.is_file() {
        let f = std::fs::read(p)?;
        if hazync_engine_verify::is_engine_proof(&f) {
            let (pr, j) = hazync_engine_verify::unpack(&f).map_err(|x| eyre!("{}: {x}", p.display()))?;
            let proof = VmStarkProof::decode_from_bytes_using_vk(e.agg_vk.as_ref(), baseline, pr)
                .wrap_err_with(|| format!("{}: the proof does not decode under this key", p.display()))?;
            return Ok((proof, j.to_vec()));
        }
    }
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

/// See `Cmd::Pack`: the published proof file (HZE1: OpenVM's plain encoding of the proof, then the journal), checked
/// by the readers' own verifier (engine/verify-core) before it is written; the key file and its pin with `--key`.
/// Needs only agg.vk and baseline.json (not the proving keys).
pub fn pack(dir: &Path, proof_path: &Path, out: &Path, key_out: Option<&Path>) -> Result<()> {
    ensure!(!dir.join(BENCH_MARK).exists(), "{}: a metering image ({BENCH_MARK}): its proofs are never a reader's file", dir.display());
    let mvk: MultiStarkVerifyingKey<SC> = read_object_from_file(dir.join("agg.vk")).wrap_err("agg.vk")?;
    let mvk = Arc::new(mvk);
    let baseline = Engine::baseline(dir)?;
    let v: VersionedVmStarkProof = read_from_file_json(with_ext(proof_path, "proof.json"))?;
    let proof = v.decode_using_vk(&mvk, &baseline)?;
    let journal = std::fs::read(with_ext(proof_path, "journal"))?;
    let img = image_of(&baseline);
    if let Ok(have) = Engine::image(dir) {
        ensure!(have == img, "image.json {have:?} is not the baseline's {img:?}");
    }
    let (file, key, pin, id) = pack_bytes(mvk.as_ref(), baseline, &proof, &journal)?;
    let enc_len = file.len() - 8 - journal.len();
    std::fs::write(out, &file)?;
    if let Some(k) = key_out {
        std::fs::write(k, &key)?;
    }
    eprintln!("pack: {} ({} bytes: proof {}, journal {}), verified as the readers verify it", out.display(), file.len(), enc_len, journal.len());
    println!("image {}", hex::encode(id));
    println!("key_pin {}{}", hex::encode(pin), key_out.map(|k| format!(" ({}, {} bytes)", k.display(), key.len())).unwrap_or_default());
    Ok(())
}

/// See `Cmd::WasmInputs`.
pub fn wasm_inputs(dir: &Path, proof_path: &Path, out: &Path) -> Result<()> {
    ensure!(!dir.join(BENCH_MARK).exists(), "{}: a metering image ({BENCH_MARK}): its proofs are never a reader's file", dir.display());
    use openvm_stark_backend::codec::Encode;
    let e = Engine::load(dir)?;
    let baseline = Engine::baseline(dir)?;
    let (proof, journal) = read_proof(&e, &baseline, proof_path)?;
    std::fs::create_dir_all(out)?;
    let vk = VmStarkVerifyingKey { mvk: e.agg_vk.as_ref().clone(), baseline };
    std::fs::write(out.join("vk.bin"), bitcode::serialize(&vk)?)?;
    let mut enc = Vec::new();
    proof.encode(&mut enc)?;
    let z = zstd::encode_all(&enc[..], 19)?;
    std::fs::write(out.join("proof.zst"), &z)?;
    // the engine proof file the verifiers read (engine/verify-core: HZE1, proof, journal), and the key's pin
    // plain encoding in the proof file (no zstd in any verifier build); proof.bin beside it for the wasm probe
    std::fs::write(out.join("proof.hze"), hazync_engine_verify::pack(&enc, &journal))?;
    std::fs::write(out.join("proof.bin"), &enc)?;
    std::fs::write(out.join("vk.sha256"), format!("{}\n", hex::encode(Sha256::digest(std::fs::read(out.join("vk.bin"))?))))?;
    let mut bad = enc.clone();
    let k = bad.len() / 2;
    bad[k] ^= 1;
    std::fs::write(out.join("proof_bad.zst"), zstd::encode_all(&bad[..], 19)?)?;
    // the native verifier on exactly these bytes, for comparison with the WASM build
    let t = std::time::Instant::now();
    let ok = openvm_verify_stark_host::verify_vm_stark_proof(&vk, &z).is_ok();
    let bad_ok = openvm_verify_stark_host::verify_vm_stark_proof(&vk, &std::fs::read(out.join("proof_bad.zst"))?).is_ok();
    eprintln!("wasm inputs: vk {} bytes, proof {} bytes encoded / {} zstd; native verify {} ({:.3} s), flipped byte {}",
              std::fs::metadata(out.join("vk.bin"))?.len(), enc.len(), z.len(), ok, t.elapsed().as_secs_f64(), bad_ok);
    Ok(())
}

pub fn verify(dir: &Path, proof_path: &Path) -> Result<()> {
    let e = Engine::load(dir)?;
    let baseline = Engine::baseline(dir)?;
    let (proof, journal) = read_proof(&e, &baseline, proof_path)?;
    let t = std::time::Instant::now();
    Sdk::verify_proof(e.agg_vk.as_ref().clone(), baseline.clone(), &proof)?;
    let c = e.child_calls(&baseline, slice::from_ref(&proof))?.pop().unwrap();
    ensure!(c.pvs == e.pvs_of(&journal).as_slice(), "public values are not sha256(journal)");
    if e.bench {
        eprintln!("verify: a METERING image ({BENCH_MARK}): its public values are tagged, no reader accepts this proof");
    }
    if let Ok(img) = Engine::image(dir) {
        ensure!(c.exe == img.exe_bytes() && c.vm == img.vm_bytes(), "proof is not of the recorded image");
    }
    eprintln!("verify: OK ({:.2} s), journal {} bytes sha256 {}", t.elapsed().as_secs_f64(), journal.len(), hex::encode(&c.pvs));
    Ok(())
}

/// The HZE1 file of a proof (OpenVM's plain encoding, then the journal), the key file, its pin and the image id,
/// checked by the readers' own verifier (engine/verify-core) first: a file that would fail is never returned.
pub fn pack_bytes(mvk: &MultiStarkVerifyingKey<SC>, baseline: VerificationBaseline, proof: &VmStarkProof, journal: &[u8])
    -> Result<(Vec<u8>, Vec<u8>, [u8; 32], [u8; 32])> {
    use openvm_stark_backend::codec::Encode;
    let mut enc = Vec::new();
    proof.encode(&mut enc)?;
    let id = crate::stream::image_id(&image_of(&baseline));
    let key = bitcode::serialize(&VmStarkVerifyingKey { mvk: mvk.clone(), baseline })?;
    let pin: [u8; 32] = Sha256::digest(&key).into();
    let file = hazync_engine_verify::pack(&enc, journal);
    let (p, j) = hazync_engine_verify::unpack(&file).map_err(|e| eyre!("pack: {e}"))?;
    hazync_engine_verify::verify_full_bytes(&key, &pin, p, j, &id).map_err(|e| eyre!("pack: the readers' verifier refuses it: {e}"))?;
    Ok((file, key, pin, id))
}
