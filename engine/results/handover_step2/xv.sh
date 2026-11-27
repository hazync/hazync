#!/bin/bash
# HANDOVER step 2 builds. (1) the default engine image from HEAD: must be keys-ch's exe (the cfg switch changes nothing);
# (2) the test variant pinning keys-ch's chain as an OpenVM-xver predecessor at 200, carrying keys-ch's key.
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
cd /root/engine-kit && git fetch -q origin engine/openvm && git merge -q --ff-only FETCH_HEAD && git log --oneline -1 | cut -c1-50
E=/root/engine-kit/engine/host/target/release/hzengine; cd /root/engine || exit 1
mk() { k=$1; g=$2; shift 2; rm -rf "${k:?}" && mkdir "$k" && cp keys-rt29/{sdk.pk,agg.vk,openvm.toml} "$k"/
  env "$@" HZ_BASE=/root/engine-base6 nice $E build --dir "$k" --guest "$g" > "${k}_build.log" 2>&1 \
    && echo "$k: $(grep -a '^build:' "${k}_build.log" | cut -c1-120)" \
    || { echo "$k BUILD_FAIL"; sed 's/\x1b\[[0-9;]*m//g' "${k}_build.log" | grep -a "error\[\|error:" -A8 | head -30; }; }
rm -rf /root/engine-guestxd && /root/engine-kit/engine/guest/assemble.sh /root/engine-guestxd > asmxd.log 2>&1 || { echo ASM_FAIL; tail -5 asmxd.log; exit 1; }
mk keys-xd /root/engine-guestxd/prover/methods/guest
echo "keys-ch was: exe 0ddcb255de793558186eee62e8e05d34e1438b04ddf57717ca5c363f0e0bd76a"
# (2) the predecessor's key, named by its pin; the lineage pinning it
mkdir -p xvkeys && cp vin200p/vk.bin xvkeys/b5b94f9c0eb4e4e90ee3653da7b1f521b056a994b6d486b90e1e19622a16da7a.vk
cat > xv_lineage.rs <<'L'
// handover rehearsal (engine/HANDOVER.md step 2): keys-ch's chain (engine, OpenVM 2.1.0), handed over at 200 through
// the ovm-xver path (verifier = SHA-256 of keys-ch's key file)
pub const LINEAGE: &[crate::handover_pin::Handover] = &[crate::handover_pin::Handover {
    image: [3087990471, 273427805, 423765843, 714736923, 1323484124, 3246500386, 4007801574, 3375336817],
    first_height: 200,
    verifier: [0xb5, 0xb9, 0x4f, 0x9c, 0x0e, 0xb4, 0xe4, 0xe9, 0x0e, 0xe3, 0x65, 0x3d, 0xa7, 0xb1, 0xf5, 0x21,
               0xb0, 0x56, 0xa9, 0x94, 0xb6, 0xd4, 0x86, 0xb9, 0x0e, 0x19, 0x62, 0x2a, 0x16, 0xda, 0x7a, 0x00],
}];
L
python3 - <<'P'
# the pin's bytes, written from the hex (no hand copy): replaces the verifier array above
import re
pin=bytes.fromhex("b5b94f9c0eb4e4e90ee3653da7b1f521b056a994b6d486b90e1e19622a16da7a")
s=open("xv_lineage.rs").read()
arr=", ".join(f"0x{b:02x}" for b in pin)
s=re.sub(r"verifier: \[[^\]]*\]", "verifier: ["+arr+"]", s, flags=re.S)
open("xv_lineage.rs","w").write(s)
P
grep -c "0x7a\]" xv_lineage.rs
rm -rf /root/engine-guestxv && HZ_OVM_XVER_KEYS=/root/engine/xvkeys HZ_TEST_LINEAGE=/root/engine/xv_lineage.rs /root/engine-kit/engine/guest/assemble.sh /root/engine-guestxv > asmxv.log 2>&1 || { echo ASM_FAIL; tail -8 asmxv.log; exit 1; }
tail -3 asmxv.log
mk keys-xv /root/engine-guestxv/prover/methods/guest HZ_GUEST_FEATURES=test-lineage,ovm-xver
echo XV_DONE
