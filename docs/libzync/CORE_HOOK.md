# Checking a hints proof inside Bitcoin Core: a sketch

**Status: a design sketch against Core PR #34004 ("Implementation of SwiftSync", closed), not compiled or run.** It
shows where a proof check fits in that PR's code and what it costs. The library it calls is real and tested
(`swiftsync-verify-ffi/`: C header, static library, `test.c` 9/9 on the real signet receipt). How SwiftSync lands in
Core is the SwiftSync authors' call; this is offered, not proposed as a PR.

## What it adds

`-utxohintsproof=<path>`, allowed only with `-utxohints`. Before the hints are used, Core:
1. reads the proof (a `.rcpt` receipt or `.sspf` bundle) and the hints file's bytes;
2. calls `swiftsync_verify(...)` with its own network and clock, and refuses to start unless the verdict is
   `SWIFTSYNC_VERIFY_OK` (a genesis-anchored proof; DEMO is never accepted);
3. remembers the proven `(height, tip_hash, chainwork)`, and when the node connects block `height`, requires that
   block to be `tip_hash`. That is step 7 of the proposal's §5, done by the node itself. If it differs (the node
   follows a different chain), the hints are dropped and the node continues as a normal IBD, or stops; which one is
   a policy choice for the authors.

The library is one call and has no global state; it holds risc0's verifier and no prover (the CLI built from the
same code is 1.8 MB).

## Sketch (against PR #34004's `src/init.cpp` and `src/validation.cpp`)

```diff
+#include <swiftsync_verify.h>   // from swiftsync-verify-ffi/include, link libswiftsync_verify.a

     argsman.AddArg("-utxohints=<path>", "Accelerate initial block download with the assistance of a UTXO hint file.", ArgsManager::ALLOW_ANY, OptionsCategory::OPTIONS),
+    argsman.AddArg("-utxohintsproof=<path>", "Only use -utxohints if this proof shows the file is correct for the chain at its stop height.", ArgsManager::ALLOW_ANY, OptionsCategory::OPTIONS);

     std::optional<swiftsync::HintsfileReader> utxo_hints;
     if (args.IsArgSet("-utxohints")) {
         fs::path path = fs::absolute(args.GetPathArg("-utxohints"));
         ...
+        if (args.IsArgSet("-utxohintsproof")) {
+            std::vector<uint8_t> proof = ReadFileBytes(fs::absolute(args.GetPathArg("-utxohintsproof")));
+            std::vector<uint8_t> hints = ReadFileBytes(path);
+            swiftsync_result res;
+            char msg[256];
+            const uint8_t net = chainparams.GetChainType() == ChainType::SIGNET ? SWIFTSYNC_NET_SIGNET : SWIFTSYNC_NET_MAINNET;
+            const int v = swiftsync_verify(proof.data(), proof.size(), hints.data(), hints.size(), net,
+                                           GetTime(), /*image_hex=*/nullptr, /*anchor_json=*/nullptr, &res, msg, sizeof(msg));
+            if (v != SWIFTSYNC_VERIFY_OK) {
+                return InitError(strprintf(_("UTXO hints proof does not verify: %s"), msg));
+            }
+            if (res.profile & 2) {
+                LogWarning("UTXO hints proof is from an assumevalid image: scripts and signatures were not checked");
+            }
+            // Step 7 is checked when block res.height is connected (validation.cpp below).
+            node.utxo_hints_expected = {res.height, uint256{std::span{res.tip_hash}}};
+        }
         ...
     }
```

```diff
 // validation.cpp, in Chainstate::ConnectBlock (or where hints are consumed), once per block:
+    if (m_utxo_hints_expected && pindex->nHeight == m_utxo_hints_expected->height) {
+        if (pindex->GetBlockHash() != m_utxo_hints_expected->tip) {
+            // The proof is about a different chain: the hints cannot be trusted for this one.
+            return FatalError(m_chainman.GetNotifications(), state, _("UTXO hints proof is for a different chain"));
+        }
+    }
```

Notes for whoever turns this into code:
- The chainwork check of step 7 is implied here: a block at that height with that hash on the node's own chain has
  the proven chainwork by construction. The library returns it anyway (`res.chainwork`).
- The image id is the trust root. `image_hex = nullptr` uses the id compiled into the library; a node could instead
  take it from a reproducibly built release (a `-utxohintsimage=<hex>` option, or a constant in chainparams).
- The check runs once at startup. Measured natively on the signet case (226 KB receipt, 3 KB hints file): 0.02 s
  and 3.5 MB of memory. Decoding the 122 MB mainnet hints file adds to that and has not been timed in Rust.
