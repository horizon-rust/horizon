#!/usr/bin/env python3
"""Checks the VR title's backdrop in a fake-runtime run (tools/vr_fake_scripts/title_backdrop.txt):
that it stays fixed in the stage as the head turns, and that the two eyes see it with parallax
(the floor in front shifts between them more than the sky far away).

  python3 tools/vr_fake_run.py --resources ... --title --world-frames 1300 \
      --script tools/vr_fake_scripts/title_backdrop.txt --capture-seconds 5,8,11 --out vr-run
  python3 tools/vr_backdrop_check.py vr-run

Fixed in the stage: the left eye turned 20 degrees (t8) is warped back to the head's first
direction (t5) through the fake headset's field of view, and the two images must agree; the
same warp without the turn (a backdrop carried by the head) must not. Parallax: the horizontal
shift between the eyes, found by matching strips of the image, near the bottom of the view
looking down (t11, the floor a few metres away) against the top (the sphere 30 m away).
Needs numpy and Pillow.
"""

import argparse
import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image

# crates/horizon-xr-fake: the left eye's field of view (radians), the right eye mirrors it.
LEFT_FOV = (-0.96, 0.78, 0.85, -0.90)  # left, right, up, down
IPD = 0.063


def load(path: Path) -> np.ndarray:
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.float32) / 255.0


def fov(eye: int):
    l, r, u, d = LEFT_FOV
    return (-r, -l, u, d) if eye else (l, r, u, d)


def rays(shape, eye: int) -> np.ndarray:
    """Each pixel's direction in the eye's space (looking down -z, y up)."""
    h, w = shape[:2]
    l, r, u, d = (math.tan(a) for a in fov(eye))
    x = l + (np.arange(w) + 0.5) / w * (r - l)
    y = u - (np.arange(h) + 0.5) / h * (u - d)
    xs, ys = np.meshgrid(x, y)
    return np.stack([xs, ys, -np.ones_like(xs)], axis=-1)


def project(dirs: np.ndarray, shape, eye: int):
    """Pixel coordinates of directions in the eye's space (NaN behind or outside)."""
    h, w = shape[:2]
    l, r, u, d = (math.tan(a) for a in fov(eye))
    z = -dirs[..., 2]
    with np.errstate(divide="ignore", invalid="ignore"):
        tx, ty = dirs[..., 0] / z, dirs[..., 1] / z
    px = (tx - l) / (r - l) * w - 0.5
    py = (u - ty) / (u - d) * h - 0.5
    bad = (z <= 0) | (px < 0) | (px > w - 1) | (py < 0) | (py > h - 1)
    px[bad] = np.nan
    py[bad] = np.nan
    return px, py


def yaw(deg: float) -> np.ndarray:
    """A turn to the left by `deg` (about +y), as the fake headset's yaw."""
    a = math.radians(deg)
    return np.array([[math.cos(a), 0, math.sin(a)], [0, 1, 0], [-math.sin(a), 0, math.cos(a)]])


def warp_error(ahead: np.ndarray, turned: np.ndarray, turn: float) -> float:
    """Mean colour difference between `ahead` and `turned` sampled where `ahead`'s rays land
    in an eye turned by `turn` degrees (nearest pixel; the eyes' own small move is ignored)."""
    dirs = rays(ahead.shape, 0) @ yaw(turn)  # the head's rotation inverse, row vectors
    px, py = project(dirs, ahead.shape, 0)
    ok = ~np.isnan(px)
    if ok.mean() < 0.3:
        return float("nan")
    xi = np.rint(px[ok]).astype(int)
    yi = np.rint(py[ok]).astype(int)
    return float(np.abs(ahead[ok] - turned[yi, xi]).mean())


def shift(left: np.ndarray, right: np.ndarray, rows: slice) -> float:
    """How far right (pixels) a strip shows in the left eye than in the right one, once the
    eyes' different fields of view are taken out (the right eye resampled onto the left's
    frustum): 0 for what is far away, more the nearer it is."""
    h, w = left.shape[:2]
    dirs = rays(left.shape, 0)
    px, py = project(dirs, left.shape, 1)
    ok = ~np.isnan(px)
    right_on_left = np.zeros_like(left)
    right_on_left[ok] = right[np.rint(py[ok]).astype(int), np.rint(px[ok]).astype(int)]
    a = left[rows].mean(axis=2)
    b = right_on_left[rows].mean(axis=2)
    valid = ok[rows].all(axis=0)
    cols = np.where(valid)[0]
    lo, hi = cols.min() + 60, cols.max() - 60
    best, best_err = 0, float("inf")
    for s in range(-50, 51):
        err = np.abs(a[:, lo:hi] - b[:, lo + s:hi + s]).mean()
        if err < best_err:
            best, best_err = s, err
    return float(-best)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run", help="the --out folder of tools/vr_fake_run.py")
    args = ap.parse_args()
    run = Path(args.run)
    ahead, turned = load(run / "t5-layer0-eye0.png"), load(run / "t8-layer0-eye0.png")
    fixed = warp_error(ahead, turned, 20.0)
    carried = warp_error(ahead, turned, 0.0)
    print(f"turned 20 degrees: warped back {fixed:.4f}, as if carried by the head {carried:.4f}")
    down_l, down_r = load(run / "t11-layer0-eye0.png"), load(run / "t11-layer0-eye1.png")
    h = down_l.shape[0]
    near = shift(down_l, down_r, slice(int(h * 0.85), int(h * 0.95)))
    up_l, up_r = load(run / "t5-layer0-eye0.png"), load(run / "t5-layer0-eye1.png")
    far = shift(up_l, up_r, slice(int(h * 0.05), int(h * 0.15)))
    print(f"shift between the eyes: floor in front {near:.0f} px, sky ahead {far:.0f} px")
    ok = fixed < 0.5 * carried and near > far + 3
    print("ok" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
