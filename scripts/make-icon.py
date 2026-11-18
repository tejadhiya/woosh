#!/usr/bin/env python3
"""Generate Woosh's app icon set.

Reproducible so the icon is never a mystery binary in the repo. Renders at 4x and
downsamples with Lanczos, because the mark has to stay legible at 32px in the
Dock -- which is what drives the design: three tapered motion streaks, thick at
the head and pointed at the tail, on a dark rounded square.
"""
import math
import os
import subprocess
import sys
from PIL import Image, ImageDraw, ImageFilter

S = 4096          # supersampled canvas
SS = S // 1024    # scale factor vs the 1024 master

BG_TOP = (14, 18, 22)
BG_BOT = (21, 27, 33)
MINT = (110, 231, 183)
MINT_DIM = (72, 190, 160)
CYAN = (125, 231, 229)


def rounded_mask(size, radius):
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=255)
    return m


def vertical_gradient(size, top, bottom):
    g = Image.new("RGB", (1, size))
    px = g.load()
    for y in range(size):
        t = y / max(1, size - 1)
        px[0, y] = tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(3))
    return g.resize((size, size), Image.NEAREST)


def streak(draw, x_head, y_mid, length, head_h, color, angle_deg=-9.0):
    """A motion streak: rounded thick head on the right, tapering to a point left."""
    a = math.radians(angle_deg)
    ca, sa = math.cos(a), math.sin(a)

    def rot(px, py):
        dx, dy = px - x_head, py - y_mid
        return (x_head + dx * ca - dy * sa, y_mid + dx * sa + dy * ca)

    steps = 160
    top, bot = [], []
    for i in range(steps + 1):
        t = i / steps                       # 0 = head (right), 1 = tail (left)
        x = x_head - length * t
        # Taper: full height at the head, easing to a point at the tail.
        h = head_h * (1.0 - t) ** 1.55 / 2.0
        top.append(rot(x, y_mid - h))
        bot.append(rot(x, y_mid + h))
    draw.polygon(top + bot[::-1], fill=color)
    # Round the head cap.
    r = head_h / 2.0
    cx, cy = rot(x_head - r * 0.15, y_mid)
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=color)


def build_master():
    img = Image.new("RGB", (S, S))
    img.paste(vertical_gradient(S, BG_TOP, BG_BOT), (0, 0))

    # Mark on its own layer so it can be softly glowed before compositing.
    mark = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(mark)

    cx, cy = S * 0.585, S * 0.5
    # Middle streak is longest and brightest; outer two shorter and dimmer, which
    # is what makes the shape read as motion rather than as three bars.
    streak(d, cx + 150 * SS, cy,              430 * SS, 118 * SS, MINT + (255,))
    streak(d, cx + 40 * SS,  cy - 185 * SS,   300 * SS, 86 * SS,  CYAN + (205,))
    streak(d, cx + 10 * SS,  cy + 190 * SS,   250 * SS, 78 * SS,  MINT_DIM + (185,))

    glow = mark.filter(ImageFilter.GaussianBlur(26 * SS))
    img = Image.alpha_composite(img.convert("RGBA"), glow)
    img = Image.alpha_composite(img, mark)

    # Hairline inner edge, the way macOS app icons catch light.
    d2 = ImageDraw.Draw(img)
    d2.rounded_rectangle(
        [6 * SS, 6 * SS, S - 6 * SS, S - 6 * SS],
        radius=int(S * 0.185), outline=(255, 255, 255, 18), width=4 * SS,
    )

    master = img.resize((1024, 1024), Image.LANCZOS)
    master.putalpha(rounded_mask(1024, int(1024 * 0.1875)))
    return master


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "app/src-tauri/icons"
    os.makedirs(out, exist_ok=True)
    master = build_master()
    master.save(os.path.join(out, "icon.png"))

    # Sizes Tauri's bundler references plus the full iconset for iconutil.
    for size in (16, 32, 64, 128, 256, 512, 1024):
        master.resize((size, size), Image.LANCZOS).save(
            os.path.join(out, f"{size}x{size}.png")
        )
    master.resize((256, 256), Image.LANCZOS).save(os.path.join(out, "128x128@2x.png"))

    iconset = os.path.join(out, "icon.iconset")
    os.makedirs(iconset, exist_ok=True)
    for base in (16, 32, 128, 256, 512):
        master.resize((base, base), Image.LANCZOS).save(
            os.path.join(iconset, f"icon_{base}x{base}.png")
        )
        master.resize((base * 2, base * 2), Image.LANCZOS).save(
            os.path.join(iconset, f"icon_{base}x{base}@2x.png")
        )
    try:
        subprocess.run(
            ["iconutil", "-c", "icns", iconset, "-o", os.path.join(out, "icon.icns")],
            check=True,
        )
        print("wrote icon.icns")
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"iconutil unavailable ({e}); .icns not generated", file=sys.stderr)
    print(f"icons written to {out}")


if __name__ == "__main__":
    main()
