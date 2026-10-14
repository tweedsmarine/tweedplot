"""Prepare land polygons for the racenav app.

world: geo-maps earth-lands-1km, simplified, coordinates rounded to 3 dp.
nz:    geo-maps earth-lands-100m clipped to the NZ region, rounded to 5 dp.
Source data: OpenStreetMap contributors (ODbL), via simonepri/geo-maps 0.6.0.
"""
import json, sys
from shapely.geometry import shape, mapping, box
import numpy as np
import shapely

src1k, src100, out = sys.argv[1], sys.argv[2], sys.argv[3]

def load(p):
    g = json.load(open(p))
    feats = g["features"] if g.get("type") == "FeatureCollection" else [g]
    return [shape(f["geometry"] if "geometry" in f else f) for f in feats]

def write(geoms, path, grid):  # grid = decimal places
    feats = []
    for g in geoms:
        if g.is_empty:
            continue
        g = shapely.transform(g, lambda c: np.round(c, grid))
        feats.append({"type": "Feature", "properties": {}, "geometry": mapping(g)})
    json.dump({"type": "FeatureCollection", "features": feats}, open(path, "w"), separators=(",", ":"))

import os
if not os.path.exists(f"{out}/land-world.geojson"):
  world = [g.simplify(0.01, preserve_topology=True) for g in load(src1k)]
  write(world, f"{out}/land-world.geojson", 3)

nzbox = box(165.0, -48.5, 179.99, -33.5)
nz = []
for g in load(src100):
    parts = getattr(g, "geoms", [g])
    for part in parts:
        if not part.envelope.intersects(nzbox):
            continue
        if nzbox.contains(part.envelope):
            nz.append(part)
            continue
        if not part.is_valid:
            part = shapely.make_valid(part)
        nz.append(part.intersection(nzbox))
write(nz, f"{out}/land-nz.geojson", 5)
