use methods::{GUEST_ELF, GUEST_ID};
use risc0_zkvm::{default_executor, default_prover, ExecutorEnv, ProverOpts};
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (mode, path, action) = (a[1].parse::<u8>().unwrap(), &a[2], a[3].as_str());
    let mut input = vec![mode];
    input.extend(std::fs::read(path).unwrap());
    let env = ExecutorEnv::builder().write_slice(&input).build().unwrap();
    match action {
        "exec" => {
            let t = Instant::now();
            let info = default_executor().execute(env, GUEST_ELF).unwrap();
            let total: u64 = info.segments.iter().map(|s| 1u64 << s.po2).sum();
            println!("RESULT vm=risc0 mode={mode} action=exec user_cycles={} padded_cycles={total} segments={} secs={:.2} out={}",
                info.cycles(), info.segments.len(), t.elapsed().as_secs_f64(), hex(&info.journal.bytes));
        }
        "prove" => {
            let t = Instant::now();
            let p = default_prover().prove_with_opts(env, GUEST_ELF, &ProverOpts::composite()).unwrap();
            let secs = t.elapsed().as_secs_f64();
            p.receipt.verify(GUEST_ID).unwrap();
            println!("RESULT vm=risc0 mode={mode} action=prove user_cycles={} total_cycles={} segments={} secs={secs:.2} out={}",
                p.stats.user_cycles, p.stats.total_cycles, p.stats.segments, hex(&p.receipt.journal.bytes));
        }
        _ => panic!("action"),
    }
}
fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
