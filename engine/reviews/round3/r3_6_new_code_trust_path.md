# R3-6: the new code on the trust path, release to verdict

Reviewer: independent, read-only. Tree: r3tree at a9b78510 (the last ~60 commits, 1b197e03..a9b78510). Scope: (1) the
zeroconf zAPP (guest, reader lib, CLI, wasm module and glue); (2) how keys, pins and image ids reach a verifier;
(3) the release and signing path; (4) the board / tip / sponsor engine paths.

## Verdict

**No way found to get a false zeroconf verdict, or a false board acceptance, to verify.** Every verifier surface
(swiftsync-verify CLI, C ABI, core wasm, the eight zAPP CLIs and wasm frames, the board's verify-any, tip_engine's
local check) now takes the key's pin from the caller and refuses a key whose SHA-256 differs. The new genesis-anchor
pin in the zeroconf guest does what it says.

The real findings are on the **operator side**: release downloads that run before or without a signature check, a
signing-time identity check with a gap, and engine smoke tests that cannot pass on real hardware. That last one is
fail-closed today, but the obvious "fix" would be an unpinned fallback.

| # | Severity | Finding | Confidence |
|---|---|---|---|
| 1 | Medium | Engine pod boot runs downloaded binaries even when the signed-manifest check fails, and accepts the boot on stdout words that those binaries can print | High |
| 2 | Medium-low | fold_rent's engine CPU fetch checks hzengine against an UNSIGNED manifest fetched on the pod, with a contributor identity on the pod | High |
| 3 | Medium-low | release-sign checks the key tarballs that have a record, not every key tarball it signs | High |
| 4 | Low (functional, fail-closed) | Every engine smoke calls `verify-any` without HAZYNC_ENGINE_VK, so it always REJECTs on a real host; the tests' fake host hides it | High |
| 5 | Low | zeroconf wasm: `now` not bounded by the clock (the CLI bounds it); input frame never freed; the DEMO-anchor path is dead | High |
| 6 | Info | The Hazync image id leaves out `expected_def_hook_commit` and `internal_recursive_vk_commit`. For a deferral-using image (zeroconf), the pin alone binds which child verifier it trusts | Medium (rests on R2-1) |
| 7 | Info | Reproducing an image checks its id, never its vk.bin, so the pin's honesty rests on the signer | High |
| 8 | Info | windows-installer.yml checks asset hashes against SHA256SUMS.txt but never checks its .asc; on the first signing the assets are trusted as uploaded | High |
| 9 | Info | Worker, app and run-workers image checks read keys/image.json, which nothing binds to guest.vmexe / agg.vk. Only a denial of service: the board verifies | High |

---

## 1. Medium: engine boot executes unverified downloads; pass condition is forgeable stdout

`coordinator/sponsor_bot.py` BOOT_SH_ENGINE (≈1236-1255):

```
for f in @FILES@; do ... curl ... done
chmod +x hazync-worker hazync-run-workers.sh hazync-host-x86_64-linux-gnu hazync-engine-x86_64-linux-gnu-cuda
sha256sum -c --quiet want.txt && echo SHA_OK || echo SHA_BAD          # no exit on failure
rm -rf keys-core && ... tar -xzf hazync-keys-core.tar.gz ...
... . ./engine.env
@SMOKE@                                                               # runs ./hazync-host (-> hzengine) regardless
echo "MID=$(./hazync-host method-id ...)"
```

`want.txt` itself is sound: it is cut from the controller's gpg-verified SHA256SUMS.txt (sponsor_bot.py:1433-1446).
But the script keeps going after SHA_BAD and runs the binary that failed the check. The controller then accepts the
boot if the words `SHA_OK`, `GPU_OK`, `KEYS_OK`, `CUDART_OK` and `MID=<id>` all appear anywhere in stdout
(sponsor_bot.py:1497-1500). An executed binary can write those words to the ssh channel, for example through
`/proc/$PPID/fd/1`, even with its own stdout sent to gpu_smoke.log. Once boot passes, `start()` copies each sponsored
contributor's `key.hex` to the pod (sponsor_bot.py:1515-1524). tip_engine boots the same way (its docstring and
`sb.SshRunner`).

There is also a retry problem: the curl loop skips any file that is non-empty (`[ -s $f ] && break`), so a bad file is
never fetched again.

Scenario: a release asset is swapped after signing, which is the case the signed manifest exists for (a token with
write access, or a tampered CDN object). The bot boots a pod. SHA_BAD is printed, but the swapped host runs anyway and
prints the pass words. The bot copies contributor identity keys to it. The attacker can now submit under those names
and burn rented GPU. Proofs still have to verify on the board, so the chain's soundness is not affected. The old
BOOT_SH has the same shape; the engine version inherits it.

Fix: `sha256sum -c --quiet want.txt || { echo SHA_BAD; exit 1; }` before any `chmod +x` or execution; the same for
KEYS and CUDART. Delete a failed file before retrying. Make the controller require SHA_OK as its own line, before any
binary output (or check the hashes over ssh in a second command after boot.sh).

## 2. Medium-low: fold_rent's engine CPU hzengine is checked against an unsigned manifest

`coordinator/fold_rent.py:87-95` CPU_FETCH_ENGINE:

```
curl ... SHA256SUMS.txt ...; curl ... hazync-engine-x86_64-linux-gnu ...
grep ' hazync-engine-x86_64-linux-gnu$' SHA256SUMS.txt | sha256sum -c - || { echo CPU_SHA_BAD; exit 1; }
```

The pod fetches the manifest from the same place as the binary and never checks the `.asc`. This catches transport
corruption, not a swapped asset with a matching swapped manifest. The binary then runs as the CPU phase's
`HAZYNC_ENGINE` (fold_rent.py:144-147), with `HAZYNC_HOME=/root/.hazync-ids/<tag>` present (fold_phase). Commit
f5207731 describes this as "the CPU hzengine sha-verified"; it is verified, but not against anything signed. The same
applies to the RISC Zero CPU_FETCH (:108-116).

Fix: add `hazync-engine-x86_64-linux-gnu` to the controller's verified want.txt and copy that file to the pod, as the
boot files are.

## 3. Medium-low: release-sign's identity check covers records, not tarballs

`.github/workflows/release-sign.yml:346-385`: the loop is `for name, (img, pin) in sorted(recs.items())` over
`reproduce/engine-release.tsv`. scripts/release-assets.txt lists seven `hazync-keys-*.tar.gz`, but the tsv is
"written at the Stage 4 freeze" and its set of images is the freeze's decision (RELEASE_TOOLING.md; release-assets.txt
says "the handover verifier left out until a handover is published"). The name-set check at :323-329 makes sure each
listed tarball is present, but **a listed tarball with no tsv record is signed without any check of its image or
key.** If readers or the GPU gate ever take the pin of such an image from the tarball (sha256 of its vk.bin), a wrong
key is signed into the release. Smaller points in the same block:
- `recs[p[0]]` is keyed by name alone, so a second `core` line, for example a signet record named `core`, silently
  replaces the first. `idh = recs["core"][0]` can then differ from the `ID` that awk took for mainnet (:344).
- Member names are normalised with `m.name.lstrip("./")`, which strips any run of leading `.` and `/` characters. A
  tarball holding both `vk.bin` and `.vk.bin` (or `/vk.bin`) has the checker read whichever comes last, while
  `tar -x` writes both under different names. A reader that pins from the record catches the mismatch (a DoS only),
  but the checker is not reading the file a user will use.
- Of the tarball's files, only vk.bin and image.json are checked. sdk.pk, agg.vk, openvm.toml and guest.vmexe are
  signed as-is: they matter for proving, not for verification soundness.

Fix: require a one-to-one match between the `hazync-keys-*.tar.gz` in release-assets.txt and the tsv records. Key the
records on (name, network) and refuse duplicates. Refuse any member that is not exactly one of KEYFILES, or that is
not a regular file, and refuse duplicate members.

## 4. Low (functional, fail-closed): engine smokes cannot pass on a real host

`prover/host/src/lz_coord.rs:93-98`: without `HAZYNC_ENGINE_VK`, `verify-any` sends an HZE1 file to the RISC Zero
branch, which returns `Err("an engine (OpenVM) proof file, but this coordinator verifies RISC Zero receipts ...")`.
None of the engine smoke paths sets the board key:
- sponsor_bot SMOKE_LIBZYNC under BOOT_SH_ENGINE: engine.env exports only HAZYNC_ENGINE / HAZYNC_ENGINE_KEYS
  (:1252). Result: GPU_BAD on every real engine pod, and the boot fails.
- run-workers.sh:222-227: the libzync smoke on an engine worker always fails.
- tip_engine.block_script (:127-129): on-pod `ok=0` every time, so `verify_fn` (the pinned local check) is never
  reached (`if verify_fn is not None and done["ok"]`).

test_sponsor_engine.py:74 and test_tip_engine.py fake the host. The sponsor fake answers `verify-any` with RANGE-OK
whatever the environment, so the suites pass and the control "expects failure" for another reason. Nothing unsound
gets in. The risk is the fix: the natural patch is to let `verify-any` fall back to `HAZYNC_ENGINE_KEYS`
(agg.vk + baseline, unpinned), which would reopen the R2-5 #1 class on the board. Fix: ship the pin with the boot,
either as a want.txt line for vk.bin or with `HAZYNC_ENGINE_VK=keys-core/vk.bin HAZYNC_ENGINE_VK_PIN=<the record's
pin>` in engine.env, taken from the controller's verified manifest or the tag's tsv. Then make the fake hosts refuse
without the pin, as test_tip_engine's fake already does (:49-52).

## 5. Low: zeroconf wasm details

- `zapps/zeroconf/wasm/src/lib.rs:283` and `zapp-zeroconf.js:416`: `now` is whatever the page passes. The CLI refuses
  a `--now` later than its own clock (cli/src/main.rs:74). A page or integration that passes a future `now` loosens
  the time-too-new check, so the wasm is weaker than the CLI on this point. Suggest documenting it, or taking the
  page's clock only.
- `verify_zeroconf` builds a slice over the input and never frees it. The core module's `verify_frame` takes
  ownership with `Vec::from_raw_parts`. Every call leaks a whole proof frame; a long-lived page grows without bound
  (DoS only).
- Since 977439f0 the guest refuses any anchor except the network's genesis, so the reader's `anchor: Some` (DEMO)
  branch can no longer be reached with a proof by the production image. The reader and wasm tests still run on the
  old image 1ee5f2d7 (keys-z-zeroconf3, which proved from the 109 anchor). No reader-level test exercises the new
  image 132969d8 through the CLI or wasm. This is not unsound, because the image id is the caller's, but the shipped
  reader is tested only against an image it will not be given.

## 6. Info: what the image id does not bind (zeroconf composes)

`engine/verify-core/src/lib.rs:142-161` `image_of` hashes the app exe commit and the app, leaf and
internal-for-leaf vk commits. The baseline also carries `internal_recursive_vk_commit` and
`expected_def_hook_commit` (openvm v2.1.0 crates/verify/src/vk.rs:48-55), and neither is in the Hazync id. The
zeroconf guest's `env::verify` (engine/platform/risc0-zkvm/src/lib.rs:156-178, through `verify_stark_unchecked`)
compares only the child's (exe, vm) with core_id. Which child verifier the deferral uses is therefore fixed by the
deferral-circuit commits in the zeroconf exe's initial memory (R2-1 §2, vm_pvs/air.rs and deferral transpiler, as
cited there) and by the pinned zeroconf key's `expected_def_hook_commit`. I could not re-derive the deferral AIRs
myself; I rely on R2-1 for "a child that itself used deferrals is constrained the same way". So for a composing image
the pin is the **only** binding of the hook at the top level. Two keys with the same Hazync id can accept different
deferral hooks. That is consistent with "pin every key", but it is worth one line in VERIFIER.md: for zeroconf, the
image id names even less than for the core.

## 7. Info: reproducibility does not cover the pin

reproduce/engine-ids.sh reproduces exe/vm/image ids. package-release.sh:158-165 and release-sign.yml check
`sha256(vk.bin) == record pin`, but nothing re-derives vk.bin from the source. RELEASE_TOOLING.md lists "engine-ids.sh
does not export ... vk.bin" as PENDING. Until it does, and two machines produce byte-identical vk.bin, the pin is the
signer's word and not a rebuildable fact. That matters because the pin is the root of trust (the image id cannot
authenticate the key).

## 8. Info: installer manifest

windows-installer.yml:156-161 checks every packed asset (now including the engine zip and hazync-keys-core.tar.gz)
against SHA256SUMS.txt, which it downloads from the same release, and never runs `gpg --verify` on the `.asc`.
:162-165: on the first signing there is no manifest, so the assets are packed as uploaded and the sign job then signs
both the installer and the assets. This is the existing design. The engine layout adds `hzengine.exe` and `keys\`
under the same trust. The app's update path (appupdate.py) does check the signature against a pinned key packet.

## 9. Info: image checks via image.json

`engine_backend::image_id_words` (prover/host/src/engine_backend.rs:302-311) and therefore `host method-id` on a
worker, run-workers.sh's id comparison, and the Windows app's `_check_engine_host` (supervisor.py:604-630) all read
`keys/image.json`. Nothing ties that file to the guest.vmexe, sdk.pk or agg.vk actually used. Mismatched keys pass the
check and then produce proofs the board refuses, which wastes claims and is not a soundness problem. On the board,
`method-id` comes from the pinned key (engine_backend.rs:358-364), which is right.

---

## What I checked and found sound

- **Zeroconf guest** (prover/zyncapps/methods/zeroconf/src/main.rs):
  - Both journals are verified under core_id.
  - `check_core_statement` (libzync/src/facts.rs:378-402) checks kind, version >= 2, self_id == core_id, the
    network's profile bits, no assumevalid, the coin log, and now `anchor_id == the network's PINNED genesis`. The
    three constants equal swiftsync-verify's `genesis_anchor_hex` (lib.rs:35, :51) and corecheck's regtest value.
  - The chunk is all_valid, at tip+1 and MTP(tip) (the sorted median, as Core computes it), and speculative (the
    zero hash / time is accepted only under the core's `txlog` feature, agg_lite.rs:547-549).
  - T's record is its first run, covers every input, and the journal has no trailing words.
  - No input is a coin of the chunk's own height, and every input leaf is unspent in the tip's coin log.
  - The record layout matches agg_lite's writer and the aggregate's reader (agg_lite.rs:590-640; t_out is read
    because coinlog is required).
  - Core rules decided in the chunk come from the leaf-committed prevout data: CheckTransaction (including duplicate
    inputs), coinbase-shaped transactions refused (:264-266), IsFinalTx, BIP68 / maturity with coin_mtp, and scripts.
- **Mode confusion:** in a libzync core image only modes 4/5/8/9/10/16-20 run (main.rs:2422). Of these, only mode 10
  (`Vec<i32>`, prover-controlled length) could start with an arbitrary word. Matching KIND_CHUNK_LITE (0xC4A10010) or
  KIND_CHAIN (0xC4A10012) would need about 3.3 billion test cases in one execution, which is infeasible. This is
  defense by size, not by tag, and worth a comment in the guest.
- **Zeroconf reader** (zapps/zeroconf/src/lib.rs):
  - The proof is checked under the pinned key (verify_full_bytes: pin, then image_id_of(vk) == id, the STARK, and
    the pvs == sha256(journal)).
  - The journal is exactly 840 bytes.
  - It checks the kind, the txid, the optional wtxid, weight and sigops within one block's limits, core_id == the
    trusted image, profile equality, the lineage, the genesis anchor, time-too-new, the caller's tip (needed for OK),
    and minimum work plus the checkpoint (needed for OK).
  - A FAIL carries no statement, and the wasm maps a non-FAIL without a statement to fail.
  - Network codes: only 0/1 in wasm, so a regtest proof is refused.
  - The mainnet record's profile already includes NODE2, so `image_for(..., true, true)` gives 0xF1.
- **Keys and pins:**
  - `read_pinned_key` and `Image::with_pinned_engine_key` (swiftsync-verify/src/lib.rs:2026-2052) and
    `corecheck::with_key` (corecheck/src/lib.rs:100-114) all compare SHA-256 with the caller's pin before use.
  - Every zAPP CLI calls `read_pinned_key` before it reads a proof.
  - Every wasm frame requires a 32-byte pin.
  - The C ABI's `swiftsync_verify_engine_facts` requires vk, vk_len and a 32-byte pin. The FFI fails on any false
    check and only fills `out` on a non-FAIL.
  - `with_engine_key_unchecked` is used only in tests, fuzz targets and behind the pin checks above.
- **Board:**
  - `verify_any_engine` (lz_coord.rs:177-201): the pinned board key, the image from the key, kind, self_id, zero
    lineage, the board's profile.
  - `range_ok`: anchor == the board's state before lo; with `--check-end` (the board passes it, server.py:1281,
    :1402-1406) the end state and tip must be the board's own.
  - A board.env without VK fails closed for HZE1 files. A worker, pod or download cannot get a false proof accepted
    there.
  - tip_engine's `local_verify` refuses without a key and pin.
- **Release:**
  - release.sh refuses at preflight (:71-78).
  - package-release.sh and release_gpu_gate.py check key tarballs against the tsv records.
  - release-sign's exact-set check and its no-re-sign-of-changed-hashes rule are unchanged and sound.

## Not verified

- OpenVM's deferral circuit internals: whether `deferred_compute` output is constrained to a proof verified under
  the hook that the exe and key fix. I relied on R2-1. I read only the guest lib (verify-stark/guest/src/lib.rs) and
  the host check (crates/verify/src/lib.rs:259-325, deferral.rs).
- Whether the speculative chunk mode itself (the core guest under `txlog`) evaluates every height-dependent rule at
  `c_height` exactly as Core does for the next block. I read only agg_lite's binding, not the C++ path.
- The record layout against real bytes: I relied on the fixture runs cited in 58047ec3 and did not run anything.
- The Windows app at runtime (firstrun / supervisor) on Windows, and how `candidate_dirs` orders finding host.exe.
  Only the diffs were read.
- release_gpu_gate.py beyond its pin handling (:239-280), the pod gate script beyond its sha lines, and
  engine-windows.yml.
- Whether vk.bin keygen is deterministic, which finding 7 depends on.
