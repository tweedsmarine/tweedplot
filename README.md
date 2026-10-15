# racenav

A modern race navigation app for Windows: chartplotter basics, a GRIB viewer for many runs at once, an in-house routing engine and race tools. Live data comes in through Signal K.

Spec and roadmap: https://claude.ai/code/artifact/e37fa4c3-20c2-4aec-af8f-641714f39c27

## Layout

| Path | What it is | Status |
| --- | --- | --- |
| `crates/nav-core` | Positions, bearings, distances, angle helpers (NM, kn, deg true) | Working, tested |
| `crates/polar` | Read qtVlm `.csv` / `.pol` polars, interpolate, best VMG | Working, tested |
| `crates/grib` | GRIB2 decoding (simple and complex packing, bitmaps, Meltemus JSON section), wind/gust/MSLP/waves/currents in space and time | Working, matches ecCodes on Meltemus files; GRIB1 next |
| `crates/routing` | Isochrone engine: land and safety-contour avoidance | Next |
| `crates/chart` | LINZ GeoPackages (Topo50 coastline, depth areas, contours, soundings, all scale bands) served as vector tiles | Working, tested on the full LINZ download |
| `crates/signalk` | Signal K WebSocket client, own-vessel state, session log | Working, tested against Signal K playing a Morgane log |
| `app/` | Tauri shell + MapLibre GL front end: offline land, own boat, track, COG vector, instrument panel | First window |

## Build and test

```
cargo test
cargo run -p polar --example polar_info -- "path\to\Morgane_median.csv"
```

See `SETUP-WINDOWS.md` for how to install the toolchain.

## Run the app

Start Signal K first (live GPS or a File Stream replay), then:

```
cd app
npm install          # first time only
npm run tauri dev    # first build takes several minutes; later ones are quick
```

The app connects to `ws://localhost:3000/signalk/v1/stream?subscribe=self`. To use another server, set `RACENAV_SK_URL` before starting. Every Signal K message is logged to `%APPDATA%\nz.tweedsmarine.racenav\logs\sk-<date>-<time>Z.log`.

## Chart data

The app reads LINZ GeoPackage downloads (zipped as downloaded is fine) from `RACENAV_CHART_DIR`, else `C:\dev\racenav-data\linz`, else `%APPDATA%\nz.tweedsmarine.racenav\charts`. On the first run it unzips them into a `.unzipped` folder beside the zips (about 600 MB for all bands); after that, start-up takes a couple of seconds. Layers used:

- NZ Coastlines and Islands Polygons (Topo 1:50k)
- Depth area polygons, depth contour polylines and sounding points (Hydro), every scale band

Depth shading follows the S-52 day colours, with the safety depth and safety contour set in the panel. LINZ data is CC BY 4.0 and **not for navigation**.

Below zoom 6 (and anywhere without LINZ data) the app falls back to OpenStreetMap land polygons via geo-maps 0.6.0 (`app/public/data`, built by `tools/prep_land*.py`).

To check a chart folder or test tiles in a browser:

```
cargo run --release -p chart --example chart_info -- C:\dev\racenav-data\linz
cargo run --release -p chart --example tileserve -- C:\dev\racenav-data\linz 7070
```

## Weather (GRIB)

The Weather panel lists GRIB files found in `RACENAV_GRIB_DIR`, `C:\dev\racenav-data\grib` and qtVlm's `C:\Program Files (x86)\qtVlm\grib`. Load as many as you like. One is shown on the chart (wind barbs plus speed shading under the land), and the table compares every loaded run at the cursor for the slider time. Outside a file's time span there is no value: a short GRIB never shows as calm.

```
cargo run -p grib --example grib_info -- path\to\file.grb2 -36.0 174.8
```

## Conventions

- WGS84 decimal degrees, south and west negative.
- Bearings in degrees true, distances in NM, speeds in knots, times in UTC.
- en-GB spelling in docs and UI text.
