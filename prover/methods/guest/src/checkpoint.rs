// ⚠ GUEST-COMPILED (libzync builds only) — editing this file changes METHOD_ID.
// The checkpoint this image pins (docs/libzync/SWIFTSYNC_PROOFS.md step 3b): at CHECKPOINT.0 the block hash must be
// CHECKPOINT.1 (internal byte order), or there is no proof. With the verifiers' minimum height (= the checkpoint height),
// an OK verdict means the chain contains this block: a fork must branch after it. Chosen at freeze: a block about a
// month old, checked against the bridge node and mempool.space (11-03).
// mainnet 970000 = 000000000000000000014e6a9bad5f954f266cfb3e8f63f8c2fb7967f29b5a07
// signet  325000 = 000000057a363b7d1d713167191e272744532b507d919c81ec5a2c57e306fcd4
#[cfg(all(not(feature = "signet"), not(feature = "regtest")))]
pub const CHECKPOINT: Option<(u32, [u8; 32])> = Some((970_000, [0x07, 0x5a, 0x9b, 0xf2, 0x67, 0x79, 0xfb, 0xc2, 0xf8, 0x63, 0x8f, 0x3e, 0xfb, 0x6c, 0x26, 0x4f, 0x95, 0x5f, 0xad, 0x9b, 0x6a, 0x4e, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]));
#[cfg(feature = "signet")]
pub const CHECKPOINT: Option<(u32, [u8; 32])> = Some((325_000, [0xd4, 0xfc, 0x06, 0xe3, 0x57, 0x2c, 0x5a, 0xec, 0x81, 0x9c, 0x91, 0x7d, 0x50, 0x2b, 0x53, 0x44, 0x27, 0x27, 0x1e, 0x19, 0x67, 0x31, 0x71, 0x1d, 0x7d, 0x3b, 0x36, 0x7a, 0x05, 0x00, 0x00, 0x00]));
#[cfg(feature = "regtest")]
pub const CHECKPOINT: Option<(u32, [u8; 32])> = None;
