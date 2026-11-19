#![no_main]
// docs/libzync/ZAPP_TESTING.md §3.4: the property is zapp_utreexo_fuzz::verify (src/lib.rs).
libfuzzer_sys::fuzz_target!(|data: &[u8]| zapp_utreexo_fuzz::verify(data));
