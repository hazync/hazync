// FEASIBILITY Q11: OpenVM v2.1.0's verifier (one-line abi_stable patch) as wasm32 in Node, on a real engine proof.
const fs = require("fs");
const t0 = performance.now();
const w = require(process.argv[3] + "/wasmv.js");
const t1 = performance.now();
const vk = fs.readFileSync(process.argv[2] + "/vk.bin");
for (const f of ["proof.zst", "proof_bad.zst", "proof.zst"]) {
  const p = fs.readFileSync(process.argv[2] + "/" + f);
  const t = performance.now(); const r = w.verify_proof(vk, p);
  console.log(`${f}: ${r} (1 valid, 0 invalid, 2 bad vk) in ${(performance.now() - t).toFixed(1)} ms`);
}
console.log(`module load ${(t1 - t0).toFixed(1)} ms`);
