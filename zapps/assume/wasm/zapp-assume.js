// Glue for the assumevalid and assumeutxo zyncAPPs' WASM module (docs/libzync/ASSUME_ZAPPS.md), a zAPP module of its
// own (docs/libzync/ZYNCAPPS.md §9), not the core verifier's hazync-verify.js (until 15.11.2026 verifyAssume was there).
// Hand-written and short on purpose, as the core's: no wasm-bindgen, nothing here touches the network.
//
//   import { loadAssume } from './zapp-assume.js';
//   const z = await loadAssume(fetch('./zapp-assume.wasm'));
//   const a = z.verifyAssume({ network: 'mainnet', receipt, imageHex, vk, pin, assumevalid: 'tip', headers, snapshot, tracked, expect });
// Engine proofs only (27.11.2026): no engine image is published yet, so imageHex, vk (the core image's key file) and pin
// (its 32-byte SHA-256, a constant the page is published with) are required.
//   a.verdict, a.assume.assumeutxo.hash_serialized_3

function readString(memory, ptr) {
  const view = new DataView(memory.buffer);
  const len = view.getUint32(ptr, true);
  return new TextDecoder().decode(new Uint8Array(memory.buffer, ptr + 4, len));
}

export async function loadAssume(source) {
  const resolved = await source;
  const { instance } =
    typeof Response !== 'undefined' && resolved instanceof Response
      ? await WebAssembly.instantiateStreaming(resolved, {})
      : await WebAssembly.instantiate(resolved instanceof Uint8Array ? resolved : new Uint8Array(resolved), {});
  const { alloc, verify_assume, memory } = instance.exports;
  return {
    /** assumevalid = 'tip' or a block hash (display order); headers = raw 80-byte headers ending at the proven tip (from
     *  genesis for a snapshot); snapshot = a Core dumptxoutset file (Uint8Array); tracked = the BIP30-tracked list text;
     *  expect ('<height>:<hash>') the tip from a source you trust (OK needs it). `image`: 'product', 'coinlog' or the
     *  built-in profile; `now` defaults to this device's clock. */
    verifyAssume({ network, image, now, receipt, imageHex, vk, pin, expect, assumevalid, headers, snapshot, tracked }) {
      const enc = new TextEncoder();
      const parts = [receipt, enc.encode(imageHex || ''), vk || new Uint8Array(0), pin || new Uint8Array(0), enc.encode(expect || ''), enc.encode(assumevalid || ''),
        headers || new Uint8Array(0), snapshot || new Uint8Array(0), enc.encode(tracked || '')];
      const out = new Uint8Array(1 + 8 + parts.reduce((n, p) => n + 4 + p.length, 0));
      const view = new DataView(out.buffer);
      out[0] = (network === 'signet' ? 1 : network === 'mainnet' ? 0 : 255)
        | (image === 'product' ? 0x20 : image === 'coinlog' ? 0x10 : 0);
      view.setBigUint64(1, BigInt(Math.floor(now ?? Date.now() / 1000)), true);
      let o = 9;
      for (const p of parts) { view.setUint32(o, p.length, true); out.set(p, o + 4); o += 4 + p.length; }
      const ptr = alloc(out.length);
      new Uint8Array(memory.buffer, ptr, out.length).set(out); // re-read buffer after alloc (it may grow)
      return JSON.parse(readString(memory, verify_assume(ptr, out.length)));
    },
  };
}
