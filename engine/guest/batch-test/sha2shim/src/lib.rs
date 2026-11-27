//! engine/platform/sha2's API (Digest::new / update / finalize -> [u8; 32] / digest) over RustCrypto's sha2, for the
//! host: the guest's shim needs OpenVM's SHA-256 extension. Both are FIPS 180-4 SHA-256.
use realsha2::Digest as _;
pub struct Sha256(realsha2::Sha256);
pub trait Digest: Sized {
    fn new() -> Self;
    fn update(&mut self, data: impl AsRef<[u8]>);
    fn finalize(self) -> [u8; 32];
    fn digest(data: impl AsRef<[u8]>) -> [u8; 32] { let mut h = Self::new(); h.update(data); h.finalize() }
}
impl Digest for Sha256 {
    fn new() -> Self { Sha256(realsha2::Sha256::new()) }
    fn update(&mut self, data: impl AsRef<[u8]>) { self.0.update(data.as_ref()) }
    fn finalize(self) -> [u8; 32] { self.0.finalize().into() }
}
