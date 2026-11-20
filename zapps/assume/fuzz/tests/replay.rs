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

#[test] fn assume_snapshot_decode() { replay("assume_snapshot_decode", zapp_assume_fuzz::snapshot_decode) }
#[test] fn assume_snapshot_check() { replay("assume_snapshot_check", zapp_assume_fuzz::snapshot_check) }
#[test] fn assume_side_inputs() { replay("assume_side_inputs", zapp_assume_fuzz::side_inputs) }
#[test] fn assume_core_receipt() { replay("assume_core_receipt", zapp_assume_fuzz::core_receipt) }
