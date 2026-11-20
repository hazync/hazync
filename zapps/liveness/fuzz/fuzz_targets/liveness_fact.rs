#![no_main]
// docs/libzync/ZAPP_TESTING.md §3.4: the property is zapp_liveness_fuzz::fact (src/lib.rs).
libfuzzer_sys::fuzz_target!(|data: &[u8]| zapp_liveness_fuzz::fact(data));
