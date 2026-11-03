# TweedPlot: product description

*Professional Marine Chart Plotter, from [TweedsMarine](https://tweedsmarine.nz). Draft for review, version 1.0.0-alpha.*

Items marked **[check]** are claims to confirm before publishing. Boxes marked **SCREENSHOT** are placeholders: each says what to capture. Capture at 1920×1080 where you can, day palette unless the note says otherwise, with a real position such as Nelson or Tasman Bay.

---

## In one paragraph

TweedPlot is a free, open-source chart plotter and passage planner for Windows. It combines vector charts, weather routing, live instruments, AIS, alarms and race tracking in a single window, and every part of it is also useful on a fishing boat or a cruising yacht. It was built for yacht racing, so it plans with the forecast as well as the chart. It keeps your data in plain files on your own PC, and it works with no internet connection once your data is downloaded.

> **SCREENSHOT 1 (hero image):** The Planning view of the whole window. Open on a recognisable coast (Tasman Bay). Chart with LINZ depth colours, a planned route and a routing result drawn over the top, wind shading from a GRIB behind it, the time slider at the bottom and one instrument panel floating. Day colours. This is the picture people will judge the product by, so tidy it: no stray panels, no error banners.

## Who it is for

- **Racing sailors** who want to route, compare routings and test them against the forecast.
- **Cruising sailors** who want a chart plotter and passage planner that handles tides and weather.
- **Recreational fishers and launch owners** who want sea temperature, currents, chlorophyll and fishing rules on the same chart as the depths.
- **Anyone who wants open software** they can inspect, fix and add to, licensed under the GNU GPL v3.

## What it runs on

- Windows 10 or 11, Intel or ARM.
- Reads live data from a Signal K server, or directly from NMEA 0183 over serial, TCP or UDP with its own multiplexer.
- No account, subscription or licence key.

---

## Feature list

### 1. Charts

- **LINZ New Zealand charts** drawn as vector tiles from LINZ open data: coastline, depth areas, contours, soundings, lights, buoys, beacons, dangers, restricted and caution areas. Colours follow the S-52 day palette.
- **Safety depth and safety contour**: set for your draft. Shallower water is shaded, the safety contour is drawn bold, and dangers deeper than it lose their symbol.
- **Display categories**: Base, Standard and All, as on an ECDIS, plus light sectors, names and soundings.
- **Official ENC cells (S-57) and encrypted S-63 charts**: load licensed charts with your own permits. The licence code in the app is only for S-63 charts, never for the software.
- **World coverage out of the box**: a bundled coastline and bathymetry, with optional downloads of the full GSHHG world coastline and the ETOPO 2022 detailed depths for any area you choose.
- **High-resolution depths**: optional 1 m LINZ coastal elevation depths where a survey has been flown.
- **Own soundings**: build a depth image from your own tracks and from TimeZero PBG and XYZ files. Set the depth range and a target depth with a three-pointer slider, to make fishing or anchoring ground stand out.
- **Boundaries and fishing rules**: LINZ maritime boundaries and Ministry for Primary Industries fishing rules and closures, with an optional alarm when you enter a closed area.
- **Layer control**: switch the world depths, detailed depths, high-resolution depths and world coastline on or off individually.
- **Optional online maps**: OpenStreetMap as a base map and OpenSeaMap seamarks, or your own tile server. **Off by default; while off, TweedPlot sends no map requests at all.** Tiles are cached on disk for offline use.

> **SCREENSHOT 2:** A harbour at high zoom (for example Nelson Haven) in Standard display, showing depth areas, lights with sectors, buoys and names. Day colours.

> **SCREENSHOT 3:** The same area in the three display levels (Base, Standard, All), shown side by side as one composed image, to explain the control.

> **SCREENSHOT 4:** The Chart drawer open with the Layers card expanded, showing Chart layers, Own soundings and Online maps. Include the Online maps privacy note.

> **SCREENSHOT 5:** The own-soundings colour card with a target depth set, over a coastline with plenty of soundings. Rainbow palette, target pointer visible.

> **SCREENSHOT 6:** The Online maps feature in use: OpenStreetMap on, seamarks on, over a place with no LINZ chart (for example Stanley, Falkland Islands). The attribution line at the bottom must be readable.

### 2. Weather, ocean and tides

- **GRIB viewer**: load many forecast runs at once. Wind, gusts, pressure, waves, currents, temperature and more, with barbs, shading, isobars, a time slider and replay.
- **Downloads**: GFS and other models, ocean data (MUR sea surface temperature, HYCOM currents, salinity and temperature at depth), Himawari-9 and NOAA chlorophyll, and Saildocs.
- **Forecast uncertainty**: compare models and runs, with a meteogram and a Verify dock to check a forecast against what happened.
- **Weather at the cursor**: point anywhere and read the forecast at the time on the slider.
- **Weather stations and buoys**: MetService, NOAA aviation weather METAR and NDBC buoys.
- **Tides**: LINZ tide predictions with secondary ports and tide curves.
- **Fishing tools**: temperature breaks, cold water at depth, chlorophyll, eddies, currents and salinity, with a manual chapter and worked examples.

> **SCREENSHOT 7:** The Weather drawer with a GRIB loaded: wind shading and barbs over the chart, the GRIB-at-cursor panel and the time slider.

> **SCREENSHOT 8:** A fishing view: sea surface temperature shading with a visible temperature break, current arrows and chlorophyll or an eddy, if you have good data. A real downloaded day is better than a synthetic one.

> **SCREENSHOT 9:** The tide curve for a port, and the meteogram dock open for the same spot.

### 3. Routing and passage planning

- **Weather routing**: an in-house isochrone router that takes the forecast, your boat's polar, tacking and gybing costs and currents into account, and avoids land and water shallower than your safety depth.
- **Polars**: read qtVlm `.csv` and `.pol` files, with a tool for making a power-boat polar.
- **Compare routings** side by side, run a **gate sweep** across departure times, and use **barriers** to keep a routing off areas you choose.
- **Robust decisions**: test a routing against different models and uncertainty.
- **Route planning**: marks, routes, legs, rounding and timing, with cross-track error and a mark you are steering to.
- **Measuring**: a ruler with rhumb-line and great-circle bearings and distances, which can be saved as a route.

> **SCREENSHOT 10:** A completed weather routing over wind shading, with the Compare routings dock open along the bottom.

> **SCREENSHOT 11:** The Route drawer with a planned route of several marks and the Navigate section at the top, showing bearing, distance and cross-track error to the next mark.

### 4. Live navigation and instruments

- **Live boat**: position, course and speed, heading and speed through the water, wind, depth and water temperature, from Signal K or direct NMEA.
- **NMEA multiplexer**: several serial, TCP and UDP inputs at once, a preferred source for each data group, and automatic failover with an alarm.
- **Follow mode with a look-ahead offset**: the chart follows the boat, and you can drag the boat to any place on the screen so more of the chart shows in the direction you are heading. It stays there until you move it.
- **Instrument panels**: build as many as you like, in any arrangement, and keep a different layout per PC.
- **Views**: Navigation, Planning and Instruments, each a different arrangement of the same screen.
- **Monitor windows**: open extra TweedPlot windows for other screens, each with its own view and colours, sharing live data, alarms and the library.
- **Colour palettes**: day, dusk, night red and night green.
- **Touch friendly**: large controls, long-press menus and optional on-screen zoom buttons.

> **SCREENSHOT 12:** The Navigation view in night-red colours, with the boat offset towards the bottom of the screen and the course line and heading line drawn ahead. Shows the follow offset and night palette together.

> **SCREENSHOT 13:** The Instruments view: a full screen of single readings with no chart.

> **SCREENSHOT 14:** Two monitor windows side by side on two screens, or a composed image of the same.

> **SCREENSHOT 15:** Settings › Connections showing the NMEA multiplexer with two or three sources and their status.

### 5. Alarms and safety

- **One Alarms drawer** for every alarm, as IMO bridge alert management asks.
- **Depth and clearance**: shallow water, safety contour ahead (from the chart's depth areas along your course), and air draft against bridges and overhead cables.
- **Navigation**: off track, approaching the next mark, and position lost.
- **Anchor watch** with a swing circle drawn on the chart.
- **Collision**: AIS closest point of approach and time to it, plus AIS-SART, MOB device and EPIRB alerts.
- **Closed areas**: an alarm when you enter an area closed to your fishing methods.
- **Man overboard**: one press, no confirmation. It marks the position, shows time since, bearing and range back, and a large instrument panel readable from the helm.
- **Drop mark**: one press to mark a position with the time, with no alarm.

> **SCREENSHOT 16:** The Alarms drawer open with one active alarm, and the red alarm banner across the top of the chart.

> **SCREENSHOT 17:** A man overboard in progress: the MOB panel, the line from the boat back to the MOB position and the banner.

### 6. AIS and targets

- **AIS targets** from Signal K and from the aisstream.io internet feed, merged by MMSI.
- **CPA and TCPA**, vectors, names and status, a target list and distress kinds from the MMSI.
- **Own fleet** and **YB race tracking**: follow a whole race fleet, replay it against the forecast, rank boats by distance to finish and start a routing from any boat's last position.

> **SCREENSHOT 18:** The chart with several AIS targets showing vectors and a CPA line, and the target list open.

> **SCREENSHOT 19:** YB race tracking for a race fleet, with a followed boat's track highlighted and the Fleet table in the dock.

### 7. Library and your data

- **Library** of marks, routes, saved routings, tracks and barriers, kept in one GeoPackage file that QGIS and other GIS software can open.
- **GPX import and export**, and a **track recorder**.
- **Plain files in one folder**: copy the data folder to another PC to move or back up everything.
- **Sharing**: libraries can be shared between PCs. **[check: describe the libshare feature as it is today]**

> **SCREENSHOT 20:** The Library drawer listing marks, routes and tracks, with the group names MOB and Drops visible.

### 8. Built-in manual

- A complete manual inside the app (F1 or the **?** on every drawer), with a reference, route-planning and weather-routing tutorials, a glossary, search and pictures drawn by the same code as the app. It works offline.

> **SCREENSHOT 21:** The manual open at a page with a figure, showing the tabs, Contents and the search box.

---

## Open source, privacy and licensing

- **Licence**: GNU General Public License v3.0 or later. Anyone can use, study, share and improve the software, but anyone who distributes a changed version must release it under the same licence with its source. The documentation is CC BY-SA 4.0.
- **Contributions** are welcome. See `CONTRIBUTING.md`. TweedPlot's name and logos are protected separately: see `TRADEMARKS.md`.
- **Privacy**: TweedPlot has no accounts, no analytics and no telemetry. **[check: confirm before publishing]** It only contacts the internet when you ask it to download data or turn on an online feature, and the online maps are off by default. The tile servers see your IP address and the area you view, as with any web map.
- **Data and credits**: LINZ (CC BY 4.0), OpenStreetMap (ODbL), Natural Earth, GSHHG, NOAA, ECMWF, HYCOM and others. Full credits are in the About box, the manual and `THIRD-PARTY-NOTICES.md`.
- **Safety**: TweedPlot is a planning and situational-awareness tool, **not a certified navigation system**. LINZ and OpenStreetMap data are not official ENCs, and forecasts are model output. Always keep a lookout and carry official charts.

> **SCREENSHOT 22:** The About box (click the TweedPlot logo) with the Data sources and credits section expanded.

---

## Suggested ordering of the screenshots

1. Hero (1), then charts (2, 3), weather (7), routing (10) and live navigation (12): the five that tell the story.
2. The rest are for the feature pages and the manual.

## Open questions for you

1. This is a general product page; the GitHub README and tester handout are separate documents.
2. Is the product name always **TweedPlot**, with "Professional Marine Chart Plotter" as a strapline?
3. The items marked **[check]** need your confirmation. I did not read the library sharing or telemetry code for this draft.
4. Should the ENC and S-63 section name any chart suppliers? I left them out.
