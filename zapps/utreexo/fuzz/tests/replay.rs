//! Replays the committed corpus (fuzz/corpus/<target>/) through the same properties the fuzz targets run, on stable
//! Rust, without libFuzzer: `cargo test --release --manifest-path fuzz/Cargo.toml`. Crash inputs found by a round are
//! kept under fuzz/artifacts/<target>/ and move to the corpus only once their fix has landed.
fn replay(target: &str, f: fn(&[u8])) {
    let dir = format!("{}/corpus/{target}", env!("CARGO_MANIFEST_DIR"));
    let mut n = 0;
    for e in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{dir}: {e}")) {
        let p = e.unwrap().path();
        f(&std::fs::read(&p).unwrap());
        n += 1;
    }
    assert!(n > 0, "{dir}: an empty corpus replays nothing");
    eprintln!("{target}: {n} inputs replayed");
}

#[test] fn utreexo_decode() { replay("utreexo_decode", zapp_utreexo_fuzz::decode) }
#[test] fn utreexo_verify() { replay("utreexo_verify", zapp_utreexo_fuzz::verify) }
