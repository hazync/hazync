//! Review R3-5 L4 (28.11.2026): corecheck's two caches are bounded. Each entry is a leaked Image (and, for a key, a
//! copy of the key, ~96 KB for a real one), so a long-running caller fed distinct caller-chosen ids or keys grew without
//! limit. Past the cap a NEW entry is refused (an error); entries already cached still answer. Its own test binary: the
//! caches are process-wide.
use sha2::Digest as _;
use zapp_corecheck::{image_for, with_key, MAX_CACHED_IMAGES, MAX_CACHED_KEYS};

// One #[test]: the two caches are process-wide and the key check resolves an image too, so the steps run in order.
#[test]
fn caches_are_bounded() {
    key_cache_is_bounded();
    image_cache_is_bounded();
}

fn image_cache_is_bounded() {
    // the key test above already resolved the signet image: fill the rest of the cap with mainnet ids
    let id = |i: usize| format!("{i:064x}");
    let mut taken = 0;
    for i in 0..MAX_CACHED_IMAGES {
        if image_for("mainnet", &id(i), false, false).is_ok() { taken += 1 } else { break }
    }
    assert_eq!(taken, MAX_CACHED_IMAGES - 1, "the cap counts every distinct image");
    let e = image_for("mainnet", &id(MAX_CACHED_IMAGES + 7), false, false).err().expect("a new image past the cap is refused");
    assert!(e.contains("distinct images"), "{e}");
    assert!(image_for("mainnet", &id(3), false, false).is_ok(), "a cached image still answers past the cap");
}

fn key_cache_is_bounded() {
    let img = image_for("signet", "", false, false).expect("the signet image");
    let key = |i: usize| format!("not a real key {i}").into_bytes();
    let pin = |k: &[u8]| -> [u8; 32] { sha2::Sha256::digest(k).into() };
    for i in 0..MAX_CACHED_KEYS {
        let k = key(i);
        assert!(with_key(img, &k, &pin(&k)).is_ok(), "key {i} under the cap");
    }
    let k = key(MAX_CACHED_KEYS);
    let e = with_key(img, &k, &pin(&k)).err().expect("a new key past the cap is refused");
    assert!(e.contains("distinct (image, key) pairs"), "{e}");
    let k0 = key(0);
    assert!(with_key(img, &k0, &pin(&k0)).is_ok(), "a cached key still answers past the cap");
    let wrong = [0u8; 32];
    assert!(with_key(img, &k0, &wrong).is_err(), "the pin is still checked first");
}
