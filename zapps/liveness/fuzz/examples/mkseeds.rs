//! Writes the seed corpus of every target from the zAPP's real test inputs: `cargo run --release --example mkseeds`
//! (from fuzz/). Seeds are committed; rerunning rewrites the same files.
use zapp_liveness_fuzz::*;
fn put(target: &str, name: &str, b: &[u8]) {
    let d = format!("{}/corpus/{target}", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(format!("{d}/{name}"), b).unwrap();
}
fn main() {
    for c in chains() {
        for (i, (b, q)) in c.proofs.iter().enumerate() {
            let h = swiftsync_verify::hex(b);
            put("liveness_proof", &format!("{}_{i}.bin", c.name), b);
            put("liveness_proof", &format!("{}_{i}.hex", c.name), h.as_bytes());
            let f = serde_json::json!({"proof": h, "ask": q}).to_string();
            put("liveness_fact", &format!("{}_{i}.json", c.name), f.as_bytes());
            put("liveness_ask", &format!("{}_{i}.txt", c.name), q.as_bytes());
        }
    }
    let extra = ["0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9:0@200=spent:170,value=5000000000",
                 "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16:0@tip",
                 "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16:1@200=spent:181:1:0,created=170,coinbase=0,script=76a9,value=1"];
    for (i, q) in extra.iter().enumerate() { put("liveness_ask", &format!("extra_{i}.txt"), q.as_bytes()); }
    for (t, f) in [("liveness_proof", proof as fn(&[u8])), ("liveness_fact", fact), ("liveness_ask", ask)] {
        let d = format!("{}/corpus/{t}", env!("CARGO_MANIFEST_DIR"));
        for e in std::fs::read_dir(&d).unwrap() { f(&std::fs::read(e.unwrap().path()).unwrap()); }
        println!("{t}: seeds hold");
    }
}
