// Glue for the SwiftSync hints-proof verifier in WebAssembly. Hand-written and short on purpose, as
// ../verifier-wasm/hazync-verify.js: no wasm-bindgen, nothing here touches the network.
//
//   import { loadVerifier } from './swiftsync-verify.js';
//   const v = await loadVerifier(fetch('./swiftsync_verify_wasm.wasm'));
//   const r = v.verify({ network: 'signet', receipt, hints });   // Uint8Arrays; optional anchorJson, imageHex
//   r.verdict  // 'ok' | 'demo' | 'fail'

function readString(memory, ptr) {
  const view = new DataView(memory.buffer);
  const len = view.getUint32(ptr, true);
  return new TextDecoder().decode(new Uint8Array(memory.buffer, ptr + 4, len));
}

function frame({ network, now, receipt, hints, anchorJson, imageHex }) {
  const enc = new TextEncoder();
  const parts = [receipt, hints, enc.encode(anchorJson || ''), enc.encode(imageHex || '')];
  const out = new Uint8Array(1 + 8 + parts.reduce((n, p) => n + 4 + p.length, 0));
  const view = new DataView(out.buffer);
  out[0] = network === 'signet' ? 1 : network === 'mainnet' ? 0 : 255;
  view.setBigUint64(1, BigInt(Math.floor(now ?? Date.now() / 1000)), true);
  let o = 9;
  for (const p of parts) {
    view.setUint32(o, p.length, true);
    out.set(p, o + 4);
    o += 4 + p.length;
  }
  return out;
}

export async function loadVerifier(source) {
  const resolved = await source;
  const { instance } =
    typeof Response !== 'undefined' && resolved instanceof Response
      ? await WebAssembly.instantiateStreaming(resolved, {})
      : await WebAssembly.instantiate(resolved instanceof Uint8Array ? resolved : new Uint8Array(resolved), {});
  const { alloc, verify_frame, image_id, memory } = instance.exports;
  return {
    /** Verify a receipt (or .sspf bundle) against a hints file. `now` defaults to this device's clock. */
    verify(args) {
      const bytes = frame(args);
      const ptr = alloc(bytes.length);
      new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes); // re-read buffer after alloc (it may grow)
      return JSON.parse(readString(memory, verify_frame(ptr, bytes.length)));
    },
    /** The built-in image id for 'mainnet' or 'signet'. */
    imageId(network) {
      return readString(memory, image_id(network === 'signet' ? 1 : 0));
    },
  };
}
