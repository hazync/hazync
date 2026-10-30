//! Network parameters the host and guest both need. The guest asserts every value here against the
//! one Bitcoin Core compiles from its own chainparams.cpp (`assert_core_constants`), so a typo here
//! stops the guest on its first run rather than proving the wrong rules.
//!
//! Hashes are in INTERNAL byte order (dsha256(header) as computed, not the reversed display form).
//! Included by the guest via #[path]: no dependencies.

pub struct Net {
    pub name: &'static str,
    /// Display-order hex, as `getblockhash 0` prints it.
    pub genesis_hex: &'static str,
    pub bip34_height: u32,
    pub bip66_height: u32,
    pub bip65_height: u32,
    pub csv_height: u32,
    pub segwit_height: u32,
    /// powLimit in compact form.
    pub pow_limit_bits: u32,
    /// Core's IsBIP30Unspendable: (height, display-order hash) of the coinbases whose coins are never
    /// indexed by a hints file (their outpoints reappear at the BIP30 overwrite blocks).
    pub bip30_unspendable: &'static [(u32, &'static str)],
    /// Blocks are signed (BIP325): the aggregate checks the block solution against the challenge.
    pub signet: bool,
}

pub const MAINNET: Net = Net {
    name: "mainnet",
    genesis_hex: "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
    bip34_height: 227_931,
    bip66_height: 363_725,
    bip65_height: 388_381,
    csv_height: 419_328,
    segwit_height: 481_824,
    pow_limit_bits: 0x1d00ffff,
    bip30_unspendable: &[
        (91_722, "00000000000271a2dc26e7667f8419f2e15416dc6955e5a6c6cdf3f2574dd08e"),
        (91_812, "00000000000af0aed4792b1acee3d966af36cf5def14935db8de83d6f9306f2f"),
    ],
    signet: false,
};

/// Default signet (Core's built-in challenge). A custom signet would need its own challenge and
/// genesis; not supported.
pub const SIGNET: Net = Net {
    name: "signet",
    genesis_hex: "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6",
    bip34_height: 1,
    bip66_height: 1,
    bip65_height: 1,
    csv_height: 1,
    segwit_height: 1,
    pow_limit_bits: 0x1e0377ae,
    bip30_unspendable: &[],
    signet: true,
};

pub fn by_name(name: &str) -> Option<&'static Net> {
    match name {
        "mainnet" | "main" => Some(&MAINNET),
        "signet" => Some(&SIGNET),
        _ => None,
    }
}

/// Display-order hex to internal-order bytes, at compile time where needed.
pub const fn hash_le(s: &str) -> [u8; 32] {
    let b = s.as_bytes();
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        let hi = nyb(b[2 * i]);
        let lo = nyb(b[2 * i + 1]);
        out[31 - i] = hi * 16 + lo;
        i += 1;
    }
    out
}

/// Display-order hex read as plain big-endian bytes (for constants such as n and p).
pub const fn hex_be(s: &str) -> [u8; 32] {
    let b = s.as_bytes();
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = nyb(b[2 * i]) * 16 + nyb(b[2 * i + 1]);
        i += 1;
    }
    out
}

const fn nyb(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        _ => panic!("hash_le: not lowercase hex"),
    }
}

impl Net {
    /// Whether the coinbase of this block is BIP30-unspendable (never indexed, never accounted).
    /// Keyed on height AND hash, like Core: on signet the same heights are ordinary blocks.
    pub fn is_bip30_unspendable(&self, height: u32, hash_internal: &[u8; 32]) -> bool {
        self.bip30_unspendable.iter().any(|(h, x)| *h == height && hash_le(x) == *hash_internal)
    }
}
