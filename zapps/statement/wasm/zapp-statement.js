// Glue for the balance statement zyncAPP's WASM module (zapp-statement.wasm; docs/libzync/STATEMENT_ZAPP.md).
// Hand-written and short on purpose, as ../../../swiftsync-verify-wasm/swiftsync-verify.js; nothing here touches the network.
//
//   import { loadStatementVerifier } from './zapp-statement.js';
//   const v = await loadStatementVerifier(fetch('./zapp-statement.wasm'));
//   const s = v.verifyStatement({ network: 'mainnet', receipt, statement, expect: '<height>:<hash>', purpose, nonce,
//                                 maxAge: 6, tipHeight });   // tipHeight: the network's tip, fetched by the page
//   s.verdict  // 'ok' | 'demo' | 'fail';  s.statement = {ok, what, total, scripts};  s.demo_because = [...]
// expect, purpose, nonce, maxAge and tipHeight are YOURS (the block you trust, the challenge you gave the holder, your
// recency policy, the tip you fetched), never the sender's. s.age_checked is false until maxAge and tipHeight are given.

function readString(memory, ptr) {
  const len = new DataView(memory.buffer).getUint32(ptr, true);
  return new TextDecoder().decode(new Uint8Array(memory.buffer, ptr + 4, len));
}

// Engine proofs only (27.11.2026): no engine image is published yet, so imageHex, vk (the core image's key file) and pin
// (its 32-byte SHA-256, a constant the page is published with) are required.
function frame({ network, now, receipt, imageHex, vk, pin, statement, expect, purpose, nonce, maxAge, tipHeight }) {
  const enc = new TextEncoder();
  const parts = [receipt, enc.encode(imageHex || ''), vk || new Uint8Array(0), pin || new Uint8Array(0),
    enc.encode(typeof statement === 'string' ? statement : JSON.stringify(statement)),
    enc.encode(expect || ''), enc.encode(purpose || ''), enc.encode(nonce || ''),
    enc.encode(maxAge == null ? '' : String(maxAge)), enc.encode(tipHeight == null ? '' : String(tipHeight))];
  const out = new Uint8Array(1 + 8 + parts.reduce((n, p) => n + 4 + p.length, 0));
  const view = new DataView(out.buffer);
  out[0] = network === 'signet' ? 1 : network === 'mainnet' ? 0 : 255;
  view.setBigUint64(1, BigInt(Math.floor(now ?? Date.now() / 1000)), true);
  let o = 9;
  for (const p of parts) { view.setUint32(o, p.length, true); out.set(p, o + 4); o += 4 + p.length; }
  return out;
}

export async function loadStatementVerifier(source) {
  const resolved = await source;
  const { instance } =
    typeof Response !== 'undefined' && resolved instanceof Response
      ? await WebAssembly.instantiateStreaming(resolved, {})
      : await WebAssembly.instantiate(resolved instanceof Uint8Array ? resolved : new Uint8Array(resolved), {});
  const { alloc, verify_statement, memory } = instance.exports;
  return {
    /** Check a balance statement against its core receipt. `now` defaults to this device's clock. */
    verifyStatement(args) {
      const bytes = frame(args);
      const ptr = alloc(bytes.length);
      new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes); // re-read buffer after alloc (it may grow)
      return JSON.parse(readString(memory, verify_statement(ptr, bytes.length)));
    },
  };
}
