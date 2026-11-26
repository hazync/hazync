# Block 962,000 negatives on the engine (roadmap 1.5; 26.11.2026 evening)

libzync CI's negative tests (scripts/libzync/ci/negatives.py: the same edits) through engine/scripts/negatives_engine.py.
Engine: keys-ch (final image: node format 2 + C heap, 2^29) for chunks, keys-chb (its metering build, env::verify
accepts without proofs) for the aggregate. RISC Zero: the product host at 90d8fef0 (format 2 + C heap, METHOD_ID
1f8df063...), dev mode off, executor only. Build box, CPU. **NEGATIVES_ENGINE_OK** (negatives_engine.out).

| case | RISC Zero | engine |
|---|---|---|
| base: hints, chunk, aggregate | valid | chunk journal identical; aggregate valid |
| base: the block's collect run (control for the refusals below) | finishes (COLLECT_EXIT) | finishes (guest exit(2) = COLLECT_EXIT) |
| maturity_ok / maturity_immature (coinbase coin 1 block short) | all_valid 1 / 0 | journals identical |
| height_lock_met / height_lock_unmet (BIP68, 1 block short) | 1 / 0 | identical |
| value_more / value_below_outputs | 1 / 0 | identical |
| bad_nonce_pow | refused: `aggregate: bv=false ph=true carry=true rt=true time=true smt=true` | **refused, the same line** |
| coinbase_overpays (+50 BTC) | refused, the same line | **refused, the same line** |

The aggregate refusals happen in the block's coin-log collect run (the aggregate without ECMH hints), before any
commit; `engine-export` now writes that run's stream (agg_collect.in) before RISC Zero runs it (commit 90d8fef0), so
the engine is given exactly the input RISC Zero refused. The published image logs only the summary line, so the
refusal is the block-validity bit; which rule (pow / subsidy) is the rig's to show against Core (results/rig_engine).

Note on the RISC Zero side of today's other mainnet comparisons (core_vectors, edge_blocks final-image re-runs): their
host was the format-2 build from BEFORE the C heap (METHOD_ID b10e1728..., results/node_format2); the engine side was
the final image in every case. This run's host has both.
