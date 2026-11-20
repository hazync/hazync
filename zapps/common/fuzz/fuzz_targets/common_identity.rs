#![no_main]
// docs/libzync/ZAPP_TESTING.md §3.4, §5: the property is zapp_common_fuzz::identity (src/lib.rs).
libfuzzer_sys::fuzz_target!(|data: &[u8]| zapp_common_fuzz::identity(data));
