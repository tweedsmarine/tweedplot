"""Make the LERC and LERC GeoTIFF test fixtures in crates/chart/testdata (needs numpy,
imagecodecs and tifffile from PyPI). Run from the repository root."""
import numpy as np, imagecodecs, tifffile

out = "crates/chart/testdata"
H, W = 48, 64
y, x = np.mgrid[0:H, 0:W].astype(np.float32)
z = (-12.0 + 0.3 * x - 0.15 * y + 2.0 * np.sin(x * np.float32(0.21)) * np.cos(y * np.float32(0.17))).astype(np.float32)
valid = ~(((x - 40) ** 2 + (y - 20) ** 2 < 36) | (x < 3))
for v in [2, 3, 4]:
    open(f"{out}/lerc_v{v}.bin", "wb").write(imagecodecs.lerc_encode(z, level=0.01, version=v))
    open(f"{out}/lerc_v{v}_mask.bin", "wb").write(imagecodecs.lerc_encode(z, level=0.01, version=v, masks=valid))
open(f"{out}/lerc_v4_lossless.bin", "wb").write(imagecodecs.lerc_encode(z, level=0, version=4))

# A LINZ-like coastal DEM tile, 256 x 256 at 1 m in NZTM2000 from (1741600, 5672400),
# tiled 128 x 128, LERC (max error 0.001), nodata -9999 on land (z > 1), with two overviews.
N = 256
y, x = np.mgrid[0:N, 0:N].astype(np.float32)
dem = (-20.0 + 0.12 * x + 0.03 * y + 1.5 * np.sin(x / 9.0) * np.cos(y / 13.0)).astype(np.float32)
dem[dem > 1.0] = -9999.0
geo = [
    (33550, 12, 3, (1.0, 1.0, 0.0), False),                                 # ModelPixelScale
    (33922, 12, 6, (0.0, 0.0, 0.0, 1741600.0, 5672400.0, 0.0), False),      # ModelTiepoint
    (42113, 2, 0, "-9999", False),                                           # GDAL_NODATA
]
with tifffile.TiffWriter(f"{out}/coastal_dem.tiff") as t:
    t.write(dem, tile=(128, 128), compression="lerc", compressionargs={"level": 0.001, "version": 4}, extratags=geo)
    for k in (2, 4):
        ov = dem.reshape(N // k, k, N // k, k)[:, 0, :, 0].copy()
        t.write(ov, tile=(128, 128) if N // k >= 128 else (64, 64), compression="lerc",
                compressionargs={"level": 0.001, "version": 4}, subfiletype=1)
np.save("/tmp/coastal_dem.npy", dem)
print("ok")
