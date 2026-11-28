// Glue for the zero-confirmation zyncAPP's WASM module (zapps/zeroconf/README.md), a zAPP module of its own
// (docs/libzync/ZYNCAPPS.md §9), not the core verifier's hazync-verify.js. Hand-written and short on purpose, as the
// core's: no wasm-bindgen, nothing here touches the network.
//
//   import { loadZeroconf } from './zapp-zeroconf.js';
//   const z = await loadZeroconf(fetch('./zapp-zeroconf.wasm'));
//   const s = z.verifyZeroconf({ proof, txid, wtxid, expectTip: '<tip height>:<hash from YOUR header chain>',
//                                imageHex, coreImageHex, vk, pin });
// Engine proofs only: no engine image is published yet, so imageHex (the zeroconf image), coreImageHex (the core image
// it rests on), vk (the zeroconf image's key file) and pin (its 32-byte SHA-256, a constant the page is published with)
// are all required. Give your CURRENT tip: OK is about the tip you pass. Consensus-valid for the next block, not
// final and not relay policy.
//   s.verdict ('ok' | 'demo' | 'fail'), s.checks, s.txid, s.wtxid, s.next_block, s.tip, s.fee, s.weight, s.sigops

function readString(memory, ptr) {
  const view = new DataView(memory.buffer);
  const len = view.getUint32(ptr, true);
  return new TextDecoder().decode(new Uint8Array(memory.buffer, ptr + 4, len));
}

const NETWORKS = { mainnet: 0, signet: 1 };

export async function loadZeroconf(source) {
  const resolved = await source;
  const { instance } =
    typeof Response !== 'undefined' && resolved instanceof Response
      ? await WebAssembly.instantiateStreaming(resolved, {})
      : await WebAssembly.instantiate(resolved instanceof Uint8Array ? resolved : new Uint8Array(resolved), {});
  const { alloc, verify_zeroconf, zeroconf_image_id, free_result, memory } = instance.exports;
  // read a result, then free it in the module (review R3-6 #5); verify_zeroconf frees the input frame itself
  const take = (p) => { const s = readString(memory, p); free_result(p); return s; };
  return {
    /** `proof` the zeroconf proof file (HZE1); `txid` the transaction you ask about, as explorers print it; `wtxid`
     *  (optional) the wtxid of the bytes you hold; `expectTip` ('<height>:<hash>') your current tip from YOUR node's
     *  header chain (without it, at most DEMO); `anchor` (optional) a trusted ChainState JSON the core chain was proved
     *  from instead of genesis (then at most DEMO); `network` 'mainnet' (default) or 'signet'; `now` defaults to this
     *  device's clock (Date.now()): the module has no clock of its own and cannot bound a `now` you pass, so let a
     *  user type one only if moving time-too-new is what you mean. */
    verifyZeroconf({ proof, txid, wtxid, expectTip, anchor, network, now, imageHex, coreImageHex, vk, pin }) {
      const code = NETWORKS[network || 'mainnet'];
      if (code === undefined) throw new Error(`unknown network ${network}`);
      const enc = new TextEncoder();
      const parts = [proof, anchor ? enc.encode(anchor) : new Uint8Array(0), enc.encode(imageHex || ''),
        enc.encode(coreImageHex || ''), enc.encode(txid || ''), enc.encode(wtxid || ''), enc.encode(expectTip || ''),
        vk || new Uint8Array(0), pin || new Uint8Array(0)];
      const bytes = new Uint8Array(9 + parts.reduce((n, p) => n + 4 + p.length, 0));
      const view = new DataView(bytes.buffer);
      bytes[0] = code;
      view.setBigUint64(1, BigInt(Math.floor(now ?? Date.now() / 1000)), true);
      let o = 9;
      for (const p of parts) { view.setUint32(o, p.length, true); bytes.set(p, o + 4); o += 4 + p.length; }
      const ptr = alloc(bytes.length);
      new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes); // re-read buffer after alloc (it may grow)
      return JSON.parse(take(verify_zeroconf(ptr, bytes.length)));
    },
    /** The built-in zeroconf image id ("" until one is published). */
    zeroconfImageId() { return take(zeroconf_image_id()); },
    /** The module's memory size in bytes (for a test that many calls do not leak). */
    _memoryBytes() { return memory.buffer.byteLength; },
  };
}
