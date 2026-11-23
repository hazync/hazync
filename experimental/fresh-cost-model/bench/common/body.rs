// Shared benchmark body, include!()d verbatim by every zkVM guest.
// Input: [mode u8][block blob from extract]. Panics unless EVERY check passes.
//   mode 1 = ECDSA verify (prehash, SEC1 pubkey, compact low-S sig)
//   mode 2 = BIP340 Schnorr verify
//   mode 3 = txids (double SHA-256 of stripped txs) + merkle root == header root
//   mode 0 = parse only (baseline: input read + parse cost)
// Returns a 32-byte digest committed as the public output.
fn rd_u32(b: &[u8], o: &mut usize) -> usize {
    let v = u32::from_le_bytes(b[*o..*o + 4].try_into().unwrap()) as usize;
    *o += 4;
    v
}

fn dsha(data: &[u8]) -> [u8; 32] {
    let h1 = sha2::Sha256::digest(data);
    let h2 = sha2::Sha256::digest(&h1[..]);
    let mut o = [0u8; 32];
    o.copy_from_slice(&h2[..]);
    o
}

fn bench_run(input: &[u8]) -> [u8; 32] {
    let mode = input[0];
    let b = &input[1..];
    let mut o = 0usize;
    let n_ec = rd_u32(b, &mut o);
    let ec_at = o;
    o += n_ec * 129;
    let n_sc = rd_u32(b, &mut o);
    let sc_at = o;
    o += n_sc * 128;
    let n_tx = rd_u32(b, &mut o);
    let tx_at = o;
    let mut checked: u32 = 0;
    if mode == 1 {
        for i in 0..n_ec {
            let r = &b[ec_at + i * 129..ec_at + (i + 1) * 129];
            let vk = k256::ecdsa::VerifyingKey::from_sec1_bytes(&r[32..65]).expect("pubkey");
            let sig = k256::ecdsa::Signature::from_slice(&r[65..129]).expect("sig");
            use k256::ecdsa::signature::hazmat::PrehashVerifier;
            vk.verify_prehash(&r[..32], &sig).expect("ecdsa verify");
            checked += 1;
        }
    }
    #[cfg(feature = "schnorr")]
    if mode == 2 {
        for i in 0..n_sc {
            let r = &b[sc_at + i * 128..sc_at + (i + 1) * 128];
            let vk = k256::schnorr::VerifyingKey::from_bytes(&r[32..64]).expect("xonly");
            let sig = k256::schnorr::Signature::try_from(&r[64..128]).expect("sig");
            vk.verify_raw(&r[..32], &sig).expect("schnorr verify");
            checked += 1;
        }
    }
    let _ = sc_at;
    let mut out = [0u8; 32];
    if mode == 3 {
        let mut level: Vec<[u8; 32]> = Vec::with_capacity(n_tx);
        let mut p = tx_at;
        for _ in 0..n_tx {
            let l = rd_u32(b, &mut p);
            level.push(dsha(&b[p..p + l]));
            p += l;
        }
        let root_hdr = &b[p..p + 32];
        while level.len() > 1 {
            if level.len() % 2 == 1 {
                let last = *level.last().unwrap();
                level.push(last);
            }
            let mut next = Vec::with_capacity(level.len() / 2);
            let mut buf = [0u8; 64];
            for pair in level.chunks(2) {
                buf[..32].copy_from_slice(&pair[0]);
                buf[32..].copy_from_slice(&pair[1]);
                next.push(dsha(&buf));
            }
            level = next;
        }
        assert_eq!(&level[0][..], root_hdr, "merkle root mismatch");
        out = level[0];
        checked = n_tx as u32;
    }
    out[..4].copy_from_slice(&checked.to_le_bytes());
    out[4] = mode;
    out
}
