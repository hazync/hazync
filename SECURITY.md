# Hazync — security review & status

## Reporting a vulnerability

Report it privately through GitHub's private vulnerability reporting:
<https://github.com/hazync/hazync/security/advisories/new> (the **Security** tab, then **Report a
vulnerability**). A report there is visible only to you and the maintainers. Conduct reports under
[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) may use the same form. Please do not open a public issue for a
way to make an invalid block prove valid.

**No commissioned professional audit has happened.** The findings so far come from our own cold review rounds
(fresh reviewers given the code, not the author's conclusions). We fix what is found, retest, and record it in the
open: the status below, the round-by-round record in [`docs/libzync/LIBZYNC.md`](docs/libzync/LIBZYNC.md) §5.
**Independent review is explicitly invited**; if you find a way to make an invalid block prove valid, that is the
finding that matters most.

The property that makes this worth reviewing: the prover runs **real Bitcoin Core v28 consensus code**
(`interpreter.cpp`, `SignatureHash`, `CheckTransaction`, merkle, PoW and retarget) and libsecp256k1 0.5.1 inside a
RISC Zero zkVM, so there is no reimplementation of script validation to diverge from Core. What is NOT Core's code,
and is therefore where a soundness bug is most likely: the batch check of each chunk's signature equations, the
block-level rules the aggregate applies from the chunks' facts, the coin log, the counting trie and the joins.
[`docs/libzync/LIBZYNC.md`](docs/libzync/LIBZYNC.md) §6 says exactly who decides what, and
[`docs/libzync/CONSENSUS_RULES.md`](docs/libzync/CONSENSUS_RULES.md) goes through Core's rules one by one.

## Verifying releases

Each release carries a PGP-signed `SHA256SUMS.txt` (signed in CI by the release-signing workflow). The
maintainer key is:

```
defenwycke <defenwycke@icloud.com>
777FE81F 8CC077FD 3D08055E 852C2B31 90F5B928
```

From a release's assets, download the binary **keeping its asset filename** (`hazync-host-x86_64-linux-gnu`) plus `SHA256SUMS.txt` and `SHA256SUMS.txt.asc`, then:

```bash
curl -LO https://github.com/hazync/hazync/releases/latest/download/hazync-host-x86_64-linux-gnu
curl -LO https://github.com/hazync/hazync/releases/latest/download/SHA256SUMS.txt
curl -LO https://github.com/hazync/hazync/releases/latest/download/SHA256SUMS.txt.asc
curl -s https://github.com/defenwycke.gpg | gpg --import   # the key, from the account that publishes
                                                           # the releases (see "second source" below)
gpg --verify SHA256SUMS.txt.asc SHA256SUMS.txt   # must report a GOOD signature from the key above
sha256sum -c --ignore-missing SHA256SUMS.txt      # → hazync-host-x86_64-linux-gnu: OK
```

**The second source is `https://github.com/defenwycke.gpg`.** A fingerprint published in the same
repository you are auditing is not independent evidence — anyone who could alter the releases could
alter this file. GitHub serves the maintainer's key from the *account* the releases come from, which is
a different system with a different compromise path, so agreement between the two is worth something.
Fetching it is also the whole import step above; `gpg --recv-keys 852C2B3190F5B928` from a keyserver
works too and is a third, independent source.

Whichever you use, the fingerprint must be:

```
777FE81F 8CC077FD 3D08055E 852C2B31 90F5B928
```

**The key expires 2028-04-25.** After that, signatures on releases made before then still verify but
`gpg` reports the key as expired — that is the key's lifetime ending, not evidence of tampering.

**`gpg` will also warn that the key "is not certified with a trusted signature".** That is expected and
is not a problem with the signature: it means *you* have not personally signed this key, which nobody
has on a fresh keyring. What matters is the `Good signature` line and that the fingerprint matches the
one above. If you want the warning to go away, sign the key locally (`gpg --lsign-key
852C2B3190F5B928`) — but check the fingerprint against a second source first, because that is the step
the warning is asking you to think about.

`--ignore-missing` matters: the manifest covers **every** published asset — eight in v0.21.4: both
hosts, both native verifiers, the WASM verifier, the worker CLI, the fleet launcher and the coordinator
script — so a plain `sha256sum -c` reports `FAILED open or read` and exits non-zero for each one you
didn't download. That is
the command complaining about a missing file, not about a bad checksum — but it looks alarming, so use
the flag and check only what you fetched.

(If you saved the binary as `host` per the quick-start, `sha256sum -c` won't find it by name — either keep the asset filename as above, or run `sha256sum host` and compare the digest to the matching line in `SHA256SUMS.txt` by eye.)

A good signature plus a matching checksum means the binary is the maintainer's published build. Note
that the *stronger* guarantee is reproducibility, not the signature: anyone can rebuild the guest and
confirm its image id matches the published one in `reproduce/LIBZYNC_IDS` (rebuilt by
`reproduce/libzync.Dockerfile`), and any proof verifies against that id regardless of who built the host. The
signature adds provenance on top of that.

## Status

**No commissioned professional audit has happened, and no external audit of libzync.** That is the first thing
wanted before anyone relies on a proof for money.

- **Review so far.** Fifteen cold review rounds by fresh reviewers given the code, not the author's conclusions,
  each finding fixed and retested: [`docs/libzync/LIBZYNC.md`](docs/libzync/LIBZYNC.md) §5. Among what they found
  and fixed: an `is_base` value that skipped verifying the previous proof, the BIP68 version compared as signed, a
  join digest that admitted only one tree shape, and a verifier that accepted a dev-mode receipt.
- **Testing.** Core's own `script_tests`, `tx_valid` and `tx_invalid` with every flag variant agree with the guest
  (22,666 / 22,666); a rig of blocks built to hit each rule agrees with Bitcoin Core 28.0 (96/96); mainnet and
  signet from genesis match independent reference implementations of the coin log and block forest; every
  tampered join, handover and trie fact tried is refused. Sources: LIBZYNC.md §5.
- **Known limits.** [`docs/libzync/LIBZYNC.md`](docs/libzync/LIBZYNC.md) §7, kept current.

## Where an hour of review is worth most

1. The batch signature check (`prover/methods/guest/src/batch_ecdsa.rs` and its callers): a hint that makes an
   invalid signature pass, or the edge cases of x = r + n.
2. The aggregate's block rules from chunk facts: fees, weight, sigops, BIP30/34/68, and that every input is covered
   exactly once.
3. The coin log and the counting trie ([`docs/libzync/COINLOG.md`](docs/libzync/COINLOG.md)): a missing or spent
   coin that opens as unspent, or a trie fact that verifies with the wrong answer.
4. Joins and handovers ([`docs/libzync/HANDOVER.md`](docs/libzync/HANDOVER.md)): two proofs joined that do not meet,
   or a handover that changes what a chain states.
5. The verifiers (`swiftsync-verify`, its WASM and FFI builds): a receipt accepted that does not say what the verdict
   claims.

The original design's review record (the CORE guest `37987b85`, audit rounds 1-11 and its status table) is kept in
[`docs/history/SECURITY_CORE.md`](docs/history/SECURITY_CORE.md) and
[`docs/history/SECURITY_AUDIT_LOG.md`](docs/history/SECURITY_AUDIT_LOG.md).
