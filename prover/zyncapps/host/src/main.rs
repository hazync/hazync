//! `zyncapp`: proves and verifies zyncAPP receipts (docs/libzync/ZYNCAPPS.md §5).
//!
//!   zyncapp prove-coin <core receipt> <fact.json from zync-path coin, at = tip> <out receipt> [--signet]
//!   zyncapp verify-coin <zyncapp receipt>
//!   zyncapp id
//!
//! The core receipt is an ASSUMPTION of the zyncAPP receipt (risc0 composition): the zyncAPP receipt's verification
//! covers the core proof. The verifier still pins the committed core image id and anchor (policy).
use risc0_zkvm::{default_prover, ExecutorEnv, Receipt};
use serde::{Deserialize, Serialize};
use zyncapp_methods::{ZYNCAPP_COIN_ELF, ZYNCAPP_COIN_ID};

#[derive(Deserialize)]
#[allow(dead_code)]
struct ChainState {
    kind: u32, version: u32, tip_hash: [u8; 32], utxo_roots: Vec<Option<[u8; 32]>>, utxo_leaves: u64, utxo_value: u64,
    cum_work: [u8; 32], height: u32, prev_nbits: u32, prev_time: u32, epoch_start: u32, recent_times: Vec<u32>,
    anchor_id: [u8; 32], self_id: [u32; 8], ss_set: u32, ss_acc: [u32; 16], ss_hints: [u8; 32], smt_root: [u8; 32],
    max_time: u32, profile: u32, block_roots: Vec<Option<[u8; 32]>>, block_leaves: u64, lineage: [u8; 32],
}

/// zync-path's CoinFact (swiftsync-verify), the fields this zyncAPP uses.
#[derive(Deserialize)]
struct CoinFact { at: u32, pos: u64, coin: String, spent_by: Option<(u32, u32, u32)>, path: Vec<String> }

#[derive(Serialize, Deserialize)]
struct CoinQuery { pos: u64, coin: [u8; 32], spent_by: Option<(u32, u32, u32)>, path: Vec<[u8; 32]> }

#[derive(Serialize, Deserialize, Debug)]
struct ZyncCoin {
    kind: u32, core_id: [u32; 8], lineage: [u8; 32], anchor_id: [u8; 32], height: u32, tip_hash: [u8; 32],
    cum_work: [u8; 32], max_time: u32, profile: u32, pos: u64, coin: [u8; 32], spent_by: Option<(u32, u32, u32)>,
}

fn die(m: &str) -> ! { eprintln!("zyncapp: {m}"); std::process::exit(1) }
fn h32(s: &str) -> [u8; 32] {
    let v: Vec<u8> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2).unwrap_or("zz"), 16).unwrap_or_else(|_| die("bad hex"))).collect();
    v.try_into().unwrap_or_else(|_| die("not 32 bytes"))
}
fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
fn id_hex(w: &[u32; 8]) -> String { hex(&w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()) }

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(|s| s.as_str()) {
        Some("id") => println!("ZYNCAPP_COIN_ID {}", id_hex(&ZYNCAPP_COIN_ID)),
        Some("prove-coin") if a.len() >= 4 => {
            let core: Receipt = bincode::deserialize(&std::fs::read(&a[1]).unwrap_or_else(|e| die(&e.to_string())))
                .unwrap_or_else(|e| die(&format!("core receipt: {e}")));
            let cs: ChainState = core.journal.decode().unwrap_or_else(|e| die(&format!("core journal: {e}")));
            let f: CoinFact = serde_json::from_slice(&std::fs::read(&a[2]).unwrap_or_else(|e| die(&e.to_string())))
                .unwrap_or_else(|e| die(&format!("fact: {e}")));
            if f.at != cs.height { die("this zyncAPP answers at the core proof's tip: make the fact with at = tip"); }
            let q = CoinQuery { pos: f.pos, coin: h32(&f.coin), spent_by: f.spent_by, path: f.path.iter().map(|x| h32(x)).collect() };
            let signet = a.iter().any(|x| x == "--signet");
            let env = ExecutorEnv::builder()
                .add_assumption(core.clone())
                .write(&cs.self_id).unwrap()
                .write(&core.journal.bytes).unwrap()
                .write(&signet).unwrap()
                .write(&q).unwrap()
                .build().unwrap();
            let r = default_prover().prove(env, ZYNCAPP_COIN_ELF).unwrap_or_else(|e| die(&format!("prove: {e}"))).receipt;
            std::fs::write(&a[3], bincode::serialize(&r).unwrap()).unwrap_or_else(|e| die(&e.to_string()));
            println!("zyncapp coin receipt written: {}", a[3]);
        }
        Some("verify-coin") if a.len() >= 2 => {
            let r: Receipt = bincode::deserialize(&std::fs::read(&a[1]).unwrap_or_else(|e| die(&e.to_string())))
                .unwrap_or_else(|e| die(&format!("receipt: {e}")));
            if matches!(r.inner, risc0_zkvm::InnerReceipt::Fake(_)) { die("a dev-mode FakeReceipt is not a proof"); }
            r.verify(ZYNCAPP_COIN_ID).unwrap_or_else(|e| die(&format!("does not verify: {e}")));
            let z: ZyncCoin = r.journal.decode().unwrap_or_else(|e| die(&format!("journal: {e}")));
            if z.kind != 0x5A43_0001 { die("not a zyncAPP coin statement"); }
            let mut t = z.tip_hash;
            t.reverse();
            let what = match z.spent_by { None => "UNSPENT".to_string(), Some((h, tx, v)) => format!("SPENT at height {h} by tx {tx} input {v}") };
            println!("ZYNCAPP_COIN_OK: coin {} at log position {} is {what} at height {} (tip {})", hex(&z.coin), z.pos, z.height, hex(&t));
            println!("  rests on core image {} with anchor_id {} and lineage {}", id_hex(&z.core_id), hex(&z.anchor_id), hex(&z.lineage));
            println!("  ⚠ check that core image is one you trust and anchor_id is the network's genesis anchor (swiftsync-verify IMAGES)");
        }
        _ => die("usage: zyncapp id | prove-coin <core.rcpt> <fact.json> <out.rcpt> [--signet] | verify-coin <zync.rcpt>"),
    }
}
