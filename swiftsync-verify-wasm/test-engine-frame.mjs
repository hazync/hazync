// node test-engine-frame.mjs <wasm built with --features engine> <swiftsync-verify/tests/data/engine200>: the engine
// proof of block 200 through verify_frame (network bit 0x40), and four refusals (engine/VERIFIER.md step 5)
import fs from 'fs';
const [wasm, dir] = process.argv.slice(2);
const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasm), {});
const x = instance.exports;
const rd = (f) => fs.readFileSync(`${dir}/${f}`);
const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const chunk = (b) => Buffer.concat([u32(b.length), b]);
function frame(code, proof, vk, pin, image) {
  const now = Buffer.alloc(8); now.writeBigUInt64LE(1790000000n);
  return Buffer.concat([Buffer.from([code]), now, chunk(proof), chunk(Buffer.alloc(0)), chunk(Buffer.from(image)), chunk(vk), chunk(pin)]);
}
function call(f) {
  const p = x.alloc(f.length); new Uint8Array(x.memory.buffer, p, f.length).set(f);
  const r = x.verify_frame(p, f.length); const n = new DataView(x.memory.buffer).getUint32(r, true);
  return JSON.parse(Buffer.from(new Uint8Array(x.memory.buffer, r + 4, n)).toString());
}
const proof = rd('proof.hze'), vk = rd('vk.bin'), image = rd('image_id').toString().trim();
const pin = Buffer.from(rd('vk.sha256').toString().trim(), 'hex');
const flip = (b, i) => { const c = Buffer.from(b); c[i] ^= 1; return c; };
let ok = true;
const t = performance.now(); const r = call(frame(0x40, proof, vk, pin, image)); const ms = (performance.now() - t).toFixed(0);
console.log(`valid: verdict ${r.verdict} in ${ms} ms; failed checks ${r.checks.filter(c => !c.ok).length}; height ${r.height ?? r.state?.height}`);
ok = ok && r.verdict === 'demo' && r.checks.every(c => c.ok);
for (const [name, f, want] of [
  ['journal bit flipped', frame(0x40, flip(proof, proof.length - 40), vk, pin, image), 'public values'],
  ['key not the pinned one', frame(0x40, proof, flip(vk, vk.length - 1), pin, image), 'pinned key'],
  ['other pin', frame(0x40, proof, vk, flip(pin, 0), image), 'pinned key'],
  ['engine proof without bit 0x40', Buffer.concat([Buffer.from([0]), Buffer.alloc(8), chunk(proof), chunk(Buffer.alloc(0)), chunk(Buffer.from(image))]), 'verifying key and pin'],
]) {
  const q = call(f); const why = q.checks.filter(c => !c.ok).map(c => c.what).join(' | ');
  const good = q.verdict === 'fail' && why.includes(want); ok = ok && good;
  console.log(`${good ? 'ok  ' : 'BAD '} ${name}: ${q.verdict}: ${why.slice(0, 140)}`);
}
console.log(ok ? 'ENGINE_FRAME_OK' : 'ENGINE_FRAME_MISMATCH'); process.exit(ok ? 0 : 1);
