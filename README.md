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

The app connects to `ws://localhost:3000/signalk/v1/stream?subscribe=self`. To use another server, set `RACENAV_SK_URL` before starting. Every Signal K message is logged to `%APPDATA%\nz.tweedsmarine.racenav\logs\sk-<date>-<time>Z.log`.

## Chart data

The app reads LINZ GeoPackage downloads (zipped as downloaded is fine) from `RACENAV_CHART_DIR`, else `C:\dev\racenav-data\linz`, else `%APPDATA%\nz.tweedsmarine.racenav\charts`. On the first run it unzips them into a `.unzipped` folder beside the zips (about 600 MB for all bands); after that, start-up takes a couple of seconds. Layers used:

- NZ Coastlines and Islands Polygons (Topo 1:50k)
- Depth area polygons, depth contour polylines and sounding points (Hydro), every scale band

Depth shading follows the S-52 day colours, with the safety depth and safety contour set in the panel. LINZ data is CC BY 4.0 and **not for navigation**.

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

## World coastline (GSHHG)

The bundled world land is coarse (small islands missing). For a detailed one, download the GSHHG shapefiles, `gshhg-shp-2.3.7.zip` (about 150 MB, LGPL), from https://www.soest.hawaii.edu/pwessel/gshhg/ and put the zip (or the unpacked `GSHHS_shp` folder) in `C:\dev\racenav-data\coastline` (or `RACENAV_COAST_DIR`). On the next start the app builds `coastline\.unzipped\gshhg_land.gpkg` once (a minute or two), then draws land at five resolutions (crude at world scale to full, every island, from zoom 8) and routing avoids it. Inside NZ, LINZ Topo50 is used instead where it is loaded.

## Weather (GRIB)

**Download GRIBs (free)** in the Weather drawer:

- **NOAA NOMADS**: GFS 0.25°/0.5°/1° (wind, gusts, pressure, rain rate, CAPE) and GFS-Wave 0.25° (significant height, peak period and direction, swell), cut to the area on the server, one request at a time.
- **ECMWF open data** (CC BY 4.0, © ECMWF): IFS 0.25° (wind, pressure, gusts where published), AIFS 0.25° (ECMWF's AI model: wind, pressure) and the IFS wave model 0.25° (significant height, peak period, mean direction). 3-hourly to 144 h then 6-hourly to 360 h (AIFS 6-hourly); the 06Z and 18Z IFS runs stop at 144 h, so longer requests use the 00Z/12Z runs. ECMWF publishes whole-globe files with an index, so only the wanted fields are fetched (HTTP ranges), four steps at a time, from data.ecmwf.int or its AWS mirror; each field (CCSDS-packed) is decoded, cut to the area and written again. The download is much bigger than the file (about 0.6 MB per field per step); the estimate shows both.

Draw the area on the chart (or use the chart view or the course), choose the forecast length (1–16 days) and step, check the size estimate, Download. The newest complete run is found by checking the server, the steps are joined into one file in `racenav-data\grib`, and it is loaded straight away.

**Saildocs email request** (inside Download): for email-only links. Builds the request line from the same area, length and step, e.g. `send GFS:33S,48S,165E,175W|0.25,0.25|0,3..120|WIND,PRMSL`, for GFS, ICON (0.125°) or the free part of ECMWF (0.4°), with the parameters ticked and a size estimate. **Email** opens a message to query@saildocs.com in your mail program; **Copy** copies the line. Save the attached reply in the GRIB folder (Saildocs sends GRIB1, which racenav now reads).

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

## Routing

Course drawer: start and marks (see below). Routing drawer, Run tab: start time (weather slider time, now, or a set time), settings, **Compute** (the GRIB shown on the chart) or **With each GRIB** (one routing per loaded GRIB, then the compare table opens). Messages are listed newest first, one line each. Routings tab: every routing of this session plus saved ones, each with its own colour; tick to show on the chart, click to make it the one shown in detail (isochrones, points, summary, instrument targets), Save, each shown routing has a triangle in its colour where its boat is at the weather slider's time (pointing along its heading; faded once finished; hover for details, click to make it the active one), or open the **Compare table** in the dock at the bottom (it has its own GRIB chooser and time slider, so the Weather drawer can stay closed) (click a column to sort, ⚙ to choose columns, best values in green). Boat drawer: choose a polar (from `C:\dev\racenav-data\polars` or qtVlm's `polar` folder), efficiency and tack/gybe time. Route drawer: **Set start** (or **From boat**), **Add marks** by clicking the chart, or click a library mark to reuse it (Esc to stop), then **Compute routing**. The routing starts at the weather slider time with the GRIB shown on the chart. Hover a routing point for heading, TWA, TWS and boat speed (it no longer moves the GRIB time unless Settings › "Hovering a routing point moves the GRIB time" is ticked); **Save routing** keeps it in the library, **GPX** writes it to the export folder.

How it works: each step fans out headings from every point of the last isochrone, boat speed from the polar at the GRIB wind (plus current if ticked), a tack or gybe costs its time, and steps that cross land (Topo50) or LINZ depth areas shallower than the safety depth are dropped. Obstacles come from a 50 m raster built from the charts for each run, with the finest chart scale winning. Pruning keeps one point per small grid cell (so routes find their way round headlands and into bays); without charts it uses the classic sectors from the start. If the GRIB runs out, the route stops and says so.

**Gate sweep** (Routing drawer, Gates tab): the gate method, automated. Choose the mark the gates come before (default the finish), a line — a meridian (e.g. 5E) or a parallel — the range along it (e.g. 30S to 40S) and the number of gates; the gates show as violet dots on the chart while the tab is open, with their spacing. **Run gate sweep** computes one routing per gate (each gate is a mark inserted before the target, reached within the arrival radius) with the start time and settings of a normal Compute, with the GRIB shown or with each loaded GRIB, two at a time (**Stop** finishes the ones running). The dock's Gates table lists each gate's arrival per GRIB and the time lost against the best gate under that GRIB; with several GRIBs, the mean and worst time lost, ranked by the mean — the gate that is nearly best under every GRIB is the robust choice when they disagree. A small graph plots time lost against gate position. Click a cell to show that routing in detail; **Show only these** hides other routings; **Remove these routings** takes the sweep's routings off the list. The routings are ordinary ones, so they can be saved, compared or exported like any other.

Not yet: rounding sides (marks are reached within the arrival radius), route optimisation, gates as lines (a gate here is a point).

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

### Tracks

Library > **Tracks**:

- **Record** logs the own boat's track from Signal K into the library (`tracks\<id>.json`). Fixes are kept every 10 s while moving (at least 5 m), sooner in a tack or gybe, and every 10 min when still; a gap of 10 min or more starts a new segment. Times come from the GPS (`navigation.datetime`) when the server sends it, so a replayed log keeps its own times. Recording carries on while plotting is paused, is saved every 30 s and on exit, and resumes the same track after a restart if its last fix is under 2 h old. **New log** starts a new track.
- **Import GPX…** (here or in Routes) reads `<trk>` tracks with times, and speed and course where the file has them (GPX 1.0 elements or extensions), along with any waypoints and routes.
- Tick a track to show it; click its name to zoom to it; **GPX** writes it to the export folder (GPX 1.1 with times, course and speed); ✎ for name, colour and notes; ✕ deletes. Click a track on the chart for the time, SOG and COG there. **Boat at weather time** puts a dot on each shown track where that boat was at the weather slider time, for replays and comparing races.

The orange line behind the boat is still the live trail since the app started (**Clear trk** clears it); the recorded track is separate.

A library mark's type can also be a buoy: port (red can) or starboard (green cone) lateral, north/east/south/west cardinal, isolated danger, safe water or special. Those are drawn with their IALA A chart symbol instead of a dot, which helps for temporary race marks and marks missing from the charts.

**Mark types**: racing mark, waypoint, start/finish line end, danger, IALA A buoy symbols, and points of interest drawn as round badges: good anchorage, mooring, shelter, fishing spot, diving spot, foul ground, wreck, hazard, landing point, camping spot, hut/building, viewpoint, water filling, fuel, provisions/shop, note. Any of them can be used as a course mark.

## Tides

Chart panel > Tides > **Download LINZ tides** fetches, once, LINZ's tide tables for this year and next into `C:\dev\racenav-data\tides` (or `RACENAV_TIDE_DIR`):

- the daily high/low tables for about 90 places (`<Place> <year>.csv`, from static.charts.linz.govt.nz);
- the secondary ports file (`secondary-ports.csv`, NZ Nautical Almanac): about 330 more places with their reference port, time differences, mean levels and range ratio.

Tide stations appear on the chart: a filled "T" for a LINZ daily table (from zoom 5), an open "T" for a secondary port (from zoom 8), grey where LINZ gives levels only. Click one for the day's tide curve, highs and lows, mean sea level and the height at the planning time (the weather slider time when planning, else now), with rising or falling; step through days with ◀ ▶. Times are NZ local (Chatham time for the Chathams), as LINZ publishes them.

Secondary ports use LINZ's method: the reference port's highs and lows shifted by the mean time differences, heights `MSL + (h_ref − MSL_ref) × ratio`. Between highs and lows the curve is LINZ's cosine curve. These are not official tide tables (Maritime Rules Part 25), and not for navigation.

## Fleet (YB race tracking)

Fleet drawer: type the race's YB name (as in `yb.tl/<race>`, e.g. `ggr2026`) and Open. The app fetches the race setup (`cf.yb.tl/JSON/<race>/RaceSetup`: boats, classes, course) and downloads the full tracks (`cf.yb.tl/BIN/<race>/AllPositions3`: every published fix of every boat with YB's distance to finish; re-fetched every 6 h or with **Full tracks**), then polls the light Expedition position file (`yb.tl/<race>-expedition.txt`, latest fix per boat) every 5 min to 3 h. Every fix is kept in `racenav-data\tracking\<race>\`, so tracks build up from what this app has downloaded. Boats are shown at their latest fix or where they were at the weather slider's time; ★ follows a boat; class filter; names (at every zoom; where names would overlap some are left out, followed boats first); YB course. The Fleet table in the dock ranks by distance to finish along the YB course, with SOG/COG made good between fixes and distance in the last 24 h. **Copy course** puts the YB course in the Course panel; **Route** on a boat starts the routing from its last fix and time (dropping YB course points it has passed when the Course panel holds the YB course).

## Own boat

Rail, bottom: **Centre** puts the boat in the middle of the chart. Chart drawer, Own boat: a heading line (solid; boat speed through the water, else SOG) and a COG line (dashed orange; SOG), each on or off, each running for a set time (1 min to 48 h) or to the edge of the chart. They are rhumb lines, so straight on the chart. Damping: moving averages of the Signal K readings (course and speed; wind; depth) over 0–60 s, with angles averaged as vectors; position is never damped. The boat symbol, its lines and the instruments all use the damped values.

## Measuring

**Cursor** instrument tiles (Instruments: add them to any panel; a Cursor panel is added once to existing layouts): the cursor's position, and the bearing (°T) and range from the boat to the cursor. On a touch screen a tap sets the cursor.

**Ruler** (left rail, above Centre): click to set the first point, click again for each further point, double-click to finish; the ruler stays on the chart (Esc stops early). Each leg shows its bearing (°T, and °M when Signal K sends the variation) and distance, with the total at the end; while drawing, the leg to the cursor and the running total follow the mouse. Distances and bearings are rhumb lines, so they match the straight lines on the chart. Points move like course marks: click one (amber ring), wait for the ring to turn green, then drag. Right-click a ruler: **Save ruler as route** (first point the start, the others new library marks; the route opens in the Course panel), **Delete ruler point**, **Delete ruler**, **Delete all rulers**. Rulers are kept between sessions.

## Instruments

Floating panels of readings, always on top. Drag a panel by its grip; double-click or double-tap the grip to fold the panel to a small title (again to open; Fold all in the drawer); tap a reading to change it (navigation, wind, route/waypoint, routing targets), add one next to it, remove it, or change the panel's size and direction. Instruments drawer: add panels, lock the layout, reset. The layout is kept per PC. Readings from Signal K grey out after 10 s without data; depth turns amber below safety depth + 2 m and red below the safety depth. The tile states (`stale`, `warn`, `alarm`) are where user-set warnings and alarms will plug in.

- **GRIB at cursor** readings: TWS, TWD, gust, MSLP, wave height, period and direction, rain, air temperature, the GRIB time and name, for the GRIB shown on the chart at the cursor (or the last tap on a touch screen) and the slider time. A panel of them is added once to existing layouts.
- **Show instrument panels** hides or shows them all; the tick beside each panel in the list hides or shows that one.
- **Time slider**: a floating bar (GRIB choice, step back/forward, play, Now, what the shading shows, the slider and time) that can be dragged anywhere, so every drawer can stay closed. On by default; × hides it, the Instruments drawer brings it back.
- **Wind arrows**: true wind (blue) and apparent wind (orange), pointing the way the wind blows with the tip on the boat, longer with more wind. With live Signal K data they sit on the boat (from the instruments); planning (paused, or no live data) they sit on the cursor: true wind from the GRIB shown, apparent wind for the heading and polar speed of the active routing within 40 px of the cursor, else for a course from the cursor to the end of the nearest course leg (polar speed × efficiency).

## Front-end layout

- Rail + one drawer (`ui.js`): controls. Panes: Weather, Course (`route.js`), Routing (`routing.js`), Library, Boat (`boat.js`), Chart, Instruments.
- Dock (`dock.js`): wide views along the bottom, registered by name (`dock.register("compare", { title, render(body, tools) })`). Future: polar plot and tuning, routing timeline graphs, ensemble spread.
- Floating instrument panels (`instruments.js`) keep clear of the dock.
- Statistics (`stats.js`): `STATS` lists the compare columns; adding one there makes it available in the table and its ⚙ menu.

## Conventions

- WGS84 decimal degrees, south and west negative.
- Bearings in degrees true, distances in NM, speeds in knots, times in UTC.
- en-GB spelling in docs and UI text.
