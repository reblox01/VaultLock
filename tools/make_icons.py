"""Generate the VaultLock app icon set from a drawn master.

macOS-style app icon: rounded-square tile, system-blue gradient,
white monoline-style lock (shackle arc + rounded body + keyhole).
Outputs: icon.png (512), icon.ico (multi-size), icon.icns (PNG-backed).
"""
import struct
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"
OUT.mkdir(parents=True, exist_ok=True)

SIZE = 1024
RADIUS = 225  # ~22% - macOS squircle-ish corner
TOP = (26, 132, 255)      # #0A84FF system blue
BOTTOM = (0, 82, 204)     # deepened blue
HI = (120, 185, 255)      # top sheen


def rounded_mask(size, radius):
    m = Image.new("L", (size, size), 0)
    d = ImageDraw.Draw(m)
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=255)
    return m


def master(size=SIZE):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    # vertical gradient tile
    grad = Image.new("RGBA", (size, size))
    gd = ImageDraw.Draw(grad)
    for y in range(size):
        t = y / (size - 1)
        c = tuple(int(TOP[i] + (BOTTOM[i] - TOP[i]) * t) for i in range(3))
        gd.line([(0, y), (size, y)], fill=c + (255,))
    tile_mask = rounded_mask(size, int(size * RADIUS / SIZE))
    img.paste(grad, (0, 0), tile_mask)
    # top sheen: soft light band, alpha-composited and clipped to the tile
    sheen = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    sd = ImageDraw.Draw(sheen)
    sd.rounded_rectangle([0, -size // 3, size, size // 4], radius=size // 8,
                         fill=HI + (42,))
    alpha = ImageChops.multiply(sheen.getchannel("A"), tile_mask)
    sheen.putalpha(alpha)
    img = Image.alpha_composite(img, sheen)
    d = ImageDraw.Draw(img, "RGBA")
    # lock, centred slightly high, white (flat, no shadow: clean Apple look)
    cx, cy = size // 2, size // 2 + size // 40
    body_w, body_h = int(size * 0.46), int(size * 0.34)
    body_x0, body_y0 = cx - body_w // 2, cy - body_h // 2 + size // 20
    body_x1, body_y1 = cx + body_w // 2, cy + body_h // 2 + size // 20
    shackle_w = int(size * 0.095)          # stroke width
    shackle_r = int(size * 0.155)          # shackle radius
    shackle_cy = body_y0 + size // 24      # shackle centre just above body top
    # shackle: white arc, open at the bottom (ends hidden behind body)
    d.arc([cx - shackle_r, shackle_cy - shackle_r,
           cx + shackle_r, shackle_cy + shackle_r],
          start=180, end=360, fill=(255, 255, 255, 255), width=shackle_w)
    # body
    d.rounded_rectangle([body_x0, body_y0, body_x1, body_y1],
                        radius=int(size * 0.05), fill=(255, 255, 255, 255))
    # keyhole: blue circle + slot
    kr = int(size * 0.045)
    kx, ky = cx, body_y0 + int(body_h * 0.38)
    d.ellipse([kx - kr, ky - kr, kx + kr, ky + kr], fill=BOTTOM + (255,))
    slot_w, slot_h = int(size * 0.038), int(size * 0.075)
    d.rounded_rectangle([kx - slot_w // 2, ky, kx + slot_w // 2, ky + slot_h],
                        radius=slot_w // 2, fill=BOTTOM + (255,))
    return img


def write_icns(png_by_size, path):
    """Minimal modern .icns: PNG data in ic07/ic08/ic09/ic10 elements."""
    types = [(128, b"ic07"), (256, b"ic08"), (512, b"ic09"), (1024, b"ic10")]
    body = b""
    for size, tag in types:
        data = png_by_size[size]
        body += tag + struct.pack(">I", 8 + len(data)) + data
    total = 8 + len(body)
    path.write_bytes(b"icns" + struct.pack(">I", total) + body)


def main():
    m = master()
    pngs = {}
    for s in (16, 32, 48, 64, 128, 256, 512, 1024):
        pngs[s] = m.resize((s, s), Image.LANCZOS)
    # icon.png (512) - the one Tauri lists
    pngs[512].save(OUT / "icon.png", format="PNG")
    # icon.ico - multi-size
    pngs[256].save(OUT / "icon.ico", format="ICO",
                   sizes=[(16, 16), (24, 24), (32, 32), (48, 48),
                          (64, 64), (128, 128), (256, 256)])
    # icon.icns
    import io
    raw = {}
    for s in (128, 256, 512, 1024):
        buf = io.BytesIO()
        pngs[s].save(buf, format="PNG")
        raw[s] = buf.getvalue()
    write_icns(raw, OUT / "icon.icns")
    for f in sorted(OUT.iterdir()):
        print(f.name, f.stat().st_size, "bytes")


if __name__ == "__main__":
    main()
