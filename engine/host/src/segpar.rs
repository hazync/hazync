//! Segment-parallel proving (engine, 25.11.2026). OpenVM proves a program's continuation segments one after another on
//! one device (`VmInstance::prove_continuations`): the aggregate's 8 segments took 11.6 s of its 16.4 s on one of four
//! RTX PRO 6000s while the other three sat idle. A segment's proof needs only the state it starts from, so here
//! `serve` workers on the other GPUs prove leading runs of segments (each re-executing up to its first), while this
//! process proves the last run, which leaves it holding the final state the aggregation and deferral steps read. The
//! rest is the SDK's own `StarkProver::prove` chain, step for step, on the assembled app proof.
//!
//! Prover-side only: the proof is the same kind of object and is verified exactly as before (prove_block verifies the
//! final proof; a wrong or misordered segment proof fails the continuation checks of the leaf aggregation).
use std::{path::Path, sync::Arc};

use eyre::{ensure, eyre, Result};
use openvm_sdk::{
    fs::{read_object_from_file, write_object_to_file},
    openvm_circuit::{
        arch::{
            execution_mode::metered::segment_ctx::Segment, hasher::poseidon2::vm_poseidon2_hasher,
            instructions::exe::VmExe, ContinuationVmProof, PreflightExecutionOutput, VmInstance,
        },
        system::memory::merkle::{public_values::UserPublicValuesProof, MerkleTree},
    },
    prover::{compute_deferral_merkle_proofs, vm::new_local_prover},
    DefaultStarkEngine, DeferralInput, StdIn, F, SC,
};
use openvm_stark_backend::{p3_field::PrimeField32, proof::Proof, StarkEngine};
use openvm_verify_stark_host::{
    pvs::{DeferralPvs, DEF_PVS_AIR_ID},
    VmStarkProof,
};

use crate::engine::Engine;

#[cfg(feature = "cuda")]
type Vb = openvm_sdk_config::SdkVmGpuBuilder;
#[cfg(not(feature = "cuda"))]
type Vb = openvm_sdk_config::SdkVmCpuBuilder;
type Inst = VmInstance<DefaultStarkEngine, Vb>;

/// A VM instance for `exe`, cutting segments at `cap` bytes when given (else this process's own cap). Segment proofs
/// from several processes fit together only when every one cuts the same segments.
fn instance(e: &Engine, exe: Arc<VmExe<F>>, cap: Option<usize>) -> Result<Inst> {
    let pk = &e.sdk.app_pk().app_vm_pk;
    match cap {
        Some(c) if c != pk.vm_config.system.config.segmentation_max_memory => {
            let mut pk2 = openvm_sdk::prover::vm::types::VmProvingKey { vm_config: pk.vm_config.clone(), vm_pk: pk.vm_pk.clone() };
            pk2.vm_config.system.config.segmentation_max_memory = c;
            Ok(new_local_prover::<DefaultStarkEngine, Vb>(Vb::default(), &pk2, exe)?)
        }
        _ => Ok(new_local_prover::<DefaultStarkEngine, Vb>(Vb::default(), pk, exe)?),
    }
}

/// This process's segment cap, bytes.
pub fn own_cap(e: &Engine) -> usize {
    e.sdk.app_pk().app_vm_pk.vm_config.system.config.segmentation_max_memory
}

/// Metered execution: the segments, exactly as `prove_continuations` cuts them. Leaves the instance at its initial
/// state for `input`.
fn segments(inst: &mut Inst, input: &StdIn) -> Result<Vec<Segment>> {
    inst.reset_state(input.clone());
    let exe = inst.exe().clone();
    let ctx = inst.vm.build_metered_ctx(&exe);
    let interp = inst.vm.metered_interpreter(&exe)?;
    let (segs, _) = interp.execute_metered(input.clone(), ctx)?;
    Ok(segs)
}

/// Run the segments in order from the instance's initial state, proving those in `lo..hi` (`prove_continuations`'
/// loop body; the others are only executed, to reach the next state).
fn run(inst: &mut Inst, segs: &[Segment], lo: usize, hi: usize) -> Result<Vec<Proof<SC>>> {
    let mut proofs = Vec::with_capacity(hi - lo);
    let mut state = inst.state_mut().take();
    for (i, seg) in segs.iter().enumerate().take(hi) {
        let from = state.take().ok_or_else(|| eyre!("segment {i}: no state"))?;
        if i >= lo {
            inst.vm.transport_init_memory_to_device(&from.memory);
        }
        let PreflightExecutionOutput { system_records, record_arenas, to_state } =
            inst.vm.execute_preflight(&mut inst.interpreter, from, Some(seg.num_insns), &seg.trace_heights)?;
        state = Some(to_state);
        if i >= lo {
            let ctx = inst.vm.generate_proving_ctx(system_records, record_arenas)?;
            proofs.push(inst.vm.engine.prove(inst.vm.pk(), ctx).map_err(|err| eyre!("segment {i}: {err:?}"))?);
        }
    }
    *inst.state_mut() = state;
    Ok(proofs)
}

/// Worker side (`serve` op "prove_segments"): segments `lo..hi` of the program on `input`, written to `out`.
pub fn prove_segments(e: &Engine, dir: &Path, input: &Path, lo: usize, hi: usize, cap: usize, out: &Path) -> Result<usize> {
    let exe = Engine::exe(dir)?;
    let stdin: StdIn = read_object_from_file(input)?;
    let mut inst = instance(e, exe, Some(cap))?;
    let segs = segments(&mut inst, &stdin)?;
    ensure!(lo < hi && hi <= segs.len(), "segments {lo}..{hi} of {}", segs.len());
    let proofs = run(&mut inst, &segs, lo, hi)?;
    write_object_to_file(out, &proofs)?;
    Ok(segs.len())
}

/// The aggregation prover with HZ_LEAF_CHILDREN children per leaf (None: unset, the SDK's own), made once.
fn leaf_children_agg(e: &Engine) -> Result<Option<Arc<openvm_sdk::prover::AggProver>>> {
    static AGG: std::sync::OnceLock<Arc<openvm_sdk::prover::AggProver>> = std::sync::OnceLock::new();
    let Some(n) = std::env::var("HZ_LEAF_CHILDREN").ok().and_then(|v| v.parse::<usize>().ok()) else { return Ok(None) };
    ensure!((1..=openvm_sdk::config::MAX_NUM_CHILDREN_LEAF).contains(&n), "HZ_LEAF_CHILDREN {n} outside 1..=4");
    Ok(Some(AGG.get_or_init(|| {
        eprintln!("leaf children: {n}");
        let tree = openvm_sdk::config::AggregationTreeConfig { num_children_leaf: n, ..Default::default() };
        Arc::new(openvm_sdk::prover::AggProver::from_pk(
            Arc::new(e.sdk.app_pk().app_vm_pk.vm_pk.get_vk()), e.sdk.agg_pk(), tree, e.sdk.def_hook_cached_commit()))
    }).clone()))
}

/// HZ_LEAF_CHILDREN is set: proofs go through `prove` (with or without workers) so the leaf batch applies.
pub fn leaf_children_set() -> bool {
    std::env::var("HZ_LEAF_CHILDREN").is_ok()
}

/// Contiguous runs of segments, one per prover, balanced by instruction count; the last run is this process's.
fn plan(segs: &[Segment], provers: usize) -> Vec<(usize, usize)> {
    let n = segs.len();
    let k = provers.clamp(1, n.max(1));
    let total: u64 = segs.iter().map(|s| s.num_insns.max(1)).sum();
    let mut runs = Vec::with_capacity(k);
    let (mut lo, mut acc) = (0usize, 0u64);
    for (i, s) in segs.iter().enumerate() {
        acc += s.num_insns.max(1);
        let left = n - i - 1;
        let want = total * (runs.len() as u64 + 1) / k as u64;
        if runs.len() + 1 < k && (acc >= want || left == k - runs.len() - 1) {
            runs.push((lo, i + 1));
            lo = i + 1;
        }
    }
    runs.push((lo, n));
    runs.retain(|r| r.0 < r.1);
    runs
}

/// The whole proof, as `Sdk::prove`, with the app proof's segments spread over `workers` (`serve` processes, each
/// handed a path to the stdin and a run of segments) and this process. `stdin_file`: where to write the stdin for them.
pub fn prove(
    e: &Engine,
    exe: Arc<VmExe<F>>,
    prove: StdIn,
    def_inputs: &[DeferralInput],
    stdin_file: &Path,
    workers: &[&crate::block::Worker],
) -> Result<(VmStarkProof, openvm_verify_stark_host::vk::VerificationBaseline)> {
    let t0 = std::time::Instant::now();
    let lap = |what: &str| eprintln!("segpar: {what} at {:.2} s", t0.elapsed().as_secs_f64());
    let mut prover = e.sdk.prover(exe.clone())?;
    // HZ_LEAF_CHILDREN (prover-side only, not in any key): how many app segment proofs one leaf proof verifies,
    // OpenVM's default and maximum 4. Fewer makes each leaf trace smaller: the GPU memory peak of a run with many
    // segments (results/gpu_a40_memory: the leaf layer's rs_code_matrix, +2.3 GiB).
    if let Some(agg) = leaf_children_agg(e)? {
        prover.agg_prover = agg;
    }
    lap("stark prover");
    let has_deferrals = prover.deferral_setup.hook_commit().is_some();
    let memory_dimensions = prover.app_prover.memory_dimensions();
    let cap = own_cap(e);
    let mut inst = instance(e, exe, None)?;
    lap("instance");
    let segs = segments(&mut inst, &prove)?;
    lap("metered execution");
    let runs = plan(&segs, workers.len() + 1);
    let mine = *runs.last().unwrap();
    eprintln!("segpar: {} segments, runs {:?} ({} on workers, the last here)", segs.len(), runs, runs.len() - 1);
    // the initial memory's Merkle tree, before execution (StarkProver::prove: needed for the deferral proofs)
    let initial_tree = has_deferrals.then(|| {
        MerkleTree::from_memory(&inst.state().as_ref().unwrap().memory.memory, &memory_dimensions, &vm_poseidon2_hasher())
    });
    lap("initial memory tree");
    if !workers.is_empty() { write_object_to_file(stdin_file, &prove)?; }
    lap("stdin written");
    let tmp = stdin_file.with_extension("segs");
    std::fs::create_dir_all(&tmp)?;
    let n_segs = segs.len();
    let (theirs, ours) = std::thread::scope(|s| -> (Result<Vec<Proof<SC>>>, Result<Vec<Proof<SC>>>) {
        let hs: Vec<_> = runs[..runs.len() - 1]
            .iter()
            .zip(workers)
            .enumerate()
            .map(|(i, (&(lo, hi), w))| {
                let out = tmp.join(format!("run{i}.bin"));
                s.spawn(move || -> Result<Vec<Proof<SC>>> {
                    let n = w.prove_segments(stdin_file, lo, hi, cap, &out)?;
                    lap(&format!("worker run {lo}..{hi} done"));
                    ensure!(n == n_segs, "a worker cut {n} segments, this process {n_segs}");
                    let p: Vec<Proof<SC>> = read_object_from_file(&out)?;
                    ensure!(p.len() == hi - lo, "worker run {lo}..{hi}: {} proofs", p.len());
                    Ok(p)
                })
            })
            .collect();
        // this process's run (the last), while the workers prove theirs
        let ours = run(&mut inst, &segs, mine.0, mine.1);
        lap(&format!("own run {}..{} done", mine.0, mine.1));
        let mut all = Vec::new();
        for h in hs {
            match h.join() {
                Ok(Ok(p)) => all.extend(p),
                Ok(Err(err)) => return (Err(err), ours),
                Err(_) => return (Err(eyre!("segment worker panicked")), ours),
            }
        }
        (Ok(all), ours)
    });
    let mut per_segment = theirs?;
    per_segment.extend(ours?);
    ensure!(per_segment.len() == segs.len(), "{} segment proofs for {} segments", per_segment.len(), segs.len());
    let _ = std::fs::remove_dir_all(&tmp);

    // prove_continuations' tail: the user public values from the final memory and the device's top tree
    let final_memory = &inst.state().as_ref().unwrap().memory.memory;
    let top = inst.vm.memory_top_tree().ok_or_else(|| eyre!("memory top tree"))?;
    let cfg = inst.vm.config().as_ref();
    let user_public_values = UserPublicValuesProof::compute(
        cfg.memory_config.memory_dimensions(),
        cfg.num_public_values,
        &vm_poseidon2_hasher(),
        final_memory,
        top,
    );
    let continuation_proof = ContinuationVmProof { per_segment, user_public_values };
    lap("app proof assembled");

    // StarkProver::prove from here on, unchanged
    let (mut stark_proof, mut internal_metadata) = prover.agg_prover.prove_vm(continuation_proof)?;
    lap("leaf/internal aggregation");
    if def_inputs.iter().any(|input| !input.is_empty()) {
        let def_agg_prover = prover
            .deferral_setup
            .prover()
            .ok_or_else(|| eyre!("non-empty deferral inputs require a deferral aggregation prover"))?;
        let def_hook_proofs = def_agg_prover.multi_deferral_circuit_prover.prove(def_inputs)?;
        let (def_proof, def_internal_recursive_layer) = def_agg_prover.agg_prover.prove_def(def_hook_proofs)?;
        stark_proof =
            prover.agg_prover.prove_mixed(stark_proof, def_proof, &mut internal_metadata, def_internal_recursive_layer)?;
    }
    lap("deferral proofs");
    stark_proof = prover.agg_prover.wrap_proof(stark_proof, &mut internal_metadata)?;
    lap("wrap");
    if has_deferrals {
        let final_tree = MerkleTree::from_memory(final_memory, &memory_dimensions, &vm_poseidon2_hasher());
        let def_pvs: &DeferralPvs<F> = std::borrow::Borrow::borrow(stark_proof.inner.public_values[DEF_PVS_AIR_ID].as_slice());
        let depth = def_pvs.depth.as_canonical_u32() as usize;
        stark_proof.deferral_merkle_proofs = Some(compute_deferral_merkle_proofs(
            memory_dimensions,
            initial_tree.as_ref().unwrap(),
            &final_tree,
            depth,
        ));
    }
    lap("deferral merkle proofs");
    let baseline = prover.generate_baseline();
    Ok((stark_proof, baseline))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seg(n: u64) -> Segment { Segment { instret_start: 0, num_insns: n, trace_heights: vec![] } }
    #[test]
    fn plan_covers_every_segment_once_in_order() {
        for n in 1..12 {
            for k in 1..6 {
                let segs: Vec<Segment> = (0..n).map(|i| seg(1 + (i as u64 * 7919) % 13)).collect();
                let r = plan(&segs, k);
                assert!(!r.is_empty() && r.len() <= k.min(n));
                assert_eq!(r[0].0, 0);
                assert_eq!(r.last().unwrap().1, n);
                for w in r.windows(2) { assert_eq!(w[0].1, w[1].0); }
                assert!(r.iter().all(|x| x.0 < x.1));
            }
        }
    }
}
