// Glue for the SwiftSync hints zyncAPP's WASM module (docs/libzync/HINTS_ZAPP.md), a zAPP module of its own
// (docs/libzync/ZYNCAPPS.md §9), not the core verifier's hazync-verify.js. Hand-written and short on purpose, as the
// core's: no wasm-bindgen, nothing here touches the network.
//
//   import { loadHints } from './zapp-hints.js';
//   const z = await loadHints(fetch('./zapp-hints.wasm'));
//   const s = z.verifyHints({ receipt, file, undo, imageHex, coreImageHex, vk, pin, expect: '<H>:<hash from YOUR header chain>',
//                             network: 'mainnet', format: 'bip' });
// `network` (default 'mainnet') is YOUR node's, compared with the proof's; `format` ('bip' or 'crate') the hints format
// your node reads, compared with the file's: without it the verdict is at most demo.
// Engine proofs only (27.11.2026): no engine image is published yet, so imageHex, coreImageHex, vk (the hints image's key
// file) and pin (its 32-byte SHA-256, a constant the page is published with) are all required.
//   s.verdict, s.height, s.hash, s.format, s.unspent

function readString(memory, ptr) {
  const view = new DataView(memory.buffer);
  const len = view.getUint32(ptr, true);
  return new TextDecoder().decode(new Uint8Array(memory.buffer, ptr + 4, len));
}

export async function loadHints(source) {
  const resolved = await source;
  const { instance } =
    typeof Response !== 'undefined' && resolved instanceof Response
      ? await WebAssembly.instantiateStreaming(resolved, {})
      : await WebAssembly.instantiate(resolved instanceof Uint8Array ? resolved : new Uint8Array(resolved), {});
  const { alloc, verify_hints, hints_image_id, memory } = instance.exports;
  return {
    /** `file` is the hints file the receipt proves for block H; `undo` (optional) the undo-hash file; `expect`
     *  ('<H>:<hash>') block H from YOUR node's header chain; `expectTip` the core tip when it is above H. The file is
     *  checked here, on the device. `now` defaults to this device's clock. */
    verifyHints({ receipt, file, undo, now, imageHex, coreImageHex, expect, expectTip, vk, pin, network, format }) {
      const enc = new TextEncoder();
      const parts = [receipt, file, undo || new Uint8Array(0), enc.encode(imageHex || ''), enc.encode(coreImageHex || ''),
        enc.encode(expect || ''), enc.encode(expectTip || ''), vk || new Uint8Array(0), pin || new Uint8Array(0),
        enc.encode(network || ''), enc.encode(format === undefined || format === null ? '' : String(format))];
      const bytes = new Uint8Array(8 + parts.reduce((n, p) => n + 4 + p.length, 0));
      const view = new DataView(bytes.buffer);
      view.setBigUint64(0, BigInt(Math.floor(now ?? Date.now() / 1000)), true);
      let o = 8;
      for (const p of parts) { view.setUint32(o, p.length, true); bytes.set(p, o + 4); o += 4 + p.length; }
      const ptr = alloc(bytes.length);
      new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes); // re-read buffer after alloc (it may grow)
      return JSON.parse(readString(memory, verify_hints(ptr, bytes.length)));
    },
    /** The built-in hints image id. */
    hintsImageId() { return readString(memory, hints_image_id()); },
  };
}
