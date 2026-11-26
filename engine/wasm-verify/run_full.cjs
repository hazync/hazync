// FEASIBILITY Q11: the full check a page makes (STARK + image id + journal) as wasm32 in Node, on a real engine proof.
// node run_full.cjs <inputs dir: vk.bin proof.zst proof_bad.zst block.journal image.json> <pkg dir>
const fs = require("fs"), crypto = require("crypto");
const [dir, pkg] = process.argv.slice(2);
const t0 = performance.now();
const w = require(pkg + "/wasmv.js");
const t1 = performance.now();
const rd = (f) => fs.readFileSync(dir + "/" + f);
const vk = rd("vk.bin"), journal = rd("block.journal");
const img = JSON.parse(rd("image.json"));
const id = crypto.createHash("sha256").update(Buffer.concat([Buffer.from(img.exe, "hex"), Buffer.from(img.vm, "hex")])).digest();
console.log(`image id (image.json) ${id.toString("hex")}`);
console.log(`image id (from vk)    ${w.image_id_hex(vk)}`);
const flip = (b, i) => { const c = Buffer.from(b); c[i] ^= 1; return c; };
const cases = [
  ["valid", rd("proof.zst"), journal, id, 1],
  ["journal bit flipped", rd("proof.zst"), flip(journal, 100), id, 4],
  ["other image id", rd("proof.zst"), journal, flip(id, 0), 5],
  ["proof byte flipped", rd("proof_bad.zst"), journal, id, 0],
  ["valid again", rd("proof.zst"), journal, id, 1],
];
let ok = true;
for (const [name, p, j, i, want] of cases) {
  const t = performance.now(); const r = w.verify_full(vk, p, j, i);
  const ms = (performance.now() - t).toFixed(1);
  ok = ok && r === want;
  console.log(`${name}: ${r} (want ${want}) in ${ms} ms`);
}
console.log(`module load ${(t1 - t0).toFixed(1)} ms; ${ok ? "ALL AS EXPECTED" : "MISMATCH"}`);
process.exit(ok ? 0 : 1);
