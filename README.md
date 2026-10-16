# racenav

A modern race navigation app for Windows: chartplotter basics, a GRIB viewer for many runs at once, an in-house routing engine and race tools. Live data comes in through Signal K.

Spec and roadmap: https://claude.ai/code/artifact/e37fa4c3-20c2-4aec-af8f-641714f39c27

## Layout

| Path | What it is | Status |
| --- | --- | --- |
| `crates/nav-core` | Positions, bearings, distances, angle helpers (NM, kn, deg true) | Working, tested |
| `crates/polar` | Read qtVlm `.csv` / `.pol` polars, interpolate, best VMG | Working, tested |
| `crates/grib` | GRIB2 decoding (simple and complex packing, bitmaps, Meltemus JSON section), wind/gust/MSLP/waves/currents in space and time | Working, matches ecCodes on Meltemus files; GRIB1 next |
| `crates/routing` | Isochrone routing (modified isochrone method) round land and water shallower than the safety depth, multi-leg, tack/gybe penalties, currents | Working, tested |
| `crates/chart` | LINZ GeoPackages (Topo50 coastline, depth areas, contours, soundings, all scale bands) served as vector tiles | Working, tested on the full LINZ download |
| `crates/signalk` | Signal K WebSocket client, own-vessel state (position, COG/SOG, heading, BSP, depth, apparent and true wind, log), true wind worked out when the server has none, session log | Working, tested against Signal K playing a Morgane log |
| `crates/library` | Mark library, routes and saved routings as JSON files; GPX import and export | Working, tested |
| `app/` | Tauri shell + MapLibre GL front end: icon rail with one drawer at a time, chart, GRIB, routing, library, floating instrument panels | Working |

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

## World coastline (GSHHG)

The bundled world land is coarse (small islands missing). For a detailed one, download the GSHHG shapefiles, `gshhg-shp-2.3.7.zip` (about 150 MB, LGPL), from https://www.soest.hawaii.edu/pwessel/gshhg/ and put the zip (or the unpacked `GSHHS_shp` folder) in `C:\dev\racenav-data\coastline` (or `RACENAV_COAST_DIR`). On the next start the app builds `coastline\.unzipped\gshhg_land.gpkg` once (a minute or two), then draws land at five resolutions (crude at world scale to full, every island, from zoom 8) and routing avoids it. Inside NZ, LINZ Topo50 is used instead where it is loaded.

## Weather (GRIB)

**Download (free, no email)** in the Weather drawer: GFS 0.25°/0.5°/1° (wind, gusts, pressure, rain rate, CAPE) and GFS-Wave 0.25° (significant height, peak period and direction, swell) from NOAA NOMADS. Draw the area on the chart (or use the chart view or the course), choose the forecast length (1–16 days) and step (1, 3, 6 or 12 h), check the size estimate, Download. The newest complete run is found by checking the server, each forecast time is fetched cut to the area (one request at a time, politely), the steps are joined into one file in `racenav-data\grib`, and it is loaded straight away.

The Weather panel lists GRIB files found in `RACENAV_GRIB_DIR`, `C:\dev\racenav-data\grib` and qtVlm's `C:\Program Files (x86)\qtVlm\grib`. Load as many as you like. One is shown on the chart (wind barbs plus speed shading under the land), and the table compares every loaded run at the cursor for the slider time. Outside a file's time span there is no value: a short GRIB never shows as calm.

```
cargo run -p grib --example grib_info -- path\to\file.grb2 -36.0 174.8
```

## Routing

Course drawer: start and marks (see below). Routing drawer, Run tab: start time (weather slider time, now, or a set time), settings, **Compute** (the GRIB shown on the chart) or **With each GRIB** (one routing per loaded GRIB, then the compare table opens). Messages are listed newest first, one line each. Routings tab: every routing of this session plus saved ones, each with its own colour; tick to show on the chart, click to make it the one shown in detail (isochrones, points, summary, instrument targets), Save, each shown routing has a triangle in its colour where its boat is at the weather slider's time (pointing along its heading; faded once finished; hover for details, click to make it the active one), or open the **Compare table** in the dock at the bottom (it has its own GRIB chooser and time slider, so the Weather drawer can stay closed) (click a column to sort, ⚙ to choose columns, best values in green). Boat drawer: choose a polar (from `C:\dev\racenav-data\polars` or qtVlm's `polar` folder), efficiency and tack/gybe time. Route drawer: **Set start** (or **From boat**), **Add marks** by clicking the chart, or click a library mark to reuse it (Esc to stop), then **Compute routing**. The routing starts at the weather slider time with the GRIB shown on the chart. Hover a routing point for heading, TWA, TWS and boat speed; **Save routing** keeps it in the library, **GPX** writes it to the export folder.

How it works: each step fans out headings from every point of the last isochrone, boat speed from the polar at the GRIB wind (plus current if ticked), a tack or gybe costs its time, and steps that cross land (Topo50) or LINZ depth areas shallower than the safety depth are dropped. Obstacles come from a 50 m raster built from the charts for each run, with the finest chart scale winning. Pruning keeps one point per small grid cell (so routes find their way round headlands and into bays); without charts it uses the classic sectors from the start. If the GRIB runs out, the route stops and says so.

Not yet: rounding sides (marks are reached within the arrival radius), route optimisation, multi-routing.

```
cargo run --release -p routing --example route_cli -- --grib file.grb2 --polar boat.csv --chart C:\dev\racenav-data\linz --start -36.838,174.805 --marks "-35.465,174.80;-35.252,174.100"
```

## Marks, routes and routings

Everything is kept as plain JSON in `C:\dev\racenav-data\library` (or `RACENAV_LIBRARY_DIR`; `<app data>\library` when racenav-data doesn't exist):

```
marks.json            every mark, shared by all routes
routes\<id>.json      a route: name, start, ordered mark ids (rounding side kept for later)
routings\<id>.json    a saved routing: its inputs (start, marks, time, GRIB, polar, settings) and the result
export\*.gpx          GPX written by the app
```

Route drawer: name the route and **Save**: new marks go into the library (named WP 1, WP 2 … until renamed), moved library marks are updated. **Navigate** picks the mark being sailed to (it steps on by itself inside the arrival radius); the route instruments use it. Library drawer: Routes (open, GPX, delete), Marks (find, edit name/type/group/position/notes, add to the route, delete; positions can be typed as `36 50.12 S` or `-36.8353`), Routings (show, delete). **Import GPX** reads waypoints and routes (qtVlm, OpenCPN, Expedition …); a waypoint with the same name as a library mark and within about 50 m becomes that mark. Right-click the chart: **New library mark here**. The start can be a library mark too: in Set start mode click the mark, right-click it (Start route at …), or press **Start** beside it in the Marks list; the route keeps the link (`start_mark`).

## Fleet (YB race tracking)

Fleet drawer: type the race's YB name (as in `yb.tl/<race>`, e.g. `ggr2026`) and Open. The app fetches the race setup (`cf.yb.tl/JSON/<race>/RaceSetup`: boats, classes, course) and downloads the full tracks (`cf.yb.tl/BIN/<race>/AllPositions3`: every published fix of every boat with YB's distance to finish; re-fetched every 6 h or with **Full tracks**), then polls the light Expedition position file (`yb.tl/<race>-expedition.txt`, latest fix per boat) every 5 min to 3 h. Every fix is kept in `racenav-data\tracking\<race>\`, so tracks build up from what this app has downloaded. Boats are shown at their latest fix or where they were at the weather slider's time; ★ follows a boat; class filter; names; YB course. The Fleet table in the dock ranks by distance to finish along the YB course, with SOG/COG made good between fixes and distance in the last 24 h. **Copy course** puts the YB course in the Course panel; **Route** on a boat starts the routing from its last fix and time (dropping YB course points it has passed when the Course panel holds the YB course).

## Own boat

Rail, bottom: **Centre** puts the boat in the middle of the chart. Chart drawer, Own boat: a heading line (solid; boat speed through the water, else SOG) and a COG line (dashed orange; SOG), each on or off, each running for a set time (1 min to 48 h) or to the edge of the chart. They are rhumb lines, so straight on the chart. Damping: moving averages of the Signal K readings (course and speed; wind; depth) over 0–60 s, with angles averaged as vectors; position is never damped. The boat symbol, its lines and the instruments all use the damped values.

## Instruments

Floating panels of readings, always on top. Drag a panel by its grip; double-click or double-tap the grip to fold the panel to a small title (again to open; Fold all in the drawer); tap a reading to change it (navigation, wind, route/waypoint, routing targets), add one next to it, remove it, or change the panel's size and direction. Instruments drawer: add panels, lock the layout, reset. The layout is kept per PC. Readings from Signal K grey out after 10 s without data; depth turns amber below safety depth + 2 m and red below the safety depth. The tile states (`stale`, `warn`, `alarm`) are where user-set warnings and alarms will plug in.

## Front-end layout

- Rail + one drawer (`ui.js`): controls. Panes: Weather, Course (`route.js`), Routing (`routing.js`), Library, Boat (`boat.js`), Chart, Instruments.
- Dock (`dock.js`): wide views along the bottom, registered by name (`dock.register("compare", { title, render(body, tools) })`). Future: polar plot and tuning, routing timeline graphs, ensemble spread.
- Floating instrument panels (`instruments.js`) keep clear of the dock.
- Statistics (`stats.js`): `STATS` lists the compare columns; adding one there makes it available in the table and its ⚙ menu.

## Conventions

- WGS84 decimal degrees, south and west negative.
- Bearings in degrees true, distances in NM, speeds in knots, times in UTC.
- en-GB spelling in docs and UI text.
