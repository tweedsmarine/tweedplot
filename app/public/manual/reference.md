# TweedPlot User Manual

**TweedPlot: Professional Marine Chart Plotter**, from [TweedsMarine](https://tweedsmarine.nz), Nelson.

TweedPlot plans and sails passages on a Windows PC: charts, weather, routing and live instruments in one app. This tab is the reference; the tutorial tabs teach route planning and weather routing step by step.

## About this manual

This manual is for anyone who plans a passage before they leave the dock: racing and cruising sailors, and recreational fishers and launch owners running power boats. The app was built for yacht racing, but most of it (charts, tides, weather, marks, measuring, instruments) serves every boat equally.

**How it is laid out**

| Part | What it covers | Read it when |
| --- | --- | --- |
| This tab (reference) | Every panel and tool, what it does and how to use it | You want to know what a button does |
| Route planning (tutorial) | How to plan any passage: appraisal, hazards, tides, waypoints, contingencies, with worked examples for a cruise, a fishing trip and a race | You are planning a trip |
| Weather routing (tutorial) | Forecast models, GRIB files, uncertainty, polars, how a router thinks, and how to read and test its answer | You want the weather to choose or check the route |
| Glossary | Terms used in the app and the tutorials | A word is unfamiliar |

**Words used in this manual**

- *Drawer*: the panel that slides out from the icon rail on the left.
- *Dock*: the wide panel along the bottom of the chart for tables and graphs.
- *GRIB*: a weather forecast file (see Weather).
- *Routing*: a computed fastest route through the weather; *route*: your planned list of marks.
- Bearings are true (°T) unless shown as °M; distances are nautical miles (NM); speeds are knots (kn).

**Safety**

TweedPlot is a planning and situational-awareness tool, not a certified navigation system. The charts come from LINZ open data and OpenStreetMap and are not official ENCs; forecasts are model output, not warnings. Keep a proper lookout, carry official charts and publications, check MetService forecasts and warnings before and during every trip, and use your judgement. A routing is a suggestion based on a forecast that will be wrong in places.

**This manual inside TweedPlot**

The manual is built into the app, so the copy you read always matches the version you run, with or without an internet connection. Press **Manual** at the bottom of the rail, or **F1**. The **?** in each drawer's title bar opens the manual at that drawer's section. In the manual, the tabs along the top switch between the reference, the two tutorials and the glossary; **Contents** lists the sections of the open tab; the search box finds a word in every tab; click a picture to see it full size; **Wide** gives the manual most of the window, and **×** or **Esc** closes it.

## Getting started

TweedPlot runs on Windows 10 or 11 (Intel or ARM) and keeps all its data in plain files in one folder, so it can be backed up or copied to another PC.

**Install**

1. Run the TweedPlot installer. (During development it is built from the racenav source; the README explains how.)
2. Create the data folder `C:\dev\tweedplot-data` with the sub-folders below, or let TweedPlot create them as it needs them.
3. Start TweedPlot. The chart opens on Nelson; the Weather drawer is open.

**The data folder**

| Folder | Holds | Notes |
| --- | --- | --- |
| `linz` | LINZ chart data (the zip files as downloaded) | See Charts for which datasets to download |
| `grib` | Weather files, downloaded or copied in | qtVlm's GRIB folder is read too |
| `polars` | Your boat speed tables (polars) | See Polars, under Your boat. Without `tweedplot-data` they live in TweedPlot's own data folder |
| `library` | Marks, routes, barriers, saved routings, tracks, GPX exports | Plain JSON: safe to back up and copy |
| `tides` | LINZ tide tables | Downloaded from the Chart drawer |
| `tracking` | YB race tracking data | One sub-folder per race |

Advanced: the environment variables `RACENAV_GRIB_DIR`, `RACENAV_POLAR_DIR`, `RACENAV_LIBRARY_DIR` and `RACENAV_SK_URL` point TweedPlot elsewhere.

**First launch checklist**

- [ ] Chart and Signal K drawer: zoom in on New Zealand to check the LINZ charts show, and set your **safety depth** and **safety contour**.
- [ ] Boat drawer: choose a polar (sail) and set efficiency and tack/gybe times.
- [ ] Weather drawer: download or load a GRIB.
- [ ] Instruments drawer: arrange the panels you want; the layout is kept per PC.
- [ ] If you have instruments: start your Signal K server (or connect your NMEA source); the Signal K button in the toolbar at the top left turns green.

**The screen**

The chart fills the window. Everything else floats over it or slides out from the edges, so any of it can be hidden. 

![The main window: Weather drawer open, an ECMWF AIFS forecast over the top of the South Island, GRIB and cursor panels floating, time slider at the bottom](img/01-main-weather.jpg)

![screen layout · rail, drawer, floating panels, dock](fig/layout.svg)

Drawers slide out from the rail; instrument panels, the time slider and the dock float over the chart and can each be hidden.

| Where | What | How to use it |
| --- | --- | --- |
| Left rail | Weather, Course, Routing, Fleet, Library, Boat, Chart, Instruments; Ruler and Centre at the bottom | Click an icon to open its drawer; click again or press Esc to close it. Only one drawer is open at a time |
| Top left | The toolbar, a column of buttons: **North up**, Signal K status, **Live / Paused**, **Follow**, **View**, **Panels**, **Chart**, **Day / night colours**, **Clear track**. It moves along when a drawer opens | Paused = planning: the boat stays where you put it. See *Views and monitor windows* under Instruments |
| Top right | **MOB** (man overboard), alone, the same size and place in every view and every window | One press, no confirmation. See *Alarms and man overboard* |
| Bottom right | **Drop**: marks the boat's position with the time | No alarm, nothing else changes |
| Bottom centre | **Zoom out** and **Zoom in**, two round buttons well apart | Shown on touch screens only, unless you change it under *Screen controls* in Settings |
| Floating panels | Instrument panels and the time slider | Drag by the grip (⋮⋮); double-click the grip to fold a panel |
| Bottom | The dock: Compare routings, Gates, Fleet, Meteogram, Verify | Opens from buttons in the drawers; drag its top edge to resize |
| Bottom left | Cursor position | Also in the Cursor instrument |
| Right-click | The chart menu | Start or add marks, move the boat, meteogram, ruler and barrier actions |

**Mouse and touch**

- Drag to pan, scroll or pinch to zoom, right-drag or two-finger twist to rotate (the **North up** button at the top of the toolbar turns the chart back to north at the top; its needle shows which way north is).
- On a touch screen a tap sets the cursor (there is no hover), and a long press opens the chart menu.
- To move a course mark, library mark, ruler point or barrier point: click it (amber ring), wait for the ring to turn green, then drag. The pause stops marks being moved by accident. A library mark's new position is saved straight away, and every route using it follows.

## Charts

TweedPlot draws its own vector chart from free data: a world coastline everywhere, and for New Zealand the LINZ hydrographic data with depth areas, contours, soundings, lights, marks, dangers and charted areas.

**ENC cells (S-57)**

TweedPlot also reads official electronic navigational charts in the IHO S-57 format: a base cell (`.000`) and its updates (`.001`, `.002`, ...). Put the cells in `tweedplot-data\enc`, in any folders you like; a whole exchange set (`ENC_ROOT` with its agency, cell, edition and update folders) can be copied in as it is. Each cell is converted once into the `.enc` folder beside it, and again when the cell or an update changes, then drawn in the same style as the LINZ charts. The cell's intended usage sets its scale band: overview, general, coastal, approach, then harbour and berthing.

Updates are applied in order. The cells of each scale band are merged into one file in `.enc`, so even thousands of cells (all of NOAA's charts, say) draw quickly.

**Encrypted ENCs (S-63).** Most official ENCs are sold encrypted under the IHO S-63 scheme. You buy permits for the cells you want, made for your system, and the charts come as exchange sets (a folder with `ENC_ROOT` in it, on a CD, DVD or download). Everything is done in **Chart › Encrypted charts**:

- **Activation**: encrypted charts are licensed to one computer. **Copy** the **Installation code** (it starts TP1-, made from this computer's main board) and send it to Tweedsmarine; paste the **Activation code** that comes back (it starts TPA1-) and press **Activate**. It works only on this computer: a second PC on the same boat gets its own activation code, with the same user permit, so the same chart permits work on both. A new main board needs a new activation code.
- **User permit**: this computer's identity for the chart suppliers, shown once activated. **Copy** it and give it to your chart supplier when ordering permits.
- **SA key**: the IHO Scheme Administrator's key, which proves where cells come from. TweedPlot comes with it (valid to 2033). **Install key…** takes a newer `IHO.CRT` or `IHO.PUB` from the IHO website or your supplier. A key that isn't the IHO's is installed with a warning (SSE 26); an expired or damaged one is refused (SSE 22, SSE 08).
- **Install permits…**: choose the `PERMIT.TXT` from your supplier. Each permit is checked and the report says which installed, which are wrong (SSE 12, SSE 13) and which expire within 30 days (SSE 20) or have expired (SSE 15). Permits are kept separately for each data server; **Cell permits** lists them, and **Remove** (press twice) takes away one data server's.
- **Import exchange set…**: choose the exchange set's folder, or the medium if it holds several. Every cell file is checked (signature, permit, decryption, CRC) and the base cells, new editions, re-issues and updates are applied in order. Cancelled cells are removed, with their replacements named. A report lists every cell with what happened and the S-63 code for anything wrong. When an update needs base media you haven't loaded, it says which (load them, then the update again).
- **Installed ENCs** lists each cell with its edition and update. **Remove all ENCs** clears them (permits and key stay).

While a cell whose permit has expired (SSE 25), or that is behind its data server's latest edition or update (SSE 27), is on the chart, a warning stays at the bottom of the chart: such a cell may be out of date and must not be used for primary navigation. The chart data made from encrypted cells is kept on disk encrypted for this computer and opened only in memory, as the S-63 licence requires. Until TweedPlot has its own manufacturer credentials from the IHO it uses the IHO's generic test credentials, so only the IHO test data can be opened. An exchange set copied straight into `tweedplot-data\enc` with its `PERMIT.TXT` is still read as before.

**Getting the New Zealand data**

Download from the [LINZ Data Service](https://data.linz.govt.nz) (free account) as GeoPackage, and put the zip files, unopened, in `tweedplot-data\linz`. TweedPlot reads every scale band it finds and draws the most detailed one available at each place.

| Dataset | Why |
| --- | --- |
| Depth area polygons (each scale band: 1:4k–1:22k, 1:22k–1:90k, 1:90k–1:350k, 1:350k–1:1.5M, 1:1.5M and smaller) | Water depth shading and the routing safety depth |
| Depth contours and soundings (same bands) | Contours, the safety contour, spot depths |
| NZ coastlines and islands (Topo50) | The detailed coastline |
| NZ lake polygons (Topo50), and optionally NZ river polygons (Topo50) | Lakes and wide rivers as water. The coastline layer is land right across them, so without these Lake Taupō, for example, shows as land under its rocks and marks. Where a lake is charted (Taupō is) its depth areas, contours and soundings show; elsewhere it is plain water. The lake and river areas in the hydrographic feature downloads are used too |
| Hydrographic feature layers (the "121 layers" GeoPackage for each band) | Lights, buoys, beacons, rocks, wrecks, obstructions, marine farms, anchorages and other charted areas |

The open data does not cover every harbour at large scale. Port Nelson's inner channel marks, for example, are only in the official ENC. Add missing marks yourself as library marks with buoy symbols (see Marks).

Wharves, breakwaters, jetties, pontoons, floating docks and causeways are drawn from LINZ's shoreline construction and pontoon layers (in the hydrographic feature downloads) where LINZ has them; click one for its type. Many marinas, Nelson's included, have no pontoons in the open data: those are only in the official ENC.

**Radar overlay test** (Chart drawer, at the bottom): a simulated radar scanner, there to prove that this PC can draw a real radar overlay smoothly before radar support is built. It makes echoes in the app's core at a real scanner's rate (24 or 48 rpm, up to 4096 spokes of 1024 samples), sends them to the chart as raw bytes and draws them with the graphics card round the boat (or the chart centre): two islands, a coast to the south, a rain cell, eight moving ships and sea clutter. The numbers underneath show frames per second, data rate, how long the chart spends taking the data in, and any backlog. Smooth means 50–60 frames a second while you pan and zoom, with no backlog building up.

**Chart and Signal K drawer: chart settings**

| Setting | What it does |
| --- | --- |
| **Safety depth (m)** | The least depth you will sail in: the draft plus a margin for swell, squat and chart error. Routing will not enter water shallower than this |
| **Safety contour (m)** | The contour drawn bold. Water shallower than it is shaded darker blue, as on a paper chart. Usually the next charted contour at or deeper than the safety depth |
| **Route round dangers** | Routing keeps off rocks, wrecks and obstructions that dry, are awash, or have less water than the safety depth or an unknown depth |
| **Route round marine farms** | Routing keeps out of charted marine farms |
| **Lights, marks, dangers, areas** | Shows the hydrographic features, with **Light sectors** and **Names and labels** on or off |
| **Chart symbol size** | Scales buoys, beacons, lights, dangers and other chart symbols from 75% to 200% (default 120%) to suit the screen and the viewing distance. Labels grow at half the rate, so they stay clear of each other |

Underwater dangers deeper than the safety contour are shown with their depth only, so the chart stays readable while anything that matters to your boat stands out.

**Hazards are never hidden by zoom.** Rocks, wrecks, obstructions, piles, platforms, marine farms, hulks, bridges and overhead cables and pipelines stay on the chart at every zoom, even with chart features switched off, and even where the chart's own scale-minimum setting (SCAMIN) would hide them. This follows the ECDIS performance standard, which puts isolated dangers in the "display base" that can't be removed, and learns from groundings such as Team Vestas Wind's on the Cargados Carajos Shoals in 2014, where a reef did not show on a racing navigator's chart display at the zoom he was using. Zoomed well out, where symbols would pile up, they are thinned to one every few pixels, keeping the most dangerous (drying and awash first, then the shallowest), and a farm or obstruction too small to see is also drawn as its symbol. Zoom in to see each one. Underwater dangers deeper than your safety contour are the exception: they appear from the scale the chart says, as their depth.

**Port entry information shows early.** Pilot boarding places, radio calling-in (reporting) points and lines, signal stations and rescue stations show from about 1:500,000 (zoom 9), well before their charted SCAMIN, which often hides them until the harbour chart. That way they can be found while planning an approach to an unfamiliar port.

**Where the chart scales meet.** LINZ charts come in scale bands, and the same farm, cable or rock is often drawn differently in each (a 1:190,000 chart may show one big farm block where the 1:122,000 chart shows the separate farms). TweedPlot uses each band's coverage, as an ECDIS does: inside the area a larger-scale chart covers, its features are the ones shown, and a smaller-scale chart's features are cut back to outside it. So a feature can change shape at a chart boundary, but never at the edge of a map tile, and nothing comes and goes as you zoom except where the charts themselves differ. Click a light, mark, danger or area for its details: light characteristics, buoy shape and colours, depth over a rock, area restrictions. 

![The popup for the Schnapper Point light at the Nelson harbour entrance: its characteristic Fl(4) R 20s 13m 8M first, then the structure and each sector, with the sector arcs drawn round the light](img/02-light-popup.jpg)

At small scales the world coastline (GSHHG) is drawn, and east of 180° (the Chathams, for example) works the same as west of it.

**Chart data: world depths and coastline**

Outside New Zealand, TweedPlot shades the sea by depth from free world data. **Chart and Signal K › Chart data** shows what is installed (a green dot) and downloads the rest.

| Data | How you get it | What it shows |
| --- | --- | --- |
| **World depths** (Natural Earth) | Comes with TweedPlot | Depth shading at 0, 200, 1000, 2000 … 10000 m, everywhere |
| **World coastline** (GSHHG) | **Download**, once, about 150 MB | Every island, at every scale. Until then a coarser outline is drawn. Coastline detail (Standard, More, Most) brings GSHHG's finer levels in when zoomed further out: Most draws the full-resolution coast from zoom 4 instead of zoom 8, a little slower to draw |
| **Detailed depths** (ETOPO 2022) | **Get for chart view**, for the area on screen, typically 1–5 MB | Finer depth bands from zoom 4 and contours (5 m to 5000 m) from zoom 7 |
| **NZ charts** (LINZ) | **LINZ…** opens the LINZ Data Service (see above) | The full hydrographic chart |
| **High-resolution depths** (LINZ coastal, 1 m) | **Get for chart view**, zoomed in to the area, 5–40 MB a tile | Shaded depths with contours from about 5 NM range in, where LINZ has flown a survey (see below) |
| **Maritime boundaries** (LINZ) | **Download**, under 1 MB | Territorial sea (12 NM), contiguous zone (24 NM), EEZ (200 NM) and the boundaries agreed with Australia (see Boundaries and fishing rules) |
| **Fishing rules** (MPI) | **Download**, several MB; **Update** before a trip | MPI's commercial fishing regulations, customary areas, voluntary closures and fisheries areas (see Boundaries and fishing rules) |

For detailed depths, zoom the chart to the area you want, pick the **Detail** (15″ is about 460 m between points) and press **Get for chart view**. The line under it gives the download size; a large area is thinned automatically, so zoom in for more detail. Each area you have downloaded is listed with **Show** and **✕** (delete). Downloads can be cancelled, and the chart redraws when one finishes.

**High-resolution depths.** LINZ publishes 1 m elevation models of the coast from aerial laser surveys (lidar), which reach the seabed down to about 25 m in clear water. Where they exist they show far more than the chart: channels, banks, bars, reefs and the shape of the seabed in harbours and anchorages. Zoom in to the area (the line under the row says how many tiles cover the chart view and how many are downloaded), press **Get for chart view**, and the depths are drawn from about 5 NM range in, shaded as if lit from the north-west, with contours every 10 m, 5 m, 2 m or 1 m as you zoom in. **Show** turns them on and off; **✕ All** deletes them. Each tile covers 4.8 × 7.2 km; up to 60 tiles are fetched at a time.

The heights are above NZVD2016, about mean sea level, not chart datum: the depths shown are deeper than the charted depths by roughly half the tidal range, so use them for the shape of the bottom, not for clearance. Above 0 m nothing is drawn (the chart shows the land and the drying areas). Surveys were flown in different years, and sand moves. Not for navigation.

World depths and ETOPO depths are for orientation only. They are coarse, or interpolated near the coast, and routing does not use them: the routing safety depth works only on LINZ depth areas. Not for navigation.

Sources and licences: Natural Earth (public domain); GSHHG, Wessel and Smith, University of Hawaiʻi and NOAA (LGPL 3, downloaded from its home site); ETOPO 2022, NOAA NCEI (free to use and redistribute), fetched through NOAA CoastWatch; LINZ hydrographic data, Toitū Te Whenua LINZ (CC BY 4.0).

**Boundaries and fishing rules**

Download both in **Chart and Signal K › Chart data**, then choose what to show in **Chart and Signal K › Boundaries and fishing rules**.

- **Maritime boundaries**: the territorial sea limit (12 NM, dashed purple), the contiguous zone limit (24 NM, dotted), the EEZ limit (200 NM, dash-dot) and the boundaries agreed with Australia, for the mainland and the outlying islands, from LINZ.
- **Fishing rules**: MPI's maps of the commercial fishing regulations. Prohibitions are red with a solid edge, restrictions orange with a dashed edge, customary areas (mātaitai, taiāpure) purple and voluntary closures grey (MPI says their boundaries are approximate). Seasonal rules are shaded more lightly. The areas are drawn under the land: some of MPI's areas run inland.
- **Fisheries areas**: the fisheries management areas (FMAs) and statistical areas, with their codes, for catch reporting.

Click inside a rule area for its rules: what is restricted, the methods, the species, the vessels, the season, MPI's category (for example "Trawl prohibitions for vessels over 46m"), the regulation, the legal description and a button that opens the regulation.

**Only the rules for my boat.** Tick the methods you fish with (bottom trawl, midwater trawl, Danish seine, purse seine, beach seine or drag net, set net, longline, potting, dredge, troll, handline, hand gathering or diving). The chart then shows only the rules for those methods, plus rules that close an area to all fishing, customary areas, and rules that name no method (species rules: shown to be safe). The vessel length comes from **Settings › Vessel**: the trawl rules for vessels over 46 m are left out for a vessel known to be 46 m or shorter.

**Closed-area alarm.** With **Closed-area alarm** ticked, the main window checks every 15 seconds whether the boat is inside an area closed to the ticked methods (all prohibitions if none are ticked) and whether it will be within the time you choose at its course and speed over ground (5 to 30 minutes), and raises a warning naming the areas. It works on the downloaded rules: update them before a trip, because temporary closures come and go.

MPI's maps are a guide to the regulations, not the regulations: the legal descriptions and the regulations themselves are the law. Source: Ministry for Primary Industries, CC BY 4.0. Maritime boundaries: sourced from Land Information New Zealand data, Crown Copyright Reserved. Not for navigation.

**Tides**

Tick **Tide stations** and press **Download LINZ tides** once a year: TweedPlot fetches the LINZ tide tables for this year and next (about 90 standard ports plus the secondary ports). Tide stations then appear on the chart. Click one for the day's tide curve, with high and low times and heights, a marker at the current time (or the weather time when planning) and arrows to step through the days. Times are New Zealand time (Chatham Islands time for Chatham ports). Secondary ports are worked from their LINZ time and height differences. These are predictions, not official tide tables, and weather (wind and pressure) can raise or lower the actual tide by several decimetres.

Tidal streams are not shown yet. ECMWF 0.1° surface currents and HYCOM currents are forecast ocean currents; the available documentation does not establish these as tidal-current predictions. Enable **Use GRIB currents** in Routing to include the available fields in a route, but do not rely on them as tidal-stream guidance.

**Display colours: day, dusk and night**

The ☀ button in the toolbar at the top left (or **Chart and Signal K › Display colours**) steps through four sets of colours:

| Colours | Use |
| --- | --- |
| **Day** | Normal colours |
| **Dusk** | Dimmer, a little less colour |
| **Night (red)** | Everything as shades of red on black: chart, GRIB shading, panels and drawers. Keeps night vision best |
| **Night (green)** | The same in green, if you find it easier to read than red |

**Brightness** sets how bright dusk and night are. Turn the screen's own brightness down at night too: night colours dim what is drawn, not the backlight.

**Each window has its own colours.** A monitor window in a lit office can stay on day colours while the one on the fly bridge is on night red. Each window remembers its own setting.

**Night cursors.** In the night modes the mouse pointer is drawn in the night colour (an arrow, a ring over things you can click, a cross while drawing and a four-way mark for dragging the chart), because the system's white pointer would spoil night vision. The pointer is drawn at a fixed brightness; the resize pointers and the text caret stay as Windows draws them. A touch screen has no pointer.

With dusk and night colours the window's title bar turns dark as well. To get rid of it altogether, press **F11** (or **Full screen** under Display colours) to fill the screen; F11 again switches back. Monitor windows do the same with their own colours and F11.

## Your boat

TweedPlot shows your own boat from live instruments, or wherever you put it when planning.

**Signal K**

TweedPlot reads live data (position, heading, speed, wind, depth) from a [Signal K](https://signalk.org) server, normally running on the same PC. Signal K takes NMEA 0183 and NMEA 2000 from your GPS and instruments through a USB adapter or gateway and serves it to any number of apps. TweedPlot connects to `ws://localhost:3000` (set `RACENAV_SK_URL` for another address). The Signal K button in the toolbar at the top left shows the connection at all times: green when data is arriving, amber when connected but nothing has arrived for 10 seconds, red with a slash when there is no connection (blinking while it first tries). Hover it for the server address, the time of the last update and what is arriving; click it for the Signal K section of the Chart and Signal K drawer.

No instruments? A GPS dongle into Signal K is enough for position, course and speed. With no Signal K at all, TweedPlot works fully in planning mode.

**NMEA 0183 direct**

TweedPlot can also read NMEA 0183 itself, without Signal K: from a COM port (USB adapter or multiplexer), a TCP connection or UDP broadcasts. Add each source in **Settings › NMEA data sources**. When NMEA sources are on, they take over from Signal K for your own boat's data.

- **Several sources:** each kind of data (position, motion, heading, speed through water, wind, depth, water temperature) can come from a different source. If the chosen one goes quiet for longer than **Source stale after**, the next source that has the data takes over (**Automatic failover**), and an alarm says so.
- **Heading:** a true heading (HDT, or VHW with a true heading) is used when there is one. Otherwise the magnetic heading (HDG, HDM) is corrected with the variation from the compass or the GPS.
- **AIS:** AIS sentences from an AIS receiver or transponder on any source (`!AIVDM`) become AIS targets, the same as AIS through Signal K. Your own transponder's reports of your own ship (`!AIVDO`) are not shown as a target.
- **Live NMEA data** shows the sentences as they arrive and what was read from them, for checking the wiring.

**Replaying a log**

TweedPlot logs every Signal K session, AIS targets included, while it runs. **Chart and Signal K › Replay a log** lists the logs by start time, length and size; choose one and press **Replay**. The log then drives the boat, instruments, AIS targets and alarms as if it were live, and the live Signal K data waits until the replay stops. A bar at the top of the chart has pause, speed (1× to 300×), a slider to jump to any time, and **✕** to go back to live data. Tracks are not recorded from a replay. Use it to go over a race, test alarms and settings at the dock, or show someone what happened.

**Live and planning**

| Mode | Button (toolbar, top left) | The boat | Time used by the instruments and routing |
| --- | --- | --- | --- |
| Live | **Live** | Follows Signal K | Now (GPS time) |
| Planning | **Paused** | Stays where you put it: right-click > **Move boat here** | The weather slider time |

**Follow** keeps the chart centred on the boat (dragging the chart turns it off); **Centre** (rail) recentres once; **Clear track** clears the short trail behind the boat.

**Size and GPS antenna**

In the **Boat** panel, set the boat's **Length** and **Beam**, and where its **GPS antenna** is: metres aft of the bow, and metres off the centre line to starboard or port. Signal K gives the antenna's position, not the boat's centre, so on a 20 m boat with the antenna at the stern the bow is 20 m ahead of the position. When you zoom in far enough for the boat to be bigger than its symbol (harbour scales), it is drawn to scale as its outline, pointing along the heading (or the course over ground when there is no heading), with a dot at the antenna. Zoom out again and the symbol comes back.

**Lines and arrows on the boat**

- *Heading line* (solid) and *COG line* (dashed orange): each as long as a set time at the boat's speed, or to the edge of the chart. Set in Chart and Signal K > Own boat.
- *Wind arrows*: true wind (blue) and apparent wind (orange), each pointing the way the wind blows with its tip on the boat and longer as the wind gets stronger. With live data they sit on the boat. When planning (paused, or no live data) they follow the cursor instead, showing the GRIB's true wind there at the weather slider time. Near a shown routing they show that routing's nearest point instead: the true wind it had at that point's time, and the apparent wind you would feel on its heading and boat speed, matching the point's hover box. Away from a routing there is no boat to feel it, so only the true wind is shown. Expect the apparent wind well forward of the true wind whenever the boat is fast for the wind: at 7 kn on a broad reach (TWA 128°) in 9 kn of true wind, the apparent wind is about 77°, forward of the beam. That is why asymmetric spinnakers are trimmed for a beam reach in light air. Hover a routing point for the exact AWA and AWS. Turn them on or off in the Instruments drawer.

**Damping**

Wave motion makes heading, speed and wind jump about. Chart and Signal K > Damping averages them over a few seconds for the boat, its lines and the instruments. Position is never damped.

**Tracks and recording**

Library > Tracks > **Record** logs the boat's track from Signal K while TweedPlot runs, even while plotting is paused. It keeps a fix every 10 s while moving, sooner in a tack or gybe, and is saved every 30 s and when the app closes. **New log** finishes one track and starts another. Tracks can be shown, renamed, coloured, exported as GPX, and imported from GPX (from a plotter, Navionics, qtVlm and so on). Click a track for the time, speed and course at that point; tick **Boat at weather time** to see where the boat was at the slider time, which is useful for comparing a race track with the forecast.

**Polars**

A polar is the table of your boat's speed at each true wind angle (TWA) and true wind speed (TWS). Routing, the target speeds and the polar efficiency all come from it, so it is worth getting right. The weather routing tutorial explains what a polar is and why every polar is optimistic; this section is about the files.

*Where TweedPlot looks.* Boat > Polar lists two kinds:

| Kind | Where | Notes |
| --- | --- | --- |
| Yours | `tweedplot-data\polars`, or without `tweedplot-data` the `polars` folder in TweedPlot's app data | **Import polar…** copies a file here; **Open folder** opens it in Explorer. Anything you put here yourself appears too |
| Shipped | Installed with TweedPlot, marked *(shipped)* in the list | Replaced on every update: don't edit them. Copy one into your folder under the same name to use your version instead |

TweedPlot reads files ending `.pol`, `.csv` or `.txt`, and lists only those it can read. Import checks a file before copying it and says what is wrong if it can't read it. `RACENAV_POLAR_DIR` points your folder elsewhere.

*The format.* A plain text table, the same as qtVlm, OpenCPN, Expedition's text export and most other routers use:

```text
TWA\TWS	4	6	8	10	12	16	20
40	3.1	4.5	5.5	6.2	6.6	6.9	7.0
52	3.9	5.5	6.5	7.1	7.4	7.6	7.7
90	4.7	6.3	7.2	7.8	8.2	8.8	9.2
120	4.2	5.9	7.0	7.7	8.2	9.0	9.6
150	2.8	4.3	5.5	6.6	7.3	8.2	9.1
180	2.0	3.1	4.1	5.1	6.0	7.3	8.2
```

- **First line**: any label in the first cell (`TWA\TWS` by custom), then the true wind speeds in knots, increasing from left to right.
- **Each further line**: a true wind angle in degrees (0 = head to wind, 180 = dead downwind), then the boat speed in knots at each wind speed. Every line needs one speed per wind speed. The angles can be in any order; the same table serves both tacks.
- **Separators**: semicolons, tabs, commas or spaces. TweedPlot works out which from the first line. Use a full stop for decimals (`6.5`, not `6,5`).
- **Comments**: blank lines and lines starting with `#` are ignored, so you can note the source and date at the top.
- **The file name** is the polar's name in the list.

*Between and beyond the numbers.* Speeds between the table's angles and wind speeds are interpolated in a straight line both ways. Below the lightest wind speed the speed falls in proportion to zero at 0 kn (unless the table has a 0 kn column, as a power polar does). Above the strongest it holds the last column: TweedPlot never extrapolates upwards. Angles inside the first row (inside 40° in the example) are interpolated towards the nearest row; put a row of zeros at 0° (or at the angle the boat stops sailing, about 30° for most yachts) so the routing doesn't think the boat can point higher than it can.

*Where to get one.*

| Source | Notes |
| --- | --- |
| ORC certificate | The best start for most racing yachts. The certificate's speed guide is a VPP polar for that boat as measured; copy its table into a text file. The ORC site has certificates for many boats, including sister ships of yours |
| Designer, builder or class association | Often on request or in the owner's documents. Usually VPP results for a fully crewed boat with new sails |
| Another router | qtVlm, OpenCPN, Expedition, Adrena and the online sailing games ship or share polars for many production boats. Their quality varies: check against what you know of your boat |
| Sailmaker | Will often run a VPP for a customer, especially with a new sail plan |
| Your own logs | The most honest, once you have hours in a range of conditions. Recording tracks builds the data; building a polar from them is planned |

*Making or checking one.*

1. Start from the nearest you can find: your boat's ORC certificate, a sister ship's, or a similar boat's scaled by length.
2. Check the shape. Upwind speeds should peak in 12–16 kn and level off; the best upwind angle is usually 38–45°, the best downwind angle 140–170° depending on the boat and wind. Speeds that keep climbing in 30+ kn are a VPP with no sea in it.
3. Compare with the instruments: sail at a steady angle for a few minutes in steady wind, note TWS, TWA and boat speed, and compare with the table. Do this at several angles and wind speeds over a season.
4. If you are consistently a set fraction below, lower the efficiency in the Boat drawer rather than editing the table. Edit the table (in a spreadsheet, saved as CSV, or any text editor) where it is wrong in shape: a sail you don't carry, an angle the boat can't hold.
5. Keep the original. Save your version under a new name (`First40_7_ours.pol`) with a `#` line saying what you changed.

For a power boat, see *Making a power polar* in the Route planning tutorial.

## Instruments

Instrument panels are rows or columns of readings that float over the chart. Make as many as you like, put them anywhere, and keep a different layout on each PC (chart table, laptop, helm screen).

**Views and monitor windows**

A **view** is an arrangement of the same screen. The **View** button in the toolbar (top left) cycles through them:

| View | Shows |
| --- | --- |
| **Navigation** | The chart with the rail, drawers and dock tucked away, for use under way |
| **Planning** | The chart with the rail, drawers and dock, for routes and weather. The layout the app always had |
| **Instruments** | A screen of single readings, with no chart |

Every option is still available in every view; a view only decides what is shown to begin with. **Panels** and **Chart** (also in the toolbar) show or hide the rail, drawers and dock, and the chart, in the current view.

Each panel (group of instruments) has a name you can edit in the Instruments drawer (the text box in its list entry), and a tick for each view, which says whether the group floats over the chart in that view.

**Monitor windows.** **Instruments › New monitor window** opens another full TweedPlot window for another screen, showing any view you choose. Open as many as you like: the Navigation view on two bridge displays, the Instruments view at the helm, Planning at the chart table. Drag each window to its screen and press **Full screen** (F11).

- Each window has its own view, colours (day, dusk, night), size and position. Monitor windows reopen where you left them when TweedPlot starts.
- Live data, alarms, marks, routes and the library are shared. Any window can silence or accept an alarm, press **MOB** or **Drop**, set or clear the anchor watch, and pan and zoom its own chart. A mark or route saved in one window appears in every other within a moment, and the route being edited in the Course panel is shown and followed in every window. Keep the route editing to one window at a time, normally the Planning window.
- Closing a monitor window closes only that window. Closing the main window closes the whole program.
- Each window can be closed, minimised, maximised and restored with the normal window buttons.

**The Instruments view**

The Instruments view is one screen of tiles, each a single reading, on a grid of 12 columns by 8 rows. It uses the same readings as the floating panels (including the graphic Wind, Compass and Highway), but its tiles are not grouped.

- **Move**: drag a tile. A tile dropped under another locks to that tile's column; it does not shuffle to the left. Tiles snap to the grid and cannot overlap or leave the screen.
- **Resize**: drag the handle in a tile's bottom right corner. Text and dials scale to fill the tile.
- **Change a reading**: click (tap) a tile to choose what it shows, or remove it.
- **Instruments drawer › Add an instrument** puts a new tile in the first free space. **Reset Instruments screen** returns to the default arrangement.
- Night colours show red (or green) readings on black.

**Screen controls**

**Settings › Screen controls** is kept for each window:

- **Zoom buttons**: Auto shows them only where a touch screen is found. **Show** or **Hide** forces it. They are two round buttons at the bottom centre, well apart.
- **Large buttons**: Auto enlarges the toolbar, MOB, Drop and zoom buttons on a touch screen. **On** does it on any screen, for gloves.

**Working with panels**

- **Move**: drag the grip (⋮⋮). Panels snap to the screen edges and keep clear of the dock.
- **Fold**: double-click (or double-tap) the grip to shrink a panel to a small title; again to open it.
- **Change a reading**: click (tap) it to choose what it shows, add another beside it, remove it, change the panel's size (small, medium, large) or lay it out across or down.
- **Instruments drawer**: **Add panel**, **Fold all**, **Lock layout** (stops accidental moves), **Reset**, **Show instrument panels** (hide or show them all at once), and a tick beside each panel to hide just that one.

**What can be shown**

| Group | Readings |
| --- | --- |
| Navigation | SOG, COG, heading (°T and °M), boat speed, depth, position, log, trip, UTC and local time |
| Wind | AWS, AWA, TWS, TWA, TWD, VMG |
| Route | Next mark, BTW, DTW, VMC, TTG, ETA, XTE; the active routing's target TWA, boat speed and heading, and % of target speed |
| Cursor | Cursor position, bearing and range from the boat to the cursor (rhumb line) |
| GRIB at cursor | Wind speed and direction, gust, pressure, wave height, period and direction, rain, air temperature, the GRIB time and its name, for the GRIB shown on the chart at the cursor and slider time |

Port and starboard are marked on wind angles. Readings from Signal K grey out after 10 seconds without data, so stale numbers can't be mistaken for live ones. Depth turns amber within 2 m of the safety depth and red below it.

**The floating time slider**

A bar with the GRIB choice, step back and forward, play, **Now**, what the shading shows, the slider and the time. Drag it anywhere. With it and the GRIB-at-cursor panel you can close every drawer and still scrub the forecast and read values anywhere on the chart, which is the most useful layout for planning or talking a route through with crew. Hide it with ×; bring it back with Instruments > **Time slider**.

![Drawers closed: only the floating time slider and the GRIB-at-cursor and cursor panels over an ECMWF AIFS forecast for New Zealand](img/03-slider-grib-panel.jpg)

**Graphical readings**

Two readings are drawn rather than written, in the Graphic group of any tile's menu:

- **Wind**: a dial with the boat pointing up, apparent wind (orange) and true wind (blue) pointers, close-hauled sectors red to port and green to starboard, true wind speed at the top and apparent at the bottom.
- **Compass**: heading at the top of a turning card, with course over ground (orange) and the bearing to the next mark (green) on its rim.
- **Highway**: a stacked navigation display with heading, course to steer (CTS), speed over ground (SOG), waypoint distance (DTW), a full-width heading-following compass ribbon, a perspective track grid, cross-track error (XTE), time to go (TTG) and estimated arrival (ETA). The heading is marked at the ribbon centre and the bearing graduations scroll continuously. The perspective lane shifts with XTE while the vessel stays fixed. CTS steers back toward the active route leg, with the correction limited to 30°. When both heading and course over ground (COG) are available, their signed difference estimates the combined track offset; the target heading compensates for that offset. A horizontal arrow below the highway shows the direction and size of the required heading change. Values ease toward new readings at 4 Hz. During MOB the display switches to direct bearing and range to the MOB position. Without a heading sensor it works from course over ground (COG), and says so.

The main window does the work that must happen once, whatever the number of windows: it checks the alarms, plays their sound, and keeps the MOB and anchor watch. Monitor windows show the same alarms and can silence them. Keep the main window open; closing it closes the program.

## Alarms and man overboard

Alarms appear in a banner at the top of the chart and sound until silenced. The banner is in every window; **Silence**, accept and **Clear MOB** work from any of them. **Silence** stops the sound; the alarm stays in the banner while its cause lasts. Click an alarm's text to show its place on the chart. They work from live data (or a replay), not in planning.

**Instruments › Alarms**

| Alarm | Raised when | Settings |
| --- | --- | --- |
| **Shallow water** | The depth (under the keel if Signal K has it, else below the transducer) is less than the alarm depth | Alarm depth; empty = the safety depth (Chart panel) |
| **Air draft** | A bridge, overhead cable or pipeline on the chart within a mile (or 10 minutes) ahead has less charted clearance than your air draft plus the margin | Air draft in the **Boat** panel; margin. Charted clearances are above Highest Astronomical Tide |
| **Anchor watch** | The boat is outside the swing circle | **Drop anchor here** (at the boat), radius; the circle is drawn on the chart. **Weigh anchor** ends it |
| **Position lost** | Signal K is connected but no position has come for 30 seconds | On or off |
| **AIS collision, AIS-SART, MOB device, EPIRB** | See AIS targets | **Fleet › AIS** |

**Alarm volume** and **Test** are at the bottom.

**Man overboard**

Press the red **MOB** button at the top right the moment someone goes over. It is alone in that corner, in the same place and size in every view and in every window, and works with one press and no confirmation: a mistaken press costs little, a delayed one may not. Clearing a MOB asks twice.

TweedPlot marks the boat's position (or where it was put when planning) and raises an alarm that shows the time since, and the bearing and distance back to the position, updated every second, with a line from the boat to it on the chart. A large red **Man overboard** instrument panel comes up below the alarms with **Brg to MOB** (°T), **Rng to MOB** (metres under 0.2 NM, then NM), **Since MOB** (minutes and seconds) and **MOB at** (the position and the time it was pressed, local and UTC, for the log and a distress call), big enough to read from the helm; it comes up even if it was folded or hidden, can be dragged like any panel, and goes when the MOB is cleared. The Highway switches to direct bearing and range to the MOB position. Every window has the MOB button and shows the same readings for as long as the MOB is set, and the MOB can be cleared in any window. The position is also saved as a library mark in the group "MOB". **Clear MOB** in the banner, pressed twice, ends it.

**Drop mark**

The **Drop** button (bottom right of every window) marks the boat's position now and stamps it with the time, as a library mark named "Drop hh:mm:ss" in the group "Drops". Use it for a fish, a pot, a floating object or a depth change. Unlike MOB it raises no alarm and changes no view or instrument. The mark shows in the Library at once, in every window.

## AIS targets

**Fleet › AIS** shows ships, aids to navigation, SAR aircraft and AIS distress beacons on the chart, from two sources merged by MMSI:

- **Signal K**: whatever an AIS receiver or transponder on board passes to the Signal K server.
- **Online (aisstream.io)**: what volunteer shore stations hear. Get a free API key by signing in at aisstream.io with a GitHub account, paste it in and press **Go online**. **Online area** is the chart view (up to about 6° across), or a **Range** in NM round the chart centre or round your boat (it follows you). Online targets are not for navigation (see the AIS tab): they show in italics, marked "Net", and raise no collision alarm unless **Alarms for internet-only targets too** is ticked.

On the chart, ships are triangles pointing along their heading (class B a little smaller), coloured by type:

- <span class="mi"><svg viewBox="0 0 20 20"><path d="M10 2 L16 17 L10 14 L4 17 Z" fill="#27ae60" stroke="#1d2a33" stroke-width="1.2" stroke-linejoin="round"/></svg></span> Cargo
- <span class="mi"><svg viewBox="0 0 20 20"><path d="M10 2 L16 17 L10 14 L4 17 Z" fill="#c0392b" stroke="#1d2a33" stroke-width="1.2" stroke-linejoin="round"/></svg></span> Tanker
- <span class="mi"><svg viewBox="0 0 20 20"><path d="M10 2 L16 17 L10 14 L4 17 Z" fill="#2e6fd8" stroke="#1d2a33" stroke-width="1.2" stroke-linejoin="round"/></svg></span> Passenger
- <span class="mi"><svg viewBox="0 0 20 20"><path d="M10 2 L16 17 L10 14 L4 17 Z" fill="#e8792e" stroke="#1d2a33" stroke-width="1.2" stroke-linejoin="round"/></svg></span> Fishing
- <span class="mi"><svg viewBox="0 0 20 20"><path d="M10 2 L16 17 L10 14 L4 17 Z" fill="#9b59b6" stroke="#1d2a33" stroke-width="1.2" stroke-linejoin="round"/></svg></span> Sailing and pleasure craft
- <span class="mi"><svg viewBox="0 0 20 20"><path d="M10 2 L16 17 L10 14 L4 17 Z" fill="#1abc9c" stroke="#1d2a33" stroke-width="1.2" stroke-linejoin="round"/></svg></span> Tugs, pilots, search and rescue and other special craft
- <span class="mi"><svg viewBox="0 0 20 20"><path d="M10 2 L16 17 L10 14 L4 17 Z" fill="#f1c40f" stroke="#1d2a33" stroke-width="1.2" stroke-linejoin="round"/></svg></span> High-speed craft
- <span class="mi"><svg viewBox="0 0 20 20"><path d="M10 2 L16 17 L10 14 L4 17 Z" fill="#5d6d7e" stroke="#1d2a33" stroke-width="1.2" stroke-linejoin="round"/></svg></span> Military
- <span class="mi"><svg viewBox="0 0 20 20"><path d="M10 2 L16 17 L10 14 L4 17 Z" fill="#8a96a0" stroke="#1d2a33" stroke-width="1.2" stroke-linejoin="round"/></svg></span> Other or unknown type

Other AIS targets:

- <span class="mi"><svg viewBox="0 0 20 20"><path d='M10 3 L17 10 L10 17 L3 10 Z' fill='none' stroke='#6a1b9a' stroke-width='2'/></svg></span> Aid to navigation (AtoN): a buoy or beacon with its own AIS
- <span class="mi"><svg viewBox="0 0 20 20"><path d='M10 3 L17 10 L10 17 L3 10 Z' fill='none' stroke='#c2185b' stroke-width='2'/></svg></span> Virtual aid to navigation: only a broadcast, nothing is physically there
- <span class="mi"><svg viewBox="0 0 20 20"><circle cx='10' cy='10' r='7' fill='none' stroke='#d00000' stroke-width='2'/><path d='M5 5 L15 15 M15 5 L5 15' stroke='#d00000' stroke-width='2'/></svg></span> AIS-SART, MOB device or EPIRB (always raises an alarm)
- <span class="mi"><svg viewBox="0 0 20 20"><rect x='4' y='4' width='12' height='12' fill='none' stroke='#5d6d7e' stroke-width='2'/></svg></span> AIS base station
- <span class="mi"><svg viewBox="0 0 20 20"><path d='M10 2 L11 8 L17 11 L17 13 L11 11 L11 15 L13 17 L7 17 L9 15 L9 11 L3 13 L3 11 L9 8 Z' fill='#e67e22'/></svg></span> Search and rescue aircraft

A line ahead of a moving ship shows where it will be in 6 minutes (**Course vectors** sets the time). Targets not heard for a few minutes fade, and go after 20 minutes. Click a target for its name, type, status, speed and course, size, destination, and the range, bearing, CPA and TCPA from your boat.

**CPA lines** draw, for dangerous targets and the one you clicked (or every target closing within the time limit), dashed lines from your boat and the target to where each will be at the closest point of approach, joined by the CPA line with its distance and time: red inside the alarm limits, amber outside.

**Target list**: **Targets** on the rail (or **T**) opens a list over the chart with name, type, range, bearing, CPA, TCPA, speed, course, the age of the last position and the source. Click a heading to sort (again to reverse); distress beacons and dangerous targets always come first. The filter shows all targets, vessels only, those under way, those closing, or only dangerous ones. Click a row to centre the chart on the target and open its details. Drag the list by its title bar and resize it from the corner; it stays where you leave it. The list under the settings in the drawer sorts by range, CPA or name.

**Alarms**: a ship whose closest point of approach (CPA) is less than the limit within the time limit (TCPA) raises a collision alarm; anchored and moored ships can be ignored. An AIS-SART, MOB device or EPIRB always raises an alarm, with its bearing and distance.

**To scale**: at harbour scales, a ship that has sent its size is drawn as its outline at its real size, placed by its GPS antenna (the A, B, C and D distances in its AIS data) and pointing along its heading (or its course when it has no heading and is moving). Click the outline for its details as you would the symbol. Ships whose size or heading isn't known keep their symbol.

**Your own boat**: put your MMSI in the **Boat** panel (Own MMSI) and your boat is never shown or alarmed as a target, whichever source hears it.

**Tracks**: tick **Track of the selected target** and the boat you last clicked shows where it has been. Tracks are what TweedPlot has heard over the last 6 hours, so they start when the app does.

**Own fleet** (Fleet › Own fleet): a list of boats to pick out, such as friends, your company's vessels or a race fleet. Add an MMSI with a name and a group (Friends, Company, Race fleet, Other), or click a boat on the chart › **Add to own fleet**. Own-fleet boats are ringed in their group's colour and named at every zoom; each has a **Track** tick, **Track all** draws them all, and **Only these on the chart** hides every other AIS target. The list shows each boat's speed, distance and how long since it was heard; click one to show it.

## Marks, routes and the library

A **mark** is a named position kept in the library. A **route** (or course) is a start and an ordered list of marks. Marks are shared: move a mark once and every route using it follows.

**Building a route (Course drawer)**

1. **Set start**, then click the chart (or click a library mark to start at it). **From boat** starts at the boat.
2. **Add marks**, then click the chart for each mark in order, or click library marks to reuse them. Press Esc or **Add marks** again to stop.
3. Under each mark choose how it is to be reached (below) and, if you like, a radius.
4. Name the route and **Save**. New marks are saved to the library under the route's name.

The list shows each leg's distance. **↑ ↓** reorder marks; **✕** takes a mark off the route (it stays in the library). Click a mark on the chart to select it; wait for the green ring to drag it, or right-click to move or remove it. **Navigate ◀ ▶** chooses the mark the instruments steer to; with live data it moves on by itself when you reach a mark.

**How each mark is reached**

| Choice | Meaning for routing | On the chart |
| --- | --- | --- |
| Pass through | Reach the mark's arrival circle: its own radius if typed, else the routing's arrival radius. For waypoints and turning points | A grey dashed ring if it has its own radius |
| Leave to port | Pass with the mark on your left, at least the radius off. The routing may not cut through the mark's circle on the way in or out | Red ring |
| Leave to starboard | The same, mark on your right | Green ring |
| As close as possible | Reach within 0.01 NM (about 20 m): a fishing spot, a mooring, a fuel berth | Black ring |

Rounding works like the string rule in the racing rules: a string following the route from the previous mark to the next must go round the mark on the side asked. When the course turns towards that side (the mark is inside the turn: a windward mark, a turning mark you go out to and come back from, an island you sail round), the routing goes right round it. When the course turns away from that side, passing it on the correct side is enough. The rounding leg ends a little way out from the mark heading for the next one (about a mile, or half the distance to the next mark), so the route never runs up a blind inlet just to touch the line to the next mark. The routing really goes round, so if the way between the mark and the shore is closed by land, water shallower than the safety depth or a barrier, it stops and says there is no way round on that side, rather than touching the mark and turning back. Then check the rounding side in the sailing instructions, and only lower the safety depth if the water is really safe.

A mark in the middle of an island (as in "leave Little Barrier to starboard") is fine: give it a clearance (radius) that keeps the route off the shore by as much as you want, and the routing rounds the island.

Use the rounding sides for race marks and headlands you must keep off; use a generous radius for marks you only need to pass near (an offshore waypoint) so the routing is not forced through a single point.

**Mark types and points of interest**

In the mark editor (Library > Marks > **Edit**, which opens under that mark in the list, or right-click > **New library mark here**) choose a type: racing mark, waypoint, start or finish line end, danger, an IALA A buoy (port, starboard, cardinals, isolated danger, safe water, special), or a point of interest: good anchorage, mooring, shelter, fishing spot, diving spot, foul ground, wreck, hazard, landing point, camping spot, hut, viewpoint, water filling, fuel, provisions, note. Each draws with its own symbol. Any of them can be a course mark. Groups (e.g. "RNZYS 2026", "Fishing spots") keep the library tidy.

**Barriers**

Barriers are places routings must keep out of, beyond what the chart already avoids: a marine reserve, a race exclusion zone from the sailing instructions, a shipping lane, a bar you won't cross, or a rough headland you'd rather stay off.

- **Lines** (Library > Barriers > **Draw line**): fences the routing may not cross. Click points, double-click to finish.
- **Zones** (**Draw zone**): areas the routing may not enter. Click the corners, double-click to finish.
- **Barrier marks**: give any mark a **Barrier radius** in the mark editor and routings keep that far off it (a red dashed ring), unless the mark is on the course.

Untick a barrier to keep it but not use it; rename it in the list; move its points like marks; right-click it to delete a point, delete it, or switch it off. Routing > Limits and barriers > **Keep off barriers** turns them all on or off for routings.

![Library › Barriers with an example exclusion zone drawn across the first leg of a Coastal Classic course](img/10-barriers.jpg)

**The library (Library drawer)**

| Tab | Holds |
| --- | --- |
| Routes | Saved routes: open one in the Course drawer, export as GPX, delete |
| Marks | Every mark, by group: edit, show on the chart, delete (a mark used by a route asks first). Filters narrow the list: one group, one type (course marks and waypoints, buoys, dangers, points of interest), in a saved route or not, and only the marks in the chart view (the list follows the chart as you pan). The count shows how many pass; Filter the chart too applies the group, type and route filters to the marks drawn on the chart; Clear shows every mark again |
| Routings | Saved routings: show, delete |
| Tracks | Logged and imported tracks (see Your boat) |
| Barriers | Lines and zones (above) |

**GPX** exchanges marks, routes and tracks with plotters, phone apps, qtVlm, OpenCPN and Navionics. Import adds new marks and matches ones you already have by name and position. Exports go to `tweedplot-data\library\export`.

## Weather

Weather comes from GRIB files: gridded forecasts from computer models, one value per grid point per forecast time. TweedPlot downloads them, reads files from other sources, shows them on the chart, compares several at once, and routes through them. The Weather routing tutorial explains models, resolution and how much to trust them.

**Loading GRIBs (Weather drawer)**

The list shows every GRIB in the GRIB folders, described by model, run time, area and how far ahead it goes rather than by file name. Choose one and **Load** (↻ rescans the folders). Load as many as you like: different models, or several runs of one. One is shown on the chart (its radio button), and the table under the legend gives every loaded GRIB's values at the cursor. A tick beside each loaded GRIB includes it in the agreement displays; × unloads it.

TweedPlot reads GRIB2 (including ECMWF's compressed files) and GRIB1 (Saildocs and older tools): wind, gusts, pressure, waves (height, period, direction, swell, wind waves), rain, CAPE, air temperature and ocean currents. Outside a file's area or time span there is no value: a short GRIB never shows as calm.

**Time**

The slider, **◀ ▶|** steps, **▶** play and **Now** set the forecast time; the floating time slider does the same with the drawers closed. The time shows in local time with UTC in brackets.

**What the chart shows**

| Control | Choices |
| --- | --- |
| **Show** (shading) | Wind speed, gusts, wave height, swell, wave period, rain, CAPE (squall risk), pressure, air temperature, or none; the agreement and verify displays (below) |
| Wind barbs | Arrows with feathers: each full feather 10 kn, half feather 5 kn, flag 50 kn |
| Wave direction | Arrows pointing the way the waves travel |
| Isobars | Pressure lines at 1 to 10 hPa intervals (2 hPa suits the tropics, 4 hPa higher latitudes); **Auto** widens the interval when zoomed out |
| Display | Barb density and size, shading opacity and steps (smooth, or 1, 2 or 5 kn bands), shading under the land or under the coastline only |

Choices a GRIB doesn't contain are greyed out, and a wave-only GRIB switches to wave height by itself.

**Downloading (Weather > Download GRIBs)**

| Source | Models | Notes |
| --- | --- | --- |
| NOAA NOMADS | GFS 0.25°, 0.5°, 1° (wind, gusts, pressure, rain, CAPE, air temperature); GFS-Wave 0.25° | Cut to your area on the server: small downloads. Free, US government data |
| ECMWF open data | IFS 0.1° (~11 km grid; model about 9 km) and 0.25° (wind, pressure, gusts, air temperature); 0.1° IFS surface currents (non-tidal); AIFS 0.25° (ECMWF's AI model); ECMWF wave 0.1° and 0.25° | Free open data, CC BY 4.0, © ECMWF. The 0.1° products arrive about two hours after the 0.25° products; 0.1° surface and pressure-level data are separate global files. TweedPlot fetches only the fields you tick and crops them to your area. Requested 0.1° global fields are cached for reuse across areas (up to 5 GiB; oldest first). IFS 06Z and 18Z runs stop at 144 h |
| NOAA CoastWatch ERDDAP | MUR sea surface temperature, 1 km (ocean, see below) | A satellite analysis, cut to your area on the server. Free to use and share (NASA JPL) |
| NOAA CoastWatch THREDDS | Himawari-9 hourly sea temperature, 2 km (ocean, see below) | One file an hour, cut to your area on the server. GHRSST data, free and open |
| NOAA CoastWatch ERDDAP | Chlorophyll-a, 9 km, daily (ocean, see below) | Cut to your area on the server. Free to use and share |
| HYCOM (US Navy) | HYCOM ESPC ocean model, 1/12°: sea temperature, salinity, currents, and temperature and salinity at a depth (ocean, see below) | Cut to your area on the server. Public release; a demonstration service, so it may be down at times |

1. Choose the model and tick the fields.
2. Set the area: **Draw** a rectangle, use the **Chart view**, or the **Course** with a margin. Edges can be typed.
3. Choose the forecast length and time step; check the size estimate.
4. **Download**. TweedPlot finds the newest complete run, downloads it, saves it in the GRIB folder and loads it.

**ECMWF 0.1° product coverage**

The new 0.1° IFS data is the same high-resolution forecast as the 0.25° product, distributed on a finer grid (roughly 11 km north–south). It is free open data, released about two hours later than the 0.25° products. ECMWF publishes surface (`sfc`, including soil-level fields) and pressure-level (`pl`) files separately; the Download choices fetch selected surface fields, not the pressure-level files.

| File | Products available from ECMWF |
| --- | --- |
| Surface atmosphere | 10 m and 100 m wind components; 10 m gusts; 2 m temperature and dewpoint; 3-hour maximum and minimum 2 m temperature; mean sea-level and surface pressure; skin temperature; low, medium, high and total cloud cover; most-unstable CAPE |
| Precipitation and snow | Convective and total precipitation, precipitation rate and type; snowfall; runoff; snow depth, density and albedo |
| Water and radiation | Total column water and water vapour; cloud ice, cloud liquid water and rain water; surface and top-of-atmosphere shortwave and longwave radiation |
| Soil, land, ocean and ice | Soil temperature and soil water; land-sea mask and sub-grid orography; eastward and northward sea-water velocity components (surface-current components), sea-surface height and sea-ice thickness; eastward and northward turbulent surface stress |
| Pressure levels | Geopotential and geopotential height, temperature, U/V wind, specific and relative humidity, vertical velocity, divergence and relative vorticity, at 1000, 925, 850, 700, 600, 500, 400, 300, 250, 200, 150, 100, 50 and 10 hPa |
| Wave stream | Significant wave height, mean wave direction, peak/mean/zero-crossing periods, wave-height bands by period, and other wave-model diagnostics |

The surface-current fields are ECMWF's eastward (`sve`) and northward (`svn`) sea-water velocity components; together they describe current speed and direction. Tick **Surface currents** in the **ECMWF IFS 0.1°** download choices to fetch them. ECMWF's open-data product documentation does not specify whether these fields include astronomical tidal-current components, so do not treat them as a tide prediction. They can be drawn with **Show > Current** and included in routing when **Use GRIB currents** is enabled. **Surface currents** from HYCOM are also available in the ocean-model download.

**Ocean: sea temperature, salinity, currents and chlorophyll**

Four free ocean sources sit in the same Download list, for fishing and for anyone who cares where the warm water, the fronts and the currents are:

| Source | What | Resolution and time | Best for |
| --- | --- | --- | --- |
| **MUR sea temperature** | Sea surface temperature analysed from all the satellites, without cloud gaps | About 1 km, one analysis a day, usually a day or two old. **Days of analyses** sets how many of the latest days to fetch | The detail: fronts, eddies, upwelling, river plumes near the coast |
| **Himawari-9 sea temperature** | Sea surface temperature seen by the Japanese geostationary satellite over the western Pacific, every hour, where the sky is clear (only its best-quality pixels are kept) | About 2 km, hourly, two or three hours after the hour. **Hours back** sets how far back to fetch (3 to 72 hours), the step every 1, 2, 3 or 6 hours. Cloud leaves gaps: each hour keeps the last clear value seen in the hours downloaded, so fetch 12 or 24 hours to fill in round passing cloud | Today's water, hour by hour: a front moving with the tide, a warm tongue that MUR has not caught up with yet |
| **Chlorophyll-a** | The green of the plankton: the base of the food chain, where bait and fish gather | About 9 km, one gap-filled analysis a day from the VIIRS satellites, about three days old. **Days of analyses** as for MUR | Plankton blooms and colour fronts (green water against blue), upwelling; read it with the sea temperature |
| **HYCOM ocean model** | Sea surface temperature and salinity, surface currents, and (ticked) temperature and salinity at a **depth** you choose (10–500 m) | About 9 km, from the latest analysis out to an 8-day forecast, every 3 hours or more (HYCOM's server is very slow on hourly steps). Fetched one day at a time, as HYCOM asks: each day takes from a few seconds to a minute or two, and the progress bar counts them | Salinity, what is happening below the surface (the thermocline), currents, and where things are heading over the next days |

Area, length and step work as for GRIBs; an area across 180° isn't offered yet (split it). The files are saved in the GRIB folder like any other and load straight away.

Ocean data is shown under whatever weather GRIB is on the chart: choose **Sea temperature**, **Sea temperature, satellite hourly (Himawari)**, **Salinity**, their **at depth** versions, **Chlorophyll-a** or **Current** under **Show** and the shading comes from the loaded ocean file while the wind barbs stay on. Under the colour bar, three pointers set a **filter**: drag the outer two to the lowest and highest temperature (or salinity) you want to see, and nothing outside them is shaded; drag the red middle one to your **target**. The colours run from the low limit to the target over the cool half of the scale and from the target to the high limit over the warm half, so the target water is always the green-yellow middle and stands out, and it gets its own bold contour. The track spans the whole loaded file's range; **Fit** puts the pointers back to it. The colours depend only on the filter, never on the zoom, and each layer keeps its own filter between sessions. **Contours** draws lines within the filter at a spacing that suits it: bunched lines are a front. Chlorophyll spans a hundredfold (0.05 to 5 mg/m³ is usual), so its colours, filter and contours run on a log scale: each step along the colour bar multiplies the value, and the contours sit at tidy values (0.1, 0.2, 0.3, 0.5, 1, 2 …).

Values at the cursor show under the GRIB table, one line for each ocean file (its name, then sea temperature, satellite temperature, salinity, at depth, current and set, chlorophyll). The same values are on the **Sea at cursor** instrument panel, which appears when an ocean file is loaded and hides again when none is; tap a reading to swap it for another (*Sea at cursor* group: Sea temp, Sea temp sat, Salinity, Temp at depth, Salinity at depth, Current, Current set, Chlorophyll), as with any panel.

A daily MUR analysis stands for four days after its date, so it still shows at the slider's "now". Remember what each is: MUR is an analysis of the satellites' skin temperature (the top millimetre, smoothed where cloud hid the sea); HYCOM is a model, good for the big picture and the days ahead but not for a 1 km eddy. Where they disagree, MUR is the better guide to today's surface. Himawari is the freshest of all (a couple of hours old) and the most detailed in time, but it only sees through clear sky and reads the top skin of the sea, which a calm sunny afternoon warms by a degree or more; MUR smooths that out. An hourly view stands for half a day after its time; a daily chlorophyll analysis for six days after its date.

**Saildocs email request** (inside Download): for boats on email-only links (satellite or HF radio). It builds the request line from the same area, length and step, for GFS, ICON (0.125°) or the free part of ECMWF (0.4°), with a size estimate. **Email** opens a message to query@saildocs.com in your mail program; **Copy** copies the line. Save the GRIB that comes back attached into the GRIB folder. Keep requests small over satellite: fewer fields, a coarser grid, a shorter forecast.

**Weather on route**

**Weather on route** (Routing › a routing's buttons, or the Course drawer) opens a view in the dock with the forecast the boat meets along the way, in strips against time:

- **Wind**: true wind speed (solid) and gusts (dashed), with arrows along the top pointing where the wind blows.
- **Waves**: significant height Hs (solid, left scale) and peak period Tp (dotted, right scale), with arrows for the wave direction.
- Tick **Pressure**, **Rain**, **Air temp**, **CAPE**, **Sea temp** or **Current** to add their strips; your choice is remembered.

It follows the active routing, with its own times and positions (and its TWA in the readout). With no routing, it follows the course (start and marks) at the speed you set, from the weather slider's time. Values come from the GRIB shown on the chart, and anything it doesn't have (waves, sea temperature) from another loaded file that does. Hover a strip to see the boat's place on the chart and every value there; click to move the weather slider to that time. Distance run is marked under the times.

![Weather on route for a Golden Globe Race boat routed from the South Atlantic towards the Indian Ocean: 2659 NM over 16 days, with a gale of 40 kn gusting 55 and 5 to 6 m seas around 26 to 27 October; the purple line marks the slider time, and the popup on the chart is the routing at that moment](img/11-weather-on-route.jpg)

**Meteogram**

Right-click the chart > **Meteogram here**: every loaded GRIB's forecast at that point over time, in the dock. Wind speed is solid with gusts dashed and an ensemble's spread shaded; direction is drawn so GRIBs that agree stay together through north; pressure underneath. A line marks the slider time; click the graph to move the slider. A quick way to see when a front arrives, how long a blow lasts, or whether the models agree for your departure. 

![A meteogram south of Africa with ECMWF IFS (blue) and GFS (red): wind speed with gusts dashed, direction and pressure, the violet line at the slider time. The models agree on the blow on Tuesday and part company after Saturday](img/04-meteogram.jpg)

**Wind and waves in one file**

Wind and waves come from different models: GFS with GFS-Wave, ECMWF IFS or AIFS with the ECMWF wave model. TweedPlot can keep them in one file, so one GRIB shows the wind, the waves, the GRIB-at-cursor readings and the meteogram together, and a routing with a wave limit finds its waves without a second file.

- **When downloading**: for GFS, ECMWF IFS or AIFS, tick **Waves … in the same file**. The wave model at the matching ECMWF resolution is downloaded for the same run, area and steps and written into the same file, named `…+waves.grb2`. If the waves can't be had (not on the server yet, say), the wind file is kept and a note says so.
- **Files you already have**: load the wind GRIB and the wave GRIB of the same run. The Weather drawer offers **Combine**; it writes `<wind file>+waves.grb2`, loads it and unloads the two. The original files are kept.

**Agreement of the loaded GRIBs**

With two or more GRIBs loaded, **Show** offers displays of how much they disagree at the slider time: green where the forecasts agree, red where they don't. Vector disagreement (speed and direction together), speed spread, direction spread (blank in light winds), pressure spread, and run trend (newest run minus oldest: is the forecast getting windier?). The note under the legend lists what is compared. Where the forecasts disagree, plan with margin.

![Wind disagreement (vector) between ECMWF IFS and GFS over the South Atlantic three days ahead: green where the models agree, pink and red where they don't](img/08-agreement.jpg)

**Verify runs**

Weather > **Verify runs…** scores earlier forecasts against what a later run says actually happened at its first hour (its analysis), over the sea in the chart view: vector error, speed and direction bias, pressure bias. The track record averages every verify time by model and lead time, so after some days of downloads it shows which model has been doing best in your area. Click a forecast to shade its error on the chart.

**Weather stations and buoys**

**Weather › Weather stations and buoys** shows what the wind is actually doing at stations around the chart, the quickest way to see which forecast is right today. Tick **Show observations** and the sources:

| Source | What | Updates |
| --- | --- | --- |
| **MetService (NZ)** | MetService's synoptic stations around New Zealand, many on the coast and islands (Farewell Spit, Stephens Island, Cape Campbell …). Open access data | Hourly or 3-hourly |
| **Airports (METAR)** | Airport weather reports worldwide from aviationweather.gov | Half-hourly |
| **Buoys (NDBC)** | NOAA moored buoys and coastal stations, mostly around the US, with waves and sea temperature | Hourly |

Each station shows an arrow pointing downwind, coloured like the wind shading, with the wind speed and the gust after a G (15G25). Click a station for its latest report and a graph of its last 24 hours: observed wind (with gusts as dots) and pressure against each loaded GRIB's forecast for that place, and the average difference (the forecast minus what was observed, in knots and degrees of veer). Observations refresh every 10 minutes while shown.

## Routing

A routing is the fastest way from the start through your marks, given a forecast, your boat's speeds and the chart. TweedPlot uses the isochrone method (explained in the Weather routing tutorial) and keeps off land, water shallower than your safety depth, charted dangers, marine farms and your barriers. Lakes (from the Topo50 lake layer) can be routed on, using their charted depths where they have them; rivers are kept as land, so no route goes up one.

**Before you compute**

- **Boat drawer**: a polar (your boat's speed at each wind angle and strength), **efficiency** (90 % is realistic for most crews; less for short-handed or cruising trim), and the time lost in each **tack** and **gybe**.
- **Course drawer**: a start and at least one mark.
- **Weather drawer**: a GRIB covering the area and the time of the passage.

**Routing drawer, Run tab**

| Control | What it does |
| --- | --- |
| **Start time** | The weather slider time, Now (when you press Compute), or a time you type |
| Settings > **Time step** | Minutes per isochrone; 0 = automatic (about 80 steps per leg, 5 minutes to 3 hours) |
| Settings > **Arrival radius** | How close counts as reaching a mark without a radius of its own |
| Settings > **Use GRIB currents** | Adds available ocean currents from the GRIB to the boat's speed; ECMWF does not document its 0.1° currents as tidal-stream predictions |
| Settings > **Hovering a routing point moves the GRIB time** | Off by default, so pointing at the screen doesn't move the forecast |
| **Limits and barriers** | Below |
| **Compute** | Routes with the GRIB shown on the chart |
| **With each GRIB** | One routing per loaded GRIB, then opens the compare table |

Each result is drawn in its own colour. The active one shows its points (hover for time, heading, TWA, TWS and boat speed) and isochrones, and a summary: start, ETA, elapsed time, distance, average speed, time upwind, reaching and downwind, wind range, tacks and gybes, and each leg. A triangle on every routing shows where that boat is at the slider time, so scrubbing the time slider plays the routings against each other. **Save** keeps it in the library, **GPX** writes it with times, **Use its course** puts its marks back in the Course drawer.

**Limits and barriers**

| Limit | Use it to |
| --- | --- |
| **Search angle** (±° from the mark) | Speed up long routings by only trying headings towards the mark. Keep 90° or more where the route may need to tack, gybe or go round land |
| **Heading step** | Spacing of headings tried: larger is faster, smaller finds finer routes |
| **Wind min / max** | Keep out of calms (min) or of more wind than you want to sail in (max) |
| **Max wave height Hs** | Keep out of big seas: uses the waves in the routing GRIB, else a loaded wave GRIB covering the start. The message warns if no waves were found |
| **TWA min / max** | Keep the boat off the wind (e.g. 50° for a cruiser that won't pinch) or off a dead run (e.g. 165°) |
| **Keep off barriers** | Use the Library's barrier lines, zones and barrier marks |

Limits stop the routing sailing on from places outside them. If every way forward is blocked, the routing says so: relax a limit, move a barrier, or change the start time.

**Comparing routings (Routings tab)**

Every routing of this session and the saved ones are listed; tick to show, click to make it the active one. **Compare table** opens a table in the dock with a column for each statistic (choose them with ⚙, sort by any column; the best values are green: earliest finish, shortest elapsed time, fewest tacks and so on. Distance is marked only when no routing reached the finish, and then the furthest is best, since it made the most progress; once routings finish, sailing further is no merit and Elapsed shows the winner), with its own GRIB chooser and time slider. Typical uses: one routing per GRIB to see how much the route depends on the model; several start times to choose when to leave.

**Gate sweep (Gates tab)**

The automated gate method: one routing through each of a row of points ("gates") on the way to a mark, to see how much each option costs. Choose the mark the gates come before, a line along a meridian (e.g. 5°E) or a parallel, the range (e.g. 30°S to 40°S) and the number of gates; the gates show as violet dots. **Run gate sweep** uses the same start time and settings as Compute, with the GRIB shown or with each loaded GRIB. The Gates table in the dock gives each gate's arrival and the time lost against the best gate; with several GRIBs, the mean and worst time lost, ranked by the mean. The gate that is nearly best under every GRIB is the robust choice when the models disagree.

![A gate sweep on 25°E from 40°S to 50°S before the GGR mark at 45°S 40°E: one routing through each gate with each GRIB, and the Gates table ranking the gates by mean time lost](img/09-gate-sweep.jpg)

**Power boats**

Routing is built around a sailing polar, and dedicated power-boat routing (fuel use, sea-state slowdown, comfort) is planned. Meanwhile a power boat can be routed with a simple "power polar": a polar file with the same cruising speed at every wind angle and strength. The routing then finds the shortest safe track round land and dangers, and the wind, wave and barrier limits keep it out of weather you don't want. See the Route planning tutorial for how to make one.

## Measuring and fleet tracking

**Ruler**

Press **Ruler** (bottom of the rail), click to set the first point, click again for each next point, and double-click to finish; Esc stops. Each leg is labelled with its bearing (°T, plus °M when Signal K knows the magnetic variation) and distance, and the total shows at the end. Rulers stay on the chart until deleted. Move their points like marks. Right-click a ruler to delete a point, delete it, delete all rulers, or **Save ruler as route** (its points become library marks and a route opens in the Course drawer): a fast way to sketch a passage.

Ruler bearings and distances are rhumb lines: a constant compass course, which is what you steer and what a paper chart's straight line shows. Over a few hundred miles a great circle is shorter; the router allows for that itself.

**Cursor readings**

The Cursor panel shows the cursor's latitude and longitude and the bearing and range to it from the boat. With the GRIB-at-cursor panel beside it you can point at any spot and read its position, distance from you and forecast at the slider time.

**Fleet (YB race tracking)**

Fleet › YB race: type the race's YB name (as in `yb.tl/<race>`, for example `ggr2026`) and **Open**. TweedPlot downloads the race setup, every boat's published track, then the latest positions every 5 minutes to 3 hours.

- Boats show at their latest fix, or where they were at the weather slider time, so you can replay a race against the forecast.
- ★ follows a boat (thicker track, name kept first when names would overlap). Choose a class, tracks for all or followed boats, names, and the YB course.
- The Fleet table in the dock ranks boats by distance to finish, with speed and course made good and distance in the last 24 hours.
- **Copy course** puts the YB course in the Course drawer; **Route** on a boat starts a routing from its last fix and time: what would the router have them do now?

Positions may be delayed by the race organiser.

## Troubleshooting

| Problem | Likely cause and fix |
| --- | --- |
| No detailed NZ chart | The LINZ zips aren't in `tweedplot-data\linz`, or only small-scale bands were downloaded. Add the bands for the area and restart |
| A harbour's buoys are missing | The open data has no large-scale coverage there. Add them as library marks with buoy symbols |
| Signal K button stays red or amber | Red: the server isn't running, or is on another port or PC. Amber: connected, but the server is sending no boat data (check its connections to the GPS and instruments). Hover the button, and check the Chart and Signal K drawer's details and `RACENAV_SK_URL` |
| Instruments show — | No data for that reading, or more than 10 s old (greyed out) |
| "Load the GRIB first" | Routing needs the GRIB shown on the chart to be loaded and cover the start time |
| "The weather runs out first" | The GRIB ends before the boat arrives: download a longer forecast, or accept the partial routing |
| "No wind data" | The route leaves the GRIB's area: download a bigger area |
| "Start (or mark) blocked" | It is on land, in water shallower than the safety depth, or inside a barrier: move it |
| "Every heading is blocked" | Land, shallow water, barriers or limits close every way forward: relax a limit or check the safety depth |
| The routing hugs a point | A mark without its own radius uses the arrival radius: give offshore waypoints a radius of a mile or more |
| ECMWF download slow or huge | ECMWF publishes whole-globe fields: choose fewer fields, a shorter forecast or a longer time step. 0.1° is much larger per field than 0.25°; requested global fields are cached (up to 5 GiB) and reused for later area requests |
| Saildocs reply too big | Coarser grid, fewer fields, shorter forecast, smaller area |
| A panel or the time slider has gone | Instruments drawer: **Show instrument panels**, the tick beside the panel, or **Time slider** |
