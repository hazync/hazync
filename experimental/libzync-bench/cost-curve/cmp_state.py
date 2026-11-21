"""cmp_state.py <executed state_h.json> <prep's state_h.json>: field-by-field comparison of the state the leaf
execution committed (decoded from its dev-mode receipt journal) with the state libzync-prep recorded for that height.
A leaf is a base step on S(h-1), so its anchor_id is the digest of S(h-1), while prep's run from genesis carries the
genesis anchor's: that one field differs by construction. Prints STATE_MATCH or STATE_MISMATCH <fields>."""
import json, sys
a, b = (json.load(open(p)) for p in sys.argv[1:3])
diff = sorted(k for k in set(a) | set(b) if a.get(k) != b.get(k))
rest = [k for k in diff if k != "anchor_id"]
print(("STATE_MATCH" if not rest else "STATE_MISMATCH " + ",".join(rest)) +
      f" (fields {len(a)}; differing: {','.join(diff) or 'none'}; height {a.get('height')} utxo_leaves {a.get('utxo_leaves')})")
sys.exit(1 if rest else 0)
