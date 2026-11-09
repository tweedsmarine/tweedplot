# Quick start

TweedPlot plans and sails passages: charts, weather, routing, AIS, alarms and fishing tools in one program. This guide covers where to find things. The full manual is one click away: **F1**, or {icon:manual} **Manual** at the bottom left.

## The screen

- **The rail** (left edge): the drawers. {icon:weather} Weather, {icon:course} Route, {icon:routing} Wx routing, {icon:fleet} Targets & fleets, {icon:library} Library, {icon:chart} Chart, {icon:inst} Instruments, {icon:alarms} Alarms. {icon:settings} Settings is at the bottom. Click one to open it; click it again (or **Esc**) to close it.
- **The toolbar** (top left of the chart): {icon:north} north up, live or paused, {icon:follow} follow the boat, {icon:ruler} ruler, the **View** list, {icon:panels} panels, {icon:day} colours (day, dusk, night).
- **On the chart**: **MOB** (top right). Bottom right: **Drop** (a mark at the boat), and above it the optional anchor button (Settings › Display › **Anchor button on the chart**).
- **Right-click the chart** (or hold a finger on it): move the boat, start a route, new mark, drop anchor, go to, meteogram, ruler… Most things start here.
- **The ?** in a drawer's title bar opens the manual at that drawer.

## First start

1. **Welcome**: choose what the boat is mostly for: **Cruising**, **Fishing** or **Racing**. It sets up the toolbar and the Instruments screen. You can change it later in Settings › Display › Use.
2. {icon:settings} **Settings › Vessel**: name, **MMSI** (so your own AIS doesn't show as a target), **draft**, **length**, **air draft**, and where the GPS antenna is (metres from the bow). Press **Save vessel details**.
3. **Settings › Connections**: where the live data comes from: a **Signal K** server, or **NMEA 0183** straight from a COM port, TCP or UDP. The live-data light in the toolbar turns green when a position arrives.
4. {icon:chart} **Chart › Safety**: the **safety depth** (draft plus a margin) and the **safety contour** (drawn bold).

## Charts

Everything for the chart is in {icon:chart} **Chart**.

- **Chart › Layers** (it opens there): what's on the chart, in one list. Tick or untick a layer. **⚙** opens its settings; **Get…** means its data isn't here yet.
- **Chart › Get data**: every download and import.
  - **NZ charts (LINZ)**: download the GeoPackage zips from the LINZ Data Service (free account) and put them, unopened, in `tweedplot-data\linz`.
  - **World coastline**: **Download** (about 150 MB, once).
  - **High-resolution depths** (LINZ, 1 m, where surveyed): zoom in to the area, **Get for chart view**.
  - **NIWA bathymetry** (250 m, the whole NZ region): download the zip from NIWA, then **Import…**.
  - **Official ENCs** (S-57) go in `tweedplot-data\enc`; encrypted S-63 charts need the plugin: Chart › Encrypted charts.
- **Base / Standard / All** at the top of Layers: how much the chart shows, as on an ECDIS.
- Click a light, buoy or area for its details.

## Soundings

Your own depth soundings build up into a seabed, as on Olex or TimeZero.

- **Record soundings**: the toolbar button with a dot (Fishing use; **D** in any use). The dot turns red and counts while it records. Click again to stop.
- **Chart › Get data › Own soundings**: **Add my tracks** (the depths logged along your tracks), **Import PBG / XYZ…** (TimeZero files), colours, depth range, contours.
- **Show them**: Chart › Layers › **Own soundings**, or the seabed button on the toolbar.
- They're reduced to chart datum with the LINZ tides, and stay on this computer.

## Weather (GRIB files)

{icon:weather} **Weather**:

1. **Download GRIBs (free)**: choose the **model**, the **area** (draw it, the chart view, or round the route), the forecast length and the time step, then **Download**. Away from the internet, **Saildocs email request** writes the email for you.
2. Choose the file at the top, **Load**, and move through time with the slider (or **[** and **]**).
3. **Show** chooses the shading (wind, gusts, rain, waves, sea temperature…); barbs, isobars and arrows are in Chart › Layers › Weather and ocean.
4. Right-click the chart › **Meteogram here** for the forecast at one place.

**Tides and weather stations** are layers too (Chart › Layers › Tides and observations); download the LINZ tide tables in Weather › Tides once a year.

## Marks and routes

- **A mark**: right-click the chart › **New library mark here**, or Library › Marks › **+ Add mark**. Marks are kept in the {icon:library} **Library**, with routes, tracks, trips, the catch log and barriers.
- **Find one**: the **Find** box in its Library tab. **Filters** narrows the list (group, type, dates, trip…); filters in use always show, with **✕** to clear them.
- **A route**: {icon:course} **Route › Plan**: **Set start**, then **Add marks** and click the chart (or library marks). Choose how each mark is passed, name it and **Save**.
- **Sail it**: **Route › Navigate**: **◀ ▶** pick the next mark; the instruments show bearing, distance and cross-track error.
- **Go to here** (right-click): a single mark to steer to, with laylines.

## Weather routing

1. Choose the boat's **polar** in Settings › Vessel › Performance (Wx routing shows which one it uses).
2. Load a GRIB, make a route (start and marks).
3. {icon:routing} **Wx routing › Run › Compute**. The fastest track draws on the chart with its isochrones; **Save** keeps it.

## AIS and alarms

- {icon:fleet} **Targets & fleets › AIS**: AIS from your receiver arrives through Signal K or NMEA; **online AIS** (aisstream.io or Open Waters, free) fills in round you. **T** opens the target list.
- {icon:alarms} **Alarms**: shallow water, the safety contour ahead, off track, the next mark, air draft, AIS collision risk, closed fishing areas. Alarms sound until **Silenced**; silenced ones stay listed while their cause lasts.
- **MOB** marks the position at once and shows bearing and distance back to it.

## Anchoring

1. Right-click › **Drop anchor here** (or **K**, or the anchor button if you've turned it on). It goes at the bow, with its swing circle.
2. During the pull test, **drag the anchor** to where it lies: the readout says how far it is from the bow. Or type "30 m at 012°T from the bow" on its card.
3. On the card: the **swing radius** (or the scope and **Radius = scope**), the bottom, how it held, notes. The **anchor alarm** sounds if the bow leaves the circle; choose its sound in Alarms › Sound.
4. **Weigh anchor** (on the card, right-click, or the red anchor button pressed twice) ends the watch. The anchor stays in Library › Marks › **Anchorages** for next time: click it › **Anchor here again**.

## Useful keys

| Key | Does |
| --- | --- |
| F1 | Manual |
| ? | Every key |
| W  R  G  L  C  I  A  S | Open a drawer: Weather, Route, Wx routing, Library, Chart, Instruments, Alarms, Settings |
| B  /  F | Centre on the boat / follow it |
| M | Ruler (measure) |
| T | Target list |
| N | Colours (day, dusk, night) |
| E | Drop a mark at the boat |
| D | Record soundings on or off |
| K | Drop anchor (twice: weigh it) |
| Ctrl+M | Man overboard |
| Shift+A | Silence the alarms |

TweedPlot is an aid to navigation. It doesn't replace official charts, a proper lookout or good seamanship.
