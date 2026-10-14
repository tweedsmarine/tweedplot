# racenav

A modern race navigation app for Windows: chartplotter basics, a GRIB viewer for many runs at once, an in-house routing engine and race tools. Live data comes in through Signal K.

Spec and roadmap: https://claude.ai/code/artifact/e37fa4c3-20c2-4aec-af8f-641714f39c27

## Layout

| Path | What it is | Status |
| --- | --- | --- |
| `crates/nav-core` | Positions, bearings, distances, angle helpers (NM, kn, deg true) | Working, tested |
| `crates/polar` | Read qtVlm `.csv` / `.pol` polars, interpolate, best VMG | Working, tested |
| `crates/grib` | GRIB1/2 decoding, many runs at once, Meltemus JSON section | Next |
| `crates/routing` | Isochrone engine: land and safety-contour avoidance | Next |
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

Land data for now: OpenStreetMap land polygons via geo-maps 0.6.0 (`app/public/data`, built by `tools/prep_land*.py`). About 300 m detail around NZ, so not good enough for Nelson Haven; the LINZ Topo50 coastline replaces it next.

## Conventions

- WGS84 decimal degrees, south and west negative.
- Bearings in degrees true, distances in NM, speeds in knots, times in UTC.
- en-GB spelling in docs and UI text.
