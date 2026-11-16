"""Draw the flowGit app icon and build assets/AppIcon.png and AppIcon.icns.

A branch leaves the main line, flows out and merges back: the commit graph as
one continuous motion. Drawn at 4x and scaled down for clean edges.

    python3 scripts/make-icon.py
"""
import math
import os
import shutil
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw, ImageFilter

SIZE = 1024
SS = 4  # supersampling
S = SIZE * SS
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "assets")

TOP = (124, 92, 255)      # violet
BOTTOM = (16, 185, 160)   # teal
INK = (255, 255, 255)


def lerp(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def squircle_mask(size, inset, radius):
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle(
        (inset, inset, size - inset, size - inset), radius=radius, fill=255
    )
    return m


def bezier(p0, p1, p2, p3, n=120):
    pts = []
    for i in range(n + 1):
        t = i / n
        u = 1 - t
        x = u**3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0]
        y = u**3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1]
        pts.append((x, y))
    return pts


def stroke(draw, pts, width, fill):
    """A solid round-capped stroke: a disc stamped every pixel along the path.
    PIL's wide polylines leave gaps on tight curves."""
    r = width / 2
    dense = [pts[0]]
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        steps = max(1, int(math.hypot(x1 - x0, y1 - y0)))
        dense += [(x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps) for i in range(1, steps + 1)]
    for x, y in dense:
        draw.ellipse((x - r, y - r, x + r, y + r), fill=fill)


def draw_icon():
    k = SS
    inset, radius = 100 * k, 185 * k

    # Diagonal gradient body.
    grad = Image.new("RGB", (S, S))
    gd = ImageDraw.Draw(grad)
    for i in range(0, 2 * S, 4):
        t = min(1.0, i / (2 * S))
        gd.line([(i, 0), (0, i)], fill=lerp(TOP, BOTTOM, t), width=6)
    body = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    body.paste(grad, (0, 0), squircle_mask(S, inset, radius))

    # The graph.
    art = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(art)
    w = 62 * k
    main_x = 392 * k
    top, bottom = 268 * k, 756 * k
    branch = (650 * k, 512 * k)
    out_from, back_to = (main_x, 352 * k), (main_x, 672 * k)

    stroke(d, [(main_x, top), (main_x, bottom)], w, INK)
    stroke(d, bezier(out_from, (main_x, 470 * k), (branch[0], 380 * k), branch), w, INK)
    stroke(d, bezier(branch, (branch[0], 644 * k), (main_x, 556 * k), back_to), w, INK)

    def node(c, r_out, r_in, color):
        x, y = c
        d.ellipse((x - r_out, y - r_out, x + r_out, y + r_out), fill=INK)
        d.ellipse((x - r_in, y - r_in, x + r_in, y + r_in), fill=color)

    node((main_x, top), 70 * k, 34 * k, lerp(TOP, BOTTOM, 0.18))
    node(branch, 70 * k, 34 * k, lerp(TOP, BOTTOM, 0.55))
    node((main_x, bottom), 70 * k, 34 * k, lerp(TOP, BOTTOM, 0.85))

    # A faint shadow under the graph lifts it off the gradient.
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    shadow.paste((20, 10, 60, 70), (0, 10 * k), art.split()[3])
    shadow = shadow.filter(ImageFilter.GaussianBlur(14 * k))
    body = Image.alpha_composite(body, shadow)
    body = Image.alpha_composite(body, art)

    # Drop shadow under the whole tile, as macOS icons have.
    out = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    tile_shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    tile_shadow.paste((0, 0, 0, 90), (0, 12 * k), squircle_mask(S, inset, radius))
    tile_shadow = tile_shadow.filter(ImageFilter.GaussianBlur(18 * k))
    out = Image.alpha_composite(out, tile_shadow)
    out = Image.alpha_composite(out, body)
    return out.resize((SIZE, SIZE), Image.LANCZOS)


def main():
    icon = draw_icon()
    png = os.path.join(ASSETS, "AppIcon.png")
    icon.save(png)
    if sys.platform != "darwin":
        print(f"Wrote {png} (icns needs macOS iconutil)")
        return
    tmp = tempfile.mkdtemp()
    iconset = os.path.join(tmp, "AppIcon.iconset")
    os.mkdir(iconset)
    for base in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            px = base * scale
            name = f"icon_{base}x{base}{'@2x' if scale == 2 else ''}.png"
            icon.resize((px, px), Image.LANCZOS).save(os.path.join(iconset, name))
    icns = os.path.join(ASSETS, "AppIcon.icns")
    subprocess.run(["iconutil", "-c", "icns", iconset, "-o", icns], check=True)
    shutil.rmtree(tmp)
    print(f"Wrote {png} and {icns}")


if __name__ == "__main__":
    main()
