#!/usr/bin/env python3
"""check_xver_provenance.py <assembled lineage dir> <HZ_OVM_XVER_KEYS dir> <records dir>: the build-time half of a
cross-version handover's key check (review R1-5 #1, 27.11.2026).

In the guest, a predecessor's proof is accepted under the key whose SHA-256 is the pin in LINEAGE.last().verifier, and
image_id_of(key) must equal LINEAGE.last().image (verify-core Refusal::Image). That binds the key to the pin and the
image to the key's BASELINE, but not the key itself: OpenVM's key is not self-certifying (a forged mvk can carry the
genuine baseline). So the pin must be the one the PREDECESSOR RELEASE published for that image. This script refuses an
assembly unless, for every lineage file whose last handover crosses OpenVM versions (verifier != 0):
  * a published record (records dir, <image hex>.json: {"image", "key_pin", "openvm", "test"}) names that image with
    exactly that key pin;
  * the keys dir carries a key file named by that pin (<pin>.vk, or v2.0.2/<pin>.vk; assemble.sh has already checked
    each file hashes to its name);
  * a record marked "test" (a rehearsal's predecessor, never released) is accepted only for lineage/test.rs, the
    test-lineage variant that is never a published image.
It also refuses an ovm-xver assembly in which no lineage file crosses versions (keys embedded for nothing).
Exit 0 and one line per checked lineage; exit 1 with the reason."""
import json, os, re, struct, sys

lin_dir, keys_dir, rec_dir = sys.argv[1:4]
H = re.compile(r"Handover\s*\{\s*image:\s*\[([^\]]*)\],\s*first_height:\s*(\d+),\s*verifier:\s*\[([^\]]*)\]", re.S)


def ints(s):
    return [int(x, 0) for x in s.replace("\n", " ").split(",") if x.strip()]


def die(m):
    print(f"check_xver_provenance: {m}", file=sys.stderr)
    sys.exit(1)


records = {}
if os.path.isdir(rec_dir):
    for f in sorted(os.listdir(rec_dir)):
        if f.endswith(".json"):
            r = json.load(open(os.path.join(rec_dir, f)))
            records[r["image"].lower()] = r
crossing = 0
for f in sorted(os.listdir(lin_dir)):
    if not f.endswith(".rs"):
        continue
    hs = H.findall(open(os.path.join(lin_dir, f)).read())
    if not hs:
        continue
    words, first, ver = hs[-1]
    w, pin = ints(words), bytes(ints(ver))
    if len(w) != 8 or len(pin) != 32:
        die(f"lineage/{f}: the last handover is not 8 image words and a 32-byte verifier")
    if pin == bytes(32):
        continue   # SAME_ZKVM: an OpenVM deferral, no carried key
    crossing += 1
    image = struct.pack("<8I", *w).hex()
    p = pin.hex()
    r = records.get(image)
    if r is None:
        die(f"lineage/{f}: no published record for the predecessor image {image} in {rec_dir}")
    if r["key_pin"].lower() != p:
        die(f"lineage/{f}: the lineage pins key {p}, but the predecessor's published record says {r['key_pin']}")
    if r.get("test") and f != "test.rs":
        die(f"lineage/{f}: the record for {image} is a TEST record (a rehearsal's predecessor): only lineage/test.rs may use it")
    if not any(os.path.exists(os.path.join(keys_dir, d, p + ".vk")) for d in ("", "v2.0.2")):
        die(f"lineage/{f}: no carried key {p}.vk in {keys_dir}")
    print(f"  provenance: lineage/{f} hands over at {first} from {image[:16]}... under the published key {p[:16]}..."
          f"{' (TEST record)' if r.get('test') else ''}")
if crossing == 0:
    die("HZ_OVM_XVER_KEYS is set, but no lineage file hands over across OpenVM versions")
