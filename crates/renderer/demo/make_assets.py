"""Generate the demo's map and token images. Python 3 standard library only.

    python make_assets.py

Writes assets/cellar-map.png (1400x980, a 20x14 grid of 70-unit cells) and two
140x140 token portraits. The pictures are drawn by this script, so there is
nothing to license. They exist to exercise the renderer's image path.
"""
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "assets")


def png(path, width, height, rows, channels):
    """Write a PNG from `rows` (a list of bytes, one per scanline)."""
    color_type = {3: 2, 4: 6}[channels]
    raw = b"".join(b"\x00" + bytes(r) for r in rows)

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    data = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, color_type, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )
    with open(path, "wb") as f:
        f.write(data)


def noise(x, y):
    """Cheap deterministic per-pixel value in 0..7."""
    return ((x * 73856093) ^ (y * 19349663)) % 8


def make_map():
    w, h = 1400, 980
    # Rooms are given in world units: (x0, y0, x1, y1).
    walls = [
        (0, 0, 1400, 28), (0, 952, 1400, 980), (0, 0, 28, 980), (1372, 0, 1400, 980),
        (700, 28, 728, 350), (700, 490, 728, 952),  # divider with a doorway at y 350..490
    ]
    rug = (140, 560, 560, 840)
    barrels = [(80, 90), (80, 160), (150, 90), (1300, 880), (1230, 880), (1300, 810)]
    rows = []
    for y in range(h):
        row = bytearray()
        for x in range(w):
            tile = ((x // 35) + (y // 35)) % 2
            base = 58 if tile else 52
            c = (base + noise(x, y), base + 2 + noise(x, y), base + 8 + noise(x, y))
            if x % 35 == 0 or y % 35 == 0:
                c = (36, 37, 44)
            if rug[0] <= x < rug[2] and rug[1] <= y < rug[3]:
                inner = rug[0] + 14 <= x < rug[2] - 14 and rug[1] + 14 <= y < rug[3] - 14
                c = (122, 38 + noise(x, y), 44) if inner else (196, 160, 70)
            for bx, by in barrels:
                d2 = (x - bx) ** 2 + (y - by) ** 2
                if d2 < 26 * 26:
                    c = (84, 56, 30) if d2 > 20 * 20 else (112, 78, 44 + noise(x, y))
            for x0, y0, x1, y1 in walls:
                if x0 <= x < x1 and y0 <= y < y1:
                    c = (22, 22, 28) if (x + y) % 9 else (30, 30, 38)
            row += bytes(c)
        rows.append(row)
    png(os.path.join(OUT, "cellar-map.png"), w, h, rows, 3)


def make_token(name, body, face, extra):
    s = 140
    c = s / 2
    rows = []
    for y in range(s):
        row = bytearray()
        for x in range(s):
            dx, dy = x + 0.5 - c, y + 0.5 - c
            d2 = dx * dx + dy * dy
            px = (0, 0, 0, 0)
            if d2 <= 68 * 68:
                px = body + (255,)
                if d2 <= 40 * 40:
                    px = face + (255,)
                if d2 > 62 * 62:
                    px = (20, 20, 20, 255)
                px = extra(x, y, dx, dy, px)
            row += bytes(px)
        rows.append(row)
    png(os.path.join(OUT, name), s, s, rows, 4)


def aria_extra(x, y, dx, dy, px):
    # Two dark eyes and a smile line.
    for ex in (-14, 14):
        if (dx - ex) ** 2 + (dy + 8) ** 2 < 25:
            return (30, 30, 50, 255)
    if 14 < dy < 18 and abs(dx) < 16 and dx * dx / 256 + (dy - 12) ** 2 / 36 > 0.6:
        return (150, 60, 60, 255)
    return px


def goblin_extra(x, y, dx, dy, px):
    # Yellow slit eyes and a wide grin.
    for ex in (-15, 15):
        if abs(dx - ex) < 9 and abs(dy + 6) < 3:
            return (240, 220, 60, 255)
    if 16 < dy < 21 and abs(dx) < 24:
        return (240, 240, 230, 255)
    return px


if __name__ == "__main__":
    os.makedirs(OUT, exist_ok=True)
    make_map()
    make_token("aria.png", (52, 96, 170), (232, 196, 160), aria_extra)
    make_token("goblin.png", (56, 120, 56), (110, 170, 80), goblin_extra)
    for n in sorted(os.listdir(OUT)):
        print(n, os.path.getsize(os.path.join(OUT, n)), "bytes")
