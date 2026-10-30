"""Builds every logo asset from one source image, so the logo can be changed or re-exported reproducibly.

    pip install pillow numpy
    python scripts/brand/make-brand-assets.py

Input:  assets/source/logo-source.png
        The original artwork, cropped to the mark, the wordmark and the tagline on its flat navy background
        (the sparkle watermark of the original export sits far outside this crop).

Output (all derived, safe to regenerate):
  assets/logo-mark.png            the mark alone, transparent, 1024 px square
  assets/logo-lockup-dark.png     mark + "lunaRun", for dark backgrounds (white wordmark)
  assets/logo-lockup-light.png    the same for light backgrounds (dark wordmark)
  assets/app-icon-1024.png        the mark on a navy rounded square: the input for `pnpm tauri icon`
  assets/social-preview.png       1280 x 640 image for GitHub's "Social preview" setting
  src/assets/app-icon.png         128 px app icon shown in the sidebar
  public/favicon.png              64 px favicon for the web build

The background is removed with a "color to alpha" step: the flat navy is turned into transparency and the
colour of every edge pixel is recovered, so there are no dark fringes on light backgrounds.
"""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "assets" / "source" / "logo-source.png"

# Flat background colour of the source artwork, and the colour used for the tile behind the app icon.
NAVY = np.array([33.0, 34.0, 53.0])
# Wordmark colour on light backgrounds: the logo's navy, darkened for contrast on white.
WORDMARK_ON_LIGHT = (30, 32, 56)


def smoothstep(x: np.ndarray, lo: float, hi: float) -> np.ndarray:
    t = np.clip((x - lo) / (hi - lo), 0.0, 1.0)
    return t * t * (3 - 2 * t)


def color_to_alpha(rgb: np.ndarray, background: np.ndarray, lo: float = 0.10, hi: float = 0.26) -> np.ndarray:
    """Remove a known background colour from an RGB image and return RGBA.

    Only pixels brighter than the background count, so the dark keylines around the artwork's shapes
    vanish instead of turning into black edges. Alpha is the smallest value that reproduces the pixel
    over the background, passed through a smoothstep so the background's low-level noise (below `lo`) is
    fully transparent while real edges stay anti-aliased. Colours are recovered at full opacity.
    """
    c = rgb.astype(np.float64)
    raw = np.clip((c - background) / (255.0 - background), 0.0, 1.0).max(axis=2)
    alpha = smoothstep(raw, lo, hi)
    safe = np.where(raw > 1e-6, raw, 1.0)[..., None]
    fg = np.clip(background + (c - background) / safe, 0, 255)
    return np.dstack([fg, alpha * 255.0]).round().astype(np.uint8)


def content_bands(rgba: np.ndarray, min_alpha: int = 90) -> list[tuple[int, int]]:
    """Vertical bands (y0, y1) that contain artwork, found from the alpha channel."""
    rows = (rgba[..., 3] > min_alpha).any(axis=1)
    bands, start = [], None
    for y, filled in enumerate(rows):
        if filled and start is None:
            start = y
        if not filled and start is not None:
            bands.append((start, y - 1))
            start = None
    if start is not None:
        bands.append((start, len(rows) - 1))
    return bands


def bbox(rgba: np.ndarray, y0: int, y1: int, min_alpha: int = 90) -> tuple[int, int, int, int]:
    """Tight (left, top, right, bottom) box of the artwork between two rows."""
    band = rgba[y0 : y1 + 1, :, 3] > min_alpha
    xs = np.where(band.any(axis=0))[0]
    return int(xs.min()), y0, int(xs.max()) + 1, y1 + 1


def pad_to_square(image: Image.Image, margin: float) -> Image.Image:
    """Center `image` on a transparent square canvas that leaves `margin` (fraction) free on every side."""
    side = int(round(max(image.size) / (1 - 2 * margin)))
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    canvas.alpha_composite(image, ((side - image.width) // 2, (side - image.height) // 2))
    return canvas


def rounded_tile(size: int, radius_fraction: float, color: tuple[int, int, int]) -> Image.Image:
    """A filled rounded square with smooth (supersampled) corners and a transparent outside."""
    scale = 4
    mask = Image.new("L", (size * scale, size * scale), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (0, 0, size * scale - 1, size * scale - 1), radius=int(size * scale * radius_fraction), fill=255
    )
    mask = mask.resize((size, size), Image.LANCZOS)
    tile = Image.new("RGBA", (size, size), (*color, 255))
    tile.putalpha(mask)
    return tile


def save(image: Image.Image, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    image.save(path, optimize=True)
    print(f"  {path.relative_to(ROOT)}  {image.width}x{image.height}  {path.stat().st_size / 1024:.0f} KB")


def main() -> int:
    if not SOURCE.exists():
        print(f"missing {SOURCE}", file=sys.stderr)
        return 1

    rgb = np.asarray(Image.open(SOURCE).convert("RGB"))
    rgba = color_to_alpha(rgb, NAVY)

    bands = content_bands(rgba)
    if len(bands) != 3:
        print(f"expected 3 content bands (mark, wordmark, tagline), found {len(bands)}: {bands}", file=sys.stderr)
        return 1
    (m0, m1), (w0, w1), (t0, t1) = bands

    # --- the mark alone ------------------------------------------------------------------------
    mark = Image.fromarray(rgba[m0 : m1 + 1], "RGBA")
    mark = mark.crop(mark.getbbox())
    mark_square = pad_to_square(mark, 0.04).resize((1024, 1024), Image.LANCZOS)
    save(mark_square, ROOT / "assets" / "logo-mark.png")

    # --- mark + wordmark (no tagline) for README headers ----------------------------------------
    left, top, right, _ = bbox(rgba, m0, w1)
    lockup = rgba[top : w1 + 1].copy()
    pad = 24
    lockup = np.pad(lockup, ((pad, pad), (pad, pad), (0, 0)))  # transparent margin
    # The wordmark is white in the source. Make the light-mode variant's wordmark dark.
    light = lockup.copy()
    word_rows = slice(w0 - top + pad, w1 - top + pad + 1)
    light[word_rows, :, :3] = WORDMARK_ON_LIGHT
    for name, array in (("dark", lockup), ("light", light)):
        image = Image.fromarray(array, "RGBA")
        image = image.crop(image.getbbox())
        width = 900
        image = image.resize((width, round(image.height * width / image.width)), Image.LANCZOS)
        save(image, ROOT / "assets" / f"logo-lockup-{name}.png")

    # --- app icon: the mark on a navy rounded square --------------------------------------------
    size = 1024
    tile = rounded_tile(size, 0.225, tuple(int(v) for v in NAVY))
    inner = mark.copy()
    target_width = int(size * 0.74)
    inner = inner.resize((target_width, round(inner.height * target_width / inner.width)), Image.LANCZOS)
    tile.alpha_composite(inner, ((size - inner.width) // 2, (size - inner.height) // 2))
    save(tile, ROOT / "assets" / "app-icon-1024.png")
    save(tile.resize((128, 128), Image.LANCZOS), ROOT / "src" / "assets" / "app-icon.png")
    save(tile.resize((64, 64), Image.LANCZOS), ROOT / "public" / "favicon.png")

    # --- GitHub social preview (1280 x 640) -----------------------------------------------------
    canvas = Image.new("RGBA", (1280, 640), (*(int(v) for v in NAVY), 255))
    full = Image.fromarray(rgba, "RGBA")
    full = full.crop(full.getbbox())
    height = 500
    full = full.resize((round(full.width * height / full.height), height), Image.LANCZOS)
    canvas.alpha_composite(full, ((1280 - full.width) // 2, (640 - full.height) // 2))
    save(canvas.convert("RGB"), ROOT / "assets" / "social-preview.png")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
