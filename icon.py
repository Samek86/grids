# Draws the Grids app icon and writes app.ico (PNG entries, 16..256 px), transparent background.
# A square cut once vertically, the left half cut once horizontally: 1 (top left), 2 (bottom left),
# 3 (the whole right half). Tile 1 is a deep pastel green tilted 20 degrees, as if lifted off the grid;
# 2 and 3 are mid gray. All tiles have rounded corners.
# Usage: python icon.py [preview_dir]
import math, struct, sys, zlib

SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
GREEN, GRAY = (0x4F, 0x96, 0x7A), (0x9C, 0xA1, 0xA9)
TILT = math.radians(20)


def tile(cx, cy, hw, hh, r, angle, color):
    """Rounded rectangle centred at (cx, cy) with half sizes hw/hh, corner radius r, rotated by angle."""
    cos, sin = math.cos(angle), math.sin(angle)

    def inside(x, y):
        dx, dy = x - cx, y - cy
        u, v = dx * cos + dy * sin, -dx * sin + dy * cos
        qx, qy = abs(u) - (hw - r), abs(v) - (hh - r)
        return math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) <= r

    reach = math.hypot(hw, hh)
    return inside, (cx - reach, cy - reach, cx + reach, cy + reach), color


def render(n):
    m = n * 0.06
    g = max(1.0, n * 0.075)
    span = n - 2 * m
    lw = (span - g) / 2  # left column width = left cells' height
    r = max(0.8, lw * 0.16)
    left_cx = m + lw / 2
    tiles = [
        tile(left_cx, n - m - lw / 2, lw / 2, lw / 2, r, 0, GRAY),  # 2
        tile(n - m - lw / 2, n / 2, lw / 2, span / 2, r, 0, GRAY),  # 3
        tile(left_cx, m + lw / 2, lw * 0.43, lw * 0.43, r * 0.9, TILT, GREEN),  # 1, shrunk so it clears the gap
    ]
    ss = 8 if n <= 48 else 4
    px = bytearray(n * n * 4)
    for y in range(n):
        for x in range(n):
            out = (0.0, 0.0, 0.0, 0.0)  # premultiplied rgb + alpha
            for inside, (x0, y0, x1, y1), c in tiles:
                if x + 1 < x0 or x > x1 or y + 1 < y0 or y > y1:
                    continue
                hit = sum(inside(x + (sx + 0.5) / ss, y + (sy + 0.5) / ss) for sy in range(ss) for sx in range(ss))
                a = hit / (ss * ss)
                if a:  # paint over what's below
                    out = tuple(c[k] * a + out[k] * (1 - a) for k in range(3)) + (a + out[3] * (1 - a),)
            if out[3]:
                px[(y * n + x) * 4:(y * n + x) * 4 + 4] = bytes([round(out[k] / out[3]) for k in range(3)] + [round(out[3] * 255)])
    return png(n, px)


def png(n, px):
    raw = b"".join(b"\0" + bytes(px[y * n * 4:(y + 1) * n * 4]) for y in range(n))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", n, n, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


images = [(n, render(n)) for n in SIZES]
head = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
entries, blobs = b"", b""
for n, data in images:
    entries += struct.pack("<BBBBHHII", n % 256, n % 256, 0, 0, 1, 32, len(data), offset + len(blobs))
    blobs += data
open("app.ico", "wb").write(head + entries + blobs)
if len(sys.argv) > 1:
    for n, data in images:
        open(f"{sys.argv[1]}/icon{n}.png", "wb").write(data)
print("app.ico", len(head + entries + blobs), "bytes")
