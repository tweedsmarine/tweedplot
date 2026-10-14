"""NZ land polygons from geo-maps earth-lands-100m (OpenStreetMap contributors, ODbL).
Keeps every polygon whose outer ring lies wholly inside the NZ box; rounds to 5 dp."""
import json, sys
src, dst = sys.argv[1], sys.argv[2]
W, E, S, N = 165.0, 179.999, -48.5, -33.5
g = json.load(open(src))
feats = g["features"] if g.get("type") == "FeatureCollection" else [g]
polys = []
def flat(geom):
    if geom["type"] == "GeometryCollection":
        for gg in geom["geometries"]:
            yield from flat(gg)
    elif geom["type"] == "MultiPolygon":
        yield from geom["coordinates"]
    elif geom["type"] == "Polygon":
        yield geom["coordinates"]
for f in feats:
    for poly in flat(f.get("geometry", f)):
        outer = poly[0]
        xs = [c[0] for c in outer]; ys = [c[1] for c in outer]
        if min(xs) >= W and max(xs) <= E and min(ys) >= S and max(ys) <= N:
            polys.append([[[round(x, 5), round(y, 5)] for x, y, *_ in ring] for ring in poly])
out = {"type": "FeatureCollection", "features": [{"type": "Feature", "properties": {}, "geometry": {"type": "MultiPolygon", "coordinates": polys}}]}
json.dump(out, open(dst, "w"), separators=(",", ":"))
print(len(polys), "polygons")
