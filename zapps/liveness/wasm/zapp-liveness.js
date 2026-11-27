// Glue for the coin-liveness zyncAPP's WASM module (docs/libzync/LIVENESS_ZAPP.md), a zAPP module of its own
// (docs/libzync/ZYNCAPPS.md §9), not the core verifier's hazync-verify.js. Hand-written and short on purpose, as the
// core's: no wasm-bindgen, nothing here touches the network.
//
//   import { loadLiveness } from './zapp-liveness.js';
//   const z = await loadLiveness(fetch('./zapp-liveness.wasm'));
//   const r = z.verifyLiveness({ network: 'mainnet', image: 'product', receipt,
//     liveness: [{ proof, ask: '<txid>:<vout>@<h|tip>=unspent' }], expect: '<height>:<hash>' });
//   (ask: YOUR question, claiming its answer; expect: the network's block at the proof's height from a source YOU
//   trust, never from the fact server)
//   r.verdict  // 'ok' | 'demo' | 'fail'; r.facts: one {ok, what} per liveness proof

function readString(memory, ptr) {
  const view = new DataView(memory.buffer);
  const len = view.getUint32(ptr, true);
  return new TextDecoder().decode(new Uint8Array(memory.buffer, ptr + 4, len));
}

export async function loadLiveness(source) {
  const resolved = await source;
  const { instance } =
    typeof Response !== 'undefined' && resolved instanceof Response
      ? await WebAssembly.instantiateStreaming(resolved, {})
      : await WebAssembly.instantiate(resolved instanceof Uint8Array ? resolved : new Uint8Array(resolved), {});
  const { alloc, verify_liveness, memory } = instance.exports;
  return {
    /** Verify a core engine proof (or .sspf bundle) and each liveness proof against it. Engine proofs only (27.11.2026):
     *  no engine image is published yet, so imageHex, vk (the core image's key file) and pin (its 32-byte SHA-256, a
     *  constant the page is published with) are required. `image`: 'product', 'coinlog' or
     *  the built-in profile; `now` defaults to this device's clock. */
    verifyLiveness({ network, image, now, receipt, anchorJson, imageHex, vk, pin, liveness, expect }) {
      const enc = new TextEncoder();
      const parts = [receipt, enc.encode(anchorJson || ''), enc.encode(imageHex || ''),
        enc.encode(JSON.stringify({ liveness: liveness || [] })), enc.encode(expect || ''), vk || new Uint8Array(0), pin || new Uint8Array(0)];
      const out = new Uint8Array(1 + 8 + parts.reduce((n, p) => n + 4 + p.length, 0));
      const view = new DataView(out.buffer);
      out[0] = (network === 'signet' ? 1 : network === 'mainnet' ? 0 : 255)
        | (image === 'product' ? 0x20 : image === 'coinlog' ? 0x10 : 0);
      view.setBigUint64(1, BigInt(Math.floor(now ?? Date.now() / 1000)), true);
      let o = 9;
      for (const p of parts) { view.setUint32(o, p.length, true); out.set(p, o + 4); o += 4 + p.length; }
      const ptr = alloc(out.length);
      new Uint8Array(memory.buffer, ptr, out.length).set(out); // re-read buffer after alloc (it may grow)
      return JSON.parse(readString(memory, verify_liveness(ptr, out.length)));
    },
  };
}
