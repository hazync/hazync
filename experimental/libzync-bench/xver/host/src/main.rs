//! `xver-host <receipt> [<image hex>]`: the cross-version handover verifier (prover/methods/guest/src/xver) run in
//! the RISC Zero 3.1.0 executor on a real succinct receipt, then each negative control (one seal word, the image, the
//! journal, the version) — each must make the guest panic. Prints cycles. Exit 0 only if every case behaves.
use risc0_zkvm::{default_executor, sha::Digestible, ExecutorEnv, InnerReceipt, Receipt};

#[path = "../../../../../prover/methods/guest/src/xver/mod.rs"]
mod xver;

fn run(params: &[u8; 32], image: [u32; 8], journal: &[u8], words: &[u32]) -> Result<(u64, u64, usize), String> {
    let env = ExecutorEnv::builder()
        .write(params).unwrap()
        .write(&image).unwrap()
        .write(&journal.to_vec()).unwrap()
        .write(&(words.len() as u32)).unwrap()
        .write_slice(words)
        .build().unwrap();
    let s = default_executor().execute(env, xver_methods::GUEST_ELF).map_err(|e| format!("{e}"))?;
    let (_, _, verify_cycles): ([u8; 32], [u32; 8], u64) = s.journal.decode().map_err(|e| e.to_string())?;
    Ok((verify_cycles, s.cycles(), s.segments.len()))
}

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("xver-host <receipt> [<image hex>]");
    let bytes = std::fs::read(path).expect("read receipt");
    let r: Receipt = bincode::deserialize(&bytes).expect("a bincode Receipt");
    let InnerReceipt::Succinct(s) = &r.inner else { panic!("{path}: not a succinct receipt") };
    let params: [u8; 32] = s.verifier_parameters.as_bytes().try_into().unwrap();
    let v = xver::version(&params).unwrap_or_else(|| panic!("verifier parameters {} are not vendored", s.verifier_parameters));
    let claim = s.claim.as_value().expect("claim");
    let image: [u32; 8] = match args.get(2) {
        Some(h) => {
            let b: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap()).collect();
            risc0_zkvm::sha::Digest::try_from(&b[..]).unwrap().into()
        }
        None => claim.pre.digest().into(),
    };
    assert!(xver::expected_claim(image, &r.journal.bytes) == s.claim.digest(), "the receipt's claim is not ok(image, journal)");
    let words = xver::encode(s);
    println!("receipt {path}: {} bytes, {} ({}), image {}, journal {} bytes, seal {} words",
             bytes.len(), v.name, hex(&params), hex(&risc0_zkvm::sha::Digest::from(image).as_bytes()), r.journal.bytes.len(), s.seal.len());
    let mut bad = 0;
    match run(&params, image, &r.journal.bytes, &words) {
        Ok((vc, total, seg)) => println!("  VALID: verifies in the guest: {vc} verify cycles, {total} total cycles, {seg} segments"),
        Err(e) => { println!("  VALID: REFUSED ({e})"); bad += 1; }
    }
    let mut seal = words.clone();
    let i = seal.len() - 100;
    seal[i] ^= 1;
    let mut img2 = image;
    img2[0] ^= 1;
    let mut j2 = r.journal.bytes.clone();
    j2[0] ^= 1;
    let other: [u8; 32] = xver::KNOWN.iter().map(|k| k.params).find(|p| *p != params).expect("two versions vendored");
    let cases: [(&str, [u8; 32], [u32; 8], &[u8], &[u32]); 4] = [
        ("one seal word changed", params, image, &r.journal.bytes, &seal),
        ("wrong predecessor image", params, img2, &r.journal.bytes, &words),
        ("wrong journal", params, image, &j2, &words),
        ("wrong RISC Zero version", other, image, &r.journal.bytes, &words),
    ];
    for (name, p, im, j, w) in cases {
        match run(&p, im, j, w) {
            Err(e) => println!("  negative control, {name}: REFUSED ({})", e.lines().last().unwrap_or("")),
            Ok(_) => { println!("  negative control, {name}: ACCEPTED (BUG)"); bad += 1; }
        }
    }
    if bad > 0 { println!("XVER-GUEST-FAIL {bad}"); std::process::exit(1); }
    println!("XVER-GUEST-OK");
}
