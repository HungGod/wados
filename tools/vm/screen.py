#!/usr/bin/env python3
"""Is anything on the screen, and where did it change? Reads screenshots
from QEMU's screendump (8-bit RGB or RGBA PNGs, not interlaced).

    screen.py FILE.png      how much of it isn't the colour of its corner;
                            exit 0 when at least MIN_DRAWN is drawn, 1 when
                            it's blank
    screen.py A.png B.png [X0 Y0 X1 Y1]
                            the box where they differ ("x0 y0 x1 y1", the
                            last pixels inside), looking only inside the
                            given box if there is one; exit 1 when they don't
    screen.py --ink FILE.png
                            a page of big black text on white is drawn:
                            exit 0 when there's enough of both

Standard library only.
"""

import struct
import sys
import zlib

# A blank screen is one colour; a desktop, even a dark one, has icons and a bar.
MIN_DRAWN = 0.002


def pixels(path):
    """(width, height, bytes per pixel, rows of raw pixels)."""
    with open(path, "rb") as f:
        data = f.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit("screen.py: not a PNG")
    pos, idat, ihdr = 8, b"", None
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            ihdr = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    w, h, depth, colour, _, _, interlace = ihdr
    if depth != 8 or colour not in (2, 6) or interlace:
        raise SystemExit(f"screen.py: an 8-bit RGB(A) PNG only (depth {depth}, colour type {colour})")
    bpp = 3 if colour == 2 else 4
    raw = zlib.decompress(idat)
    stride = w * bpp
    rows, prev = [], bytearray(stride)
    for y in range(h):
        f = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(line))
        prev = line
    return w, h, bpp, rows


def diff(a, b, within=None):
    wa, ha, ba, ra = pixels(a)
    wb, hb, bb, rb = pixels(b)
    if (wa, ha) != (wb, hb):
        raise SystemExit(f"screen.py: {wa}×{ha} and {wb}×{hb}")
    lx, ly, hx, hy = within or (0, 0, wa - 1, ha - 1)
    x0 = y0 = None
    x1 = y1 = -1
    for y in range(max(0, ly), min(ha - 1, hy) + 1):
        la, lb = ra[y], rb[y]
        if la == lb and ba == bb:
            continue
        for x in range(max(0, lx), min(wa - 1, hx) + 1):
            if la[x * ba:x * ba + 3] != lb[x * bb:x * bb + 3]:
                x0 = x if x0 is None else min(x0, x)
                y0 = y if y0 is None else y0
                x1, y1 = max(x1, x), y
    if x0 is None:
        print("the same")
        sys.exit(1)
    print(f"{x0} {y0} {x1} {y1}")


def ink(path):
    """A web page of big black text on white: how much of each there is."""
    w, h, bpp, rows = pixels(path)
    black = white = 0
    for line in rows:
        for x in range(0, w * bpp, bpp):
            r, g, b = line[x], line[x + 1], line[x + 2]
            if r < 8 and g < 8 and b < 8:  # the text (#000), not the dark chrome (#080815)
                black += 1
            elif r > 230 and g > 230 and b > 230:
                white += 1
    print(f"black {black}, white {white}")
    sys.exit(0 if black > MIN_INK and white > MIN_PAPER else 1)


# A page of the check's (huge text on white) has far more of both than this.
MIN_INK = 5000
MIN_PAPER = 100000


def main(argv):
    if len(argv) == 3 and argv[1] == "--ink":
        ink(argv[2])
        return
    if len(argv) in (3, 7):
        diff(argv[1], argv[2], tuple(int(v) for v in argv[3:]) if len(argv) == 7 else None)
        return
    if len(argv) != 2:
        raise SystemExit(__doc__)
    w, h, bpp, rows = pixels(argv[1])
    corner = rows[0][:3]
    drawn = 0
    for line in rows:
        for x in range(0, w * bpp, bpp):
            if line[x:x + 3] != corner:
                drawn += 1
    share = drawn / (w * h)
    print(f"{w}×{h}: {share:.1%} drawn (the rest is #{corner.hex()})")
    sys.exit(0 if share >= MIN_DRAWN else 1)


if __name__ == "__main__":
    main(sys.argv)
