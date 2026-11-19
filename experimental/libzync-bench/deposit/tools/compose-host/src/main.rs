//! `deposit-compose <deposit.json> [--prove <out.rcpt>]`: the composed deposit guest (DEPOSIT_ZAPP.md §2, option (b))
//! on a deposit bundle from `zapp-deposit build` (one header receipt from genesis, a block at or below the core tip).
//! Executes it through r0vm (cycles, segments), or with --prove proves it (succinct, on the CPU unless r0vm has a GPU),
//! verifies the receipt against the guest's image id and prints the time and size. A measurement tool.
use deposit_compose_methods::{DEPOSIT_COMPOSE_ELF, DEPOSIT_COMPOSE_ID};
use risc0_zkvm::{default_executor, default_prover, ExecutorEnv, ProverOpts, Receipt};
use serde::Serialize;
use zapp_deposit::DepositBundle;

#[derive(Serialize)]
struct Input {
    core_id: [u32; 8],
    core_journal: Vec<u8>,
    headers_id: [u32; 8],
    header_journal: Vec<u8>,
    core_tip_mmr_path: Vec<[u8; 32]>,
    height: u32,
    header: Vec<u8>,
    state: [u8; 32],
    undo: Option<[u8; 32]>,
    forest_path: Vec<[u8; 32]>,
    tx: Vec<u8>,
    index: u32,
    branch: Vec<[u8; 32]>,
    coinbase: Vec<u8>,
    coinbase_branch: Vec<[u8; 32]>,
}

fn h(x: &str) -> [u8; 32] { swiftsync_verify::hex32(x).expect("a hash") }
fn hs(v: &[String]) -> Vec<[u8; 32]> { v.iter().map(|x| h(x)).collect() }
fn id_of(r: &Receipt) -> [u32; 8] {
    use risc0_zkvm::sha::Digestible;
    let d = r.claim().unwrap().as_value().unwrap().pre.digest();
    let mut w = [0u32; 8];
    w.copy_from_slice(d.as_words());
    w
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let b: DepositBundle = serde_json::from_slice(&std::fs::read(&a[0]).expect("read bundle")).expect("a deposit bundle");
    let core: Receipt = bincode::deserialize(&swiftsync_verify::unhex(&b.core_receipt).unwrap()).unwrap();
    assert_eq!(b.header_receipts.len(), 1, "one header receipt from genesis");
    let hdr: Receipt = bincode::deserialize(&swiftsync_verify::unhex(&b.header_receipts[0]).unwrap()).unwrap();
    let f = b.block.forest.as_ref().expect("a block at or below the core tip");
    let input = Input {
        core_id: id_of(&core), core_journal: core.journal.bytes.clone(), headers_id: id_of(&hdr), header_journal: hdr.journal.bytes.clone(),
        core_tip_mmr_path: hs(&b.core_tip_mmr_path), height: b.block.height, header: swiftsync_verify::unhex(&b.block.header).unwrap(),
        state: h(&f.state), undo: f.undo.as_ref().map(|u| h(u)), forest_path: hs(&f.path), tx: swiftsync_verify::unhex(&b.block.tx).unwrap(),
        index: b.block.index, branch: hs(&b.block.branch), coinbase: swiftsync_verify::unhex(&b.block.coinbase).unwrap(),
        coinbase_branch: hs(&b.block.coinbase_branch),
    };
    let env = || ExecutorEnv::builder().add_assumption(core.clone()).add_assumption(hdr.clone()).write(&input).unwrap().build().unwrap();
    let id_hex: String = DEPOSIT_COMPOSE_ID.iter().flat_map(|w| w.to_le_bytes()).map(|x| format!("{x:02x}")).collect();
    let t = std::time::Instant::now();
    let info = default_executor().execute(env(), DEPOSIT_COMPOSE_ELF).expect("execute");
    println!("COMPOSE_EXEC image {id_hex}: {} cycles (user {}), {} segment(s), journal {} bytes, executed in {:.2} s",
             info.cycles(), info.segments.iter().map(|s| s.cycles as u64).sum::<u64>(), info.segments.len(), info.journal.bytes.len(), t.elapsed().as_secs_f64());
    if let Some(out) = a.iter().position(|x| x == "--prove").and_then(|i| a.get(i + 1)) {
        let t = std::time::Instant::now();
        let r = default_prover().prove_with_opts(env(), DEPOSIT_COMPOSE_ELF, &ProverOpts::succinct()).expect("prove").receipt;
        let secs = t.elapsed().as_secs_f64();
        r.verify(DEPOSIT_COMPOSE_ID).expect("the composed receipt verifies");
        let bytes = bincode::serialize(&r).unwrap();
        std::fs::write(out, &bytes).unwrap();
        println!("COMPOSE_PROVE succinct receipt {} bytes (journal {} bytes) in {secs:.1} s; verifies against {id_hex}", bytes.len(), r.journal.bytes.len());
    }
}
