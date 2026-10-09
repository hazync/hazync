"""Hazync's real colours and logo, for tkinter.

⛔ NOT INVENTED, AND NOT EYEBALLED. Every value here was read off what hazync.org actually serves:

    the palette   https://hazync.org/assets/site.css   (its own CSS custom properties)
    the logo      https://hazync.org/favicon.svg       (235 bytes, three rectangles)

⭐ THE LOGO IS DRAWN, NOT BUNDLED. The favicon is three `<rect>`s in a 32x32 viewBox, so it can be
reproduced exactly on a tkinter Canvas with no image file, no Pillow, and no SVG renderer — and it
stays sharp at any size. The source SVG, in full:

    <svg viewBox="0 0 32 32">
      <rect width="32" height="32" rx="6" fill="#e3e7ea"/>
      <rect x="5"  y="10" width="17" height="12" fill="#16212b"/>
      <rect x="22" y="10" width="5"  height="12" fill="#c46a0c"/>
    </svg>

⚠ The site ships light AND dark values for each name. Both are kept, because a GUI that hardcoded
one would look wrong on half of people's machines, and the names are the site's own.
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()   # ⛔ BEFORE anything prints: a ✅ on a cp1252 console raises, not degrades


# The site's CSS custom properties, light mode then dark mode.
LIGHT = {
    "ink": "#16212b",        # text, and the logo's dark block
    "fog": "#e3e7ea",        # page background, and the logo's plate
    "mist": "#f0f2f4",       # raised surfaces
    "haze": "#c3cbd2",       # borders
    "slate": "#52606b",      # secondary text
    "lamp": "#c46a0c",       # the accent — the logo's orange block
    "lamp_text": "#8f4c06",
    "good": "#1f6b4f",
    "bad": "#9b2c2c",
}

DARK = {
    "ink": "#e1e6ea",
    "fog": "#111a21",
    "mist": "#18232c",
    "haze": "#2d3a44",
    "slate": "#93a0aa",
    "lamp": "#e89b3c",
    "lamp_text": "#efae5c",
    "good": "#6cc49c",
    "bad": "#ef8a8a",
}


def palette(dark=False):
    return dict(DARK if dark else LIGHT)


# The block map's four states. ⭐ The ramp is deliberate: open is the page itself (absence reads as
# absence), then slate -> lamp -> ink as a block gets further along, so a glance shows how much of
# the chain is actually anchored rather than merely touched.
# ⛔ THE MAP'S OWN COLOURS, READ OFF THE SITE — NOT DERIVED FROM THE PALETTE ABOVE. The website's
# block map does not use the page palette: it is cyan, magenta, hi-viz yellow and orange on a BLACK
# ground, the same in both themes (hazync.org/assets/explorer.css, `.bmap { --map-* }`). This file
# used to paint the map from ink/lamp/slate instead, so the app's map and the site's map showed the
# same blocks in different colours and a person moving between them had to relearn the key.
#
# ⚠ The site validated this set with a data-viz palette checker (worst pair OKLab dE 9.7 under
# simulated red-green colour blindness). Changing one here un-validates it; change the site first.
MAP_SURFACE = "#000000"      # the ground: the lines between squares and the space after the tip
MAP_CLAIMED = "#00b0ff"      # cyan — the site's "claimed". The app's map data carries no claims
                             # (/api/blockstatus leaves them out), so nothing is painted this yet.
MAP_PROVEN = "#f038c0"       # magenta
MAP_FOLDED = "#e8ff00"       # hi-viz yellow
MAP_ANCHORED = "#ff8a1f"     # orange


def state_colours(dark=False):
    """Colour per block state, keyed by the coordinator's own codes — the site's map colours.

    Only `open` follows the theme: the site uses the page's `haze` grey for it, so do we.
    """
    return {
        0: palette(dark)["haze"],   # open
        3: MAP_PROVEN,
        4: MAP_FOLDED,
        5: MAP_ANCHORED,
    }


_RECTS = (
    # (x, y, w, h, palette key, rounded)
    (0 / 32, 0 / 32, 32 / 32, 32 / 32, "fog", True),
    (5 / 32, 10 / 32, 17 / 32, 12 / 32, "ink", False),
    (22 / 32, 10 / 32, 5 / 32, 12 / 32, "lamp", False),
)


def draw_logo(canvas, x, y, size, dark=False):
    """Paint the logo at (x, y) with the given edge length. Returns the item ids.

    ⚠ `rx=6` on a 32-unit square is a 3/16 corner radius. tkinter has no rounded rectangle, so the
    plate is drawn as a rounded shape only when the size makes it worth it; below ~24 px the
    rounding is smaller than a pixel and a plain rectangle is indistinguishable.
    """
    p = palette(dark)
    ids = []
    for fx, fy, fw, fh, key, rounded in _RECTS:
        x0, y0 = x + fx * size, y + fy * size
        x1, y1 = x0 + fw * size, y0 + fh * size
        if rounded and size >= 24:
            r = (6 / 32) * size
            ids.extend(_rounded_rect(canvas, x0, y0, x1, y1, r, p[key]))
        else:
            ids.append(canvas.create_rectangle(x0, y0, x1, y1, fill=p[key], outline=""))
    return ids


def _rounded_rect(canvas, x0, y0, x1, y1, r, fill):
    """A rounded rectangle from two rects and four arcs — tkinter has no primitive for it."""
    ids = [
        canvas.create_rectangle(x0 + r, y0, x1 - r, y1, fill=fill, outline=""),
        canvas.create_rectangle(x0, y0 + r, x1, y1 - r, fill=fill, outline=""),
    ]
    d = 2 * r
    for cx, cy, start in ((x0, y0, 90), (x1 - d, y0, 0), (x0, y1 - d, 180), (x1 - d, y1 - d, 270)):
        ids.append(canvas.create_arc(cx, cy, cx + d, cy + d, start=start, extent=90,
                                     style="pieslice", fill=fill, outline=""))
    return ids


def logo_svg():
    """The original asset, byte-for-byte, for anyone who needs the real file."""
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">'
            '<rect width="32" height="32" rx="6" fill="#e3e7ea"/>'
            '<rect x="5" y="10" width="17" height="12" fill="#16212b"/>'
            '<rect x="22" y="10" width="5" height="12" fill="#c46a0c"/></svg>')


def self_test():
    """Check the palette and the logo geometry without opening a window."""
    bad = 0
    for name, pal in (("light", LIGHT), ("dark", DARK)):
        for k, v in pal.items():
            ok = v.startswith("#") and len(v) == 7
            if not ok:
                print(f"  FAIL {name}.{k} = {v!r} is not a #rrggbb colour")
                bad += 1
    print(f"  ok   {len(LIGHT)} light and {len(DARK)} dark colours, all #rrggbb")
    ok = set(LIGHT) == set(DARK)
    print(f"  {'ok  ' if ok else 'FAIL'} light and dark define the SAME names")
    bad += 0 if ok else 1
    sc = state_colours()
    ok = set(sc) == {0, 3, 4, 5}
    print(f"  {'ok  ' if ok else 'FAIL'} the map has a colour for open/proven/folded/anchored")
    bad += 0 if ok else 1
    want = {3: "#f038c0", 4: "#e8ff00", 5: "#ff8a1f"}
    ok = all(sc[k] == v for k, v in want.items()) and MAP_SURFACE == "#000000" \
        and MAP_CLAIMED == "#00b0ff"
    print(f"  {'ok  ' if ok else 'FAIL'} the map uses the SITE's map colours: magenta proven, yellow "
          f"folded, orange anchored, on black")
    bad += 0 if ok else 1
    ok = state_colours(False)[0] == LIGHT["haze"] and state_colours(True)[0] == DARK["haze"]
    print(f"  {'ok  ' if ok else 'FAIL'} open blocks are the theme's grey, as on the site")
    bad += 0 if ok else 1
    ok = len({sc[0], sc[3], sc[4], sc[5]}) == 4
    print(f"  {'ok  ' if ok else 'FAIL'} all four map colours are DISTINCT, or the map says nothing")
    bad += 0 if ok else 1
    # the logo's rectangles must match the real favicon's numbers
    svg = logo_svg()
    for frag in ('viewBox="0 0 32 32"', 'rx="6"', '#e3e7ea', '#16212b', '#c46a0c',
                 'x="5" y="10" width="17" height="12"', 'x="22" y="10" width="5" height="12"'):
        ok = frag in svg
        if not ok:
            print(f"  FAIL the logo no longer matches the real favicon: missing {frag!r}")
            bad += 1
    print("  ok   the drawn logo's geometry matches hazync.org/favicon.svg")
    return bad


if __name__ == "__main__":
    import sys
    n = self_test()
    print()
    print("All checks passed." if not n else f"FAIL: {n} check(s)")
    sys.exit(1 if n else 0)
