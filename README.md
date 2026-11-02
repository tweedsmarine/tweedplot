# TweedPlot

**Professional marine chart plotter, passage planner and weather router for Windows.** Free and open source, from [TweedsMarine](https://tweedsmarine.nz), Nelson, New Zealand.

<!-- Add a hero screenshot here: docs/images/hero.jpg -->

TweedPlot puts vector charts, forecast weather, ocean data, weather routing, live instruments, AIS, alarms and fleet tracking in one window. It was built for yacht racing and works as well for cruising, fishing and powerboat use. Your data stays in plain files on your own PC, there are no accounts or subscriptions, and once your charts and forecasts are downloaded it works with no internet connection.

> **Not a certified navigation system.** TweedPlot is a planning and situational-awareness aid. Charts and forecasts can be wrong or out of date. Keep a lookout and carry official charts.

## Features

**Charts**
- LINZ New Zealand vector charts with S-52 colours, safety depth and contour, Base / Standard / All display and light sectors.
- Official S-57 ENCs and encrypted S-63 charts with your own permits.
- Worldwide coastline and depths (GSHHG, Natural Earth, ETOPO 2022), 1 m LINZ high-resolution depths, boundaries and fishing rules.
- Own soundings: build a depth picture from your tracks and TimeZero PBG/XYZ files, with a target-depth highlight.
- Optional OpenStreetMap and OpenSeaMap layers, **off by default and sending no traffic while off**, cached for offline use.

**Weather, ocean and tides**
- GRIB viewer for many runs at once, with a time slider, comparison between models, a meteogram and forecast verification.
- Downloads of GFS, ECMWF AIFS and other models, and ocean data: sea surface temperature, currents, salinity and temperature at depth, chlorophyll.
- Weather stations and buoys, LINZ tide predictions.
- Fishing tools: temperature breaks, eddies, chlorophyll and currents on the same chart as the depths.

**Routing and planning**
- Isochrone weather routing with polars, currents, tack and gybe costs, and avoidance of land and shallow water.
- Compare routings, sweep departure times, barriers and robust decisions across models.
- Routes, marks, rulers, GPX import and export, all in a GeoPackage library that QGIS can open.

**Navigation and instruments**
- Live data from Signal K or the built-in NMEA 0183 multiplexer (serial, TCP, UDP) with source failover.
- Follow mode with a look-ahead offset: drag the boat anywhere on screen and the chart keeps it there.
- Floating instrument panels, Navigation / Planning / Instruments views, extra monitor windows, day, dusk and night colours, touch support.
- Alarms for depth, safety contour, air draft, off track, anchor, collision (AIS CPA), closed areas and lost position. One-press man overboard.
- AIS from Signal K and aisstream.io, and YB race tracking.

A full manual is built into the app (F1) and works offline.

## Download

Windows 10 and 11, Intel and ARM. Installers are on the [Releases](../../releases) page. They are not yet code-signed, so Windows SmartScreen may warn you the first time: choose **More info › Run anyway**.

You provide your own chart data: see the manual's *Getting started* and *Charts* sections for downloading LINZ data and loading ENC or S-63 cells.

## Build from source

You need Rust, Node.js and the Tauri prerequisites for Windows. See [SETUP-WINDOWS.md](SETUP-WINDOWS.md), then:

```
cargo test
cd app
npm install
npm test
npm run tauri dev
```

Architecture, crate layout and data details are in [DEVELOPING.md](DEVELOPING.md).

## Contributing

Bug reports, ideas and pull requests are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) first.

## Licence

- Code: [GNU GPL v3.0 or later](LICENSE), with a [plugin exception](LICENSE-PLUGIN-EXCEPTION.md): separate plugin programs that talk to TweedPlot only through its plugin interface may be under any licence. If you distribute a modified version you must release it under the same licence, with its source.
- The shared crates `crates/enc-s57` (S-57 reader and converter) and `crates/plugin-api` (the plugin interface): MIT or Apache-2.0.
- Manual and documentation: CC BY-SA 4.0 ([LICENSE-DOCS.txt](LICENSE-DOCS.txt)).
- The TweedPlot and TweedsMarine names and logos are not licensed for modified versions: see [TRADEMARKS.md](TRADEMARKS.md).
- Third-party data and libraries (LINZ, OpenStreetMap, Natural Earth, GSHHG, NOAA, ECMWF and others): see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) and [NOTICE.md](NOTICE.md).

Encrypted (S-63) charts are not handled by TweedPlot itself but by the **TweedPlot S-63 Charts** plugin, TweedsMarine's separately licensed program, installed from Chart › Encrypted charts (download or package file). Unencrypted S-57 ENCs (NOAA and others) and the LINZ data need no plugin.
