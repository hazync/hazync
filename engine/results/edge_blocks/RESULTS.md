# blocks.sh's edge blocks on the engine (26.11.2026, build box, CPU)

edge.sh: block JSON, hints + engine-export (product host of the engine branch), every chunk and the trie EXECUTED on
the engine (keys-m26) against RISC Zero's journals, the aggregate on a metering build of the same guest (keys-b26,
bench-noverify: env::verify accepts without proofs) against RISC Zero's aggregate journal byte by byte.

| block | what | chunks | chunk + trie journals | aggregate vs RISC Zero |
|---|---|---|---|---|
| 91,842 | BIP30 repeat | 0 | 1/1 identical | 3,688 B, 32 B differ (image id) |
| 91,880 | BIP30 repeat | 1 | 2/2 identical | 3,816 B, 32 B differ |
| 185,956 | spends coins made in the same block | 1 | 2/2 identical | 3,708 B, 32 B differ |
| 227,931 | BIP34 activation | 1 | 2/2 identical | 4,352 B, 32 B differ |
| 363,725 | BIP66 activation | 0 | 1/1 identical | 4,364 B, 32 B differ |
| 364,292 | largest legacy transaction | 16 | 17/17 identical | 4,748 B, 32 B differ |
| 388,381 | BIP65 activation | 2 | 3/3 identical | 4,880 B, 32 B differ |
| 419,328 | retarget + CSV activation | 4 | 5/5 identical | 4,112 B, 32 B differ |
| 481,824 | retarget + segwit activation | 4 | 5/5 identical | 4,244 B, 32 B differ |
| 2,016 | first retarget | - | NOT RUN | - |

- The 32 differing aggregate bytes are the image id the aggregate carries (the engine's own, RISC Zero's METHOD_ID in
  the reference), as on every block before.
- 419,328 and 481,824 first failed in RISC Zero's own hints step (`rt=false time=false`): the block JSON made by
  mkblock.py lacks the retarget epoch's context. With the prepared files (prover/testdata/boundaries, as blocks.sh
  uses) both pass. 2,016's prepared file lives in blocks.sh's CI temp dir, not in the repo: not run.
