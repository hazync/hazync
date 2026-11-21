//! The utreexo tip step's block rules (prover/zyncapps/methods/utreexo-tip/src/step.rs, the file the tip guest and the
//! host #[path]-include) under test natively, in the zAPP's own module so CI's zapps job runs it (zapps/utreexo/test.sh).
//! Nothing here is part of any image.
extern crate alloc;
/// libzync's base derivation, under the name step.rs uses for it.
pub mod utreexo_roots {
    pub use libzync::utreexo_roots::*;
}
/// libzync's compact-size reader, under the name step.rs uses for it.
pub mod facts {
    pub use libzync::facts::*;
}
#[path = "../../../../prover/zyncapps/methods/utreexo-tip/src/step.rs"]
pub mod step;

#[cfg(test)]
mod tests {
    use super::step;
    use super::utreexo_roots;
    type H = [u8; 32];

    fn hx(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn put_tx(b: &mut Vec<u8>, inputs: &[(H, u32)], outputs: &[(u64, &[u8])]) -> H {
        let s0 = b.len();
        b.extend_from_slice(&1u32.to_le_bytes());
        b.push(inputs.len() as u8);
        for (t, v) in inputs {
            b.extend_from_slice(t);
            b.extend_from_slice(&v.to_le_bytes());
            b.push(1);
            b.push(0x51);
            b.extend_from_slice(&u32::MAX.to_le_bytes());
        }
        b.push(outputs.len() as u8);
        for (v, spk) in outputs {
            b.extend_from_slice(&v.to_le_bytes());
            b.push(spk.len() as u8);
            b.extend_from_slice(spk);
        }
        b.extend_from_slice(&0u32.to_le_bytes());
        step::dsha(&b[s0..])
    }

    /// A block whose second transaction spends `outpoint`, with a valid Merkle root, after `prev`.
    fn block_spending(prev: &H, outpoint: (H, u32)) -> Vec<u8> {
        let mut txs = Vec::new();
        let cb = put_tx(&mut txs, &[([0; 32], u32::MAX)], &[(50_0000_0000, &[0x51])]);
        let t1 = put_tx(&mut txs, &[outpoint], &[(1, &[0x51])]);
        let mut cat = [0u8; 64];
        cat[..32].copy_from_slice(&cb);
        cat[32..].copy_from_slice(&t1);
        let root = step::dsha(&cat);
        let mut b = Vec::new();
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(prev);
        b.extend_from_slice(&root);
        b.extend_from_slice(&[0u8; 12]);
        b.push(2);
        b.extend_from_slice(&txs);
        b
    }

    /// The BIP30 refusal (step.rs `block_changes`): a block spending output 0 of the coinbase of 91,722 / 91,812
    /// (the same txids as 91,880's / 91,842's coinbases), given the OVERWRITTEN coin's data, rebuilds Floresta's
    /// UNSPENDABLE_BIP30_UTXO_* leaf and is refused, as Floresta's `update_acc` refuses it. Control: the same spend with
    /// the recreated coin's data (another creating block and height) is not refused by this check.
    #[test]
    fn a_deletion_of_a_bip30_leaf_is_refused() {
        const CB: [&str; 2] = [
            "01000000010000000000000000000000000000000000000000000000000000000000000000ffffffff060456720e1b00ffffffff0100f2052a01000000434104124b212f5416598a92ccec88819105179dcb2550d571842601492718273fe0f2179a9695096bff94cd99dcccdea7cd9bd943bfca8fea649cac963411979a33e9ac00000000",
            "01000000010000000000000000000000000000000000000000000000000000000000000000ffffffff060456720e1b00ffffffff0100f2052a010000004341046896ecfc449cb8560594eb7f413f199deb9b4e5d947a142e7dc7d2de0b811b8e204833ea2a2fd9d4c7b153a8ca7661d0a0b7fc981df1f42f55d64b26b3da1e9cac00000000",
        ];
        let tag = utreexo_roots::tag_state();
        for (i, cb) in CB.iter().enumerate() {
            let raw = hx(cb);
            let txid = step::dsha(&raw);
            let (height, bh) = utreexo_roots::MAINNET_BIP30[i];
            let spk = &raw[raw.len() - 4 - 67..raw.len() - 4]; // the one output's 67-byte P2PK script
            let block = block_spending(&[7; 32], (txid, 0));
            let mut c = step::Counts::default();
            let mut spent = Vec::new();
            step::put_spent(&mut spent, &bh, (height << 1) | 1, 50_0000_0000, spk);
            let r = step::block_changes(&tag, &block, &[7; 32], 100_000, &spent, &mut c);
            assert!(matches!(&r, Err(e) if e.contains("BIP30")), "{height}: the BIP30 leaf was not refused");
            let mut other = Vec::new();
            step::put_spent(&mut other, &[9; 32], (91_880 << 1) | 1, 50_0000_0000, spk);
            assert!(step::block_changes(&tag, &block, &[7; 32], 100_000, &other, &mut c).is_ok(), "{height}: control refused");
        }
    }
}
