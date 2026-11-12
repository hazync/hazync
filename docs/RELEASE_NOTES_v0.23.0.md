> **Withdrawn on 2026-11-11: use [v0.24.0](RELEASE_NOTES_v0.24.0.md).** v0.23.0 was built on RISC Zero 3.0.5, whose
> proof system has soundness bugs fixed in 3.1.0. Its image `863c9ce3` was never run by the public board. These are
> the notes as published, kept as the record.

v0.23.0 is the first release of **libzync**, the core that replaces the original Hazync design. Every
binary in it is new code, the program a proof names is new, and the board moves to it.

**The image changes.** A proof now names one of the published libzync images:

```
mainnet  863c9ce35104740d8b6fa2972f0dd1b29289e3e67e5679400f0e8ecdbef69af5
signet   73b62530ca89eea376e7753f47ab4f16aed3be3138d6052d84d30510aed0f38d
```

`reproduce/libzync.Dockerfile` rebuilds them bit for bit (`reproduce/LIBZYNC_IDS`), and CI rebuilds them
on every change. Proofs by the original image (`37987b85…`) are not libzync proofs, and this release's
verifier refuses them (exit `1`: the receipt does not match the libzync image); v0.22.1 still verifies them.

## What changed in what you download

The asset names stay the same; what is inside them is libzync.

| asset | now |
|---|---|
| `hazync-host-x86_64-linux-gnu`, `-cuda` | the libzync host: proves leaf bundles (`prove-range-bridge`), joins ranges (`fold-range`), checks them (`verify-any`) |
| `hazync-host-x86_64-windows-cuda.zip` | **new.** The same host for Windows with an NVIDIA card: `host.exe` and `cudart64_12.dll` |
| `hazync-verify-*`, `hazync-verify.wasm`, `.js` | the libzync verifier (`swiftsync-verify`), in the terminal and in the browser |
| `hazync-worker`, `hazync-coordinator.py`, `hazync-run-workers.sh` | the proof-party tools, now speaking leaf bundles |

**The verifier has three answers, not two.** Exit `0` means the proof is valid and reaches Bitcoin's
minimum chainwork. Exit `2` (`DEMO`) means it is a valid chain from genesis that has not got there
yet. Every early proof of the board gives `2`, and that is not a failure. Exit `1` means it was refused.
Scripts that treat any non-zero exit as a failure need to change.

## What libzync does differently

It still runs Bitcoin Core's own C++ (`interpreter.cpp`, `SignatureHash`, `libsecp256k1`) inside the
zkVM. Around it, it adds:
- one batch check of each chunk's signatures, where the original made one proof per signature;
- an append-only **coin log**, which makes a missing or double spend unprovable at every block;
- a **counting trie**, which answers questions by script (balance, history, "no such coin");
- **joins**, which fold blocks proved independently into one proof.

So blocks can be proved in any order on any number of cards, and the chain is assembled afterwards.

Measured, sources under `experimental/libzync-bench/`:
- **Mainnet genesis to block 200**, by the published image: 3,031 s on one RTX PRO 6000. The result is a 227 KB receipt that verifies in a browser.
- **Blocks 200,000 to 300,000 on real mainnet state**: 9.5x fewer proven cycles and 7.0x less card time than the original design (`results/CYCLES_REAL_STATE.md`).
- **Windows**, on an RTX PRO 6000 under the WDDM driver model (the mode #631's crash was tied to): block 170 in 319 s, block 400,000 in 823 s. Both receipts were verified on Windows, and again independently on Linux (`windows/RESULT_xesktop_rtxpro6000.txt`).

## The coin check

`swiftsync-verify-wasm/zapp/` is a page that verifies the chain proof on your own device. It then checks a
server's answers about any script against that proof. Testing it on real data found that a history
answer could carry altered bytes and still be accepted. The answer itself could not be changed this
way, but the bytes were malleable. #695 makes the decoding one-to-one, so every word of a fact is now
bound. This was fixed before any release shipped the code.

## The board

[DISCUSS: the progress wording. The facts as of writing: the libzync board's spine is genesis to block 4;
the state before every block is prepared up to block ~463,000 of ~975,000; the board proves upward from
genesis, while preparation continues ahead of it.]

The board switches to libzync with this release. Contributors carry over by identity (their handle, key, first-seen date and
last version), with a block count of zero, because a block proved by the original image is not a libzync block. Nothing
else carries over.

## Not yet

- **No independent audit.** The parts that are not Core's code (the coin log, the trie, the batch
  signature check, the aggregate) are where review matters most.
- **No proof of the whole chain yet.**
- **Following the tip with libzync** is designed but not running. The tip tools in `coordinator/`
  (`tip_board.py`, `tip_runner.py`, `tip_smoke.py`, `sync_bundles.py`) are still the original design's.

[DISCUSS: tone. The draft follows v0.22.x: plain, measured, every figure sourced.]
