// Glue for the libzync chain-proof verifier in WebAssembly. Hand-written and short on purpose, as
// ../verifier-wasm/hazync-verify.js: no wasm-bindgen, nothing here touches the network.
//
//   import { loadVerifier } from './swiftsync-verify.js';
//   const v = await loadVerifier(fetch('./swiftsync_verify_wasm.wasm'));
//   const r = v.verify({ network: 'signet', image: 'product', receipt });   // Uint8Array; optional anchorJson, imageHex
//   const f = v.verifyFacts({ ...same, facts: { tries: [trieFact] }, expect: '<height>:<hash>' });
//   (expect: the network's block at the proof's height from a source YOU trust, never from the fact server)
//   r.verdict  // 'ok' | 'demo' | 'fail'

function readString(memory, ptr) {
  const view = new DataView(memory.buffer);
  const len = view.getUint32(ptr, true);
  return new TextDecoder().decode(new Uint8Array(memory.buffer, ptr + 4, len));
}

// image: 'product' (coin log + counting trie + live-coin hash), 'coinlog', or the built-in profile.
function frame({ network, image, now, receipt, anchorJson, imageHex, facts, expect }) {
  const enc = new TextEncoder();
  const parts = [receipt, enc.encode(anchorJson || ''), enc.encode(imageHex || '')];
  if (facts !== undefined) parts.push(enc.encode(JSON.stringify(facts)), enc.encode(expect || ''));
  const out = new Uint8Array(1 + 8 + parts.reduce((n, p) => n + 4 + p.length, 0));
  const view = new DataView(out.buffer);
  out[0] = (network === 'signet' ? 1 : network === 'mainnet' ? 0 : 255)
    | (image === 'product' ? 0x20 : image === 'coinlog' ? 0x10 : 0);
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
  const { alloc, verify_frame, verify_facts, image_id, memory } = instance.exports;
  const call = (fn, args) => {
    const bytes = frame(args);
    const ptr = alloc(bytes.length);
    new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes); // re-read buffer after alloc (it may grow)
    return JSON.parse(readString(memory, fn(ptr, bytes.length)));
  };
  return {
    /** Verify a receipt (or .sspf bundle). `now` defaults to this device's clock. */
    verify(args) {
      // an expected tip is a check, never dropped: with one, verify through the facts path with no facts (round 9)
      return args.expect ? call(verify_facts, { ...args, facts: {} }) : call(verify_frame, { ...args, facts: undefined });
    },
    /** As verify, plus facts ({coins, journeys, tries}) checked against the verified state; `expect` ('<height>:<hash>')
     *  is the tip from a source you trust, passed beside the facts, never inside them. */
    verifyFacts(args) { return call(verify_facts, { ...args, facts: args.facts ?? {} }); },
    /** The built-in image id for 'mainnet' or 'signet'. */
    imageId(network) {
      return readString(memory, image_id(network === 'signet' ? 1 : 0));
    },
  };
}
