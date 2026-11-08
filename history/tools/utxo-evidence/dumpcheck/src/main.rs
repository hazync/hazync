use hazync_verify::{hazync_check_utxo_dump, hazync_verify_proof, HazyncState};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 3 {
        eprintln!("usage: dumpcheck <spine.bin> <dump.hzutxo>");
        std::process::exit(2);
    }
    let proof = std::fs::read(&a[1]).expect("read proof");
    let dump = std::fs::read(&a[2]).expect("read dump");
    let mut st: HazyncState = unsafe { std::mem::zeroed() };
    let rc = unsafe { hazync_verify_proof(proof.as_ptr(), proof.len(), &mut st) };
    println!("hazync_verify_proof rc={rc} height={} utxo_leaves={} roots={}",
             st.height, st.utxo_leaves, st.root_count);
    if rc != 0 {
        std::process::exit(1);
    }
    let t = std::time::Instant::now();
    let rc2 = unsafe { hazync_check_utxo_dump(dump.as_ptr(), dump.len(), &st) };
    println!("hazync_check_utxo_dump rc={rc2} ({:.1}s, {} bytes)", t.elapsed().as_secs_f64(), dump.len());
    std::process::exit(if rc2 == 0 { 0 } else { 1 });
}
