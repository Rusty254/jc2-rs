#!/usr/bin/env python3
"""Quick software render of an OBJ (flat shaded, z-buffered) to PNG for eyeballing geometry."""
import sys, numpy as np
from PIL import Image

def load_obj(path):
    v, f, n = [], [], []
    for line in open(path):
        if line.startswith("v "):
            v.append([float(x) for x in line.split()[1:4]])
        elif line.startswith("f "):
            idx = [int(p.split("/")[0]) - 1 for p in line.split()[1:4]]
            f.append(idx)
    return np.array(v, dtype=np.float64), np.array(f, dtype=np.int64)

def render(v, f, yaw, pitch, size=700, light=(0.4, 0.8, 0.5)):
    c = (v.min(0) + v.max(0)) / 2
    r = np.linalg.norm(v.max(0) - v.min(0)) / 2
    p = (v - c) / r
    cy, sy = np.cos(yaw), np.sin(yaw)
    cp, sp = np.cos(pitch), np.sin(pitch)
    Ry = np.array([[cy, 0, sy], [0, 1, 0], [-sy, 0, cy]])
    Rx = np.array([[1, 0, 0], [0, cp, -sp], [0, sp, cp]])
    q = p @ Ry.T @ Rx.T
    img = np.zeros((size, size, 3), dtype=np.float32) + 0.12
    zbuf = np.full((size, size), -1e9)
    tri = q[f]  # (n,3,3)
    e1 = tri[:, 1] - tri[:, 0]; e2 = tri[:, 2] - tri[:, 0]
    nrm = np.cross(e1, e2)
    ln = np.linalg.norm(nrm, axis=1, keepdims=True); ln[ln == 0] = 1
    nrm = nrm / ln
    L = np.array(light) / np.linalg.norm(light)
    shade = np.abs(nrm @ L) * 0.8 + 0.2
    # screen coords: x right, y up -> image row down
    sx = (tri[:, :, 0] * 0.45 + 0.5) * size
    sy_ = (0.5 - tri[:, :, 1] * 0.45) * size
    sz = tri[:, :, 2]
    order = np.argsort(sz.mean(1))  # painter's order (back to front)
    for i in order:
        xs, ys, zs = sx[i], sy_[i], sz[i]
        x0, x1 = int(max(0, xs.min())), int(min(size - 1, xs.max()))
        y0, y1 = int(max(0, ys.min())), int(min(size - 1, ys.max()))
        if x1 < x0 or y1 < y0:
            continue
        xx, yy = np.meshgrid(np.arange(x0, x1 + 1), np.arange(y0, y1 + 1))
        # barycentric
        d = (ys[1] - ys[2]) * (xs[0] - xs[2]) + (xs[2] - xs[1]) * (ys[0] - ys[2])
        if abs(d) < 1e-9:
            continue
        a = ((ys[1] - ys[2]) * (xx - xs[2]) + (xs[2] - xs[1]) * (yy - ys[2])) / d
        b = ((ys[2] - ys[0]) * (xx - xs[2]) + (xs[0] - xs[2]) * (yy - ys[2])) / d
        g = 1 - a - b
        inside = (a >= 0) & (b >= 0) & (g >= 0)
        if not inside.any():
            continue
        z = a * zs[0] + b * zs[1] + g * zs[2]
        sub = zbuf[y0:y1 + 1, x0:x1 + 1]
        upd = inside & (z > sub)
        sub[upd] = z[upd]
        col = np.array([0.75, 0.78, 0.85]) * shade[i]
        img[y0:y1 + 1, x0:x1 + 1][upd] = col
    return (np.clip(img, 0, 1) * 255).astype(np.uint8)

if __name__ == "__main__":
    src, out = sys.argv[1], sys.argv[2]
    v, f = load_obj(src)
    views = [("side", 0.0, 0.0), ("front-3/4", 0.8, 0.35), ("top", 0.0, 1.45)]
    tiles = [render(v, f, yaw, pitch) for _, yaw, pitch in views]
    strip = np.concatenate(tiles, axis=1)
    Image.fromarray(strip).save(out)
    print(f"{src}: {len(v)} verts {len(f)} tris -> {out}")
