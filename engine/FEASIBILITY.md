# OpenVM engine: feasibility questions (25.11.2026)

The questions to answer before deciding whether the engine replaces RISC Zero. Each answer is marked MEASURED,
ESTIMATED or UNKNOWN and cites a file or log for every number. Answers go under each question; the list is the user's.

  A. Trust and soundness
  1. Exactly which OpenVM version, commit and proving system does the engine use, and which audits cover that exact version? Include the auditor, the date,
     the scope, and any open findings. Which parts are unaudited (GPU prover, recursion, SHA and bigint extensions, the STARK-to-SNARK wrapper)?
  2. What security level do the proof parameters give, in bits? Does that rely on any conjectures, and how does it compare with risc0's 3.1.0 settings?
  3. Is OpenVM still actively maintained? Give its release pace, how it discloses security issues, and who maintains it.
  4. The compatibility layer that runs risc0's guest API on OpenVM is our own code on the soundness path. List every place where it could accept something
     risc0 would refuse, and what tests cover each one.
  5. How is the image id bound? What exactly does "exe commitment + vm" commit to? Can two different guests or configurations share an id? Can a proof from
     one configuration verify under another?
  6. List every difference between the engine guest and the product guest (CHANGES.md), and explain why each one cannot change a verdict or committed state.

  B. Completeness: can every block Core accepts still be proved?
  7. Do the #794 attack blocks prove on the engine: the memory block, the single tapscript input, and the costly SIGHASH_NONE block at a small scale? What
  allocator does the guest use, and does it ever free memory?
  8. Does it prove an old block with many inputs (364,292), plus a few of the edge blocks from blocks.sh?
  9. Do all 22,666 Core test vectors and the 97 rig cases pass with verdicts identical to risc0's?
  10. What are the limits on memory, cycles per segment and input size, and where do they come from?

  C. Where it has to run
  11. Browser and phone verifier: can a proof be checked in WASM in a browser? How large is the proof and verifier, how long does a check take on a phone,
  and what does the verifier trust?
  12. Windows: does the GPU prover build and prove on Windows with a native CUDA toolchain? What's the minimum driver version?
  13. Cards: which GPUs are supported (oldest architecture, minimum memory)? Do consumer cards like a 3060 or 4090 work, and how fast?
  14. Is there a CPU-only prover, and how fast is it?
  15. Reproducible ids: do two clean builds on different machines give the same image id? Can that be pinned the way reproduce/LIBZYNC_IDS is pinned today?

  D. Cost and speed
  16. What is the whole-chain cost, from genesis to the tip, in GPU-hours and dollars? Use measured per-block costs across eras, and say which blocks the
  estimate rests on.
  17. How do the tip-block times compare (detect to verified) with risc0's current 191 s on 4× PRO 6000?
  18. What do the three levers still on the table buy, measured: fewer and larger chunks, CPU/GPU overlap, and native SHA? Does any of them change a
  commitment or format?
  19. How does it scale across several GPUs for one block?

  E. Migration
  20. What has to be rebuilt? Cover the zAPPs (headers, hints, utreexo, deposit, assume, liveness, statement), the verifiers, the board and coordinator, the
  worker and the Windows app, the release tooling and the site pins.
  21. Which commitments and formats can stay byte-identical, so existing tools and zAPP logic carry over?
  22. What's the plan for re-proving from genesis, and does the existing risc0 chain proof have any role in it?
  23. Does the engine support composition, so a zAPP can verify a core proof inside its own proof as it does today?
  24. What does it cost to maintain over time? Do we follow upstream OpenVM releases, and what happens to image ids when we do?

  F. The deciding question
  25. Which of the above, if it came back badly, would rule the switch out, and which are just work?
