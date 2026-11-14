**Hazync for Windows, as a normal installer.** This release adds `hazync-setup-x86_64-windows.exe`. It installs
the Hazync app for the current user, with no administrator rights needed. Your proving name and key are kept
across upgrades and uninstalls. The app inside is the one that has been proving on the board:
- pick a segment size from a list (Automatic is recommended, and 22 is offered for 48 GB cards);
- a timestamped technical log on its own tab;
- the Home page says how many workers are proving and how many are folding.

The installer is unsigned, so Windows SmartScreen will warn the first time. The release's signed `SHA256SUMS.txt`
lists it. The release check installed it on a clean Windows runner and confirmed the installed prover is
byte-identical to the one in `hazync-host-x86_64-windows-cuda.zip`.

**The images do not change:**

```
mainnet  8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4
signet   d4210dae455340743cca097f2b422eee9846c9c02af0dae31d1268fadef4f387
headers  2db34e12cc993dd1edb003f00f85c93d02e0e61208589b57fc5c969d39b3e632
```

Every proof made under v0.24.0 stays valid, and the board does not restart. A worker on v0.24.0 keeps working, but
updating is recommended for the fixes below.

## What changed

- **The Windows installer** (#713, #715). It is built and signed as part of the release. A release now has 13
  assets; up to v0.24.0 it had 12.
- **Release integrity** (#707):
  - a CI gate on the main test suite;
  - the signature is bound to the tag;
  - clean Windows release builds;
  - pinned fetches.
- **The verifier** reads a v2 proof as v2 without `--product` (#708).
- **Prep after a crash:** the trie is checked before the coin log is rewound (#709).
- **The board:**
  - it refuses a replayed public proof before verifying it (#712);
  - the Coin check follower publishes only facts about the proof's own chain (#711);
  - its state cleanup no longer fails most runs (#706);
  - it follows the board, with a reproducible build that cannot race (#705).
- **The board server after the v2 switch** (#716):
  - the sponsor bot requires the v2 coordinator;
  - the backup watch checks the v2 ledger;
  - an alert refused by ntfy's daily quota fails at once instead of hanging its check.
- **A security fix:** quinn-udp 0.5.16, for RUSTSEC-2026-0538 (#714).
- **Docs** (#704, #710): Hazync v2 throughout. A fact check's OK says the facts hold, not that they answer
  your question.

The verifier's three answers are unchanged: `0` OK, `2` DEMO (valid, not yet at minimum work), `1` refused.
