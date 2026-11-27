// Diagnostic (27.11.2026, engine/VERIFIER.md "Which key bytes a proof binds"): cargo run --release --example vktail -- <vk.bin> <proof.hze> <image_id> [n]
// Which bytes of a key file the verifier does not bind: for each of the last N bytes, flip it, decode, compare the
// decoded key with the original (Debug), re-encode, and run the full check.
use hazync_engine_verify as ev;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let vk = std::fs::read(&a[1]).unwrap();
    let file = std::fs::read(&a[2]).unwrap();
    let id = std::fs::read_to_string(&a[3]).unwrap();
    let id: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&id.trim()[2 * i..2 * i + 2], 16).unwrap()).collect();
    let (proof, journal) = ev::unpack(&file).unwrap();
    let orig = ev::decode_vk(&vk).unwrap();
    let od = format!("{orig:?}");
    let re = bitcode::serialize(&orig).unwrap();
    println!("key {} bytes; re-encoded {} bytes, identical: {}", vk.len(), re.len(), re == vk);
    let n: usize = a.get(4).map(|x| x.parse().unwrap()).unwrap_or(24);
    for i in vk.len() - n..vk.len() {
        let mut m = vk.clone();
        m[i] ^= 0x01;
        let r = match ev::decode_vk(&m) {
            Err(_) => "decode refused".to_string(),
            Ok(k) => {
                let d = format!("{k:?}");
                let diff = od.bytes().zip(d.bytes()).position(|(x, y)| x != y);
                let what = match diff { None if od.len() == d.len() => "decoded key IDENTICAL".to_string(),
                    _ => { let p = diff.unwrap_or(od.len().min(d.len())); let s = p.saturating_sub(80);
                           format!("differs near: ...{}", &d[s..(p + 40).min(d.len())]) } };
                let v = ev::verify_full_pinned(&k, proof, journal, &id);
                format!("{what} | verify: {}", if v.is_ok() { "OK" } else { "refused" })
            }
        };
        println!("byte -{:>2} ({i}): {r}", vk.len() - i);
    }
}
