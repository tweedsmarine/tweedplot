"""Makes the small test images for the imagery crate (needs rasterio and numpy):
    python make_fixtures.py
Writes the images and expected.json (values read back with GDAL) beside this script. The
GeoTIFFs are kept as base64 text (name.tif.b64; the tests decode them): some file copying
tools rewrite TIFF files' metadata, which would change the tests' input."""
import json, os
import numpy as np
import rasterio
from rasterio.transform import from_origin
from rasterio.enums import Resampling

here = os.path.dirname(os.path.abspath(__file__))
W, H = 600, 400
yy, xx = np.mgrid[0:H, 0:W]
rgb = np.stack([(xx * 255 // (W - 1)), (yy * 255 // (H - 1)), ((xx + yy) % 256)]).astype(np.uint8)
rgb[:, 0:20, 0:30] = 0  # a no-data corner
# UTM 59S near Nelson, 10 m pixels, like a Sentinel-2 true-colour image
utm = from_origin(680000.0, 5440000.0, 10.0, 10.0)
expected = {}

def cog(name, **kw):
    path = os.path.join(here, name)
    prof = dict(driver="GTiff", width=W, height=H, count=3, dtype="uint8", crs="EPSG:32759", transform=utm,
                tiled=True, blockxsize=128, blockysize=128, nodata=0, **kw)
    with rasterio.open(path, "w", **prof) as d:
        d.write(rgb)
        d.build_overviews([2, 4], Resampling.average)
    with rasterio.open(path) as d:
        a = d.read()
        expected[name] = {
            "size": [d.width, d.height], "overviews": d.overviews(1),
            "pixels": {f"{c},{r}": [int(v) for v in a[:, r, c]] for c, r in [(0, 0), (35, 25), (599, 399), (300, 200), (129, 130)]},
        }

cog("utm_deflate.tif", compress="deflate", predictor=2)
cog("utm_jpeg_ycbcr.tif", compress="jpeg", photometric="ycbcr", jpeg_quality=95)
cog("utm_jpeg_rgb.tif", compress="jpeg", jpeg_quality=95)

# palette image in lon/lat, LZW, in strips
pal = ((xx // 50 + yy // 50) % 4).astype(np.uint8)
path = os.path.join(here, "ll_palette_lzw.tif")
with rasterio.open(path, "w", driver="GTiff", width=W, height=H, count=1, dtype="uint8", crs="EPSG:4326",
                   transform=from_origin(173.0, -41.0, 0.001, 0.001), compress="lzw", photometric="palette") as d:
    d.write(pal, 1)
    d.write_colormap(1, {0: (255, 0, 0, 255), 1: (0, 255, 0, 255), 2: (0, 0, 255, 255), 3: (250, 250, 250, 255)})
expected["ll_palette_lzw.tif"] = {"pixels": {"0,0": [255, 0, 0], "60,0": [0, 255, 0], "60,60": [0, 0, 255], "160,10": [250, 250, 250]}}

# where some points fall (pixel column and row, from rasterio's transform and pyproj)
from pyproj import Transformer
t = Transformer.from_crs(4326, 32759, always_xy=True)
pts = []
for lon, lat in [(173.18, -41.19), (173.15, -41.2), (173.21, -41.175)]:
    x, y = t.transform(lon, lat)
    c, r = ~utm * (x, y)
    pts.append({"lon": lon, "lat": lat, "col": c, "row": r})
expected["utm_points"] = pts

# a KAP chart (BSB 3) with a mercator grid, read back by GDAL's BSB driver
def merc_y(lat):
    import math
    return math.log(math.tan(math.pi / 4 + math.radians(lat) / 2))
KW, KH = 300, 200
lon0, lon1, lat0, lat1 = 173.0, 173.3, -41.1, -41.3
def px(lon, lat):
    x = (lon - lon0) / (lon1 - lon0) * KW
    y = (merc_y(lat0) - merc_y(lat)) / (merc_y(lat0) - merc_y(lat1)) * KH
    return x, y
idx = (1 + ((xx[:KH, :KW] // 20 + yy[:KH, :KW] // 25) % 5)).astype(np.uint8)
idx[150:, 250:] = 6  # a long run of one colour
pal = {1: (10, 20, 30), 2: (200, 210, 220), 3: (255, 255, 255), 4: (90, 160, 90), 5: (180, 40, 40), 6: (0, 0, 200)}
refs = [(0, 0, lat0, lon0), (KW, 0, lat0, lon1), (KW, KH, lat1, lon1), (0, KH, lat1, lon0), (KW // 2, KH // 2, None, None)]
lines = ["! made by make_fixtures.py", "VER/3.0", f"BSB/NA=Test Harbour,NU=TEST1,RA={KW},{KH},DU=254",
         "KNP/SC=25000,GD=WGS84,PR=MERCATOR,PP=-41.2,PI=UNKNOWN,SP=UNKNOWN,SK=0.0",
         "    TA=90.0,UN=METRES,SD=LOWEST ASTRONOMICAL TIDE,DX=10.0,DY=10.0",
         "CED/SE=1,RE=1,ED=05/11/2026", "DTM/0.0,0.0"]
for i, (c, r, la, lo) in enumerate(refs):
    if la is None:
        lo = (lon0 + lon1) / 2
        import math
        my = (merc_y(lat0) + merc_y(lat1)) / 2
        la = math.degrees(2 * math.atan(math.exp(my)) - math.pi / 2)
    lines.append(f"REF/{i+1},{c},{r},{la:.8f},{lo:.8f}")
for i, (la, lo) in enumerate([(lat0, lon0), (lat0, 173.25), (lat1, 173.25), (lat1, lon0)]):
    lines.append(f"PLY/{i+1},{la:.6f},{lo:.6f}")
for k, (r, g, b) in pal.items():
    lines.append(f"RGB/{k},{r},{g},{b}")
header = ("\r\n".join(lines) + "\r\n").encode() + b"\x1a\x00"
depth = 3
body = bytearray([depth])
offsets = []
def vlq(n):
    out = [n & 0x7f]
    n >>= 7
    while n:
        out.append(0x80 | (n & 0x7f)); n >>= 7
    return bytes(reversed(out))
for r in range(KH):
    offsets.append(len(header) + len(body))
    body += vlq(r + 1)
    row = idx[r]
    x = 0
    while x < KW:
        v = int(row[x]); n = 1
        while x + n < KW and row[x + n] == v: n += 1
        run = n - 1
        shift = 7 - depth
        cmask = (1 << shift) - 1
        # first byte: colour in the high bits, the run's top bits in the low ones
        extra = []
        rest = run
        low = []
        while rest > cmask:
            low.append(rest & 0x7f); rest >>= 7
        first = (v << shift) | rest
        bs = [first] + list(reversed(low))
        for i in range(len(bs) - 1):
            bs[i] |= 0x80
        body += bytes(bs)
        x += n
    body.append(0)
table = len(header) + len(body)
for o in offsets:
    body += o.to_bytes(4, "big")
body += table.to_bytes(4, "big")
with open(os.path.join(here, "test.kap"), "wb") as f:
    f.write(header + bytes(body))
with rasterio.open(os.path.join(here, "test.kap")) as d:
    a = d.read(1)
    cm = d.colormap(1)
    assert d.width == KW and d.height == KH, (d.width, d.height)
    got = [[int(v) for v in cm[int(a[r, c])][:3]] for c, r in [(0, 0), (25, 30), (299, 199), (260, 160)]]
expected["test.kap"] = {"gdal_pixels": got, "indices_match": bool((a == idx - 1).all()),  # GDAL counts the palette from 0,
                        "points": [{"lon": lo, "lat": la, "col": px(lo, la)[0], "row": px(lo, la)[1]} for lo, la in [(173.1, -41.15), (173.29, -41.29)]]}
import base64
for n in ["utm_deflate.tif", "utm_jpeg_ycbcr.tif", "utm_jpeg_rgb.tif", "ll_palette_lzw.tif"]:
    p = os.path.join(here, n)
    with open(p, "rb") as f, open(p + ".b64", "w") as o:
        o.write(base64.encodebytes(f.read()).decode())
    os.remove(p)
with open(os.path.join(here, "expected.json"), "w") as f:
    json.dump(expected, f, indent=1)
print(json.dumps({k: v for k, v in expected.items() if k != "utm_points"})[:1500])
