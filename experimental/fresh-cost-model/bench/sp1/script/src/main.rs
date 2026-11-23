use sp1_sdk::prelude::*;
use sp1_sdk::ProverClient;
use std::time::Instant;

const ELF: Elf = include_elf!("bench-program");

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (mode, path, action) = (a[1].parse::<u8>().unwrap(), &a[2], a[3].as_str());
    let mut input = vec![mode];
    input.extend(std::fs::read(path).unwrap());
    let mut stdin = SP1Stdin::new();
    stdin.write_vec(input);
    let client = ProverClient::from_env().await;
    match action {
        "exec" => {
            let t = Instant::now();
            let (pv, report) = client.execute(ELF, stdin).await.unwrap();
            println!("RESULT vm=sp1 mode={mode} action=exec instructions={} syscalls={} gas={:?} secs={:.2} out={}",
                report.total_instruction_count(), report.total_syscall_count(), report.gas(),
                t.elapsed().as_secs_f64(), hex(pv.as_slice()));
        }
        "prove" => {
            let pk = client.setup(ELF).await.unwrap();
            let t = Instant::now();
            let proof = client.prove(&pk, stdin).core().await.unwrap();
            let secs = t.elapsed().as_secs_f64();
            client.verify(&proof, pk.verifying_key(), None).expect("verify");
            println!("RESULT vm=sp1 mode={mode} action=prove secs={secs:.2} out={}", hex(proof.public_values.as_slice()));
        }
        _ => panic!("action"),
    }
}
fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
