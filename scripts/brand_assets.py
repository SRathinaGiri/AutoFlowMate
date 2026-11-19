"""Build the app icon, splash and README images from the artwork in assets/brand/.

    python scripts/brand_assets.py

Needs Pillow (pip install pillow). Writes:
  assets/autoflowmate.ico     Windows icon: 16-48 px use a simplified mark (the flowchart "F"),
                              64-256 px the full logo, since the robot and letters blur when small
  assets/autoflowmate.png     256 px window icon
  assets/splash.png           banner for Studio's splash screen and About window
  assets/wordmark.png         logo with dark text (light backgrounds)
  assets/wordmark-dark.png    logo with white text (dark backgrounds, e.g. GitHub dark mode)
  assets/social-preview.png   1280 x 640 banner for GitHub's social preview (repository settings)
"""

from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "assets" / "brand"
OUT = ROOT / "assets"

# The flowchart "F" inside icon.webp (1254 x 1254), and the icon's rounded square.
F_BOX = (418, 432, 752, 898)
SQUARE_BOX = (90, 109, 1156, 1157)
TOP, BOTTOM = (2, 36, 112), (3, 15, 79)  # background gradient of the icon


def rounded_square(size: int) -> Image.Image:
    """Dark rounded square with the icon's vertical gradient, transparent corners."""
    grad = Image.new("RGBA", (size, size))
    draw = ImageDraw.Draw(grad)
    for y in range(size):
        t = y / max(size - 1, 1)
        draw.line([(0, y), (size, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)) + (255,))
    mask = Image.new("L", (size * 4, size * 4), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, size * 4 - 1, size * 4 - 1], radius=size * 4 * 22 // 100, fill=255)
    grad.putalpha(mask.resize((size, size), Image.LANCZOS))
    return grad


def small_icon(icon: Image.Image, size: int) -> Image.Image:
    """The "F" mark alone, large and centred, for 16-48 px."""
    base = rounded_square(size)
    f = icon.crop(F_BOX)
    h = round(size * 0.80)
    w = round(f.width * h / f.height)
    f = f.resize((w, h), Image.LANCZOS)
    base.alpha_composite(f, ((size - w) // 2, (size - h) // 2))
    return base


def large_icon(icon: Image.Image, size: int) -> Image.Image:
    sq = icon.crop(SQUARE_BOX)
    side = max(sq.size)
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    canvas.alpha_composite(sq, ((side - sq.width) // 2, (side - sq.height) // 2))
    return canvas.resize((size, size), Image.LANCZOS)


def white_text(img: Image.Image) -> Image.Image:
    """Turn the near-black letters white; the coloured parts stay as they are."""
    out = img.copy()
    px = out.load()
    for y in range(out.height):
        for x in range(out.width):
            r, g, b, a = px[x, y]
            if a and max(r, g, b) < 90 and max(r, g, b) - min(r, g, b) < 60:
                px[x, y] = (245, 248, 255, a)
    return out


def main() -> None:
    icon = Image.open(SRC / "icon.webp").convert("RGBA")
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    images = [small_icon(icon, s) if s <= 48 else large_icon(icon, s) for s in sizes]
    images[-1].save(OUT / "autoflowmate.ico", format="ICO", sizes=[(s, s) for s in sizes], append_images=images[:-1])
    images[-1].save(OUT / "autoflowmate.png", optimize=True)

    banner = Image.open(SRC / "banner.webp").convert("RGB")
    banner.resize((960, round(banner.height * 960 / banner.width)), Image.LANCZOS).save(OUT / "splash.png", optimize=True)

    # GitHub social preview: 2:1, cropped from the middle of the banner.
    w = banner.width
    h = w // 2
    top = (banner.height - h) // 2
    banner.crop((0, top, w, top + h)).resize((1280, 640), Image.LANCZOS).save(OUT / "social-preview.png", optimize=True)

    word = Image.open(SRC / "wordmark.webp").convert("RGBA")
    word = word.crop(word.getchannel("A").point(lambda a: 255 if a > 10 else 0).getbbox())
    word = word.resize((1000, round(word.height * 1000 / word.width)), Image.LANCZOS)
    word.save(OUT / "wordmark.png", optimize=True)
    white_text(word).save(OUT / "wordmark-dark.png", optimize=True)
    for name in ["autoflowmate.ico", "autoflowmate.png", "splash.png", "wordmark.png", "wordmark-dark.png", "social-preview.png"]:
        print(f"{name:20} {(OUT / name).stat().st_size // 1024} KB")


if __name__ == "__main__":
    main()
