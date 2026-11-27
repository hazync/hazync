// The proof system's soundness for THIS build's circuits (ROADMAP 4.4): stark-backend's SoundnessCalculator on every
// verifying key in a keys dir (the app VM, leaf, internal, the internal-recursive layer, the deferral circuits, and
// the final key readers pin, vk.bin). calculate_from_vk takes the bounds from the key itself (AIR count, constraints
// and interactions per AIR, trace columns), not the SDK's example assumptions.
//   cargo run --release --example soundness -- <keys dir> [<vk.bin, the key readers pin>]
use openvm_sdk::{fs::read_object_from_file, keygen::SdkCachedProvingKey, SC};
use openvm_sdk_config::SdkVmConfig;
use openvm_stark_backend::{keygen::types::MultiStarkVerifyingKey, soundness::SoundnessCalculator};

fn report(name: &str, vk: &MultiStarkVerifyingKey<SC>) {
    let n = vk.inner.per_air.len();
    let (mut mc, mut mi, mut cols) = (0, 0, 0);
    for a in &vk.inner.per_air {
        mc = mc.max(a.symbolic_constraints.constraints.constraint_idx.len());
        mi = mi.max(a.symbolic_constraints.interactions.len());
        cols += a.params.width.total_width();
    }
    let s = SoundnessCalculator::calculate_from_vk(vk);
    println!("{name:<28} airs {n:>3}  max constraints/air {mc:>5}  max interactions/air {mi:>4}  columns {cols:>6}  log_stacked_height {:>2} | total {:.1} bits (logup {:.1}, gkr {:.1}/{:.1}, zerocheck {:.1}, batching {:.1}, stacked {:.1}, whir {:.1})",
             vk.inner.params.log_stacked_height(), s.total_bits, s.logup_bits, s.gkr_sumcheck_bits, s.gkr_batching_bits,
             s.zerocheck_sumcheck_bits, s.constraint_batching_bits, s.stacked_reduction_bits, s.whir_bits);
}

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("soundness <keys dir>"));
    let pk: SdkCachedProvingKey<SdkVmConfig> = read_object_from_file(dir.join("sdk.pk")).expect("sdk.pk");
    report("app VM", &pk.app_pk.app_vm_pk.vm_pk.get_vk());
    report("leaf", &pk.agg_pk.prefix.leaf.get_vk());
    report("internal (for leaf)", &pk.agg_pk.prefix.internal_for_leaf.get_vk());
    report("internal recursive", &pk.agg_pk.internal_recursive.get_vk());
    if let Some(d) = &pk.deferral_pk {
        for (i, c) in d.circuits.iter().enumerate() {
            report(&format!("deferral circuit {i}: leaf"), &c.agg_prefix_pk.leaf.get_vk());
            report(&format!("deferral circuit {i}: internal"), &c.agg_prefix_pk.internal_for_leaf.get_vk());
        }
        report("deferral internal recursive", &d.def_internal_recursive_pk.get_vk());
        report("deferral hook", &d.def_hook_pk.get_vk());
    }
    if let Some(a) = &pk.deferral_agg_pk {
        report("deferral agg: leaf", &a.prefix.leaf.get_vk());
        report("deferral agg: internal rec.", &a.internal_recursive.get_vk());
    }
    // the key readers pin (vk.bin, as `hzengine pack --key` writes it), decoded as every reader decodes it (verify-core)
    if let Some(k) = std::env::args().nth(2) {
        let vk = hazync_engine_verify::decode_vk(&std::fs::read(&k).expect("vk.bin")).expect("vk.bin decodes");
        report("the reader's key (vk.bin)", &vk.mvk);
    }
}
