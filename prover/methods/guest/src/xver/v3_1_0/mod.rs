// ⚠ GUEST-COMPILED with feature xver-v3_1_0. RISC Zero 3.1.0's recursion circuit (risc0-circuit-recursion 4.1.0): the
// files control_id.rs, info.rs, poly_ext.rs and taps.rs are copied VERBATIM from that crate (generated; never edit
// them). This file supplies what the crate's lib.rs did. See ../mod.rs.
//
// 3.1.0 is the version the published images (8888d59c, d4210dae) run. When an image moves to a later RISC Zero, it
// enables this feature and pins its predecessor with `verifier: v3_1_0::PARAMS` (docs/HANDOVER.md §3). Until then a
// 3.1.0 image carrying it verifies its own version's receipts through it: the self-test of this path.
#![allow(dead_code, clippy::all)]

pub mod control_id;
mod info;
mod poly_ext;
mod taps;

use risc0_zkp::{
    adapter::{CircuitCoreDef, CircuitInfo, TapsProvider},
    field::baby_bear::BabyBear,
    taps::TapSet,
};

pub struct CircuitImpl;

impl TapsProvider for CircuitImpl {
    fn get_taps(&self) -> &'static TapSet<'static> {
        self::taps::TAPSET
    }
}

impl CircuitCoreDef<BabyBear> for CircuitImpl {}

pub const CIRCUIT: CircuitImpl = CircuitImpl;

/// SuccinctReceiptVerifierParameters digest of RISC Zero 3.1.0 (the `verifier_parameters` its succinct receipts
/// carry). `host xver-selftest` recomputes it from this circuit and checks it against a real 3.1.0 board receipt.
pub const PARAMS: [u8; 32] = super::hex32("f57f355f816d73b9c94cf5c63baa6c70a45fff10cfe3f298a9c2508dc4a06ae8");

pub const VERSION: super::Version = super::Version {
    name: "risc0 3.1.0",
    params: PARAMS,
    control_root: control_id::ALLOWED_CONTROL_ROOT,
    circuit_info: <CircuitImpl as CircuitInfo>::CIRCUIT_INFO,
    verify_seal: |suite, seal, check| super::verify_seal_with(&CIRCUIT, suite, seal, check),
    output_size: <CircuitImpl as CircuitInfo>::OUTPUT_SIZE,
};
