#!/usr/bin/env python3
"""The only edits to Hazync's guest main.rs for the OpenVM port (see README.md)."""
import sys
p = sys.argv[1]; s = open(p).read()
def rep(old, new, n=1):
    global s
    assert s.count(old) == n, (old[:60], s.count(old)); s = s.replace(old, new)
# 1) cross-version handover verifier (old RISC Zero recursion circuits): not ported; mainnet's coin-log lineage is empty.
i = s.index("mod xver;")
rep("mod xver;", """mod xver {   // OpenVM port: cross-version handover NOT PORTED (mainnet coin-log lineage is empty, so never called)
    pub const fn carried(_v: &[u8; 32]) -> bool { false }
    pub mod accel { pub fn suite() {} }
    pub fn verify<A, B, C, D, E>(_: A, _: B, _: C, _: D, _: E) -> Result<(), &'static str> { panic!("xver not ported to OpenVM") }
}""")
# 2) batch_ecdsa: the OpenVM implementation (same public items)
rep('#[cfg(feature = "batch-ecdsa")]\nmod batch_ecdsa;', '#[cfg(feature = "batch-ecdsa")]\n#[path = "batch_ecdsa_ovm.rs"]\nmod batch_ecdsa;')
# 3) reveal the journal hash at the end of main (the host prints the same "journal sha256" for RISC Zero)
i = s.index("\nfn main() {"); j = s.index("\n}\n", i)
s = s[:j] + "\n    risc0_zkvm::guest::env::finalize_journal();" + s[j:]
open(p, "w").write(s); print("main.rs edited")
