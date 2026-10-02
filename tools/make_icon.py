#!/usr/bin/env python3
"""Draws the app icon (a salmon nigiri with a face) with the standard library only.

    tools/make_icon.py          # writes app/src-tauri/icons/{icon.png,32x32.png,128x128.png,icon.ico}
    tools/make_icon.py --check  # fails if the files on disk differ from what this would write
"""
import math
import struct
import sys
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "app" / "src-tauri" / "icons"
S = 256
SS = 3  # supersampling per axis

RICE, RICE_EDGE, SALMON, FAT, INK, CHEEK = (251, 246, 236), (227, 216, 196), (255, 138, 107), (255, 201, 181), (43, 32, 35), (255, 154, 166)


def rrect(x, y, cx, cy, w, h, r):
    """Inside a rounded rectangle centred on (cx, cy)?"""
    dx, dy = abs(x - cx) - (w / 2 - r), abs(y - cy) - (h / 2 - r)
    return math.hypot(max(dx, 0), max(dy, 0)) + min(max(dx, dy), 0) <= r


def ellipse(x, y, cx, cy, rx, ry):
    return ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2 <= 1


def shade(x, y):
    """RGBA of the icon at (x, y) in 0..1 coordinates scaled to S."""
    if rrect(x, y, 128, 128, 248, 248, 56):  # background tile
        c = (255, 244, 232, 255)
    else:
        return (0, 0, 0, 0)
    if rrect(x, y, 128, 158, 190, 92, 44):
        c = RICE_EDGE + (255,)
        if rrect(x, y, 128, 156, 184, 86, 41):
            c = RICE + (255,)
    if rrect(x, y, 128, 104, 204, 80, 40):
        c = SALMON + (255,)
        if rrect(x, y, 128, 98, 100, 8, 4) or rrect(x, y, 128, 116, 66, 7, 3.5):
            c = FAT + (255,)
    for ex in (101, 155):
        if rrect(x, y, ex, 156, 17, 28, 8.5):
            c = INK + (255,)
        if ellipse(x, y, ex + 2, 148, 3.5, 3.5):
            c = (255, 255, 255, 255)
    for cx in (72, 184):
        if rrect(x, y, cx, 176, 24, 13, 6.5):
            c = CHEEK + (255,)
    if rrect(x, y, 128, 180, 26, 9, 4.5):
        c = INK + (255,)
    return c


def render(size):
    px = bytearray()
    scale = S / size
    for j in range(size):
        px.append(0)  # PNG filter: none
        for i in range(size):
            r = g = b = a = 0
            n = SS * SS
            for sj in range(SS):
                for si in range(SS):
                    cr, cg, cb, ca = shade((i + (si + 0.5) / SS) * scale, (j + (sj + 0.5) / SS) * scale)
                    r, g, b, a = r + cr * ca, g + cg * ca, b + cb * ca, a + ca
            if a:
                px += bytes((round(r / a), round(g / a), round(b / a), round(a / n)))
            else:
                px += bytes(4)
    return png(size, bytes(px))


def png(size, raw):
    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def ico(images):
    """An .ico holding PNG images (supported since Windows Vista)."""
    head = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, blobs = b"", b""
    for size, data in images:
        entries += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset + len(blobs))
        blobs += data
    return head + entries + blobs


def main():
    sizes = {n: render(n) for n in (32, 128, 256)}
    files = {
        "icon.png": sizes[256],
        "32x32.png": sizes[32],
        "128x128.png": sizes[128],
        "icon.ico": ico([(32, sizes[32]), (256, sizes[256])]),
    }
    if "--check" in sys.argv:
        bad = [n for n, d in files.items() if not (OUT / n).exists() or (OUT / n).read_bytes() != d]
        if bad:
            sys.exit("icons out of date: " + ", ".join(bad))
        return
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in files.items():
        (OUT / name).write_bytes(data)


main()
