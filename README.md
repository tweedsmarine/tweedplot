# TweedPlot

**TweedPlot: Professional Marine Chart Plotter**, from [TweedsMarine](https://tweedsmarine.nz). Formerly developed as "racenav": crate names, `RACENAV_*` environment variables and `racenav.*` settings keys keep the old name internally. Data folder: `C:\dev\tweedplot-data`, or the old `C:\dev\racenav-data` while that is the one present; app data `%APPDATA%\nz.tweedsmarine.tweedplot` (copied from the old `nz.tweedsmarine.racenav` on first start).

A modern race navigation app for Windows: chartplotter basics, a GRIB viewer for many runs at once, an in-house routing engine and race tools. Live data can come from Signal K or the native NMEA multiplexer; Signal K remains available for replay and as an alternative live source.

Spec and roadmap: https://claude.ai/code/artifact/e37fa4c3-20c2-4aec-af8f-641714f39c27

## Layout

| Path | What it is | Status |
| --- | --- | --- |
| `crates/nav-core` | Positions, bearings, distances, angle helpers (NM, kn, deg true) | Working, tested |
| `crates/gribdl` ocean | MUR SST (ERDDAP) and HYCOM ESPC (NCSS) downloads: NetCDF-3 read (`grib::netcdf`) and written as GRIB2 (SeaTemp, Salinity, SubTemp, SubSalinity, currents) | Working; servers not reachable from the dev sandbox, untested live |
| `crates/polar` | Read qtVlm `.csv` / `.pol` polars, interpolate, best VMG | Working, tested |
| `crates/grib` | GRIB2 decoding (simple and complex packing, bitmaps, Meltemus JSON section), wind/gust/MSLP/waves/currents in space and time | Working, matches ecCodes on Meltemus files; GRIB1 next |
| `crates/routing` | Isochrone routing (modified isochrone method) round land and water shallower than the safety depth, multi-leg, tack/gybe penalties, currents | Working, tested |
| `crates/chart` | LINZ GeoPackages (Topo50 coastline, depth areas, contours, soundings, all scale bands) served as vector tiles | Working, tested on the full LINZ download |
| `crates/signalk` | Signal K WebSocket client (`subscribe=all`), own-vessel state (position, COG/SOG, heading, BSP, depth, apparent and true wind, log), true wind worked out when the server has none, other contexts (AIS) passed on, session log and log replay | Working, tested against Signal K playing a Morgane log |
| `crates/marine-nmea` | NMEA 0183 framing, checksum validation, common sentence decoding, and searchable sentence catalog; unknown valid sentences retained | Parser tested; connected to the Settings viewer and source-aware multiplexer |
| `app/src-tauri/src/nmea.rs` | Concurrent native serial, TCP client and UDP listener workers; source status and observations; per-data-group source preferences; automatic failover with alarms | Multiple inputs can drive one composed vessel state. Actisense/NMEA 2000 and protected configuration remain future work; virtual COM output is a release goal; network I/O will be developed with RADAR overlay |
| `crates/ais` | AIS targets: Signal K AIS contexts and the aisstream.io WebSocket feed, merged by MMSI, expiry, distress kinds from MMSI | Parsers tested; aisstream.io not yet tried live |
| `crates/stations` | Weather observations: MetService WIS2 (OGC API), aviationweather.gov METAR, NOAA NDBC buoys | Parsers tested; not yet tried live |
| `crates/library` | Mark library, routes, saved routings and tracks as JSON files; track recorder; GPX import and export | Working, tested |
| `crates/tides` | LINZ tide predictions: daily tables, secondary ports from LINZ differences, cosine curves, downloader | Working, tested on LINZ files |
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

The app connects to `ws://localhost:3000/signalk/v1/stream?subscribe=self`. To use another server, set `RACENAV_SK_URL` before starting. Every Signal K message is logged to `%APPDATA%\nz.tweedsmarine.tweedplot\logs\sk-<date>-<time>Z.log`.

Configure NMEA inputs and the multiplexer in Settings. Enabled sources are monitored concurrently; each data group (position, COG/SOG, heading, speed through water, wind, depth, and water temperature) can have a preferred source. Automatic failover is enabled by default and uses the configured source order after the preferred source becomes stale. A source switch raises an alarm. NMEA inputs currently feed TweedPlot only; virtual COM output remains a release goal, and network I/O is coordinated with RADAR overlay development.

## Windows builds (GitHub Actions)

- `.github/workflows/build.yml`: builds only when asked, so ordinary pushes use no Actions minutes. Either **Actions › build › Run workflow** (choose **both**, **x64** or **arm64**, and whether to run the tests first), `gh workflow run build.yml -f arch=arm64` from a terminal, or put `[build]` in the message of the last commit you push. It runs the Rust tests (on Linux: half the Windows rate) and builds the NSIS installers for **x64** (`windows-latest`) and **ARM64** (`windows-11-arm`, native). They are on the run page under Artifacts for 30 days.
- `.github/workflows/release.yml`: push a tag `vX.Y.Z` (matching `version` in `app/src-tauri/tauri.conf.json`) and both installers go on a draft pre-release; check it, add notes, Publish.
- The installers are not code-signed: SmartScreen warns until they are (More info › Run anyway).

## Chart data

The app reads LINZ GeoPackage downloads (zipped as downloaded is fine) from `RACENAV_CHART_DIR`, else `C:\dev\racenav-data\linz`, else `%APPDATA%\nz.tweedsmarine.tweedplot\charts`. On the first run it unzips them into a `.unzipped` folder beside the zips (about 600 MB for all bands); after that, start-up takes a couple of seconds. Layers used:

- NZ Coastlines and Islands Polygons (Topo 1:50k)
- Depth area polygons, depth contour polylines and sounding points (Hydro), every scale band

Depth shading follows the S-52 day colours, with the safety depth and safety contour set in the panel. LINZ data is CC BY 4.0 and **not for navigation**.

World depths use the bundled Natural Earth bathymetry as a global fallback, including outside downloaded ETOPO regions. ETOPO provides more detailed shading only within its downloaded coverage; it supplements rather than replaces Natural Earth. The 1° graticule is hidden at world scales and appears from zoom 5.

### Chart features (lights, marks, dangers, areas)

The LINZ hydrographic feature layers (the "121 layers" download, any scale band, GeoPackage) go in the same folder. The app draws:

- lights with paper-chart labels (`Fl(2) WR 12s 22m 20-17M`) and sector arcs;
- lateral, cardinal, isolated danger, safe water and special buoys and beacons, with colours and topmarks;
- rocks (covers and uncovers, awash, underwater), wrecks and obstructions, with depths;
- restricted areas, marine farms, anchorages, cable areas, caution areas;
- submarine cables, pipelines, leading lines and recommended tracks;
- names of bays and sea areas; landmarks, racons, pilot boarding places, moorings.

Each feature shows from the scale its chart gives it (SCAMIN). Underwater rocks, wrecks and obstructions deeper than the safety contour show as their depth only, without the danger symbol. Charts east of 180° (the Chathams; LINZ stores them as 180–190°E) draw on the right side of the date line. Where several bands are loaded the finest wins. Click a feature for all its details in words. Chart panel: switch features, light sectors and labels on or off. The depth layers inside that download duplicate the separate depth downloads and are read once.

Routing also keeps off rocks, wrecks and obstructions that dry, are awash, or have less water than the safety depth (or an unknown depth), and out of marine farms. Both can be switched off in the Chart panel.

Below zoom 6 (and anywhere without LINZ data) the app falls back to OpenStreetMap land polygons via geo-maps 0.6.0 (`app/public/data`, built by `tools/prep_land*.py`).

To check a chart folder or test tiles in a browser:

```
cargo run --release -p chart --example chart_info -- C:\dev\racenav-data\linz
cargo run --release -p chart --example tileserve -- C:\dev\racenav-data\linz 7070
```

## World coastline and depths

Chart panel > **Chart data** shows what is installed and downloads the rest. Nothing here is used by routing except the GSHHG land; world and ETOPO depths are for orientation only.

| Data | How | Where | Licence |
| --- | --- | --- | --- |
| World depths: Natural Earth 10 m bathymetry (0, 200, 1000 … 10000 m) | Bundled: `app/src-tauri/resources/world_depth.gpkg` (33 MB), built by `cargo run --release -p chart --example world_depth -- <geojson dir> <out.gpkg>` from the `ne_10m_bathymetry_*.geojson` files of nvkelso/natural-earth-vector | Installer resources | Public domain |
| World coastline: GSHHG 2.3.7 shapefiles | **Download** (about 150 MB, from soest.hawaii.edu or the NOAA NGDC mirror), or put `gshhg-shp-2.3.7.zip` there yourself | `RACENAV_COAST_DIR`, else `C:\dev\racenav-data\coastline`, else app data `coast` | LGPL 3 |
| Detailed depths: ETOPO 2022 15″ | **Get for chart view**: the area on screen from NOAA CoastWatch ERDDAP (`ETOPO_2022_v1_15s`, DAP2 binary), thinned to at most 6 M points, split at the date line. Written as `etopo_<S>_<N>_<W>_<E>_<n>s.gpkg` (depth bands, plus contours 5–5000 m) | `RACENAV_BATHY_DIR`, else `C:\dev\racenav-data\bathy`, else app data `bathy` | Free to use and redistribute (NOAA NCEI) |

GSHHG: on the next chart load the app builds `coastline\.unzipped\gshhg_land.gpkg` once (a minute or two; build version 2 cuts land polygons into pieces of at most 4000 points and keeps the coastline as line chunks in `gshhs_<r>_ln`, so a tile reads only what it needs, and an older build is redone automatically), then draws land at five resolutions (crude at world scale to full, every island, from zoom 8) and routing avoids it. Inside NZ, LINZ Topo50 is used instead where it is loaded. **Coastline detail** (Chart data: Standard, More, Most) shifts each resolution one or two zoom bands further out (`chart::coast::LAND_DETAIL`, set by the `chart_land_detail` command; Most = full from zoom 4); tiles are simplified to screen resolution either way, so only drawing time grows.

The world depths draw as the `wdepth` tile layer (Natural Earth below zoom 4 from a simplified table, full detail from zoom 4, ETOPO over it where downloaded) and ETOPO contours as `wdcnt` from zoom 7, all under the LINZ depth areas. `RACENAV_ETOPO_URL` overrides the ERDDAP dataset URL.

## Weather (GRIB)

**Download GRIBs (free)** in the Weather drawer:

- **NOAA NOMADS**: GFS 0.25°/0.5°/1° (wind, gusts, pressure, rain rate, CAPE) and GFS-Wave 0.25° (significant height, peak period and direction, swell), cut to the area on the server, one request at a time.
- **ECMWF open data** (CC BY 4.0, © ECMWF): IFS 0.1° (about 9 km; wind, pressure, gusts where published, air temperature, surface currents) and 0.25° (wind, pressure, gusts where published), AIFS 0.25° (ECMWF's AI model: wind, pressure) and IFS waves at both resolutions (significant height, peak period, mean direction). ECMWF does not document the 0.1° current fields as tidal-stream predictions. IFS is 3-hourly to 144 h then 6-hourly to 360 h (AIFS 6-hourly); the 06Z and 18Z IFS runs stop at 144 h, so longer requests use the 00Z/12Z runs. The 0.1° forecast is free open data, published about two hours after the 0.25° data, and its surface and pressure-level fields are separate files. ECMWF publishes whole-globe files with an index, so only the wanted fields are fetched (HTTP ranges), four steps at a time, from data.ecmwf.int or its AWS mirror; each field (CCSDS-packed) is decoded, cut to the area and written again. Requested 0.1° global field ranges are cached in `grib\.ecmwf-cache` (up to 5 GiB, oldest first) and reused for other areas of the same run; only newly requested ranges are fetched. Downloads are much bigger than the resulting area files (about 0.6 MB per 0.25° field, or 3 MB per 0.1° field per step); the estimate shows both.

Draw the area on the chart (or use the chart view or the course), choose the forecast length (1–16 days) and step, check the size estimate, Download. The newest complete run is found by checking the server, the steps are joined into one file in `racenav-data\grib`, and it is loaded straight away.

**Saildocs email request** (inside Download): for email-only links. Builds the request line from the same area, length and step, e.g. `send GFS:33S,48S,165E,175W|0.25,0.25|0,3..120|WIND,PRMSL`, for GFS, ICON (0.125°) or the free part of ECMWF (0.4°), with the parameters ticked and a size estimate. **Email** opens a message to query@saildocs.com in your mail program; **Copy** copies the line. Save the attached reply in the GRIB folder (Saildocs sends GRIB1, which TweedPlot now reads).

GRIB formats read: GRIB2 with simple, complex, complex + spatial differencing and CCSDS packing (ECMWF since 2023), and GRIB1 with simple packing (regular lat/lon), with or without a bitmap.

The Weather panel lists GRIB files found in `RACENAV_GRIB_DIR`, `C:\dev\racenav-data\grib` and qtVlm's `C:\Program Files (x86)\qtVlm\grib`. Load as many as you like. One is shown on the chart (wind barbs plus speed shading under the land), and the table compares every loaded run at the cursor for the slider time. Outside a file's time span there is no value: a short GRIB never shows as calm.

**Show** picks what the shading displays: wind, gusts, wave height (Hs), swell, wave period, rain (mm/h), CAPE (squall risk) or pressure, or nothing. Choices the shown GRIB lacks are greyed out, and a wave-only GRIB switches to wave height by itself. Tick boxes add wind barbs, wave direction arrows (pointing the way the waves travel) and isobars at a chosen interval (1, 2, 3, 4, 5, 8 or 10 hPa; with **Auto** ticked the interval doubles below zoom 4 and quadruples below zoom 2.5), whatever the shading shows. The time slider follows the shown layer, so wave and rain files work without any wind in them.

```
cargo run -p grib --example grib_info -- path\to\file.grb2 -36.0 174.8
```

### GRIB agreement

With two or more GRIBs loaded (different models, or several runs of one), **Show > Agreement of the loaded GRIBs** shades the chart by how much they disagree at the slider time — the displays of the earlier stand-alone agreement viewer:

- **Wind disagreement (vector)**: RMS distance of each GRIB's wind from their mean wind (kn), speed and direction together;
- **Wind speed spread** (kn) and **Pressure spread** (hPa): highest minus lowest, or P90 − P10 with seven or more GRIBs;
- **Wind direction spread** (°): about the mean direction, blank where the mean wind is under 6 kn;
- **Run trend**: newest run minus oldest, wind speed (kn; red = the newer run is windier).

Green means the forecasts agree, red that they don't. The tick beside each loaded GRIB includes it or leaves it out; GRIBs not covering the slider time or the view are left out automatically. Each is sampled bilinearly, linear in time, on a common grid as coarse as the coarsest GRIB. When every GRIB compared is an ensemble, each member is a layer (the note says so); otherwise an ensemble counts once, as its mean. The line under the legend lists the GRIBs compared and the mean and maximum in view, and the table below still gives every GRIB's values at the cursor.

### Meteogram

Right-click the chart > **Meteogram here**: every loaded GRIB at that point over its whole span, in the dock — wind speed (solid; gusts dashed; an ensemble's member range shaded), direction (drawn about the GRIBs' shared mean direction, so GRIBs that agree stay together through north and a veer through north stays one line) and pressure. The violet line is the slider time; click the graph to move the slider there. **Clear point** removes the violet ring from the chart.

### Verify

Weather drawer > **Verify runs…** (the Verify tab in the dock) scores earlier runs against what a later run says actually happened at its hour 0 (its analysis — the best estimate available without observations). A verify time is any loaded run's hour 0 that an earlier loaded run also forecast; the reference is the mean of the deterministic runs with that hour 0. Scores are over sea points in the chart view (land left out when a wave GRIB is loaded): RMS vector error, speed bias (+ = forecast too windy), mean absolute direction error and mean veer (+ = forecast veered), and MSL bias, best RMS in green. The track record averages every verify time by model and lead (≤ 24 h, ≤ 48 h, > 48 h), so after a few days of downloads it shows which model has been doing best here. Click a forecast to shade its error on the chart (**Vector**, **Speed** or **Direction**: Show > Verify); **Rescore this view** after moving the chart, **Stop shading** to go back to the wind.

```
cargo run --release -p grib --example verify -- older.grb2 newer.grb2   (BBOX=w,s,e,n limits the area)
```

**Wind and waves in one file.** Wind and waves come from different models (GFS + GFS-Wave, ECMWF IFS/AIFS + ECMWF wave). Download: tick "Waves … in the same file" for GFS, IFS or AIFS; the wave model at the matching resolution is fetched for the same run, area and steps and appended, giving `…+waves.grb2` (if the waves fail the wind file is kept and the status says why: `gribdl::download_ex`, `Downloaded { path, note }`). Files already on disk: load a wind GRIB and a wave GRIB of the same run and the Weather drawer offers **Combine** (`grib_combine` → `grib::combine`: checks one has wind without waves and the other waves without wind, same run, overlapping areas; writes the two files one after the other as `<wind>+waves.<ext>`, nothing re-encoded; originals kept). GRIB messages each carry their own grid, so the file needs no other change; the name reads "ECMWF IFS + waves".

## Routing

Course drawer: start and marks (see below). Routing drawer, Run tab: start time (weather slider time, now, or a set time), settings, **Compute** (the GRIB shown on the chart) or **With each GRIB** (one routing per loaded GRIB, then the compare table opens). Messages are listed newest first, one line each. Routings tab: every routing of this session plus saved ones, each with its own colour; tick to show on the chart, click to make it the one shown in detail (isochrones, points, summary, instrument targets), Save, each shown routing has a triangle in its colour where its boat is at the weather slider's time (pointing along its heading; faded once finished; hover for details, click to make it the active one), or open the **Compare table** in the dock at the bottom (it has its own GRIB chooser and time slider, so the Weather drawer can stay closed) (click a column to sort, ⚙ to choose columns, best values in green). Boat drawer: choose a polar (your own, in `C:\dev\racenav-data\polars` or the app data `polars` folder, or one shipped in `app/src-tauri/resources/polars`; **Import polar…** copies a file in after checking it), efficiency and tack/gybe time. Route drawer: **Set start** (or **From boat**), **Add marks** by clicking the chart, or click a library mark to reuse it (Esc to stop), then **Compute routing**. The routing starts at the weather slider time with the GRIB shown on the chart. Hover a routing point for heading, TWA, TWS, AWA/AWS and boat speed (it no longer moves the GRIB time unless Settings › "Hovering a routing point moves the GRIB time" is ticked); **Save routing** keeps it in the library, **GPX** writes it to the export folder.

How it works: each step fans out headings from every point of the last isochrone, boat speed from the polar at the GRIB wind (plus current if ticked), a tack or gybe costs its time, and steps that cross land (Topo50) or LINZ depth areas shallower than the safety depth are dropped. Obstacles come from a 50 m raster built from the charts for each run, with the finest chart scale winning. Pruning keeps one point per small grid cell (so routes find their way round headlands and into bays); without charts it uses the classic sectors from the start. If the GRIB runs out, the route stops and says so.

**Gate sweep** (Routing drawer, Gates tab): the gate method, automated. Choose the mark the gates come before (default the finish), a line — a meridian (e.g. 5E) or a parallel — the range along it (e.g. 30S to 40S) and the number of gates; the gates show as violet dots on the chart while the tab is open, with their spacing. **Run gate sweep** computes one routing per gate (each gate is a mark inserted before the target, reached within the arrival radius) with the start time and settings of a normal Compute, with the GRIB shown or with each loaded GRIB, two at a time (**Stop** finishes the ones running). The dock's Gates table lists each gate's arrival per GRIB and the time lost against the best gate under that GRIB; with several GRIBs, the mean and worst time lost, ranked by the mean — the gate that is nearly best under every GRIB is the robust choice when they disagree. A small graph plots time lost against gate position. Click a cell to show that routing in detail; **Show only these** hides other routings; **Remove these routings** takes the sweep's routings off the list. The routings are ordinary ones, so they can be saved, compared or exported like any other.

**How each mark is reached** (Course drawer, under each mark): *Pass through* (enter its arrival circle: its own radius if typed, else the routing's arrival radius), *Leave to port* / *Leave to starboard* (at least the radius off; the mark is a disc of that radius the route may not cut through on that leg and the next; following the string rule, the route goes round the mark from the previous mark's bearing to the next one's, so a mark inside the turn is gone right round; if land, shallows or a barrier close off that side the routing stops and says so), or *As close as possible* (a 0.01 NM circle). Rounding marks show a red (port) or green (starboard) ring; a radius of its own shows a grey dashed ring. The settings are saved with the route.

**Limits and barriers** (Routing › Run): search angle (only headings within ± that of the bearing to the next mark are tried: faster, but keep 90° or more where the route may tack or go round land), heading step, minimum and maximum wind (the routing won't sail on from places outside that range), maximum wave height (from the routing GRIB's waves, else the newest loaded wave GRIB covering the start; the message says when no waves were found), and the true wind angles allowed (e.g. 50° to stay off the wind, 165° to avoid a dead run). **Keep off barriers** uses Library › Barriers: lines the routing may not cross and zones it may not enter (draw them like a ruler: click points, double-click to finish; untick to keep one but not use it; right-click to delete a point or the barrier or switch it off), and library marks given a **Barrier radius** in the mark editor (a red dashed ring; ignored when the mark is on the course). All are kept in `racenav-data\library` (`barriers.json`, marks, routes).

Not yet: route optimisation, gates as lines (a gate here is a point), power-boat routing (fuel and comfort limits).

```
cargo run --release -p routing --example route_cli -- --grib file.grb2 --polar boat.csv --chart C:\dev\racenav-data\linz --start -36.838,174.805 --marks "-35.465,174.80;-35.252,174.100"
```

## Marks, routes and routings

Everything is kept as plain JSON in `C:\dev\racenav-data\library` (or `RACENAV_LIBRARY_DIR`; `<app data>\library` when racenav-data doesn't exist):

```
marks.json            every mark, shared by all routes
routes\<id>.json      a route: name, start, ordered mark ids with rounding and radius
barriers.json         barrier lines and zones
routings\<id>.json    a saved routing: its inputs (start, marks, time, GRIB, polar, settings) and the result
export\*.gpx          GPX written by the app
```

Route drawer: name the route and **Save**: new marks go into the library (named WP 1, WP 2 … until renamed), moved library marks are updated. **Draw legs as great circles** changes the displayed course legs from straight rhumb lines to great-circle arcs; this chart-display choice is kept on this PC. **Navigate** picks the mark being sailed to (it steps on by itself inside the arrival radius); the route instruments use it. Library drawer: Routes (open, GPX, delete), Marks (find, edit name/type/group/position/notes, add to the route, delete; positions can be typed as `36 50.12 S` or `-36.8353`; filters by group, type, in a route or not, and in the chart view, optionally applied to the chart too, kept per PC), Routings (show, delete). **Import GPX** reads waypoints and routes (qtVlm, OpenCPN, Expedition …); a waypoint with the same name as a library mark and within about 50 m becomes that mark. Right-click the chart: **New library mark here**. The start can be a library mark too: in Set start mode click the mark, right-click it (Start route at …), or press **Start** beside it in the Marks list; the route keeps the link (`start_mark`).

### Tracks

Library > **Tracks**:

- **Record** logs the own boat's track from Signal K into the library (`tracks\<id>.json`). Fixes are kept every 10 s while moving (at least 5 m), sooner in a tack or gybe, and every 10 min when still; a gap of 10 min or more starts a new segment. Times come from the GPS (`navigation.datetime`) when the server sends it, so a replayed log keeps its own times. Recording carries on while plotting is paused, is saved every 30 s and on exit, and resumes the same track after a restart if its last fix is under 2 h old. **New log** starts a new track.
- **Import GPX…** (here or in Routes) reads `<trk>` tracks with times, and speed and course where the file has them (GPX 1.0 elements or extensions), along with any waypoints and routes.
- Tick a track to show it; click its name to zoom to it; **GPX** writes it to the export folder (GPX 1.1 with times, course and speed); ✎ for name, colour and notes; ✕ deletes. Click a track on the chart for the time, SOG and COG there. **Boat at weather time** puts a dot on each shown track where that boat was at the weather slider time, for replays and comparing races.

The orange line behind the boat is still the live trail since the app started (the Clear track button in the toolbar at the top left clears it); the recorded track is separate.

A library mark's type can also be a buoy: port (red can) or starboard (green cone) lateral, north/east/south/west cardinal, isolated danger, safe water or special. Those are drawn with their IALA A chart symbol instead of a dot, which helps for temporary race marks and marks missing from the charts.

**Mark types**: racing mark, waypoint, start/finish line end, danger, IALA A buoy symbols, and points of interest drawn as round badges: good anchorage, mooring, shelter, fishing spot, diving spot, foul ground, wreck, hazard, landing point, camping spot, hut/building, viewpoint, water filling, fuel, provisions/shop, note. Any of them can be used as a course mark.

## Tides

Chart panel > Tides > **Download LINZ tides** fetches, once, LINZ's tide tables for this year and next into `C:\dev\racenav-data\tides` (or `RACENAV_TIDE_DIR`):

- the daily high/low tables for about 90 places (`<Place> <year>.csv`, from static.charts.linz.govt.nz);
- the secondary ports file (`secondary-ports.csv`, NZ Nautical Almanac): about 330 more places with their reference port, time differences, mean levels and range ratio.

Tide stations appear on the chart: a filled "T" for a LINZ daily table (from zoom 5), an open "T" for a secondary port (from zoom 8), grey where LINZ gives levels only. Click one for the day's tide curve, highs and lows, mean sea level and the height at the planning time (the weather slider time when planning, else now), with rising or falling; step through days with ◀ ▶. Times are NZ local (Chatham time for the Chathams), as LINZ publishes them.

Secondary ports use LINZ's method: the reference port's highs and lows shifted by the mean time differences, heights `MSL + (h_ref − MSL_ref) × ratio`. Between highs and lows the curve is LINZ's cosine curve. These are not official tide tables (Maritime Rules Part 25), and not for navigation.

## Fleet (YB race tracking: Fleet › Race (YB))

Fleet drawer: type the race's YB name (as in `yb.tl/<race>`, e.g. `ggr2026`) and Open. The app fetches the race setup (`cf.yb.tl/JSON/<race>/RaceSetup`: boats, classes, course) and downloads the full tracks (`cf.yb.tl/BIN/<race>/AllPositions3`: every published fix of every boat with YB's distance to finish; re-fetched every 6 h or with **Full tracks**), then polls the light Expedition position file (`yb.tl/<race>-expedition.txt`, latest fix per boat) every 5 min to 3 h. Every fix is kept in `racenav-data\tracking\<race>\`, so tracks build up from what this app has downloaded. Boats are shown at their latest fix or where they were at the weather slider's time; ★ follows a boat; class filter; names (at every zoom; where names would overlap some are left out, followed boats first); YB course. The Fleet table in the dock ranks by distance to finish along the YB course, with SOG/COG made good between fixes and distance in the last 24 h. **Copy course** puts the YB course in the Course panel; **Route** on a boat starts the routing from its last fix and time (dropping YB course points it has passed when the Course panel holds the YB course).

## Own boat

Settings (cog at the bottom of the rail) holds the persistent vessel profile: name, IMO, call sign, MMSI, draft, length, breadth, air draft, optional reference position and GPS antenna offsets. The reference position is the chart's initial fallback centre when no live GPS fix is available; it never substitutes for a live fix and does not override a location the user has already chosen. Existing length, breadth, antenna offset, air draft and MMSI values from the former Boat panel's browser settings are migrated on first launch. Polar, efficiency and tack/gybe costs are in Routing › Boat and polar.

Rail, bottom: **Centre** puts the boat in the middle of the chart. Chart drawer, Own boat: a heading line (solid; boat speed through the water, else SOG) and a COG line (dashed orange; SOG), each on or off, each running for a set time (1 min to 48 h) or to the edge of the chart. They are rhumb lines, so straight on the chart. Damping: moving averages of the Signal K readings (course and speed; wind; depth) over 0–60 s, with angles averaged as vectors; position is never damped. The boat symbol, its lines and the instruments all use the damped values.

## Measuring

**Cursor** instrument tiles (Instruments: add them to any panel; a Cursor panel is added once to existing layouts): the cursor's position, and the bearing (°T) and range from the boat to the cursor. On a touch screen a tap sets the cursor.

**Ruler** (left rail, above Centre): click to set the first point, click again for each further point, double-click to finish; the ruler stays on the chart (Esc stops early). Each leg shows its bearing (°T, and °M when Signal K sends the variation) and distance, with the total at the end; while drawing, the leg to the cursor and the running total follow the mouse. Rulers use rhumb lines by default; right-click a ruler and choose **Show ruler as great circle** to display great-circle arcs with matching great-circle distances and initial bearings (choose **Show ruler as rhumb line** to switch back). Points move like course marks: click one (amber ring), wait for the ring to turn green, then drag. Right-click a ruler: **Save ruler as route** (first point the start, the others new library marks; the route opens in the Course panel), **Delete ruler point**, **Delete ruler**, **Delete all rulers**. Rulers and their line type are kept between sessions.

## Instruments

Floating panels of readings, always on top. Drag a panel by its grip; double-click or double-tap the grip to fold the panel to a small title (again to open; Fold all in the drawer); tap a reading to change it (navigation, wind, route/waypoint, routing targets), add one next to it, remove it, or change the panel's size and direction. Instruments drawer: add panels, rename them, choose the views each shows in, lock the layout, reset. The layout is kept per PC. Readings from Signal K grey out after 10 s without data; depth turns amber below safety depth + 2 m and red below the safety depth. The tile states (`stale`, `warn`, `alarm`) are where user-set warnings and alarms will plug in.

- **GRIB at cursor** readings: TWS, TWD, gust, MSLP, wave height, period and direction, rain, air temperature, the GRIB time and name, for the GRIB shown on the chart at the cursor (or the last tap on a touch screen) and the slider time. A panel of them is added once to existing layouts.
- **Show instrument panels** hides or shows them all; the tick beside each panel in the list hides or shows that one.
- **Time slider**: a floating bar (GRIB choice, step back/forward, play, Now, what the shading shows, the slider and time) that can be dragged anywhere, so every drawer can stay closed. On by default; × hides it, the Instruments drawer brings it back.
- **Wind arrows**: true wind (blue) and apparent wind (orange), pointing the way the wind blows with the tip on the boat, longer with more wind. With live Signal K data they sit on the boat (from the instruments); planning (paused, or no live data) they sit on the cursor: true wind from the GRIB shown, apparent wind only for a shown routing within 40 px of the cursor (its heading and boat speed there); away from a routing only the true wind is drawn.

## Live data: replay, AIS, alarms, stations

- **Replay** (Chart › Replay a log): plays a Signal K session log (`<app data>\logs\sk-*.log`, one `time<TAB>message` line each) in place of the live stream, with pause, speed, seek and stop (`signalk::replay`, commands `replay_logs/start/control/stop`). Live events are dropped while a replay runs; the track recorder ignores replays.
- **AIS** (Fleet › AIS; Fleet › Own fleet): own MMSI (Settings) dropped in `ais::Targets::set_own` (`ais_set_own`); per-target tracks of the last 6 h (`ais_tracks`) drawn for own-fleet boats (localStorage `racenav.ownfleet`: MMSI, name, group, track) and optionally the selected target. Signal K AIS contexts and aisstream.io (free key; the subscription follows the chart view, up to 6°, at most every 5 s) merged in `ais::Targets`; changes go to the page every 2 s (`ais` event). CPA/TCPA in `app/src/ais.js`; collision alarm under a CPA within a TCPA (anchored/moored ignorable); AIS-SART/MOB/EPIRB (MMSI 970/972/974, or nav status 14) always alarm.
- **Alarms** (`app/src/alarms.js`): banner + Web Audio tones until silenced (in the primary window; any window can silence); shallow water, anchor watch, air draft (Settings) against charted VERCLR/VERCSA of bridges, overhead cables and pipelines (`chart::ChartStore::clearances`, command `chart_clearances`), position lost, own MOB (also saved as a library mark, group MOB). Other modules raise their own (`alarms.raise/clear`).
- **Weather stations** (Weather › Weather stations and buoys): `stations_latest` (10-minute cache per source) and `station_history`; the popup plots observed wind, gusts and pressure against `grib_meteogram` for the loaded GRIBs, with the mean forecast-minus-observed difference.
- **Display colours**: day, dusk, night red, night green: CSS filters on the whole page (invert, then an SVG colour matrix to one channel), with already-dark elements inverted first so they stay dark (`app/src/palette.js`).
- **Views and monitor windows** (`views.js`, `monitors.js`, `role.js`, `windowstate.js`, `palette.js`, `uiprefs.js`): Navigation, Planning and Instruments views (`VIEWS`; more such as radar or sounder are added there). Every window is the same app (`index.html?view=X`); `main` is the primary and runs the once-only jobs (alarm checks and sound, MOB, anchor watch, AIS online subscription), while `monitor-N` windows mirror the `alarm-state` event and send `alarm-cmd` (`ack`, `dismiss`, `mob`, `clearmob`, `anchor`). Per-window settings (view, palette, UI prefs) use `own(key)` in `localStorage`, which adds `@label` for non-primary windows; window position, size and the open monitors are kept by `windowstate.js`. Every window label needs an entry in `capabilities/default.json`. On Windows, window-creating commands must be `async`. Marks, routes and routings changed through the backend commands (`mark_save`, `route_save`, `routing_save`, `gpx_import` and their deletes) emit `library-changed`, which each window answers by reloading the library. The route being edited (`racenav.route.work`) follows between windows through the `storage` event.
- **Instruments view** (`insttiles.js`, `gridmath.js`): a 12 × 8 grid of single-reading tiles kept in `layout.tiles` of `racenav.instruments`. `gridmath.js` (`fits`, `clampTile`, `snapBelow`, `placeTile`, `firstFree`) is tested in `gridmath.test.js` (`npm test`).
- **Screen controls** (`uiprefs.js`): per-window zoom buttons and large controls, `auto` following `(any-pointer: coarse)`. MOB is alone at the top right, Drop at the bottom right, the toolbar at the top left and zoom buttons at the bottom centre.
- **Night cursors**: the colour filters do not reach the system cursor, so `style.css` supplies red and green SVG cursors for the night palettes (`--cur-*` variables), including resize cursors and the chart canvas, whose cursor MapLibre and the modules set inline.
- **Tooltips** (`tooltips.js`): the system `title` tooltips cannot be coloured, so the page draws its own (`#tip`), taking the text from `title` attributes; it goes through the night filter like the rail.

## User manual

The manual is built into the app: **Manual** on the rail (or F1) opens it in a panel on the right, and the **?** in each drawer's title bar opens it at that drawer's section. Tabs: Reference, Route planning, Weather routing, Glossary; contents, search across every tab, pictures and diagrams full size on click, Wide. External links open in the browser (`open_url` allows mailto:, http: and https: only).

The text is written as a Claude Doc ("TweedPlot User Manual") and bundled from `app/public/manual` (Markdown, screenshots in `img/`, diagrams as inline SVG in `fig/`), so every build carries the manual for its own version and it works offline. To update it: export each tab as Markdown, then `python tools/manual/import.py <exports> <screenshots> tools/manual/fig`; the script turns the export's picture and diagram placeholders back into images (by alt text) and SVGs (by caption). The diagram sources are in `tools/manual/fig` (`*.jsx`, rendered with `render.mjs`, needs esbuild). The viewer is `app/src/manual.js` (uses `marked`).

## Front-end layout

- Rail + one drawer (`ui.js`): controls. Panes: Weather, Course (`route.js`), Routing (`routing.js`), Library, Boat (`boat.js`), Chart, Instruments.
- Dock (`dock.js`): wide views along the bottom, registered by name (`dock.register("compare", { title, render(body, tools) })`). Future: polar plot and tuning, routing timeline graphs, ensemble spread.
- Floating instrument panels (`instruments.js`) keep clear of the dock.
- Statistics (`stats.js`): `STATS` lists the compare columns; adding one there makes it available in the table and its ⚙ menu.

## Conventions

- WGS84 decimal degrees, south and west negative.
- Bearings in degrees true, distances in NM, speeds in knots, times in UTC.
- en-GB spelling in docs and UI text.
