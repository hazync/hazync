// ⚠ GUEST-COMPILED with feature xver-v3_0_5. RISC Zero 3.0.5's recursion circuit (risc0-circuit-recursion 4.0.5): the
// files control_id.rs, info.rs, poly_ext.rs and taps.rs are copied VERBATIM from that crate (generated; never edit
// them). This file supplies what the crate's lib.rs did. See ../mod.rs.
//
// ⛔ TEST VECTOR ONLY. RISC Zero 3.1.0 fixed soundness bugs in 3.0.x, so no product image may pin a 3.0.5 predecessor
// (docs/HANDOVER.md §4). It is here because real 3.0.5 receipts exist to test the cross-version path against.
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

/// SuccinctReceiptVerifierParameters digest of RISC Zero 3.0.5 (the `verifier_parameters` its succinct receipts
/// carry). `host xver-selftest` recomputes it from this circuit and checks it against real 3.0.5 receipts.
pub const PARAMS: [u8; 32] = super::hex32("ece5e9b8ae2cd6ea6b1827b464ff0348f9a7f4decd269c0087fdfd75098da013");

pub const VERSION: super::Version = super::Version {
    name: "risc0 3.0.5",
    params: PARAMS,
    control_root: control_id::ALLOWED_CONTROL_ROOT,
    circuit_info: <CircuitImpl as CircuitInfo>::CIRCUIT_INFO,
    verify_seal: |suite, seal, check| super::verify_seal_with(&CIRCUIT, suite, seal, check),
    output_size: <CircuitImpl as CircuitInfo>::OUTPUT_SIZE,
};
