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

This page covers **Hazync v2** (the libzync engine): published images mainnet `1ff5a129`, signet `28e39600` and the
header-chain zyncAPP `2db34e12` ([`reproduce/LIBZYNC_IDS`](reproduce/LIBZYNC_IDS)), release v0.26.0, RISC Zero 3.1.0.
The mainnet image continues `8888d59c` (v0.24.0-v0.25.0) at block 2000 by a handover ([`docs/HANDOVER.md`](docs/HANDOVER.md)):
blocks below 2000 are `8888d59c`'s proofs, and the verifier keeps that image.
What a proof rests on is set out in [`docs/SOUNDNESS.md`](docs/SOUNDNESS.md), and how to review it in
[`docs/EXTERNAL_REVIEW.md`](docs/EXTERNAL_REVIEW.md).

The property that makes this worth reviewing: the prover runs **real Bitcoin Core v28 consensus code**
(`interpreter.cpp`, `SignatureHash`, `CheckTransaction`, merkle, PoW and retarget) and libsecp256k1 0.5.1 inside a
RISC Zero zkVM, so there is no reimplementation of script validation to diverge from Core. What is NOT Core's code,
and is therefore where a soundness bug is most likely: the batch check of each chunk's signature equations, the
block-level rules the aggregate applies from the chunks' facts, the coin log, the counting trie and the joins.
[`docs/libzync/LIBZYNC.md`](docs/libzync/LIBZYNC.md) §6 says exactly who decides what, and
[`docs/libzync/CONSENSUS_RULES.md`](docs/libzync/CONSENSUS_RULES.md) goes through Core's rules one by one.

## Supported versions

| release | engine | status |
|---|---|---|
| v0.26.0 (latest) | Hazync v2, image `1ff5a129`, RISC Zero 3.1.0 | supported: the public board runs it from the handover at block 2000 |
| v0.24.0, v0.25.0 | Hazync v2, image `8888d59c`, RISC Zero 3.1.0 | supported for verifying: its proofs are the board's blocks 1..1999 |
| v0.23.0 | Hazync v2, image `863c9ce3`, RISC Zero 3.0.5 | **withdrawn**: do not deploy; 3.1.0 verifiers refuse its proofs |
| v0.22.1 and earlier | Hazync v1, image `37987b85` (since v0.21.0) | not maintained; v0.22.1 still verifies v1 proofs |

## Verifying releases

Each release carries a PGP-signed `SHA256SUMS.txt` (signed in CI by the release-signing workflow). From v0.25.0
its first line names what it is for, `# hazync <tag> commit <sha> image <id>`, and the signature covers that line
too: check that the tag is the one you downloaded and the image is the one the release notes publish. Before
signing, the workflow checks that every host and verifier carries that image, and a re-sign may not change a
hash that was already signed. The
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

`--ignore-missing` matters: the manifest covers **every** published asset (in v0.24.0: the Linux CPU and CUDA
hosts, the Windows CUDA host zip, the x86-64 and aarch64 verifiers, the WASM verifier and its JavaScript glue, the
worker CLI, the fleet launcher and the coordinator script; `scripts/release-assets.txt` is the list), so a plain
`sha256sum -c` reports `FAILED open or read` and exits non-zero for each one you didn't download. That is the
command complaining about a missing file, not about a bad checksum — but it looks alarming, so use the flag and
check only what you fetched. (The flag cannot tell you that the file you meant to check is absent; to catch that
too, pipe only the lines you want to `sha256sum -c -`, as [`CONTRIBUTING.md`](CONTRIBUTING.md) does.)

(If you saved the binary as `host` per the quick-start, `sha256sum -c` won't find it by name — either keep the asset filename as above, or run `sha256sum host` and compare the digest to the matching line in `SHA256SUMS.txt` by eye.)

A good signature plus a matching checksum means the binary is the maintainer's published build. Note
that the *stronger* guarantee is reproducibility, not the signature: anyone can rebuild the guest and
confirm its image id matches the published one in `reproduce/LIBZYNC_IDS` (rebuilt by
`reproduce/libzync.Dockerfile`), and any proof verifies against that id regardless of who built the host. The
signature adds provenance on top of that.

## Status

**No commissioned professional audit has happened, and no external audit of libzync.** That is the first thing
wanted before anyone relies on a proof for money.

- **The proof system beneath it.** v0.24.0 moves to **RISC Zero 3.1.0**, which fixes soundness bugs in the zkVM's
  proof system (an rv32im trace could hold two executions; `sys_read` words were not range-checked; the recursion
  circuit's `BIT_AND_ELEM` decompositions were not unique; Groth16 claim handling; the Poseidon S-box). None was in
  Hazync's own code, but a proof is only as sound as what checks it. **v0.23.0, built on 3.0.5, is withdrawn**, and
  3.1.0 verifiers refuse every 3.0.x receipt; a test vector checks this with v0.23.0's real proof of blocks 1..200
  ([`docs/RELEASE_NOTES_v0.24.0.md`](docs/RELEASE_NOTES_v0.24.0.md)).
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

Hazync v1's review record (the original CORE guest `37987b85`, audit rounds 1-11 and its status table) is kept in
[`docs/history/SECURITY_CORE.md`](docs/history/SECURITY_CORE.md) and
[`docs/history/SECURITY_AUDIT_LOG.md`](docs/history/SECURITY_AUDIT_LOG.md).
