# lineage files and key / record dirs for the R1-5 #3 images (28.11.2026)
import json, os, shutil, struct, sys
W, pinA, pinB = sys.argv[1:4]
A = [3087990471, 273427805, 423765843, 714736923, 1323484124, 3246500386, 4007801574, 3375336817]  # keys-ch (step 2)
imgA = struct.pack("<8I", *A).hex()
def h(img, first, pin):
    v = ", ".join(f"0x{b:02x}" for b in bytes.fromhex(pin))
    return (f"crate::handover_pin::Handover {{\n    image: [{', '.join(map(str, img))}],\n    first_height: {first},\n"
            f"    verifier: [{v}],\n}}")
def lin(path, hs, note):
    open(path, "w").write(f"// {note}\npub const LINEAGE: &[crate::handover_pin::Handover] = &[" + ", ".join(hs) + "];\n")
lin(f"{W}/lin1.rs", [h(A, 200, pinA)], "R1-5 #3 (28.11.2026): keys-ch's chain handed over at 200 (step 2's lineage)")
lin(f"{W}/lin6.rs", [h(A, 200, pinB)], "R1-5 #3 case 6: keys-ch's image pinned with ANOTHER image's key pin (B)")
lin(f"{W}/lin7.rs", [h(A, 150, pinA), h(A, 200, pinA)], "R1-5 #3 case 7: a two-entry lineage the predecessor does not have")
for d, pin, src in (("keysA", pinA, "/root/engine/xvkeys/%s.vk" % pinA), ("keysB", pinB, f"{W}/vkB/vk.bin")):
    os.makedirs(f"{W}/{d}", exist_ok=True); shutil.copy(src, f"{W}/{d}/{pin}.vk")
for d, pin, note in (("recA", pinA, "TRUE: keys-ch's key file's pin (step 2)"),
                     ("recB", pinB, "DELIBERATELY FALSE (case 6 bypass): keys-ch's image with the default image's key pin")):
    os.makedirs(f"{W}/{d}", exist_ok=True)
    json.dump({"image": imgA, "key_pin": pin, "openvm": "v2.1.0", "test": True, "note": "R1-5 #3 " + note},
              open(f"{W}/{d}/{imgA}.json", "w"), indent=1)
print("image A", imgA)
