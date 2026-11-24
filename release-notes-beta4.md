Test build. Not for navigation.

**Installers:** `TweedPlot_1.0.0-beta.4_x64-setup.exe` for most Windows PCs; `TweedPlot_1.0.0-beta.4_arm64-setup.exe` for Windows on Arm (Snapdragon, Surface Pro X …). The installer is not code-signed yet: Windows SmartScreen will warn ("Windows protected your PC") — choose More info › Run anyway. Installed betas offer this update themselves.

## What's new since beta.3

**Ocean data**
- Sea temperature, currents and chlorophyll download seamlessly across 180° and at full resolution (the size line says when an area is thinned).
- Copernicus Marine: the global ocean model (temperature, salinity, currents, mixed layer, sea level) and biology model, and 4 km chlorophyll. No account needed.
- Two more sea temperature sources: ACSPO L3S-LEO (2 km) and Geo-Polar Blended (5 km, with its analysis error).
- Below the surface: temperature and current at a chosen depth, thermocline and current shear maps, the water column under the cursor (in the dock, pin with a click), and a section along a line.
- Animated current flow.
- Four-pointer filter: a highlight band with a target inside the colour scale, set from the colour key in any view; contours follow the view or the band.
- Species presets: albacore, southern bluefin, yellowfin, bigeye (at 250 m), skipjack, striped marlin and mahi-mahi, each with its usual temperature band (sources in the manual).
- Tidy up removes files by when their data ends, and says why older GRIBs in qtVlm's folder are left alone.

**Charts and images**
- Images can be used where they are (charts already installed for another program, or on an external drive) as well as copied in; whole folders at once.
- MBTiles (NOAA chart tiles, OpenSeaMap, satellite collections).
- Pictures (PNG, JPEG, TIFF) placed by a world file, or calibrated by hand: control points on the picture and the chart, error per point, nudge to line up with your track. Old KAP charts and GeoTIFFs that sit in the wrong place can be corrected the same way.

**Screen and controls**
- Laid out for mouse and keyboard at one size; touch support dropped for now.
- Dark drawers; a compact data panel head (MOB, Drop and Anchor always at the top, views as icons, chart detail beside them, Std for the standard display); every toolbar button labelled, in three groups; Left and Right panel buttons (P and O).
- Capture (Ctrl+Shift+S): a screenshot of the window into TweedPlot-Screenshots on the Desktop (or a folder you choose), for your records or for support.
- Tides (rising/falling and tidal streams) follow the chart time in Planning and Environment.
- Legibility: menus that were unreadable in the dark drawers fixed; amber warnings with dark text; nothing relies on red against green alone (status dots differ in shape, the drag-ready ring turns blue and fills, the compass's bearing to the mark is a hollow diamond, dangerous targets carry ⚠).

**Fixes**
- Warnings flashed dark red like alarms; they now flash amber.
- The message "the route being navigated is not the one below" was hidden.
- Esc with a drawer open threw an error.

Please report problems and ideas as GitHub issues, with a Capture screenshot where it helps.
