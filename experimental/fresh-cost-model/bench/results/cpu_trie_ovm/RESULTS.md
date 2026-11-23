# Hazync's counting-trie update (guest mode 17) on OpenVM 2.1 — CPU executor, block 976,379 (23.11.2026)

Guest `ovm-trie/`: mode 17's body copied unchanged from prover/methods/guest/src/main.rs, `libzync` (main 261f0a59) as an
unmodified no_std dependency. libzync's zkVM path calls risc0's `sys_sha_compress`; supplied on OpenVM's SHA-256
extension (no_std build: OpenVM's std PAL defines that symbol as an `unreachable!()` stub).
Input from `trieprep/` (native, libzync std): the block's update list built with libzync's own `coin_key` / `spender`
— **5,134 marks, 7,072 inserts (= Hazync's run 2 exactly)** — and the old trie MODELLED as Hazync's measurement path does
(`trie_model::synth`, sub 2, counts, seed 0x5eed+1), population 2.6e9 -> witness 1,308,817 words (Hazync's: 1,304,744).

| | result |
|---|---|
| journal SHA-256, native `ttree::apply_stream_perm` | 74b08f52ee189be2a1cdcb5c8dded7689874559f22fae687069ac040617ba196 |
| journal SHA-256, OpenVM guest | **identical** |
| instructions / OpenVM cost | 268.2M / 13.59B |
| SHA-256 compressions | 324,278 (~830 instructions each, incl. ~250 of shim byte swaps + copies) |
| Hazync product, same block, same card (run 2) | trie update PROVED in 211.8 s |

Not proved on a GPU. At the Core guest's measured 5.9 ns/cost (run 3) ≈ 80 s; at the k256 guest's 14.9 ns/cost ≈ 200 s.
Lever: libzync keeps W in risc0's byte-swapped layout; an OpenVM-native layout (cfg in coinlog.rs IVs / w_of / h_of)
removes the shim's per-compression swaps (~30% of instructions).
