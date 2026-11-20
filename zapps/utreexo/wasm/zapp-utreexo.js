// Glue for the Utreexo roots zyncAPP checker in WebAssembly. Hand-written and short on purpose (no wasm-bindgen,
// nothing here touches the network), as ../../../swiftsync-verify-wasm/swiftsync-verify.js.
//
//   import { loadUtreexo } from './zapp-utreexo.js';
//   const u = await loadUtreexo(fetch('./zapp_utreexo_wasm.wasm'));
//   const r = u.verify({ receipt, network: 'mainnet', expectAt: '<H>:<hash>' });   // expectAt from YOUR header chain; expectTip when T > H
//   r.verdict ('ok' | 'demo' | 'fail'), r.network, r.height, r.block_hash, r.leaves, r.roots, r.image_pinned
//   (network: the one the proof must be for, 'mainnet' (the default) or 'signet'; a proof of another network is 'fail')
//   a TIP receipt (UTREEXO_ZAPP.md §10) is recognised by its journal: r.mode === 'tip', r.steps, r.from, r.base_height;
//   baseImageHex names a base image other than the built-in one (tip receipts only)
//   (a root "empty" is rustreexo's Empty: load it as BitcoinNodeHash::Empty, never as the hash of 32 zero bytes)

function readString(memory, ptr) {
  const len = new DataView(memory.buffer).getUint32(ptr, true);
  return new TextDecoder().decode(new Uint8Array(memory.buffer, ptr + 4, len));
}

export async function loadUtreexo(source) {
  const resolved = await source;
  const { instance } =
    typeof Response !== 'undefined' && resolved instanceof Response
      ? await WebAssembly.instantiateStreaming(resolved, {})
      : await WebAssembly.instantiate(resolved instanceof Uint8Array ? resolved : new Uint8Array(resolved), {});
  const { alloc, verify_utreexo, image_id, tip_image_id, memory } = instance.exports;
  return {
    /** Verify the final run's receipt for `network` (default 'mainnet'). `now` defaults to this device's clock. */
    verify({ receipt, now, imageHex, expectAt, expectTip, network, baseImageHex }) {
      const enc = new TextEncoder();
      const parts = [receipt, enc.encode(imageHex || ''), enc.encode(expectAt || ''), enc.encode(expectTip || ''),
                     enc.encode(network || 'mainnet')];
      if (baseImageHex) parts.push(enc.encode(baseImageHex));
      const bytes = new Uint8Array(8 + parts.reduce((n, p) => n + 4 + p.length, 0));
      const view = new DataView(bytes.buffer);
      view.setBigUint64(0, BigInt(Math.floor(now ?? Date.now() / 1000)), true);
      let o = 8;
      for (const p of parts) { view.setUint32(o, p.length, true); bytes.set(p, o + 4); o += 4 + p.length; }
      const ptr = alloc(bytes.length);
      new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes); // re-read buffer after alloc (it may grow)
      return JSON.parse(readString(memory, verify_utreexo(ptr, bytes.length)));
    },
    /** The built-in utreexo image id. */
    imageId() { return readString(memory, image_id()); },
    /** The built-in utreexo tip image id. */
    tipImageId() { return readString(memory, tip_image_id()); },
  };
}
